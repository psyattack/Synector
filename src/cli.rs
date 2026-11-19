//! Command-line rule authoring: validate, apply, list, export and delete rules without the GUI.
//!
//! Rule files are plain JSON in the [`Rule`] serde shape: one rule object, an array of rules, or
//! `{"rules": [...]}`. Applying upserts by `id`; when the daemon is running the change goes
//! through IPC so it takes effect immediately, otherwise `rules.json` is edited directly.
//! Runtime counters stay in the daemon-owned `runtime.json`, never in rule files.

use std::io::Read;
use std::path::Path;

use serde_json::{json, Value};

use schemars::schema::{InstanceType, RootSchema, Schema, SchemaObject, SingleOrVec};

use crate::config::{AppConfig, RulesFile, RuntimeState};
use crate::ipc::{IpcClient, IpcRequest, IpcResponse};
use crate::models::{Action, QuotaPeriod, Rule, ScheduleType, Trigger};

pub const USAGE: &str = r#"Synector - rule authoring from the command line

  --rules-validate <file|->   Parse and validate a JSON rule file; no changes are made
  --rules-apply <file|->      Create or update rules from a JSON rule file (upsert by id)
  --rules-list [--json]       List configured rules and their triggers
  --rules-export [<file>]     Print all rules as JSON, optionally to a file
  --rules-share [<id>]        Print a compact SYN1: share code for one rule or all rules
  --rules-import-share <code|file|->  Import automation(s) from a SYN1: share code
  --rules-delete <id>         Delete one rule by id
  --rules-sync <file|->       Make the configured rules match the file exactly (upsert and delete missing)
  --rules-enable <id>         Enable one rule
  --rules-disable <id>        Disable one rule
  --rules-reset <id>          Reset a rule's execution counter and re-enable it
  --rules-example             Print an example rule file
  --rules-schema              Print the draft-07 JSON Schema for one rule object
  --rules-reference           Print a human-readable reference of every trigger, action and condition
  --rules-template <t[:a]>    Print a ready-to-edit rule skeleton for a trigger and action
  --json                      Machine-readable output for validate/apply/sync/delete/enable/disable/reset
  --dry-run                   With --rules-apply or --rules-sync: report changes without writing
  --enable-imported           With --rules-import-share: enable the imported rules (default: disabled)
  --help                      Print this help

A rule file is one rule object, an array of rules, or {"rules": [...]}. Every rule
needs an "id" (the upsert key) and a "name". Rules are applied to the running daemon
over IPC; without a daemon the same upsert is written to rules.json and takes effect
on the next daemon start. Runtime counters (execution_count, last_executed_at) live in
runtime.json and are preserved when an existing rule is updated.

Share codes are compact SYN1: strings that also carry the macro presets and overlays
the rules reference. Imported automations are disabled by default; pass
--enable-imported to activate them immediately. Codes are imported from the argument
itself, from a file, or from stdin with "-".

Use --rules-schema for machine-readable discovery (AI agents, IDE, validators) and
--rules-reference for a compact human-readable catalogue.
"#;

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

const KNOWN_FLAGS: &[&str] = &[
    "--rules-validate",
    "--rules-apply",
    "--rules-list",
    "--rules-export",
    "--rules-share",
    "--rules-import-share",
    "--rules-delete",
    "--rules-sync",
    "--rules-enable",
    "--rules-disable",
    "--rules-reset",
    "--rules-example",
    "--rules-schema",
    "--rules-reference",
    "--rules-template",
    "--json",
    "--dry-run",
    "--enable-imported",
    "--help",
    "-h",
];

/// Result of an upsert: which rule ids were created and which replaced an existing rule.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ApplyReport {
    pub created: Vec<String>,
    pub updated: Vec<String>,
}

/// Entry point from `main`. Returns `false` when the arguments are not a CLI authoring command.
pub fn run(args: &[String]) -> bool {
    if !args.iter().any(|arg| KNOWN_FLAGS.contains(&arg.as_str())) {
        return false;
    }
    ensure_console();
    let command = log_invocation(args);
    let exit_code = dispatch(args);
    if exit_code == 0 {
        crate::logging::info("CLI", &format!("{command} completed"));
    } else {
        crate::logging::error("CLI", &format!("{command} exited with code {exit_code}"));
    }
    std::process::exit(exit_code)
}

/// Records which command was requested. Only the flag and its path/id argument are logged, never
/// rule JSON, script bodies or other content read from the file.
fn log_invocation(args: &[String]) -> String {
    let is_known = |arg: &String| KNOWN_FLAGS.contains(&arg.as_str());
    let is_modifier = |arg: &String| matches!(arg.as_str(), "--json" | "--dry-run" | "-h");
    let command = args
        .iter()
        .find(|arg| is_known(arg) && !is_modifier(arg))
        .or_else(|| args.iter().find(|arg| is_known(arg)));
    let Some(command) = command else {
        return "unknown command".to_string();
    };
    let label = if command == "--rules-import-share" {
        // Never log the code itself: it contains the user's automation definitions.
        command.clone()
    } else {
        match flag_optional_value(args, command) {
            Some(value) if !value.is_empty() => format!("{command} {value}"),
            _ => command.clone(),
        }
    };
    crate::logging::info("CLI", &format!("Invoked {label}"));
    label
}

fn dispatch(args: &[String]) -> i32 {
    if has_flag(args, "--help") || has_flag(args, "-h") {
        print!("{USAGE}");
        return 0;
    }
    let json_output = has_flag(args, "--json");
    let dry_run = has_flag(args, "--dry-run");
    if has_flag(args, "--rules-example") {
        println!("{EXAMPLE_RULES_JSON}");
        return 0;
    }
    if has_flag(args, "--rules-schema") {
        return cmd_schema();
    }
    if has_flag(args, "--rules-reference") {
        return cmd_reference();
    }
    if has_flag(args, "--rules-template") {
        let spec = flag_optional_value(args, "--rules-template");
        return cmd_template(spec.as_deref());
    }

    match flag_value(args, "--rules-validate") {
        Ok(Some(path)) => return cmd_validate(&path, json_output),
        Ok(None) => {}
        Err(err) => return fail_with(json_output, &err),
    }
    match flag_value(args, "--rules-apply") {
        Ok(Some(path)) => return cmd_apply(&path, json_output, dry_run),
        Ok(None) => {}
        Err(err) => return fail_with(json_output, &err),
    }
    match flag_value(args, "--rules-sync") {
        Ok(Some(path)) => return cmd_sync(&path, json_output, dry_run),
        Ok(None) => {}
        Err(err) => return fail_with(json_output, &err),
    }
    match flag_value(args, "--rules-delete") {
        Ok(Some(id)) => return cmd_delete(&id, json_output),
        Ok(None) => {}
        Err(err) => return fail_with(json_output, &err),
    }
    match flag_value(args, "--rules-enable") {
        Ok(Some(id)) => return cmd_toggle(&id, true, json_output),
        Ok(None) => {}
        Err(err) => return fail_with(json_output, &err),
    }
    match flag_value(args, "--rules-disable") {
        Ok(Some(id)) => return cmd_toggle(&id, false, json_output),
        Ok(None) => {}
        Err(err) => return fail_with(json_output, &err),
    }
    match flag_value(args, "--rules-reset") {
        Ok(Some(id)) => return cmd_reset(&id, json_output),
        Ok(None) => {}
        Err(err) => return fail_with(json_output, &err),
    }
    if has_flag(args, "--rules-list") {
        return cmd_list(args);
    }
    if has_flag(args, "--rules-export") {
        return cmd_export(args);
    }
    if has_flag(args, "--rules-share") {
        return cmd_share(args, json_output);
    }
    match flag_value(args, "--rules-import-share") {
        Ok(Some(code)) => {
            return cmd_import_share(&code, json_output, has_flag(args, "--enable-imported"))
        }
        Ok(None) => {}
        Err(err) => return fail_with(json_output, &err),
    }

    eprintln!("No command given.");
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
        emit_json(json!({
            "ok": errors.is_empty(),
            "rules": rules.len(),
            "errors": errors,
            "warnings": warnings,
        }));
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
            emit_json(json!({ "ok": false, "errors": errors, "warnings": warnings }));
        } else {
            for warning in &warnings {
                eprintln!("warning: {warning}");
            }
            for error in &errors {
                eprintln!("error: {error}");
            }
            eprintln!("Nothing was applied.");
        }
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
        match IpcClient::call(&IpcRequest::ImportRules(rules)) {
            Ok(IpcResponse::Ok) => {
                if json_output {
                    emit_json(report_json(true, "daemon", &report, &[], false, &warnings));
                } else {
                    println!("Applied to the running daemon.");
                    print_report(&report);
                }
                0
            }
            Ok(IpcResponse::Error(err)) => {
                fail_with(json_output, &format!("daemon rejected the rules: {err}"))
            }
            Ok(other) => fail_with(
                json_output,
                &format!("unexpected daemon response: {other:?}"),
            ),
            Err(err) => fail_with(json_output, &format!("IPC error: {err}")),
        }
    } else {
        let previous = cfg.rules.clone();
        let report = upsert_rules(&mut cfg.rules, &rules);
        if let Err(err) = save_rules(&cfg.rules) {
            return fail_with(json_output, &err);
        }
        if let Err(err) = reset_redefined_quota_runtime(&previous, &rules) {
            return fail_with(json_output, &err);
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
}

fn cmd_sync(path: &str, json_output: bool, dry_run: bool) -> i32 {
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
            emit_json(json!({ "ok": false, "errors": errors, "warnings": warnings }));
        } else {
            for warning in &warnings {
                eprintln!("warning: {warning}");
            }
            for error in &errors {
                eprintln!("error: {error}");
            }
            eprintln!("Nothing was synced.");
        }
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
        // One declarative request: the daemon replaces the set atomically, persists once
        // and reloads hotkeys once, instead of an upsert followed by a delete per rule.
        match IpcClient::call(&IpcRequest::SyncRules(rules)) {
            Ok(IpcResponse::Ok) => {
                if json_output {
                    emit_json(report_json(true, "daemon", &report, &deleted, false, &warnings));
                } else {
                    println!("Synced to the running daemon.");
                    print_report(&report);
                    print_deleted(&deleted);
                }
                0
            }
            Ok(IpcResponse::Error(err)) => {
                fail_with(json_output, &format!("daemon rejected the rules: {err}"))
            }
            Ok(other) => fail_with(
                json_output,
                &format!("unexpected daemon response: {other:?}"),
            ),
            Err(err) => fail_with(json_output, &format!("IPC error: {err}")),
        }
    } else {
        cfg.rules = rules;
        if let Err(err) = save_rules(&cfg.rules) {
            return fail_with(json_output, &err);
        }
        if let Err(err) = reset_redefined_quota_runtime(&previous, &cfg.rules) {
            return fail_with(json_output, &err);
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
}

fn cmd_list(args: &[String]) -> i32 {
    let (cfg, from_daemon) = match current_config() {
        Ok(value) => value,
        Err(err) => return fail(&err),
    };
    if has_flag(args, "--json") {
        return match serde_json::to_string_pretty(&cfg.rules) {
            Ok(json) => {
                println!("{json}");
                0
            }
            Err(err) => fail(&err.to_string()),
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

fn cmd_export(args: &[String]) -> i32 {
    let (cfg, _) = match current_config() {
        Ok(value) => value,
        Err(err) => return fail(&err),
    };
    let json = match serde_json::to_string_pretty(&cfg.rules) {
        Ok(json) => json,
        Err(err) => return fail(&err.to_string()),
    };

    match flag_optional_value(args, "--rules-export") {
        Some(path) => match std::fs::write(&path, format!("{json}\n")) {
            Ok(()) => {
                println!("Exported {} rule(s) to {path}", cfg.rules.len());
                0
            }
            Err(err) => fail(&format!("cannot write {path}: {err}")),
        },
        None => {
            println!("{json}");
            0
        }
    }
}

// ---------------------------------------------------------------------------
// Share codes
// ---------------------------------------------------------------------------

/// Prints a `SYN1:` share code for one rule (by id or name) or for every rule.
fn cmd_share(args: &[String], json_output: bool) -> i32 {
    let cfg = match current_config() {
        Ok((cfg, _)) => cfg,
        Err(err) => return fail_with(json_output, &err),
    };

    let rules = match flag_optional_value(args, "--rules-share") {
        Some(selector) => match cfg
            .rules
            .iter()
            .find(|rule| rule.id == selector || rule.name.eq_ignore_ascii_case(&selector))
        {
            Some(rule) => vec![rule.clone()],
            None => return fail_with(json_output, &format!("rule \"{selector}\" not found")),
        },
        None => cfg.rules.clone(),
    };
    if rules.is_empty() {
        return fail_with(json_output, "no rules configured to share");
    }

    let count = rules.len();
    let (payload, warnings) = crate::sharing::build_payload(rules, &cfg.overlays, &cfg.macro_presets);
    let warning_texts: Vec<String> = warnings
        .iter()
        .map(share_warning_text_cli)
        .collect();

    match crate::sharing::compress_and_encode(&payload) {
        Ok(code) => {
            if json_output {
                emit_json(json!({
                    "ok": true,
                    "rules": count,
                    "bytes": code.len(),
                    "code": code,
                    "warnings": warning_texts,
                }));
            } else {
                for warning in &warning_texts {
                    eprintln!("warning: {warning}");
                }
                // The code alone goes to stdout so it can be piped or redirected safely.
                println!("{code}");
                eprintln!("{count} rule(s), {} byte(s)", code.len());
            }
            0
        }
        Err(err) => fail_with(
            json_output,
            &format!("cannot build share code: {}", crate::sharing::error_text(&err)),
        ),
    }
}

/// Imports a share code from a literal argument, a file path, or stdin (`-`).
fn cmd_import_share(raw_argument: &str, json_output: bool, enable: bool) -> i32 {
    let raw = match read_share_argument(raw_argument) {
        Ok(raw) => raw,
        Err(err) => return fail_with(json_output, &err),
    };
    let payload = match crate::sharing::decode_and_decompress(&raw) {
        Ok(payload) => payload,
        Err(err) => {
            return fail_with(
                json_output,
                &format!("invalid share code: {}", crate::sharing::error_text(&err)),
            )
        }
    };

    let (cfg, from_daemon) = match current_config() {
        Ok(value) => value,
        Err(err) => return fail_with(json_output, &err),
    };
    let prepared = match crate::sharing::prepare_import(
        payload.clone(),
        &cfg.rules,
        &cfg.overlays,
        &cfg.macro_presets,
        enable,
    ) {
        Ok(prepared) => prepared,
        Err(err) => {
            return fail_with(
                json_output,
                &format!("cannot import share code: {}", crate::sharing::error_text(&err)),
            )
        }
    };

    let errors = validate_rules(&prepared.rules);
    if !errors.is_empty() {
        log_errors(&errors);
        if json_output {
            emit_json(json!({ "ok": false, "errors": errors }));
        } else {
            for error in &errors {
                eprintln!("error: {error}");
            }
            eprintln!("Nothing was imported.");
        }
        return 1;
    }
    let warning_texts: Vec<String> = prepared
        .warnings
        .iter()
        .map(share_warning_text_cli)
        .collect();

    let summary = if from_daemon {
        match IpcClient::call(&IpcRequest::ImportShare {
            payload: Box::new(payload),
            enable,
        }) {
            Ok(IpcResponse::ShareImportResult(summary)) => summary,
            Ok(IpcResponse::Error(err)) => {
                return fail_with(json_output, &format!("daemon rejected the share code: {err}"))
            }
            Ok(other) => {
                return fail_with(
                    json_output,
                    &format!("unexpected daemon response: {other:?}"),
                )
            }
            Err(err) => return fail_with(json_output, &format!("IPC error: {err}")),
        }
    } else {
        let mut rules = cfg.rules.clone();
        rules.extend(prepared.rules.iter().cloned());
        if let Err(err) = save_rules(&rules) {
            return fail_with(json_output, &err);
        }
        if !prepared.overlays.is_empty() || !prepared.macro_presets.is_empty() {
            let mut library = crate::config::Library::read();
            library.overlays.extend(prepared.overlays.iter().cloned());
            library.macro_presets.extend(prepared.macro_presets.iter().cloned());
            if let Err(err) = library.save() {
                return fail_with(json_output, &format!("cannot write library: {err}"));
            }
        }
        crate::sharing::ShareImportSummary {
            imported: prepared.rules.len(),
            skipped_duplicates: prepared.skipped_duplicates,
            overlays: prepared.overlays.len(),
            macro_presets: prepared.macro_presets.len(),
            enabled: enable,
        }
    };

    for warning in &warning_texts {
        crate::logging::warn("CLI", warning);
        if !json_output {
            eprintln!("warning: {warning}");
        }
    }

    if json_output {
        emit_json(json!({
            "ok": true,
            "imported": summary.imported,
            "skipped_duplicates": summary.skipped_duplicates,
            "overlays": summary.overlays,
            "macro_presets": summary.macro_presets,
            "enabled": summary.enabled,
            "target": if from_daemon { "daemon" } else { "rules" },
            "warnings": warning_texts,
        }));
    } else {
        println!(
            "Imported {} automation(s); {} duplicate(s) skipped.",
            summary.imported, summary.skipped_duplicates
        );
        if summary.overlays > 0 || summary.macro_presets > 0 {
            println!(
                "Added {} overlay(s) and {} macro preset(s) to the shared library.",
                summary.overlays, summary.macro_presets
            );
        }
        if summary.imported == 0 && summary.overlays == 0 && summary.macro_presets == 0 {
            println!("Nothing new was imported.");
        } else if !enable && summary.imported > 0 {
            println!(
                "Imported rules are disabled; pass --enable-imported or use --rules-enable <id> to activate them."
            );
        }
        if !from_daemon {
            println!("Applied to {} (daemon is offline).", RulesFile::path().display());
        }
    }
    0
}

/// A share argument is either a literal code, a file path, or `-` for stdin.
fn read_share_argument(value: &str) -> Result<String, String> {
    if value == "-" || Path::new(value).is_file() {
        read_input(value)
    } else {
        Ok(value.to_string())
    }
}

/// English CLI wording for a share-code notice (the GUI localizes its own).
fn share_warning_text_cli(warning: &crate::sharing::ShareWarning) -> String {
    use crate::sharing::ShareWarning as W;
    match warning {
        W::MissingMacroPreset(id) => {
            format!("macro preset \"{id}\" is missing and will not work")
        }
        W::MissingOverlay(id) => format!("overlay \"{id}\" is missing and will not work"),
        W::SensitiveScript => "contains scripts that run code when executed".to_string(),
        W::SensitiveAdminProcess => "starts processes with administrator rights".to_string(),
        W::SensitiveWebhook => "sends HTTP requests and may contain tokens".to_string(),
        W::CustomIconDropped => "custom icon is not included".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Schema and reference discovery
// ---------------------------------------------------------------------------

fn cmd_schema() -> i32 {
    let mut root = schemars::schema_for!(crate::models::Rule);
    if root.meta_schema.is_none() {
        root.meta_schema = Some("http://json-schema.org/draft-07/schema#".to_string());
    }
    match serde_json::to_string_pretty(&root) {
        Ok(text) => {
            println!("{text}");
            0
        }
        Err(err) => fail(&err.to_string()),
    }
}

fn cmd_reference() -> i32 {
    print!("{}", render_reference());
    0
}

/// Human-readable catalogue generated from the same JSON Schema as `--rules-schema`,
/// so the two can never drift apart.
fn render_reference() -> String {
    let root = schemars::schema_for!(crate::models::Rule);
    let mut out = String::new();
    out.push_str("Synector rule reference\n");
    out.push_str("Machine-readable draft-07 JSON Schema: Synector.exe --rules-schema\n\n");
    out.push_str("RULE FIELDS\n");
    if let Some(object) = &root.schema.object {
        render_fields(&root, &object.properties, &object.required, &mut out);
    }
    for (definition, tag, title) in [
        ("Trigger", "$trigger_type", "TRIGGERS"),
        ("Action", "$action_type", "ACTIONS"),
        ("RuleCondition", "$condition_type", "CONDITIONS"),
        ("WaitCondition", "type", "WAIT CONDITIONS"),
        ("MacroEvent", "type", "MACRO EVENTS"),
    ] {
        out.push('\n');
        out.push_str(title);
        out.push_str(&format!(" ({tag})\n"));
        if let Some(definition_schema) = root.definitions.get(definition) {
            render_variants(&root, definition_schema, tag, &mut out);
        }
    }
    out.push_str("\nNested objects (overlays, tags, branches) are described by --rules-schema.\n");
    out
}

fn render_fields(
    root: &RootSchema,
    properties: &schemars::Map<String, Schema>,
    required: &std::collections::BTreeSet<String>,
    out: &mut String,
) {
    for (name, schema) in properties {
        let marker = if is_required(required, name) {
            " (required)"
        } else {
            ""
        };
        out.push_str(&format!(
            "  {name}: {}{marker}\n",
            type_label(root, &schema.clone().into_object(), 0)
        ));
    }
}

fn render_variants(root: &RootSchema, definition: &Schema, tag: &str, out: &mut String) {
    let definition = definition.clone().into_object();
    let Some(variants) = definition
        .subschemas
        .as_ref()
        .and_then(|subschemas| subschemas.one_of.as_ref())
    else {
        return;
    };
    for variant in variants {
        let variant = variant.clone().into_object();
        let Some(object) = &variant.object else {
            continue;
        };
        let variant_name = object
            .properties
            .get(tag)
            .map(|schema| schema.clone().into_object())
            .and_then(|schema| tag_value(&schema))
            .unwrap_or_else(|| "<unknown>".to_string());
        let mut fields = Vec::new();
        for (name, schema) in &object.properties {
            if name == tag {
                continue;
            }
            let marker = if is_required(&object.required, name) {
                " (required)"
            } else {
                ""
            };
            fields.push(format!(
                "{name}: {}{marker}",
                type_label(root, &schema.clone().into_object(), 0)
            ));
        }
        if fields.is_empty() {
            out.push_str(&format!("  {variant_name} - no fields\n"));
        } else {
            out.push_str(&format!("  {variant_name} - {}\n", fields.join("; ")));
        }
    }
}

fn is_required(required: &std::collections::BTreeSet<String>, name: &str) -> bool {
    required.contains(name)
}

fn tag_value(schema: &SchemaObject) -> Option<String> {
    if let Some(values) = &schema.enum_values {
        if let Some(value) = values.first().and_then(Value::as_str) {
            return Some(value.to_string());
        }
    }
    schema
        .const_value
        .as_ref()
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn type_label(root: &RootSchema, schema: &SchemaObject, depth: usize) -> String {
    if depth > 6 {
        return "...".to_string();
    }
    if let Some(reference) = &schema.reference {
        let name = reference.rsplit('/').next().unwrap_or(reference);
        if let Some(definition) = root.definitions.get(name) {
            let definition = definition.clone().into_object();
            if let Some(values) = &definition.enum_values {
                if values.iter().all(Value::is_string) {
                    return values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("|");
                }
            }
        }
        return name.to_string();
    }
    if let Some(instance_types) = &schema.instance_type {
        return match instance_types {
            SingleOrVec::Single(single) => instance_type_label(single.as_ref()).to_string(),
            SingleOrVec::Vec(types) => types
                .iter()
                .map(|instance_type| instance_type_label(instance_type))
                .collect::<Vec<_>>()
                .join(" or "),
        };
    }
    if let Some(values) = &schema.enum_values {
        if values.iter().all(Value::is_string) {
            return values
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("|");
        }
    }
    if let Some(subschemas) = &schema.subschemas {
        if let Some(candidates) = subschemas.any_of.as_ref().or(subschemas.one_of.as_ref()) {
            let labels: Vec<String> = candidates
                .iter()
                .map(|candidate| type_label(root, &candidate.clone().into_object(), depth + 1))
                .collect();
            let non_null: Vec<&String> = labels
                .iter()
                .filter(|label| label.as_str() != "null")
                .collect();
            if labels.len() > 1 && non_null.len() == 1 {
                return format!("{} or null", non_null[0]);
            }
            return labels.join(" or ");
        }
        if let Some(all_of) = &subschemas.all_of {
            if let Some(first) = all_of.first() {
                return type_label(root, &first.clone().into_object(), depth + 1);
            }
        }
    }
    if let Some(array) = &schema.array {
        if let Some(items) = &array.items {
            let item = match items {
                SingleOrVec::Single(item) => Some((**item).clone().into_object()),
                SingleOrVec::Vec(items) => items.first().cloned().map(Schema::into_object),
            };
            if let Some(item) = item {
                return format!("array of {}", type_label(root, &item, depth + 1));
            }
        }
        return "array".to_string();
    }
    if schema.object.is_some() {
        return "object".to_string();
    }
    "any".to_string()
}

fn instance_type_label(instance_type: &InstanceType) -> &'static str {
    match instance_type {
        InstanceType::Null => "null",
        InstanceType::Boolean => "boolean",
        InstanceType::Object => "object",
        InstanceType::Array => "array",
        InstanceType::Number => "number",
        InstanceType::String => "string",
        InstanceType::Integer => "integer",
    }
}

// ---------------------------------------------------------------------------
// Rule templates
// ---------------------------------------------------------------------------

fn cmd_template(spec: Option<&str>) -> i32 {
    let root = schemars::schema_for!(crate::models::Rule);
    let triggers = variant_names(&root, "Trigger", "$trigger_type");
    let actions = variant_names(&root, "Action", "$action_type");
    let Some(spec) = spec else {
        println!("Available triggers: {}", triggers.join(", "));
        println!("Available actions: {}", actions.join(", "));
        println!("Usage: --rules-template <trigger>[:<action>]   (action defaults to notification)");
        return 0;
    };
    let (trigger, action) = parse_template_spec(spec);
    if !triggers.contains(&trigger) {
        return fail(&format!(
            "unknown trigger \"{trigger}\"; available: {}",
            triggers.join(", ")
        ));
    }
    if !actions.contains(&action) {
        return fail(&format!(
            "unknown action \"{action}\"; available: {}",
            actions.join(", ")
        ));
    }
    match template_rule(&root, &trigger, &action) {
        Ok(value) => match serde_json::to_string_pretty(&value) {
            Ok(text) => {
                println!("{text}");
                0
            }
            Err(err) => fail(&err.to_string()),
        },
        Err(err) => fail(&err),
    }
}

fn parse_template_spec(spec: &str) -> (String, String) {
    let spec = spec.trim();
    let (trigger, action) = match spec.split_once(':') {
        Some((trigger, action)) => (trigger.trim(), action.trim()),
        None => (spec, ""),
    };
    let trigger = if trigger.is_empty() {
        "manual".to_string()
    } else {
        trigger.to_string()
    };
    let action = if action.is_empty() {
        "notification".to_string()
    } else {
        action.to_string()
    };
    (trigger, action)
}

/// Builds a minimal but valid rule for the requested trigger/action pair.
fn template_rule(root: &RootSchema, trigger: &str, action: &str) -> Result<Value, String> {
    let mut rule = serde_json::Map::new();
    rule.insert(
        "id".to_string(),
        Value::String(format!("rule_{trigger}_{action}")),
    );
    rule.insert(
        "name".to_string(),
        Value::String(format!("New {trigger} rule")),
    );
    rule.insert(
        "trigger".to_string(),
        variant_value(root, "Trigger", "$trigger_type", trigger, 0)?,
    );
    rule.insert(
        "actions".to_string(),
        Value::Array(vec![variant_value(
            root,
            "Action",
            "$action_type",
            action,
            0,
        )?]),
    );
    Ok(Value::Object(rule))
}

fn variant_names(root: &RootSchema, definition: &str, tag: &str) -> Vec<String> {
    let Some(definition) = root.definitions.get(definition) else {
        return Vec::new();
    };
    let definition = definition.clone().into_object();
    let Some(variants) = definition
        .subschemas
        .as_ref()
        .and_then(|subschemas| subschemas.one_of.as_ref())
    else {
        return Vec::new();
    };
    variants
        .iter()
        .filter_map(|variant| {
            let variant = variant.clone().into_object();
            let object = variant.object.as_ref()?;
            object
                .properties
                .get(tag)
                .map(|schema| schema.clone().into_object())
                .and_then(|schema| tag_value(&schema))
        })
        .collect()
}

fn variant_value(
    root: &RootSchema,
    definition: &str,
    tag: &str,
    variant_name: &str,
    depth: usize,
) -> Result<Value, String> {
    if depth > 8 {
        return Ok(Value::Null);
    }
    let definition_schema = root
        .definitions
        .get(definition)
        .cloned()
        .ok_or_else(|| format!("schema is missing the {definition} definition"))?
        .into_object();
    let variants = definition_schema
        .subschemas
        .as_ref()
        .and_then(|subschemas| subschemas.one_of.as_ref())
        .ok_or_else(|| format!("schema is missing {definition} variants"))?;
    let variant = variants
        .iter()
        .filter_map(|variant| {
            let variant = variant.clone().into_object();
            let object = variant.object.as_ref()?;
            let name = object
                .properties
                .get(tag)
                .map(|schema| schema.clone().into_object())
                .and_then(|schema| tag_value(&schema))?;
            (name == variant_name).then_some(variant)
        })
        .next()
        .ok_or_else(|| format!("unknown {definition} variant \"{variant_name}\""))?;
    let object = variant
        .object
        .as_ref()
        .ok_or_else(|| format!("{definition} variant \"{variant_name}\" is not an object"))?;
    let mut fields = serde_json::Map::new();
    fields.insert(tag.to_string(), Value::String(variant_name.to_string()));
    for required in object.required.iter() {
        if required == tag {
            continue;
        }
        if let Some(schema) = object.properties.get(required) {
            let value = placeholder_value(
                root,
                &schema.clone().into_object(),
                required,
                depth + 1,
            );
            fields.insert(required.clone(), value);
        }
    }
    for (name, value) in extra_fields(variant_name) {
        if object.properties.contains_key(name) {
            fields.insert(name.to_string(), value);
        }
    }
    Ok(Value::Object(fields))
}

/// Fields that are optional in the schema but required by runtime validation rules, plus
/// illustrative defaults that make the generated skeleton immediately useful.
fn extra_fields(variant_name: &str) -> Vec<(&'static str, Value)> {
    match variant_name {
        "resource" => vec![("cpu_percent", Value::from(80.0))],
        "schedule" => vec![
            ("schedule_type", Value::String("interval".to_string())),
            ("interval_sec", Value::from(60)),
        ],
        "play_sound" => vec![
            ("play_beep", Value::Bool(true)),
            ("beep_frequency", Value::from(1000)),
            ("beep_duration_ms", Value::from(200)),
        ],
        "wait" => vec![
            ("mode", Value::String("timer".to_string())),
            ("seconds", Value::from(1.0)),
        ],
        _ => Vec::new(),
    }
}

fn placeholder_value(
    root: &RootSchema,
    schema: &SchemaObject,
    field_name: &str,
    depth: usize,
) -> Value {
    if depth > 8 {
        return Value::Null;
    }
    if let Some(reference) = &schema.reference {
        let name = reference.rsplit('/').next().unwrap_or(reference);
        if let Some(definition) = root.definitions.get(name).cloned() {
            let definition = definition.into_object();
            if let Some(values) = &definition.enum_values {
                if let Some(first) = values.first() {
                    return first.clone();
                }
            }
            if let Some(variants) = definition
                .subschemas
                .as_ref()
                .and_then(|subschemas| subschemas.one_of.as_ref())
            {
                if let Some(first) = variants.first() {
                    let first = first.clone().into_object();
                    if let Some(object) = &first.object {
                        if let Some(tag) = discover_tag_key(object) {
                            if let Some(tag_name) = object
                                .properties
                                .get(&tag)
                                .map(|schema| schema.clone().into_object())
                                .and_then(|schema| tag_value(&schema))
                            {
                                if let Ok(value) =
                                    variant_value(root, name, &tag, &tag_name, depth + 1)
                                {
                                    return value;
                                }
                            }
                        }
                    }
                }
            }
            if let Some(object) = &definition.object {
                let mut fields = serde_json::Map::new();
                for required in object.required.iter() {
                    if let Some(field_schema) = object.properties.get(required) {
                        fields.insert(
                            required.clone(),
                            placeholder_value(
                                root,
                                &field_schema.clone().into_object(),
                                required,
                                depth + 1,
                            ),
                        );
                    }
                }
                return Value::Object(fields);
            }
        }
        return Value::String(name.to_string());
    }
    if let Some(instance_types) = &schema.instance_type {
        return match instance_types {
            SingleOrVec::Single(instance_type) => {
                single_type_placeholder(instance_type, field_name)
            }
            SingleOrVec::Vec(instance_types) => instance_types
                .iter()
                .find(|instance_type| !is_null_instance_type(instance_type))
                .map(|instance_type| single_type_placeholder(instance_type, field_name))
                .unwrap_or(Value::Null),
        };
    }
    if let Some(values) = &schema.enum_values {
        if let Some(first) = values.first() {
            return first.clone();
        }
    }
    if let Some(subschemas) = &schema.subschemas {
        if let Some(candidates) = subschemas.any_of.as_ref().or(subschemas.one_of.as_ref()) {
            for candidate in candidates {
                let candidate = candidate.clone().into_object();
                if is_null_type(&candidate) {
                    continue;
                }
                return placeholder_value(root, &candidate, field_name, depth + 1);
            }
        }
        if let Some(all_of) = &subschemas.all_of {
            if let Some(first) = all_of.first() {
                return placeholder_value(root, &first.clone().into_object(), field_name, depth + 1);
            }
        }
    }
    if schema.array.is_some() {
        return Value::Array(Vec::new());
    }
    if schema.object.is_some() {
        return Value::Object(serde_json::Map::new());
    }
    Value::String("value".to_string())
}

fn single_type_placeholder(instance_type: &InstanceType, field_name: &str) -> Value {
    match instance_type {
        InstanceType::Null => Value::Null,
        InstanceType::Boolean => Value::Bool(false),
        InstanceType::Integer => Value::from(1),
        InstanceType::Number => Value::from(1.0),
        InstanceType::String => Value::String(string_placeholder(field_name)),
        InstanceType::Array => Value::Array(Vec::new()),
        InstanceType::Object => Value::Object(serde_json::Map::new()),
    }
}

fn string_placeholder(field_name: &str) -> String {
    let name = field_name.to_ascii_lowercase();
    if name.contains("key_combination") || name == "hotkey" {
        "CTRL+ALT+N".to_string()
    } else if name.contains("folder_path") {
        "C:\\Path\\To\\Folder".to_string()
    } else if name.contains("file_path") || name.ends_with("path") {
        "C:\\Windows\\System32\\notepad.exe".to_string()
    } else if name.contains("process") {
        "your_process.exe".to_string()
    } else if name.contains("url") {
        "https://example.com/webhook".to_string()
    } else if name.contains("title") {
        "Title".to_string()
    } else if name.contains("message") {
        "Message".to_string()
    } else if name.contains("script") {
        "echo hello".to_string()
    } else if name.contains("name") {
        "name".to_string()
    } else {
        "value".to_string()
    }
}

fn discover_tag_key(object: &schemars::schema::ObjectValidation) -> Option<String> {
    object.properties.iter().find_map(|(name, schema)| {
        let schema = schema.clone().into_object();
        let single_value_enum = schema
            .enum_values
            .as_ref()
            .map(|values| values.len() == 1)
            .unwrap_or(false);
        (schema.const_value.is_some() || single_value_enum).then(|| name.clone())
    })
}

fn is_null_instance_type(instance_type: &InstanceType) -> bool {
    matches!(instance_type, InstanceType::Null)
}

fn is_null_type(schema: &SchemaObject) -> bool {
    match &schema.instance_type {
        Some(SingleOrVec::Single(instance_type)) => {
            matches!(instance_type.as_ref(), InstanceType::Null)
        }
        Some(SingleOrVec::Vec(instance_types)) => instance_types
            .iter()
            .all(|instance_type| matches!(instance_type, InstanceType::Null)),
        None => false,
    }
}

fn cmd_delete(id: &str, json_output: bool) -> i32 {
    let id = id.trim();
    if id.is_empty() {
        return fail_with(json_output, "\"id\" is required");
    }

    if IpcClient::is_daemon_running() {
        let cfg = match daemon_config() {
            Ok(cfg) => cfg,
            Err(err) => return fail_with(json_output, &err),
        };
        if !cfg.rules.iter().any(|rule| rule.id == id) {
            return fail_with(json_output, &format!("rule \"{id}\" not found"));
        }
        return match IpcClient::call(&IpcRequest::DeleteRule(id.to_string())) {
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
        };
    }

    let mut cfg = load_config_for_cli();
    let before = cfg.rules.len();
    cfg.rules.retain(|rule| rule.id != id);
    if cfg.rules.len() == before {
        return fail_with(json_output, &format!("rule \"{id}\" not found"));
    }
    if let Err(err) = save_rules(&cfg.rules) {
        return fail_with(json_output, &err);
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
        match IpcClient::call(&IpcRequest::ToggleRule {
            id: id.to_string(),
            enabled,
        }) {
            Ok(IpcResponse::Ok) => {}
            Ok(IpcResponse::Error(err)) => return fail_with(json_output, &err),
            Ok(other) => {
                return fail_with(json_output, &format!("unexpected daemon response: {other:?}"));
            }
            Err(err) => return fail_with(json_output, &format!("IPC error: {err}")),
        }
    } else if let Err(err) = save_rules(&cfg.rules) {
        return fail_with(json_output, &err);
    }
    if json_output {
        emit_json(json!({
            "ok": true,
            "id": id,
            "is_enabled": enabled,
            "target": target_name(from_daemon),
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
        match IpcClient::call(&IpcRequest::ResetRule(id.to_string())) {
            Ok(IpcResponse::Ok) => {}
            Ok(IpcResponse::Error(err)) => return fail_with(json_output, &err),
            Ok(other) => {
                return fail_with(json_output, &format!("unexpected daemon response: {other:?}"));
            }
            Err(err) => return fail_with(json_output, &format!("IPC error: {err}")),
        }
    } else {
        if let Err(err) = save_rules(&cfg.rules) {
            return fail_with(json_output, &err);
        }
        if let Err(err) = forget_runtime_counters(&[id]) {
            return fail_with(json_output, &err);
        }
    }
    if json_output {
        emit_json(json!({
            "ok": true,
            "id": id,
            "execution_count": 0,
            "is_enabled": true,
            "target": target_name(from_daemon),
        }));
    } else {
        println!("Reset rule \"{id}\": execution counter cleared, rule enabled.");
    }
    0
}

// ---------------------------------------------------------------------------
// Rule file parsing and validation
// ---------------------------------------------------------------------------

/// Parses a rule file: a single rule object, an array of rules, or `{"rules": [...]}`.
pub fn parse_rules_json(raw: &str) -> Result<Vec<Rule>, String> {
    let clean = raw.trim_start_matches('\u{feff}').trim();
    if clean.is_empty() {
        return Err("input is empty".into());
    }
    let value: Value =
        serde_json::from_str(clean).map_err(|err| format!("not valid JSON: {err}"))?;

    let items: Vec<Value> = match value {
        Value::Array(items) => items,
        Value::Object(mut map) => match map.remove("rules") {
            Some(Value::Array(items)) => items,
            Some(_) => return Err("\"rules\" must be an array".into()),
            None => vec![Value::Object(map)],
        },
        other => {
            return Err(format!(
                "expected a rule object or an array of rules, got {}",
                json_type(&other)
            ))
        }
    };
    if items.is_empty() {
        return Err("rule file contains no rules".into());
    }

    let mut rules = Vec::with_capacity(items.len());
    for (index, item) in items.into_iter().enumerate() {
        let number = index + 1;
        let object = match item {
            Value::Object(object) => object,
            other => {
                return Err(format!(
                    "rule #{number}: expected an object, got {}",
                    json_type(&other)
                ))
            }
        };
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if name.is_empty() {
            return Err(format!("rule #{number}: \"name\" is required"));
        }
        let id = object
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if id.is_empty() {
            return Err(format!(
                "rule #{number} ({name}): \"id\" is required - it is the upsert key used by --rules-apply"
            ));
        }
        let rule: Rule = serde_json::from_value(Value::Object(object))
            .map_err(|err| format!("rule #{number} ({name}): {err}"))?;
        rules.push(rule);
    }
    Ok(rules)
}

/// Structural rule errors that must block `--rules-apply`.
pub fn validate_rules(rules: &[Rule]) -> Vec<String> {
    let mut errors = Vec::new();
    let mut seen_ids: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for (index, rule) in rules.iter().enumerate() {
        let number = index + 1;
        let id = rule.id.trim();
        if id.is_empty() {
            errors.push(format!(
                "rule #{number}: \"id\" is required - it is the upsert key used by --rules-apply"
            ));
        } else if let Some(previous) = seen_ids.insert(id.to_string(), number) {
            errors.push(format!(
                "duplicate rule id \"{id}\" (rules #{previous} and #{number})"
            ));
        }
        if rule.name.trim().is_empty() {
            errors.push(format!("rule #{number}: \"name\" is required"));
        }
        if !rule.cooldown_seconds.is_finite() || rule.cooldown_seconds < 0.0 {
            errors.push(format!(
                "rule #{number} ({}): cooldown_seconds must be a non-negative number",
                display_id(rule)
            ));
        }
        if let Some(QuotaPeriod::Custom { value: 0, .. }) = rule.quota_period {
            errors.push(format!(
                "rule #{number} ({}): a custom quota_period window needs value > 0",
                display_id(rule)
            ));
        }

        validate_trigger(&rule.trigger, number, rule, &mut errors);

        let mut has_infinite_macro = false;
        let mut check = |action: &Action| {
            if matches!(action, Action::Macro { infinite: true, .. }) {
                has_infinite_macro = true;
            }
        };
        walk_actions(&rule.actions, &mut check);
        walk_actions(&rule.on_exit_actions, &mut check);
        if has_infinite_macro && !matches!(rule.trigger, Some(Trigger::GlobalHotkey { .. })) {
            errors.push(format!(
                "rule #{number} ({}): an infinite macro requires a global hotkey trigger, otherwise the rule can never be stopped",
                display_id(rule)
            ));
        }
    }
    errors
}

fn validate_trigger(
    trigger: &Option<Trigger>,
    number: usize,
    rule: &Rule,
    errors: &mut Vec<String>,
) {
    let id = display_id(rule);
    match trigger {
        Some(Trigger::Process { process_name, .. }) if process_name.trim().is_empty() => {
            errors.push(format!(
                "rule #{number} ({id}): process trigger needs a non-empty process_name"
            ));
        }
        Some(Trigger::WindowFocus { process_name, .. }) if process_name.trim().is_empty() => {
            errors.push(format!(
                "rule #{number} ({id}): window_focus trigger needs a non-empty process_name"
            ));
        }
        Some(Trigger::ResourceThreshold {
            process_name,
            cpu_percent,
            ram_mb,
            ..
        }) => {
            if process_name.trim().is_empty() {
                errors.push(format!(
                    "rule #{number} ({id}): resource trigger needs a non-empty process_name"
                ));
            }
            if cpu_percent.is_none() && ram_mb.is_none() {
                errors.push(format!(
                    "rule #{number} ({id}): resource trigger needs cpu_percent or ram_mb"
                ));
            }
        }
        Some(Trigger::GlobalHotkey {
            key_combination, ..
        }) if key_combination.trim().is_empty() => {
            errors.push(format!(
                "rule #{number} ({id}): hotkey trigger needs a non-empty key_combination"
            ));
        }
        Some(Trigger::FileWatcher { folder_path, .. }) if folder_path.trim().is_empty() => {
            errors.push(format!(
                "rule #{number} ({id}): file_watcher trigger needs a non-empty folder_path"
            ));
        }
        Some(Trigger::Schedule {
            schedule_type: ScheduleType::Interval,
            interval_sec,
            ..
        }) if *interval_sec == 0 => {
            errors.push(format!(
                "rule #{number} ({id}): interval schedule needs interval_sec > 0"
            ));
        }
        _ => {}
    }
}

/// Non-blocking warnings: the rule is structurally valid but will not do what its author expects.
pub fn rule_warnings(rules: &[Rule], cfg: &AppConfig) -> Vec<String> {
    let mut warnings = Vec::new();
    for (index, rule) in rules.iter().enumerate() {
        let number = index + 1;
        let id = display_id(rule);
        if rule.trigger.is_none() {
            warnings.push(format!(
                "rule #{number} ({id}): no trigger, it can only be started manually or with --run-rule"
            ));
        }
        if rule.actions.is_empty() {
            warnings.push(format!(
                "rule #{number} ({id}): the action pipeline is empty"
            ));
        }
        if rule.max_executions == Some(0) {
            warnings.push(format!(
                "rule #{number} ({id}): max_executions is 0, the rule will never run"
            ));
        }
        if rule.quota_period.is_some() && rule.max_executions.is_none() {
            warnings.push(format!(
                "rule #{number} ({id}): quota_period has no effect without max_executions"
            ));
        }

        let mut check = |action: &Action| match action {
            Action::Macro {
                preset_id: Some(preset_id),
                ..
            } if cfg.find_macro(preset_id).is_none() => warnings.push(format!(
                "rule #{number} ({id}): macro preset \"{preset_id}\" does not exist"
            )),
            Action::Overlay {
                overlay_id,
                custom_overlay: None,
                ..
            } if !overlay_id.is_empty() && cfg.find_overlay(overlay_id).is_none() => {
                warnings.push(format!(
                    "rule #{number} ({id}): overlay \"{overlay_id}\" does not exist"
                ))
            }
            Action::Wait {
                mode: crate::models::WaitMode::UntilCondition,
                condition: None,
                ..
            } => warnings.push(format!(
                "rule #{number} ({id}): wait has no condition and will block until its timeout"
            )),
            Action::StartProcess { file_path, .. } if file_path.trim().is_empty() => warnings.push(
                format!("rule #{number} ({id}): start_process has an empty file_path"),
            ),
            _ => {}
        };
        walk_actions(&rule.actions, &mut check);
        walk_actions(&rule.on_exit_actions, &mut check);
    }
    warnings
}

fn walk_actions<F: FnMut(&Action)>(actions: &[Action], visit: &mut F) {
    for action in actions {
        visit(action);
        if let Action::IfElse {
            then_actions,
            else_if_branches,
            else_actions,
            ..
        } = action
        {
            walk_actions(then_actions, visit);
            for branch in else_if_branches {
                walk_actions(&branch.actions, visit);
            }
            walk_actions(else_actions, visit);
        }
    }
}

// ---------------------------------------------------------------------------
// Applying rules
// ---------------------------------------------------------------------------

/// Upserts `incoming` into `existing` by id, returning what was created and what was replaced.
pub fn upsert_rules(existing: &mut Vec<Rule>, incoming: &[Rule]) -> ApplyReport {
    let mut report = ApplyReport::default();
    for rule in incoming {
        match existing.iter_mut().find(|candidate| candidate.id == rule.id) {
            Some(slot) => {
                *slot = rule.clone();
                report.updated.push(rule.id.clone());
            }
            None => {
                existing.push(rule.clone());
                report.created.push(rule.id.clone());
            }
        }
    }
    report
}

/// Describes what `upsert_rules` would do without mutating anything.
pub fn classify_rules(existing: &[Rule], incoming: &[Rule]) -> ApplyReport {
    let mut report = ApplyReport::default();
    for rule in incoming {
        if existing.iter().any(|candidate| candidate.id == rule.id) {
            report.updated.push(rule.id.clone());
        } else {
            report.created.push(rule.id.clone());
        }
    }
    report
}

fn print_report(report: &ApplyReport) {
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

fn print_deleted(deleted: &[String]) {
    for id in deleted {
        println!("  deleted  {id}");
    }
    println!("{} deleted", deleted.len());
}

/// Plans a declarative sync: upserts from `incoming` plus deletion of ids missing from it.
fn plan_sync(existing: &[Rule], incoming: &[Rule]) -> (ApplyReport, Vec<String>) {
    let report = classify_rules(existing, incoming);
    let deleted = existing
        .iter()
        .filter(|existing_rule| {
            !incoming
                .iter()
                .any(|incoming_rule| incoming_rule.id == existing_rule.id)
        })
        .map(|rule| rule.id.clone())
        .collect();
    (report, deleted)
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

fn target_name(from_daemon: bool) -> &'static str {
    if from_daemon {
        "daemon"
    } else {
        "rules"
    }
}

fn report_json(
    ok: bool,
    target: &str,
    report: &ApplyReport,
    deleted: &[String],
    dry_run: bool,
    warnings: &[String],
) -> Value {
    json!({
        "ok": ok,
        "target": target,
        "dry_run": dry_run,
        "created": report.created,
        "updated": report.updated,
        "deleted": deleted,
        "warnings": warnings,
    })
}

fn emit_json(value: Value) {
    match serde_json::to_string_pretty(&value) {
        Ok(text) => println!("{text}"),
        Err(_) => println!("{{}}"),
    }
}

fn fail_with(json_output: bool, message: &str) -> i32 {
    if json_output {
        crate::logging::error("CLI", message);
        emit_json(json!({ "ok": false, "error": message, "errors": [message] }));
        1
    } else {
        fail(message)
    }
}

fn log_warnings(warnings: &[String]) {
    for warning in warnings {
        crate::logging::warn("CLI", warning);
    }
}

fn log_errors(errors: &[String]) {
    for error in errors {
        crate::logging::error("CLI", error);
    }
}

// ---------------------------------------------------------------------------
// Config access
// ---------------------------------------------------------------------------

/// Reads the split config sections without the side effects of [`AppConfig::load`]
/// (file creation and autostart registration).
fn load_config_for_cli() -> AppConfig {
    AppConfig::read()
}

fn save_rules(rules: &[Rule]) -> Result<(), String> {
    RulesFile::save_rules(rules)
        .map_err(|err| format!("cannot write {}: {err}", RulesFile::path().display()))
}

/// Drops daemon-owned runtime counters of deleted or reset rules.
fn forget_runtime_counters(ids: &[&str]) -> Result<(), String> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut runtime = RuntimeState::read();
    for id in ids {
        runtime.forget(id);
    }
    runtime
        .save()
        .map_err(|err| format!("cannot write {}: {err}", RuntimeState::path().display()))
}

/// Keeps `runtime.json` in step with a rule file applied while the daemon is offline.
/// Redefining a quota (changing its period, or enabling one) starts a fresh counting
/// window, matching the daemon's `adopt_rule_runtime`; every other update keeps the
/// counters, and `last_executed_at` is always preserved because schedules rely on it.
fn reset_redefined_quota_runtime(previous: &[Rule], applied: &[Rule]) -> Result<(), String> {
    let redefined: Vec<&str> = applied
        .iter()
        .filter(|rule| {
            previous.iter().find(|old| old.id == rule.id).is_some_and(|old| {
                old.quota_period != rule.quota_period
                    || (old.max_executions.is_none() && rule.max_executions.is_some())
            })
        })
        .map(|rule| rule.id.as_str())
        .collect();
    if redefined.is_empty() {
        return Ok(());
    }
    let mut runtime = RuntimeState::read();
    let mut changed = false;
    for id in redefined {
        if let Some(entry) = runtime.rules.get_mut(id) {
            changed |= entry.execution_count != 0 || entry.quota_window_start.is_some();
            entry.execution_count = 0;
            entry.quota_window_start = None;
        }
    }
    if !changed {
        return Ok(());
    }
    runtime
        .save()
        .map_err(|err| format!("cannot write {}: {err}", RuntimeState::path().display()))
}

fn daemon_config() -> Result<AppConfig, String> {
    match IpcClient::call(&IpcRequest::GetState) {
        Ok(IpcResponse::State(state)) => Ok(state.config),
        Ok(IpcResponse::Error(err)) => Err(format!("daemon error: {err}")),
        Ok(other) => Err(format!("unexpected daemon response: {other:?}")),
        Err(err) => Err(format!("IPC error: {err}")),
    }
}

/// Returns the current rules plus whether they came from the running daemon.
fn current_config() -> Result<(AppConfig, bool), String> {
    if IpcClient::is_daemon_running() {
        Ok((daemon_config()?, true))
    } else {
        Ok((load_config_for_cli(), false))
    }
}

fn config_source(from_daemon: bool) -> String {
    if from_daemon {
        "the running daemon".to_string()
    } else {
        RulesFile::path().display().to_string()
    }
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn read_input(path: &str) -> Result<String, String> {
    if path == "-" {
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .map_err(|err| format!("cannot read stdin: {err}"))?;
        Ok(buffer)
    } else {
        std::fs::read_to_string(Path::new(path)).map_err(|err| format!("cannot read {path}: {err}"))
    }
}

fn display_id(rule: &Rule) -> &str {
    if rule.id.trim().is_empty() {
        "<no id>"
    } else {
        rule.id.as_str()
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

fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

fn flag_value(args: &[String], flag: &str) -> Result<Option<String>, String> {
    let Some(position) = args.iter().position(|arg| arg == flag) else {
        return Ok(None);
    };
    match args.get(position + 1) {
        Some(value) if value == "-" || !value.starts_with('-') => Ok(Some(value.clone())),
        _ => Err(format!("{flag} requires a file path argument")),
    }
}

fn flag_optional_value(args: &[String], flag: &str) -> Option<String> {
    let position = args.iter().position(|arg| arg == flag)?;
    match args.get(position + 1) {
        Some(value) if !value.starts_with('-') => Some(value.clone()),
        _ => None,
    }
}

fn fail(message: &str) -> i32 {
    crate::logging::error("CLI", message);
    eprintln!("error: {message}");
    1
}

/// Release builds use the Windows GUI subsystem, so a CLI invocation has to reattach to the
/// parent console before the first line is printed. Redirected std handles (pipes) are left
/// untouched so machine-readable output keeps working.
fn ensure_console() {
    #[cfg(all(windows, not(debug_assertions)))]
    attach_parent_console();
}

#[cfg(all(windows, not(debug_assertions)))]
fn attach_parent_console() {
    use windows::core::w;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, GetFileType, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_WRITE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, FILE_TYPE_UNKNOWN, OPEN_EXISTING,
    };
    use windows::Win32::System::Console::{
        AttachConsole, GetStdHandle, SetConsoleOutputCP, SetStdHandle, ATTACH_PARENT_PROCESS,
        STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
    };

    unsafe {
        let stdout_is_real = GetStdHandle(STD_OUTPUT_HANDLE)
            .map(|handle| GetFileType(handle) != FILE_TYPE_UNKNOWN)
            .unwrap_or(false);
        if stdout_is_real {
            return;
        }
        if AttachConsole(ATTACH_PARENT_PROCESS).is_err() {
            return;
        }
        let _ = SetConsoleOutputCP(65001);
        if let Ok(conout) = CreateFileW(
            w!("CONOUT$"),
            FILE_GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        ) {
            let _ = SetStdHandle(STD_OUTPUT_HANDLE, conout);
            let _ = SetStdHandle(STD_ERROR_HANDLE, conout);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{MacroOperation, WaitMode};

    fn rule_with_notification(id: &str) -> Rule {
        Rule {
            id: id.to_string(),
            name: format!("Rule {id}"),
            trigger: Some(Trigger::Manual),
            actions: vec![Action::Notification {
                title: "Title".to_string(),
                message: "Message".to_string(),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn test_parse_single_object() {
        let raw = r#"{
            "id": "rule_single",
            "name": "Single",
            "actions": [{ "$action_type": "notification", "title": "T", "message": "M" }]
        }"#;
        let rules = parse_rules_json(raw).expect("parse single rule");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].id, "rule_single");
        assert!(rules[0].trigger.is_none());
    }

    #[test]
    fn test_parse_array_and_wrapper() {
        let array = r#"[
            { "id": "a", "name": "A", "actions": [] },
            { "id": "b", "name": "B", "actions": [] }
        ]"#;
        assert_eq!(parse_rules_json(array).expect("array").len(), 2);

        let wrapped = r#"{"rules": [{ "id": "a", "name": "A", "actions": [] }]}"#;
        assert_eq!(parse_rules_json(wrapped).expect("wrapper").len(), 1);
    }

    #[test]
    fn test_parse_strips_bom_and_rejects_bad_input() {
        let raw = "\u{feff}[{\"id\":\"a\",\"name\":\"A\",\"actions\":[]}]";
        assert_eq!(parse_rules_json(raw).expect("bom").len(), 1);

        assert!(parse_rules_json("").is_err());
        assert!(parse_rules_json("{\"rules\": 5}").is_err());
        assert!(parse_rules_json("42").is_err());
    }

    #[test]
    fn test_parse_requires_id_and_name() {
        let missing_id = r#"{"name": "No id", "actions": []}"#;
        let err = parse_rules_json(missing_id).expect_err("missing id");
        assert!(err.contains("\"id\" is required"), "{err}");

        let missing_name = r#"{"id": "x", "actions": []}"#;
        let err = parse_rules_json(missing_name).expect_err("missing name");
        assert!(err.contains("\"name\" is required"), "{err}");
    }

    #[test]
    fn test_parse_reports_action_errors() {
        let raw = r#"[{"id":"a","name":"A","actions":[{"$action_type":"unknown"}]}]"#;
        let err = parse_rules_json(raw).expect_err("unknown action");
        assert!(err.contains("rule #1 (A)"), "{err}");
    }

    #[test]
    fn test_validate_duplicate_ids_and_empty_name() {
        let mut first = rule_with_notification("dup");
        first.name = String::new();
        let second = rule_with_notification("dup");
        let errors = validate_rules(&[first, second]);
        assert!(errors.iter().any(|e| e.contains("duplicate rule id \"dup\"")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("\"name\" is required")), "{errors:?}");
    }

    #[test]
    fn test_validate_trigger_requirements() {
        let mut rule = rule_with_notification("process_rule");
        rule.trigger = Some(Trigger::Process {
            process_name: "  ".to_string(),
            match_mode: Default::default(),
            single_instance: false,
        });
        let errors = validate_rules(&[rule]);
        assert!(
            errors.iter().any(|e| e.contains("non-empty process_name")),
            "{errors:?}"
        );
    }

    #[test]
    fn test_validate_infinite_macro_requires_hotkey() {
        let infinite_macro = Action::Macro {
            preset_id: None,
            operation: MacroOperation::Run,
            events: Vec::new(),
            repeat: 1,
            speed: 1.0,
            infinite: true,
        };

        let mut process_rule = rule_with_notification("macro_process");
        process_rule.trigger = Some(Trigger::Process {
            process_name: "game.exe".to_string(),
            match_mode: Default::default(),
            single_instance: false,
        });
        process_rule.actions = vec![infinite_macro.clone()];
        let errors = validate_rules(&[process_rule]);
        assert!(
            errors.iter().any(|e| e.contains("infinite macro requires a global hotkey")),
            "{errors:?}"
        );

        let mut hotkey_rule = rule_with_notification("macro_hotkey");
        hotkey_rule.trigger = Some(Trigger::GlobalHotkey {
            key_combination: "F8".to_string(),
            hold_mode: false,
            repeat_while_held: false,
        });
        hotkey_rule.actions = vec![infinite_macro];
        assert!(validate_rules(&[hotkey_rule]).is_empty());
    }

    #[test]
    fn test_validate_custom_quota_period() {
        let mut rule = rule_with_notification("custom_quota");
        rule.max_executions = Some(5);
        rule.quota_period = Some(QuotaPeriod::Custom {
            value: 0,
            unit: crate::models::QuotaUnit::Hours,
        });
        let errors = validate_rules(&[rule.clone()]);
        assert!(
            errors.iter().any(|e| e.contains("custom quota_period window needs value > 0")),
            "{errors:?}"
        );

        rule.quota_period = Some(QuotaPeriod::Custom {
            value: 2,
            unit: crate::models::QuotaUnit::Days,
        });
        assert!(validate_rules(&[rule.clone()]).is_empty());
        assert!(rule_warnings(&[rule], &AppConfig::default()).is_empty());

        // A period without a limit does nothing; it is a warning, not an error.
        let mut orphan = rule_with_notification("orphan_period");
        orphan.quota_period = Some(QuotaPeriod::Day);
        assert!(validate_rules(&[orphan.clone()]).is_empty());
        let warnings = rule_warnings(&[orphan], &AppConfig::default());
        assert!(
            warnings.iter().any(|w| w.contains("quota_period has no effect")),
            "{warnings:?}"
        );
    }

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
    fn test_upsert_and_classify() {
        let mut existing = vec![rule_with_notification("keep"), rule_with_notification("update")];
        let incoming = vec![
            rule_with_notification("update"),
            rule_with_notification("new"),
        ];

        let report = classify_rules(&existing, &incoming);
        assert_eq!(report.created, vec!["new".to_string()]);
        assert_eq!(report.updated, vec!["update".to_string()]);

        let report = upsert_rules(&mut existing, &incoming);
        assert_eq!(report.created, vec!["new".to_string()]);
        assert_eq!(existing.len(), 3);
        assert!(existing.iter().any(|rule| rule.id == "keep"));
    }

    #[test]
    fn test_updates_replace_rule_definitions() {
        let mut existing = vec![rule_with_notification("runtime")];
        existing[0].execution_count = 17;
        existing[0].last_executed_at = Some("2026-01-01 00:00:00".to_string());

        let mut incoming = rule_with_notification("runtime");
        incoming.name = "Renamed".to_string();
        let report = upsert_rules(&mut existing, std::slice::from_ref(&incoming));
        assert_eq!(report.updated, vec!["runtime".to_string()]);
        assert_eq!(existing[0].name, "Renamed");
        // A rule file never carries counters; runtime.json stays the daemon's business.
        assert_eq!(existing[0].execution_count, 0);
        assert!(existing[0].last_executed_at.is_none());
    }

    #[test]
    fn test_warnings_for_missing_preset_and_empty_pipeline() {
        let cfg = AppConfig::default();
        let mut rule = rule_with_notification("preset_rule");
        rule.actions.push(Action::Macro {
            preset_id: Some("missing_preset".to_string()),
            operation: MacroOperation::Run,
            events: Vec::new(),
            repeat: 1,
            speed: 1.0,
            infinite: false,
        });
        rule.actions.push(Action::Wait {
            mode: WaitMode::UntilCondition,
            seconds: 0.0,
            condition: None,
            timeout_sec: 5.0,
            timeout_policy: Default::default(),
        });
        let warnings = rule_warnings(&[rule], &cfg);
        assert!(
            warnings.iter().any(|w| w.contains("macro preset \"missing_preset\"")),
            "{warnings:?}"
        );
        assert!(
            warnings.iter().any(|w| w.contains("wait has no condition")),
            "{warnings:?}"
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
    fn test_rules_schema_has_trigger_and_action_definitions() {
        let root = schemars::schema_for!(crate::models::Rule);
        let json = serde_json::to_value(&root).expect("schema serializes");
        assert!(json["definitions"]["Trigger"].is_object());
        assert!(json["definitions"]["Action"].is_object());
        assert!(json["definitions"]["RuleCondition"].is_object());
        let text = json.to_string();
        for needle in [
            "$trigger_type",
            "$action_type",
            "$condition_type",
            "play_sound",
            "schedule",
            "max_executions",
            "quota_period",
        ] {
            assert!(text.contains(needle), "schema missing {needle}");
        }
    }

    #[test]
    fn test_reference_covers_all_trigger_and_action_variants() {
        let root = schemars::schema_for!(crate::models::Rule);
        let reference = render_reference();
        for (definition, tag) in [("Trigger", "$trigger_type"), ("Action", "$action_type")] {
            let definition = root.definitions[definition].clone().into_object();
            let variants = definition
                .subschemas
                .as_ref()
                .and_then(|subschemas| subschemas.one_of.as_ref())
                .expect("variants");
            assert!(variants.len() >= 9, "{definition:?} has too few variants");
            for variant in variants {
                let variant = variant.clone().into_object();
                let object = variant.object.as_ref().expect("variant object");
                let name = object
                    .properties
                    .get(tag)
                    .map(|schema| schema.clone().into_object())
                    .and_then(|schema| tag_value(&schema))
                    .expect("variant tag");
                assert!(
                    reference.contains(&format!("  {name}")),
                    "reference is missing {definition:?} variant {name}"
                );
            }
        }
    }

    #[test]
    fn test_reference_mentions_rule_fields_and_sections() {
        let reference = render_reference();
        for needle in [
            "RULE FIELDS",
            "max_executions",
            "quota_period",
            "TRIGGERS",
            "ACTIONS",
            "CONDITIONS",
            "WAIT CONDITIONS",
            "MACRO EVENTS",
        ] {
            assert!(reference.contains(needle), "reference missing {needle}");
        }
    }

    #[test]
    fn test_template_generates_valid_rule_for_every_combination() {
        let root = schemars::schema_for!(crate::models::Rule);
        let triggers = variant_names(&root, "Trigger", "$trigger_type");
        let actions = variant_names(&root, "Action", "$action_type");
        assert_eq!(triggers.len(), 9, "{triggers:?}");
        assert_eq!(actions.len(), 25, "{actions:?}");
        for trigger in &triggers {
            for action in &actions {
                let value = template_rule(&root, trigger, action).expect("template");
                let rule: Rule = serde_json::from_value(value.clone()).unwrap_or_else(|err| {
                    panic!("{trigger}:{action} does not parse: {err}\n{value}")
                });
                let errors = validate_rules(std::slice::from_ref(&rule));
                assert!(
                    errors.is_empty(),
                    "{trigger}:{action} template is invalid: {errors:?}\n{value}"
                );
            }
        }
    }

    #[test]
    fn test_template_spec_shorthand_and_errors() {
        assert_eq!(
            parse_template_spec("hotkey"),
            ("hotkey".to_string(), "notification".to_string())
        );
        assert_eq!(
            parse_template_spec(":play_sound"),
            ("manual".to_string(), "play_sound".to_string())
        );
        assert_eq!(
            parse_template_spec("schedule:wait"),
            ("schedule".to_string(), "wait".to_string())
        );
        let root = schemars::schema_for!(crate::models::Rule);
        assert!(template_rule(&root, "nope", "notification").is_err());
        assert!(template_rule(&root, "manual", "nope").is_err());
    }

    #[test]
    fn test_template_play_sound_roundtrip() {
        let root = schemars::schema_for!(crate::models::Rule);
        let value = template_rule(&root, "schedule", "play_sound").expect("template");
        let json = serde_json::to_string(&value).expect("json");
        let rules = parse_rules_json(&json).expect("parse");
        assert_eq!(rules.len(), 1);
        assert!(matches!(
            rules[0].actions[0],
            Action::PlaySound { play_beep: true, .. }
        ));
        assert!(matches!(
            rules[0].trigger,
            Some(Trigger::Schedule {
                interval_sec: 60,
                ..
            })
        ));
    }

    #[test]
    fn test_plan_sync_reports_created_updated_deleted() {
        let existing = vec![rule_with_notification("keep"), rule_with_notification("update")];
        let incoming = vec![rule_with_notification("update"), rule_with_notification("new")];
        let (report, deleted) = plan_sync(&existing, &incoming);
        assert_eq!(report.created, vec!["new".to_string()]);
        assert_eq!(report.updated, vec!["update".to_string()]);
        assert_eq!(deleted, vec!["keep".to_string()]);
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

    #[test]
    fn test_report_json_shape() {
        let report = ApplyReport {
            created: vec!["a".to_string()],
            updated: vec!["b".to_string()],
        };
        let value = report_json(true, "daemon", &report, &["c".to_string()], true, &[]);
        assert_eq!(value["ok"], serde_json::json!(true));
        assert_eq!(value["target"], serde_json::json!("daemon"));
        assert_eq!(value["dry_run"], serde_json::json!(true));
        assert_eq!(value["created"][0], serde_json::json!("a"));
        assert_eq!(value["updated"][0], serde_json::json!("b"));
        assert_eq!(value["deleted"][0], serde_json::json!("c"));
    }

    #[test]
    fn test_share_argument_accepts_literal_and_stdin() {
        assert_eq!(
            read_share_argument("SYN1:abc").expect("literal"),
            "SYN1:abc"
        );
        // A value that is not an existing file is treated as a literal code, not as a path.
        assert_eq!(
            read_share_argument("synector_missing_code_file_12345").expect("literal fallback"),
            "synector_missing_code_file_12345"
        );
    }

    #[test]
    fn test_share_warning_cli_text() {
        assert!(share_warning_text_cli(&crate::sharing::ShareWarning::SensitiveScript)
            .contains("script"));
        assert!(share_warning_text_cli(&crate::sharing::ShareWarning::MissingOverlay(
            "ov".into()
        ))
        .contains("ov"));
        assert_eq!(
            share_warning_text_cli(&crate::sharing::ShareWarning::CustomIconDropped),
            "custom icon is not included"
        );
    }
}
