use std::collections::{HashMap, HashSet};

use crate::ipc::DaemonState;
use crate::models::Action;

use super::executions::ActiveExecutionItem;
use super::DaemonEngine;

/// Builds the [`DaemonState`] snapshot the GUI renders. It is a read-only view over the engine:
/// [`DaemonEngine::get_current_state`] keeps the two state mutations (event drain and quota
/// rollover) and delegates the aggregation to this projector.
pub(super) struct StateProjector<'a> {
    engine: &'a DaemonEngine,
}

impl<'a> StateProjector<'a> {
    pub(super) fn new(engine: &'a DaemonEngine) -> Self {
        Self { engine }
    }

    pub(super) fn project(&self) -> DaemonState {
        let active_rules = self
            .engine
            .config
            .rules
            .iter()
            .filter(|rule| rule.is_enabled)
            .count();
        let total_rules = self.engine.config.rules.len();

        let active_overlay_ids = crate::daemon::overlay_manager::get_active_overlay_ids();
        let active_rule_executions = self.active_rule_executions(&active_overlay_ids);
        let active_test_rule_ids = self.engine.executions.recurring_rule_ids().cloned().collect();

        crate::daemon::telemetry::update_engine_stats(
            active_rules,
            active_rule_executions.len(),
            None,
            None,
        );

        let (update_progress, update_status) = self.engine.update_progress_snapshot();

        DaemonState {
            autorun_enabled: crate::autorun::is_autorun_enabled(crate::autorun::APP_NAME),
            uptime_secs: self.engine.start_time.elapsed().as_secs(),
            daemon_memory_bytes: Self::daemon_memory(),
            active_rules_count: active_rules,
            total_rules_count: total_rules,
            config: self.engine.config.clone(),
            logs: self.engine.logs.clone(),
            active_rule_executions,
            active_overlay_ids,
            active_test_rule_ids,
            analytics: self.engine.analytics.clone(),
            available_update: self.engine.update.available().cloned(),
            update_progress,
            update_status,
            pending_open_update_modal: self.engine.update.pending_open_modal(),
            pending_auto_start_update: self.engine.update.pending_auto_start(),
        }
    }

    /// Active executions grouped per rule, plus the runs implied by a bound companion or a
    /// showing overlay so the GUI can render a rule as running while its satellite lives.
    fn active_rule_executions(
        &self,
        active_overlay_ids: &HashSet<String>,
    ) -> HashMap<String, crate::ipc::ActiveRuleExecutionState> {
        let mut active_rule_executions = HashMap::new();
        let mut grouped: HashMap<String, Vec<&ActiveExecutionItem>> = HashMap::new();
        for item in self.engine.executions.active_executions() {
            grouped.entry(item.rule_id.clone()).or_default().push(item);
        }
        for (rule_id, items) in grouped {
            let active_count = items.len();
            if let Some(latest) = items.into_iter().max_by_key(|item| item.started_at) {
                active_rule_executions.insert(rule_id, crate::ipc::ActiveRuleExecutionState {
                    active_count,
                    current_step: latest.step_index,
                    total_steps: latest.total_steps,
                    is_exit: latest.is_exit,
                });
            }
        }

        // Track active bound satellites waiting for host process exit
        for bp in self.engine.executions.bound_processes() {
            if !active_rule_executions.contains_key(&bp.rule_id) {
                if let Some(rule) = self.engine.config.rules.iter().find(|r| r.id == bp.rule_id) {
                    let step_idx = rule
                        .actions
                        .iter()
                        .position(|a| matches!(a, Action::StartProcess { close_on_exit: true, .. }))
                        .unwrap_or(0);
                    active_rule_executions.insert(
                        bp.rule_id.clone(),
                        crate::ipc::ActiveRuleExecutionState {
                            active_count: 1,
                            current_step: step_idx,
                            total_steps: rule.actions.len(),
                            is_exit: false,
                        },
                    );
                }
            }
        }

        // Track active bound overlays waiting for host process exit
        for bo in self.engine.executions.bound_overlays() {
            if !active_rule_executions.contains_key(&bo.rule_id) {
                if let Some(rule) = self.engine.config.rules.iter().find(|r| r.id == bo.rule_id) {
                    let step_idx = rule
                        .actions
                        .iter()
                        .position(|a| matches!(a, Action::Overlay { close_on_exit: true, .. }))
                        .unwrap_or(0);
                    active_rule_executions.insert(
                        bo.rule_id.clone(),
                        crate::ipc::ActiveRuleExecutionState {
                            active_count: 1,
                            current_step: step_idx,
                            total_steps: rule.actions.len(),
                            is_exit: false,
                        },
                    );
                }
            }
        }

        // Track rules with active showing overlays
        if !active_overlay_ids.is_empty() {
            for rule in &self.engine.config.rules {
                if !active_rule_executions.contains_key(&rule.id) {
                    let overlay_step = rule.actions.iter().enumerate().find_map(|(idx, a)| {
                        match a {
                            Action::Overlay { overlay_id, custom_overlay, .. } => {
                                let id = custom_overlay.as_ref().map(|o| o.id.as_str()).unwrap_or(overlay_id.as_str());
                                if active_overlay_ids.contains(id) {
                                    Some(idx)
                                } else {
                                    None
                                }
                            }
                            Action::IfElse { then_actions, else_if_branches, else_actions, .. } => {
                                let in_then = then_actions.iter().any(|sub| match sub {
                                    Action::Overlay { overlay_id, custom_overlay, .. } => {
                                        let id = custom_overlay.as_ref().map(|o| o.id.as_str()).unwrap_or(overlay_id.as_str());
                                        active_overlay_ids.contains(id)
                                    }
                                    _ => false,
                                });
                                let in_elseif = else_if_branches.iter().any(|b| b.actions.iter().any(|sub| match sub {
                                    Action::Overlay { overlay_id, custom_overlay, .. } => {
                                        let id = custom_overlay.as_ref().map(|o| o.id.as_str()).unwrap_or(overlay_id.as_str());
                                        active_overlay_ids.contains(id)
                                    }
                                    _ => false,
                                }));
                                let in_else = else_actions.iter().any(|sub| match sub {
                                    Action::Overlay { overlay_id, custom_overlay, .. } => {
                                        let id = custom_overlay.as_ref().map(|o| o.id.as_str()).unwrap_or(overlay_id.as_str());
                                        active_overlay_ids.contains(id)
                                    }
                                    _ => false,
                                });
                                if in_then || in_elseif || in_else {
                                    Some(idx)
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        }
                    });

                    if let Some(step_idx) = overlay_step {
                        active_rule_executions.insert(
                            rule.id.clone(),
                            crate::ipc::ActiveRuleExecutionState {
                                active_count: 1,
                                current_step: step_idx,
                                total_steps: rule.actions.len(),
                                is_exit: false,
                            },
                        );
                    }
                }
            }
        }

        // Track rules in active recurring test execution (e.g. interval triggers)
        for rule_id in self.engine.executions.recurring_rule_ids() {
            if !active_rule_executions.contains_key(rule_id) {
                let total_steps = self.engine.config.rules.iter().find(|r| &r.id == rule_id)
                    .map(|r| r.actions.len().max(1))
                    .unwrap_or(1);
                active_rule_executions.insert(rule_id.clone(), crate::ipc::ActiveRuleExecutionState {
                    active_count: 1,
                    current_step: 0,
                    total_steps,
                    is_exit: false,
                });
            }
        }

        active_rule_executions
    }

    fn daemon_memory() -> u64 {
        #[cfg(windows)]
        unsafe {
            use windows::Win32::System::Threading::GetCurrentProcess;
            use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
            let mut pmc: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
            pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
            if K32GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb).as_bool() {
                return pmc.WorkingSetSize as u64;
            }
        }
        2 * 1024 * 1024 // ~2MB default
    }
}
