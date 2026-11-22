//! Direct `--run-rule` execution: the daemon IPC path and the autonomous offline fallback.

use crate::daemon::engine::{run_rule_offline, OfflineRunResult, RuleRunPolicy};
use crate::ipc::{IpcClient, IpcRequest, IpcResponse};
use crate::models::Rule;

pub(crate) fn run_rule_cli(rule_id: &str) {
    crate::logging::info("CLI", &format!("Direct rule run requested: {rule_id}"));

    // 1. Try sending via IPC if background daemon is already running
    if IpcClient::is_daemon_running() {
        if let Ok(IpcResponse::Ok) = IpcClient::call(&IpcRequest::TestRunRule {
            id: rule_id.to_string(),
            rule: None,
        }) {
            println!("Rule '{rule_id}' sent to running daemon for execution.");
            crate::logging::info("CLI", &format!("Rule '{rule_id}' sent to the running daemon"));
            return;
        }
    }

    // 2. Autonomous execution when daemon is offline. The engine's shared runner applies the
    // same gates as the daemon's manual run (quota and preconditions; not the cooldown or
    // `is_enabled`), so the same rule behaves identically with and without a daemon.
    let cfg = crate::config::AppConfig::read();
    let Some(rule) = cfg.rules.iter().find(|r| r.id == rule_id).cloned() else {
        eprintln!("Rule with ID '{rule_id}' was not found in configuration.");
        crate::logging::warn(
            "CLI",
            &format!("Rule '{rule_id}' was not found in configuration"),
        );
        return;
    };
    println!("Running rule '{}' ({}) autonomously...", rule.name, rule.id);
    crate::logging::info(
        "CLI",
        &format!("Running rule '{}' ({}) without a daemon", rule.name, rule.id),
    );
    match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => {
            let result = rt.block_on(run_rule_offline(&rule, RuleRunPolicy::MANUAL));
            report_offline_run(&rule, &result);
        }
        Err(err) => crate::logging::error(
            "CLI",
            &format!("Failed to create Tokio runtime for the direct rule run: {err}"),
        ),
    }
    crate::logging::info("CLI", &format!("Direct rule run finished: {rule_id}"));
}

/// Reports an autonomous `--run-rule` run. Action errors can embed script output, so the
/// console gets the detail while the daily file log records only the outcome — the same split
/// the daemon applies through its `SensitiveLog` event.
fn report_offline_run(rule: &Rule, result: &OfflineRunResult) {
    if let Some(reason) = &result.skipped {
        eprintln!("{reason}");
        crate::logging::warn(
            "CLI",
            &format!("Rule '{}' was not started: {reason}", rule.id),
        );
        return;
    }
    crate::logging::debug(
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
        crate::logging::info(
            "CLI",
            &format!(
                "Rule '{}' pipeline halted at step {}: {}",
                rule.id, step, reason
            ),
        );
    }
    for (step, error) in &result.failures {
        eprintln!("Action at step {step} failed: {error}");
        crate::logging::error(
            "CLI",
            &format!(
                "Rule '{}' action at step {} failed (details omitted from the file log)",
                rule.id, step
            ),
        );
        if rule.stop_on_error {
            crate::logging::warn(
                "CLI",
                &format!(
                    "Rule '{}' pipeline stopped at step {} due to error",
                    rule.id, step
                ),
            );
        }
    }
}
