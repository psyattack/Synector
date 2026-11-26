//! Action-side view of the shared subprocess helpers in [`crate::command`].
//!
//! The bounded runner, `CommandOutcome` and the PowerShell command constructors live in the
//! crate-level `command` module. This module keeps the action-family import paths and the
//! action-specific wall-clock ceilings in place.

pub(in crate::daemon::actions) use crate::command::{
    apply_no_window, powershell_command, powershell_helper_command, run_command_bounded_async,
    run_command_bounded_tracked_async, run_system_tool, CommandCancellation, CommandOutcome,
    UTILITY_TIMEOUT,
};

/// Wall-clock ceilings for helper subprocesses. User scripts may legitimately run for
/// minutes; elevation waits for a UAC prompt; archive payloads are large.
pub(in crate::daemon::actions) const SCRIPT_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(300);
pub(in crate::daemon::actions) const ARCHIVE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(600);
pub(in crate::daemon::actions) const ELEVATION_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(120);
pub(in crate::daemon::actions) const AUDIO_HELPER_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(30);

pub(in crate::daemon::actions) fn ps_single_quoted(value: &str) -> String {
    crate::win_args::ps_single_quoted(value)
}
