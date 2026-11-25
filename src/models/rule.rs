use serde::{Deserialize, Serialize};

use super::action::Action;
use super::condition::RuleConditionItem;
use super::quota::QuotaPeriod;
use super::serde_defaults::*;
use super::trigger::Trigger;

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
pub struct RuleTag {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

/// Parses a tag colour written as `#RRGGBB` or `RRGGBB`. Only six ASCII hex digits are
/// accepted; anything else returns `None` so callers can fall back or reject the rule.
pub fn parse_hex_color(value: &str) -> Option<[u8; 3]> {
    let clean = value.trim().trim_start_matches('#');
    if clean.len() != 6 || !clean.is_ascii() {
        return None;
    }
    let rgb = u32::from_str_radix(clean, 16).ok()?;
    Some([(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8])
}

impl RuleTag {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            color: None,
            icon: None,
        }
    }

    pub fn with_color(mut self, color: impl Into<String>) -> Self {
        self.color = Some(color.into());
        self
    }

    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Rule {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub is_enabled: bool,
    #[serde(default = "default_cooldown", skip_serializing_if = "is_zero_f64")]
    pub cooldown_seconds: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_executions: Option<u64>,
    /// Reset period for `max_executions`. `None` counts executions for the lifetime of
    /// the rule (historical behavior), `Some(_)` limits them per period.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_period: Option<QuotaPeriod>,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub execution_count: u64,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub stop_on_error: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<Trigger>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<RuleConditionItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_icon_path: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub auto_process_icon: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_executed_at: Option<String>,
    /// Runtime anchor of a custom quota window, local `%Y-%m-%d %H:%M:%S`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_window_start: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<RuleTag>,
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            description: String::new(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            max_executions: None,
            quota_period: None,
            execution_count: 0,
            stop_on_error: true,
            trigger: Some(Trigger::Manual),
            conditions: Vec::new(),
            actions: Vec::new(),
            custom_icon_path: None,
            auto_process_icon: false,
            last_executed_at: None,
            quota_window_start: None,
            tags: Vec::new(),
        }
    }
}

/// Visits every action of an action list depth-first, in pipeline order: the action itself,
/// then its `if_else` `then_actions`, each `else_if_branches[i].actions` in order, and finally
/// its `else_actions`. Actions outside an `if_else` are leaves.
pub fn visit_action_list(actions: &[Action], visit: &mut impl FnMut(&Action)) {
    for action in actions {
        visit(action);
        if let Action::IfElse {
            then_actions,
            else_if_branches,
            else_actions,
            ..
        } = action
        {
            visit_action_list(then_actions, visit);
            for branch in else_if_branches {
                visit_action_list(&branch.actions, visit);
            }
            visit_action_list(else_actions, visit);
        }
    }
}

impl Rule {
    pub fn clone_with_new_id(&self) -> Self {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let count = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let new_id = format!(
            "rule_{:x}_{:x}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
            count
        );
        let new_name = generate_clone_name(&self.name);

        let mut cloned = self.clone();
        cloned.id = new_id;
        cloned.name = new_name;
        cloned.is_enabled = false;
        cloned.clear_quota_runtime();
        cloned.stop_on_error = self.stop_on_error;
        cloned.custom_icon_path = self.custom_icon_path.clone();
        cloned.auto_process_icon = self.auto_process_icon;

        // Ensure cloned overlay actions get their own fresh unique IDs so they don't collide with original rule
        cloned.normalize();
        for action in cloned.actions.iter_mut() {
            action.refresh_overlay_ids();
        }

        cloned
    }

    /// Description of the first tag colour that is not `#RRGGBB`, for apply/import rejection.
    pub fn tag_color_error(&self) -> Option<String> {
        self.tags.iter().find_map(|tag| {
            let color = tag.color.as_deref()?;
            if parse_hex_color(color).is_some() {
                None
            } else {
                Some(format!(
                    "tag \"{}\" has colour \"{}\"; expected #RRGGBB",
                    tag.name, color
                ))
            }
        })
    }

    /// Normalizes every action definition of the rule (currently overlay ids).
    pub fn normalize(&mut self) {
        for action in self.actions.iter_mut() {
            action.normalize_overlay_ids();
        }
    }

    /// Visits every action of the rule read-only, in execution order, depth-first through
    /// nested `if_else` branches.
    pub fn visit_actions(&self, visit: &mut impl FnMut(&Action)) {
        visit_action_list(&self.actions, visit);
    }
}

pub fn generate_clone_name(name: &str) -> String {
    if let Some(base) = name.strip_suffix(" (Copy)") {
        format!("{} (Copy 2)", base)
    } else if let Some((base, num_str)) = name.rsplit_once(" (Copy ") {
        if let Some(num_trimmed) = num_str.strip_suffix(')') {
            if let Ok(n) = num_trimmed.parse::<u32>() {
                format!("{} (Copy {})", base, n + 1)
            } else {
                format!("{} (Copy)", name)
            }
        } else {
            format!("{} (Copy)", name)
        }
    } else {
        format!("{} (Copy)", name)
    }
}
