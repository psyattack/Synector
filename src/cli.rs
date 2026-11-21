//! Command-line rule authoring: validate, apply, list, export and delete rules without the GUI.
//!
//! Rule files are plain JSON in the [`Rule`] serde shape: one rule object, an array of rules, or
//! `{"rules": [...]}`. Applying upserts by `id`; when the daemon is running the change goes
//! through IPC so it takes effect immediately, otherwise `rules.json` is edited directly.
//! Runtime counters stay in the daemon-owned `runtime.json`, never in rule files.

use std::io::{Read, Write};
use std::path::Path;

use serde_json::{json, Map, Value};

use crate::config::{AppConfig, RulesFile, RuntimeState};
use crate::ipc::{IpcClient, IpcRequest, IpcResponse};
use crate::models::{Action, QuotaPeriod, Rule, Trigger};

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
  --json                      Machine-readable output for validate/apply/sync/list/export/
                              share/import-share/delete/enable/disable/reset
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

/// Flags that modify a command instead of selecting one.
const MODIFIER_FLAGS: &[&str] = &["--json", "--dry-run", "--enable-imported"];

/// One rule-authoring command. The command is resolved before anything runs, so the priority that
/// used to be implicit in the ordered `if` chain is gone: two command flags in one invocation are
/// a usage error instead of a silent preference ([U13-002]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    Help,
    Example,
    Schema,
    Reference,
    Template,
    Validate,
    Apply,
    Sync,
    Delete,
    Enable,
    Disable,
    Reset,
    List,
    Export,
    Share,
    ImportShare,
}

/// How a command consumes the argument after its flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandArgument {
    /// A plain switch: the next argument is not part of the command.
    None,
    /// The value is required. `-` means stdin for the commands that read input.
    Required,
    /// The value is optional; without it the command uses its default selector.
    Optional,
}

impl Command {
    fn from_flag(flag: &str) -> Option<Self> {
        Some(match flag {
            "--help" | "-h" => Self::Help,
            "--rules-example" => Self::Example,
            "--rules-schema" => Self::Schema,
            "--rules-reference" => Self::Reference,
            "--rules-template" => Self::Template,
            "--rules-validate" => Self::Validate,
            "--rules-apply" => Self::Apply,
            "--rules-sync" => Self::Sync,
            "--rules-delete" => Self::Delete,
            "--rules-enable" => Self::Enable,
            "--rules-disable" => Self::Disable,
            "--rules-reset" => Self::Reset,
            "--rules-list" => Self::List,
            "--rules-export" => Self::Export,
            "--rules-share" => Self::Share,
            "--rules-import-share" => Self::ImportShare,
            _ => return None,
        })
    }

    fn argument(self) -> CommandArgument {
        match self {
            Self::Help | Self::Example | Self::Schema | Self::Reference | Self::List => {
                CommandArgument::None
            }
            Self::Template | Self::Export | Self::Share => CommandArgument::Optional,
            Self::Validate
            | Self::Apply
            | Self::Sync
            | Self::Delete
            | Self::Enable
            | Self::Disable
            | Self::Reset
            | Self::ImportShare => CommandArgument::Required,
        }
    }

    /// What the value after the flag is, for usage errors ([U13-014]).
    fn value_label(self) -> &'static str {
        match self {
            Self::Template => "a trigger[:action] spec",
            Self::Validate | Self::Apply | Self::Sync => "a file path or -",
            Self::Delete | Self::Enable | Self::Disable | Self::Reset => "a rule id",
            Self::Export => "an output file path",
            Self::Share => "a rule id or name",
            Self::ImportShare => "a share code, file path or -",
            Self::Help | Self::Example | Self::Schema | Self::Reference | Self::List => {
                "no argument"
            }
        }
    }
}

fn is_known_flag(arg: &str) -> bool {
    Command::from_flag(arg).is_some() || MODIFIER_FLAGS.contains(&arg)
}

/// A validated invocation: exactly one command and its optional value.
#[derive(Debug, PartialEq, Eq)]
struct Invocation {
    command: Command,
    value: Option<String>,
}

/// Result of an upsert: which rule ids were created and which replaced an existing rule.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ApplyReport {
    pub created: Vec<String>,
    pub updated: Vec<String>,
}

/// Entry point from `main`. Returns `false` when the arguments are not a CLI authoring command.
pub fn run(args: &[String]) -> bool {
    if !args.iter().any(|arg| is_known_flag(arg)) {
        // A mistyped `--rules-*` command must not fall through to update cleanup, UAC
        // self-elevation and a daemon/GUI start: fail closed with usage instead ([U13-001]).
        if has_unresolved_rule_command(args) {
            ensure_console();
            crate::logging::error("CLI", "Unknown rule command");
            eprintln!("error: unknown rule command");
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
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

/// True when argv contains a `--rules*` spelling that matches no command while no known flag is
/// present. `main` used to fall through to self-elevation and a GUI start for these ([U13-001]).
fn has_unresolved_rule_command(args: &[String]) -> bool {
    !args.iter().any(|arg| is_known_flag(arg))
        && args.iter().any(|arg| arg.starts_with("--rules"))
}

/// Records which command was requested. Only the flag and its path/id argument are logged, never
/// rule JSON, script bodies or other content read from the file.
fn log_invocation(args: &[String]) -> String {
    let Some(flag) = args.iter().find(|arg| Command::from_flag(arg).is_some()) else {
        return "unknown command".to_string();
    };
    let Some(command) = Command::from_flag(flag) else {
        return "unknown command".to_string();
    };
    let label = if command == Command::ImportShare {
        // Never log the code itself: it contains the user's automation definitions.
        flag.clone()
    } else {
        match command_argument_value(args, flag, command) {
            Some(value) if !value.is_empty() => format!("{flag} {value}"),
            _ => flag.clone(),
        }
    };
    crate::logging::info("CLI", &format!("Invoked {label}"));
    label
}

fn dispatch(args: &[String]) -> i32 {
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
fn resolve_invocation(args: &[String], json_output: bool) -> Result<Invocation, i32> {
    parse_invocation(args).map_err(|message| usage_error(json_output, &message))
}

/// Resolves exactly one command and validates every other argument. Conflicting or repeated
/// commands, a missing value and arguments that are neither that command's value nor a known flag
/// are usage errors ([U13-001], [U13-002]).
fn parse_invocation(args: &[String]) -> Result<Invocation, String> {
    let mut selected: Option<(Command, &str)> = None;
    for arg in args {
        let Some(command) = Command::from_flag(arg) else {
            continue;
        };
        match selected {
            Some((existing, existing_flag)) if existing == command => {
                return Err(format!(
                    "{existing_flag} and {arg} select the same command; give it once"
                ));
            }
            Some((_, existing_flag)) => {
                return Err(format!("conflicting commands: {existing_flag} and {arg}"));
            }
            None => selected = Some((command, arg.as_str())),
        }
    }
    let Some((command, flag)) = selected else {
        return Err("no command given".to_string());
    };
    let flag_position = args
        .iter()
        .position(|arg| arg == flag)
        .expect("the selected command flag is present");

    let value = match command.argument() {
        CommandArgument::None => None,
        CommandArgument::Required => Some(required_value(args, flag_position, flag, command)?),
        CommandArgument::Optional => command_argument_value(args, flag, command),
    };
    let value_position = value
        .as_ref()
        .filter(|_| command.argument() != CommandArgument::None)
        .map(|_| flag_position + 1);

    // Everything else must be a modifier. A typo like `--dry-runn` used to be ignored, so the
    // command ran for real; rejecting it is the point of [U13-001].
    for (position, argument) in args.iter().enumerate() {
        if position == flag_position || Some(position) == value_position {
            continue;
        }
        if MODIFIER_FLAGS.contains(&argument.as_str()) {
            continue;
        }
        return Err(format!("unexpected argument \"{argument}\""));
    }

    Ok(Invocation { command, value })
}

/// The value attached to `flag`, if the next argument can be a value. `-` is stdin.
fn command_argument_value(args: &[String], flag: &str, command: Command) -> Option<String> {
    if command.argument() == CommandArgument::None {
        return None;
    }
    let position = args.iter().position(|arg| arg == flag)?;
    args.get(position + 1)
        .filter(|value| value.as_str() == "-" || !value.starts_with('-'))
        .cloned()
}

fn required_value(
    args: &[String],
    flag_position: usize,
    flag: &str,
    command: Command,
) -> Result<String, String> {
    let label = command.value_label();
    match args.get(flag_position + 1) {
        Some(value) if value == "-" || !value.starts_with('-') => Ok(value.clone()),
        Some(value) => Err(format!("{flag} requires {label}, not \"{value}\"")),
        None => Err(format!("{flag} requires {label}")),
    }
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

/// Applies an upsert through the running daemon. Shared by the daemon-online path and the race
/// where the daemon starts after the CLI took its offline snapshot ([U13-003]).
fn apply_via_daemon(
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

/// Replaces the rule set through the running daemon as one declarative request: it persists once
/// and reloads hotkeys once instead of an upsert followed by a delete per rule ([U13-003]).
fn sync_via_daemon(
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

// ---------------------------------------------------------------------------
// Share codes
// ---------------------------------------------------------------------------

/// Prints a `SYN1:` share code for one rule (by id or name) or for every rule.
fn cmd_share(selector: Option<&str>, json_output: bool) -> i32 {
    let cfg = match current_config() {
        Ok((cfg, _)) => cfg,
        Err(err) => return fail_with(json_output, &err),
    };

    let rules = match selector {
        Some(selector) => match cfg
            .rules
            .iter()
            .find(|rule| rule.id == selector || rule.name.eq_ignore_ascii_case(selector))
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

    let warning_texts: Vec<String> = prepared
        .warnings
        .iter()
        .map(share_warning_text_cli)
        .collect();
    let errors = validate_rules(&prepared.rules);
    if !errors.is_empty() {
        log_errors(&errors);
        if json_output {
            return fail_validation(&errors, &warning_texts, target_name(from_daemon));
        }
        for error in &errors {
            eprintln!("error: {error}");
        }
        eprintln!("Nothing was imported.");
        return 1;
    }

    let mut target = target_name(from_daemon);
    let summary = if from_daemon {
        match import_share_via_daemon(&payload, enable, json_output) {
            Ok(summary) => summary,
            Err(exit_code) => return exit_code,
        }
    } else if IpcClient::is_daemon_running() {
        // The daemon became reachable after the snapshot: one IPC request gets the import right
        // instead of writing the sections under it ([U13-003]).
        target = "daemon";
        match import_share_via_daemon(&payload, enable, json_output) {
            Ok(summary) => summary,
            Err(exit_code) => return exit_code,
        }
    } else {
        match import_share_offline(&cfg, &prepared, enable, json_output) {
            Ok(summary) => summary,
            Err(exit_code) => return exit_code,
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
            "target": target,
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
        if target == "rules" {
            println!("Applied to {} (daemon is offline).", RulesFile::path().display());
        }
    }
    0
}

/// Sends one share import through the running daemon.
fn import_share_via_daemon(
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

/// Writes a share import straight to the config sections. The library is committed first, because
/// a rule that references a preset or overlay the library never received is worse than an unused
/// library entry; a failed rules write is reported as a partial commit ([U13-005]).
fn import_share_offline(
    cfg: &AppConfig,
    prepared: &crate::sharing::PreparedShareImport,
    enable: bool,
    json_output: bool,
) -> Result<crate::sharing::ShareImportSummary, i32> {
    let library = merged_library(prepared, crate::config::Library::read());
    if let Some(library) = &library {
        if let Err(err) = library.save() {
            return Err(fail_with(
                json_output,
                &format!("cannot write library: {err}"),
            ));
        }
    }
    // Re-check immediately before the second write: a daemon that started in between would be
    // overwritten by this whole-section rewrite ([U13-003]).
    if IpcClient::is_daemon_running() {
        return Err(fail_with(
            json_output,
            "the daemon started while the import was running; the library was updated but the rules were not imported - re-run the command so it goes through IPC",
        ));
    }
    let mut rules = cfg.rules.clone();
    rules.extend(prepared.rules.iter().cloned());
    if let Err(err) = save_rules(&rules) {
        if library.is_some() {
            return Err(fail_with(
                json_output,
                &format!("library was updated but rules could not be written: {err}"),
            ));
        }
        return Err(fail_with(json_output, &err));
    }
    Ok(crate::sharing::ShareImportSummary {
        imported: prepared.rules.len(),
        skipped_duplicates: prepared.skipped_duplicates,
        overlays: prepared.overlays.len(),
        macro_presets: prepared.macro_presets.len(),
        enabled: enable,
    })
}

/// Merges the library entries a share payload references into `library`, returning the snapshot to
/// write. `None` when the payload carries no library entries, so the section is left untouched
/// ([U13-005]).
fn merged_library(
    prepared: &crate::sharing::PreparedShareImport,
    mut library: crate::config::Library,
) -> Option<crate::config::Library> {
    if prepared.overlays.is_empty() && prepared.macro_presets.is_empty() {
        return None;
    }
    library.overlays.extend(prepared.overlays.iter().cloned());
    library.macro_presets.extend(prepared.macro_presets.iter().cloned());
    Some(library)
}

/// A share argument is a literal `SYN1:` code, a file path, or `-` for stdin. Anything else is a
/// path, so a mistyped path reports a read error instead of a misleading decode error ([U13-015]).
fn read_share_argument(value: &str) -> Result<String, String> {
    if value.starts_with(crate::sharing::SHARE_PREFIX) {
        Ok(value.to_string())
    } else {
        read_input(value)
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
        W::OverlayAssetDropped => "overlay images and icons are not included".to_string(),
        W::MissingRule(id) => {
            format!("rule \"{id}\" is missing; overlay buttons that use it will not work")
        }
    }
}

// ---------------------------------------------------------------------------
// Schema and reference discovery
// ---------------------------------------------------------------------------

/// Generates the draft-07 JSON Schema for one rule object.
fn rule_schema() -> Value {
    schemars::generate::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<crate::models::Rule>()
        .to_value()
}

fn cmd_schema() -> i32 {
    match serde_json::to_string_pretty(&rule_schema()) {
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
    let root = rule_schema();
    let mut out = String::new();
    out.push_str("Synector rule reference\n");
    out.push_str("Machine-readable draft-07 JSON Schema: Synector.exe --rules-schema\n\n");
    out.push_str("RULE FIELDS\n");
    if let Some(properties) = root.get("properties").and_then(Value::as_object) {
        let required = required_names(&root);
        render_fields(&root, properties, &required, &mut out);
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
        render_variants(&root, definition, tag, &mut out);
    }
    out.push_str("\nNested objects (overlays, tags, branches) are described by --rules-schema.\n");
    out
}

fn required_names(schema: &Value) -> std::collections::BTreeSet<String> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .map(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn definition<'a>(root: &'a Value, name: &str) -> Option<&'a Value> {
    root.get("definitions")?.get(name)
}

fn definition_variants<'a>(root: &'a Value, name: &str) -> &'a [Value] {
    definition(root, name)
        .and_then(|definition| definition.get("oneOf"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn render_fields(
    root: &Value,
    properties: &Map<String, Value>,
    required: &std::collections::BTreeSet<String>,
    out: &mut String,
) {
    for (name, schema) in properties {
        let marker = if is_required(required, name) {
            " (required)"
        } else {
            ""
        };
        out.push_str(&format!("  {name}: {}{marker}\n", type_label(root, schema, 0)));
    }
}

fn render_variants(root: &Value, definition: &str, tag: &str, out: &mut String) {
    for variant in definition_variants(root, definition) {
        let Some(properties) = variant.get("properties").and_then(Value::as_object) else {
            continue;
        };
        let variant_name = properties
            .get(tag)
            .and_then(tag_value)
            .unwrap_or_else(|| "<unknown>".to_string());
        let required = required_names(variant);
        let mut fields = Vec::new();
        for (name, schema) in properties {
            if name == tag {
                continue;
            }
            let marker = if is_required(&required, name) {
                " (required)"
            } else {
                ""
            };
            fields.push(format!("{name}: {}{marker}", type_label(root, schema, 0)));
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

fn tag_value(schema: &Value) -> Option<String> {
    if let Some(value) = schema.get("const").and_then(Value::as_str) {
        return Some(value.to_string());
    }
    schema
        .get("enum")
        .and_then(Value::as_array)
        .and_then(|values| values.first())
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn type_label(root: &Value, schema: &Value, depth: usize) -> String {
    if depth > 6 {
        return "...".to_string();
    }
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let name = reference.rsplit('/').next().unwrap_or(reference);
        if let Some(definition) = definition(root, name) {
            if let Some(values) = definition.get("enum").and_then(Value::as_array) {
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
    if let Some(instance_types) = schema.get("type") {
        return match instance_types {
            Value::String(instance_type) if instance_type == "array" => {
                array_label(root, schema, depth)
            }
            Value::String(instance_type) => instance_type.clone(),
            Value::Array(types) => types
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" or "),
            _ => "any".to_string(),
        };
    }
    if schema.get("items").is_some() {
        return array_label(root, schema, depth);
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        if values.iter().all(Value::is_string) {
            return values
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("|");
        }
    }
    if let Some(candidates) = schema
        .get("anyOf")
        .or_else(|| schema.get("oneOf"))
        .and_then(Value::as_array)
    {
        let labels: Vec<String> = candidates
            .iter()
            .map(|candidate| type_label(root, candidate, depth + 1))
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
    if let Some(first) = schema
        .get("allOf")
        .and_then(Value::as_array)
        .and_then(|schemas| schemas.first())
    {
        return type_label(root, first, depth + 1);
    }
    if schema.get("properties").is_some() {
        return "object".to_string();
    }
    "any".to_string()
}

fn array_label(root: &Value, schema: &Value, depth: usize) -> String {
    match schema.get("items") {
        Some(Value::Array(items)) => items
            .first()
            .map(|first| format!("array of {}", type_label(root, first, depth + 1)))
            .unwrap_or_else(|| "array".to_string()),
        Some(items @ Value::Object(_)) => {
            format!("array of {}", type_label(root, items, depth + 1))
        }
        _ => "array".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Rule templates
// ---------------------------------------------------------------------------

fn cmd_template(spec: Option<&str>) -> i32 {
    let root = rule_schema();
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
fn template_rule(root: &Value, trigger: &str, action: &str) -> Result<Value, String> {
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

fn variant_names(root: &Value, definition: &str, tag: &str) -> Vec<String> {
    definition_variants(root, definition)
        .iter()
        .filter_map(|variant| {
            variant
                .get("properties")
                .and_then(|properties| properties.get(tag))
                .and_then(tag_value)
        })
        .collect()
}

fn variant_value(
    root: &Value,
    definition: &str,
    tag: &str,
    variant_name: &str,
    depth: usize,
) -> Result<Value, String> {
    if depth > 8 {
        return Ok(Value::Null);
    }
    let variants = definition_variants(root, definition);
    if variants.is_empty() {
        return Err(format!("schema is missing the {definition} definition"));
    }
    let variant = variants
        .iter()
        .find(|variant| {
            variant
                .get("properties")
                .and_then(|properties| properties.get(tag))
                .and_then(tag_value)
                .as_deref()
                == Some(variant_name)
        })
        .ok_or_else(|| format!("unknown {definition} variant \"{variant_name}\""))?;
    let properties = variant
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{definition} variant \"{variant_name}\" is not an object"))?;
    let mut fields = serde_json::Map::new();
    fields.insert(tag.to_string(), Value::String(variant_name.to_string()));
    for required in required_names(variant) {
        if required == tag {
            continue;
        }
        if let Some(schema) = properties.get(&required) {
            fields.insert(
                required.clone(),
                placeholder_value(root, schema, &required, depth + 1),
            );
        }
    }
    for (name, value) in extra_fields(variant_name) {
        if properties.contains_key(name) {
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

fn placeholder_value(root: &Value, schema: &Value, field_name: &str, depth: usize) -> Value {
    if depth > 8 {
        return Value::Null;
    }
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let name = reference.rsplit('/').next().unwrap_or(reference);
        if let Some(definition) = definition(root, name) {
            if let Some(values) = definition.get("enum").and_then(Value::as_array) {
                if let Some(first) = values.first() {
                    return first.clone();
                }
            }
            if let Some(first) = definition_variants(root, name).first() {
                if let Some(properties) = first.get("properties").and_then(Value::as_object) {
                    if let Some(tag) = discover_tag_key(properties) {
                        if let Some(tag_name) = properties.get(&tag).and_then(tag_value) {
                            if let Ok(value) = variant_value(root, name, &tag, &tag_name, depth + 1)
                            {
                                return value;
                            }
                        }
                    }
                }
            }
            if let Some(properties) = definition.get("properties").and_then(Value::as_object) {
                let mut fields = serde_json::Map::new();
                for required in required_names(definition) {
                    if let Some(field_schema) = properties.get(&required) {
                        fields.insert(
                            required.clone(),
                            placeholder_value(root, field_schema, &required, depth + 1),
                        );
                    }
                }
                return Value::Object(fields);
            }
        }
        return Value::String(name.to_string());
    }
    if let Some(instance_types) = schema.get("type") {
        return match instance_types {
            Value::String(instance_type) => single_type_placeholder(instance_type, field_name),
            Value::Array(instance_types) => instance_types
                .iter()
                .filter_map(Value::as_str)
                .find(|instance_type| *instance_type != "null")
                .map(|instance_type| single_type_placeholder(instance_type, field_name))
                .unwrap_or(Value::Null),
            _ => Value::Null,
        };
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        if let Some(first) = values.first() {
            return first.clone();
        }
    }
    if let Some(candidates) = schema
        .get("anyOf")
        .or_else(|| schema.get("oneOf"))
        .and_then(Value::as_array)
    {
        for candidate in candidates {
            if is_null_type(candidate) {
                continue;
            }
            return placeholder_value(root, candidate, field_name, depth + 1);
        }
    }
    if let Some(first) = schema
        .get("allOf")
        .and_then(Value::as_array)
        .and_then(|schemas| schemas.first())
    {
        return placeholder_value(root, first, field_name, depth + 1);
    }
    if schema.get("items").is_some() {
        return Value::Array(Vec::new());
    }
    if schema.get("properties").is_some() {
        return Value::Object(serde_json::Map::new());
    }
    Value::String("value".to_string())
}

fn single_type_placeholder(instance_type: &str, field_name: &str) -> Value {
    match instance_type {
        "null" => Value::Null,
        "boolean" => Value::Bool(false),
        "integer" => Value::from(1),
        "number" => Value::from(1.0),
        "string" => Value::String(string_placeholder(field_name)),
        "array" => Value::Array(Vec::new()),
        "object" => Value::Object(serde_json::Map::new()),
        _ => Value::Null,
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

fn discover_tag_key(object: &Map<String, Value>) -> Option<String> {
    object.iter().find_map(|(name, schema)| {
        let single_value_enum = schema
            .get("enum")
            .and_then(Value::as_array)
            .map(|values| values.len() == 1)
            .unwrap_or(false);
        (schema.get("const").is_some() || single_value_enum).then(|| name.clone())
    })
}

fn is_null_type(schema: &Value) -> bool {
    match schema.get("type") {
        Some(Value::String(instance_type)) => instance_type == "null",
        Some(Value::Array(instance_types)) => {
            !instance_types.is_empty()
                && instance_types
                    .iter()
                    .all(|instance_type| instance_type.as_str() == Some("null"))
        }
        _ => false,
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

/// Deletes one rule through the running daemon, checking that it exists first.
fn delete_via_daemon(id: &str, json_output: bool) -> i32 {
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

fn toggle_via_daemon(id: &str, enabled: bool, json_output: bool) -> i32 {
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

fn reset_via_daemon(id: &str, json_output: bool) -> i32 {
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

// ---------------------------------------------------------------------------
// Rule file parsing and validation
// ---------------------------------------------------------------------------

/// Parses a rule file: a single rule object, an array of rules, or `{"rules": [...]}`.
pub fn parse_rules_json(raw: &str) -> Result<Vec<Rule>, String> {
    parse_rule_file(raw, false)
}

/// Parses a rule file for `--rules-sync`. An empty array is the exact representation of "no rules"
/// and must be accepted, otherwise a declarative sync can never delete every rule ([U13-009]);
/// `--rules-apply` and `--rules-validate` keep rejecting an empty set.
fn parse_rules_json_for_sync(raw: &str) -> Result<Vec<Rule>, String> {
    parse_rule_file(raw, true)
}

fn parse_rule_file(raw: &str, allow_empty: bool) -> Result<Vec<Rule>, String> {
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
    if items.is_empty() && !allow_empty {
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
        let rule_value = Value::Object(object);
        if let Err(unknown) = crate::models::validate_rule_keys(&rule_value) {
            let details = unknown
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ");
            return Err(format!("rule #{number} ({name}): {details}"));
        }
        let mut rule: Rule = serde_json::from_value(rule_value)
            .map_err(|err| format!("rule #{number} ({name}): {err}"))?;
        rule.normalize();
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
        if let Some(period) = rule.quota_period {
            if let Some(message) = period.validation_error() {
                errors.push(format!(
                    "rule #{number} ({}): {message}",
                    display_id(rule)
                ));
            }
        }

        validate_trigger(&rule.trigger, number, rule, &mut errors);

        for (condition_index, item) in rule.conditions.iter().enumerate() {
            for message in item.condition.validation_errors() {
                errors.push(format!(
                    "rule #{number} ({}): condition #{}: {message}",
                    display_id(rule),
                    condition_index + 1
                ));
            }
        }
        if let Some(message) = rule.tag_color_error() {
            errors.push(format!(
                "rule #{number} ({}): {message}",
                display_id(rule)
            ));
        }

        let mut has_infinite_macro = false;
        let mut check = |action: &Action| {
            if matches!(action, Action::Macro { infinite: true, .. }) {
                has_infinite_macro = true;
            }
            for message in action.validation_errors() {
                errors.push(format!(
                    "rule #{number} ({}): action: {message}",
                    display_id(rule)
                ));
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
            for message in crate::models::resource_threshold_errors(
                process_name,
                *cpu_percent,
                *ram_mb,
            ) {
                errors.push(format!("rule #{number} ({id}): {message}"));
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
        Some(trigger @ Trigger::Schedule { .. }) => {
            for message in trigger.schedule_errors() {
                errors.push(format!("rule #{number} ({id}): {message}"));
            }
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
                "rule #{number} ({id}): no trigger, it runs only on demand (--run-rule, the GUI, or a rule_control call)"
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

        let rule_exists = |target: &str| {
            cfg.rules.iter().any(|r| r.id == target) || rules.iter().any(|r| r.id == target)
        };
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
                "rule #{number} ({id}): an until_condition wait without a condition returns immediately"
            )),
            Action::Wait {
                mode: crate::models::WaitMode::UntilCondition,
                condition: Some(
                    crate::models::WaitCondition::RuleStarted { rule_id }
                    | crate::models::WaitCondition::RuleStopped { rule_id },
                ),
                ..
            } => {
                let target = rule_id.trim();
                if target.is_empty() {
                    warnings.push(format!(
                        "rule #{number} ({id}): wait for a rule has an empty rule_id"
                    ));
                } else if !rule_exists(target) {
                    warnings.push(format!(
                        "rule #{number} ({id}): wait target \"{target}\" does not exist"
                    ));
                }
            }
            Action::Check {
                condition: crate::models::RuleCondition::RuleRunning { rule_id, .. },
            } => {
                let target = rule_id.trim();
                if target.is_empty() {
                    warnings.push(format!(
                        "rule #{number} ({id}): check has an empty rule_id"
                    ));
                } else if !rule_exists(target) {
                    warnings.push(format!(
                        "rule #{number} ({id}): check target \"{target}\" does not exist"
                    ));
                }
            }
            Action::StartProcess { file_path, .. } if file_path.trim().is_empty() => warnings.push(
                format!("rule #{number} ({id}): start_process has an empty file_path"),
            ),
            Action::RuleControl { rule_id, operation, .. } => {
                let target = rule_id.trim();
                if target.is_empty() {
                    warnings.push(format!(
                        "rule #{number} ({id}): rule_control has an empty rule_id"
                    ));
                } else {
                    if matches!(operation, crate::models::RuleControlOperation::Call)
                        && target == rule.id
                    {
                        warnings.push(format!(
                            "rule #{number} ({id}): rule_control calls the rule itself; the engine rejects the cycle"
                        ));
                    }
                    if !rule_exists(target) {
                        warnings.push(format!(
                            "rule #{number} ({id}): rule_control target \"{target}\" does not exist"
                        ));
                    }
                }
            }
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

/// One `--json` failure envelope for every failure path, so a machine consumer can always read
/// `.ok`, `.error`, `.errors`, `.warnings` and `.target` regardless of the failure class
/// ([U13-011]).
fn error_json(
    error: &str,
    errors: &[String],
    warnings: &[String],
    target: Option<&str>,
) -> Value {
    json!({
        "ok": false,
        "error": error,
        "errors": errors,
        "warnings": warnings,
        "target": target,
    })
}

fn emit_error_json(
    error: &str,
    errors: &[String],
    warnings: &[String],
    target: Option<&str>,
) -> i32 {
    emit_json(error_json(error, errors, warnings, target));
    1
}

/// Reports validation errors in `--json` mode. The first error is promoted to `error`; the full
/// list stays in `errors` ([U13-011]).
fn fail_validation(errors: &[String], warnings: &[String], target: &str) -> i32 {
    let error = errors
        .first()
        .map(String::as_str)
        .unwrap_or("validation failed");
    emit_error_json(error, errors, warnings, Some(target))
}

fn fail_with(json_output: bool, message: &str) -> i32 {
    if json_output {
        crate::logging::error("CLI", message);
        emit_error_json(message, &[message.to_string()], &[], None)
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
            previous
                .iter()
                .find(|old| old.id == rule.id)
                .is_some_and(|old| rule.quota_redefined_from(old))
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

/// Removes one rule by id and returns it, so the caller can drop its runtime counters ([U13-004]).
fn take_rule(rules: &mut Vec<Rule>, id: &str) -> Option<Rule> {
    let position = rules.iter().position(|rule| rule.id == id)?;
    Some(rules.remove(position))
}

/// Writes text through a sibling temp file and a rename, the same way the config sections do, so a
/// crash or full disk during `--rules-export <file>` never leaves the previous export truncated
/// ([U13-013]).
fn write_text_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp_name = path.as_os_str().to_os_string();
    tmp_name.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp_name);
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
    }
    // Antivirus and indexers can hold a fresh file open for a moment on Windows; a short retry
    // keeps a transient share violation from losing the write.
    let mut last_error = None;
    for attempt in 0..3 {
        match std::fs::rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(err) => last_error = Some(err),
        }
        if attempt < 2 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    let _ = std::fs::remove_file(&tmp);
    Err(last_error.unwrap_or_else(|| std::io::Error::other("atomic rename failed")))
}

fn fail(message: &str) -> i32 {
    crate::logging::error("CLI", message);
    eprintln!("error: {message}");
    1
}

/// Release builds use the Windows GUI subsystem, so a CLI invocation has to reattach to the
/// parent console before the first line is printed. Redirected std handles (pipes) are left
/// untouched so machine-readable output keeps working.
pub(crate) fn ensure_console() {
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
    use crate::models::{MacroOperation, ScheduleType, WaitMode};

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
    fn test_parse_rejects_unknown_keys() {
        let raw = r#"[{"id":"a","name":"A","max_execution":5,"actions":[]}]"#;
        let err = parse_rules_json(raw).expect_err("typo must be rejected");
        assert!(err.contains("max_execution"), "{err}");

        let nested = r#"[{"id":"a","name":"A","actions":[{"$action_type":"notification","title":"T","message":"M","mesage":"typo"}]}]"#;
        let err = parse_rules_json(nested).expect_err("nested typo must be rejected");
        assert!(err.contains("mesage"), "{err}");
    }

    #[test]
    fn test_validate_rules_schedule_quota_and_wait_errors() {
        let mut daily = rule_with_notification("daily");
        daily.trigger = Some(Trigger::Schedule {
            schedule_type: ScheduleType::Daily,
            interval_sec: 0,
            daily_time: None,
            days_of_week: vec![],
            day_of_month: None,
            target_datetime: None,
        });
        let errors = validate_rules(&[daily]);
        assert!(
            errors.iter().any(|error| error.contains("daily_time")),
            "{errors:?}"
        );

        let mut wait = rule_with_notification("wait");
        wait.actions = vec![Action::Wait {
            mode: WaitMode::UntilCondition,
            seconds: 0.0,
            condition: None,
            timeout_sec: 30.0,
            timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
        }];
        let errors = validate_rules(&[wait]);
        assert!(
            errors.iter().any(|error| error.contains("needs a condition")),
            "{errors:?}"
        );

        let mut quota = rule_with_notification("quota");
        quota.max_executions = Some(1);
        quota.quota_period = Some(QuotaPeriod::Custom {
            value: u32::MAX,
            unit: crate::models::QuotaUnit::Minutes,
        });
        let errors = validate_rules(&[quota]);
        assert!(
            errors.iter().any(|error| error.contains("between 1 and 100000")),
            "{errors:?}"
        );
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
            errors
                .iter()
                .any(|e| e.contains("custom quota window length must be between 1 and 100000")),
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
            warnings
                .iter()
                .any(|w| w.contains("until_condition wait without a condition")),
            "{warnings:?}"
        );
    }

    #[test]
    fn test_warnings_for_rule_activity_targets() {
        let cfg = AppConfig::default();
        let mut rule = rule_with_notification("activity_rule");
        rule.actions.push(Action::Wait {
            mode: WaitMode::UntilCondition,
            seconds: 0.0,
            condition: Some(crate::models::WaitCondition::RuleStarted {
                rule_id: String::new(),
            }),
            timeout_sec: 5.0,
            timeout_policy: Default::default(),
        });
        rule.actions.push(Action::Wait {
            mode: WaitMode::UntilCondition,
            seconds: 0.0,
            condition: Some(crate::models::WaitCondition::RuleStopped {
                rule_id: "ghost_rule".to_string(),
            }),
            timeout_sec: 5.0,
            timeout_policy: Default::default(),
        });
        rule.actions.push(Action::Check {
            condition: crate::models::RuleCondition::RuleRunning {
                rule_id: "ghost_rule".to_string(),
                must_be_running: true,
            },
        });

        let warnings = rule_warnings(&[rule], &cfg);
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("wait for a rule has an empty rule_id")),
            "{warnings:?}"
        );
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("wait target \"ghost_rule\" does not exist")),
            "{warnings:?}"
        );
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("check target \"ghost_rule\" does not exist")),
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
        let json = rule_schema();
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
        let root = rule_schema();
        let reference = render_reference();
        for (definition, tag) in [("Trigger", "$trigger_type"), ("Action", "$action_type")] {
            let variants = definition_variants(&root, definition);
            assert!(variants.len() >= 9, "{definition:?} has too few variants");
            for variant in variants {
                let properties = variant
                    .get("properties")
                    .and_then(Value::as_object)
                    .expect("variant object");
                let name = properties
                    .get(tag)
                    .and_then(tag_value)
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
        let root = rule_schema();
        let triggers = variant_names(&root, "Trigger", "$trigger_type");
        let actions = variant_names(&root, "Action", "$action_type");
        assert_eq!(triggers.len(), 9, "{triggers:?}");
        assert_eq!(actions.len(), 26, "{actions:?}");
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
        let root = rule_schema();
        assert!(template_rule(&root, "nope", "notification").is_err());
        assert!(template_rule(&root, "manual", "nope").is_err());
    }

    #[test]
    fn test_template_play_sound_roundtrip() {
        let root = rule_schema();
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

    fn cli_args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn test_unknown_argument_is_rejected_with_usage_exit_code() {
        // U13-001: `--dry-runn` next to a real command used to be ignored and the apply ran.
        let unknown = cli_args(&["--rules-apply", "rules.json", "--dry-runn"]);
        let message = parse_invocation(&unknown).expect_err("unknown flag");
        assert!(
            message.contains("unexpected argument \"--dry-runn\""),
            "{message}"
        );
        assert_eq!(resolve_invocation(&unknown, false).expect_err("usage exit code"), 2);

        // A misspelled command with no known flag must be claimed by the CLI and fail closed
        // instead of falling through to elevation and a GUI start.
        let misspelled = cli_args(&["--rules-appply", "rules.json"]);
        assert!(has_unresolved_rule_command(&misspelled));
        assert_eq!(
            parse_invocation(&misspelled).expect_err("no command"),
            "no command given"
        );
        assert!(resolve_invocation(&misspelled, false).is_err());

        assert!(!has_unresolved_rule_command(&cli_args(&[
            "--rules-apply",
            "rules.json"
        ])));
        assert!(!has_unresolved_rule_command(&cli_args(&["--json"])));
        assert!(!has_unresolved_rule_command(&cli_args(&[])));
    }

    #[test]
    fn test_conflicting_and_repeated_commands_are_rejected() {
        // U13-002: the ordered if-chain silently preferred the first command it matched.
        let conflicting = cli_args(&["--rules-validate", "a.json", "--rules-apply", "b.json"]);
        let message = parse_invocation(&conflicting).expect_err("conflict");
        assert!(message.contains("conflicting commands"), "{message}");
        assert!(message.contains("--rules-validate"), "{message}");
        assert!(message.contains("--rules-apply"), "{message}");

        let repeated = cli_args(&["--rules-apply", "a.json", "--rules-apply", "b.json"]);
        let message = parse_invocation(&repeated).expect_err("repeated command");
        assert!(message.contains("select the same command"), "{message}");

        assert!(parse_invocation(&cli_args(&["--help", "--rules-list"])).is_err());

        let invocation =
            parse_invocation(&cli_args(&["--rules-apply", "rules.json", "--json", "--dry-run"]))
                .expect("valid invocation");
        assert_eq!(invocation.command, Command::Apply);
        assert_eq!(invocation.value.as_deref(), Some("rules.json"));
    }

    #[test]
    fn test_command_values_are_validated_per_command() {
        // U13-014: the error names the value the flag actually takes.
        let message = parse_invocation(&cli_args(&["--rules-delete"])).expect_err("missing id");
        assert!(
            message.contains("--rules-delete requires a rule id"),
            "{message}"
        );

        let message =
            parse_invocation(&cli_args(&["--rules-delete", "--json"])).expect_err("flag as id");
        assert!(
            message.contains("--rules-delete requires a rule id"),
            "{message}"
        );
        assert!(message.contains("--json"), "{message}");

        let message = parse_invocation(&cli_args(&["--rules-import-share", "--json"]))
            .expect_err("missing share argument");
        assert!(message.contains("a share code, file path or -"), "{message}");

        let invocation =
            parse_invocation(&cli_args(&["--rules-export", "rules.json"])).expect("export path");
        assert_eq!(invocation.value.as_deref(), Some("rules.json"));
        let invocation = parse_invocation(&cli_args(&["--rules-export"])).expect("export stdout");
        assert_eq!(invocation.value, None);
        let invocation =
            parse_invocation(&cli_args(&["--rules-list", "--json"])).expect("list has no value");
        assert_eq!(invocation.command, Command::List);
        assert_eq!(invocation.value, None);
    }

    #[test]
    fn test_json_error_envelope_shape() {
        // U13-011: validation failures used to emit {ok:false, errors} with no "error" key.
        let errors = vec!["rule #1 (typo): bad".to_string()];
        let warnings = vec!["no trigger".to_string()];
        let value = error_json("rule #1 (typo): bad", &errors, &warnings, Some("rules"));
        assert_eq!(value["ok"], serde_json::json!(false));
        assert_eq!(value["error"], serde_json::json!("rule #1 (typo): bad"));
        assert_eq!(value["errors"][0], serde_json::json!("rule #1 (typo): bad"));
        assert_eq!(value["warnings"][0], serde_json::json!("no trigger"));
        assert_eq!(value["target"], serde_json::json!("rules"));

        let value = error_json("boom", &["boom".to_string()], &[], None);
        assert!(value["target"].is_null());
        assert!(
            value.as_object().expect("object").contains_key("target"),
            "every failure envelope names the target key"
        );
    }

    #[test]
    fn test_sync_accepts_an_empty_rule_set_but_apply_rejects_it() {
        // U13-009: sync makes the configured set match the file, including "no rules at all".
        assert!(parse_rules_json("[]").is_err());
        assert!(parse_rules_json_for_sync("[]").expect("empty array").is_empty());
        assert!(
            parse_rules_json_for_sync(r#"{"rules": []}"#)
                .expect("empty wrapper")
                .is_empty()
        );
        assert!(
            parse_rules_json_for_sync("").is_err(),
            "an empty file is still malformed"
        );

        let existing = vec![rule_with_notification("keep"), rule_with_notification("gone")];
        let (report, deleted) = plan_sync(&existing, &[]);
        assert!(report.created.is_empty() && report.updated.is_empty());
        assert_eq!(deleted, vec!["keep".to_string(), "gone".to_string()]);
    }

    #[test]
    fn test_offline_updates_drop_removed_runtime_counters() {
        // U13-004: a removed rule's counters must be pruned, otherwise a rule re-created with the
        // same id inherits an exhausted quota and a stale last_executed_at.
        let mut rules = vec![rule_with_notification("keep"), rule_with_notification("gone")];
        let removed = take_rule(&mut rules, "gone").expect("rule is removed");
        assert_eq!(removed.id, "gone");
        assert!(rules.iter().all(|rule| rule.id != "gone"));
        assert!(
            take_rule(&mut rules, "gone").is_none(),
            "delete reports a missing id"
        );

        let existing = vec![rule_with_notification("keep"), rule_with_notification("gone")];
        let applied = vec![rule_with_notification("keep")];
        let (_, deleted) = plan_sync(&existing, &applied);
        assert_eq!(deleted, vec!["gone".to_string()]);
    }

    #[test]
    fn test_import_plan_merges_library_entries_before_rules_are_written() {
        // U13-005: the library snapshot is built and written before rules.json so a failed second
        // write cannot leave imported rules pointing at missing presets/overlays.
        let mut prepared = crate::sharing::PreparedShareImport::default();
        prepared.rules.push(rule_with_notification("imported"));
        prepared.overlays.push(crate::models::OverlayDefinition::default());
        prepared
            .macro_presets
            .push(crate::models::default_macro_presets().remove(0));

        let base = crate::config::Library::default();
        let overlay_count = base.overlays.len();
        let preset_count = base.macro_presets.len();
        let library = merged_library(&prepared, base.clone()).expect("library entries");
        assert_eq!(library.overlays.len(), overlay_count + 1);
        assert_eq!(library.macro_presets.len(), preset_count + 1);

        let mut rules_only = crate::sharing::PreparedShareImport::default();
        rules_only.rules.push(rule_with_notification("imported"));
        assert!(merged_library(&rules_only, base).is_none());
    }

    #[test]
    fn test_export_write_replaces_the_target_and_leaves_no_temp_file() {
        // U13-013: the export previously truncated the target in place; an atomic sibling write
        // must replace it and clean up after itself.
        let dir = std::env::temp_dir().join(format!("synector_cli_export_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("rules.json");
        std::fs::write(&path, "old export").expect("previous export");

        write_text_atomic(&path, "new export\n").expect("atomic write");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read export"),
            "new export\n"
        );
        assert!(
            !dir.join("rules.json.tmp").exists(),
            "the sibling temp file must not survive"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_share_argument_is_literal_only_for_syn1_codes() {
        assert_eq!(
            read_share_argument("SYN1:abc").expect("literal"),
            "SYN1:abc"
        );
        // Anything that is not a SYN1: code is a path, so a typo reports a read error instead of
        // a misleading "invalid share code" ([U13-015]).
        let err = read_share_argument("synector_missing_code_file_12345")
            .expect_err("missing path must not become a literal");
        assert!(err.contains("cannot read"), "{err}");
        assert!(err.contains("synector_missing_code_file_12345"), "{err}");
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

    #[test]
    fn test_rule_control_warnings() {
        use crate::models::RuleControlOperation;

        let mut config = crate::config::AppConfig::default();
        config.rules.push(Rule {
            id: "existing".into(),
            name: "Existing".into(),
            ..Default::default()
        });

        let rules = vec![
            Rule {
                id: "caller".into(),
                name: "Caller".into(),
                actions: vec![
                    Action::RuleControl {
                        rule_id: "missing".into(),
                        operation: RuleControlOperation::Call,
                        wait: false,
                    },
                    Action::RuleControl {
                        rule_id: "caller".into(),
                        operation: RuleControlOperation::Call,
                        wait: false,
                    },
                    Action::RuleControl {
                        rule_id: String::new(),
                        operation: RuleControlOperation::Stop,
                        wait: false,
                    },
                    // On-disk and same-batch targets are both resolved.
                    Action::RuleControl {
                        rule_id: "existing".into(),
                        operation: RuleControlOperation::Stop,
                        wait: false,
                    },
                    Action::RuleControl {
                        rule_id: "batch_target".into(),
                        operation: RuleControlOperation::Call,
                        wait: false,
                    },
                ],
                ..Default::default()
            },
            Rule {
                id: "batch_target".into(),
                name: "Batch Target".into(),
                ..Default::default()
            },
        ];

        let warnings = rule_warnings(&rules, &config);
        assert!(
            warnings.iter().any(|warning| warning.contains("\"missing\" does not exist")),
            "{warnings:?}"
        );
        assert!(
            warnings.iter().any(|warning| warning.contains("calls the rule itself")),
            "{warnings:?}"
        );
        assert!(
            warnings.iter().any(|warning| warning.contains("empty rule_id")),
            "{warnings:?}"
        );
        assert!(
            !warnings.iter().any(|warning| {
                warning.contains("rule_control target \"existing\"")
                    || warning.contains("rule_control target \"batch_target\"")
            }),
            "valid targets must not be reported: {warnings:?}"
        );
    }
}
