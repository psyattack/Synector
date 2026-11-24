//! Analytics dialog state and polling.
//!
//! The level filter of the activity-log section lives here (not in egui's temporary data) so a
//! handoff from the global search can reset it and the count the search promises matches the
//! stream it opens. The snapshot itself is produced by the daemon: the GUI queues one narrow
//! `GetAnalytics` request per second only while the dialog is open.

use std::time::{Duration, Instant};

use crate::ipc::IpcRequest;

use super::ipc::IpcJobKind;
use super::GuiState;

/// Level filter of the analytics activity log.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AnalyticsLogFilter {
    #[default]
    All,
    Rules,
    Processes,
    Daemon,
    Errors,
}

/// Refresh cadence of the analytics dashboard. The collector samples system metrics once a second,
/// so a faster poll would only re-read the same numbers.
pub(super) const ANALYTICS_POLL: Duration = Duration::from_millis(1000);

impl GuiState {
    /// Makes the next analytics poll due immediately, so opening the dialog shows fresh data
    /// instead of waiting out the previous interval.
    pub(super) fn schedule_analytics_poll(&mut self) {
        self.ipc.analytics_last_poll = Instant::now() - ANALYTICS_POLL;
    }

    /// Queues one analytics snapshot when the dialog is open and the previous request finished.
    /// The worker owns the round-trip; the frame thread never blocks on the named pipe.
    pub fn poll_analytics(&mut self) {
        if !self.show_analytics_modal {
            // A request that was in flight when the dialog closed still lands and refreshes the
            // cache; its handler clears the flag. Nothing new is queued.
            return;
        }
        if self.ipc.analytics_in_flight {
            return;
        }
        let elapsed = self.ipc.analytics_last_poll.elapsed();
        if elapsed < ANALYTICS_POLL {
            // The window repaints on a fixed cadence of its own, so without this the poll lands on
            // the next frame after the interval instead of when it is due.
            self.ipc
                .request_repaint_after(ANALYTICS_POLL.saturating_sub(elapsed));
            return;
        }
        self.ipc.analytics_last_poll = Instant::now();
        if self.enqueue_ipc(IpcJobKind::Analytics, IpcRequest::GetAnalytics, None) {
            self.ipc.analytics_in_flight = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::state::ipc::IpcJobResult;
    use crate::ipc::IpcResponse;

    fn state_with_dialog(open: bool) -> GuiState {
        let mut state = GuiState::new();
        state.show_analytics_modal = open;
        state
    }

    #[test]
    fn a_closed_dialog_queues_no_poll() {
        let mut state = state_with_dialog(false);
        state.ipc.analytics_last_poll = Instant::now() - ANALYTICS_POLL;
        state.poll_analytics();
        assert!(!state.ipc.analytics_in_flight, "a closed dialog never polls");
        assert!(
            state.ipc.analytics_last_poll.elapsed() >= ANALYTICS_POLL,
            "the closed dialog leaves the poll stamp alone"
        );
    }

    #[test]
    fn an_open_dialog_polls_once_per_interval() {
        let mut state = state_with_dialog(true);
        // Not due yet: the poll is skipped, and the frame loop is woken when it becomes due.
        state.ipc.analytics_last_poll = Instant::now();
        state.poll_analytics();
        assert!(!state.ipc.analytics_in_flight);

        // Due: exactly one request is queued.
        state.ipc.analytics_last_poll = Instant::now() - ANALYTICS_POLL;
        state.poll_analytics();
        assert!(state.ipc.analytics_in_flight);
        let stamp = state.ipc.analytics_last_poll;
        state.poll_analytics();
        assert_eq!(
            state.ipc.analytics_last_poll, stamp,
            "an in-flight request blocks a second poll"
        );

        // The result clears the flag so the next interval can queue again.
        let snapshot = crate::models::AnalyticsSnapshot {
            session_executions: 2,
            ..Default::default()
        };
        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::Analytics,
            result: Ok(IpcResponse::Analytics(snapshot.clone())),
        });
        assert!(!state.ipc.analytics_in_flight);
        assert_eq!(state.analytics, Some(snapshot));
    }
}
