//! Drag-and-drop action pipeline and its top-level/nested hosts.

use super::*;
use crate::gui::components::dnd;

pub(crate) fn render_add_action_button(
    ui: &mut Ui,
    state: &mut GuiState,
    loc: &crate::i18n::Strings,
    is_exit_pipeline: bool,
    t: &Tokens,
) {
    ui.add_space(2.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
    let is_hov = resp.hovered();
    let p = ui.painter();

    let border = if is_hov {
        Stroke::new(1.0, t.accent_ink)
    } else {
        Stroke::new(1.0, t.muted.gamma_multiply(0.35))
    };
    p.rect_stroke(rect, t.inset_corner(), border, egui::StrokeKind::Inside);

    let text_color = if is_hov { t.accent_ink } else { t.faint };
    let plus_galley = p.layout_no_wrap(
        egui_phosphor::regular::PLUS.to_string(),
        egui::FontId::new(12.0, theme::phosphor()),
        text_color,
    );
    let label_galley = widgets::caps(ui, loc.inspector_btn_open_catalog, 10.0, theme::mono(), 0.8, text_color);

    let spacing = 8.0;
    let plus_w = plus_galley.size().x;
    let plus_h = plus_galley.size().y;
    let total_w = plus_w + spacing + label_galley.size().x;
    let start_x = rect.center().x - total_w * 0.5;

    let plus_pos = pos2(start_x, rect.center().y - plus_h * 0.5);
    p.galley(plus_pos, plus_galley, text_color);

    let label_pos = pos2(start_x + plus_w + spacing, rect.center().y - label_galley.size().y * 0.5);
    p.galley_with_override_text_color(label_pos, label_galley, text_color);

    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);

    if resp.clicked() {
        state.open_action_catalog(is_exit_pipeline, None);
    }
}

/// Renders one drag-and-drop action pipeline. The rule's main/exit pipelines and the nested
/// If/Else branches share it, so drop slots, edge auto-scroll, the floating preview, the
/// infinite-macro warning and unreachable-step dimming behave identically everywhere.
pub(crate) fn render_action_pipeline(
    ui: &mut Ui,
    actions: &mut Vec<Action>,
    pipeline_id: &str,
    prefix: &str,
    t: &Tokens,
    host: &mut dyn PipelineHost,
) {
    let dnd_id = ui.scope_id().with(("action_pipeline_dnd", pipeline_id));
    let loc = host.loc();

    let mut dnd_session: Option<DndDragSession> = ui.data_mut(|d| d.get_temp(dnd_id));

    // Cancel on Escape key
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        dnd_session = None;
        ui.data_mut(|d| d.remove_temp::<DndDragSession>(dnd_id));
    }

    let pointer_released = ui.input(|i| i.pointer.any_released());
    let pointer_pos = ui.input(|i| i.pointer.interact_pos().or(i.pointer.hover_pos()));

    // When dragging: handle mouse wheel scrolling and edge auto-scrolling
    if dnd_session.is_some() {
        // 1. Mouse wheel scrolling while holding drag
        dnd::wheel_scroll(ui);

        // 2. Edge auto-scrolling when pointer is near top or bottom of viewport
        if let Some(pointer) = pointer_pos {
            let viewport = ui.clip_rect();
            // Check if pointer is horizontally within or near the drawer column
            if pointer.x >= viewport.left() - 60.0 && pointer.x <= viewport.right() + 60.0 {
                dnd::edge_autoscroll(ui, pointer, 90.0);
            }
        }
    }

    let active_src = dnd_session.as_ref().map(|s| s.source_idx);
    let active_dst = dnd_session.as_ref().map(|s| s.target_idx);

    let mut remove_idx = None;
    let mut clone_idx = None;
    let mut drag_started_idx = None;
    let mut current_centers = Vec::with_capacity(actions.len());
    let total = actions.len();
    let mut has_infinite_prior = false;

    for (idx, action) in actions.iter_mut().enumerate() {
        // Render drop slot BEFORE card idx
        render_drop_slot(ui, loc, idx, active_dst, active_src, pipeline_id, prefix, t);

        let is_ghost = active_src == Some(idx);
        let is_dimmed = has_infinite_prior;
        let outcome = host.render_card(ui, action, idx, is_ghost, is_dimmed);
        current_centers.push(outcome.card_rect.center().y);

        let is_infinite_macro = matches!(action, Action::Macro { infinite: true, .. });
        if is_infinite_macro {
            has_infinite_prior = true;
            host.render_infinite_warning(ui);
        }

        if outcome.remove_clicked {
            remove_idx = Some(idx);
        }
        if outcome.clone_clicked {
            clone_idx = Some(idx);
        }
        if outcome.drag_started {
            drag_started_idx = Some(idx);
        }
        host.apply(outcome, idx);
    }

    // Render trailing drop slot (after last card)
    if total > 0 {
        render_drop_slot(ui, loc, total, active_dst, active_src, pipeline_id, prefix, t);
    }

    // Update target_idx using fresh current_centers from this exact frame
    if let (Some(session), Some(pointer)) = (&mut dnd_session, pointer_pos) {
        if !current_centers.is_empty() {
            session.target_idx = dnd::slot_from_pointer(&current_centers, pointer.y);
        }
    }

    // Commit reorder on mouse release
    if pointer_released {
        if let Some(session) = dnd_session {
            let src = session.source_idx;
            let dst = session.target_idx;
            if src < actions.len() && dst <= actions.len() {
                if dst < src {
                    host.before_mutation();
                    let item = actions.remove(src);
                    actions.insert(dst, item);
                } else if dst > src + 1 {
                    host.before_mutation();
                    let item = actions.remove(src);
                    actions.insert(dst - 1, item);
                }
            }
            dnd_session = None;
            ui.data_mut(|d| d.remove_temp::<DndDragSession>(dnd_id));
        }
    }

    // Handle step cloning
    if let Some(idx) = clone_idx {
        if idx < actions.len() {
            host.before_mutation();
            let cloned = actions[idx].duplicate_with_fresh_ids();
            actions.insert(idx + 1, cloned);
        }
    }
    if let Some(idx) = remove_idx {
        if idx < actions.len() {
            host.before_mutation();
            actions.remove(idx);
        }
    }

    // Handle newly started drag
    if let Some(src_idx) = drag_started_idx {
        dnd_session = Some(DndDragSession {
            source_idx: src_idx,
            target_idx: src_idx,
            card_width: ui.available_width(),
        });
    }

    // Save or clear dnd_session
    if let Some(action) = dnd_preview_action(actions, &mut dnd_session) {
        let session = dnd_session.expect("a resolved preview keeps its session");
        ui.data_mut(|d| d.insert_temp(dnd_id, session));
        ui.ctx().request_repaint();

        // Render floating drag preview card!
        render_floating_drag_preview(
            ui,
            loc,
            action,
            session,
            pointer_pos,
            prefix,
            pipeline_id,
            t,
        );
    } else if dnd_session.is_none() {
        ui.data_mut(|d| d.remove_temp::<DndDragSession>(dnd_id));
    }

    // Add action button at the bottom of the pipeline
    host.render_add_button(ui);
}

/// Host for the rule's own main and exit pipelines.
pub(crate) struct RulePipelineHost<'a> {
    pub(crate) state: &'a mut GuiState,
    pub(crate) t: &'a Tokens,
    pub(crate) is_exit_pipeline: bool,
    pub(crate) exec_opt: Option<crate::ipc::protocol::ActiveRuleExecutionState>,
    pub(crate) switch_to_hunter: Option<usize>,
}

impl PipelineHost for RulePipelineHost<'_> {
    fn loc(&self) -> &'static crate::i18n::Strings {
        self.state.loc()
    }

    fn render_card(
        &mut self,
        ui: &mut Ui,
        action: &mut Action,
        idx: usize,
        is_ghost: bool,
        is_dimmed: bool,
    ) -> PipelineCardOutcome {
        let is_overlay_active = match action {
            Action::Overlay { overlay_id, custom_overlay, .. } => {
                let ov_id = custom_overlay.as_ref().map(|o| o.id.as_str()).unwrap_or(overlay_id.as_str());
                self.state.daemon_state.as_ref().is_some_and(|d| d.active_overlay_ids.contains(ov_id))
                    || self.state.active_test_overlays.contains(ov_id)
            }
            _ => false,
        };
        let is_executing = is_overlay_active
            || self.exec_opt.is_some_and(|e| e.is_exit == self.is_exit_pipeline && e.current_step == idx);
        render_action_card(
            ui,
            &*self.state,
            action,
            idx,
            self.t,
            self.is_exit_pipeline,
            is_ghost,
            is_executing,
            is_dimmed,
        )
    }

    fn apply(&mut self, outcome: PipelineCardOutcome, idx: usize) {
        if let Some(lang) = outcome.expand_script {
            self.state.open_script_editor(idx, self.is_exit_pipeline, lang);
        }
        if outcome.expand_macro {
            self.state.open_macro_editor(idx, self.is_exit_pipeline);
        }
        if let Some(act_idx) = outcome.expand_overlay_action {
            self.state.open_overlay_studio_for_action(act_idx, self.is_exit_pipeline);
        } else if let Some(ov_id) = outcome.expand_overlay {
            self.state.open_overlay_studio(&ov_id);
        }
        if outcome.switch_to_hunter {
            self.switch_to_hunter = Some(idx);
        }
        if let Some(target) = outcome.open_catalog_nested {
            self.state.open_action_catalog_nested(self.is_exit_pipeline, target);
        }
        if let Some(target) = outcome.open_condition_catalog {
            self.state.open_condition_catalog_with_target(target);
        }
        if let Some(target) = outcome.pick_target {
            self.state.open_process_picker(target);
        }
        for id in outcome.tested_overlay_ids {
            self.state.active_test_overlays.insert(id);
        }
    }

    fn before_mutation(&mut self) {
        self.state.push_undo_snapshot();
    }

    fn render_add_button(&mut self, ui: &mut Ui) {
        let loc = self.state.loc();
        render_add_action_button(ui, &mut *self.state, loc, self.is_exit_pipeline, self.t);
    }

    fn render_infinite_warning(&mut self, ui: &mut Ui) {
        render_infinite_macro_warning(ui, &*self.state, self.t);
    }
}

/// Everything a pipeline card can ask the surrounding editor to do, shared by the
/// top-level pipelines and the nested If/Else branches.
pub(crate) struct PipelineCardOutcome {
    pub remove_clicked: bool,
    pub clone_clicked: bool,
    pub expand_script: Option<ScriptLanguage>,
    pub expand_macro: bool,
    pub expand_overlay: Option<String>,
    pub expand_overlay_action: Option<usize>,
    pub drag_started: bool,
    pub switch_to_hunter: bool,
    pub open_catalog_nested: Option<crate::gui::state::IfElseCatalogTarget>,
    pub open_condition_catalog: Option<crate::gui::state::ConditionCatalogTarget>,
    pub pick_target: Option<crate::gui::state::ProcessPickerTarget>,
    pub card_rect: Rect,
    pub tested_overlay_ids: Vec<String>,
}

impl Default for PipelineCardOutcome {
    fn default() -> Self {
        Self {
            remove_clicked: false,
            clone_clicked: false,
            expand_script: None,
            expand_macro: false,
            expand_overlay: None,
            expand_overlay_action: None,
            drag_started: false,
            switch_to_hunter: false,
            open_catalog_nested: None,
            open_condition_catalog: None,
            pick_target: None,
            card_rect: Rect::ZERO,
            tested_overlay_ids: Vec::new(),
        }
    }
}

/// Host-specific behaviour of [`render_action_pipeline`]: how one card is drawn and what
/// its outcome means for the editor around it.
pub(crate) trait PipelineHost {
    fn loc(&self) -> &'static crate::i18n::Strings;

    fn render_card(
        &mut self,
        ui: &mut Ui,
        action: &mut Action,
        idx: usize,
        is_ghost: bool,
        is_dimmed: bool,
    ) -> PipelineCardOutcome;

    fn apply(&mut self, outcome: PipelineCardOutcome, idx: usize);

    /// Called before a reorder/clone/remove so the host can take an undo snapshot.
    fn before_mutation(&mut self);

    fn render_add_button(&mut self, ui: &mut Ui);

    fn render_infinite_warning(&mut self, ui: &mut Ui);
}

#[derive(Clone, Copy)]
pub(crate) enum BranchPickerKind {
    Then,
    ElseIf(usize),
    Else,
}

impl BranchPickerKind {
    fn make_target(self, parent_idx: usize, sub_idx: usize, is_exit: bool) -> crate::gui::state::ProcessPickerTarget {
        match self {
            BranchPickerKind::Then => crate::gui::state::ProcessPickerTarget::IfElseThen { parent_idx, sub_idx, is_exit },
            BranchPickerKind::ElseIf(branch_idx) => crate::gui::state::ProcessPickerTarget::IfElseElseIf { parent_idx, branch_idx, sub_idx, is_exit },
            BranchPickerKind::Else => crate::gui::state::ProcessPickerTarget::IfElseElse { parent_idx, sub_idx, is_exit },
        }
    }
}

/// Widget-id salt for a nested If/Else branch. Built from an ASCII branch key/index only, so the
/// ids stay stable when the UI language (and therefore the visible branch label) changes.
#[derive(Clone, Copy)]
pub(crate) enum IfElseBranchSalt {
    Then,
    Else,
    ElseIf(usize),
}

pub(crate) fn ifelse_branch_salt(branch: IfElseBranchSalt) -> String {
    match branch {
        IfElseBranchSalt::Then => "then".to_string(),
        IfElseBranchSalt::Else => "else".to_string(),
        IfElseBranchSalt::ElseIf(branch_idx) => format!("elif_{branch_idx}"),
    }
}

/// Host for a nested If/Else branch: cards are the compact sub-action cards and every
/// outcome is folded back into the parent editor's response.
pub(crate) struct NestedPipelineHost<'a> {
    pub(crate) state: &'a GuiState,
    pub(crate) resp: &'a mut ActionFieldsResponse,
    pub(crate) t: &'a Tokens,
    pub(crate) parent_idx: usize,
    pub(crate) branch_salt: &'a str,
    pub(crate) branch_label: &'a str,
    pub(crate) picker_kind: BranchPickerKind,
    pub(crate) is_exit_pipeline: bool,
    pub(crate) depth: usize,
    pub(crate) add_btn_label: &'a str,
    pub(crate) catalog_target: Option<crate::gui::state::IfElseCatalogTarget>,
}

impl PipelineHost for NestedPipelineHost<'_> {
    fn loc(&self) -> &'static crate::i18n::Strings {
        self.state.loc()
    }

    fn render_card(
        &mut self,
        ui: &mut Ui,
        action: &mut Action,
        idx: usize,
        is_ghost: bool,
        is_dimmed: bool,
    ) -> PipelineCardOutcome {
        let default_picker = self.picker_kind.make_target(self.parent_idx, idx, self.is_exit_pipeline);
        render_subaction_card(
            ui,
            self.state,
            action,
            self.parent_idx,
            idx,
            self.branch_salt,
            self.branch_label,
            default_picker,
            self.is_exit_pipeline,
            is_ghost,
            is_dimmed,
            true,
            self.t,
            self.depth,
        )
    }

    fn apply(&mut self, outcome: PipelineCardOutcome, _idx: usize) {
        if let Some(target) = outcome.pick_target {
            self.resp.pick_target = Some(target);
        }
        if let Some(lang) = outcome.expand_script {
            self.resp.expand_script = Some(lang);
        }
        if outcome.expand_macro {
            self.resp.expand_macro = true;
        }
        if let Some(ov_id) = outcome.expand_overlay {
            self.resp.expand_overlay = Some(ov_id);
        }
        if let Some(target) = outcome.open_catalog_nested {
            self.resp.open_catalog_nested = Some(target);
        }
        self.resp.tested_overlay_ids.extend(outcome.tested_overlay_ids);
    }

    fn before_mutation(&mut self) {
        // The parent editor records one undo snapshot per frame, so nested pipelines do not
        // push their own snapshot here.
    }

    fn render_add_button(&mut self, ui: &mut Ui) {
        if render_full_width_action_button(ui, self.add_btn_label, "", self.t) {
            if let Some(target) = self.catalog_target {
                self.resp.open_catalog_nested = Some(target);
            }
        }
    }

    fn render_infinite_warning(&mut self, ui: &mut Ui) {
        render_infinite_macro_warning(ui, self.state, self.t);
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_subactions_branch_pipeline(
    ui: &mut Ui,
    state: &GuiState,
    sub_actions: &mut Vec<Action>,
    parent_idx: usize,
    branch_key: &str,
    branch_label: &str,
    is_exit_pipeline: bool,
    picker_kind: BranchPickerKind,
    add_btn_label: &str,
    t: &Tokens,
    depth: usize,
    resp: &mut ActionFieldsResponse,
    catalog_target: Option<crate::gui::state::IfElseCatalogTarget>,
) {
    let pipeline_id = format!(
        "sub_{}_{}_{}",
        parent_idx,
        branch_key,
        if is_exit_pipeline { "exit" } else { "main" }
    );
    let mut host = NestedPipelineHost {
        state,
        resp,
        t,
        parent_idx,
        branch_salt: branch_key,
        branch_label,
        picker_kind,
        is_exit_pipeline,
        depth,
        add_btn_label,
        catalog_target,
    };
    render_action_pipeline(ui, sub_actions, &pipeline_id, branch_label, t, &mut host);
}


#[cfg(test)]
mod tests {
    use super::*;

    /// The branch labels are localized, while the egui ids are salted only from ASCII branch
    /// keys — switching the UI language must not re-key the nested If/Else cards.
    #[test]
    fn test_ifelse_branch_labels_localize_while_ids_stay_constant() {
        let en = crate::i18n::get(crate::i18n::Language::English);
        let ru = crate::i18n::get(crate::i18n::Language::Russian);

        assert_ne!(en.inspector_ifelse_then_badge, ru.inspector_ifelse_then_badge);
        assert_ne!(en.inspector_ifelse_else_badge, ru.inspector_ifelse_else_badge);
        let elif_en = en.inspector_ifelse_elif_badge.replace("{:02}", "01");
        let elif_ru = ru.inspector_ifelse_elif_badge.replace("{:02}", "01");
        assert_ne!(elif_en, elif_ru, "ELIF label must follow the UI language");

        // The id salts are ASCII branch keys/indices, so they are identical for every language.
        assert_eq!(ifelse_branch_salt(IfElseBranchSalt::Then), "then");
        assert_eq!(ifelse_branch_salt(IfElseBranchSalt::Else), "else");
        assert_eq!(ifelse_branch_salt(IfElseBranchSalt::ElseIf(2)), "elif_2");
        for salt in [
            ifelse_branch_salt(IfElseBranchSalt::Then),
            ifelse_branch_salt(IfElseBranchSalt::Else),
            ifelse_branch_salt(IfElseBranchSalt::ElseIf(0)),
        ] {
            assert!(salt.is_ascii(), "branch id salt must stay ASCII: {salt}");
            assert!(!salt.contains("ТОГДА") && !salt.contains("ИНАЧЕ"));
        }
    }
}
