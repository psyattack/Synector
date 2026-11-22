//! One-shot `--status` report rendered over IPC.

use crate::ipc::{IpcClient, IpcRequest, IpcResponse};

/// Prints the daemon status and returns; the dispatch never continues after `--status`.
pub(crate) fn report_status() {
    match IpcClient::call(&IpcRequest::GetState) {
        Ok(IpcResponse::State(state)) => {
            println!("DAEMON STATUS: Online");
            println!("  Rules: {} (active: {})", state.total_rules_count, state.active_rules_count);
            println!("  Daemon Memory: {:.2} MB", state.daemon_memory_bytes as f64 / 1_048_576.0);
        }
        Ok(other) => println!("Daemon response: {:?}", other),
        Err(e) => println!("DAEMON STATUS: Offline ({e})"),
    }
}
