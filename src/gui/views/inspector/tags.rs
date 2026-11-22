//! Rule tag editor: pills, picker, palette and icon picker.

use super::*;

pub(crate) fn render_inspector_tag_section(
    ui: &mut Ui,
    state: &mut GuiState,
    rule: &mut Rule,
    loc: &crate::i18n::Strings,
    t: &Tokens,
) {
    ui.add_space(2.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.spacing_mut().item_spacing.y = 4.0;

        let mut remove_idx = None;
        for (idx, tag) in rule.tags.iter().enumerate() {
            let color = theme::tag_color(tag, t.accent_ink);
            let has_icon = tag.icon.as_ref().is_some_and(|i| !i.is_empty());
            let name_galley = widgets::caps(ui, &tag.name, 10.0, theme::medium(), 0.0, color);
            let icon_w = if has_icon { 16.0 } else { 0.0 };
            let pill_w = name_galley.size().x + icon_w + 24.0;
            let (rect, _resp) = ui.allocate_exact_size(vec2(pill_w, 20.0), Sense::hover());

            ui.painter().rect(
                rect,
                CornerRadius::same(10),
                color.gamma_multiply(0.18),
                Stroke::new(1.0, color.gamma_multiply(0.45)),
                egui::StrokeKind::Inside,
            );

            let mut curr_x = rect.left() + 8.0;
            if let Some(ico) = &tag.icon {
                if !ico.is_empty() {
                    let ico_rect = Rect::from_min_size(pos2(curr_x, rect.top()), vec2(14.0, rect.height()));
                    ui.painter().text(
                        ico_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        ico,
                        egui::FontId::new(11.0, theme::phosphor()),
                        color,
                    );
                    curr_x += 15.0;
                }
            }

            let text_pos = pos2(curr_x, rect.center().y - name_galley.size().y / 2.0);
            ui.painter().galley(text_pos, name_galley, color);

            let close_rect = Rect::from_center_size(pos2(rect.right() - 8.0, rect.center().y), vec2(14.0, 14.0));
            let close_resp = ui.interact(close_rect, ui.scope_id().with(("tag_rem", idx)), Sense::click());
            let close_hover = close_resp.hovered();
            ui.painter().text(
                close_rect.center(),
                egui::Align2::CENTER_CENTER,
                egui_phosphor::regular::X,
                egui::FontId::new(9.5, theme::phosphor()),
                if close_hover { t.ink_strong } else { color.gamma_multiply(0.7) },
            );
            if close_resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                remove_idx = Some(idx);
            }
        }

        if let Some(idx) = remove_idx {
            rule.tags.remove(idx);
            state.push_undo_snapshot();
        }

        let add_label = loc.inspector_tags_add;
        let add_galley = widgets::caps(ui, add_label, 10.0, theme::mono(), 0.5, t.ink);
        let add_w = add_galley.size().x + 18.0;
        let (add_rect, add_resp) = ui.allocate_exact_size(vec2(add_w, 20.0), Sense::click());
        let add_hovered = add_resp.hovered();
        ui.painter().rect(
            add_rect,
            CornerRadius::same(10),
            if add_hovered { t.sunken } else { egui::Color32::TRANSPARENT },
            Stroke::new(1.0, if state.tag_picker_open { t.accent_ink } else if add_hovered { t.ink } else { t.line }),
            egui::StrokeKind::Inside,
        );
        widgets::paint_centred(
            ui,
            add_rect,
            add_galley,
            if state.tag_picker_open { t.accent_ink } else if add_hovered { t.ink_strong } else { t.muted },
        );
        if add_resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            state.tag_picker_open = !state.tag_picker_open;
        }
    });

    if state.tag_picker_open {
        ui.add_space(4.0);
        egui::Frame::new()
            .fill(t.sunken.gamma_multiply(0.6))
            .stroke(t.hairline())
            .corner_radius(CornerRadius::same(4))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;

                // 1. Existing Tags
                widgets::hud(ui, loc.inspector_tags_existing, t.muted);
                let all_existing = state.all_distinct_tags();
                let available: Vec<_> = all_existing
                    .into_iter()
                    .filter(|et| !rule.tags.iter().any(|t| t.name.eq_ignore_ascii_case(&et.name)))
                    .collect();

                if available.is_empty() {
                    ui.label(egui::RichText::new(loc.inspector_tags_no_existing).size(11.0).color(t.faint));
                } else {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.spacing_mut().item_spacing.y = 4.0;
                        for ext in available {
                            let c = theme::tag_color(&ext, t.accent_ink);
                            let has_ico = ext.icon.as_ref().is_some_and(|i| !i.is_empty());
                            let name_galley = widgets::caps(ui, &ext.name, 9.5, theme::medium(), 0.0, c);
                            let pill_w = name_galley.size().x + if has_ico { 14.0 } else { 0.0 } + 14.0;
                            let (pill_rect, pill_resp) = ui.allocate_exact_size(vec2(pill_w, 18.0), Sense::click());
                            let h = pill_resp.hovered();
                            ui.painter().rect(
                                pill_rect,
                                CornerRadius::same(9),
                                if h { c.gamma_multiply(0.35) } else { c.gamma_multiply(0.18) },
                                Stroke::new(1.0, if h { c } else { c.gamma_multiply(0.5) }),
                                egui::StrokeKind::Inside,
                            );

                            let mut cx = pill_rect.left() + 6.0;
                            if let Some(ico) = &ext.icon {
                                if !ico.is_empty() {
                                    let ico_rect = Rect::from_min_size(pos2(cx, pill_rect.top()), vec2(12.0, pill_rect.height()));
                                    ui.painter().text(
                                        ico_rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        ico,
                                        egui::FontId::new(10.0, theme::phosphor()),
                                        if h { t.ink_strong } else { c },
                                    );
                                    cx += 13.0;
                                }
                            }
                            let tp = pos2(cx, pill_rect.center().y - name_galley.size().y / 2.0);
                            ui.painter().galley(tp, name_galley, if h { t.ink_strong } else { c });

                            if pill_resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                rule.tags.push(ext);
                                state.push_undo_snapshot();
                            }
                        }
                    });
                }

                widgets::hrule(ui);

                // 2. Create New Tag
                widgets::hud(ui, loc.inspector_tags_create_title, t.muted);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.new_tag_name)
                            .hint_text(loc.inspector_tags_name_placeholder)
                            .desired_width(140.0)
                            .min_size(vec2(0.0, 24.0))
                            .margin(vec2(6.0, 4.0)),
                    );

                    let palette = [
                        "#3B82F6", "#06B6D4", "#10B981", "#F59E0B",
                        "#EF4444", "#8B5CF6", "#EC4899", "#F97316", "#64748B",
                    ];
                    ui.add_space(2.0);
                    for hex in palette {
                        let c = theme::tag_color(&crate::models::RuleTag::new("").with_color(hex), t.accent_ink);
                        let is_active = state.new_tag_color.as_deref() == Some(hex);
                        let (swatch, resp) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
                        ui.painter().rect_filled(swatch, CornerRadius::same(3), c);
                        if is_active {
                            ui.painter().rect_stroke(
                                swatch.expand(1.5),
                                CornerRadius::same(4),
                                Stroke::new(1.5, t.ink_strong),
                                egui::StrokeKind::Outside,
                            );
                        }
                        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            state.new_tag_color = Some(hex.into());
                        }
                    }
                });

                ui.horizontal(|ui| {
                    let icons: [&str; 13] = [
                        "",
                        egui_phosphor::regular::TAG,
                        egui_phosphor::regular::STAR,
                        egui_phosphor::regular::HEART,
                        egui_phosphor::regular::GAME_CONTROLLER,
                        egui_phosphor::regular::DESKTOP,
                        egui_phosphor::regular::LIGHTNING,
                        egui_phosphor::regular::FIRE,
                        egui_phosphor::regular::BELL,
                        egui_phosphor::regular::SHIELD,
                        egui_phosphor::regular::CLOCK,
                        egui_phosphor::regular::FOLDER,
                        egui_phosphor::regular::GLOBE,
                    ];

                    for ico in icons {
                        let is_sel = if ico.is_empty() {
                            state.new_tag_icon.is_none()
                        } else {
                            state.new_tag_icon.as_deref() == Some(ico)
                        };
                        let (btn_rect, mut btn_resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
                        let hovered = btn_resp.hovered();
                        if is_sel {
                            ui.painter().rect_filled(btn_rect, CornerRadius::same(3), t.sunken);
                            ui.painter().rect_stroke(btn_rect, CornerRadius::same(3), Stroke::new(1.0, t.accent_ink), egui::StrokeKind::Inside);
                        } else if hovered {
                            ui.painter().rect_filled(btn_rect, CornerRadius::same(3), t.sunken.gamma_multiply(0.5));
                        }
                        if ico.is_empty() {
                            ui.painter().text(
                                btn_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                egui_phosphor::regular::MINUS,
                                egui::FontId::new(11.0, theme::phosphor()),
                                if is_sel { t.ink_strong } else { t.muted },
                            );
                            btn_resp = btn_resp.on_hover_text(loc.inspector_tags_no_icon);
                        } else {
                            ui.painter().text(
                                btn_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                ico,
                                egui::FontId::new(12.0, theme::phosphor()),
                                if is_sel { t.ink_strong } else { t.muted },
                            );
                        }
                        if btn_resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            state.new_tag_icon = if ico.is_empty() { None } else { Some(ico.into()) };
                        }
                    }

                    ui.add_space(8.0);

                    let can_add = !state.new_tag_name.trim().is_empty();
                    let btn_galley = widgets::caps(
                        ui,
                        loc.inspector_tags_btn_create,
                        9.5,
                        theme::mono_bold(),
                        0.5,
                        if can_add { t.invert_fg } else { t.faint },
                    );
                    let btn_w = btn_galley.size().x + 20.0;
                    let (btn_rect, btn_resp) = ui.allocate_exact_size(vec2(btn_w, 20.0), Sense::click());
                    let btn_hovered = btn_resp.hovered() && can_add;
                    let btn_fill = if can_add {
                        if btn_hovered { t.invert_bg.gamma_multiply(0.85) } else { t.invert_bg }
                    } else {
                        t.sunken
                    };
                    let btn_stroke = if can_add { Stroke::NONE } else { Stroke::new(1.0, t.line) };
                    ui.painter().rect(btn_rect, CornerRadius::same(3), btn_fill, btn_stroke, egui::StrokeKind::Inside);
                    widgets::paint_centred(ui, btn_rect, btn_galley, if can_add { t.invert_fg } else { t.faint });

                    if btn_resp.on_hover_cursor(if can_add { egui::CursorIcon::PointingHand } else { egui::CursorIcon::NotAllowed }).clicked() && can_add {
                        let new_tag = crate::models::RuleTag {
                            name: state.new_tag_name.trim().to_string(),
                            color: state.new_tag_color.clone(),
                            icon: state.new_tag_icon.clone(),
                        };
                        if !rule.tags.iter().any(|t| t.name.eq_ignore_ascii_case(&new_tag.name)) {
                            rule.tags.push(new_tag);
                            state.push_undo_snapshot();
                        }
                        state.new_tag_name.clear();
                    }
                });
            });
    }
}
