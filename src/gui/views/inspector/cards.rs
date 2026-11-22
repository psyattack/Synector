//! Top-level and nested action cards.

use super::*;

pub(crate) fn render_infinite_macro_warning(ui: &mut Ui, state: &GuiState, t: &Tokens) {
    let loc = state.loc();
    ui.add_space(2.0);
    egui::Frame::new()
        .fill(t.warn.gamma_multiply(0.12))
        .stroke(Stroke::new(1.0, t.warn.gamma_multiply(0.4)))
        .corner_radius(CornerRadius::ZERO)
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(egui_phosphor::regular::WARNING)
                        .font(egui::FontId::new(13.0, theme::phosphor()))
                        .color(t.warn),
                );
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(loc.inspector_infinite_macro_warning)
                        .family(theme::mono())
                        .size(11.0)
                        .color(t.warn),
                );
            });
        });
    ui.add_space(2.0);
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_action_card(
    ui: &mut Ui,
    state: &GuiState,
    action: &mut Action,
    idx: usize,
    t: &Tokens,
    is_exit_pipeline: bool,
    is_being_dragged: bool,
    is_executing: bool,
    is_dimmed: bool,
) -> PipelineCardOutcome {
    let loc = state.loc();
    let mut remove_clicked = false;
    let mut clone_clicked = false;
    let mut drag_started = false;
    let mut step_tested_overlays = Vec::new();
    let mut fields_resp = ActionFieldsResponse::default();

    let ghost_id = ui.scope_id().with(("card_ghost", is_exit_pipeline, idx));
    let ghost_anim = ui.ctx().animate_bool_with_time_and_easing(ghost_id, is_being_dragged, 0.15, easing::cubic_out);

    let (fill, stroke) = if is_dimmed {
        (t.sunken.gamma_multiply(0.4), Stroke::new(1.0, t.muted.gamma_multiply(0.2)))
    } else if is_executing {
        (t.sunken, Stroke::new(1.5, t.accent_ink))
    } else if ghost_anim > 0.001 {
        (
            t.sunken,
            Stroke::new(1.0, t.accent_ink.gamma_multiply(0.45 * ghost_anim)),
        )
    } else {
        (t.sunken, t.hairline())
    };

    let card_response = egui::Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::ZERO)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_max_width(ui.available_width());
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                ui.set_max_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 8.0;

                // 1. CARD TOP STRIP: Step number + Type badge on left; Close button exclusively on far right
                ui.horizontal(|ui| {
                    ui.set_width(ui.available_width());

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::glyph_button(ui, Glyph::Close, loc.inspector_tip_remove_step).clicked() {
                            remove_clicked = true;
                        }

                        if widgets::glyph_button(ui, Glyph::Clone, loc.inspector_tip_clone_step).clicked() {
                            clone_clicked = true;
                        }

                        if widgets::glyph_button(ui, Glyph::Play, loc.inspector_btn_test_step).clicked() {
                            if let Action::Overlay { overlay_id, custom_overlay, .. } = action {
                                step_tested_overlays.push(overlay_id.clone());
                                if let Some(cust) = custom_overlay {
                                    step_tested_overlays.push(cust.id.clone());
                                }
                            }
                            let key = if is_exit_pipeline { idx + 1000 } else { idx };
                            if let Action::Macro { events, speed, .. } = action {
                                let (ok, msg) = if events.is_empty() {
                                    (false, loc.inspector_macro_test_empty.to_string())
                                } else {
                                    crate::gui::macro_recorder::run_macro_test_safe(events.clone(), *speed);
                                    (true, loc.inspector_macro_testing_actions.replace("{}", &events.len().to_string()))
                                };
                                ui.data_mut(|d| d.insert_temp(Id::unique(("test_step_res", key)), (ok, msg, Instant::now())));
                            } else {
                                // The round-trip runs on the IPC worker; the outcome badge appears
                                // when the result is applied on a later frame.
                                let mut ctx = HashMap::new();
                                ctx.insert("ProcessName".to_string(), "test.exe".to_string());
                                ctx.insert("Timestamp".to_string(), chrono::Local::now().to_rfc3339());
                                state.test_step(action, ctx, Id::unique(("test_step_res", key)));
                            }
                        }

                        let test_key = if is_exit_pipeline { idx + 1000 } else { idx };
                        if let Some((ok, msg, at)) = ui.data(|d| d.get_temp::<(bool, String, Instant)>(Id::unique(("test_step_res", test_key)))) {
                            if at.elapsed().as_secs() < 8 {
                                let badge_color = if ok { t.good } else { t.bad };
                                widgets::badge(ui, if ok { loc.inspector_test_step_pass_badge } else { loc.inspector_test_step_fail_badge }, badge_color).on_hover_text(msg);
                            }
                        }

                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            // Drag grip handle on far left
                            let grip_resp = widgets::drag_handle(ui, loc.inspector_tip_drag_step);
                            if grip_resp.drag_started() {
                                drag_started = true;
                            }

                            let prefix = if is_exit_pipeline { loc.inspector_exit_prefix } else { loc.inspector_step_prefix };
                            widgets::hud(ui, &format!("{} {:02}", prefix, idx + 1), if is_dimmed { t.muted } else { t.accent_ink });

                            let type_badge = action_type_badge(action, loc);
                            if is_dimmed {
                                widgets::badge(ui, loc.inspector_badge_unreachable, t.warn.gamma_multiply(0.8))
                                    .on_hover_text(loc.inspector_badge_unreachable_tip);
                            } else if is_executing {
                                widgets::badge(ui, loc.inspector_badge_executing_now, t.accent_ink);
                            } else if is_being_dragged {
                                widgets::badge(ui, loc.inspector_badge_moving_step, t.accent_ink);
                            } else {
                                widgets::badge(ui, type_badge, t.muted);
                            }

                            // Middle blank space on top strip is also draggable
                            let empty_w = (ui.available_width() - 4.0).max(0.0);
                            if empty_w > 0.0 {
                                let (_, strip_resp) = ui.allocate_exact_size(vec2(empty_w, theme::CONTROL_H), Sense::click_and_drag());
                                if strip_resp.drag_started() {
                                    drag_started = true;
                                }
                                if strip_resp.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                                }
                            }
                        });
                    });
                });

                widgets::hrule(ui);

                let default_picker = if is_exit_pipeline {
                    crate::gui::state::ProcessPickerTarget::ExitAction(idx)
                } else {
                    crate::gui::state::ProcessPickerTarget::Action(idx)
                };

                // 2. CARD BODY: Full-width stacked parameter fields
                fields_resp = ui.add_enabled_ui(!is_being_dragged && !is_dimmed, |ui| {
                    render_action_parameter_fields(
                        ui,
                        state,
                        action,
                        idx,
                        is_exit_pipeline,
                        default_picker,
                        t,
                        0,
                    )
                }).inner;
            });
        });

    let mut tested_overlay_ids = fields_resp.tested_overlay_ids;
    tested_overlay_ids.extend(step_tested_overlays);

    PipelineCardOutcome {
        remove_clicked,
        clone_clicked,
        expand_script: fields_resp.expand_script,
        expand_macro: fields_resp.expand_macro,
        expand_overlay: fields_resp.expand_overlay,
        expand_overlay_action: fields_resp.expand_overlay_action,
        drag_started,
        switch_to_hunter: fields_resp.switch_to_hunter,
        open_catalog_nested: fields_resp.open_catalog_nested,
        open_condition_catalog: fields_resp.open_condition_catalog,
        pick_target: fields_resp.pick_target,
        card_rect: card_response.response.rect,
        tested_overlay_ids,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_subaction_card(
    ui: &mut Ui,
    state: &GuiState,
    sub_act: &mut Action,
    parent_idx: usize,
    sub_idx: usize,
    branch_salt: &str,
    branch_label: &str,
    default_picker: crate::gui::state::ProcessPickerTarget,
    is_exit_pipeline: bool,
    is_being_dragged: bool,
    is_dimmed: bool,
    can_remove: bool,
    t: &Tokens,
    depth: usize,
) -> PipelineCardOutcome {
    let loc = state.loc();
    let mut remove_clicked = false;
    let mut clone_clicked = false;
    let mut drag_started = false;
    let mut step_tested_overlays = Vec::new();
    let type_badge = action_type_badge(sub_act, loc);

    let mut fields_resp = ActionFieldsResponse::default();

    let ghost_id = ui.scope_id().with(("sub_card_ghost", parent_idx, branch_salt, sub_idx));
    let ghost_anim = ui.ctx().animate_bool_with_time_and_easing(ghost_id, is_being_dragged, 0.15, easing::cubic_out);

    let is_overlay_active = match sub_act {
        Action::Overlay { overlay_id, custom_overlay, .. } => {
            let ov_id = custom_overlay.as_ref().map(|o| o.id.as_str()).unwrap_or(overlay_id.as_str());
            state.daemon_state.as_ref().is_some_and(|d| d.active_overlay_ids.contains(ov_id))
                || state.active_test_overlays.contains(ov_id)
        }
        _ => false,
    };

    let (fill, stroke) = if is_dimmed {
        (t.sunken.gamma_multiply(0.4), Stroke::new(1.0, t.muted.gamma_multiply(0.2)))
    } else if is_overlay_active {
        (t.bg, Stroke::new(1.5, t.accent_ink))
    } else if ghost_anim > 0.001 {
        (
            t.bg,
            Stroke::new(1.0, t.accent_ink.gamma_multiply(0.45 * ghost_anim)),
        )
    } else if depth > 0 {
        (t.bg, Stroke::new(1.0, t.line_strong.gamma_multiply(0.45)))
    } else {
        (t.sunken, t.hairline())
    };

    let card_response = egui::Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::ZERO)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 8.0;

                // Sub-action top bar: Step # + Type badge on left; Test / Clone / Remove on right
                ui.horizontal(|ui| {
                    ui.set_width(ui.available_width());

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if can_remove && widgets::glyph_button(ui, Glyph::Close, loc.inspector_tip_remove_step).clicked() {
                            remove_clicked = true;
                        }
                        if widgets::glyph_button(ui, Glyph::Clone, loc.inspector_tip_clone_step).clicked() {
                            clone_clicked = true;
                        }
                        if widgets::glyph_button(ui, Glyph::Play, loc.inspector_btn_test_step).clicked() {
                            if let Action::Overlay { overlay_id, custom_overlay, .. } = sub_act {
                                step_tested_overlays.push(overlay_id.clone());
                                if let Some(cust) = custom_overlay {
                                    step_tested_overlays.push(cust.id.clone());
                                }
                            }
                            let sub_test_key = ui.scope_id().with(("sub_test_res", parent_idx, branch_salt, sub_idx));
                            if let Action::Macro { events, speed, .. } = sub_act {
                                let (ok, msg) = if events.is_empty() {
                                    (false, loc.inspector_macro_test_empty.to_string())
                                } else {
                                    crate::gui::macro_recorder::run_macro_test_safe(events.clone(), *speed);
                                    (true, loc.inspector_macro_testing_actions.replace("{}", &events.len().to_string()))
                                };
                                ui.data_mut(|d| d.insert_temp(sub_test_key, (ok, msg, std::time::Instant::now())));
                            } else {
                                // The round-trip runs on the IPC worker; the outcome badge appears
                                // when the result is applied on a later frame.
                                let mut ctx = std::collections::HashMap::new();
                                ctx.insert("ProcessName".to_string(), "test.exe".to_string());
                                ctx.insert("Timestamp".to_string(), chrono::Local::now().to_rfc3339());
                                state.test_step(sub_act, ctx, sub_test_key);
                            }
                        }

                        let sub_test_key = ui.scope_id().with(("sub_test_res", parent_idx, branch_salt, sub_idx));
                        if let Some((ok, msg, at)) = ui.data(|d| d.get_temp::<(bool, String, std::time::Instant)>(sub_test_key)) {
                            if at.elapsed().as_secs() < 8 {
                                let badge_color = if ok { t.good } else { t.bad };
                                widgets::badge(ui, if ok { loc.inspector_test_step_pass_badge } else { loc.inspector_test_step_fail_badge }, badge_color).on_hover_text(msg);
                            }
                        }

                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            // Drag grip handle on far left
                            let grip_resp = widgets::drag_handle(ui, loc.inspector_tip_drag_step);
                            if grip_resp.drag_started() {
                                drag_started = true;
                            }

                            widgets::hud(ui, &format!("{} {:02}", branch_label, sub_idx + 1), if is_dimmed { t.muted } else { t.accent_ink });

                            if is_dimmed {
                                widgets::badge(ui, loc.inspector_badge_unreachable, t.warn.gamma_multiply(0.8))
                                    .on_hover_text(loc.inspector_badge_unreachable_tip);
                            } else if is_being_dragged {
                                widgets::badge(ui, loc.inspector_badge_moving_step, t.accent_ink);
                            } else if is_overlay_active {
                                widgets::badge(ui, loc.inspector_badge_executing_now, t.accent_ink);
                            } else {
                                widgets::badge(ui, type_badge, t.muted);
                            }

                            // Middle blank space on top strip is also draggable
                            let empty_w = (ui.available_width() - 4.0).max(0.0);
                            if empty_w > 0.0 {
                                let (_, strip_resp) = ui.allocate_exact_size(vec2(empty_w, theme::CONTROL_H), Sense::click_and_drag());
                                if strip_resp.drag_started() {
                                    drag_started = true;
                                }
                                if strip_resp.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                                }
                            }
                        });
                    });
                });

                widgets::hrule(ui);

                // Sub-action full parameter fields!
                fields_resp = ui.add_enabled_ui(!is_being_dragged && !is_dimmed, |ui| {
                    render_action_parameter_fields(
                        ui,
                        state,
                        sub_act,
                        parent_idx * 100 + sub_idx + 500,
                        is_exit_pipeline,
                        default_picker,
                        t,
                        depth,
                    )
                }).inner;
            });
        });

    let mut tested_overlay_ids = fields_resp.tested_overlay_ids;
    tested_overlay_ids.extend(step_tested_overlays);

    PipelineCardOutcome {
        remove_clicked,
        clone_clicked,
        expand_script: fields_resp.expand_script,
        expand_macro: fields_resp.expand_macro,
        expand_overlay: fields_resp.expand_overlay,
        pick_target: fields_resp.pick_target,
        drag_started,
        card_rect: card_response.response.rect,
        tested_overlay_ids,
        ..Default::default()
    }
}
