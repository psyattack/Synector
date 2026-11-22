//! Command dispatch and the command bodies for validate, apply, sync, list, export, delete,
//! enable/disable and reset.

use std::path::Path;

use serde_json::json;

use crate::config::RulesFile;
use crate::ipc::IpcClient;
use crate::models::{QuotaPeriod, Rule, Trigger};
use crate::validation::{rule_warnings, validate_rules};

use super::args::{has_flag, parse_invocation, Command, Invocation, USAGE};
use super::discovery::{cmd_reference, cmd_schema, cmd_template};
use super::json::{
    emit_error_json, emit_json, error_json, fail_validation, fail_with, log_errors, log_warnings,
    report_json,
};
use super::rules_io::{
    classify_rules, forget_runtime_counters, parse_rules_json, parse_rules_json_for_sync, plan_sync,
    read_input, reset_redefined_quota_runtime, save_rules, take_rule, upsert_rules,
    write_text_atomic, ApplyReport,
};
use super::share::{cmd_import_share, cmd_share};
use super::target::{
    apply_via_daemon, config_source, current_config, daemon_config, delete_via_daemon,
    load_config_for_cli, reset_via_daemon, sync_via_daemon, toggle_via_daemon,
};

const EXAMPLE_RULES_JSON: &str = r#"[
  {
    "id": "rule_open_notepad",
    "name": "Open Notepad",
    "description": "Created from a JSON rule file",
    "trigger": {
      "$trigger_type": "hotkey",
      "key_combination": "CTRL+ALT+N"
    },
    "actions": [
      {
        "$action_type": "start_process",
        "file_path": "C:\\Windows\\System32\\notepad.exe",
        "only_if_not_running": true
      },
      {
        "$action_type": "wait",
        "mode": "timer",
        "seconds": 1.0
      },
      {
        "$action_type": "notification",
        "title": "Synector",
        "message": "Notepad is running"
      }
    ]
  }
]"#;

pub(super) fn dispatch(args: &[String]) -> i32 {
    let json_output = has_flag(args, "--json");
    let Invocation { command, value } = match resolve_invocation(args, json_output) {
        Ok(invocation) => invocation,
        Err(exit_code) => return exit_code,
    };
    let dry_run = has_flag(args, "--dry-run");
    let enable_imported = has_flag(args, "--enable-imported");
    let value = value.as_deref().unwrap_or_default();

    match command {
        Command::Help => {
            print!("{USAGE}");
            0
        }
        Command::Example => {
            println!("{EXAMPLE_RULES_JSON}");
            0
        }
        Command::Schema => cmd_schema(),
        Command::Reference => cmd_reference(),
        Command::Template => cmd_template(Some(value).filter(|value| !value.is_empty())),
        Command::Validate => cmd_validate(value, json_output),
        Command::Apply => cmd_apply(value, json_output, dry_run),
        Command::Sync => cmd_sync(value, json_output, dry_run),
        Command::Delete => cmd_delete(value, json_output),
        Command::Enable => cmd_toggle(value, true, json_output),
        Command::Disable => cmd_toggle(value, false, json_output),
        Command::Reset => cmd_reset(value, json_output),
        Command::List => cmd_list(json_output),
        Command::Export => cmd_export(Some(value).filter(|value| !value.is_empty()), json_output),
        Command::Share => cmd_share(Some(value).filter(|value| !value.is_empty()), json_output),
        Command::ImportShare => cmd_import_share(value, json_output, enable_imported),
    }
}

/// Parses the invocation the way `dispatch` does, returning the usage exit code on rejection.
/// Split out so tests can exercise the real argument parser without executing a command.
pub(super) fn resolve_invocation(args: &[String], json_output: bool) -> Result<Invocation, i32> {
    parse_invocation(args).map_err(|message| usage_error(json_output, &message))
}

/// Prints a usage error with the command reference and returns the usage exit code ([U13-001]).
fn usage_error(json_output: bool, message: &str) -> i32 {
    crate::logging::error("CLI", &format!("usage: {message}"));
    if json_output {
        emit_error_json(message, &[message.to_string()], &[], None);
    }
    eprintln!("error: {message}");
    eprintln!("{USAGE}");
    2
}

fn cmd_validate(path: &str, json_output: bool) -> i32 {
    let raw = match read_input(path) {
        Ok(raw) => raw,
        Err(err) => return fail_with(json_output, &err),
    };
    let rules = match parse_rules_json(&raw) {
        Ok(rules) => rules,
        Err(err) => return fail_with(json_output, &format!("invalid rule file: {err}")),
    };

    let warnings = rule_warnings(&rules, &load_config_for_cli());
    let errors = validate_rules(&rules);
    log_warnings(&warnings);

    if json_output {
        if let Some(first) = errors.first() {
            emit_json(error_json(first, &errors, &warnings, None));
        } else {
            emit_json(json!({
                "ok": true,
                "rules": rules.len(),
                "errors": errors,
                "warnings": warnings,
            }));
        }
    } else {
        for warning in &warnings {
            eprintln!("warning: {warning}");
        }
        if !errors.is_empty() {
            for error in &errors {
                eprintln!("error: {error}");
            }
            log_errors(&errors);
            eprintln!("{} rule(s) checked, {} error(s)", rules.len(), errors.len());
        } else {
            println!("OK: {} rule(s) valid", rules.len());
            for rule in &rules {
                println!(
                    "  {:<28}  {}  ({} action(s), {})",
                    rule.id,
                    rule.name,
                    rule.actions.len(),
                    trigger_summary(&rule.trigger)
                );
            }
        }
    }

    if errors.is_empty() {
        0
    } else {
        1
    }
}

fn cmd_apply(path: &str, json_output: bool, dry_run: bool) -> i32 {
    let raw = match read_input(path) {
        Ok(raw) => raw,
        Err(err) => return fail_with(json_output, &err),
    };
    let rules = match parse_rules_json(&raw) {
        Ok(rules) => rules,
        Err(err) => return fail_with(json_output, &format!("invalid rule file: {err}")),
    };

    let (mut cfg, from_daemon) = match current_config() {
        Ok(value) => value,
        Err(err) => return fail_with(json_output, &err),
    };
    let warnings = rule_warnings(&rules, &cfg);
    let errors = validate_rules(&rules);
    log_warnings(&warnings);
    if !errors.is_empty() {
        log_errors(&errors);
        if json_output {
            return fail_validation(&errors, &warnings, target_name(from_daemon));
        }
        for warning in &warnings {
            eprintln!("warning: {warning}");
        }
        for error in &errors {
            eprintln!("error: {error}");
        }
        eprintln!("Nothing was applied.");
        return 1;
    }
    if !json_output {
        for warning in &warnings {
            eprintln!("warning: {warning}");
        }
    }

    let report = classify_rules(&cfg.rules, &rules);
    if dry_run {
        if json_output {
            emit_json(report_json(true, target_name(from_daemon), &report, &[], true, &warnings));
        } else {
            println!("Dry run: nothing was written.");
            print_report(&report);
        }
        return 0;
    }

    if from_daemon {
        return apply_via_daemon(rules, &report, json_output, &warnings);
    }

    let previous = cfg.rules.clone();
    if IpcClient::is_daemon_running() {
        // The daemon became reachable after the snapshot and now owns the rule set. Route the
        // change through IPC instead of rewriting rules.json under it ([U13-003]).
        return match daemon_config() {
            Ok(current) => {
                let report = classify_rules(&current.rules, &rules);
                apply_via_daemon(rules, &report, json_output, &warnings)
            }
            Err(err) => fail_with(
                json_output,
                &format!(
                    "the daemon started while the command was running; re-run it so the change goes through IPC: {err}"
                ),
            ),
        };
    }

    let report = upsert_rules(&mut cfg.rules, &rules);
    if let Err(err) = save_rules(&cfg.rules) {
        return fail_with(json_output, &err);
    }
    if let Err(err) = reset_redefined_quota_runtime(&previous, &rules) {
        return fail_with(
            json_output,
            &format!("rules were written but runtime counters could not be updated: {err}"),
        );
    }
    if json_output {
        emit_json(report_json(true, "rules", &report, &[], false, &warnings));
    } else {
        println!(
            "Daemon is offline: rules written to {}",
            RulesFile::path().display()
        );
        println!("They take effect on the next daemon start.");
        print_report(&report);
    }
    0
}

fn cmd_sync(path: &str, json_output: bool, dry_run: bool) -> i32 {
    let raw = match read_input(path) {
        Ok(raw) => raw,
        Err(err) => return fail_with(json_output, &err),
    };
    let rules = match parse_rules_json_for_sync(&raw) {
        Ok(rules) => rules,
        Err(err) => return fail_with(json_output, &format!("invalid rule file: {err}")),
    };

    let (mut cfg, from_daemon) = match current_config() {
        Ok(value) => value,
        Err(err) => return fail_with(json_output, &err),
    };
    let warnings = rule_warnings(&rules, &cfg);
    let errors = validate_rules(&rules);
    log_warnings(&warnings);
    if !errors.is_empty() {
        log_errors(&errors);
        if json_output {
            return fail_validation(&errors, &warnings, target_name(from_daemon));
        }
        for warning in &warnings {
            eprintln!("warning: {warning}");
        }
        for error in &errors {
            eprintln!("error: {error}");
        }
        eprintln!("Nothing was synced.");
        return 1;
    }
    if !json_output {
        for warning in &warnings {
            eprintln!("warning: {warning}");
        }
    }

    let previous = cfg.rules.clone();
    let (report, deleted) = plan_sync(&cfg.rules, &rules);
    if dry_run {
        if json_output {
            emit_json(report_json(
                true,
                target_name(from_daemon),
                &report,
                &deleted,
                true,
                &warnings,
            ));
        } else {
            println!("Dry run: nothing was written.");
            print_report(&report);
            print_deleted(&deleted);
        }
        return 0;
    }

    if from_daemon {
        return sync_via_daemon(rules, &report, &deleted, json_output, &warnings);
    }

    if IpcClient::is_daemon_running() {
        // The daemon became reachable after the snapshot: send the declarative replace over IPC
        // rather than overwriting rules.json under it ([U13-003]).
        return match daemon_config() {
            Ok(current) => {
                let (report, deleted) = plan_sync(&current.rules, &rules);
                sync_via_daemon(rules, &report, &deleted, json_output, &warnings)
            }
            Err(err) => fail_with(
                json_output,
                &format!(
                    "the daemon started while the command was running; re-run it so the change goes through IPC: {err}"
                ),
            ),
        };
    }

    cfg.rules = rules;
    if let Err(err) = save_rules(&cfg.rules) {
        return fail_with(json_output, &err);
    }
    // A removed rule must not keep counters that a later rule with the same id would inherit
    // ([U13-004]); `reset_redefined_quota_runtime` only touches surviving ids.
    let removed: Vec<&str> = deleted.iter().map(String::as_str).collect();
    if let Err(err) = forget_runtime_counters(&removed) {
        return fail_with(
            json_output,
            &format!("rules were synced but runtime counters could not be pruned: {err}"),
        );
    }
    if let Err(err) = reset_redefined_quota_runtime(&previous, &cfg.rules) {
        return fail_with(
            json_output,
            &format!("rules were synced but runtime counters could not be updated: {err}"),
        );
    }
    if json_output {
        emit_json(report_json(true, "rules", &report, &deleted, false, &warnings));
    } else {
        println!(
            "Daemon is offline: rules written to {}",
            RulesFile::path().display()
        );
        println!("They take effect on the next daemon start.");
        print_report(&report);
        print_deleted(&deleted);
    }
    0
}

fn cmd_list(json_output: bool) -> i32 {
    let (cfg, from_daemon) = match current_config() {
        Ok(value) => value,
        Err(err) => return fail_with(json_output, &err),
    };
    if json_output {
        return match serde_json::to_string_pretty(&cfg.rules) {
            Ok(json) => {
                println!("{json}");
                0
            }
            Err(err) => fail_with(true, &err.to_string()),
        };
    }

    println!("{} rule(s) from {}", cfg.rules.len(), config_source(from_daemon));
    let now = chrono::Local::now();
    for rule in &cfg.rules {
        // `quota_used` reports zero for a period that already rolled over, so the listing
        // matches what the daemon will allow even before it persists the reset.
        let used = rule.quota_used(now);
        let executions = match rule.max_executions {
            Some(max) => match rule.quota_period {
                Some(period) => format!("{}/{} ({})", used, max, quota_period_summary(period)),
                None => format!("{}/{}", used, max),
            },
            None => used.to_string(),
        };
        println!(
            "  {:<28}  {:<32}  {:<8}  {:>2} action(s)  exec {:<18}  {}",
            rule.id,
            truncate(&rule.name, 32),
            if rule.is_enabled { "enabled" } else { "disabled" },
            rule.actions.len(),
            executions,
            trigger_summary(&rule.trigger)
        );
    }
    0
}

/// Compact period description for `--rules-list`.
fn quota_period_summary(period: QuotaPeriod) -> String {
    match period {
        QuotaPeriod::Minute => "every minute".to_string(),
        QuotaPeriod::Hour => "hourly".to_string(),
        QuotaPeriod::Day => "daily".to_string(),
        QuotaPeriod::Week => "weekly".to_string(),
        QuotaPeriod::Month => "monthly".to_string(),
        QuotaPeriod::Custom { value, unit } => {
            let unit = match unit {
                crate::models::QuotaUnit::Minutes => "min",
                crate::models::QuotaUnit::Hours => "h",
                crate::models::QuotaUnit::Days => "d",
                crate::models::QuotaUnit::Weeks => "w",
                crate::models::QuotaUnit::Months => "mo",
            };
            format!("every {value}{unit}")
        }
    }
}

/// Exports all rules as JSON to stdout or to an atomic file write ([U13-013]).
fn cmd_export(output: Option<&str>, json_output: bool) -> i32 {
    let (cfg, _) = match current_config() {
        Ok(value) => value,
        Err(err) => return fail_with(json_output, &err),
    };
    let json = match serde_json::to_string_pretty(&cfg.rules) {
        Ok(json) => json,
        Err(err) => return fail_with(json_output, &err.to_string()),
    };

    match output.filter(|path| *path != "-") {
        Some(path) => match write_text_atomic(Path::new(path), &format!("{json}\n")) {
            Ok(()) => {
                println!("Exported {} rule(s) to {path}", cfg.rules.len());
                0
            }
            Err(err) => fail_with(json_output, &format!("cannot write {path}: {err}")),
        },
        None => {
            println!("{json}");
            0
        }
    }
}

fn cmd_delete(id: &str, json_output: bool) -> i32 {
    let id = id.trim();
    if id.is_empty() {
        return fail_with(json_output, "\"id\" is required");
    }

    if IpcClient::is_daemon_running() {
        return delete_via_daemon(id, json_output);
    }

    let mut cfg = load_config_for_cli();
    let Some(removed) = take_rule(&mut cfg.rules, id) else {
        return fail_with(json_output, &format!("rule \"{id}\" not found"));
    };
    if IpcClient::is_daemon_running() {
        // A daemon that started after the offline read owns runtime and rules now ([U13-003]).
        return delete_via_daemon(id, json_output);
    }
    if let Err(err) = save_rules(&cfg.rules) {
        return fail_with(json_output, &err);
    }
    // The id no longer exists in rules.json, so its counters must not survive to be inherited by
    // a later rule with the same id ([U13-004]).
    if let Err(err) = forget_runtime_counters(&[removed.id.as_str()]) {
        return fail_with(
            json_output,
            &format!("rule was deleted but runtime counters could not be pruned: {err}"),
        );
    }
    if json_output {
        emit_json(json!({ "ok": true, "id": id, "target": "rules" }));
    } else {
        println!(
            "Deleted rule \"{id}\" from {}",
            RulesFile::path().display()
        );
    }
    0
}

fn cmd_toggle(id: &str, enabled: bool, json_output: bool) -> i32 {
    let id = id.trim();
    if id.is_empty() {
        return fail_with(json_output, "\"id\" is required");
    }
    let (mut cfg, from_daemon) = match current_config() {
        Ok(value) => value,
        Err(err) => return fail_with(json_output, &err),
    };
    if let Err(err) = set_rule_enabled(&mut cfg.rules, id, enabled) {
        return fail_with(json_output, &err);
    }
    if from_daemon {
        return toggle_via_daemon(id, enabled, json_output);
    }
    if IpcClient::is_daemon_running() {
        // The daemon became reachable after the snapshot and would be overwritten here
        // ([U13-003]).
        return toggle_via_daemon(id, enabled, json_output);
    }
    if let Err(err) = save_rules(&cfg.rules) {
        return fail_with(json_output, &err);
    }
    if json_output {
        emit_json(json!({
            "ok": true,
            "id": id,
            "is_enabled": enabled,
            "target": "rules",
        }));
    } else {
        println!(
            "Rule \"{id}\" is now {}.",
            if enabled { "enabled" } else { "disabled" }
        );
    }
    0
}

fn cmd_reset(id: &str, json_output: bool) -> i32 {
    let id = id.trim();
    if id.is_empty() {
        return fail_with(json_output, "\"id\" is required");
    }
    let (mut cfg, from_daemon) = match current_config() {
        Ok(value) => value,
        Err(err) => return fail_with(json_output, &err),
    };
    if reset_rule_runtime(&mut cfg.rules, id).is_err() {
        return fail_with(json_output, &format!("rule \"{id}\" not found"));
    }
    if from_daemon {
        return reset_via_daemon(id, json_output);
    }
    if IpcClient::is_daemon_running() {
        // The daemon became reachable after the snapshot; it owns runtime counters ([U13-003]).
        return reset_via_daemon(id, json_output);
    }
    if let Err(err) = save_rules(&cfg.rules) {
        return fail_with(json_output, &err);
    }
    if let Err(err) = forget_runtime_counters(&[id]) {
        return fail_with(
            json_output,
            &format!("rule was reset but runtime counters could not be cleared: {err}"),
        );
    }
    if json_output {
        emit_json(json!({
            "ok": true,
            "id": id,
            "execution_count": 0,
            "is_enabled": true,
            "target": "rules",
        }));
    } else {
        println!("Reset rule \"{id}\": execution counter cleared, rule enabled.");
    }
    0
}

pub(super) fn print_report(report: &ApplyReport) {
    for id in &report.created {
        println!("  created  {id}");
    }
    for id in &report.updated {
        println!("  updated  {id}");
    }
    println!(
        "{} created, {} updated",
        report.created.len(),
        report.updated.len()
    );
}

pub(super) fn print_deleted(deleted: &[String]) {
    for id in deleted {
        println!("  deleted  {id}");
    }
    println!("{} deleted", deleted.len());
}

fn set_rule_enabled(rules: &mut [Rule], id: &str, enabled: bool) -> Result<(), String> {
    let rule = rules
        .iter_mut()
        .find(|rule| rule.id == id)
        .ok_or_else(|| format!("rule \"{id}\" not found"))?;
    rule.is_enabled = enabled;
    Ok(())
}

fn reset_rule_runtime<'a>(rules: &'a mut [Rule], id: &str) -> Result<&'a mut Rule, String> {
    let rule = rules
        .iter_mut()
        .find(|rule| rule.id == id)
        .ok_or_else(|| format!("rule \"{id}\" not found"))?;
    rule.clear_quota_runtime();
    rule.is_enabled = true;
    Ok(rule)
}

pub(super) fn target_name(from_daemon: bool) -> &'static str {
    if from_daemon {
        "daemon"
    } else {
        "rules"
    }
}

fn trigger_summary(trigger: &Option<Trigger>) -> String {
    match trigger {
        None => "no trigger".to_string(),
        Some(Trigger::Manual) => "manual".to_string(),
        Some(Trigger::Process { process_name, .. }) => format!("process:{process_name}"),
        Some(Trigger::WindowFocus { process_name, .. }) => format!("focus:{process_name}"),
        Some(Trigger::ResourceThreshold { process_name, .. }) => {
            format!("resource:{process_name}")
        }
        Some(Trigger::GlobalHotkey {
            key_combination, ..
        }) => format!("hotkey:{key_combination}"),
        Some(Trigger::SystemEvent { event_type, .. }) => format!("system:{event_type:?}"),
        Some(Trigger::Schedule { schedule_type, .. }) => format!("schedule:{schedule_type:?}"),
        Some(Trigger::FileWatcher { folder_path, .. }) => format!("files:{folder_path}"),
        Some(Trigger::Webhook { endpoint_path }) => format!("webhook:{endpoint_path}"),
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut truncated: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    truncated.push('…');
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::rules_io::rule_with_notification;

    #[test]
    fn test_quota_period_summary() {
        assert_eq!(quota_period_summary(QuotaPeriod::Minute), "every minute");
        assert_eq!(quota_period_summary(QuotaPeriod::Hour), "hourly");
        assert_eq!(quota_period_summary(QuotaPeriod::Day), "daily");
        assert_eq!(quota_period_summary(QuotaPeriod::Week), "weekly");
        assert_eq!(quota_period_summary(QuotaPeriod::Month), "monthly");
        assert_eq!(
            quota_period_summary(QuotaPeriod::Custom {
                value: 90,
                unit: crate::models::QuotaUnit::Minutes
            }),
            "every 90min"
        );
        assert_eq!(
            quota_period_summary(QuotaPeriod::Custom {
                value: 3,
                unit: crate::models::QuotaUnit::Months
            }),
            "every 3mo"
        );
    }

    #[test]
    fn test_example_rule_file_is_valid() {
        let rules = parse_rules_json(EXAMPLE_RULES_JSON).expect("example parses");
        assert_eq!(rules.len(), 1);
        assert!(validate_rules(&rules).is_empty(), "{:?}", validate_rules(&rules));
    }

    #[test]
    fn test_trigger_summary() {
        assert_eq!(trigger_summary(&None), "no trigger");
        assert_eq!(trigger_summary(&Some(Trigger::Manual)), "manual");
        assert_eq!(
            trigger_summary(&Some(Trigger::GlobalHotkey {
                key_combination: "CTRL+F1".to_string(),
                hold_mode: false,
                repeat_while_held: false,
            })),
            "hotkey:CTRL+F1"
        );
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("abcdef", 4), "abc…");
    }

    #[test]
    fn test_enable_disable_and_reset_runtime() {
        let mut rules = vec![rule_with_notification("x")];
        rules[0].execution_count = 5;
        rules[0].last_executed_at = Some("2026-01-01 00:00:00".to_string());
        rules[0].is_enabled = false;

        set_rule_enabled(&mut rules, "x", true).expect("enable");
        assert!(rules[0].is_enabled);
        set_rule_enabled(&mut rules, "x", false).expect("disable");
        assert!(!rules[0].is_enabled);
        assert!(set_rule_enabled(&mut rules, "missing", true).is_err());

        let rule = reset_rule_runtime(&mut rules, "x").expect("reset");
        assert_eq!(rule.execution_count, 0);
        assert!(rule.last_executed_at.is_none());
        assert!(rule.is_enabled);
        assert!(reset_rule_runtime(&mut rules, "missing").is_err());
    }
}
