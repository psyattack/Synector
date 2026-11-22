//! Ordered §01–§05 shell of the rule editor and the §01 basic-info block.

use super::*;

pub(crate) fn quota_period_label(period: Option<QuotaPeriod>, loc: &crate::i18n::Strings) -> String {
    match period {
        None => loc.inspector_quota_period_total.to_string(),
        Some(QuotaPeriod::Minute) => loc.inspector_quota_period_minute.to_string(),
        Some(QuotaPeriod::Hour) => loc.inspector_quota_period_hour.to_string(),
        Some(QuotaPeriod::Day) => loc.inspector_quota_period_day.to_string(),
        Some(QuotaPeriod::Week) => loc.inspector_quota_period_week.to_string(),
        Some(QuotaPeriod::Month) => loc.inspector_quota_period_month.to_string(),
        Some(QuotaPeriod::Custom { .. }) => loc.inspector_quota_period_custom.to_string(),
    }
}

pub(crate) fn quota_unit_label(unit: QuotaUnit, loc: &crate::i18n::Strings) -> &'static str {
    match unit {
        QuotaUnit::Minutes => loc.inspector_quota_unit_minutes,
        QuotaUnit::Hours => loc.inspector_quota_unit_hours,
        QuotaUnit::Days => loc.inspector_quota_unit_days,
        QuotaUnit::Weeks => loc.inspector_quota_unit_weeks,
        QuotaUnit::Months => loc.inspector_quota_unit_months,
    }
}

/// Left-hand form label centred against a standard-height control row.
pub(crate) fn quota_form_label(ui: &mut Ui, text: &str, t: &Tokens) -> egui::Response {
    let row_h = ui.spacing().interact_size.y;
    ui.allocate_ui_with_layout(
        vec2(58.0, row_h),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| widgets::hud(ui, text, t.muted),
    )
    .inner
}

pub(crate) fn render_rule_editor(ui: &mut Ui, state: &mut GuiState, rule: &mut Rule, t: &Tokens) {
    state.ensure_processes();

    let scrollbar_w = ui.spacing().scroll.bar_width + ui.spacing().scroll.bar_inner_margin + ui.spacing().scroll.bar_outer_margin;
    let right_m = (20.0 - scrollbar_w).max(0.0).round() as i8;

    egui::Frame::new()
        .inner_margin(egui::Margin {
            left: 20,
            right: right_m,
            top: 14,
            bottom: 20,
        })
        .show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_max_width(ui.available_width());
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            ui.set_max_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 12.0;

            // 1. Basic Info & Icon Avatar
            let loc = state.loc();
            let is_manual = rule.trigger.as_ref().is_none_or(|t| matches!(t, Trigger::Manual));

            widgets::eyebrow(ui, "01", loc.inspector_eyebrow_task_details);

            egui::Frame::new()
                .fill(t.sunken)
                .stroke(t.hairline())
                .corner_radius(CornerRadius::ZERO)
                .inner_margin(egui::Margin::symmetric(14, 12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_max_width(ui.available_width());
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 8.0;

                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;

                            // Interactive Icon Avatar Box (38x38)
                            let icon_box_size = vec2(38.0, 38.0);
                            let (icon_rect, icon_resp) = ui.allocate_exact_size(icon_box_size, Sense::click());
                            let is_hovered = icon_resp.hovered();
                            let p = ui.painter();

                            p.rect_filled(icon_rect, CornerRadius::ZERO, if is_hovered { t.sunken } else { t.bg });
                            let stroke_color = if is_hovered { t.accent_ink } else { t.line_strong };
                            p.rect_stroke(icon_rect, CornerRadius::ZERO, Stroke::new(1.0, stroke_color), egui::StrokeKind::Inside);

                            let mut rendered = false;
                            if let Some(path) = &rule.custom_icon_path {
                                if let Some(tex) = state.icon_cache.get_icon_from_path(ui.ctx(), path) {
                                    let size = tex.size_vec2();
                                    let max_dim = size.x.max(size.y).max(1.0);
                                    let scale = 26.0 / max_dim;
                                    let render_size = vec2((size.x * scale).round().max(1.0), (size.y * scale).round().max(1.0));
                                    let img_rect = Rect::from_center_size(icon_rect.center(), render_size);
                                    p.image(tex.id(), img_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
                                    rendered = true;
                                }
                            }
                            if !rendered {
                                let glyph = crate::gui::views::rule_summary::default_icon_glyph(rule);
                                p.text(icon_rect.center(), egui::Align2::CENTER_CENTER, glyph, egui::FontId::new(18.0, theme::phosphor()), if is_hovered { t.accent_ink } else { t.ink_strong });
                            }

                            let icon_tooltip = match &rule.custom_icon_path {
                                Some(p) => {
                                    let fname = std::path::Path::new(p).file_name().unwrap_or_default().to_string_lossy();
                                    format!("{}: {}\n{}", loc.inspector_icon_custom_label, fname, loc.inspector_icon_browse_tip)
                                }
                                None => format!("{}\n{}", loc.inspector_icon_default_label, loc.inspector_icon_browse_tip),
                            };
                            let icon_resp = icon_resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(icon_tooltip);

                            if icon_resp.clicked() {
                                if let Some(picked) = rfd::FileDialog::new()
                                    .set_title(loc.inspector_dialog_icon_title)
                                    .add_filter(loc.inspector_dialog_filter_image_exe, &["exe", "ico", "png", "jpg", "jpeg", "dll"])
                                    .pick_file()
                                {
                                    rule.custom_icon_path = Some(picked.to_string_lossy().to_string());
                                }
                            }
                            if icon_resp.secondary_clicked() && rule.custom_icon_path.is_some() {
                                rule.custom_icon_path = None;
                            }

                            // Name Input taking remaining available width
                            let input_w = if rule.custom_icon_path.is_some() {
                                (ui.available_width() - 36.0).max(100.0)
                            } else {
                                ui.available_width()
                            };

                            ui.add(
                                egui::TextEdit::singleline(&mut rule.name)
                                    .hint_text(loc.rules_default_new_name)
                                    .desired_width(input_w)
                                    .min_size(vec2(0.0, 38.0))
                                    .font(egui::FontId::new(14.0, theme::semibold()))
                                    .align(egui::Align2::LEFT_CENTER)
                                    .margin(vec2(10.0, 9.0)),
                            );

                            if rule.custom_icon_path.is_some()
                                && widgets::glyph_button(ui, Glyph::Reset, loc.inspector_icon_reset_tip).clicked()
                            {
                                rule.custom_icon_path = None;
                            }
                        });

                        // Tag Manager (Replaces removed description field)
                        render_inspector_tag_section(ui, state, rule, loc, t);

                        // Cooldown row
                        ui.add_space(-3.0);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().interact_size.y = 24.0;
                            let cooldown_label = widgets::hud(ui, loc.inspector_field_cooldown, t.muted);
                            // The model stores any finite non-negative cooldown (`validate_rules`),
                            // so the field must not impose a lower cap than the CLI/daemon accept.
                            widgets::duration_secs_f64(ui, loc, "rule_cooldown", &mut rule.cooldown_seconds, 0.0..=f64::MAX)
                                .labelled_by(cooldown_label.id);
                        });

                        ui.add_space(4.0);

                        // Unified compact options group
                        widgets::checkbox_group(ui, |ui| {
                            // Icon from target process (separate line below cooldown)
                            ui.checkbox(&mut rule.auto_process_icon, loc.inspector_icon_auto_process)
                                .on_hover_text(loc.inspector_icon_auto_process_tip);

                            // Execution quota: the limit and the period its counter resets on.
                            widgets::checkbox_row(ui, |ui| {
                                let mut has_quota = rule.max_executions.is_some();
                                if ui.checkbox(&mut has_quota, loc.inspector_field_max_executions).changed() {
                                    if has_quota {
                                        rule.max_executions = Some(10);
                                    } else {
                                        rule.max_executions = None;
                                        rule.quota_period = None;
                                        rule.clear_quota_runtime();
                                    }
                                }
                            });

                            if rule.max_executions.is_some() {
                                let mut reset_clicked = false;
                                egui::Frame::new()
                                    .fill(t.bg)
                                    .stroke(t.hairline())
                                    .inner_margin(egui::Margin::symmetric(10, 8))
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        // Every control in the panel uses the same height so the
                                        // labels and the fields line up vertically.
                                        ui.spacing_mut().interact_size.y = theme::framed_control_height(ui);
                                        egui::Grid::new("inspector_quota_grid")
                                            .num_columns(2)
                                            .spacing(vec2(10.0, 6.0))
                                            .show(ui, |ui| {
                                                let limit_label = quota_form_label(ui, loc.inspector_quota_limit, t);
                                                if let Some(limit) = &mut rule.max_executions {
                                                    ui.add(
                                                        egui::DragValue::new(limit)
                                                            .range(1..=100_000)
                                                            .suffix(" runs"),
                                                    )
                                                    .labelled_by(limit_label.id);
                                                }
                                                ui.end_row();

                                                let period_label = quota_form_label(ui, loc.inspector_quota_period, t);
                                                let mut period = rule.quota_period;
                                                let period_combo = egui::ComboBox::from_id_salt("inspector_quota_period")
                                                    .selected_text(quota_period_label(period, loc))
                                                    .width(170.0)
                                                    // Seven full-height rows; the 200 px default
                                                    // would hide the last option below a scrollbar.
                                                    .height(300.0)
                                                    .show_ui(ui, |ui| {
                                                        ui.selectable_value(&mut period, None, loc.inspector_quota_period_total);
                                                        ui.selectable_value(&mut period, Some(QuotaPeriod::Minute), loc.inspector_quota_period_minute);
                                                        ui.selectable_value(&mut period, Some(QuotaPeriod::Hour), loc.inspector_quota_period_hour);
                                                        ui.selectable_value(&mut period, Some(QuotaPeriod::Day), loc.inspector_quota_period_day);
                                                        ui.selectable_value(&mut period, Some(QuotaPeriod::Week), loc.inspector_quota_period_week);
                                                        ui.selectable_value(&mut period, Some(QuotaPeriod::Month), loc.inspector_quota_period_month);
                                                        ui.selectable_value(
                                                            &mut period,
                                                            Some(QuotaPeriod::Custom { value: 1, unit: QuotaUnit::Hours }),
                                                            loc.inspector_quota_period_custom,
                                                        );
                                                    });
                                                period_combo.response.labelled_by(period_label.id);
                                                rule.quota_period = period;
                                                ui.end_row();

                                                if let Some(QuotaPeriod::Custom { value, unit }) = &mut rule.quota_period {
                                                    let window_label = quota_form_label(ui, loc.inspector_quota_period, t)
                                                        .on_hover_text(loc.inspector_quota_custom_tip);
                                                    ui.horizontal(|ui| {
                                                        ui.add(
                                                            egui::DragValue::new(value)
                                                                .range(1..=100_000)
                                                                .speed(1.0),
                                                        )
                                                        .labelled_by(window_label.id);
                                                        egui::ComboBox::from_id_salt("inspector_quota_unit")
                                                            .selected_text(quota_unit_label(*unit, loc))
                                                            .width(64.0)
                                                            .height(220.0)
                                                            .show_ui(ui, |ui| {
                                                                for option in QuotaUnit::ALL {
                                                                    ui.selectable_value(unit, option, quota_unit_label(option, loc));
                                                                }
                                                            })
                                                            .response
                                                            .labelled_by(window_label.id);
                                                    });
                                                    ui.end_row();
                                                }
                                            });

                                        // Footer: remaining budget (separated from the fields)
                                        // and the counter reset action right next to it.
                                        ui.add_space(2.0);
                                        ui.separator();
                                        ui.add_space(2.0);
                                        let now = chrono::Local::now();
                                        let exhausted = rule.quota_exhausted(now);
                                        let limit = rule.max_executions.unwrap_or(0);
                                        let remaining = rule.quota_remaining(now).unwrap_or(0);
                                        let mut status = loc
                                            .inspector_runs_remaining
                                            .replacen("{}", &remaining.to_string(), 1)
                                            .replacen("{}", &limit.to_string(), 1);
                                        if exhausted {
                                            if let Some(reset) = rule.quota_next_reset(now) {
                                                status.push_str(" · ");
                                                status.push_str(
                                                    &loc.inspector_quota_resets_at
                                                        .replacen("{}", &reset.format("%H:%M").to_string(), 1),
                                                );
                                            }
                                        }
                                        ui.allocate_ui_with_layout(
                                            vec2(ui.available_width(), ui.spacing().interact_size.y),
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| {
                                                ui.spacing_mut().item_spacing.x = 8.0;
                                                // The counter reset is a bare glyph, not a framed
                                                // button: it only reacts to hover.
                                                let (reset_rect, reset_response) =
                                                    ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                                                let reset_color = if reset_response.hovered() { t.accent_ink } else { t.faint };
                                                ui.painter().text(
                                                    reset_rect.center(),
                                                    egui::Align2::CENTER_CENTER,
                                                    egui_phosphor::regular::ARROW_COUNTER_CLOCKWISE,
                                                    egui::FontId::new(15.0, theme::phosphor()),
                                                    reset_color,
                                                );
                                                reset_response.widget_info(|| {
                                                    egui::WidgetInfo::labeled(
                                                        egui::Role::Button,
                                                        true,
                                                        loc.inspector_btn_reset,
                                                    )
                                                });
                                                let reset_response = reset_response
                                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                                    .on_hover_text(loc.inspector_btn_reset);
                                                if reset_response.clicked() {
                                                    reset_clicked = true;
                                                }
                                                widgets::hud(ui, &status, if exhausted { t.warn } else { t.faint });
                                            },
                                        );
                                    });
                                if reset_clicked {
                                    state.reset_rule_quota(rule);
                                }
                            }

                            // Stop pipeline on error (separate line)
                            ui.checkbox(&mut rule.stop_on_error, loc.inspector_field_stop_on_error)
                                .on_hover_text(loc.inspector_field_stop_on_error_tip);

                            // Rule active (at the very end of the block on a separate line)
                            if is_manual {
                                rule.is_enabled = true;
                            }
                            let cb = egui::Checkbox::new(&mut rule.is_enabled, loc.inspector_checkbox_rule_active);
                            let resp = ui.add_enabled(!is_manual, cb);
                            if is_manual {
                                resp.on_disabled_hover_text(loc.inspector_checkbox_rule_active_manual_tip);
                            }
                        });
                    });
                });

            super::trigger::render_trigger_section(ui, state, rule, t);

            ui.add_space(8.0);

            // 3. Pre-conditions (Section 03)
            widgets::eyebrow(ui, "03", loc.inspector_eyebrow_conditions);
            render_conditions_section(ui, state, rule, t);

            ui.add_space(8.0);

            let is_process_trigger = matches!(rule.trigger, Some(Trigger::Process { .. }));

            // 4. Action Sequence Builder (Execution on trigger)
            let action_header = if is_process_trigger {
                loc.inspector_eyebrow_actions_start
            } else {
                loc.inspector_eyebrow_actions_pipeline
            };
            widgets::eyebrow(ui, "04", action_header);

            let exec_opt = state.get_rule_execution_state(rule);
            let step_prefix = loc.inspector_step_prefix;
            let switch_to_hunter = {
                let mut host = RulePipelineHost {
                    state,
                    t,
                    is_exit_pipeline: false,
                    exec_opt,
                    switch_to_hunter: None,
                };
                render_action_pipeline(
                    ui,
                    &mut rule.actions,
                    "main_actions",
                    step_prefix,
                    t,
                    &mut host,
                );
                host.switch_to_hunter
            };
            if let Some(idx) = switch_to_hunter {
                state.open_process_picker(crate::gui::state::ProcessPickerTarget::Action(idx));
            }

            // 5. Actions on Process Exit
            if is_process_trigger {
                ui.add_space(8.0);
                widgets::eyebrow(ui, "05", loc.inspector_eyebrow_actions_exit);

                let exit_prefix = loc.inspector_exit_prefix;
                let switch_to_hunter = {
                    let mut host = RulePipelineHost {
                        state,
                        t,
                        is_exit_pipeline: true,
                        exec_opt,
                        switch_to_hunter: None,
                    };
                    render_action_pipeline(
                        ui,
                        &mut rule.on_exit_actions,
                        "exit_actions",
                        exit_prefix,
                        t,
                        &mut host,
                    );
                    host.switch_to_hunter
                };
                if let Some(idx) = switch_to_hunter {
                    state.open_process_picker(crate::gui::state::ProcessPickerTarget::ExitAction(idx));
                }
            }

        });
    });
}
