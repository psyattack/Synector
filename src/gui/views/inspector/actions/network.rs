//! `network` action field editors.

use super::*;

pub(crate) fn render_http_webhook_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    _resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let idx = ctx.idx;
    let Action::HttpWebhook { url, method, payload, .. } = action else { return; };
    widgets::hud(ui, loc.inspector_field_webhook_url, t.muted);
    ui.horizontal(|ui| {
        ui.set_width(ui.available_width());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let combo_id = ui.scope_id().with(idx).with("meth_combo");
            egui::ComboBox::from_id_salt(combo_id)
                .selected_text(method.as_str())
                .width(65.0)
                .show_ui(ui, |ui| {
                    for m in &["POST", "GET", "PUT", "DELETE"] {
                        ui.selectable_value(method, m.to_string(), *m);
                    }
                });
            ui.add(
                egui::TextEdit::singleline(url)
                    .hint_text(loc.inspector_field_webhook_url_hint)
                    .desired_width(ui.available_width())
                    .min_size(vec2(0.0, theme::CONTROL_H))
                    .align(egui::Align2::LEFT_CENTER)
                    .margin(vec2(10.0, 8.0)),
            );
        });
    });

    widgets::hud(ui, loc.inspector_field_json_body, t.muted);
    let mut p_str = payload.clone().unwrap_or_default();
    if ui.add(
        egui::TextEdit::multiline(&mut p_str)
            .hint_text(loc.inspector_hint_webhook_body)
            .desired_width(ui.available_width())
            .desired_rows(2)
            .font(egui::FontId::new(12.0, theme::mono()))
            .margin(vec2(10.0, 8.0)),
    ).changed() {
        *payload = if p_str.trim().is_empty() { None } else { Some(p_str) };
    }
}

pub(crate) fn render_window_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let idx = ctx.idx;
    let default_picker_target = ctx.default_picker_target;
    let Action::Window { window_action, target_process, x, y, width, height, transparency, } = action else { return; };
    widgets::hud(ui, loc.inspector_field_target_process, t.muted);
    if render_optional_process_name_row(ui, loc, target_process, loc.inspector_hint_window_process) {
        resp.pick_target = Some(default_picker_target);
    }

    widgets::hud(ui, loc.inspector_field_window_operation, t.muted);
    let combo_id = ui.scope_id().with(idx).with("win_act_combo");
    egui::ComboBox::from_id_salt(combo_id)
        .selected_text(window_action_label(window_action, loc))
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            ui.selectable_value(window_action, WindowActionType::Minimize, loc.inspector_win_action_minimize);
            ui.selectable_value(window_action, WindowActionType::Maximize, loc.inspector_win_action_maximize);
            ui.selectable_value(window_action, WindowActionType::Restore, loc.inspector_win_action_restore);
            ui.selectable_value(window_action, WindowActionType::ToggleMinimizeRestore, loc.inspector_win_toggle_min_restore);
            ui.selectable_value(window_action, WindowActionType::Hide, loc.inspector_win_action_hide);
            ui.selectable_value(window_action, WindowActionType::Show, loc.inspector_win_action_show);
            ui.selectable_value(window_action, WindowActionType::ToggleShowHide, loc.inspector_win_toggle_show_hide);
            ui.selectable_value(window_action, WindowActionType::Close, loc.inspector_win_action_close);
            ui.selectable_value(window_action, WindowActionType::AlwaysOnTop, loc.inspector_win_action_always_on_top);
            ui.selectable_value(window_action, WindowActionType::RemoveAlwaysOnTop, loc.inspector_win_action_remove_topmost);
            ui.selectable_value(window_action, WindowActionType::ToggleAlwaysOnTop, loc.inspector_win_action_toggle_topmost);
            ui.selectable_value(window_action, WindowActionType::SetTransparency, loc.inspector_win_action_transparency);
            ui.selectable_value(window_action, WindowActionType::SnapToPosition, loc.inspector_win_action_position);
        });

    if *window_action == WindowActionType::SetTransparency {
        widgets::hud(ui, loc.inspector_field_opacity, t.muted);
        let mut val = transparency.unwrap_or(255);
        if ui.add(egui::Slider::new(&mut val, 10..=255)).changed() {
            *transparency = Some(val);
        }
    } else if *window_action == WindowActionType::SnapToPosition {
        ui.horizontal(|ui| {
            widgets::hud(ui, loc.inspector_field_pos_x, t.muted);
            let mut xv = x.unwrap_or(0);
            if ui.add(egui::DragValue::new(&mut xv).range(0..=7680)).changed() { *x = Some(xv); }
            ui.add_space(10.0);
            widgets::hud(ui, loc.inspector_field_pos_y, t.muted);
            let mut yv = y.unwrap_or(0);
            if ui.add(egui::DragValue::new(&mut yv).range(0..=4320)).changed() { *y = Some(yv); }
        });
        ui.horizontal(|ui| {
            widgets::hud(ui, loc.inspector_field_width, t.muted);
            let mut wv = width.unwrap_or(800);
            if ui.add(egui::DragValue::new(&mut wv).range(100..=7680)).changed() { *width = Some(wv); }
            ui.add_space(10.0);
            widgets::hud(ui, loc.inspector_field_height, t.muted);
            let mut hv = height.unwrap_or(600);
            if ui.add(egui::DragValue::new(&mut hv).range(100..=4320)).changed() { *height = Some(hv); }
        });
    }
}
