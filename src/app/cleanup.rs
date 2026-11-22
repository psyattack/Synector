//! Update-file cleanup, with its two gates spelled out instead of implied by call order.

use crate::ipc::IpcClient;

/// Removes staged update downloads, but only while the daemon is offline: a running daemon may be
/// streaming a download into one of those files.
fn remove_temp_update_files_when_offline() {
    if !IpcClient::is_daemon_running() {
        crate::updater::clean_temp_update_files();
    }
}

/// Runs both cleanup gates in dispatch order: the stale executable is unconditional (the updater
/// owns it), temporary update files are removed only while the daemon is offline.
pub(crate) fn run_update_cleanup() {
    crate::updater::remove_old_executable();
    remove_temp_update_files_when_offline();
}
