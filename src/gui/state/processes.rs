//! Process snapshot cache and the process-picker targets: opening the picker, applying the
//! chosen process to a trigger, action or condition.

use std::time::Instant;

use crate::ipc::IpcRequest;

use super::ipc::IpcJobKind;
use super::modals::ProcessPickerTarget;
use super::GuiState;

/// Minimum spacing between failed process-snapshot retries: the inspector asks for a snapshot on
/// every frame while it edits, and a down daemon must not turn that into a request flood.
const PROCESS_SNAPSHOT_RETRY: std::time::Duration = std::time::Duration::from_secs(2);

impl GuiState {
    /// Fetches a process snapshot when the cache is still empty. Non-blocking: the result is
    /// applied by the next frame.
    pub fn ensure_processes(&mut self) {
        self.request_process_snapshot(false);
    }

    /// Queues a `GetProcesses` round-trip unless one is already in flight. `force` is used when
    /// the picker opens, so the list is fresh exactly when the user asks for it.
    fn request_process_snapshot(&mut self, force: bool) {
        if self.ipc.processes_in_flight {
            return;
        }
        if !force {
            if !self.cached_processes.is_empty() {
                return;
            }
            if self
                .ipc
                .last_process_request
                .is_some_and(|at| at.elapsed() < PROCESS_SNAPSHOT_RETRY)
            {
                return;
            }
        }
        if self.enqueue_ipc(IpcJobKind::Processes, IpcRequest::GetProcesses, None) {
            self.ipc.processes_in_flight = true;
            self.ipc.last_process_request = Some(Instant::now());
        }
    }

    pub fn open_process_picker(&mut self, target: ProcessPickerTarget) {
        // Opening the picker is the explicit refresh point: the list is fetched for exactly the
        // moment the user needs it, instead of on every daemon-sync tick while editing.
        self.request_process_snapshot(true);
        self.process_search_query.clear();
        self.picker_target = Some(target);
        self.last_picker_target = Some(target);
    }

    pub fn close_process_picker(&mut self) {
        self.picker_target = None;
    }

    pub fn apply_selected_process(&mut self, proc_name: &str) {
        let loc = self.loc();
        let target = self.picker_target.take().or(self.last_picker_target);
        self.picker_target = None;

        self.push_undo_snapshot();
        if let Some(rule) = &mut self.editing_rule {
            match target {
                Some(ProcessPickerTarget::Action(idx)) => {
                    if let Some(action) = rule.actions.get_mut(idx) {
                        apply_process_to_action(action, proc_name);
                    }
                }
                Some(ProcessPickerTarget::Condition(idx)) => {
                    if let Some(item) = rule.conditions.get_mut(idx) {
                        apply_process_to_condition(&mut item.condition, proc_name);
                    }
                }
                Some(ProcessPickerTarget::ActionCondition { action_idx, cond_idx }) => {
                    if let Some(crate::models::Action::IfElse { conditions, .. }) = rule.actions.get_mut(action_idx) {
                        if let Some(item) = conditions.get_mut(cond_idx) {
                            apply_process_to_condition(&mut item.condition, proc_name);
                        }
                    }
                }
                Some(ProcessPickerTarget::ActionElseIfCondition { action_idx, branch_idx, cond_idx }) => {
                    if let Some(crate::models::Action::IfElse { else_if_branches, .. }) = rule.actions.get_mut(action_idx) {
                        if let Some(branch) = else_if_branches.get_mut(branch_idx) {
                            if let Some(item) = branch.conditions.get_mut(cond_idx) {
                                apply_process_to_condition(&mut item.condition, proc_name);
                            }
                        }
                    }
                }
                Some(ProcessPickerTarget::IfElseThen { parent_idx, sub_idx }) => {
                    if let Some(crate::models::Action::IfElse { then_actions, .. }) = rule.actions.get_mut(parent_idx) {
                        if let Some(sub_action) = then_actions.get_mut(sub_idx) {
                            apply_process_to_action(sub_action, proc_name);
                        }
                    }
                }
                Some(ProcessPickerTarget::IfElseElseIf { parent_idx, branch_idx, sub_idx }) => {
                    if let Some(crate::models::Action::IfElse { else_if_branches, .. }) = rule.actions.get_mut(parent_idx) {
                        if let Some(branch) = else_if_branches.get_mut(branch_idx) {
                            if let Some(sub_action) = branch.actions.get_mut(sub_idx) {
                                apply_process_to_action(sub_action, proc_name);
                            }
                        }
                    }
                }
                Some(ProcessPickerTarget::IfElseElse { parent_idx, sub_idx }) => {
                    if let Some(crate::models::Action::IfElse { else_actions, .. }) = rule.actions.get_mut(parent_idx) {
                        if let Some(sub_action) = else_actions.get_mut(sub_idx) {
                            apply_process_to_action(sub_action, proc_name);
                        }
                    }
                }
                Some(ProcessPickerTarget::Trigger) | None => {
                    match &mut rule.trigger {
                        Some(crate::models::Trigger::Process { process_name, .. }) => {
                            *process_name = proc_name.to_string();
                        }
                        Some(crate::models::Trigger::WindowFocus { process_name, .. }) => {
                            *process_name = proc_name.to_string();
                        }
                        Some(crate::models::Trigger::ResourceThreshold { process_name, .. }) => {
                            *process_name = proc_name.to_string();
                        }
                        _ => {
                            rule.trigger = Some(crate::models::Trigger::Process {
                                process_name: proc_name.to_string(),
                                match_mode: crate::models::ProcessMatchMode::Exact,
                                single_instance: false,
                                event: crate::models::ProcessTriggerEvent::Started,
                            });
                        }
                    }

                    if rule.name.is_empty()
                        || rule.name == loc.rules_default_new_name
                        || rule.name.starts_with(loc.inspector_rule_track_prefix)
                    {
                        rule.name = format!("{} {}", loc.inspector_rule_track_prefix, proc_name);
                    }
                }
            }
            rule.auto_process_icon = true;
        }
    }

    pub fn apply_process_to_draft_or_new_rule(&mut self, proc_name: &str) {
        let loc = self.loc();
        let mut rule = self.initial_new_rule();
        rule.trigger = Some(crate::models::Trigger::Process {
            process_name: proc_name.to_string(),
            match_mode: crate::models::ProcessMatchMode::Exact,
            single_instance: false,
            event: crate::models::ProcessTriggerEvent::Started,
        });
        rule.name = format!("{} {}", loc.inspector_rule_track_prefix, proc_name);
        rule.auto_process_icon = true;
        self.is_creating_new_rule = true;
        self.selected_rule_id = None;
        self.editing_rule = Some(rule);
        self.selected_process = None;
    }
}

fn apply_process_to_action(action: &mut crate::models::Action, proc_name: &str) {
    match action {
        crate::models::Action::KillProcess { process_name, .. } => {
            *process_name = proc_name.to_string();
        }
        crate::models::Action::AudioControl { target, process_name, .. } => {
            if *target == crate::models::AudioTarget::Process {
                *process_name = Some(proc_name.to_string());
            }
        }
        crate::models::Action::Firewall { process_name, .. } => {
            *process_name = proc_name.to_string();
        }
        crate::models::Action::Window { target_process, .. } => {
            *target_process = Some(proc_name.to_string());
        }
        crate::models::Action::ProcessTuning { target_process, .. } => {
            *target_process = Some(proc_name.to_string());
        }
        crate::models::Action::Wait { condition: Some(crate::models::WaitCondition::ProcessStarted { process_name, .. } | crate::models::WaitCondition::ProcessExited { process_name, .. } | crate::models::WaitCondition::WindowOpened { process_name, .. } | crate::models::WaitCondition::WindowClosed { process_name, .. }), .. } => {
            *process_name = proc_name.to_string();
        }
        crate::models::Action::Check { condition } => {
            apply_process_to_condition(condition, proc_name);
        }
        _ => {}
    }
}

fn apply_process_to_condition(cond: &mut crate::models::RuleCondition, proc_name: &str) {
    match cond {
        crate::models::RuleCondition::ProcessRunning { process_name, .. } => {
            *process_name = proc_name.to_string();
        }
        crate::models::RuleCondition::WindowFocused { process_name, .. } => {
            *process_name = proc_name.to_string();
        }
        crate::models::RuleCondition::ResourceThreshold { process_name, .. } => {
            *process_name = proc_name.to_string();
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::ipc::IpcJobResult;

    #[test]
    fn test_process_snapshot_requests_are_rate_limited() {
        let mut state = GuiState::new();
        assert!(state.cached_processes.is_empty());

        state.ensure_processes();
        assert!(state.ipc.processes_in_flight, "an empty cache requests a snapshot");

        // The request fails (no daemon). The inspector asks again on the next frame: the retry
        // must not be issued immediately.
        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::Processes,
            result: Err("Cannot connect to daemon pipe".into()),
        });
        assert!(!state.ipc.processes_in_flight);
        state.ensure_processes();
        assert!(!state.ipc.processes_in_flight, "failed retries are spaced out");

        // Opening the picker is an explicit request and bypasses the spacing.
        state.open_process_picker(ProcessPickerTarget::Trigger);
        assert!(state.ipc.processes_in_flight);
    }

    /// Every picker target writes the chosen process into its own place in the draft, and each
    /// commit is one undo step.
    #[test]
    fn test_picker_commit_maps_target() {
        use crate::models::{
            Action, ElseIfBranch, ProcessMatchMode, Rule, RuleCondition, RuleConditionItem, Trigger,
        };

        let kill = |name: &str| Action::KillProcess {
            process_name: name.to_string(),
            kill_tree: false,
            force: false,
        };
        let condition = || RuleCondition::ProcessRunning {
            process_name: "seed.exe".to_string(),
            match_mode: ProcessMatchMode::Exact,
            must_be_running: true,
        };

        let mut rule = Rule {
            name: "Picker targets".to_string(),
            trigger: Some(Trigger::Process {
                process_name: "seed.exe".to_string(),
                match_mode: ProcessMatchMode::Exact,
                single_instance: false,
                event: crate::models::ProcessTriggerEvent::Started,
            }),
            ..Rule::default()
        };
        rule.conditions.push(RuleConditionItem::new(condition()));
        rule.actions.push(kill("seed.exe"));
        rule.actions.push(Action::IfElse {
            conditions: vec![RuleConditionItem::new(condition())],
            then_actions: vec![kill("seed.exe")],
            else_if_branches: vec![ElseIfBranch {
                conditions: vec![RuleConditionItem::new(condition())],
                actions: vec![kill("seed.exe")],
            }],
            else_actions: vec![kill("seed.exe")],
        });

        let mut state = GuiState::new();
        state.editing_rule = Some(rule);

        let pick = |state: &mut GuiState, target: ProcessPickerTarget, name: &str| {
            state.open_process_picker(target);
            state.apply_selected_process(name);
        };

        pick(&mut state, ProcessPickerTarget::Trigger, "trigger.exe");
        pick(&mut state, ProcessPickerTarget::Action(0), "action.exe");
        pick(&mut state, ProcessPickerTarget::Condition(0), "condition.exe");
        pick(
            &mut state,
            ProcessPickerTarget::ActionCondition {
                action_idx: 1,
                cond_idx: 0,
            },
            "ifelse_cond.exe",
        );
        pick(
            &mut state,
            ProcessPickerTarget::ActionElseIfCondition {
                action_idx: 1,
                branch_idx: 0,
                cond_idx: 0,
            },
            "elseif_cond.exe",
        );
        pick(
            &mut state,
            ProcessPickerTarget::IfElseThen {
                parent_idx: 1,
                sub_idx: 0,
            },
            "then.exe",
        );
        pick(
            &mut state,
            ProcessPickerTarget::IfElseElseIf {
                parent_idx: 1,
                branch_idx: 0,
                sub_idx: 0,
            },
            "elseif_action.exe",
        );
        pick(
            &mut state,
            ProcessPickerTarget::IfElseElse {
                parent_idx: 1,
                sub_idx: 0,
            },
            "else.exe",
        );

        let rule = state.editing_rule.as_ref().unwrap();
        assert!(matches!(
            &rule.trigger,
            Some(Trigger::Process { process_name, .. }) if process_name == "trigger.exe"
        ));
        assert!(matches!(
            &rule.actions[0],
            Action::KillProcess { process_name, .. } if process_name == "action.exe"
        ));
        assert!(matches!(
            &rule.conditions[0].condition,
            RuleCondition::ProcessRunning { process_name, .. } if process_name == "condition.exe"
        ));
        let Some(Action::IfElse {
            conditions,
            then_actions,
            else_if_branches,
            else_actions,
        }) = rule.actions.get(1)
        else {
            panic!("expected the IfElse action at index 1");
        };
        assert!(matches!(
            &conditions[0].condition,
            RuleCondition::ProcessRunning { process_name, .. } if process_name == "ifelse_cond.exe"
        ));
        assert!(matches!(
            &else_if_branches[0].conditions[0].condition,
            RuleCondition::ProcessRunning { process_name, .. } if process_name == "elseif_cond.exe"
        ));
        assert!(matches!(
            &then_actions[0],
            Action::KillProcess { process_name, .. } if process_name == "then.exe"
        ));
        assert!(matches!(
            &else_if_branches[0].actions[0],
            Action::KillProcess { process_name, .. } if process_name == "elseif_action.exe"
        ));
        assert!(matches!(
            &else_actions[0],
            Action::KillProcess { process_name, .. } if process_name == "else.exe"
        ));
        assert!(rule.auto_process_icon, "a picked process keeps the auto icon on");
        assert_eq!(state.undo_stack.len(), 8, "each pick is one undo step");
    }
}
