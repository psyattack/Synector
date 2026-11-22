use std::time::Instant;

use crate::ipc::IpcResponse;
use crate::models::LogItem;

use super::DaemonEngine;

/// Serializes config-section writes inside this process. The daemon is the only writer, but the
/// test harness runs many engines in one process against one config directory, and two atomic
/// writes to the same section would otherwise race each other and report a spurious failure.
static CONFIG_WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(super) fn config_write_guard() -> std::sync::MutexGuard<'static, ()> {
    CONFIG_WRITE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl DaemonEngine {
    /// Publishes the current rule definitions to recurring executions. A recurring loop re-reads
    /// its rule every cycle instead of executing the pipeline it captured when it started.
    pub(super) fn refresh_live_rules(&self) {
        if let Ok(mut live) = self.live_rules.write() {
            *live = self.config.rules.clone();
        }
    }
}

impl DaemonEngine {
    /// Persists positions streamed by an overlay drag at most once per window; the
    /// in-memory position is always current, only the disk write is deferred.
    pub fn persist_overlay_positions_if_due(&mut self) {
        if !self.overlay_position_library_dirty && !self.overlay_position_rules_dirty {
            return;
        }
        if self.last_overlay_position_save.elapsed() < Self::OVERLAY_POSITION_SAVE_INTERVAL {
            return;
        }
        self.persist_overlay_positions_now();
    }

    /// Forces deferred overlay positions to disk (daemon shutdown, calibration end).
    pub fn persist_overlay_positions_now(&mut self) {
        if self.overlay_position_library_dirty && self.persist_library() {
            self.overlay_position_library_dirty = false;
        }
        if self.overlay_position_rules_dirty && self.persist_rules() {
            self.overlay_position_rules_dirty = false;
        }
        self.last_overlay_position_save = Instant::now();
    }

    pub(super) fn mark_overlay_position_dirty(&mut self, library_changed: bool, rules_changed: bool) {
        self.overlay_position_library_dirty |= library_changed;
        self.overlay_position_rules_dirty |= rules_changed;
    }

    pub(super) fn persist_rules(&mut self) -> bool {
        #[cfg(test)]
        if self.forced_persist_failure {
            self.log("ERROR", "CONFIG", "Failed to save rules: injected test failure");
            return false;
        }
        let _write = config_write_guard();
        match self.config.save_rules() {
            Ok(()) => true,
            Err(err) => {
                self.log("ERROR", "CONFIG", &format!("Failed to save rules: {err}"));
                false
            }
        }
    }

    pub(super) fn persist_library(&mut self) -> bool {
        let _write = config_write_guard();
        match self.config.save_library() {
            Ok(()) => true,
            Err(err) => {
                self.log("ERROR", "CONFIG", &format!("Failed to save presets: {err}"));
                false
            }
        }
    }

    pub(super) fn persist_settings(&mut self) {
        let _write = config_write_guard();
        if let Err(err) = self.config.save_settings() {
            self.log("ERROR", "CONFIG", &format!("Failed to save settings: {err}"));
        }
    }

    /// IPC answer for a failed config write. A handler that answers for a write must never report
    /// success when the daemon could not store it.
    pub(super) fn persist_failure(section: &str) -> IpcResponse {
        IpcResponse::Error(format!("failed to save {section}"))
    }
}

impl DaemonEngine {
    pub fn log(&mut self, level: &str, category: &str, message: &str) {
        self.log_with_file_message(level, category, message, None);
    }

    /// Shared implementation of [`Self::log`]. `file_message` overrides the text mirrored into the
    /// daily file log when the in-app message may carry user data (script output, HTTP bodies).
    pub(super) fn log_with_file_message(
        &mut self,
        level: &str,
        category: &str,
        message: &str,
        file_message: Option<&str>,
    ) {
        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();

        if let Some(last) = self.logs.last_mut() {
            if last.level == level && last.category == category {
                let (is_match, count) = if last.message == message {
                    (true, 1)
                } else if let Some(stripped) = last.message.strip_prefix(message) {
                    if let Some(inner) = stripped.strip_prefix(" (x").and_then(|s| s.strip_suffix(')')) {
                        if let Ok(n) = inner.parse::<usize>() {
                            (true, n)
                        } else {
                            (false, 1)
                        }
                    } else {
                        (false, 1)
                    }
                } else {
                    (false, 1)
                };

                if is_match {
                    let next_count = count + 1;
                    last.message = format!("{} (x{})", message, next_count);
                    last.timestamp = timestamp;
                    return;
                }
            }
        }

        while self.logs.len() >= Self::MAX_LOGS {
            // Prioritize preserving RULE and ERROR logs: evict oldest entry that is neither RULE nor ERROR
            if let Some(pos) = self.logs.iter().position(|l| !l.category.eq_ignore_ascii_case("RULE") && !l.level.eq_ignore_ascii_case("ERROR")) {
                self.logs.remove(pos);
            } else {
                self.logs.remove(0);
            }
        }
        self.logs.push(LogItem {
            timestamp,
            level: level.to_string(),
            category: category.to_string(),
            message: message.to_string(),
        });
        // Mirror the engine event stream into the daily file log. Consecutive duplicates are
        // already collapsed above, so a chatty rule cannot flood the file.
        crate::logging::engine_log(level, category, file_message.unwrap_or(message));
    }
}

impl DaemonEngine {
    /// The updater fields the update-progress overlay needs. `GetUpdateProgress` answers from
    /// this pair without projecting a [`DaemonState`]; the full snapshot projects the same pair,
    /// so the two answers cannot disagree.
    pub(super) fn update_progress_snapshot(&self) -> (Option<f32>, Option<String>) {
        self.update.progress_snapshot()
    }
}
