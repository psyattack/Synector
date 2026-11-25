//! Rule draft lifecycle: creating/editing rules, save/delete/clone/reorder/toggle/test-run
//! operations, the daemon result handlers and the macro preset identity awaiting confirmation.

use std::time::Instant;

use crate::ipc::{IpcClient, IpcRequest, IpcResponse};
use crate::models::Rule;

use super::ipc::{DirectRuleChange, DirectWrite, IpcJobKind};
use super::GuiState;

/// A `SaveMacroPreset` awaiting its result. The recorded events stay in the action — the only
/// live copy until the preset is persisted — and are cleared only after confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PendingMacroSave {
    pub(super) step_idx: usize,
    pub(super) preset_id: String,
}

impl GuiState {
    pub fn initial_new_rule(&self) -> Rule {
        Rule {
            id: format!("rule_{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()),
            name: String::new(),
            description: String::new(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            max_executions: None,
            quota_period: None,
            execution_count: 0,
            stop_on_error: true,
            trigger: Some(crate::models::Trigger::Manual),
            actions: Vec::new(),
            custom_icon_path: None,
            auto_process_icon: false,
            conditions: Vec::new(),
            last_executed_at: None,
            quota_window_start: None,
            tags: Vec::new(),
        }
    }

    pub fn start_new_rule(&mut self) {
        self.selected_rule_id = None;
        self.selected_process = None;
        self.is_creating_new_rule = true;
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.tag_picker_open = false;
        self.new_tag_name.clear();
        self.new_tag_color = Some("#3B82F6".into());
        self.new_tag_icon = None;
        let rule = self.builder_draft.take().unwrap_or_else(|| self.initial_new_rule());
        self.editing_rule = Some(rule);
    }

    pub fn reset_builder_state(&mut self) {
        self.builder_draft = None;
        self.close_action_catalog();
        if self.is_creating_new_rule {
            self.editing_rule = Some(self.initial_new_rule());
        }
    }

    /// Collects the overlay ids referenced by one action. Branch traversal is
    /// [`Rule::visit_actions`]'s job, so this stays shallow.
    pub fn extract_action_overlay_ids(action: &crate::models::Action, out: &mut Vec<String>) {
        if let crate::models::Action::Overlay { overlay_id, custom_overlay, .. } = action {
            out.push(overlay_id.clone());
            if let Some(cust) = custom_overlay {
                out.push(cust.id.clone());
            }
        }
    }

    pub fn extract_rule_overlay_ids(rule: &Rule) -> Vec<String> {
        let mut ids = Vec::new();
        rule.visit_actions(&mut |action| {
            Self::extract_action_overlay_ids(action, &mut ids);
        });
        ids
    }

    pub fn get_rule_execution_state(&self, rule: &Rule) -> Option<crate::ipc::ActiveRuleExecutionState> {
        if let Some(daemon_exec) = self.daemon_state.as_ref().and_then(|s| s.active_rule_executions.get(&rule.id)).copied() {
            return Some(daemon_exec);
        }

        if let Some(started_at) = self.active_test_runs.get(&rule.id) {
            let total = rule.actions.len().max(1);
            let elapsed_ms = started_at.elapsed().as_millis() as u64;
            let step_dur = 400;
            let current_step = if rule.actions.is_empty() {
                0
            } else {
                ((elapsed_ms / step_dur) as usize).min(rule.actions.len() - 1)
            };
            return Some(crate::ipc::ActiveRuleExecutionState {
                active_count: 1,
                current_step,
                total_steps: total,
            });
        }

        let has_active_overlay = rule.actions.iter().enumerate().find_map(|(idx, a)| match a {
                crate::models::Action::Overlay { overlay_id, custom_overlay, .. } => {
                    let ov_id = custom_overlay.as_ref().map(|o| o.id.as_str()).unwrap_or(overlay_id.as_str());
                    if self.daemon_state.as_ref().is_some_and(|d| d.active_overlay_ids.contains(ov_id))
                        || self.active_test_overlays.contains(ov_id)
                    {
                        Some(idx)
                    } else {
                        None
                    }
                }
                _ => None,
            });
        has_active_overlay.map(|step_idx| crate::ipc::ActiveRuleExecutionState {
            active_count: 1,
            current_step: step_idx,
            total_steps: rule.actions.len(),
        })
    }

    /// Queues hiding for every overlay preview the rule can open. The studio/editor never waits
    /// for the overlay process, so closing a rule or the drawer cannot stall on IPC.
    fn hide_overlay_previews(&mut self, ids: Vec<String>) {
        for id in ids {
            self.request_hide_overlay_preview(&id);
        }
    }

    /// Exit path: preview windows must be gone before the process ends, so these hiders stay
    /// synchronous (the shutdown budget already covers them). Interactive closes below queue the
    /// request instead of blocking the frame.
    fn hide_preview_now(overlay_id: &str) {
        let _ = IpcClient::call(&IpcRequest::HideOverlayPreview(overlay_id.to_string()));
    }

    pub fn cleanup_on_exit(&mut self) {
        if let Some(studio) = &self.overlay_studio {
            Self::hide_preview_now(&studio.overlay.id);
        }
        let mut rule_ids = Vec::new();
        if let Some(rule) = &self.editing_rule {
            rule_ids.extend(Self::extract_rule_overlay_ids(rule));
        }
        if let Some(rule) = &self.builder_draft {
            rule_ids.extend(Self::extract_rule_overlay_ids(rule));
        }
        for id in rule_ids {
            Self::hide_preview_now(&id);
        }
        let test_ids: Vec<String> = self.active_test_overlays.drain().collect();
        for id in test_ids {
            Self::hide_preview_now(&id);
        }
    }

    pub fn close_editor(&mut self) {
        self.close_action_catalog();
        let studio_id = self.overlay_studio.as_ref().map(|studio| studio.overlay.id.clone());
        if let Some(id) = studio_id {
            self.request_hide_overlay_preview(&id);
        }
        let rule_ids = self
            .editing_rule
            .as_ref()
            .map(Self::extract_rule_overlay_ids)
            .unwrap_or_default();
        self.hide_overlay_previews(rule_ids);
        let test_ids: Vec<String> = self.active_test_overlays.drain().collect();
        self.hide_overlay_previews(test_ids);
        if self.is_creating_new_rule {
            self.builder_draft = self.editing_rule.take();
        } else {
            self.editing_rule = None;
        }
        self.is_creating_new_rule = false;
        self.selected_rule_id = None;
        self.picker_target = None;
    }

    pub fn edit_rule(&mut self, rule: Rule) {
        if self.is_creating_new_rule {
            self.builder_draft = self.editing_rule.take();
        }
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.tag_picker_open = false;
        self.new_tag_name.clear();
        self.new_tag_color = Some("#3B82F6".into());
        self.new_tag_icon = None;
        self.selected_rule_id = Some(rule.id.clone());
        self.editing_rule = Some(rule);
        self.is_creating_new_rule = false;
        self.selected_process = None;
    }

    /// Clears the rule's quota progress. The daemon owns the counters, so a rule that
    /// already exists is reset through `ResetRule`; an unsaved draft is cleared locally.
    pub fn reset_rule_quota(&mut self, rule: &mut Rule) {
        rule.clear_quota_runtime();
        rule.is_enabled = true;
        let saved = self
            .daemon_state
            .as_ref()
            .is_some_and(|state| state.config.rules.iter().any(|saved| saved.id == rule.id));
        if saved {
            let id = rule.id.clone();
            let direct = DirectWrite::ResetRuntime { rule_id: id.clone() };
            self.enqueue_ipc(IpcJobKind::ResetRule { rule_id: id.clone() }, IpcRequest::ResetRule(id), Some(direct));
        }
    }

    pub fn save_rule(&mut self, mut rule: Rule) {
        if rule.name.trim().is_empty() {
            rule.name = self.loc().rules_default_new_name.to_string();
        }
        let mut seen_ids = std::collections::HashSet::new();
        fn deduplicate_overlay_ids(actions: &mut [crate::models::Action], seen: &mut std::collections::HashSet<String>) {
            for action in actions.iter_mut() {
                match action {
                    crate::models::Action::Overlay { overlay_id, custom_overlay, .. } => {
                        if !seen.insert(overlay_id.clone()) {
                            static DEDUP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
                            let count = DEDUP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            let fresh_id = format!(
                                "overlay_{:x}_{:x}",
                                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos(),
                                count
                            );
                            *overlay_id = fresh_id.clone();
                            if let Some(ref mut cust) = custom_overlay {
                                cust.id = fresh_id;
                            }
                        } else if let Some(ref mut cust) = custom_overlay {
                            cust.id = overlay_id.clone();
                        }
                    }
                    crate::models::Action::IfElse { then_actions, else_if_branches, else_actions, .. } => {
                        deduplicate_overlay_ids(then_actions, seen);
                        for b in else_if_branches {
                            deduplicate_overlay_ids(&mut b.actions, seen);
                        }
                        deduplicate_overlay_ids(else_actions, seen);
                    }
                    _ => {}
                }
            }
        }
        deduplicate_overlay_ids(&mut rule.actions, &mut seen_ids);

        // The draft stays in `editing_rule` until the daemon confirms the save; a transport or
        // validation failure must not discard the only copy of the user's rule.
        let rule_id = rule.id.clone();
        let direct = DirectWrite::Rules(DirectRuleChange::Upsert(Box::new(rule.clone())));
        self.enqueue_ipc(IpcJobKind::SaveRule { rule_id }, IpcRequest::SaveRule(rule), Some(direct));
    }

    /// Applies the daemon's answer to `SaveRule`: clears the editor only on confirmation and
    /// reports the reason otherwise.
    pub(super) fn apply_save_rule_result(&mut self, rule_id: &str, result: Result<IpcResponse, String>) {
        match Self::mutation_failure(result) {
            None => {
                self.editing_rule = None;
                self.is_creating_new_rule = false;
                self.selected_rule_id = None;
                self.builder_draft = None;
                self.set_toast(self.loc().toast_rule_saved);
            }
            Some(failure) => {
                crate::logging::warn(
                    "GUI",
                    &format!("SaveRule '{rule_id}' was not applied: {}", failure.diagnostic()),
                );
                self.set_toast(self.mutation_failure_toast(&failure));
            }
        }
        self.daemon_sync.force();
    }

    pub fn delete_rule(&mut self, id: &str) {
        // Previews are hidden right away; the rule itself is removed only once the daemon
        // confirms, so a failed delete leaves the editor and selection intact.
        let mut preview_ids = Vec::new();
        if let Some(rule) = self.daemon_state.as_ref().and_then(|s| s.config.rules.iter().find(|r| r.id == id)) {
            preview_ids.extend(Self::extract_rule_overlay_ids(rule));
        }
        if let Some(rule) = self.editing_rule.as_ref().filter(|rule| rule.id == id) {
            preview_ids.extend(Self::extract_rule_overlay_ids(rule));
        }
        self.hide_overlay_previews(preview_ids);
        let direct = DirectWrite::Rules(DirectRuleChange::Delete(id.to_string()));
        self.enqueue_ipc(
            IpcJobKind::DeleteRule { rule_id: id.to_string() },
            IpcRequest::DeleteRule(id.to_string()),
            Some(direct),
        );
    }

    pub fn clone_rule(&mut self, rule: &Rule) {
        let cloned = rule.clone_with_new_id();
        let new_name = cloned.name.clone();

        // The clone opens in the editor immediately; the daemon save is confirmed separately.
        self.selected_rule_id = None;
        if self.editing_rule.is_some() {
            self.editing_rule = Some(cloned.clone());
            self.is_creating_new_rule = false;
        }
        self.set_toast(format!("{}: {}", self.loc().toast_rule_cloned, new_name));
        let direct = DirectWrite::Rules(DirectRuleChange::Upsert(Box::new(cloned.clone())));
        self.enqueue_ipc(
            IpcJobKind::CloneRule { rule_id: cloned.id.clone() },
            IpcRequest::SaveRule(cloned),
            Some(direct),
        );
    }

    pub fn reorder_rule_by_id(&mut self, src_id: &str, target_id: Option<&str>, insert_after: bool) {
        let mut order = None;
        if let Some(ds) = &mut self.daemon_state {
            if let Some(src_pos) = ds.config.rules.iter().position(|r| r.id == src_id) {
                let rule = ds.config.rules.remove(src_pos);
                let target_pos = if let Some(tid) = target_id {
                    ds.config.rules.iter().position(|r| r.id == tid)
                } else {
                    None
                };
                let insert_idx = match target_pos {
                    Some(pos) => {
                        if insert_after {
                            pos + 1
                        } else {
                            pos
                        }
                    }
                    None => ds.config.rules.len(),
                };
                let insert_idx = insert_idx.min(ds.config.rules.len());
                ds.config.rules.insert(insert_idx, rule);
                order = Some(ds.config.rules.iter().map(|rule| rule.id.clone()).collect::<Vec<String>>());
            }
        }
        if let Some(order) = order {
            let direct = DirectWrite::Rules(DirectRuleChange::Reorder(order.clone()));
            self.enqueue_ipc(IpcJobKind::ReorderRules, IpcRequest::ReorderRules(order), Some(direct));
        }
    }

    pub fn toggle_rule(&mut self, id: &str, enabled: bool) {
        let direct = DirectWrite::Rules(DirectRuleChange::SetEnabled { id: id.to_string(), enabled });
        self.enqueue_ipc(
            IpcJobKind::ToggleRule { rule_id: id.to_string(), enabled },
            IpcRequest::ToggleRule { id: id.to_string(), enabled },
            Some(direct),
        );
    }

    pub fn test_run_rule(&mut self, id: &str) {
        self.active_test_runs.insert(id.to_string(), Instant::now());
        let mut overlay_ids = Vec::new();
        if let Some(rule) = self.daemon_state.as_ref().and_then(|s| s.config.rules.iter().find(|r| r.id == id)) {
            overlay_ids.extend(Self::extract_rule_overlay_ids(rule));
        }
        let mut rule_override = None;
        if let Some(rule) = &self.editing_rule {
            if rule.id == id {
                overlay_ids.extend(Self::extract_rule_overlay_ids(rule));
                rule_override = Some(Box::new(rule.clone()));
            }
        }
        for overlay_id in overlay_ids {
            self.active_test_overlays.insert(overlay_id);
        }
        self.enqueue_ipc(
            IpcJobKind::TestRunRule { rule_id: id.to_string() },
            IpcRequest::TestRunRule { id: id.to_string(), rule: rule_override },
            None,
        );
        self.daemon_sync.force();
    }

    /// Overlay ids referenced by the saved rule or by the open draft with the same id.
    fn rule_overlay_ids(&self, rule_id: &str) -> Option<Vec<String>> {
        self.daemon_state
            .as_ref()
            .and_then(|s| s.config.rules.iter().find(|r| r.id == rule_id))
            .map(Self::extract_rule_overlay_ids)
            .or_else(|| {
                self.editing_rule
                    .as_ref()
                    .filter(|r| r.id == rule_id)
                    .map(Self::extract_rule_overlay_ids)
            })
    }

    /// A test run starts only when the daemon accepts it; otherwise the optimistic running
    /// markers are rolled back.
    pub(super) fn apply_test_run_result(&mut self, rule_id: &str, result: Result<IpcResponse, String>) {
        match Self::mutation_failure(result) {
            None => self.set_toast(self.loc().toast_test_execution),
            Some(failure) => {
                self.active_test_runs.remove(rule_id);
                if let Some(ids) = self.rule_overlay_ids(rule_id) {
                    for overlay_id in ids {
                        self.active_test_overlays.remove(&overlay_id);
                    }
                }
                crate::logging::warn(
                    "GUI",
                    &format!("TestRunRule '{rule_id}' was not applied: {}", failure.diagnostic()),
                );
                self.set_toast(self.mutation_failure_toast(&failure));
            }
        }
        self.daemon_sync.force();
    }

    pub fn stop_rule_execution(&mut self, id: &str) {
        self.active_test_runs.remove(id);
        if let Some(ids) = self.rule_overlay_ids(id) {
            for ov_id in ids {
                self.active_test_overlays.remove(&ov_id);
            }
        }
        self.enqueue_ipc(
            IpcJobKind::StopRuleExecution { rule_id: id.to_string() },
            IpcRequest::StopRuleExecution(id.to_string()),
            None,
        );
        self.daemon_sync.force();
    }

    /// Clears the recorded events only once the preset is persisted. On failure the recording
    /// stays in the action — it is the only live copy, and the engine hydrates empty events from
    /// the library — and the error is surfaced instead of a silent loss.
    pub(super) fn apply_macro_save_result(&mut self, preset_id: &str, result: Result<IpcResponse, String>) {
        let failure = Self::mutation_failure(result);
        let pending = self
            .pending_macro_saves
            .iter()
            .position(|pending| pending.preset_id == preset_id)
            .map(|position| self.pending_macro_saves.remove(position));
        match (failure, pending) {
            (None, Some(pending)) => {
                if let Some(rule) = &mut self.editing_rule {
                    if let Some(crate::models::Action::Macro { preset_id: Some(ref pid), events, .. }) =
                        rule.actions.get_mut(pending.step_idx)
                    {
                        if pid.as_str() == preset_id {
                            events.clear();
                        }
                    }
                }
            }
            (None, None) => {}
            (Some(failure), _) => {
                crate::logging::warn(
                    "GUI",
                    &format!(
                        "SaveMacroPreset '{preset_id}' was not applied: {}",
                        failure.diagnostic()
                    ),
                );
                self.set_toast(self.mutation_failure_toast(&failure));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::DaemonState;
    use crate::models::Action;
    use super::super::ipc::IpcJobResult;

    fn macro_rule(state: &GuiState, preset_id: &str) -> Rule {
        let mut rule = state.initial_new_rule();
        rule.name = "Macro rule".into();
        rule.actions.push(Action::Macro {
            preset_id: Some(preset_id.into()),
            operation: crate::models::MacroOperation::Run,
            events: vec![crate::models::MacroEvent::KeyPress { key: "A".into() }],
            repeat: 1,
            speed: 1.0,
            infinite: false,
        });
        rule
    }

    fn macro_events(state: &GuiState, step_idx: usize) -> &[crate::models::MacroEvent] {
        match &state.editing_rule.as_ref().expect("draft").actions[step_idx] {
            Action::Macro { events, .. } => events,
            other => panic!("expected a macro action, got {other:?}"),
        }
    }

    #[test]
    fn test_save_rule_failure_keeps_the_draft() {
        let mut state = GuiState::new();
        state.language = crate::i18n::Language::English;
        let mut rule = state.initial_new_rule();
        rule.name = "Unsaved draft".into();
        state.editing_rule = Some(rule.clone());
        state.is_creating_new_rule = true;

        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::SaveRule { rule_id: rule.id.clone() },
            result: Err("Cannot connect to daemon pipe".into()),
        });

        let draft = state.editing_rule.as_ref().expect("a failed save must keep the only draft");
        assert_eq!(draft.id, rule.id);
        assert!(state.is_creating_new_rule, "the editor stays in create mode");
        let toast = state.toast.as_ref().expect("failure is reported").0.clone();
        assert_eq!(
            toast,
            crate::i18n::get(state.language).toast_request_failed.replace("{}", "Cannot connect to daemon pipe")
        );

        // A confirmed save is the only path that clears the editor.
        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::SaveRule { rule_id: rule.id.clone() },
            result: Ok(IpcResponse::Ok),
        });
        assert!(state.editing_rule.is_none());
        assert!(!state.is_creating_new_rule);
        assert_eq!(state.toast.as_ref().unwrap().0, crate::i18n::get(state.language).toast_rule_saved);
    }

    #[test]
    fn test_macro_preset_failure_keeps_the_recorded_events() {
        let mut state = GuiState::new();
        state.language = crate::i18n::Language::English;
        state.editing_rule = Some(macro_rule(&state, "macro_demo"));
        let preset_id = "macro_demo".to_string();
        state.pending_macro_saves.push(PendingMacroSave {
            step_idx: 0,
            preset_id: preset_id.clone(),
        });

        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::SaveMacroPreset { preset_id: preset_id.clone() },
            result: Err("daemon unavailable".into()),
        });

        assert!(!macro_events(&state, 0).is_empty(), "a failed preset save must keep the recording");
        assert!(state.pending_macro_saves.is_empty(), "no retry is queued");
        let toast = state.toast.as_ref().expect("failure is reported").0.clone();
        assert_eq!(
            toast,
            crate::i18n::get(state.language).toast_request_failed.replace("{}", "daemon unavailable")
        );

        // Confirmation is the only path that drops the events from the rule.
        state.pending_macro_saves.push(PendingMacroSave {
            step_idx: 0,
            preset_id: preset_id.clone(),
        });
        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::SaveMacroPreset { preset_id },
            result: Ok(IpcResponse::Ok),
        });
        assert!(macro_events(&state, 0).is_empty(), "confirmed presets are referenced by id");
    }

    #[test]
    fn test_reorder_rule_by_id() {
        let mut state = GuiState::new();
        let r1 = Rule { id: "rule-1".into(), ..Rule::default() };
        let r2 = Rule { id: "rule-2".into(), ..Rule::default() };
        let r3 = Rule { id: "rule-3".into(), ..Rule::default() };
        let r4 = Rule { id: "rule-4".into(), ..Rule::default() };
        let config = crate::config::AppConfig {
            rules: vec![r1, r2, r3, r4],
            ..Default::default()
        };
        state.daemon_state = Some(DaemonState {
            autorun_enabled: false,
            uptime_secs: 0,
            daemon_memory_bytes: 0,
            active_rules_count: 0,
            total_rules_count: 0,
            config,
            logs: vec![],
            active_rule_executions: std::collections::HashMap::new(),
            active_overlay_ids: std::collections::HashSet::new(),
            active_test_rule_ids: std::collections::HashSet::new(),
            available_update: None,
            update_progress: None,
            update_status: None,
            pending_open_update_modal: false,
            pending_auto_start_update: false,
        });

        // Move rule-1 after rule-3
        state.reorder_rule_by_id("rule-1", Some("rule-3"), true);
        let ids: Vec<_> = state.daemon_state.as_ref().unwrap().config.rules.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["rule-2", "rule-3", "rule-1", "rule-4"]);

        // Move rule-4 before rule-2
        state.reorder_rule_by_id("rule-4", Some("rule-2"), false);
        let ids: Vec<_> = state.daemon_state.as_ref().unwrap().config.rules.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["rule-4", "rule-2", "rule-3", "rule-1"]);

        // Move rule-2 to bottom (target None)
        state.reorder_rule_by_id("rule-2", None, false);
        let ids: Vec<_> = state.daemon_state.as_ref().unwrap().config.rules.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["rule-4", "rule-3", "rule-1", "rule-2"]);
    }
}
