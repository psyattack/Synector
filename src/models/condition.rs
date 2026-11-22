use serde::{Deserialize, Serialize};

use super::serde_defaults::*;
use super::trigger::{ProcessMatchMode, SystemEventType};
use super::validation::{days_of_week_are_valid, is_valid_hh_mm, resource_threshold_errors};

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConditionOperator {
    #[default]
    And,
    Or,
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(from = "RuleConditionItemHelper", into = "RuleConditionItemHelper")]
pub struct RuleConditionItem {
    pub operator: ConditionOperator,
    pub condition: RuleCondition,
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
enum RuleConditionItemHelper {
    Item {
        #[serde(default)]
        operator: ConditionOperator,
        condition: RuleCondition,
    },
    Bare(RuleCondition),
}

impl From<RuleConditionItemHelper> for RuleConditionItem {
    fn from(helper: RuleConditionItemHelper) -> Self {
        match helper {
            RuleConditionItemHelper::Item { operator, condition } => Self { operator, condition },
            RuleConditionItemHelper::Bare(condition) => Self {
                operator: ConditionOperator::And,
                condition,
            },
        }
    }
}

impl From<RuleConditionItem> for RuleConditionItemHelper {
    fn from(item: RuleConditionItem) -> Self {
        RuleConditionItemHelper::Item {
            operator: item.operator,
            condition: item.condition,
        }
    }
}

impl RuleConditionItem {
    pub fn new(condition: RuleCondition) -> Self {
        Self {
            operator: ConditionOperator::And,
            condition,
        }
    }

    pub fn with_operator(operator: ConditionOperator, condition: RuleCondition) -> Self {
        Self {
            operator,
            condition,
        }
    }
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "$condition_type", rename_all = "snake_case")]
pub enum RuleCondition {
    #[serde(rename = "process_running")]
    ProcessRunning {
        process_name: String,
        #[serde(default)]
        match_mode: ProcessMatchMode,
        #[serde(default = "default_true")]
        must_be_running: bool,
    },
    #[serde(rename = "window_focused")]
    WindowFocused {
        process_name: String,
        #[serde(default)]
        match_mode: ProcessMatchMode,
        #[serde(default = "default_true")]
        must_be_focused: bool,
    },
    #[serde(rename = "time_range")]
    TimeRange {
        start_time: String, // "HH:MM"
        end_time: String,   // "HH:MM"
        #[serde(default)]
        days_of_week: Vec<u8>, // 1 = Monday .. 7 = Sunday
    },
    #[serde(rename = "event")]
    Event {
        event_type: SystemEventType,
        #[serde(default = "default_idle_sec")]
        idle_duration_sec: u32,
    },
    #[serde(rename = "resource")]
    ResourceThreshold {
        process_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cpu_percent: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ram_mb: Option<u64>,
    },
    #[serde(rename = "file_watcher")]
    FileWatcher {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter_extension: Option<String>,
        #[serde(default = "default_true")]
        must_exist: bool,
    },
    /// Whether another rule has a running action pipeline right now.
    #[serde(rename = "rule_running")]
    RuleRunning {
        /// Id of the observed rule. An empty id never satisfies the condition.
        rule_id: String,
        #[serde(default = "default_true")]
        must_be_running: bool,
    },
}

impl RuleCondition {
    pub fn default_process() -> Self {
        Self::ProcessRunning {
            process_name: String::new(),
            match_mode: ProcessMatchMode::Exact,
            must_be_running: true,
        }
    }

    /// Shared value checks used by `--rules-apply` and share import.
    pub fn validation_errors(&self) -> Vec<&'static str> {
        match self {
            RuleCondition::TimeRange {
                start_time,
                end_time,
                days_of_week,
            } => {
                let mut errors = Vec::new();
                if !start_time.is_empty() && !is_valid_hh_mm(start_time) {
                    errors.push("time_range start_time must be a zero-padded HH:MM between 00:00 and 23:59");
                }
                if !end_time.is_empty() && !is_valid_hh_mm(end_time) {
                    errors.push("time_range end_time must be a zero-padded HH:MM between 00:00 and 23:59");
                }
                if !days_of_week_are_valid(days_of_week) {
                    errors.push("days_of_week must list days between 1 (Monday) and 7 (Sunday)");
                }
                errors
            }
            RuleCondition::ResourceThreshold {
                process_name,
                cpu_percent,
                ram_mb,
                ..
            } => resource_threshold_errors(process_name, *cpu_percent, *ram_mb),
            _ => Vec::new(),
        }
    }
}
