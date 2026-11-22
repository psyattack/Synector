//! Background-daemon start-up: the serialized spawn guard, masked-image preparation and the pipe
//! wait that proves the daemon is ready before the GUI connects.
//!
//! The child argument list is written here — `--daemon --spawned-by-gui [--gui-exe <path>]
//! [--no-elevate]` — and is checked against the shared runtime vocabulary by the tests.

use std::env;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use crate::ipc::IpcClient;

/// How long a freshly spawned daemon gets to start serving the named pipe.
const DAEMON_START_TIMEOUT: Duration = Duration::from_secs(2);
/// Interval between daemon pipe probes while a start-up is in flight.
const DAEMON_START_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Starts the background daemon when it is not already reachable.
///
/// `no_elevate` is the user's `--no-elevate` opt-out, computed once by the launch context: it
/// must cover the whole process tree, not just the GUI, so the child inherits it.
///
/// Check, masked-image preparation and spawn are serialized behind a short-lived named mutex, so
/// two concurrent launches (double-click, or the mask-switch worker racing start-up) cannot both
/// spawn a daemon. The child is inspected while the pipe is awaited, so a failed or immediately
/// exited daemon is reported instead of silently opening a disconnected GUI.
pub(crate) fn ensure_daemon_running(no_elevate: bool) -> Result<(), String> {
    if IpcClient::is_daemon_running() {
        crate::logging::debug("DAEMON", "Background daemon is already running");
        return Ok(());
    }

    let Some(_spawn_guard) = crate::single_instance::SingleInstanceGuard::acquire_daemon_spawn()
    else {
        // Another process is preparing or spawning the daemon: wait for its pipe instead.
        return wait_for_daemon_pipe(DAEMON_START_TIMEOUT).ok_or_else(|| {
            "Another Synector instance is starting the background daemon, but it did not become ready"
                .to_string()
        });
    };

    // The other starter may have finished between the first probe and taking the guard.
    if IpcClient::is_daemon_running() {
        return Ok(());
    }

    let mask_daemon_process = crate::config::Settings::read().mask_daemon_process;
    let exe_path = if mask_daemon_process {
        crate::daemon::pe_mask::prepare_masked_daemon().unwrap_or_else(|err| {
            crate::logging::warn(
                "DAEMON",
                &format!("Could not prepare masked daemon image: {err}"),
            );
            env::current_exe().unwrap_or_default()
        })
    } else {
        env::current_exe().unwrap_or_default()
    };

    if !exe_path.exists() {
        crate::logging::error(
            "DAEMON",
            &format!(
                "Daemon executable path does not exist: {}",
                exe_path.display()
            ),
        );
        return Err(format!(
            "Daemon executable path does not exist: {}",
            exe_path.display()
        ));
    }

    let mut cmd = Command::new(&exe_path);
    cmd.args(daemon_spawn_arguments(
        env::current_exe().ok().as_deref(),
        no_elevate,
    ));
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn().map_err(|err| {
        crate::logging::error(
            "DAEMON",
            &format!("Failed to spawn background daemon: {err}"),
        );
        format!("Failed to spawn the background daemon: {err}")
    })?;
    crate::logging::info(
        "DAEMON",
        &format!("Spawned background daemon: {}", exe_path.display()),
    );

    // Wait for the pipe to become available, but notice a child that already exited (lost
    // single-instance race, failed masked-image copy, config failure) instead of waiting out
    // the whole timeout.
    let deadline = Instant::now() + DAEMON_START_TIMEOUT;
    while Instant::now() < deadline {
        if IpcClient::is_daemon_running() {
            return Ok(());
        }
        if let Ok(Some(status)) = child.try_wait() {
            if IpcClient::is_daemon_running() {
                return Ok(());
            }
            let message = format!("The background daemon exited during start-up ({status})");
            crate::logging::error("DAEMON", &message);
            return Err(message);
        }
        thread::sleep(DAEMON_START_POLL_INTERVAL);
    }

    if IpcClient::is_daemon_running() {
        Ok(())
    } else {
        let message = "The background daemon did not become ready in time".to_string();
        crate::logging::warn("DAEMON", &message);
        Err(message)
    }
}

/// Arguments for the background daemon child.
///
/// `--gui-exe` hands the launching GUI executable to the daemon so the updater can replace the
/// real executable for a portable installation with process masking: the daemon then runs from
/// the masked copy, which is only a cache, and the GUI window may already be closed.
fn daemon_spawn_arguments(
    gui_exe: Option<&std::path::Path>,
    no_elevate: bool,
) -> Vec<std::ffi::OsString> {
    let mut arguments: Vec<std::ffi::OsString> = vec![
        crate::arg_spellings::DAEMON.into(),
        crate::arg_spellings::SPAWNED_BY_GUI.into(),
    ];
    if let Some(gui_exe) = gui_exe {
        arguments.push(crate::arg_spellings::GUI_EXE.into());
        arguments.push(gui_exe.as_os_str().to_owned());
    }
    // The user's --no-elevate opt-out must cover the whole process tree, not just the GUI.
    if no_elevate {
        arguments.push(crate::arg_spellings::NO_ELEVATE.into());
    }
    arguments
}

fn wait_for_daemon_pipe(timeout: Duration) -> Option<()> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if IpcClient::is_daemon_running() {
            return Some(());
        }
        thread::sleep(DAEMON_START_POLL_INTERVAL);
    }
    IpcClient::is_daemon_running().then_some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::args::{validate_runtime_arguments, LaunchContext};

    fn daemon_child_launch(gui_exe: Option<&std::path::Path>, no_elevate: bool) -> LaunchContext {
        let mut process_argv = vec!["Synector.exe".to_string()];
        process_argv.extend(
            daemon_spawn_arguments(gui_exe, no_elevate)
                .iter()
                .map(|argument| argument.to_string_lossy().into_owned()),
        );
        LaunchContext::from_process_args(process_argv)
    }

    #[test]
    fn test_daemon_spawn_arguments_pass_runtime_argument_validation() {
        // The daemon child must never be rejected by the runtime check: it is spawned with
        // --daemon --spawned-by-gui [--gui-exe <path>] [--no-elevate].
        let gui_exe = std::path::Path::new("C:\\Program Files\\Synector\\Synector.exe");
        for no_elevate in [false, true] {
            let launch = daemon_child_launch(Some(gui_exe), no_elevate);
            assert_eq!(validate_runtime_arguments(launch.argv()), Ok(()));
            // The child lands on the daemon path with the opt-out propagated.
            assert!(launch.is_daemon());
            assert_eq!(launch.no_elevate(), no_elevate);
        }
        assert_eq!(
            validate_runtime_arguments(daemon_child_launch(None, false).argv()),
            Ok(())
        );
    }

    #[test]
    fn test_daemon_spawn_arguments_pass_the_launching_gui_exe() {
        let gui_exe = std::path::Path::new("C:\\Program Files\\Synector\\Synector.exe");
        let spawn = daemon_spawn_arguments(Some(gui_exe), false);
        assert_eq!(
            spawn,
            vec![
                std::ffi::OsString::from("--daemon"),
                std::ffi::OsString::from("--spawned-by-gui"),
                std::ffi::OsString::from("--gui-exe"),
                gui_exe.as_os_str().to_owned(),
            ]
        );

        // The daemon parser must recover exactly the path the GUI passed, spaces included.
        let daemon_argv: Vec<String> = spawn
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            crate::daemon::parse_gui_exe_argument(&daemon_argv).as_deref(),
            Some(gui_exe)
        );

        // --no-elevate still covers the daemon process and does not disturb the path.
        let spawn = daemon_spawn_arguments(Some(gui_exe), true);
        assert_eq!(spawn.last(), Some(&std::ffi::OsString::from("--no-elevate")));
        assert_eq!(spawn.get(2), Some(&std::ffi::OsString::from("--gui-exe")));

        // Without a resolvable GUI path no dangling empty --gui-exe value is emitted.
        assert_eq!(
            daemon_spawn_arguments(None, false),
            vec![
                std::ffi::OsString::from("--daemon"),
                std::ffi::OsString::from("--spawned-by-gui"),
            ]
        );
    }
}
