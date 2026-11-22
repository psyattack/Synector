//! Daemon/offline target resolution, the shared IPC round-trip adapter and the offline/daemon
//! race guard.

use serde_json::json;

use crate::config::{AppConfig, RulesFile};
use crate::ipc::{IpcClient, IpcRequest, IpcResponse};
use crate::models::Rule;

use super::commands::{print_deleted, print_report};
use super::json::{emit_json, fail_with, report_json};
use super::rules_io::ApplyReport;

/// One IPC round-trip for every daemon path (F6).
///
/// All six adapters share the same four response arms; only the success handling and the
/// rejection label are command-specific:
///
/// - `Ok(Error(err))` fails with `"{reject_prefix}{err}"` (an empty prefix shows the daemon error
///   verbatim, as delete/enable/disable/reset always did),
/// - the command's expected response is accepted by `accept`,
/// - any other `Ok(_)` fails with `"unexpected daemon response: …"`,
/// - `Err(err)` fails with `"IPC error: …"`.
///
/// `accept` borrows the response so the helper can still name it when it is unexpected; it owns
/// the only `fail_with` calls and every daemon path prints the same texts and exits 1.
fn round_trip<T>(
    request: IpcRequest,
    reject_prefix: &str,
    json_output: bool,
    accept: impl FnOnce(&IpcResponse) -> Option<T>,
) -> Result<T, i32> {
    match IpcClient::call(&request) {
        Ok(IpcResponse::Error(err)) => Err(fail_with(
            json_output,
            &format!("{reject_prefix}{err}"),
        )),
        Ok(response) => match accept(&response) {
            Some(value) => Ok(value),
            None => Err(fail_with(
                json_output,
                &format!("unexpected daemon response: {response:?}"),
            )),
        },
        Err(err) => Err(fail_with(json_output, &format!("IPC error: {err}"))),
    }
}

/// The bare acknowledgement five commands expect. Any other response stays unexpected to
/// [`round_trip`].
fn accept_acknowledgement(response: &IpcResponse) -> Option<()> {
    matches!(response, IpcResponse::Ok).then_some(())
}

/// Applies an upsert through the running daemon. Shared by the daemon-online path and the race
/// where the daemon starts after the CLI took its offline snapshot ([U13-003]).
pub(super) fn apply_via_daemon(
    rules: Vec<Rule>,
    report: &ApplyReport,
    json_output: bool,
    warnings: &[String],
) -> i32 {
    match round_trip(
        IpcRequest::ImportRules(rules),
        "daemon rejected the rules: ",
        json_output,
        accept_acknowledgement,
    ) {
        Ok(()) => {
            if json_output {
                emit_json(report_json(true, "daemon", report, &[], false, warnings));
            } else {
                println!("Applied to the running daemon.");
                print_report(report);
            }
            0
        }
        Err(exit_code) => exit_code,
    }
}

/// Replaces the rule set through the running daemon as one declarative request: it persists once
/// and reloads hotkeys once instead of an upsert followed by a delete per rule ([U13-003]).
pub(super) fn sync_via_daemon(
    rules: Vec<Rule>,
    report: &ApplyReport,
    deleted: &[String],
    json_output: bool,
    warnings: &[String],
) -> i32 {
    match round_trip(
        IpcRequest::SyncRules(rules),
        "daemon rejected the rules: ",
        json_output,
        accept_acknowledgement,
    ) {
        Ok(()) => {
            if json_output {
                emit_json(report_json(
                    true, "daemon", report, deleted, false, warnings,
                ));
            } else {
                println!("Synced to the running daemon.");
                print_report(report);
                print_deleted(deleted);
            }
            0
        }
        Err(exit_code) => exit_code,
    }
}

/// Sends one share import through the running daemon.
pub(super) fn import_share_via_daemon(
    payload: &crate::sharing::SharePayload,
    enable: bool,
    json_output: bool,
) -> Result<crate::sharing::ShareImportSummary, i32> {
    round_trip(
        IpcRequest::ImportShare {
            payload: Box::new(payload.clone()),
            enable,
        },
        "daemon rejected the share code: ",
        json_output,
        |response| match response {
            IpcResponse::ShareImportResult(summary) => Some(summary.clone()),
            _ => None,
        },
    )
}

/// Deletes one rule through the running daemon, checking that it exists first.
pub(super) fn delete_via_daemon(id: &str, json_output: bool) -> i32 {
    match daemon_config() {
        Ok(cfg) if !cfg.rules.iter().any(|rule| rule.id == id) => {
            fail_with(json_output, &format!("rule \"{id}\" not found"))
        }
        Ok(_) => match round_trip(
            IpcRequest::DeleteRule(id.to_string()),
            "",
            json_output,
            accept_acknowledgement,
        ) {
            Ok(()) => {
                if json_output {
                    emit_json(json!({ "ok": true, "id": id, "target": "daemon" }));
                } else {
                    println!("Deleted rule \"{id}\"");
                }
                0
            }
            Err(exit_code) => exit_code,
        },
        Err(err) => fail_with(
            json_output,
            &format!("the daemon became reachable but could not be used; nothing was written: {err}"),
        ),
    }
}

pub(super) fn toggle_via_daemon(id: &str, enabled: bool, json_output: bool) -> i32 {
    match round_trip(
        IpcRequest::ToggleRule {
            id: id.to_string(),
            enabled,
        },
        "",
        json_output,
        accept_acknowledgement,
    ) {
        Ok(()) => {
            if json_output {
                emit_json(json!({
                    "ok": true,
                    "id": id,
                    "is_enabled": enabled,
                    "target": "daemon",
                }));
            } else {
                println!(
                    "Rule \"{id}\" is now {}.",
                    if enabled { "enabled" } else { "disabled" }
                );
            }
            0
        }
        Err(exit_code) => exit_code,
    }
}

pub(super) fn reset_via_daemon(id: &str, json_output: bool) -> i32 {
    match round_trip(
        IpcRequest::ResetRule(id.to_string()),
        "",
        json_output,
        accept_acknowledgement,
    ) {
        Ok(()) => {
            if json_output {
                emit_json(json!({
                    "ok": true,
                    "id": id,
                    "execution_count": 0,
                    "is_enabled": true,
                    "target": "daemon",
                }));
            } else {
                println!("Reset rule \"{id}\": execution counter cleared, rule enabled.");
            }
            0
        }
        Err(exit_code) => exit_code,
    }
}

/// Reads the split config sections without the side effects of [`AppConfig::load`]
/// (file creation and autostart registration).
pub(super) fn load_config_for_cli() -> AppConfig {
    AppConfig::read()
}

pub(super) fn daemon_config() -> Result<AppConfig, String> {
    match IpcClient::call(&IpcRequest::GetState) {
        Ok(IpcResponse::State(state)) => Ok(state.config),
        Ok(IpcResponse::Error(err)) => Err(format!("daemon error: {err}")),
        Ok(other) => Err(format!("unexpected daemon response: {other:?}")),
        Err(err) => Err(format!("IPC error: {err}")),
    }
}

/// Returns the current rules plus whether they came from the running daemon.
pub(super) fn current_config() -> Result<(AppConfig, bool), String> {
    if IpcClient::is_daemon_running() {
        Ok((daemon_config()?, true))
    } else {
        Ok((load_config_for_cli(), false))
    }
}

/// The offline/daemon race guard ([U13-003]): the daemon became reachable after the command took
/// its offline snapshot and now owns the rule set and runtime counters, so rewriting the config
/// sections under it would overwrite it. Callers route the change through IPC instead of writing
/// offline; a caller that has already written one section calls this immediately before the next
/// write, because a daemon that started in between would be overwritten by that whole-section
/// rewrite. Exactly one `is_daemon_running` probe is made per call, at the same point the
/// copy-pasted guards probed.
pub(super) fn daemon_took_over_after_snapshot() -> bool {
    IpcClient::is_daemon_running()
}

pub(super) fn config_source(from_daemon: bool) -> String {
    if from_daemon {
        "the running daemon".to_string()
    } else {
        RulesFile::path().display().to_string()
    }
}
