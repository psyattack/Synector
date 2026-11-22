//! Rules-list filtering: the distinct-tag aggregation and the visibility predicate behind
//! `GuiState::visible_rules`. `rules_view::visible_rule_order` keeps its own copy this session.

use crate::models::Rule;

use super::GuiState;

impl GuiState {
    pub fn all_distinct_tags(&self) -> Vec<crate::models::RuleTag> {
        let mut map: std::collections::BTreeMap<String, crate::models::RuleTag> = std::collections::BTreeMap::new();
        if let Some(ds) = &self.daemon_state {
            for r in &ds.config.rules {
                for t in &r.tags {
                    map.entry(t.name.clone()).or_insert_with(|| t.clone());
                }
            }
        }
        if let Some(r) = &self.editing_rule {
            for t in &r.tags {
                map.entry(t.name.clone()).or_insert_with(|| t.clone());
            }
        }
        if let Some(r) = &self.builder_draft {
            for t in &r.tags {
                map.entry(t.name.clone()).or_insert_with(|| t.clone());
            }
        }
        map.into_values().collect()
    }

    /// Rules matching the active tag filter and search query.
    pub fn visible_rules(&self) -> Vec<Rule> {
        let rules = self
            .daemon_state
            .as_ref()
            .map(|s| s.config.rules.clone())
            .unwrap_or_default();
        let needle = self.search_query.trim().to_lowercase();
        rules
            .into_iter()
            .filter(|rule| matches_visible_filters(rule, self.selected_tag_filter.as_deref(), &needle))
            .collect()
    }
}

/// Whether one rule matches the active tag filter and the lowercased search needle. Lives in
/// state so the visible list and the "copy all" export cannot disagree (U05-013).
fn matches_visible_filters(rule: &Rule, tag_filter: Option<&str>, needle: &str) -> bool {
    if let Some(tag) = tag_filter {
        if !rule.tags.iter().any(|t| t.name.eq_ignore_ascii_case(tag)) {
            return false;
        }
    }
    if needle.is_empty() {
        return true;
    }
    rule.name.to_lowercase().contains(needle)
        || rule.tags.iter().any(|t| t.name.to_lowercase().contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with_shared_rules() -> GuiState {
        let mut state = GuiState::new();
        let json = serde_json::json!({
            "autorun_enabled": false,
            "uptime_secs": 0,
            "daemon_memory_bytes": 0,
            "daemon_cpu_usage": 0.0,
            "active_rules_count": 3,
            "total_rules_count": 3,
            "config": {
                "rules": [
                    {"id": "work_rule", "name": "Work Profile", "tags": [{"name": "Work"}]},
                    {"id": "game_rule", "name": "Game Boost", "tags": [{"name": "Gaming"}]},
                    {"id": "misc_rule", "name": "Misc Workflow"}
                ]
            },
            "logs": []
        });
        state.daemon_state =
            Some(serde_json::from_value(json).expect("daemon state deserializes"));
        state
    }

    #[test]
    fn test_visible_rules_respect_tag_filter_and_search() {
        let mut state = state_with_shared_rules();
        assert_eq!(state.visible_rules().len(), 3);

        state.selected_tag_filter = Some("Work".into());
        let filtered = state.visible_rules();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "work_rule");

        state.selected_tag_filter = None;
        state.search_query = "work".into();
        let searched = state.visible_rules();
        assert_eq!(searched.len(), 2, "name and tag matches are both kept");

        state.search_query = "nothing-matches".into();
        assert!(state.visible_rules().is_empty());
    }
}
