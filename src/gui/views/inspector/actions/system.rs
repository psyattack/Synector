//! `system` action field editors.

use super::*;

pub(crate) fn render_notification_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    _resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let Action::Notification { title, message } = action else { return; };
    widgets::hud(ui, loc.inspector_field_notif_title, t.muted);
    ui.add(
        egui::TextEdit::singleline(title)
            .hint_text(loc.inspector_field_notif_title_hint)
            .desired_width(ui.available_width())
            .min_size(vec2(0.0, theme::control_height(ui)))
            .align(egui::Align2::LEFT_CENTER)
            .margin(vec2(10.0, 8.0)),
    );

    widgets::hud(ui, loc.inspector_field_notif_message, t.muted);
    ui.add(
        egui::TextEdit::singleline(message)
            .hint_text(loc.inspector_field_notif_message_hint)
            .desired_width(ui.available_width())
            .min_size(vec2(0.0, theme::control_height(ui)))
            .align(egui::Align2::LEFT_CENTER)
            .margin(vec2(10.0, 8.0)),
    );
}

pub(crate) fn render_system_power_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    _resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let Action::SystemPower { power_type } = action else { return; };
    widgets::hud(ui, loc.inspector_field_power_action, t.muted);
    let power_options = [
        (SystemPowerType::Lock, Cell::new(loc.inspector_cell_lock, loc.inspector_power_lock)),
        (SystemPowerType::Sleep, Cell::new(loc.inspector_cell_sleep, loc.inspector_power_sleep)),
        (SystemPowerType::TurnScreenOff, Cell::new(loc.inspector_cell_screen_off, loc.inspector_power_screen_off)),
        (SystemPowerType::Restart, Cell::new(loc.inspector_cell_restart, loc.inspector_power_restart)),
        (SystemPowerType::Shutdown, Cell::new(loc.inspector_cell_shutdown, loc.inspector_power_shutdown)),
    ];
    widgets::cells(ui, power_type, &power_options);
}

pub(crate) fn render_power_plan_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    _resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let Action::PowerPlan { plan } = action else { return; };
    widgets::hud(ui, loc.inspector_field_energy_scheme, t.muted);
    let plan_options = [
        (PowerPlanType::HighPerformance, Cell::new(loc.inspector_power_plan_high, loc.inspector_power_plan_high_tip)),
        (PowerPlanType::Balanced, Cell::new(loc.inspector_power_plan_balanced, loc.inspector_power_plan_balanced_tip)),
        (PowerPlanType::PowerSaver, Cell::new(loc.inspector_power_plan_saver, loc.inspector_power_plan_saver_tip)),
    ];
    widgets::cells(ui, plan, &plan_options);
}

pub(crate) fn render_firewall_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let default_picker_target = ctx.default_picker_target;
    let Action::Firewall { process_name, action } = action else { return; };
    widgets::hud(ui, loc.inspector_field_target_process, t.muted);
    if render_process_name_row(ui, loc, process_name, loc.inspector_hint_action_process) {
        resp.pick_target = Some(default_picker_target);
    }

    ui.add_space(4.0);
    let fw_options = [
        (crate::models::FirewallAction::Block, Cell::new(loc.inspector_cell_block_traffic, "")),
        (crate::models::FirewallAction::Allow, Cell::new(loc.inspector_cell_allow_traffic, "")),
        (crate::models::FirewallAction::Toggle, Cell::new(loc.inspector_cell_toggle_traffic, "")),
    ];
    widgets::cells(ui, action, &fw_options);
}

pub(crate) fn render_display_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    _resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let Action::Display { width, height, refresh_rate, monitor_index } = action else { return; };
    widgets::hud(ui, loc.inspector_field_monitor, t.muted);
    widgets::checkbox_row(ui, |ui| {
        let mut has_mon = monitor_index.is_some();
        if ui.checkbox(&mut has_mon, loc.inspector_display_monitor_enable).changed() {
            *monitor_index = if has_mon { Some(1) } else { None };
        }
        if let Some(m) = monitor_index {
            ui.add(egui::DragValue::new(m).range(1..=16).prefix(loc.inspector_field_monitor_prefix));
        }
    });

    ui.add_space(4.0);
    widgets::hud(ui, loc.inspector_field_resolution, t.muted);
    ui.horizontal(|ui| {
        // `None` means "keep the current size"; only a real edit may write a
        // value back, mirroring the x/y rows of the window action below.
        let mut w_val = width.unwrap_or(1920);
        let mut h_val = height.unwrap_or(1080);
        if ui.add(egui::DragValue::new(&mut w_val).range(640..=7680).prefix(loc.inspector_field_w_prefix).suffix(" px")).changed() {
            *width = Some(w_val);
        }
        if ui.add(egui::DragValue::new(&mut h_val).range(480..=4320).prefix(loc.inspector_field_h_prefix).suffix(" px")).changed() {
            *height = Some(h_val);
        }
    });

    ui.add_space(4.0);
    widgets::hud(ui, loc.inspector_field_refresh_rate, t.muted);
    widgets::checkbox_row(ui, |ui| {
        let mut has_hz = refresh_rate.is_some();
        if ui.checkbox(&mut has_hz, loc.inspector_display_refresh_enable).changed() {
            *refresh_rate = if has_hz { Some(144) } else { None };
        }
        if let Some(hz) = refresh_rate {
            ui.add(egui::DragValue::new(hz).range(30..=360).suffix(" Hz"));
        }
    });
}

pub(crate) fn render_trim_working_set_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let default_picker_target = ctx.default_picker_target;
    let Action::TrimWorkingSet { target_process, soft_limit_mb, skip_if_focused, } = action else { return; };
    widgets::hud(ui, loc.inspector_field_target_process, t.muted);
    if render_optional_process_name_row(ui, loc, target_process, loc.inspector_trim_target_hint) {
        resp.pick_target = Some(default_picker_target);
    }
    ui.add_space(4.0);
    widgets::checkbox_group(ui, |ui| {
        widgets::checkbox_row(ui, |ui| {
            let mut has_limit = soft_limit_mb.is_some();
            if ui.checkbox(&mut has_limit, loc.inspector_trim_enable_soft_limit).changed() {
                *soft_limit_mb = if has_limit { Some(256) } else { None };
            }
            if let Some(mb) = soft_limit_mb {
                let mut mb_val = *mb;
                if ui.add(egui::DragValue::new(&mut mb_val).range(2..=65536).suffix(" MB")).changed() {
                    *soft_limit_mb = Some(mb_val);
                }
            }
        });
        ui.checkbox(skip_if_focused, loc.inspector_trim_skip_focused);
    });
}

pub(crate) fn render_discord_rpc_set_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    _resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let Action::DiscordRpcSet { app_id, details, state: rpc_state, large_image, large_text, small_image, small_text, show_elapsed, buttons, auto_clear_on_process_exit, } = action else { return; };
    widgets::hud(ui, loc.inspector_discord_app_id, t.muted);
    ui.add(
        egui::TextEdit::singleline(app_id)
            .hint_text(loc.inspector_hint_discord_app_id)
            .desired_width(ui.available_width())
            .min_size(vec2(0.0, theme::control_height(ui)))
            .margin(vec2(10.0, 8.0)),
    );
    ui.add_space(4.0);

    widgets::hud(ui, loc.inspector_discord_details, t.muted);
    ui.add(
        egui::TextEdit::singleline(details)
            .hint_text(loc.inspector_hint_discord_details)
            .desired_width(ui.available_width())
            .min_size(vec2(0.0, theme::control_height(ui)))
            .margin(vec2(10.0, 8.0)),
    );
    ui.add_space(4.0);

    widgets::hud(ui, loc.inspector_discord_state, t.muted);
    ui.add(
        egui::TextEdit::singleline(rpc_state)
            .hint_text(loc.inspector_hint_discord_state)
            .desired_width(ui.available_width())
            .min_size(vec2(0.0, theme::control_height(ui)))
            .margin(vec2(10.0, 8.0)),
    );
    ui.add_space(4.0);

    widgets::hud(ui, loc.inspector_discord_large_image, t.muted);
    let mut l_img = large_image.clone().unwrap_or_default();
    if ui.add(
        egui::TextEdit::singleline(&mut l_img)
            .hint_text(loc.inspector_hint_discord_large_image)
            .desired_width(ui.available_width())
            .min_size(vec2(0.0, theme::control_height(ui)))
            .margin(vec2(10.0, 8.0)),
    ).changed() {
        *large_image = if l_img.trim().is_empty() { None } else { Some(l_img) };
    }
    ui.add_space(4.0);

    widgets::hud(ui, loc.inspector_discord_large_text, t.muted);
    let mut l_txt = large_text.clone().unwrap_or_default();
    if ui.add(
        egui::TextEdit::singleline(&mut l_txt)
            .hint_text(loc.inspector_hint_discord_large_text)
            .desired_width(ui.available_width())
            .min_size(vec2(0.0, theme::control_height(ui)))
            .margin(vec2(10.0, 8.0)),
    ).changed() {
        *large_text = if l_txt.trim().is_empty() { None } else { Some(l_txt) };
    }
    ui.add_space(4.0);

    widgets::hud(ui, loc.inspector_discord_small_image, t.muted);
    let mut s_img = small_image.clone().unwrap_or_default();
    if ui.add(
        egui::TextEdit::singleline(&mut s_img)
            .hint_text(loc.inspector_hint_discord_small_image)
            .desired_width(ui.available_width())
            .min_size(vec2(0.0, theme::control_height(ui)))
            .margin(vec2(10.0, 8.0)),
    ).changed() {
        *small_image = if s_img.trim().is_empty() { None } else { Some(s_img) };
    }
    ui.add_space(4.0);

    widgets::hud(ui, loc.inspector_discord_small_text, t.muted);
    let mut s_txt = small_text.clone().unwrap_or_default();
    if ui.add(
        egui::TextEdit::singleline(&mut s_txt)
            .hint_text(loc.inspector_hint_discord_small_text)
            .desired_width(ui.available_width())
            .min_size(vec2(0.0, theme::control_height(ui)))
            .margin(vec2(10.0, 8.0)),
    ).changed() {
        *small_text = if s_txt.trim().is_empty() { None } else { Some(s_txt) };
    }
    ui.add_space(4.0);

    widgets::checkbox_group(ui, |ui| {
        ui.checkbox(show_elapsed, loc.inspector_discord_show_elapsed);
        ui.checkbox(auto_clear_on_process_exit, loc.inspector_discord_auto_clear);
    });
    ui.add_space(6.0);

    // Buttons
    widgets::hud(ui, loc.inspector_discord_buttons, t.muted);
    let mut remove_btn_idx = None;
    for (b_i, btn) in buttons.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.set_width(ui.available_width());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::glyph_button(ui, Glyph::Close, loc.inspector_tip_remove_step).clicked() {
                    remove_btn_idx = Some(b_i);
                }
                let avail = ui.available_width();
                let label_w = (avail * 0.35).clamp(80.0, 120.0);
                let url_w = (avail - label_w - ui.spacing().item_spacing.x).max(40.0);
                ui.add(
                    egui::TextEdit::singleline(&mut btn.url)
                        .hint_text(loc.inspector_discord_btn_url)
                        .desired_width(url_w)
                        .min_size(vec2(0.0, theme::control_height(ui)))
                        .align(egui::Align2::LEFT_CENTER)
                        .margin(vec2(8.0, 6.0)),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut btn.label)
                        .hint_text(loc.inspector_discord_btn_label)
                        .desired_width(label_w)
                        .min_size(vec2(0.0, theme::control_height(ui)))
                        .align(egui::Align2::LEFT_CENTER)
                        .margin(vec2(8.0, 6.0)),
                );
            });
        });
        ui.add_space(2.0);
    }
    if let Some(del) = remove_btn_idx {
        buttons.remove(del);
    }
    if buttons.len() < 2
        && render_full_width_action_button(ui, loc.inspector_discord_btn_add, "", t)
    {
        buttons.push(crate::models::DiscordRpcButton::default());
    }
}

pub(crate) fn render_discord_rpc_clear_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    _resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let Action::DiscordRpcClear = action else { return; };
    egui::Frame::new()
        .fill(t.sunken)
        .stroke(t.hairline())
        .corner_radius(t.card_corner())
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.label(
                    egui::RichText::new(egui_phosphor::regular::X_CIRCLE)
                        .size(20.0)
                        .color(t.bad),
                );
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(loc.catalog_desc_discord_rpc_clear)
                            .size(11.5)
                            .color(t.ink_strong),
                    ).wrap(),
                );
            });
        });
}


#[cfg(test)]
mod tests {
    use super::*;

    use crate::gui::state::{GuiState, ProcessPickerTarget};


    #[test]
    fn test_display_fields_keep_unset_size() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let state = GuiState::new();
        let mut action = Action::Display {
            width: None,
            height: None,
            refresh_rate: None,
            monitor_index: None,
        };

        // `None` means "keep the current size"; merely rendering the editor must not turn it
        // into an explicit 1920x1080.
        for _ in 0..3 {
            let mut out = ctx.run_ui(Default::default(), |ctx| {
                let t = Tokens::get(ctx);
                egui::CentralPanel::default().show(ctx, |ui| {
                    let _ = render_action_parameter_fields(
                        ui,
                        &state,
                        &mut action,
                        0,
                        ProcessPickerTarget::Action(0),
                        &t,
                        0,
                    );
                });
            });
            out.textures_delta.clear();
        }

        match action {
            Action::Display { width, height, .. } => {
                assert_eq!(width, None, "an unset width stays unset");
                assert_eq!(height, None, "an unset height stays unset");
            }
            _ => unreachable!(),
        }
    }
}
