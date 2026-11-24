//! `overlay` action field editors.

use super::*;

pub(crate) fn render_overlay_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let idx = ctx.idx;
    let is_exit_pipeline = ctx.is_exit_pipeline;
    let state = ctx.state;
    let Action::Overlay { overlay_id, custom_overlay, operation, override_text, duration_ms, close_on_exit, } = action else { return; };
    let all_overlays = state.daemon_state.as_ref()
        .map(|s| s.config.overlays.clone())
        .unwrap_or_else(|| crate::config::Library::read().overlays);

    // An unset overlay id is rendered from the first available preset but is written
    // into the draft only when the user picks a preset, detaches or opens the studio.
    let display_overlay = all_overlays
        .iter()
        .find(|o| o.id == *overlay_id || o.name.eq_ignore_ascii_case(overlay_id))
        .or_else(|| all_overlays.first());

    // 1. Preset Source Selector
    let is_custom = custom_overlay.is_some();
    widgets::hud(ui, loc.inspector_preset_source, t.muted);
    ui.horizontal(|ui| {
        let current_label = if is_custom {
            loc.inspector_preset_custom.to_string()
        } else {
            let matched_name = display_overlay
                .map(|o| o.name.clone())
                .unwrap_or_else(|| overlay_id.clone());
            loc.inspector_preset_prefix.replace("{}", &matched_name)
        };

        // The selector takes whatever the trailing detach button leaves. The bounded slot makes
        // the combo truncate the preset name instead of widening the step card with it.
        let detach_w = if !is_custom {
            widgets::compact_button_width(ui, loc.inspector_preset_detach)
                + ui.spacing().item_spacing.x
        } else {
            0.0
        };
        let combo_w = (ui.available_width() - detach_w).max(1.0);
        preset_selector_slot(ui, combo_w, |ui| {
            egui::ComboBox::from_id_salt(("overlay_preset_select", idx))
                .selected_text(RichText::new(current_label).size(12.0))
                .width(combo_w)
                .truncate()
                .show_ui(ui, |ui| {
                    if ui.selectable_label(is_custom, loc.inspector_preset_custom).clicked()
                        && custom_overlay.is_none()
                    {
                        let id = if overlay_id.is_empty() {
                            display_overlay
                                .map(|o| o.id.clone())
                                .unwrap_or_else(|| "overlay_crosshair".to_string())
                        } else {
                            overlay_id.clone()
                        };
                        let mut def = display_overlay
                            .cloned()
                            .unwrap_or_else(crate::models::OverlayDefinition::default_new_overlay);
                        def.id = id.clone();
                        *overlay_id = id;
                        *custom_overlay = Some(Box::new(def));
                    }
                    for ov in &all_overlays {
                        let is_sel = !is_custom && (ov.id == *overlay_id || ov.name.eq_ignore_ascii_case(overlay_id));
                        if ui.selectable_label(is_sel, &ov.name).clicked() {
                            *custom_overlay = None;
                            *overlay_id = ov.id.clone();
                        }
                    }
                });
        });

        if !is_custom
            && widgets::compact_button(ui, loc.inspector_preset_detach, Kind::Secondary, 24.0)
                .on_hover_text(loc.inspector_preset_detach_tip)
                .clicked()
        {
            let id = if overlay_id.is_empty() {
                display_overlay
                    .map(|o| o.id.clone())
                    .unwrap_or_else(|| "overlay_crosshair".to_string())
            } else {
                overlay_id.clone()
            };
            let mut def = display_overlay
                .cloned()
                .unwrap_or_else(crate::models::OverlayDefinition::default_new_overlay);
            def.id = id.clone();
            *overlay_id = id;
            *custom_overlay = Some(Box::new(def));
        }
    });

    // 2. Overlay Studio Button (shown if not Hide)
    if *operation != crate::models::OverlayOperation::Hide {
        ui.add_space(6.0);
        let banner_h = 44.0;
        let (rect, overlay_btn_resp) = ui.allocate_exact_size(vec2(ui.available_width(), banner_h), Sense::click());
        let hovered = overlay_btn_resp.hovered();

        if overlay_btn_resp.clicked() {
            // Opening the studio is an explicit user action, so an unset id may be
            // resolved here instead of during render.
            if overlay_id.is_empty() {
                let id = display_overlay
                    .map(|o| o.id.clone())
                    .unwrap_or_else(|| "overlay_crosshair".to_string());
                *overlay_id = id.clone();
                if let Some(def) = custom_overlay.as_deref_mut() {
                    def.id = id;
                }
            }
            resp.expand_overlay_action = Some(idx);
        }

        let is_overlay_showing = state.daemon_state.as_ref().is_some_and(|d| d.active_overlay_ids.contains(overlay_id))
            || state.active_test_overlays.contains(overlay_id);

        let p = ui.painter();
        let bg = if hovered { t.sunken } else { t.bg };
        let border_color = if is_overlay_showing || hovered {
            t.accent_ink
        } else {
            t.hairline().color
        };
        p.rect_filled(rect, t.inset_corner(), bg);
        p.rect_stroke(rect, t.inset_corner(), Stroke::new(1.0, border_color), egui::StrokeKind::Inside);

        // Icon chip on left
        let icon_box = Rect::from_min_size(rect.min + vec2(8.0, 7.0), vec2(30.0, 30.0));
        let icon_bg = if is_overlay_showing || hovered { t.accent_ink.gamma_multiply(0.22) } else { t.accent_ink.gamma_multiply(0.12) };
        p.rect_filled(icon_box, t.inset_corner(), icon_bg);
        p.text(
            icon_box.center(),
            egui::Align2::CENTER_CENTER,
            egui_phosphor::regular::FRAME_CORNERS,
            egui::FontId::new(18.0, theme::phosphor()),
            t.accent_ink,
        );

        // Middle title
        let text_left = icon_box.right() + 10.0;
        let btn_title = if is_custom {
            loc.inspector_overlay_studio_title.to_string()
        } else {
            let matched_name = display_overlay
                .map(|o| o.name.as_str())
                .unwrap_or(overlay_id.as_str());
            format!("{} ({})", loc.inspector_overlay_studio_title, matched_name)
        };
        let title_color = if is_overlay_showing || hovered { t.accent_ink } else { t.ink_strong };
        let (status_text, status_color) = if is_overlay_showing {
            (loc.inspector_badge_executing_now, t.accent_ink)
        } else if hovered {
            (loc.inspector_overlay_studio_open, t.accent_ink)
        } else {
            (loc.inspector_overlay_studio_open, t.faint)
        };
        // The title carries the preset name, so it is truncated before the right-hand pill
        // instead of painting over it or past the card.
        let pill_left = paint_studio_banner_pill(
            p,
            status_text,
            rect.right() - 14.0,
            rect.center().y,
            status_color,
        );
        paint_studio_banner_title(
            p,
            &btn_title,
            text_left,
            rect.center().y,
            pill_left - 8.0,
            title_color,
        );
    }

    ui.add_space(6.0);

    // 3. Operation selector (Show / Hide / Toggle / Flash / SetText in 2 rows to fit any screen width)
    widgets::hud(ui, loc.inspector_overlay_operation, t.muted);
    let row1 = [
        (crate::models::OverlayOperation::Show, widgets::Cell::new(loc.inspector_overlay_op_show, loc.inspector_overlay_op_show_desc)),
        (crate::models::OverlayOperation::Hide, widgets::Cell::new(loc.inspector_overlay_op_hide, loc.inspector_overlay_op_hide_desc)),
        (crate::models::OverlayOperation::Toggle, widgets::Cell::new(loc.inspector_overlay_op_toggle, loc.inspector_overlay_op_toggle_desc)),
    ];
    widgets::cells(ui, operation, &row1);
    ui.add_space(2.0);
    let row2 = [
        (crate::models::OverlayOperation::Flash, widgets::Cell::new(loc.inspector_overlay_op_flash, loc.inspector_overlay_op_flash_desc)),
        (crate::models::OverlayOperation::SetText, widgets::Cell::new(loc.inspector_overlay_op_set_text, loc.inspector_overlay_op_set_text_desc)),
    ];
    widgets::cells(ui, operation, &row2);

    if *operation == crate::models::OverlayOperation::Flash {
        ui.add_space(4.0);
        widgets::hud(ui, loc.inspector_overlay_duration_ms, t.muted);
        let mut d = duration_ms.unwrap_or(3000);
        if widgets::DurationField::millis(ui, loc, ("overlay_flash", idx, is_exit_pipeline), 200.0..=30_000.0)
            .default_unit(widgets::TimeUnit::Milliseconds)
            .show_millis_u32(ui, &mut d)
            .changed()
        {
            *duration_ms = Some(d);
        }
    }

    if *operation != crate::models::OverlayOperation::Hide {
        ui.add_space(4.0);
        widgets::hud(ui, loc.inspector_overlay_override_text, t.muted);
        let mut ov_txt = override_text.clone().unwrap_or_default();
        if ui.add(
            egui::TextEdit::singleline(&mut ov_txt)
                .hint_text(loc.inspector_overlay_hint_override)
                .desired_width(ui.available_width())
                .min_size(vec2(0.0, theme::control_height(ui)))
                .margin(vec2(10.0, 8.0)),
        ).changed() {
            *override_text = if ov_txt.trim().is_empty() { None } else { Some(ov_txt) };
        }
    }

    // Compact checkbox group for overlay behaviors (click-through, draggable dependent on click-through, hide from capture)
    if let Some(overlay_def) = custom_overlay.as_deref_mut() {
        ui.add_space(4.0);
        widgets::hud(ui, loc.inspector_overlay_fade_title, t.muted);
        ui.columns(2, |cols| {
            let mut fade_in = overlay_def.fade_in_ms.unwrap_or(0);
            let fade_in_field = widgets::DurationField::millis(&cols[0], loc, ("overlay_fade_in", idx, is_exit_pipeline), 0.0..=5000.0)
                .default_unit(widgets::TimeUnit::Milliseconds)
                .prefix(format!("{}: ", loc.inspector_overlay_fade_in));
            if fade_in_field.show_millis_u32(&mut cols[0], &mut fade_in).changed() {
                overlay_def.fade_in_ms = if fade_in == 0 { None } else { Some(fade_in) };
            }

            let mut fade_out = overlay_def.fade_out_ms.unwrap_or(0);
            let fade_out_field = widgets::DurationField::millis(&cols[1], loc, ("overlay_fade_out", idx, is_exit_pipeline), 0.0..=5000.0)
                .default_unit(widgets::TimeUnit::Milliseconds)
                .prefix(format!("{}: ", loc.inspector_overlay_fade_out));
            if fade_out_field.show_millis_u32(&mut cols[1], &mut fade_out).changed() {
                overlay_def.fade_out_ms = if fade_out == 0 { None } else { Some(fade_out) };
            }
        });

        if overlay_def.anchor == crate::models::OverlayAnchor::WindowRelative {
            ui.add_space(4.0);
            widgets::hud(ui, loc.overlay_studio_target_process_title, t.muted);
            let mut proc_txt = overlay_def.target_process.clone().unwrap_or_default();
            if ui.add(
                egui::TextEdit::singleline(&mut proc_txt)
                    .hint_text(loc.overlay_studio_hint_process)
                    .desired_width(ui.available_width())
                    .min_size(vec2(0.0, theme::control_height(ui)))
                    .margin(vec2(10.0, 8.0)),
            ).changed() {
                overlay_def.target_process = if proc_txt.trim().is_empty() { None } else { Some(proc_txt) };
            }
            ui.checkbox(&mut overlay_def.close_with_target, loc.overlay_studio_close_with_target)
                .on_hover_text(loc.overlay_studio_close_with_target_tip);
        }

        ui.add_space(6.0);
        widgets::checkbox_group(ui, |ui| {
            let ct_resp = ui.checkbox(&mut overlay_def.click_through, loc.overlay_studio_click_through);
            if ct_resp.changed() && overlay_def.click_through {
                overlay_def.draggable = false;
            }

            let drag_enabled = !overlay_def.click_through;
            let drag_resp = ui.add_enabled(
                drag_enabled,
                egui::Checkbox::new(&mut overlay_def.draggable, loc.overlay_studio_draggable),
            );
            if drag_resp.changed() && overlay_def.draggable {
                overlay_def.click_through = false;
            }

            ui.checkbox(&mut overlay_def.hide_from_capture, loc.overlay_studio_hide_from_capture);
            let is_process_trigger = state.editing_rule.as_ref().is_some_and(|r| matches!(r.trigger, Some(Trigger::Process { .. })));
            let can_companion = is_process_trigger && !is_exit_pipeline;
            if !can_companion {
                *close_on_exit = false;
            }
            let resp = ui.add_enabled(
                can_companion,
                egui::Checkbox::new(close_on_exit, loc.inspector_overlay_close_on_exit),
            );
            if !can_companion {
                resp.on_disabled_hover_text(loc.inspector_companion_disabled_tip);
            } else {
                resp.on_hover_text(loc.inspector_overlay_close_on_exit_tip);
            }
        });
    }
}
