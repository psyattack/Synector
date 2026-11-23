//! §03 pre-condition cards, operators and the single-condition editor.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConditionKind {
    Process,
    Window,
    Time,
    Event,
    Resource,
    FileWatcher,
    Rule,
}

impl ConditionKind {
    pub(crate) fn from_condition(c: &RuleCondition) -> Self {
        match c {
            RuleCondition::ProcessRunning { .. } => ConditionKind::Process,
            RuleCondition::WindowFocused { .. } => ConditionKind::Window,
            RuleCondition::TimeRange { .. } => ConditionKind::Time,
            RuleCondition::Event { .. } => ConditionKind::Event,
            RuleCondition::ResourceThreshold { .. } => ConditionKind::Resource,
            RuleCondition::FileWatcher { .. } => ConditionKind::FileWatcher,
            RuleCondition::RuleRunning { .. } => ConditionKind::Rule,
        }
    }

    pub(crate) fn label(&self, loc: &crate::i18n::Strings) -> &'static str {
        match self {
            ConditionKind::Process => loc.inspector_cond_type_process,
            ConditionKind::Window => loc.inspector_cond_type_window,
            ConditionKind::Time => loc.inspector_cond_type_time,
            ConditionKind::Event => loc.inspector_cond_type_event,
            ConditionKind::Resource => loc.inspector_cond_type_resource,
            ConditionKind::FileWatcher => loc.inspector_cond_type_file,
            ConditionKind::Rule => loc.inspector_cond_type_rule,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_condition_card(
    ui: &mut Ui,
    idx: usize,
    item: &mut RuleConditionItem,
    loc: &crate::i18n::Strings,
    t: &Tokens,
    is_nested: bool,
    can_remove: bool,
    on_pick_process: impl FnMut(),
    state: &GuiState,
) -> (bool, bool) {
    let mut remove_clicked = false;
    let mut change_clicked = false;
    let kind = ConditionKind::from_condition(&item.condition);

    let (fill, stroke) = if is_nested {
        (t.bg, Stroke::new(1.0, t.line_strong.gamma_multiply(0.45)))
    } else {
        (t.sunken, t.hairline())
    };

    egui::Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(t.card_corner())
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 8.0;

                // 1. Top Strip: Step index + Badge on left; Close button and Change button on right
                ui.horizontal(|ui| {
                    let pfx = format!("{} {:02}", loc.inspector_conditions_badge, idx + 1);
                    widgets::hud(ui, &pfx, t.accent_ink);
                    widgets::badge(ui, kind.label(loc), t.muted);

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if can_remove && widgets::glyph_button(ui, Glyph::Close, loc.inspector_cond_remove_tip).clicked() {
                            remove_clicked = true;
                        }
                        if widgets::button(ui, loc.inspector_btn_change_condition, Kind::Secondary, 0.0).clicked() {
                            change_clicked = true;
                        }
                    });
                });

                widgets::hrule(ui);

                // 2. Condition Parameter Fields
                render_single_condition_editor(ui, loc, &mut item.condition, idx, on_pick_process, t, state);
            });
        });

    (remove_clicked, change_clicked)
}

pub(crate) fn render_condition_operator_connector(
    ui: &mut Ui,
    op: &mut ConditionOperator,
    loc: &crate::i18n::Strings,
    t: &Tokens,
) {
    ui.add_space(2.0);
    let avail_w = ui.available_width();
    let op_options = [
        (ConditionOperator::And, Cell::new(loc.inspector_cond_op_and, loc.inspector_cond_op_and_tip)),
        (ConditionOperator::Or, Cell::new(loc.inspector_cond_op_or, loc.inspector_cond_op_or_tip)),
    ];

    let mut cells_w = 0.0;
    for (_, c) in &op_options {
        let galley = widgets::caps(ui, c.label, theme::HUD_SIZE, theme::mono_bold(), theme::BUTTON_SPACING, t.ink);
        cells_w += (galley.size().x + 24.0).max(theme::control_height(ui));
    }

    let side_w = ((avail_w - cells_w - 12.0) * 0.5).max(12.0);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;

        let (rect_left, _) = ui.allocate_exact_size(vec2(side_w, theme::control_height(ui)), Sense::hover());
        ui.painter().line_segment(
            [pos2(rect_left.left(), rect_left.center().y), pos2(rect_left.right(), rect_left.center().y)],
            Stroke::new(1.0, t.muted.gamma_multiply(0.25)),
        );

        widgets::cells(ui, op, &op_options);

        let rem_w = (ui.available_width() - 2.0).max(0.0);
        if rem_w > 0.0 {
            let (rect_right, _) = ui.allocate_exact_size(vec2(rem_w, theme::control_height(ui)), Sense::hover());
            ui.painter().line_segment(
                [pos2(rect_right.left(), rect_right.center().y), pos2(rect_right.right(), rect_right.center().y)],
                Stroke::new(1.0, t.muted.gamma_multiply(0.25)),
            );
        }
    });
    ui.add_space(2.0);
}

pub(crate) fn render_add_condition_button(
    ui: &mut Ui,
    loc: &crate::i18n::Strings,
    t: &Tokens,
) -> bool {
    render_full_width_action_button(ui, loc.inspector_conditions_btn_add, "", t)
}

pub(crate) fn render_single_condition_editor(
    ui: &mut Ui,
    loc: &crate::i18n::Strings,
    cond: &mut RuleCondition,
    idx: usize,
    mut on_pick_process: impl FnMut(),
    t: &Tokens,
    state: &GuiState,
) {
    match cond {
        RuleCondition::ProcessRunning { process_name, match_mode, must_be_running } => {
            widgets::hud(ui, loc.inspector_field_target_process, t.muted);
            if render_process_name_row(ui, loc, process_name, loc.inspector_hint_cond_process) {
                on_pick_process();
            }

            widgets::hud(ui, loc.inspector_field_match_mode, t.muted);
            let match_options = [
                (ProcessMatchMode::Exact, Cell::new(loc.inspector_match_tab_exact, "")),
                (ProcessMatchMode::Wildcard, Cell::new(loc.inspector_match_tab_wildcard, "")),
                (ProcessMatchMode::Regex, Cell::new(loc.inspector_match_tab_regex, "")),
            ];
            widgets::cells(ui, match_mode, &match_options);

            ui.add_space(2.0);
            widgets::hud(ui, loc.inspector_cond_state_label, t.muted);
            let state_options = [
                (true, Cell::new(loc.inspector_cond_state_running, "")),
                (false, Cell::new(loc.inspector_cond_state_not_running, "")),
            ];
            widgets::cells(ui, must_be_running, &state_options);
        }
        RuleCondition::WindowFocused { process_name, match_mode, must_be_focused } => {
            widgets::hud(ui, loc.inspector_field_target_process, t.muted);
            if render_process_name_row(ui, loc, process_name, loc.inspector_hint_cond_process) {
                on_pick_process();
            }

            widgets::hud(ui, loc.inspector_field_match_mode, t.muted);
            let match_options = [
                (ProcessMatchMode::Exact, Cell::new(loc.inspector_match_tab_exact, "")),
                (ProcessMatchMode::Wildcard, Cell::new(loc.inspector_match_tab_wildcard, "")),
                (ProcessMatchMode::Regex, Cell::new(loc.inspector_match_tab_regex, "")),
            ];
            widgets::cells(ui, match_mode, &match_options);

            ui.add_space(2.0);
            widgets::hud(ui, loc.inspector_cond_focus_label, t.muted);
            let state_options = [
                (true, Cell::new(loc.inspector_cond_state_focused, "")),
                (false, Cell::new(loc.inspector_cond_state_not_focused, "")),
            ];
            widgets::cells(ui, must_be_focused, &state_options);
        }
        RuleCondition::TimeRange { start_time, end_time, days_of_week } => {
            widgets::hud(ui, loc.inspector_cond_time_range_label, t.muted);
            ui.horizontal(|ui| {
                widgets::hud(ui, loc.inspector_cond_time_from, t.muted);
                ui.add(
                    egui::TextEdit::singleline(start_time)
                        .hint_text(loc.inspector_hint_cond_time_from)
                        .desired_width(65.0)
                        .min_size(vec2(0.0, theme::control_height(ui)))
                        .align(egui::Align2::LEFT_CENTER)
                        .margin(vec2(8.0, 6.0)),
                );
                widgets::hud(ui, loc.inspector_cond_time_to, t.muted);
                ui.add(
                    egui::TextEdit::singleline(end_time)
                        .hint_text(loc.inspector_hint_cond_time_to)
                        .desired_width(65.0)
                        .min_size(vec2(0.0, theme::control_height(ui)))
                        .align(egui::Align2::LEFT_CENTER)
                        .margin(vec2(8.0, 6.0)),
                );
            });

            widgets::hud(ui, loc.inspector_active_days_of_week, t.muted);
            render_days_of_week_selector(ui, loc, days_of_week);
        }
        RuleCondition::Event { event_type, idle_duration_sec } => {
            render_system_event_pair_toggle(ui, event_type, loc, t);
            let meta = system_event_meta(*event_type, loc, t);
            render_trigger_info_frame(ui, meta.glyph, meta.description, meta.tint, t);
            if matches!(event_type, crate::models::SystemEventType::UserIdle | crate::models::SystemEventType::UserActive) {
                ui.add_space(6.0);
                widgets::hud(ui, loc.inspector_field_idle_duration, t.muted);
                // `idle_duration_sec` is a `u32` in the model; 0 means "already idle", which the
                // engine compares directly, so no arbitrary 5 s floor is imposed here.
                widgets::duration_secs_u32(ui, loc, ("cond_idle_duration", idx), idle_duration_sec, 0.0..=f64::from(u32::MAX));
            }
        }
        RuleCondition::ResourceThreshold { process_name, cpu_percent, ram_mb } => {
            widgets::hud(ui, loc.inspector_field_target_process, t.muted);
            if render_process_name_row(ui, loc, process_name, loc.inspector_hint_process_executable) {
                on_pick_process();
            }

            ui.add_space(6.0);
            widgets::checkbox_row(ui, |ui| {
                let mut has_cpu = cpu_percent.is_some();
                if ui.checkbox(&mut has_cpu, loc.inspector_cond_res_cpu).changed() {
                    *cpu_percent = if has_cpu { Some(80.0) } else { None };
                }
                if let Some(cpu) = cpu_percent {
                    ui.add(egui::DragValue::new(cpu).range(1.0..=100.0).suffix("%"));
                }

                ui.add_space(10.0);
                let mut has_mem = ram_mb.is_some();
                if ui.checkbox(&mut has_mem, loc.inspector_cond_res_ram).changed() {
                    *ram_mb = if has_mem { Some(1024) } else { None };
                }
                if let Some(mem) = ram_mb {
                    ui.add(egui::DragValue::new(mem).range(10..=65536).suffix(" MB"));
                }
            });
        }
        RuleCondition::FileWatcher { path, filter_extension, must_exist } => {
            widgets::hud(ui, loc.inspector_cond_file_path_label, t.muted);
            ui.horizontal(|ui| {
                ui.set_width(ui.available_width());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::button(ui, loc.inspector_cond_file_btn_folder, Kind::Secondary, 0.0).clicked() {
                        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                            *path = folder.to_string_lossy().to_string();
                        }
                    }
                    if widgets::button(ui, loc.inspector_cond_file_btn_file, Kind::Secondary, 0.0).clicked() {
                        if let Some(file) = rfd::FileDialog::new().pick_file() {
                            *path = file.to_string_lossy().to_string();
                        }
                    }
                    ui.add(
                        egui::TextEdit::singleline(path)
                            .hint_text(loc.inspector_hint_cond_file_path)
                            .desired_width(ui.available_width())
                            .min_size(vec2(0.0, theme::control_height(ui)))
                            .align(egui::Align2::LEFT_CENTER)
                            .margin(vec2(10.0, 8.0)),
                    );
                });
            });

            ui.add_space(4.0);
            widgets::hud(ui, loc.inspector_cond_file_ext_filter, t.muted);
            let mut ext_buf = filter_extension.clone().unwrap_or_default();
            if ui.add(
                egui::TextEdit::singleline(&mut ext_buf)
                    .hint_text(loc.inspector_cond_file_ext_hint)
                    .desired_width(ui.available_width())
                    .min_size(vec2(0.0, theme::control_height(ui)))
                    .align(egui::Align2::LEFT_CENTER)
                    .margin(vec2(10.0, 8.0)),
            ).changed() {
                let trimmed = ext_buf.trim().to_string();
                *filter_extension = if trimmed.is_empty() { None } else { Some(trimmed) };
            }

            ui.add_space(4.0);
            widgets::hud(ui, loc.inspector_cond_file_state_label, t.muted);
            let file_options = [
                (true, Cell::new(loc.inspector_cond_file_exists, "")),
                (false, Cell::new(loc.inspector_cond_file_not_exists, "")),
            ];
            widgets::cells(ui, must_exist, &file_options);
        }
        RuleCondition::RuleRunning { rule_id, must_be_running } => {
            let choices = rule_choices(state);
            render_rule_choice_combo(ui, &choices, rule_id, ("cond_rule_target", idx), loc, t);

            ui.add_space(2.0);
            widgets::hud(ui, loc.inspector_cond_rule_state_label, t.muted);
            let state_options = [
                (true, Cell::new(loc.inspector_cond_rule_running, "")),
                (false, Cell::new(loc.inspector_cond_rule_not_running, "")),
            ];
            widgets::cells(ui, must_be_running, &state_options);
        }
    }
}

pub(crate) fn render_conditions_section(
    ui: &mut Ui,
    state: &mut GuiState,
    rule: &mut Rule,
    t: &Tokens,
) {
    let loc = state.loc();
    let mut remove_idx = None;
    let mut change_idx = None;
    let mut pick_cond_idx = None;

    for (idx, item) in rule.conditions.iter_mut().enumerate() {
        if idx > 0 {
            render_condition_operator_connector(ui, &mut item.operator, loc, t);
        }

        let (remove_clicked, change_clicked) = render_condition_card(
            ui,
            idx,
            item,
            loc,
            t,
            false,
            true,
            || { pick_cond_idx = Some(idx); },
            state,
        );
        if remove_clicked {
            remove_idx = Some(idx);
        }
        if change_clicked {
            change_idx = Some(idx);
        }
    }

    if let Some(idx) = remove_idx {
        state.push_undo_snapshot();
        if idx < rule.conditions.len() {
            rule.conditions.remove(idx);
        }
    }

    if let Some(idx) = change_idx {
        state.open_condition_catalog_with_target(crate::gui::state::ConditionCatalogTarget::ChangeRulePrecondition(idx));
    }

    if let Some(idx) = pick_cond_idx {
        state.open_process_picker(crate::gui::state::ProcessPickerTarget::Condition(idx));
    }

    // 44px Add Condition button at the bottom of the section
    if render_add_condition_button(ui, loc, t) {
        state.open_condition_catalog_with_target(crate::gui::state::ConditionCatalogTarget::RulePrecondition);
    }
}
