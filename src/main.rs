#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub mod autorun;
mod cli;
pub mod config;
mod daemon;
mod gui;
pub mod i18n;
mod ipc;
pub mod logging;
pub mod models;
pub mod sharing;
pub mod single_instance;
pub mod shortcut;
pub mod updater;

use std::env;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};
use ipc::IpcClient;

/// How long a freshly spawned daemon gets to start serving the named pipe.
const DAEMON_START_TIMEOUT: Duration = Duration::from_secs(2);
/// Interval between daemon pipe probes while a start-up is in flight.
const DAEMON_START_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Parsed `--run-rule` / `-r` request from the process arguments.
#[derive(Debug, PartialEq, Eq)]
enum RunRuleArgument {
    /// The flag is not present; normal start-up continues.
    Absent,
    /// A non-empty rule id to run.
    RuleId(String),
    /// The flag is present but its value is missing or empty; this is a usage error.
    MissingValue,
}

fn parse_run_rule_argument(args: &[String]) -> RunRuleArgument {
    let Some(position) = args.iter().position(|arg| arg == "--run-rule" || arg == "-r") else {
        return RunRuleArgument::Absent;
    };
    match args.get(position + 1) {
        Some(rule_id) if !rule_id.trim().is_empty() => RunRuleArgument::RuleId(rule_id.clone()),
        _ => RunRuleArgument::MissingValue,
    }
}

/// Whether a runtime flag carries a value in the next argument or is a plain switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeFlag {
    /// A flag without a value.
    Switch,
    /// A flag that takes the next argument as its value.
    Value,
}

impl RuntimeFlag {
    /// Classifies one runtime flag spelling. `None` means the argument is not a runtime flag.
    ///
    /// This is the surface that reaches the daemon/GUI dispatch. The standalone overlay fast path
    /// and the CLI rule commands parse and consume their own arguments before this point, so
    /// overlay-only companions (`--calibration`, `--duration`, `--override-text`, `--overlay-data`)
    /// are deliberately absent: they are not valid once the fast path has returned.
    fn from_spelling(flag: &str) -> Option<Self> {
        Some(match flag {
            "--gui" | "--daemon" | "-d" | "--service" | "-s" | "--status" | "--no-elevate"
            | "--spawned-by-gui" | "--help" | "-h" => Self::Switch,
            "--run-rule" | "-r" | "--overlay-window" | "-o" | "--gui-exe" => Self::Value,
            _ => return None,
        })
    }
}

/// A process argument that cannot be honored once the runtime dispatch is reached.
#[derive(Debug, PartialEq, Eq)]
enum RuntimeArgumentError {
    /// A `-`-prefixed argument matches no runtime flag.
    UnknownFlag(String),
    /// A value-taking flag is followed by nothing, an empty value or another flag.
    MissingValue(String),
}

impl std::fmt::Display for RuntimeArgumentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownFlag(flag) => write!(formatter, "unknown argument \"{flag}\""),
            Self::MissingValue(flag) => write!(formatter, "{flag} requires a non-empty value"),
        }
    }
}

/// Usage line for a malformed runtime invocation; the rule-authoring reference is `--help`.
const RUNTIME_USAGE: &str = "Usage: Synector.exe [--gui | --daemon] [--status] [--run-rule <rule id>] [--no-elevate] [--help]";

/// Validates the process arguments that reach the daemon/GUI dispatch.
///
/// The standalone overlay window and the CLI rule commands consume their own arguments and return
/// before this check, so only the runtime surface may appear here. `argv[0]` is the executable
/// path and is skipped; every later argument starting with `-` must be a known runtime flag, and a
/// value-taking flag must be followed by a non-empty value that is not itself a known flag. A value
/// such as a rule id may still start with a single `-`, so `--run-rule -custom-id` keeps working.
/// Positional arguments are not flags and are left to the dispatch.
///
/// [U13-001]: without this check a bare unknown flag fell through update-file cleanup, UAC
/// self-elevation and a daemon/GUI start, so a typo launched the elevated GUI instead of failing.
fn validate_runtime_arguments(args: &[String]) -> Result<(), RuntimeArgumentError> {
    let mut position = 1;
    while let Some(argument) = args.get(position) {
        if !argument.starts_with('-') {
            position += 1;
            continue;
        }
        match RuntimeFlag::from_spelling(argument) {
            Some(RuntimeFlag::Switch) => position += 1,
            Some(RuntimeFlag::Value) => match args.get(position + 1) {
                Some(value)
                    if !value.trim().is_empty() && RuntimeFlag::from_spelling(value).is_none() =>
                {
                    position += 2;
                }
                _ => return Err(RuntimeArgumentError::MissingValue(argument.clone())),
            },
            None => return Err(RuntimeArgumentError::UnknownFlag(argument.clone())),
        }
    }
    Ok(())
}

/// Reports a malformed runtime invocation on stderr and exits with the usage status.
fn report_runtime_usage_error(error: &RuntimeArgumentError) -> ! {
    // Release builds are GUI-subsystem executables; reattach the parent console so the usage error
    // is actually visible when a console launches the app with a malformed command line.
    crate::cli::ensure_console();
    crate::logging::error("APP", &format!("Invalid command line: {error}"));
    eprintln!("error: {error}");
    eprintln!("{RUNTIME_USAGE}");
    std::process::exit(2);
}

/// Quotes one argument so that an elevated re-exec (or a shortcut launch) reconstructs it exactly.
///
/// This is the `CommandLineToArgvW`/MSVC CRT convention: backslashes are only special in front of
/// a double quote, so they are doubled before quotes and before the closing quote; embedded quotes
/// are preceded by `2n + 1` backslashes. Empty arguments are preserved as `""`.
pub(crate) fn quote_windows_argument(argument: &str) -> String {
    if argument.is_empty() {
        return "\"\"".to_string();
    }
    if !argument.contains([' ', '\t', '"']) {
        return argument.to_string();
    }

    let mut quoted = String::with_capacity(argument.len() + 2);
    quoted.push('"');
    let mut backslashes = 0usize;
    for character in argument.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            other => {
                quoted.push_str(&"\\".repeat(backslashes));
                quoted.push(other);
                backslashes = 0;
            }
        }
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

fn build_elevation_arguments(args: &[String]) -> String {
    args.iter()
        .map(|argument| quote_windows_argument(argument))
        .collect::<Vec<_>>()
        .join(" ")
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
    let mut arguments: Vec<std::ffi::OsString> =
        vec!["--daemon".into(), "--spawned-by-gui".into()];
    if let Some(gui_exe) = gui_exe {
        arguments.push("--gui-exe".into());
        arguments.push(gui_exe.as_os_str().to_owned());
    }
    // The user's --no-elevate opt-out must cover the whole process tree, not just the GUI.
    if no_elevate {
        arguments.push("--no-elevate".into());
    }
    arguments
}

/// Surfaces a logging-init failure once.
///
/// `logging::init` already reports the first file-logging failure on stderr, but its return value
/// used to be discarded, so a degraded run stayed silent. A release GUI-subsystem build has no
/// console, so this explicit line is the diagnostic trace for a broken log directory.
fn report_logging_init(report: &logging::InitReport) {
    if let Some(error) = &report.error {
        eprintln!("[Synector] File logging is unavailable for this run: {error}");
    }
}

fn main() {
    let logging_report = logging::init();
    logging::install_panic_hook();
    report_logging_init(&logging_report);

    let args: Vec<String> = env::args().collect();

    // 1. Fast-path: standalone overlay window processes should NEVER touch update files or single instance mutexes
    if let Some(pos) = args.iter().position(|a| a == "--overlay-window" || a == "-o") {
        let overlay_id = args.get(pos + 1).cloned().unwrap_or_default();
        let calibration = args.iter().any(|a| a == "--calibration");
        let duration_ms = args.iter().position(|a| a == "--duration").and_then(|p| args.get(p + 1)).and_then(|s| s.parse::<u32>().ok());
        let override_text = args.iter().position(|a| a == "--override-text").and_then(|p| args.get(p + 1)).cloned();
        let overlay_data = args.iter().position(|a| a == "--overlay-data").and_then(|p| args.get(p + 1)).cloned();
        if let Err(e) = gui::overlay_window::run_overlay_window(overlay_id, calibration, override_text, duration_ms, overlay_data) {
            logging::error("OVERLAY", &format!("Failed to launch overlay window: {e}"));
            eprintln!("Failed to launch overlay window: {e}");
        }
        return;
    }

    // 2. Rule authoring commands are console tools: run them before elevation and update cleanup
    if cli::run(&args) {
        return;
    }

    // 3. Reject a mistyped or malformed runtime argument before any side effect: unknown flags must
    //    not fall through to update cleanup, UAC self-elevation and a daemon/GUI start ([U13-001]).
    if let Err(error) = validate_runtime_arguments(&args) {
        report_runtime_usage_error(&error);
    }

    // Clean up temporary update files ONLY if daemon is not running (to avoid deleting active downloads)
    if let Ok(exe_path) = env::current_exe() {
        let old = exe_path.with_extension("old");
        if old.exists() {
            let _ = std::fs::remove_file(old);
        }
    }
    if !IpcClient::is_daemon_running() {
        updater::clean_temp_update_files();
    }

    let is_daemon_flag = args.iter().any(|a| {
        a == "--daemon" || a == "-d" || a == "--service" || a == "-s"
    });

    if args.iter().any(|a| a == "--status") {
        match IpcClient::call(&ipc::IpcRequest::GetState) {
            Ok(ipc::IpcResponse::State(state)) => {
                println!("DAEMON STATUS: Online");
                println!("  Rules: {} (active: {})", state.total_rules_count, state.active_rules_count);
                println!("  Daemon Memory: {:.2} MB", state.daemon_memory_bytes as f64 / 1_048_576.0);
            }
            Ok(other) => println!("Daemon response: {:?}", other),
            Err(e) => println!("DAEMON STATUS: Offline ({e})"),
        }
        return;
    }

    // Direct rule execution via CLI or Windows shortcut: --run-rule <rule_id>
    match parse_run_rule_argument(&args) {
        RunRuleArgument::RuleId(rule_id) => {
            run_rule_cli(&rule_id);
            return;
        }
        RunRuleArgument::MissingValue => {
            eprintln!("Usage error: --run-rule requires a non-empty rule id.");
            std::process::exit(2);
        }
        RunRuleArgument::Absent => {}
    }

    // Auto-elevate to Administrator rights by default for system-wide ETW & UIPI input control
    let no_elevate = args.iter().any(|a| a == "--no-elevate");
    #[cfg(windows)]
    if !no_elevate && try_self_elevate() {
        logging::info("APP", "Not elevated: relaunching Synector as administrator");
        return;
    }

    if is_daemon_flag {
        daemon::run_daemon();
    } else {
        // Both default execution and --gui ensure background daemon is running before opening GUI
        if let Err(err) = ensure_daemon_running() {
            logging::error("DAEMON", &err);
            eprintln!("{err}");
        }
        if let Err(e) = gui::run_gui() {
            logging::error("GUI", &format!("Failed to launch GUI: {e}"));
            eprintln!("Failed to launch GUI: {e}");
        }
    }
}

/// Starts the background daemon when it is not already reachable.
///
/// Check, masked-image preparation and spawn are serialized behind a short-lived named mutex, so
/// two concurrent launches (double-click, or the mask-switch worker racing start-up) cannot both
/// spawn a daemon. The child is inspected while the pipe is awaited, so a failed or immediately
/// exited daemon is reported instead of silently opening a disconnected GUI.
pub fn ensure_daemon_running() -> Result<(), String> {
    if IpcClient::is_daemon_running() {
        logging::debug("DAEMON", "Background daemon is already running");
        return Ok(());
    }

    let Some(_spawn_guard) = single_instance::SingleInstanceGuard::acquire_daemon_spawn() else {
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

    let mask_daemon_process = config::Settings::read().mask_daemon_process;
    let exe_path = if mask_daemon_process {
        daemon::pe_mask::prepare_masked_daemon().unwrap_or_else(|err| {
            logging::warn(
                "DAEMON",
                &format!("Could not prepare masked daemon image: {err}"),
            );
            env::current_exe().unwrap_or_default()
        })
    } else {
        env::current_exe().unwrap_or_default()
    };

    if !exe_path.exists() {
        logging::error(
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
        std::env::args().any(|arg| arg == "--no-elevate"),
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
        logging::error(
            "DAEMON",
            &format!("Failed to spawn background daemon: {err}"),
        );
        format!("Failed to spawn the background daemon: {err}")
    })?;
    logging::info(
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
            logging::error("DAEMON", &message);
            return Err(message);
        }
        thread::sleep(DAEMON_START_POLL_INTERVAL);
    }

    if IpcClient::is_daemon_running() {
        Ok(())
    } else {
        let message = "The background daemon did not become ready in time".to_string();
        logging::warn("DAEMON", &message);
        Err(message)
    }
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

#[cfg(windows)]
fn try_self_elevate() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::SW_NORMAL;
    use windows::Win32::UI::Shell::{ShellExecuteExW, SHELLEXECUTEINFOW, SEE_MASK_NOASYNC};
    use windows::core::{HSTRING, PCWSTR};

    if daemon::watcher::is_elevated() {
        return false;
    }

    let exe_path = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return false,
    };

    let args: Vec<String> = std::env::args().skip(1).collect();
    let args_str = build_elevation_arguments(&args);

    let verb = HSTRING::from("runas");
    let file = HSTRING::from(exe_path.to_string_lossy().to_string());
    let params = HSTRING::from(args_str);

    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_NORMAL.0,
        ..Default::default()
    };

    unsafe {
        ShellExecuteExW(&mut info).is_ok()
    }
}

pub fn run_rule_cli(rule_id: &str) {
    logging::info("CLI", &format!("Direct rule run requested: {rule_id}"));

    // 1. Try sending via IPC if background daemon is already running
    if IpcClient::is_daemon_running() {
        if let Ok(ipc::IpcResponse::Ok) = IpcClient::call(&ipc::IpcRequest::TestRunRule {
            id: rule_id.to_string(),
            rule: None,
        }) {
            println!("Rule '{rule_id}' sent to running daemon for execution.");
            logging::info("CLI", &format!("Rule '{rule_id}' sent to the running daemon"));
            return;
        }
    }

    // 2. Autonomous execution when daemon is offline. The engine's shared runner applies the
    // same gates as the daemon's manual run (quota and preconditions; not the cooldown or
    // `is_enabled`), so the same rule behaves identically with and without a daemon.
    let cfg = config::AppConfig::read();
    let Some(rule) = cfg.rules.iter().find(|r| r.id == rule_id).cloned() else {
        eprintln!("Rule with ID '{rule_id}' was not found in configuration.");
        logging::warn(
            "CLI",
            &format!("Rule '{rule_id}' was not found in configuration"),
        );
        return;
    };
    println!("Running rule '{}' ({}) autonomously...", rule.name, rule.id);
    logging::info(
        "CLI",
        &format!("Running rule '{}' ({}) without a daemon", rule.name, rule.id),
    );
    match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => {
            let result = rt.block_on(daemon::engine::run_rule_offline(
                &rule,
                daemon::engine::RuleRunPolicy::MANUAL,
            ));
            report_offline_run(&rule, &result);
        }
        Err(err) => logging::error(
            "CLI",
            &format!("Failed to create Tokio runtime for the direct rule run: {err}"),
        ),
    }
    logging::info("CLI", &format!("Direct rule run finished: {rule_id}"));
}

/// Reports an autonomous `--run-rule` run. Action errors can embed script output, so the
/// console gets the detail while the daily file log records only the outcome — the same split
/// the daemon applies through its `SensitiveLog` event.
fn report_offline_run(rule: &models::Rule, result: &daemon::engine::OfflineRunResult) {
    if let Some(reason) = &result.skipped {
        eprintln!("{reason}");
        logging::warn(
            "CLI",
            &format!("Rule '{}' was not started: {reason}", rule.id),
        );
        return;
    }
    logging::debug(
        "CLI",
        &format!(
            "Rule '{}' offline run finished (halted: {}, tokens: {})",
            rule.id,
            result.halted,
            result.context.len()
        ),
    );
    if let Some((step, reason)) = &result.abort {
        println!("Pipeline gracefully halted at step {step}: {reason}");
        logging::info(
            "CLI",
            &format!(
                "Rule '{}' pipeline halted at step {}: {}",
                rule.id, step, reason
            ),
        );
    }
    for (step, error) in &result.failures {
        eprintln!("Action at step {step} failed: {error}");
        logging::error(
            "CLI",
            &format!(
                "Rule '{}' action at step {} failed (details omitted from the file log)",
                rule.id, step
            ),
        );
        if rule.stop_on_error {
            logging::warn(
                "CLI",
                &format!(
                    "Rule '{}' pipeline stopped at step {} due to error",
                    rule.id, step
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(raw: &[&str]) -> Vec<String> {
        raw.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn test_parse_run_rule_argument() {
        assert_eq!(parse_run_rule_argument(&args(&["Synector.exe"])), RunRuleArgument::Absent);
        assert_eq!(
            parse_run_rule_argument(&args(&["Synector.exe", "--run-rule", "rule_1"])),
            RunRuleArgument::RuleId("rule_1".into())
        );
        assert_eq!(
            parse_run_rule_argument(&args(&["Synector.exe", "-r", "rule_2"])),
            RunRuleArgument::RuleId("rule_2".into())
        );
        assert_eq!(
            parse_run_rule_argument(&args(&["Synector.exe", "--run-rule"])),
            RunRuleArgument::MissingValue
        );
        assert_eq!(
            parse_run_rule_argument(&args(&["Synector.exe", "--run-rule", ""])),
            RunRuleArgument::MissingValue
        );
        assert_eq!(
            parse_run_rule_argument(&args(&["Synector.exe", "--run-rule", "   "])),
            RunRuleArgument::MissingValue
        );
    }

    #[test]
    fn test_validate_runtime_arguments_accepts_known_runtime_flags() {
        let valid = [
            args(&["Synector.exe"]),
            args(&["Synector.exe", "--gui"]),
            args(&["Synector.exe", "--daemon"]),
            args(&["Synector.exe", "-d"]),
            args(&["Synector.exe", "--service"]),
            args(&["Synector.exe", "-s"]),
            args(&["Synector.exe", "--status"]),
            args(&["Synector.exe", "--no-elevate"]),
            args(&["Synector.exe", "--spawned-by-gui"]),
            args(&["Synector.exe", "--help"]),
            args(&["Synector.exe", "-h"]),
            args(&["Synector.exe", "--run-rule", "rule_1"]),
            args(&["Synector.exe", "-r", "rule_1"]),
            // A rule id may legally start with a dash; only a known flag is refused as a value.
            args(&["Synector.exe", "--run-rule", "-custom-rule-id"]),
            args(&["Synector.exe", "--overlay-window", "overlay_1"]),
            args(&["Synector.exe", "-o", "overlay_1"]),
            args(&["Synector.exe", "--gui-exe", "C:\\Program Files\\Synector\\Synector.exe"]),
            args(&["Synector.exe", "--gui", "--no-elevate", "--spawned-by-gui"]),
        ];
        for case in valid {
            assert_eq!(validate_runtime_arguments(&case), Ok(()), "{case:?} must be accepted");
        }
    }

    #[test]
    fn test_daemon_spawn_arguments_pass_runtime_argument_validation() {
        // The daemon child must never be rejected by the new check: it is spawned with
        // --daemon --spawned-by-gui [--gui-exe <path>] [--no-elevate].
        let gui_exe = std::path::Path::new("C:\\Program Files\\Synector\\Synector.exe");
        let mut daemon_argv = args(&["Synector.exe"]);
        daemon_argv.extend(
            daemon_spawn_arguments(Some(gui_exe), true)
                .iter()
                .map(|argument| argument.to_string_lossy().into_owned()),
        );
        assert_eq!(validate_runtime_arguments(&daemon_argv), Ok(()));
    }

    #[test]
    fn test_validate_runtime_arguments_rejects_unknown_flags() {
        assert_eq!(
            validate_runtime_arguments(&args(&["Synector.exe", "--bogus"])),
            Err(RuntimeArgumentError::UnknownFlag("--bogus".into()))
        );
        assert_eq!(
            validate_runtime_arguments(&args(&["Synector.exe", "--gui", "--dry-runn"])),
            Err(RuntimeArgumentError::UnknownFlag("--dry-runn".into())),
            "an unknown flag next to a known one must still be rejected"
        );
        assert_eq!(
            validate_runtime_arguments(&args(&["Synector.exe", "-x"])),
            Err(RuntimeArgumentError::UnknownFlag("-x".into()))
        );
        // Overlay-only companions are not valid once the overlay fast path has returned.
        assert_eq!(
            validate_runtime_arguments(&args(&["Synector.exe", "--calibration"])),
            Err(RuntimeArgumentError::UnknownFlag("--calibration".into()))
        );
    }

    #[test]
    fn test_validate_runtime_arguments_rejects_missing_values() {
        for flag in ["--run-rule", "-r", "--gui-exe", "--overlay-window", "-o"] {
            assert_eq!(
                validate_runtime_arguments(&args(&["Synector.exe", flag])),
                Err(RuntimeArgumentError::MissingValue(flag.into())),
                "{flag} without a value"
            );
            assert_eq!(
                validate_runtime_arguments(&args(&["Synector.exe", flag, ""])),
                Err(RuntimeArgumentError::MissingValue(flag.into())),
                "{flag} with an empty value"
            );
        }
        assert_eq!(
            validate_runtime_arguments(&args(&["Synector.exe", "--run-rule", "   "])),
            Err(RuntimeArgumentError::MissingValue("--run-rule".into()))
        );
        // A following known flag is not a value: `--run-rule --gui` must not try to run a rule
        // literally named `--gui`.
        assert_eq!(
            validate_runtime_arguments(&args(&["Synector.exe", "--run-rule", "--gui"])),
            Err(RuntimeArgumentError::MissingValue("--run-rule".into()))
        );
        // A following flag is not a value: `--gui-exe --daemon` must not treat `--daemon` as a
        // path and then open the GUI.
        assert_eq!(
            validate_runtime_arguments(&args(&["Synector.exe", "--gui-exe", "--daemon"])),
            Err(RuntimeArgumentError::MissingValue("--gui-exe".into()))
        );
    }

    #[test]
    fn test_runtime_argument_error_messages() {
        assert_eq!(
            RuntimeArgumentError::UnknownFlag("--bogus".into()).to_string(),
            "unknown argument \"--bogus\""
        );
        assert_eq!(
            RuntimeArgumentError::MissingValue("--run-rule".into()).to_string(),
            "--run-rule requires a non-empty value"
        );
    }

    #[test]
    fn test_quote_windows_argument_round_trips_through_command_line_to_argv() {
        let cases = [
            "",
            "plain",
            "with space",
            "tab\tinside",
            "leading and trailing ",
            "trailing\\",
            "C:\\Program Files\\Synector\\",
            "embedded\"quote",
            "quote\" and space",
            "backslashes\\\\before\"quote",
            "'apostrophe'; Start-Process calc",
            "$\"nested\" $(quote)",
        ];
        for case in cases {
            let command_line = quote_windows_argument(case);
            assert_eq!(
                parse_arguments(&command_line),
                vec![case.to_string()],
                "argument {case:?} must round-trip through {command_line:?}"
            );
        }
    }

    #[test]
    fn test_build_elevation_arguments_keeps_empty_arguments() {
        let command_line = build_elevation_arguments(&args(&["--gui", "", "a b"]));
        assert_eq!(command_line, "--gui \"\" \"a b\"");
        assert_eq!(parse_arguments(&command_line), args(&["--gui", "", "a b"]));
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

    #[test]
    fn test_logging_init_report_is_surfaced_without_panicking() {
        // A healthy report prints nothing; a failed one prints one diagnostic line. Both must be
        // side-effect free for the rest of start-up.
        report_logging_init(&logging::InitReport { error: None });
        report_logging_init(&logging::InitReport {
            error: Some("logs directory is not writable".to_string()),
        });
    }

    /// Parses the arguments after the program name, which is how ShellExecuteExW and shortcut
    /// launchers deliver our parameters. `CommandLineToArgvW` parses the first token differently,
    /// so the helper must not stand in for argv[0].
    fn parse_arguments(command_line: &str) -> Vec<String> {
        let mut parsed = parse_command_line_to_argv(&format!("Synector.exe {command_line}"));
        if !parsed.is_empty() {
            parsed.remove(0);
        }
        parsed
    }

    /// Parses a command line with the same Win32 parser that reconstructs the elevated argv.
    fn parse_command_line_to_argv(command_line: &str) -> Vec<String> {
        use windows::core::{HSTRING, PCWSTR};
        use windows::Win32::Foundation::{LocalFree, HLOCAL};
        use windows::Win32::UI::Shell::CommandLineToArgvW;

        let command_line = HSTRING::from(command_line);
        unsafe {
            let mut count = 0i32;
            let pointer = CommandLineToArgvW(PCWSTR(command_line.as_ptr()), &mut count);
            if pointer.is_null() || count <= 0 {
                return Vec::new();
            }
            let arguments = std::slice::from_raw_parts(pointer, count as usize)
                .iter()
                .map(|argument| {
                    let mut length = 0usize;
                    while *argument.0.add(length) != 0 {
                        length += 1;
                    }
                    String::from_utf16_lossy(std::slice::from_raw_parts(argument.0, length))
                })
                .collect();
            let _ = LocalFree(Some(HLOCAL(pointer as *mut _)));
            arguments
        }
    }
}
