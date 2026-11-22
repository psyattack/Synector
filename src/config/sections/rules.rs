//! `rules.json`: automation definitions, without runtime counters.
//!
//! A hand-edited rule with unknown keys or invalid values is skipped on its own instead of
//! treating the whole section as corrupt.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::storage::{read_json_filtered, SectionSpec, RULES_FILE};
use crate::config::{default_version, AppConfig, CONFIG_VERSION};
use crate::models::Rule;

/// Rule definitions. Runtime counters are written as zero and ignored on read: the
/// only authoritative home for `execution_count`/`last_executed_at` is `runtime.json`,
/// so a rule edit never rewrites counters and a counter bump never rewrites definitions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RulesFile {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

impl Default for RulesFile {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            rules: Vec::new(),
        }
    }
}

fn version_mut(rules: &mut RulesFile) -> &mut u32 {
    &mut rules.version
}

/// Path and version stamp for `rules.json`; it has no read cache to invalidate.
static SECTION: SectionSpec<RulesFile> = SectionSpec {
    file: RULES_FILE,
    version: version_mut,
    invalidate: None,
};

/// A typo in a hand-edited rule is not a corrupt section: drop only that rule so the rest of
/// the file keeps loading, and leave a visible error naming the rule and keys.
fn skip_rules_with_unknown_keys(value: &mut serde_json::Value) {
    if let Some(rules) = value.get_mut("rules").and_then(serde_json::Value::as_array_mut) {
        rules.retain(|rule| match crate::models::validate_rule_keys(rule) {
            Ok(()) => true,
            Err(unknown) => {
                let id = rule
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("<no id>");
                let details = unknown
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ");
                crate::logging::error(
                    "CONFIG",
                    &format!("rules.json: rule '{id}': {details}; skipping the rule"),
                );
                false
            }
        });
    }
}

impl RulesFile {
    pub fn path() -> PathBuf {
        Self::path_in(&AppConfig::config_dir())
    }

    pub fn path_in(dir: &Path) -> PathBuf {
        SECTION.path_in(dir)
    }

    pub fn read_from(dir: &Path) -> Self {
        let path = Self::path_in(dir);
        let Some(mut rules) =
            read_json_filtered::<RulesFile>(&path, skip_rules_with_unknown_keys)
        else {
            return Self::default();
        };
        for rule in &mut rules.rules {
            rule.normalize();
        }
        // Value checks are not a corrupt section either: a hand-edited rule the CLI and
        // the GUI both reject (an out-of-range custom quota window, an impossible wait,
        // a malformed schedule) is skipped on its own, named in the error log. The same
        // per-rule validator the GUI's Save path runs is used, so nothing that path
        // accepts is dropped here.
        rules.rules.retain(|rule| {
            let errors = crate::validation::validate_rules(std::slice::from_ref(rule));
            if errors.is_empty() {
                return true;
            }
            crate::logging::error(
                "CONFIG",
                &format!(
                    "rules.json: rule '{}': {}; skipping the rule",
                    rule.id,
                    errors.join("; ")
                ),
            );
            false
        });
        SECTION.normalize(&mut rules);
        rules
    }

    pub fn save_rules(rules: &[Rule]) -> std::io::Result<()> {
        Self::save_rules_to(&AppConfig::config_dir(), rules)
    }

    pub fn save_rules_to(dir: &Path, rules: &[Rule]) -> std::io::Result<()> {
        let snapshot = RulesFile {
            version: CONFIG_VERSION,
            rules: rules.iter().map(rule_definition).collect(),
        };
        SECTION.save(dir, snapshot)
    }
}

/// Strips daemon-owned runtime counters from a rule before it is written to disk.
fn rule_definition(rule: &Rule) -> Rule {
    let mut definition = rule.clone();
    definition.clear_quota_runtime();
    definition
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "synector-config-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn rule(id: &str, name: &str) -> Rule {
        Rule {
            id: id.to_string(),
            name: name.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn rules_file_strips_runtime_counters() {
        let dir = temp_dir("rules");
        let mut stored = rule("rule_a", "A");
        stored.execution_count = 42;
        stored.last_executed_at = Some("2026-01-01 00:00:00".to_string());
        stored.quota_window_start = Some("2026-01-01 00:00:00".to_string());
        stored.is_enabled = false;

        RulesFile::save_rules_to(&dir, &[stored]).expect("save rules");

        let decoded = RulesFile::read_from(&dir);
        assert_eq!(decoded.version, CONFIG_VERSION);
        assert_eq!(decoded.rules.len(), 1);
        assert_eq!(decoded.rules[0].id, "rule_a");
        assert_eq!(decoded.rules[0].execution_count, 0);
        assert!(decoded.rules[0].last_executed_at.is_none());
        assert!(decoded.rules[0].quota_window_start.is_none());
        assert!(!decoded.rules[0].is_enabled);

        let raw = fs::read_to_string(RulesFile::path_in(&dir)).expect("rules json");
        assert!(!raw.contains("last_executed_at"), "runtime counter leaked: {raw}");
        assert!(!raw.contains("quota_window_start"), "quota anchor leaked: {raw}");
        assert!(raw.contains("\"version\": 1"));
    }

    #[test]
    fn rules_file_skips_rules_with_unknown_keys() {
        let dir = temp_dir("rules-unknown");
        let path = RulesFile::path_in(&dir);
        let raw = r#"{
            "version": 1,
            "rules": [
                {"id": "rule_good", "name": "Good"},
                {"id": "rule_typo", "name": "Typo", "max_execution": 5}
            ]
        }"#;
        fs::write(&path, raw).expect("write rules");

        let decoded = RulesFile::read_from(&dir);
        assert_eq!(decoded.rules.len(), 1);
        assert_eq!(decoded.rules[0].id, "rule_good");
    }

    /// U12-002 residual: a hand-edited rule with a custom quota window the CLI/GUI reject is
    /// skipped on its own, like an unknown key, instead of loading and being silently clamped.
    #[test]
    fn rules_file_skips_rules_with_invalid_values() {
        let dir = temp_dir("rules-invalid-values");
        let path = RulesFile::path_in(&dir);
        let raw = r#"{
            "version": 1,
            "rules": [
                {"id": "rule_good", "name": "Good"},
                {
                    "id": "rule_bad_window",
                    "name": "Bad Window",
                    "max_executions": 5,
                    "quota_period": {"custom": {"value": 100001, "unit": "hours"}}
                }
            ]
        }"#;
        fs::write(&path, raw).expect("write rules");

        let decoded = RulesFile::read_from(&dir);
        assert_eq!(decoded.rules.len(), 1);
        assert_eq!(decoded.rules[0].id, "rule_good");
    }

    /// The load-time check uses the same per-rule validator as the GUI's Save path, so every
    /// rule that path accepts survives a write/read round trip unchanged (boundary values
    /// included).
    #[test]
    fn rules_file_keeps_valid_rules_after_load_validation() {
        let dir = temp_dir("rules-valid-roundtrip");
        let mut stored = rule("rule_window", "Window Rule");
        stored.max_executions = Some(2);
        stored.quota_period = Some(crate::models::QuotaPeriod::Custom {
            value: crate::models::QuotaPeriod::MAX_CUSTOM_WINDOW_UNITS,
            unit: crate::models::QuotaUnit::Days,
        });
        stored.actions = vec![crate::models::Action::Wait {
            mode: crate::models::WaitMode::Timer,
            seconds: 86_400.0,
            condition: None,
            timeout_sec: 86_400.0,
            timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
        }];
        RulesFile::save_rules_to(&dir, std::slice::from_ref(&stored)).expect("save rules");

        let decoded = RulesFile::read_from(&dir);
        assert_eq!(
            decoded.rules.len(),
            1,
            "a rule the CLI/GUI accept must not be dropped on load"
        );
        assert_eq!(decoded.rules[0], stored, "the loaded rule must be unchanged");
    }

    #[test]
    fn rules_file_normalizes_overlay_ids_on_load() {
        let dir = temp_dir("rules-overlay-ids");
        let path = RulesFile::path_in(&dir);
        let raw = r#"{"version":1,"rules":[{"id":"r","name":"R","actions":[{"$action_type":"overlay","overlay_id":"outside","custom_overlay":{"id":"inside","name":"HUD"},"operation":"show"}]}]}"#;
        fs::write(&path, raw).expect("write rules");

        let decoded = RulesFile::read_from(&dir);
        match &decoded.rules[0].actions[0] {
            crate::models::Action::Overlay {
                overlay_id,
                custom_overlay,
                ..
            } => {
                assert_eq!(overlay_id, "outside");
                assert_eq!(
                    custom_overlay.as_ref().map(|definition| definition.id.as_str()),
                    Some("outside")
                );
            }
            other => panic!("expected an overlay action, got {other:?}"),
        }
    }
}
