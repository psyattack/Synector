//! The frozen process dispatch sequence.
//!
//! The order is a product contract (AGENTS.md "Execution model"): logging and the panic hook →
//! standalone overlay fast path → CLI rule commands → runtime argument validation → update-file
//! cleanup → `--status` → `--run-rule` → self-elevation → daemon or GUI. Every stage that handles
//! the invocation returns instead of falling through, so no later stage can run as a side effect
//! of an earlier one.
//!
//! [U13-001]: runtime validation sits before update cleanup, elevation and the daemon/GUI start
//! so a typo cannot launch the elevated GUI.

use crate::app::args::{
    parse_run_rule_argument, validate_runtime_arguments, Argument, LaunchContext, RunRuleArgument,
    RuntimeArgumentError, RUNTIME_USAGE,
};
use crate::app::{cleanup, console, run_rule, spawn, status};

/// Runs logging start-up and then the frozen dispatch stages.
pub(crate) fn run() {
    let logging_report = crate::logging::init();
    crate::logging::install_panic_hook();
    report_logging_init(&logging_report);

    let launch = LaunchContext::from_env();

    // 1. Fast-path: standalone overlay window processes should NEVER touch update files or single
    //    instance mutexes.
    if let Some(overlay) = launch.overlay() {
        if let Err(e) = crate::gui::overlay_window::run_overlay_window(
            overlay.overlay_id,
            overlay.calibration,
            overlay.override_text,
            overlay.duration_ms,
            overlay.overlay_data,
        ) {
            crate::logging::error("OVERLAY", &format!("Failed to launch overlay window: {e}"));
            eprintln!("Failed to launch overlay window: {e}");
        }
        return;
    }

    // 2. Rule authoring commands are console tools: run them before elevation and update cleanup.
    if crate::cli::run(launch.argv()) {
        return;
    }

    // 3. Reject a mistyped or malformed runtime argument before any side effect: unknown flags must
    //    not fall through to update cleanup, UAC self-elevation and a daemon/GUI start ([U13-001]).
    if let Err(error) = validate_runtime_arguments(launch.argv()) {
        report_runtime_usage_error(&error);
    }

    // 4. Clean up update files: the `.old` executable unconditionally, temporary update files only
    //    while the daemon is offline (it may be downloading into one of them).
    cleanup::run_update_cleanup();

    // 5. One-shot daemon status report.
    if launch.contains(Argument::Status) {
        status::report_status();
        return;
    }

    // 6. Direct rule execution via a shortcut: --run-rule <rule_id>.
    match parse_run_rule_argument(launch.argv()) {
        RunRuleArgument::RuleId(rule_id) => {
            run_rule::run_rule_cli(&rule_id);
            return;
        }
        RunRuleArgument::MissingValue => {
            eprintln!("Usage error: --run-rule requires a non-empty rule id.");
            std::process::exit(2);
        }
        RunRuleArgument::Absent => {}
    }

    // 7. Auto-elevate to Administrator rights by default for system-wide ETW & UIPI input control.
    #[cfg(windows)]
    if !launch.no_elevate() && crate::app::elevation::try_self_elevate(&launch) {
        crate::logging::info("APP", "Not elevated: relaunching Synector as administrator");
        return;
    }

    if launch.is_daemon() {
        crate::daemon::run_daemon();
    } else {
        // Both default execution and --gui ensure background daemon is running before opening GUI.
        if let Err(err) = spawn::ensure_daemon_running(launch.no_elevate()) {
            crate::logging::error("DAEMON", &err);
            eprintln!("{err}");
        }
        if let Err(e) = crate::gui::run_gui() {
            crate::logging::error("GUI", &format!("Failed to launch GUI: {e}"));
            eprintln!("Failed to launch GUI: {e}");
        }
    }
}

/// Reports a malformed runtime invocation on stderr and exits with the usage status.
fn report_runtime_usage_error(error: &RuntimeArgumentError) -> ! {
    // Release builds are GUI-subsystem executables; reattach the parent console so the usage error
    // is actually visible when a console launches the app with a malformed command line.
    console::ensure_console();
    crate::logging::error("APP", &format!("Invalid command line: {error}"));
    eprintln!("error: {error}");
    eprintln!("{RUNTIME_USAGE}");
    std::process::exit(2);
}

/// Surfaces a logging-init failure once.
///
/// `logging::init` already reports the first file-logging failure on stderr, but its return value
/// used to be discarded, so a degraded run stayed silent. A release GUI-subsystem build has no
/// console, so this explicit line is the diagnostic trace for a broken log directory.
fn report_logging_init(report: &crate::logging::InitReport) {
    if let Some(error) = &report.error {
        eprintln!("[Synector] File logging is unavailable for this run: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_logging_init_report_is_surfaced_without_panicking() {
        // A healthy report prints nothing; a failed one prints one diagnostic line. Both must be
        // side-effect free for the rest of start-up.
        report_logging_init(&crate::logging::InitReport { error: None });
        report_logging_init(&crate::logging::InitReport {
            error: Some("logs directory is not writable".to_string()),
        });
    }
}
