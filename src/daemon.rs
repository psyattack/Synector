pub mod actions;
pub mod conditions;
pub mod discord_rpc;
pub mod engine;
pub mod file_watcher;
pub mod hotkeys;
pub mod ipc_server;
pub mod lua_runner;
pub mod overlay_manager;
pub mod pe_mask;
pub mod runtime;
pub mod system_events;
pub mod telemetry;
pub mod tray;
pub mod watcher;
pub mod webhook_server;

pub use engine::activity as rule_activity;
pub use engine::rule_call;

#[cfg(test)]
pub(crate) use runtime::launch::parse_gui_exe_argument;
pub use runtime::launch::launching_gui_exe;
pub use runtime::shutdown::request_shutdown;

pub fn run_daemon() {
    runtime::bootstrap::run();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::runtime::shutdown::{run_shutdown_sequence, ShutdownSignal};
    use std::path::PathBuf;
    use std::sync::Arc;

    #[test]
    fn test_shutdown_signal_wakes_waiter() {
        let signal = Arc::new(ShutdownSignal::new());
        let waiter = {
            let signal = Arc::clone(&signal);
            std::thread::spawn(move || signal.wait())
        };
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(
            !waiter.is_finished(),
            "the headless wait must block until a shutdown is requested"
        );
        signal.request();
        waiter.join().expect("waiter must finish after the shutdown request");
    }

    #[test]
    fn test_shutdown_request_is_idempotent() {
        let signal = ShutdownSignal::new();
        signal.request();
        signal.request();
        signal.wait();
    }

    #[test]
    fn test_parse_gui_exe_argument_keeps_the_launching_path() {
        let args = vec![
            "Synector.exe".to_string(),
            "--daemon".to_string(),
            "--spawned-by-gui".to_string(),
            "--gui-exe".to_string(),
            "C:\\Program Files\\Synector\\Synector.exe".to_string(),
        ];
        assert_eq!(
            parse_gui_exe_argument(&args),
            Some(PathBuf::from("C:\\Program Files\\Synector\\Synector.exe"))
        );
        assert_eq!(
            parse_gui_exe_argument(&[
                "Synector.exe".to_string(),
                "--daemon".to_string()
            ]),
            None
        );
        assert_eq!(
            parse_gui_exe_argument(&[
                "Synector.exe".to_string(),
                "--daemon".to_string(),
                "--gui-exe".to_string(),
                String::new()
            ]),
            None,
            "an empty value is treated as unknown, not as an empty path"
        );
        assert_eq!(
            parse_gui_exe_argument(&[
                "Synector.exe".to_string(),
                "--daemon".to_string(),
                "--gui-exe".to_string()
            ]),
            None,
            "a missing value is ignored"
        );
    }

    #[test]
    fn test_shutdown_sequence_keeps_engine_first_and_flushes_last() {
        let order = std::cell::RefCell::new(Vec::new());
        run_shutdown_sequence(
            || order.borrow_mut().push("shutdown-rules"),
            || order.borrow_mut().push("overlays"),
            || order.borrow_mut().push("persist"),
        );
        assert_eq!(
            *order.borrow(),
            vec!["shutdown-rules", "overlays", "persist"],
            "overlays must fade out before the final runtime/overlay-position flush"
        );
    }
}
