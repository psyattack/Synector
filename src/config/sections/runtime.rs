//! `runtime.json`: daemon-owned per-rule execution counters.
//!
//! The daemon flushes it coalesced, so a fast-firing rule cannot turn every execution into a
//! disk write.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::storage::{SectionSpec, RUNTIME_FILE};
use crate::config::{default_version, AppConfig, CONFIG_VERSION};
use crate::models::Rule;

/// One rule's daemon-owned execution counters.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleRuntime {
    #[serde(default)]
    pub execution_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_executed_at: Option<String>,
    /// Anchor of a `QuotaPeriod::Custom` window, local `%Y-%m-%d %H:%M:%S`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_window_start: Option<String>,
}

/// Hot runtime state. The daemon flushes it coalesced, so a fast-firing rule cannot
/// turn every execution into a disk write.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RuntimeState {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rules: BTreeMap<String, RuleRuntime>,
}

fn version_mut(runtime: &mut RuntimeState) -> &mut u32 {
    &mut runtime.version
}

/// Path and version stamp for `runtime.json`; it is never served from a read cache.
static SECTION: SectionSpec<RuntimeState> = SectionSpec {
    file: RUNTIME_FILE,
    version: version_mut,
    invalidate: None,
};

impl RuntimeState {
    pub fn path() -> PathBuf {
        Self::path_in(&AppConfig::config_dir())
    }

    pub fn path_in(dir: &Path) -> PathBuf {
        SECTION.path_in(dir)
    }

    pub fn read() -> Self {
        Self::read_from(&AppConfig::config_dir())
    }

    pub fn read_from(dir: &Path) -> Self {
        SECTION.load(&Self::path_in(dir)).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&AppConfig::config_dir())
    }

    pub fn save_to(&self, dir: &Path) -> std::io::Result<()> {
        SECTION.save(dir, self.clone())
    }

    /// Snapshots the counters of every rule that has run at least once (including the
    /// anchor of a custom quota window, which only survives a restart through this file).
    pub fn capture(rules: &[Rule]) -> Self {
        let mut runtime = Self {
            version: CONFIG_VERSION,
            rules: BTreeMap::new(),
        };
        for rule in rules {
            if rule.execution_count == 0
                && rule.last_executed_at.is_none()
                && rule.quota_window_start.is_none()
            {
                continue;
            }
            runtime.rules.insert(
                rule.id.clone(),
                RuleRuntime {
                    execution_count: rule.execution_count,
                    last_executed_at: rule.last_executed_at.clone(),
                    quota_window_start: rule.quota_window_start.clone(),
                },
            );
        }
        runtime
    }

    /// Restores the counters captured by [`Self::capture`] onto freshly loaded rules.
    pub fn apply_to(&self, rules: &mut [Rule]) {
        for rule in rules {
            if let Some(entry) = self.rules.get(&rule.id) {
                rule.execution_count = entry.execution_count;
                rule.last_executed_at = entry.last_executed_at.clone();
                rule.quota_window_start = entry.quota_window_start.clone();
            }
        }
    }

    /// Drops a deleted rule's counters, and implements the CLI `--rules-reset`.
    pub fn forget(&mut self, rule_id: &str) {
        self.rules.remove(rule_id);
    }
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
    fn runtime_state_capture_and_apply() {
        let mut first = rule("rule_a", "A");
        first.execution_count = 7;
        first.last_executed_at = Some("2026-02-03 04:05:06".to_string());
        first.quota_window_start = Some("2026-02-03 01:00:00".to_string());
        let untouched = rule("rule_b", "B");

        let runtime = RuntimeState::capture(&[first, untouched]);
        assert_eq!(runtime.rules.len(), 1);
        assert_eq!(runtime.rules["rule_a"].execution_count, 7);
        assert_eq!(
            runtime.rules["rule_a"].quota_window_start.as_deref(),
            Some("2026-02-03 01:00:00")
        );

        let mut loaded = vec![rule("rule_a", "A"), rule("rule_b", "B")];
        runtime.apply_to(&mut loaded);
        assert_eq!(loaded[0].execution_count, 7);
        assert_eq!(
            loaded[0].last_executed_at.as_deref(),
            Some("2026-02-03 04:05:06")
        );
        assert_eq!(
            loaded[0].quota_window_start.as_deref(),
            Some("2026-02-03 01:00:00")
        );
        assert_eq!(loaded[1].execution_count, 0);
    }

    #[test]
    fn runtime_state_round_trip_and_forget() {
        let dir = temp_dir("runtime");
        let mut runtime = RuntimeState::default();
        runtime.rules.insert(
            "rule_a".to_string(),
            RuleRuntime {
                execution_count: 3,
                last_executed_at: None,
                quota_window_start: Some("2026-02-03 01:00:00".to_string()),
            },
        );
        runtime.save_to(&dir).expect("save runtime");

        let mut decoded = RuntimeState::read_from(&dir);
        assert_eq!(decoded.version, CONFIG_VERSION);
        assert_eq!(
            decoded.rules["rule_a"].quota_window_start.as_deref(),
            Some("2026-02-03 01:00:00")
        );
        assert_eq!(decoded.rules["rule_a"].execution_count, 3);
        decoded.forget("rule_a");
        assert!(decoded.rules.is_empty());
        decoded.forget("missing");
    }
}
