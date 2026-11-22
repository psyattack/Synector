//! Per-frame daemon sync: poll cadence, timeout detection, snapshot application and the
//! sustained-outage rule that turns single missed polls into a reported disconnect.

use std::time::Instant;

use crate::ipc::{DaemonState, IpcRequest};

use super::ipc::IpcJobKind;
use super::GuiState;


/// Daemon sync cadence: fast while something executes (live progress), relaxed when idle.
const DAEMON_SYNC_FAST_MS: u128 = 80;
const DAEMON_SYNC_IDLE_MS: u128 = 400;
/// Consecutive failed polls and the quiet period that turn a single miss into a confirmed
/// outage: the client reports a lost daemon instead of treating one dropped poll as a disconnect.
const DAEMON_OUTAGE_POLLS: u32 = 3;
const DAEMON_OUTAGE_GRACE: std::time::Duration = std::time::Duration::from_secs(1);
/// A state poll that takes longer than this is treated as a failure. The IPC client has no read
/// timeout, so without it a hung daemon would leave the GUI waiting silently forever.
const DAEMON_SYNC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Daemon sync cadence and outage bookkeeping.
pub(super) struct DaemonSync {
    last_sync: Instant,
    last_success: Instant,
    last_error: Option<String>,
    /// Set while a state request is outstanding; used to detect a daemon that never answers.
    pending_since: Option<Instant>,
}

impl DaemonSync {
    pub(super) fn new() -> Self {
        let now = Instant::now();
        Self {
            // Start due so the first frame primes the state immediately.
            last_sync: now - std::time::Duration::from_secs(10),
            last_success: now - std::time::Duration::from_secs(10),
            last_error: None,
            pending_since: None,
        }
    }

    fn due(&self, executing: bool) -> bool {
        let interval = if executing {
            DAEMON_SYNC_FAST_MS
        } else {
            DAEMON_SYNC_IDLE_MS
        };
        self.last_sync.elapsed().as_millis() >= interval
    }

    fn mark_requested(&mut self) {
        self.last_sync = Instant::now();
        self.pending_since = Some(self.last_sync);
    }

    /// Makes the next poll due immediately (after a mutation, so the fresh state is fetched).
    pub(super) fn force(&mut self) {
        self.last_sync = Instant::now() - std::time::Duration::from_secs(1);
    }

    fn note_success(&mut self) {
        self.last_success = Instant::now();
        self.last_error = None;
        self.pending_since = None;
    }

    /// Records a failed poll. Returns `Some(detail)` the first time a streak starts so the
    /// caller can log it once instead of on every 80 ms retry.
    fn note_failure(&mut self, error: String) -> Option<String> {
        let first = self.last_error.is_none();
        self.last_error = Some(error.clone());
        if first {
            Some(error)
        } else {
            None
        }
    }

    fn outage_confirmed(&self, consecutive_failures: u32) -> bool {
        consecutive_failures >= DAEMON_OUTAGE_POLLS
            && self.last_success.elapsed() >= DAEMON_OUTAGE_GRACE
    }
}

impl GuiState {
    /// Drains finished IPC work and queues a daemon poll when one is due. Called once per frame
    /// from `SynectorApp::ui`; it never blocks on the named pipe.
    pub fn sync_with_daemon(&mut self) {
        self.drain_ipc_results();
        self.check_sync_timeout();
        if self.ipc.sync_in_flight || !self.daemon_sync.due(self.is_any_rule_executing()) {
            return;
        }
        self.daemon_sync.mark_requested();
        if self.enqueue_ipc(IpcJobKind::Sync, IpcRequest::GetState, None) {
            self.ipc.sync_in_flight = true;
        }
    }

    /// An outstanding poll that never returns must not stall the client silently: treat it as a
    /// failure and let the cadence retry (the hung request stays queued in the worker).
    fn check_sync_timeout(&mut self) {
        let Some(since) = self.daemon_sync.pending_since else { return };
        if since.elapsed() < DAEMON_SYNC_TIMEOUT {
            return;
        }
        self.ipc.sync_in_flight = false;
        self.daemon_sync.pending_since = None;
        self.note_sync_failure("the daemon did not answer the state request within 2 s".to_string());
    }

    /// Applies every completed job on the frame thread.
    fn drain_ipc_results(&mut self) {
        while let Ok(result) = self.ipc.result_rx.try_recv() {
            self.apply_ipc_result(result);
        }
    }

    pub(super) fn apply_daemon_state(&mut self, state: DaemonState) {
        self.has_connected_daemon = true;
        self.failed_daemon_syncs = 0;
        self.is_daemon_connected = true;
        self.daemon_sync.note_success();
        let rules_action_counts: std::collections::HashMap<String, usize> = state
            .config
            .rules
            .iter()
            .map(|r| (r.id.clone(), r.actions.len()))
            .collect();

        self.active_test_runs.retain(|id, started_at| {
            if state.active_test_rule_ids.contains(id) {
                return true;
            }
            let num_actions = rules_action_counts.get(id).copied().unwrap_or(1);
            let min_dur = std::time::Duration::from_millis(((num_actions * 400).max(900) as u64).min(4000));
            started_at.elapsed() < min_dur || state.active_rule_executions.contains_key(id)
        });
        for id in &state.active_test_rule_ids {
            self.active_test_runs.entry(id.clone()).or_insert_with(Instant::now);
        }

        match state.available_update {
            Some(ref update) => {
                if self.available_update.as_ref().map(|u| &u.version) != Some(&update.version) {
                    self.available_update = Some(update.clone());
                }
            }
            // The daemon is the authority: a manual check that found nothing and a fresh daemon
            // must not leave the client offering a stale release. A snapshot that was in flight
            // while the client changed the advertisement cannot be distinguished from a fresh
            // one, so it is ignored for the grace period.
            None => {
                if self.updates.daemon_may_clear() {
                    self.clear_available_update();
                }
            }
        }
        if let Some(pct) = state.update_progress {
            self.is_downloading_update = true;
            self.update_download_progress = pct;
        } else if self.is_downloading_update {
            self.is_downloading_update = false;
        }
        if state.pending_open_update_modal {
            self.show_changelog_modal = true;
            self.enqueue_ipc(
                IpcJobKind::Background,
                IpcRequest::AcknowledgeOpenUpdateModal,
                None,
            );
        }
        if state.pending_auto_start_update && !self.is_downloading_update {
            if let Some(ref update) = state.available_update {
                self.enqueue_ipc(
                    IpcJobKind::Background,
                    IpcRequest::AcknowledgeAutoStartUpdate,
                    None,
                );
                self.start_update(update.clone());
            }
        }

        self.daemon_state = Some(state);
    }

    pub(super) fn note_sync_failure(&mut self, error: String) {
        self.failed_daemon_syncs = self.failed_daemon_syncs.saturating_add(1);
        if let Some(first) = self.daemon_sync.note_failure(error) {
            crate::logging::warn("GUI", &format!("Daemon state sync failed: {first}"));
        }
        // A single missed poll is not an outage: report the disconnected state only after a
        // sustained stretch of failures, with the collected error detail.
        if self.daemon_sync.outage_confirmed(self.failed_daemon_syncs) && self.is_daemon_connected {
            let last_error = self.daemon_sync.last_error.clone();
            crate::logging::warn(
                "GUI",
                &format!(
                    "Daemon unreachable after {} failed polls: {}",
                    self.failed_daemon_syncs,
                    last_error.as_deref().unwrap_or("no response")
                ),
            );
            self.is_daemon_connected = false;
            let detail = last_error.as_deref().unwrap_or(self.loc().toast_daemon_no_response);
            self.set_toast(format!("{}: {detail}", self.loc().settings_daemon_offline));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::IpcResponse;
    use super::super::ipc::IpcJobResult;

    fn daemon_state_json(value: serde_json::Value) -> DaemonState {
        serde_json::from_value(value).expect("daemon state deserializes")
    }

    #[test]
    fn test_sync_failure_needs_a_sustained_outage_and_keeps_the_error() {
        let mut state = GuiState::new();
        state.language = crate::i18n::Language::English;
        state.has_connected_daemon = true;
        state.is_daemon_connected = true;
        state.daemon_sync.note_success();

        for failures in 1..=2 {
            state.apply_ipc_result(IpcJobResult {
                kind: IpcJobKind::Sync,
                result: Err("pipe closed".into()),
            });
            assert_eq!(state.failed_daemon_syncs, failures);
            assert!(state.is_daemon_connected, "one or two missed polls are not an outage yet");
        }
        assert_eq!(state.daemon_sync.last_error.as_deref(), Some("pipe closed"));

        // The quiet period has elapsed: the third failure confirms the outage and is reported.
        state.daemon_sync.last_success = Instant::now() - std::time::Duration::from_secs(2);
        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::Sync,
            result: Err("pipe closed".into()),
        });
        assert!(!state.is_daemon_connected);
        let toast = state.toast.as_ref().expect("the outage is reported").0.clone();
        assert_eq!(
            toast,
            format!("{}: pipe closed", crate::i18n::get(state.language).settings_daemon_offline)
        );

        // A successful poll resets the streak and the stored error.
        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::Sync,
            result: Ok(IpcResponse::State(daemon_state_json(serde_json::json!({
                "autorun_enabled": false,
                "uptime_secs": 0,
                "daemon_memory_bytes": 0,
                "daemon_cpu_usage": 0.0,
                "active_rules_count": 0,
                "total_rules_count": 0,
                "config": { "rules": [] },
                "logs": []
            })))),
        });
        assert!(state.is_daemon_connected);
        assert_eq!(state.failed_daemon_syncs, 0);
        assert!(state.daemon_sync.last_error.is_none());
    }

    #[test]
    fn test_overdue_sync_is_reported_instead_of_waiting_forever() {
        let mut state = GuiState::new();
        state.has_connected_daemon = true;
        state.is_daemon_connected = true;
        state.daemon_sync.note_success();
        state.ipc.sync_in_flight = true;
        state.daemon_sync.pending_since = Some(Instant::now() - DAEMON_SYNC_TIMEOUT);

        state.check_sync_timeout();

        assert!(!state.ipc.sync_in_flight, "an overdue poll stops blocking the next one");
        assert_eq!(state.failed_daemon_syncs, 1);
        let error = state.daemon_sync.last_error.as_deref().expect("timeout is recorded");
        assert!(error.contains("did not answer"), "error: {error}");
        assert!(state.is_daemon_connected, "one overdue poll is not an outage");
    }
}
