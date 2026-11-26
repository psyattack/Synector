//! Domain validation for rule definitions.
//!
//! Shared by the CLI, `rules.json` loading, the daemon engine and the GUI catalog pre-filters, so
//! the persistence layer, the engine and the editor do not depend on the command-line front end.
//! The messages, their order and the action walk order are behaviour: `--rules-validate` prints
//! them, `rules.json` logs them and the daemon returns them over IPC.

use crate::config::AppConfig;
use crate::models::{Action, ProcessTriggerEvent, ResourceLifetime, Rule, Trigger};

/// Structural rule errors that must block `--rules-apply`.
pub(crate) fn validate_rules(rules: &[Rule]) -> Vec<String> {
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
        let can_follow_trigger = matches!(
            rule.trigger,
            Some(Trigger::Process { event: ProcessTriggerEvent::Started, .. })
        );
        let mut check = |action: &Action| {
            if matches!(action, Action::Macro { infinite: true, .. }) {
                has_infinite_macro = true;
            }
            let lifetime = match action {
                Action::StartProcess { lifetime, .. } | Action::Overlay { lifetime, .. } => {
                    Some(*lifetime)
                }
                _ => None,
            };
            if lifetime == Some(ResourceLifetime::UntilTriggerProcessExit) && !can_follow_trigger {
                errors.push(format!(
                    "rule #{number} ({}): action: lifetime \"until_trigger_process_exit\" requires an On Process Start trigger",
                    display_id(rule)
                ));
            }
            for message in action.validation_errors() {
                errors.push(format!(
                    "rule #{number} ({}): action: {message}",
                    display_id(rule)
                ));
            }
        };
        crate::models::visit_action_list(&rule.actions, &mut check);
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
pub(crate) fn rule_warnings(rules: &[Rule], cfg: &AppConfig) -> Vec<String> {
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
        crate::models::visit_action_list(&rule.actions, &mut check);
    }
    warnings
}

fn display_id(rule: &Rule) -> &str {
    if rule.id.trim().is_empty() {
        "<no id>"
    } else {
        rule.id.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{MacroOperation, QuotaPeriod, ScheduleType, WaitMode};

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
            event: Default::default(),
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
            event: Default::default(),
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
