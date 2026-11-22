//! Rule-file reading and parsing, rule-set planning and the offline `rules.json` / `runtime.json`
//! persistence helpers.

use std::io::{Read, Write};
use std::path::Path;

use serde_json::Value;

use crate::config::{AppConfig, RulesFile, RuntimeState};
use crate::ipc::IpcClient;
use crate::models::Rule;
#[cfg(test)]
use crate::models::{Action, Trigger};

use super::json::fail_with;

/// Result of an upsert: which rule ids were created and which replaced an existing rule.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ApplyReport {
    pub created: Vec<String>,
    pub updated: Vec<String>,
}

/// Parses a rule file: a single rule object, an array of rules, or `{"rules": [...]}`.
pub(super) fn parse_rules_json(raw: &str) -> Result<Vec<Rule>, String> {
    parse_rule_file(raw, false)
}

/// Parses a rule file for `--rules-sync`. An empty array is the exact representation of "no rules"
/// and must be accepted, otherwise a declarative sync can never delete every rule ([U13-009]);
/// `--rules-apply` and `--rules-validate` keep rejecting an empty set.
pub(super) fn parse_rules_json_for_sync(raw: &str) -> Result<Vec<Rule>, String> {
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

/// Upserts `incoming` into `existing` by id, returning what was created and what was replaced.
pub(super) fn upsert_rules(existing: &mut Vec<Rule>, incoming: &[Rule]) -> ApplyReport {
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
pub(super) fn classify_rules(existing: &[Rule], incoming: &[Rule]) -> ApplyReport {
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

/// Plans a declarative sync: upserts from `incoming` plus deletion of ids missing from it.
pub(super) fn plan_sync(existing: &[Rule], incoming: &[Rule]) -> (ApplyReport, Vec<String>) {
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

pub(super) fn save_rules(rules: &[Rule]) -> Result<(), String> {
    RulesFile::save_rules(rules)
        .map_err(|err| format!("cannot write {}: {err}", RulesFile::path().display()))
}

/// Drops daemon-owned runtime counters of deleted or reset rules.
pub(super) fn forget_runtime_counters(ids: &[&str]) -> Result<(), String> {
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
pub(super) fn reset_redefined_quota_runtime(previous: &[Rule], applied: &[Rule]) -> Result<(), String> {
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

/// Writes a share import straight to the config sections. The library is committed first, because
/// a rule that references a preset or overlay the library never received is worse than an unused
/// library entry; a failed rules write is reported as a partial commit ([U13-005]).
pub(super) fn import_share_offline(
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
pub(super) fn merged_library(
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

pub(super) fn read_input(path: &str) -> Result<String, String> {
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

/// Removes one rule by id and returns it, so the caller can drop its runtime counters ([U13-004]).
pub(super) fn take_rule(rules: &mut Vec<Rule>, id: &str) -> Option<Rule> {
    let position = rules.iter().position(|rule| rule.id == id)?;
    Some(rules.remove(position))
}

/// Writes text through a sibling temp file and a rename, the same way the config sections do, so a
/// crash or full disk during `--rules-export <file>` never leaves the previous export truncated
/// ([U13-013]).
pub(super) fn write_text_atomic(path: &Path, text: &str) -> std::io::Result<()> {
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

/// Test-only: the minimal valid rule the rule-set and share-import planning tests build on.
#[cfg(test)]
pub(super) fn rule_with_notification(id: &str) -> Rule {
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_parse_reports_action_errors() {
        let raw = r#"[{"id":"a","name":"A","actions":[{"$action_type":"unknown"}]}]"#;
        let err = parse_rules_json(raw).expect_err("unknown action");
        assert!(err.contains("rule #1 (A)"), "{err}");
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
    fn test_plan_sync_reports_created_updated_deleted() {
        let existing = vec![rule_with_notification("keep"), rule_with_notification("update")];
        let incoming = vec![rule_with_notification("update"), rule_with_notification("new")];
        let (report, deleted) = plan_sync(&existing, &incoming);
        assert_eq!(report.created, vec!["new".to_string()]);
        assert_eq!(report.updated, vec!["update".to_string()]);
        assert_eq!(deleted, vec!["keep".to_string()]);
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
}
