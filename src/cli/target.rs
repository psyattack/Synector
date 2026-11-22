//! Daemon/offline target resolution and the per-command IPC round-trip adapters.

use serde_json::json;

use crate::config::{AppConfig, RulesFile};
use crate::ipc::{IpcClient, IpcRequest, IpcResponse};
use crate::models::Rule;

use super::commands::{print_deleted, print_report};
use super::json::{emit_json, fail_with, report_json};
use super::rules_io::ApplyReport;

/// Applies an upsert through the running daemon. Shared by the daemon-online path and the race
/// where the daemon starts after the CLI took its offline snapshot ([U13-003]).
pub(super) fn apply_via_daemon(
    rules: Vec<Rule>,
    report: &ApplyReport,
    json_output: bool,
    warnings: &[String],
) -> i32 {
    match IpcClient::call(&IpcRequest::ImportRules(rules)) {
        Ok(IpcResponse::Ok) => {
            if json_output {
                emit_json(report_json(true, "daemon", report, &[], false, warnings));
            } else {
                println!("Applied to the running daemon.");
                print_report(report);
            }
            0
        }
        Ok(IpcResponse::Error(err)) => {
            fail_with(json_output, &format!("daemon rejected the rules: {err}"))
        }
        Ok(other) => fail_with(json_output, &format!("unexpected daemon response: {other:?}")),
        Err(err) => fail_with(json_output, &format!("IPC error: {err}")),
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
    match IpcClient::call(&IpcRequest::SyncRules(rules)) {
        Ok(IpcResponse::Ok) => {
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
        Ok(IpcResponse::Error(err)) => {
            fail_with(json_output, &format!("daemon rejected the rules: {err}"))
        }
        Ok(other) => fail_with(json_output, &format!("unexpected daemon response: {other:?}")),
        Err(err) => fail_with(json_output, &format!("IPC error: {err}")),
    }
}

/// Sends one share import through the running daemon.
pub(super) fn import_share_via_daemon(
    payload: &crate::sharing::SharePayload,
    enable: bool,
    json_output: bool,
) -> Result<crate::sharing::ShareImportSummary, i32> {
    match IpcClient::call(&IpcRequest::ImportShare {
        payload: Box::new(payload.clone()),
        enable,
    }) {
        Ok(IpcResponse::ShareImportResult(summary)) => Ok(summary),
        Ok(IpcResponse::Error(err)) => Err(fail_with(
            json_output,
            &format!("daemon rejected the share code: {err}"),
        )),
        Ok(other) => Err(fail_with(
            json_output,
            &format!("unexpected daemon response: {other:?}"),
        )),
        Err(err) => Err(fail_with(json_output, &format!("IPC error: {err}"))),
    }
}

/// Deletes one rule through the running daemon, checking that it exists first.
pub(super) fn delete_via_daemon(id: &str, json_output: bool) -> i32 {
    match daemon_config() {
        Ok(cfg) if !cfg.rules.iter().any(|rule| rule.id == id) => {
            fail_with(json_output, &format!("rule \"{id}\" not found"))
        }
        Ok(_) => match IpcClient::call(&IpcRequest::DeleteRule(id.to_string())) {
            Ok(IpcResponse::Ok) => {
                if json_output {
                    emit_json(json!({ "ok": true, "id": id, "target": "daemon" }));
                } else {
                    println!("Deleted rule \"{id}\"");
                }
                0
            }
            Ok(IpcResponse::Error(err)) => fail_with(json_output, &err),
            Ok(other) => fail_with(json_output, &format!("unexpected daemon response: {other:?}")),
            Err(err) => fail_with(json_output, &format!("IPC error: {err}")),
        },
        Err(err) => fail_with(
            json_output,
            &format!("the daemon became reachable but could not be used; nothing was written: {err}"),
        ),
    }
}

pub(super) fn toggle_via_daemon(id: &str, enabled: bool, json_output: bool) -> i32 {
    match IpcClient::call(&IpcRequest::ToggleRule {
        id: id.to_string(),
        enabled,
    }) {
        Ok(IpcResponse::Ok) => {
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
        Ok(IpcResponse::Error(err)) => fail_with(json_output, &err),
        Ok(other) => fail_with(json_output, &format!("unexpected daemon response: {other:?}")),
        Err(err) => fail_with(json_output, &format!("IPC error: {err}")),
    }
}

pub(super) fn reset_via_daemon(id: &str, json_output: bool) -> i32 {
    match IpcClient::call(&IpcRequest::ResetRule(id.to_string())) {
        Ok(IpcResponse::Ok) => {
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
        Ok(IpcResponse::Error(err)) => fail_with(json_output, &err),
        Ok(other) => fail_with(json_output, &format!("unexpected daemon response: {other:?}")),
        Err(err) => fail_with(json_output, &format!("IPC error: {err}")),
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

pub(super) fn config_source(from_daemon: bool) -> String {
    if from_daemon {
        "the running daemon".to_string()
    } else {
        RulesFile::path().display().to_string()
    }
}
