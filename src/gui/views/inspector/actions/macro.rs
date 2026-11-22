//! `macro` action field editors.

use super::*;

pub(crate) fn render_macro_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let idx = ctx.idx;
    let state = ctx.state;
    let Action::Macro { preset_id, operation, events, repeat, speed, infinite } = action else { return; };
    let all_macro_presets = state.daemon_state.as_ref()
        .map(|s| s.config.macro_presets.clone())
        .unwrap_or_else(|| crate::config::Library::read().macro_presets);

    // 1. Operation selector (Play / Stop / Toggle)
    widgets::hud(ui, loc.inspector_macro_operation, t.muted);
    let op_options = [
        (crate::models::MacroOperation::Run, widgets::Cell::new(loc.inspector_macro_op_run, loc.inspector_macro_op_run_desc)),
        (crate::models::MacroOperation::Stop, widgets::Cell::new(loc.inspector_macro_op_stop, loc.inspector_macro_op_stop_desc)),
        (crate::models::MacroOperation::Toggle, widgets::Cell::new(loc.inspector_macro_op_toggle, loc.inspector_macro_op_toggle_desc)),
    ];
    widgets::cells(ui, operation, &op_options);

    ui.add_space(4.0);

    // 2. Preset Source selector
    let is_preset = preset_id.is_some();
    widgets::hud(ui, loc.inspector_preset_source, t.muted);
    ui.horizontal(|ui| {
        let current_label = match preset_id {
            Some(pid) => {
                let matched_name = all_macro_presets.iter()
                    .find(|p| p.id == *pid || p.name.eq_ignore_ascii_case(pid))
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| pid.clone());
                loc.inspector_preset_prefix.replace("{}", &matched_name)
            }
            None => loc.inspector_preset_custom.to_string(),
        };

        egui::ComboBox::from_id_salt(("macro_preset_select", idx))
            .selected_text(RichText::new(current_label).size(12.0))
            .width((ui.available_width() - if is_preset { 120.0 } else { 0.0 }).max(140.0))
            .show_ui(ui, |ui| {
                if ui.selectable_label(!is_preset, loc.inspector_preset_custom).clicked() {
                    if let Some(pid) = preset_id.take() {
                        if let Some(p) = all_macro_presets.iter().find(|p| p.id == pid || p.name.eq_ignore_ascii_case(&pid)) {
                            if events.is_empty() {
                                *events = p.events.clone();
                            }
                            *repeat = p.repeat;
                            *speed = p.speed;
                            *infinite = p.infinite;
                        }
                    }
                }
                for p in &all_macro_presets {
                    let is_sel = preset_id.as_deref().is_some_and(|id| id == p.id || id.eq_ignore_ascii_case(&p.name));
                    if ui.selectable_label(is_sel, &p.name).clicked() {
                        *preset_id = Some(p.id.clone());
                        events.clear();
                    }
                }
            });

        if is_preset
            && widgets::compact_button(ui, loc.inspector_preset_detach, Kind::Secondary, 24.0)
                .on_hover_text(loc.inspector_preset_detach_tip)
                .clicked()
        {
            if let Some(pid) = preset_id.take() {
                if let Some(p) = all_macro_presets.iter().find(|p| p.id == pid || p.name.eq_ignore_ascii_case(&pid)) {
                    *events = p.events.clone();
                    *repeat = p.repeat;
                    *speed = p.speed;
                    *infinite = p.infinite;
                }
            }
        }
    });

    // 3. Studio Banner (when Play / Run / Toggle)
    if *operation != crate::models::MacroOperation::Stop {
        ui.add_space(6.0);
        let banner_h = 44.0;
        let (rect, macro_btn_resp) = ui.allocate_exact_size(vec2(ui.available_width(), banner_h), Sense::click());
        let hovered = macro_btn_resp.hovered();

        if macro_btn_resp.clicked() {
            resp.expand_macro = true;
        }

        let p = ui.painter();
        let bg = if hovered { t.sunken } else { t.bg };
        let border_color = if hovered { t.accent_ink } else { t.hairline().color };
        p.rect_filled(rect, CornerRadius::same(2), bg);
        p.rect_stroke(rect, CornerRadius::same(2), Stroke::new(1.0, border_color), egui::StrokeKind::Inside);

        // Icon chip on left
        let icon_box = Rect::from_min_size(rect.min + vec2(8.0, 7.0), vec2(30.0, 30.0));
        let icon_bg = if hovered { t.accent_ink.gamma_multiply(0.22) } else { t.accent_ink.gamma_multiply(0.12) };
        p.rect_filled(icon_box, CornerRadius::same(2), icon_bg);
        p.text(
            icon_box.center(),
            egui::Align2::CENTER_CENTER,
            egui_phosphor::regular::GAME_CONTROLLER,
            egui::FontId::new(18.0, theme::phosphor()),
            t.accent_ink,
        );

        // Middle title (clean, centered vertically)
        let text_left = icon_box.right() + 10.0;
        let title_pos = pos2(text_left, rect.center().y);
        let studio_btn_text = if is_preset {
            let pname = preset_id.as_deref().and_then(|pid| all_macro_presets.iter().find(|p| p.id == pid || p.name.eq_ignore_ascii_case(pid))).map(|p| p.name.as_str()).unwrap_or(loc.inspector_preset_unknown);
            format!("{} ({})", loc.inspector_macro_studio_title, pname)
        } else {
            loc.inspector_macro_studio_title.to_string()
        };
        p.text(
            title_pos,
            egui::Align2::LEFT_CENTER,
            studio_btn_text,
            egui::FontId::new(13.0, theme::semibold()),
            if hovered { t.accent_ink } else { t.ink_strong },
        );

        // Action arrow pill on right
        let arrow_pos = pos2(rect.right() - 14.0, rect.center().y);
        p.text(
            arrow_pos,
            egui::Align2::RIGHT_CENTER,
            loc.inspector_macro_studio_open,
            egui::FontId::new(11.0, theme::mono_bold()),
            if hovered { t.accent_ink } else { t.faint },
        );
    }
}
