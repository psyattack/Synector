//! Navigation and modal state: the payload half of every modal `last_*` closing mirror plus
//! the open/close/commit methods that drive the catalogs, editors and the process picker.

use crate::ipc::IpcRequest;

use super::drafts::PendingMacroSave;
use super::ipc::{DirectLibraryChange, DirectWrite, IpcJobKind};
use super::GuiState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveView {
    Rules,
    Analytics,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProcessSortColumn {
    #[default]
    Name,
    Pid,
    WindowTitle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProcessCategoryFilter {
    #[default]
    All,
    WindowedOnly,
    BackgroundOnly,
}

#[derive(Debug, Clone)]
pub struct ExpandedScriptEditorState {
    pub step_idx: usize,
    pub is_exit_pipeline: bool,
    pub language: crate::gui::components::script_highlighter::ScriptLanguage,
}

#[derive(Debug, Clone)]
pub struct MacroEditorModalState {
    pub step_idx: usize,
    pub is_exit_pipeline: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessPickerTarget {
    Trigger,
    Action(usize),
    ExitAction(usize),
    Condition(usize),
    ActionCondition { action_idx: usize, cond_idx: usize, is_exit: bool },
    ActionElseIfCondition { action_idx: usize, branch_idx: usize, cond_idx: usize, is_exit: bool },
    IfElseThen { parent_idx: usize, sub_idx: usize, is_exit: bool },
    IfElseElseIf { parent_idx: usize, branch_idx: usize, sub_idx: usize, is_exit: bool },
    IfElseElse { parent_idx: usize, sub_idx: usize, is_exit: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IfElseCatalogTarget {
    Then(usize),
    ElseIf { parent_idx: usize, branch_idx: usize },
    Else(usize),
}

#[derive(Debug, Clone)]
pub struct ActionCatalogModalState {
    pub is_exit_pipeline: bool,
    pub insert_idx: Option<usize>,
    pub selected_category: usize,
    pub search_query: String,
    pub ifelse_target: Option<IfElseCatalogTarget>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionCatalogTarget {
    RulePrecondition,
    ChangeRulePrecondition(usize),
    ActionIfElse { action_idx: usize, is_exit: bool },
    ChangeActionIfElse { action_idx: usize, cond_idx: usize, is_exit: bool },
    ActionElseIf { action_idx: usize, branch_idx: usize, is_exit: bool },
    ChangeActionElseIf { action_idx: usize, branch_idx: usize, cond_idx: usize, is_exit: bool },
}

#[derive(Debug, Clone, Default)]
pub struct TriggerCatalogState {
    pub selected_category: usize,
    pub search_query: String,
}

#[derive(Debug, Clone, Default)]
pub struct ConditionCatalogState {
    pub selected_category: usize,
    pub search_query: String,
    pub target: Option<ConditionCatalogTarget>,
}

#[derive(Debug, Clone, Default)]
pub struct ElementCatalogModalState {
    pub search_query: String,
    pub selected_category: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RuleSortMode {
    #[default]
    Default,
    NameAsc,
    NameDesc,
    TriggerType,
    ActionCount,
    Status,
}

impl GuiState {
    pub fn open_script_editor(
        &mut self,
        step_idx: usize,
        is_exit_pipeline: bool,
        language: crate::gui::components::script_highlighter::ScriptLanguage,
    ) {
        let editor = ExpandedScriptEditorState {
            step_idx,
            is_exit_pipeline,
            language,
        };
        self.expanded_script_editor = Some(editor.clone());
        self.last_script_editor = Some(editor);
    }

    pub fn close_script_editor(&mut self) {
        self.expanded_script_editor = None;
    }

    pub fn open_macro_editor(&mut self, step_idx: usize, is_exit_pipeline: bool) {
        if let Some(rule) = &mut self.editing_rule {
            let actions = if is_exit_pipeline { &mut rule.on_exit_actions } else { &mut rule.actions };
            if let Some(crate::models::Action::Macro { preset_id: Some(ref pid), events, repeat, speed, infinite, .. }) = actions.get_mut(step_idx) {
                if events.is_empty() {
                    let library = crate::config::Library::read();
                    if let Some(preset) = library.find_macro(pid) {
                        *events = preset.events.clone();
                        *repeat = preset.repeat;
                        *speed = preset.speed;
                        *infinite = preset.infinite;
                    }
                }
            }
        }
        let editor = MacroEditorModalState {
            step_idx,
            is_exit_pipeline,
        };
        self.macro_editor = Some(editor.clone());
        self.last_macro_editor = Some(editor);
        self.macro_preset_save_name = self
            .editing_rule
            .as_ref()
            .map(|r| r.name.clone())
            .unwrap_or_else(|| self.loc().macro_studio_custom_preset_name.to_string());
        crate::gui::macro_recorder::set_studio_active(true);
    }

    pub fn close_macro_editor(&mut self) {
        let rec = crate::gui::macro_recorder::MacroRecorder::global();
        if rec.is_active() {
            rec.stop_recording();
        }
        crate::gui::macro_recorder::stop_macro_test();
        if let Some(edit_info) = self.macro_editor.take() {
            let mut pending_preset = None;
            if let Some(rule) = &mut self.editing_rule {
                let actions = if edit_info.is_exit_pipeline { &mut rule.on_exit_actions } else { &mut rule.actions };
                if let Some(crate::models::Action::Macro { preset_id: Some(ref pid), events, repeat, speed, infinite, .. }) = actions.get_mut(edit_info.step_idx) {
                    let library = crate::config::Library::read();
                    let preset_name = library.find_macro(pid).map(|p| p.name.clone()).unwrap_or_else(|| pid.clone());
                    // The recorded events stay in the action until the daemon confirms the preset:
                    // they are the only live copy and the engine hydrates empty events from the
                    // library, so clearing them on a failed save would lose the recording.
                    pending_preset = Some(crate::models::MacroPreset {
                        id: pid.clone(),
                        name: preset_name,
                        events: events.clone(),
                        repeat: *repeat,
                        speed: *speed,
                        infinite: *infinite,
                    });
                }
            }
            if let Some(preset) = pending_preset {
                self.pending_macro_saves.push(PendingMacroSave {
                    step_idx: edit_info.step_idx,
                    is_exit_pipeline: edit_info.is_exit_pipeline,
                    preset_id: preset.id.clone(),
                });
                let direct = DirectWrite::Library(DirectLibraryChange::UpsertMacroPreset(Box::new(preset.clone())));
                self.enqueue_ipc(
                    IpcJobKind::SaveMacroPreset { preset_id: preset.id.clone() },
                    IpcRequest::SaveMacroPreset(preset),
                    Some(direct),
                );
            }
        }
        crate::gui::macro_recorder::set_studio_active(false);
    }

    pub fn open_action_catalog(&mut self, is_exit_pipeline: bool, insert_idx: Option<usize>) {
        let catalog = ActionCatalogModalState {
            is_exit_pipeline,
            insert_idx,
            selected_category: 0,
            search_query: String::new(),
            ifelse_target: None,
        };
        self.action_catalog = Some(catalog.clone());
        self.last_action_catalog = Some(catalog);
    }

    pub fn open_action_catalog_nested(&mut self, is_exit_pipeline: bool, target: IfElseCatalogTarget) {
        let catalog = ActionCatalogModalState {
            is_exit_pipeline,
            insert_idx: None,
            selected_category: 0,
            search_query: String::new(),
            ifelse_target: Some(target),
        };
        self.action_catalog = Some(catalog.clone());
        self.last_action_catalog = Some(catalog);
    }

    pub fn close_action_catalog(&mut self) {
        self.action_catalog = None;
    }

    pub fn open_trigger_catalog(&mut self) {
        let catalog = TriggerCatalogState::default();
        self.trigger_catalog = Some(catalog.clone());
        self.last_trigger_catalog = Some(catalog);
    }

    pub fn close_trigger_catalog(&mut self) {
        self.trigger_catalog = None;
    }

    pub fn set_trigger_from_catalog(&mut self, trigger: Option<crate::models::Trigger>) {
        self.push_undo_snapshot();
        if let Some(rule) = &mut self.editing_rule {
            rule.trigger = trigger;
        }
        self.close_trigger_catalog();
    }

    pub fn open_condition_catalog_with_target(&mut self, target: ConditionCatalogTarget) {
        let catalog = ConditionCatalogState {
            target: Some(target),
            ..Default::default()
        };
        self.condition_catalog = Some(catalog.clone());
        self.last_condition_catalog = Some(catalog);
    }

    #[cfg(test)]
    pub fn open_condition_catalog(&mut self) {
        self.open_condition_catalog_with_target(ConditionCatalogTarget::RulePrecondition);
    }

    pub fn close_condition_catalog(&mut self) {
        self.condition_catalog = None;
    }

    pub fn open_element_catalog(&mut self) {
        let catalog = ElementCatalogModalState::default();
        self.element_catalog = Some(catalog.clone());
        self.last_element_catalog = Some(catalog);
    }

    pub fn close_element_catalog(&mut self) {
        self.element_catalog = None;
    }

    pub fn apply_condition_from_catalog(&mut self, condition: crate::models::RuleCondition) {
        self.push_undo_snapshot();
        let target = self
            .condition_catalog
            .as_ref()
            .and_then(|c| c.target)
            .unwrap_or(ConditionCatalogTarget::RulePrecondition);

        if let Some(rule) = &mut self.editing_rule {
            match target {
                ConditionCatalogTarget::RulePrecondition => {
                    rule.conditions.push(crate::models::RuleConditionItem::new(condition));
                }
                ConditionCatalogTarget::ChangeRulePrecondition(idx) => {
                    if let Some(item) = rule.conditions.get_mut(idx) {
                        item.condition = condition;
                    }
                }
                ConditionCatalogTarget::ActionIfElse { action_idx, is_exit } => {
                    let actions = if is_exit { &mut rule.on_exit_actions } else { &mut rule.actions };
                    if let Some(crate::models::Action::IfElse { conditions, .. }) = actions.get_mut(action_idx) {
                        conditions.push(crate::models::RuleConditionItem::new(condition));
                    }
                }
                ConditionCatalogTarget::ChangeActionIfElse { action_idx, cond_idx, is_exit } => {
                    let actions = if is_exit { &mut rule.on_exit_actions } else { &mut rule.actions };
                    if let Some(crate::models::Action::IfElse { conditions, .. }) = actions.get_mut(action_idx) {
                        if let Some(item) = conditions.get_mut(cond_idx) {
                            item.condition = condition;
                        }
                    }
                }
                ConditionCatalogTarget::ActionElseIf { action_idx, branch_idx, is_exit } => {
                    let actions = if is_exit { &mut rule.on_exit_actions } else { &mut rule.actions };
                    if let Some(crate::models::Action::IfElse { else_if_branches, .. }) = actions.get_mut(action_idx) {
                        if let Some(branch) = else_if_branches.get_mut(branch_idx) {
                            branch.conditions.push(crate::models::RuleConditionItem::new(condition));
                        }
                    }
                }
                ConditionCatalogTarget::ChangeActionElseIf { action_idx, branch_idx, cond_idx, is_exit } => {
                    let actions = if is_exit { &mut rule.on_exit_actions } else { &mut rule.actions };
                    if let Some(crate::models::Action::IfElse { else_if_branches, .. }) = actions.get_mut(action_idx) {
                        if let Some(branch) = else_if_branches.get_mut(branch_idx) {
                            if let Some(item) = branch.conditions.get_mut(cond_idx) {
                                item.condition = condition;
                            }
                        }
                    }
                }
            }
        }
        self.close_condition_catalog();
    }

    pub fn add_action_from_catalog(
        &mut self,
        action: crate::models::Action,
        is_exit_pipeline: bool,
        insert_idx: Option<usize>,
        ifelse_target: Option<IfElseCatalogTarget>,
    ) {
        self.push_undo_snapshot();
        if let Some(rule) = &mut self.editing_rule {
            if let Some(target) = ifelse_target {
                let list = if is_exit_pipeline {
                    &mut rule.on_exit_actions
                } else {
                    &mut rule.actions
                };
                match target {
                    IfElseCatalogTarget::Then(parent_idx) => {
                        if let Some(crate::models::Action::IfElse { then_actions, .. }) = list.get_mut(parent_idx) {
                            then_actions.push(action);
                        }
                    }
                    IfElseCatalogTarget::ElseIf { parent_idx, branch_idx } => {
                        if let Some(crate::models::Action::IfElse { else_if_branches, .. }) = list.get_mut(parent_idx) {
                            if let Some(branch) = else_if_branches.get_mut(branch_idx) {
                                branch.actions.push(action);
                            }
                        }
                    }
                    IfElseCatalogTarget::Else(parent_idx) => {
                        if let Some(crate::models::Action::IfElse { else_actions, .. }) = list.get_mut(parent_idx) {
                            else_actions.push(action);
                        }
                    }
                }
            } else {
                let list = if is_exit_pipeline {
                    &mut rule.on_exit_actions
                } else {
                    &mut rule.actions
                };
                if let Some(idx) = insert_idx {
                    let clamped = idx.min(list.len());
                    list.insert(clamped, action);
                } else {
                    list.push(action);
                }
            }
        }
        self.close_action_catalog();
    }

    /// Whether any modal popup (script editor, action catalog, process picker, or changelog) is currently open.
    pub fn is_modal_open(&self) -> bool {
        self.expanded_script_editor.is_some()
            || self.action_catalog.is_some()
            || self.trigger_catalog.is_some()
            || self.condition_catalog.is_some()
            || self.element_catalog.is_some()
            || self.picker_target.is_some()
            || (self.show_changelog_modal && self.available_update.is_some())
            || self.show_guide_modal
            || self.macro_editor.is_some()
            || self.overlay_studio.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Action, ProcessMatchMode, Rule, RuleCondition, RuleConditionItem, SystemEventType};

    #[test]
    fn test_is_modal_open() {
        let mut state = GuiState::new();
        assert!(!state.is_modal_open());

        // Process picker
        state.open_process_picker(ProcessPickerTarget::Trigger);
        assert!(state.is_modal_open());
        state.close_process_picker();
        assert!(!state.is_modal_open());

        // Action catalog
        state.action_catalog = Some(ActionCatalogModalState {
            is_exit_pipeline: false,
            insert_idx: None,
            search_query: String::new(),
            selected_category: 0,
            ifelse_target: None,
        });
        assert!(state.is_modal_open());
        state.action_catalog = None;
        assert!(!state.is_modal_open());

        // Script editor
        state.expanded_script_editor = Some(ExpandedScriptEditorState {
            step_idx: 0,
            is_exit_pipeline: false,
            language: crate::gui::components::script_highlighter::ScriptLanguage::PowerShell,
        });
        assert!(state.is_modal_open());
        state.expanded_script_editor = None;
        assert!(!state.is_modal_open());

        // Changelog modal (requires show_changelog_modal && available_update.is_some())
        state.show_changelog_modal = true;
        assert!(!state.is_modal_open());
        state.available_update = Some(crate::updater::ReleaseInfo {
            version: "1.4.0".into(),
            tag_name: "v1.4.0".into(),
            name: "v1.4.0".into(),
            body: "Notes".into(),
            html_url: "http://example.com".into(),
            portable: Some(crate::updater::UpdateAsset {
                name: "Synector-v1.4.0-Windows-x64.zip".into(),
                url: "http://example.com/asset.zip".into(),
                size: 1024,
                sha256: None,
            }),
            installer: None,
        });
        assert!(state.is_modal_open());
        state.show_changelog_modal = false;
        assert!(!state.is_modal_open());

        // Guide modal
        state.show_guide_modal = true;
        assert!(state.is_modal_open());
        state.show_guide_modal = false;
        assert!(!state.is_modal_open());

        // Macro editor
        state.open_macro_editor(0, false);
        assert!(state.is_modal_open());
        state.close_macro_editor();
        assert!(!state.is_modal_open());

        // Overlay studio
        state.open_overlay_studio("overlay_crosshair");
        assert!(state.is_modal_open());
        state.close_overlay_studio();
        assert!(!state.is_modal_open());
    }

    #[test]
    fn test_condition_catalog_targets_application() {
        let mut state = GuiState::new();
        let mut rule = Rule::default();
        rule.actions.push(Action::IfElse {
            conditions: vec![RuleConditionItem::new(RuleCondition::default_process())],
            then_actions: vec![],
            else_if_branches: vec![crate::models::ElseIfBranch {
                conditions: vec![RuleConditionItem::new(RuleCondition::default_process())],
                actions: vec![],
            }],
            else_actions: vec![],
        });
        state.editing_rule = Some(rule);

        // 1. Add rule precondition
        state.open_condition_catalog_with_target(ConditionCatalogTarget::RulePrecondition);
        state.apply_condition_from_catalog(RuleCondition::TimeRange {
            start_time: "08:00".into(),
            end_time: "17:00".into(),
            days_of_week: vec![],
        });
        assert_eq!(state.editing_rule.as_ref().unwrap().conditions.len(), 1);
        assert!(matches!(state.editing_rule.as_ref().unwrap().conditions[0].condition, RuleCondition::TimeRange { .. }));

        // 2. Change rule precondition
        state.open_condition_catalog_with_target(ConditionCatalogTarget::ChangeRulePrecondition(0));
        state.apply_condition_from_catalog(RuleCondition::FileWatcher {
            path: "C:\\test.txt".into(),
            filter_extension: None,
            must_exist: true,
        });
        assert_eq!(state.editing_rule.as_ref().unwrap().conditions.len(), 1);
        assert!(matches!(state.editing_rule.as_ref().unwrap().conditions[0].condition, RuleCondition::FileWatcher { .. }));

        // 3. Change primary IfElse condition (card 0)
        state.open_condition_catalog_with_target(ConditionCatalogTarget::ChangeActionIfElse {
            action_idx: 0,
            cond_idx: 0,
            is_exit: false,
        });
        state.apply_condition_from_catalog(RuleCondition::WindowFocused {
            process_name: "code.exe".into(),
            match_mode: ProcessMatchMode::Exact,
            must_be_focused: true,
        });
        if let Some(Action::IfElse { conditions, .. }) = state.editing_rule.as_ref().unwrap().actions.first() {
            assert!(matches!(conditions[0].condition, RuleCondition::WindowFocused { .. }));
        } else {
            panic!("Expected IfElse action");
        }

        // 4. Add condition to primary IfElse
        state.open_condition_catalog_with_target(ConditionCatalogTarget::ActionIfElse {
            action_idx: 0,
            is_exit: false,
        });
        state.apply_condition_from_catalog(RuleCondition::TimeRange {
            start_time: "10:00".into(),
            end_time: "12:00".into(),
            days_of_week: vec![],
        });
        if let Some(Action::IfElse { conditions, .. }) = state.editing_rule.as_ref().unwrap().actions.first() {
            assert_eq!(conditions.len(), 2);
            assert!(matches!(conditions[1].condition, RuleCondition::TimeRange { .. }));
        } else {
            panic!("Expected IfElse action");
        }

        // 5. Change condition in else_if_branches
        state.open_condition_catalog_with_target(ConditionCatalogTarget::ChangeActionElseIf {
            action_idx: 0,
            branch_idx: 0,
            cond_idx: 0,
            is_exit: false,
        });
        state.apply_condition_from_catalog(RuleCondition::Event {
            event_type: SystemEventType::PowerSourceBattery,
            idle_duration_sec: 0,
        });
        if let Some(Action::IfElse { else_if_branches, .. }) = state.editing_rule.as_ref().unwrap().actions.first() {
            assert!(matches!(else_if_branches[0].conditions[0].condition, RuleCondition::Event { .. }));
        } else {
            panic!("Expected IfElse action");
        }

        // 6. Add condition to else_if_branches
        state.open_condition_catalog_with_target(ConditionCatalogTarget::ActionElseIf {
            action_idx: 0,
            branch_idx: 0,
            is_exit: false,
        });
        state.apply_condition_from_catalog(RuleCondition::TimeRange {
            start_time: "13:00".into(),
            end_time: "14:00".into(),
            days_of_week: vec![],
        });
        if let Some(Action::IfElse { else_if_branches, .. }) = state.editing_rule.as_ref().unwrap().actions.first() {
            assert_eq!(else_if_branches[0].conditions.len(), 2);
            assert!(matches!(else_if_branches[0].conditions[1].condition, RuleCondition::TimeRange { .. }));
        } else {
            panic!("Expected IfElse action");
        }
    }

    /// The catalog's commit adapter inserts at the requested top-level index, appends to the exit
    /// pipeline, and routes a nested pick into Then / ElseIf / Else instead of the top level.
    #[test]
    fn test_action_catalog_commit_targets_pipeline() {
        let notification = |title: &str| Action::Notification {
            title: title.to_string(),
            message: title.to_string(),
        };
        let kill = |name: &str| Action::KillProcess {
            process_name: name.to_string(),
            kill_tree: false,
            force: false,
        };

        let mut rule = Rule {
            name: "Catalog commit".to_string(),
            ..Rule::default()
        };
        rule.actions.push(notification("a"));
        rule.actions.push(notification("b"));
        rule.actions.push(Action::IfElse {
            conditions: vec![RuleConditionItem::new(RuleCondition::default_process())],
            then_actions: vec![],
            else_if_branches: vec![crate::models::ElseIfBranch {
                conditions: vec![RuleConditionItem::new(RuleCondition::default_process())],
                actions: vec![],
            }],
            else_actions: vec![],
        });

        let mut state = GuiState::new();
        state.editing_rule = Some(rule);

        // 1. Insert at the requested index; the modal closes on commit.
        state.open_action_catalog(false, Some(1));
        state.add_action_from_catalog(kill("new.exe"), false, Some(1), None);
        assert!(state.action_catalog.is_none(), "a commit closes the catalog");
        {
            let actions = &state.editing_rule.as_ref().unwrap().actions;
            assert_eq!(actions.len(), 4);
            assert!(matches!(&actions[1], Action::KillProcess { process_name, .. } if process_name == "new.exe"));
        }

        // 2. An out-of-range index is clamped to the end, and the exit pipeline is separate.
        state.open_action_catalog(false, Some(99));
        state.add_action_from_catalog(notification("tail"), false, Some(99), None);
        assert!(matches!(
            state.editing_rule.as_ref().unwrap().actions.last(),
            Some(Action::Notification { title, .. }) if title == "tail"
        ));

        state.open_action_catalog(true, None);
        state.add_action_from_catalog(notification("exit"), true, None, None);
        assert_eq!(state.editing_rule.as_ref().unwrap().on_exit_actions.len(), 1);
        assert_eq!(state.editing_rule.as_ref().unwrap().actions.len(), 5);

        // 3. Nested targets append inside the If/Else instead of the top level.
        let ifelse = 3;
        state.open_action_catalog_nested(false, IfElseCatalogTarget::Then(ifelse));
        state.add_action_from_catalog(
            notification("then"),
            false,
            None,
            Some(IfElseCatalogTarget::Then(ifelse)),
        );
        state.open_action_catalog_nested(false, IfElseCatalogTarget::Else(ifelse));
        state.add_action_from_catalog(
            notification("else"),
            false,
            None,
            Some(IfElseCatalogTarget::Else(ifelse)),
        );
        state.open_action_catalog_nested(
            false,
            IfElseCatalogTarget::ElseIf {
                parent_idx: ifelse,
                branch_idx: 0,
            },
        );
        state.add_action_from_catalog(
            notification("elseif"),
            false,
            None,
            Some(IfElseCatalogTarget::ElseIf {
                parent_idx: ifelse,
                branch_idx: 0,
            }),
        );

        let actions = &state.editing_rule.as_ref().unwrap().actions;
        assert_eq!(actions.len(), 5, "nested commits never touch the top level");
        if let Some(Action::IfElse {
            then_actions,
            else_if_branches,
            else_actions,
            ..
        }) = actions.get(ifelse)
        {
            assert!(matches!(then_actions.as_slice(), [Action::Notification { title, .. }] if title == "then"));
            assert!(matches!(else_actions.as_slice(), [Action::Notification { title, .. }] if title == "else"));
            assert!(matches!(
                else_if_branches[0].actions.as_slice(),
                [Action::Notification { title, .. }] if title == "elseif"
            ));
        } else {
            panic!("expected the IfElse action at index {ifelse}");
        }

        // Every commit pushed exactly one undo snapshot.
        assert_eq!(state.undo_stack.len(), 6);
    }

    /// Choosing a trigger replaces the draft's trigger and closes the catalog; `None` clears it.
    #[test]
    fn test_trigger_catalog_commit_replaces_trigger() {
        let rule = Rule {
            name: "Trigger commit".to_string(),
            trigger: None,
            ..Rule::default()
        };
        let mut state = GuiState::new();
        state.editing_rule = Some(rule);

        state.open_trigger_catalog();
        state.set_trigger_from_catalog(Some(crate::models::Trigger::Manual));
        assert!(matches!(
            state.editing_rule.as_ref().unwrap().trigger,
            Some(crate::models::Trigger::Manual)
        ));
        assert!(state.trigger_catalog.is_none(), "a commit closes the catalog");

        state.open_trigger_catalog();
        state.set_trigger_from_catalog(Some(crate::models::Trigger::Process {
            process_name: "target.exe".to_string(),
            match_mode: ProcessMatchMode::Exact,
            single_instance: false,
        }));
        assert!(matches!(
            &state.editing_rule.as_ref().unwrap().trigger,
            Some(crate::models::Trigger::Process { process_name, .. }) if process_name == "target.exe"
        ));

        state.open_trigger_catalog();
        state.set_trigger_from_catalog(None);
        assert!(state.editing_rule.as_ref().unwrap().trigger.is_none());

        assert_eq!(state.undo_stack.len(), 3, "each commit is one undo step");
    }

    #[test]
    fn test_macro_preset_default_name_is_localized() {
        let mut state = GuiState::new();
        state.language = crate::i18n::Language::English;
        state.open_macro_editor(0, false);
        assert_eq!(
            state.macro_preset_save_name,
            crate::i18n::get(crate::i18n::Language::English).macro_studio_custom_preset_name
        );

        state.language = crate::i18n::Language::Russian;
        state.open_macro_editor(0, false);
        assert_eq!(
            state.macro_preset_save_name,
            crate::i18n::get(crate::i18n::Language::Russian).macro_studio_custom_preset_name
        );
        assert_ne!(
            crate::i18n::get(crate::i18n::Language::English).macro_studio_custom_preset_name,
            crate::i18n::get(crate::i18n::Language::Russian).macro_studio_custom_preset_name,
            "the default preset name is translated, not an English literal"
        );
    }
}
