use egui::{CornerRadius, pos2, Rect, Sense, Stroke, Ui, vec2};
use crate::gui::components::icons::ProcessIconCache;
use crate::gui::components::widgets::{self, Glyph};
use crate::gui::theme::{self, Tokens};
use crate::i18n::Strings;
use crate::models::Rule;

use super::pipeline::render_track_pipeline;
use super::tags::{measure_tags_width, render_card_tags_at};
use super::{default_icon_glyph, RuleRowIntent};

#[allow(clippy::too_many_arguments)]
pub(super) fn render_track_row(
    ui: &mut Ui,
    icons: &mut ProcessIconCache,
    loc: &Strings,
    rule: &Rule,
    is_selected: bool,
    is_being_dragged: bool,
    is_dnd_allowed: bool,
    is_dnd_active: bool,
    exec_opt: Option<crate::ipc::ActiveRuleExecutionState>,
    is_test_running: bool,
    grip_tip: &str,
    intents: &mut Vec<RuleRowIntent>,
    t: &Tokens,
) -> (Rect, bool) {
    let is_manual = rule.trigger.as_ref().is_none_or(|t| matches!(t, crate::models::Trigger::Manual));
    let is_running = exec_opt.is_some() || is_test_running;

    if is_running {
        ui.ctx().request_repaint();
    }

    let stroke = if is_being_dragged {
        Stroke::new(1.5, t.accent_ink.gamma_multiply(0.7))
    } else if is_selected {
        Stroke::new(1.5, t.accent)
    } else if is_running {
        Stroke::new(1.5, t.good)
    } else if !rule.is_enabled && !is_manual {
        Stroke::new(1.0, t.line.gamma_multiply(0.4))
    } else {
        t.hairline()
    };

    let fill = if is_being_dragged {
        t.sunken.gamma_multiply(0.6)
    } else if !rule.is_enabled && !is_manual {
        t.bg.gamma_multiply(0.65)
    } else {
        t.bg
    };

    let row_inner_h = 28.0;

    let frame = egui::Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::ZERO)
        .inner_margin(egui::Margin::symmetric(10, 6));

    let mut grip_started = false;

    let response = frame.show(ui, |ui| {
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), row_inner_h),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.spacing_mut().item_spacing.y = 0.0;

                // 1. Drag & Drop handle on far left
                let grip_resp = widgets::drag_handle_custom_interactive(ui, 26.0, grip_tip, is_dnd_allowed);
                if grip_resp.drag_started() && is_dnd_allowed {
                    grip_started = true;
                }

                // 2. Rule Icon (28x28)
                let icon_size = vec2(28.0, 28.0);
                let (icon_rect, icon_resp) = ui.allocate_exact_size(icon_size, Sense::click());
                ui.painter().rect_filled(icon_rect, CornerRadius::ZERO, t.sunken);
                ui.painter().rect_stroke(icon_rect, CornerRadius::ZERO, t.hairline(), egui::StrokeKind::Inside);

                let mut loaded_texture = false;
                if let Some(path) = &rule.custom_icon_path {
                    if let Some(tex) = icons.get_icon_from_path(ui.ctx(), path) {
                        let size = tex.size_vec2();
                        let max_dim = size.x.max(size.y).max(1.0);
                        let scale = 18.0 / max_dim;
                        let render_size = vec2((size.x * scale).round().max(1.0), (size.y * scale).round().max(1.0));
                        let img_rect = Rect::from_center_size(icon_rect.center(), render_size);
                        ui.painter().image(
                            tex.id(),
                            img_rect,
                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                        loaded_texture = true;
                    }
                }

                if !loaded_texture {
                    let glyph = default_icon_glyph(rule);
                    let icon_color = if rule.is_enabled || is_manual { t.ink_strong } else { t.muted };
                    ui.painter().text(
                        icon_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        glyph,
                        egui::FontId::new(14.0, theme::phosphor()),
                        icon_color,
                    );
                }

                if icon_resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    intents.push(RuleRowIntent::Edit(rule.clone()));
                }

                // 3. Rule Name and optional Tag Pills (In a row, strictly fixed height 28px - never alters card size)
                let title_color = if rule.is_enabled || is_manual { t.ink_strong } else { t.muted };
                let title_font = egui::FontId::new(13.0, theme::semibold());

                let natural_title_w = ui.painter().layout_no_wrap(
                    rule.name.clone(),
                    title_font.clone(),
                    title_color,
                ).size().x;

                let has_tags = !rule.tags.is_empty();
                let max_title_w = if has_tags { 160.0 } else { 220.0 };
                let title_w = natural_title_w.min(max_title_w).max(40.0);

                let format = egui::TextFormat {
                    font_id: title_font,
                    color: title_color,
                    ..Default::default()
                };
                let mut job = egui::text::LayoutJob::single_section(rule.name.clone(), format);
                job.wrap = egui::text::TextWrapping::truncate_at_width(title_w);
                let title_galley = ui.painter().layout_job(job);
                let is_elided = title_galley.elided;

                let (name_rect, mut name_col_resp) = ui.allocate_exact_size(vec2(title_w, row_inner_h), Sense::click());
                name_col_resp = name_col_resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                if is_elided {
                    name_col_resp = name_col_resp.on_hover_text(&rule.name);
                }
                if name_col_resp.clicked() {
                    intents.push(RuleRowIntent::Edit(rule.clone()));
                }

                let title_pos = pos2(name_rect.left(), name_rect.center().y - title_galley.size().y / 2.0);
                ui.painter().galley(title_pos, title_galley, title_color);

                if has_tags {
                    let tags_w = measure_tags_width(ui, &rule.tags).min(180.0);
                    let (tags_rect, _) = ui.allocate_exact_size(vec2(tags_w, row_inner_h), Sense::hover());
                    let pill_h = 17.0;
                    let pill_y = tags_rect.center().y - pill_h / 2.0;
                    render_card_tags_at(
                        ui,
                        &rule.tags,
                        tags_rect.left(),
                        pill_y,
                        tags_w,
                        &rule.id,
                        t,
                    );
                }

                // Spacing from title/tags to trigger badge (symmetric 8px margin on both sides of pipeline container)
                const PIPELINE_SIDE_MARGIN: f32 = 8.0;
                let left_extra_gap = (PIPELINE_SIDE_MARGIN - ui.spacing().item_spacing.x).max(0.0);
                if left_extra_gap > 0.0 {
                    ui.add_space(left_extra_gap);
                }

                // 4. Center Pipeline Stream in Horizontal ScrollArea (Pixel-perfect vertical centering)
                // 6 action buttons (26px each) + 5 gaps (4px each) = 176px.
                // Toggle switch (38px) + add_space (4px) + 2 gaps (8px) = 50px.
                // Symmetric PIPELINE_SIDE_MARGIN (8px) matches the gap on the left of the pipeline container.
                const BTNS_W: f32 = 6.0 * 26.0 + 5.0 * 4.0; // 176.0
                const TOGGLE_W: f32 = 38.0 + 4.0 + 4.0 + 4.0; // 50.0
                let controls_w = if is_manual { BTNS_W } else { BTNS_W + TOGGLE_W };
                let right_controls_w = controls_w + PIPELINE_SIDE_MARGIN;
                let center_w = (ui.available_width() - right_controls_w).max(40.0);

                ui.allocate_ui_with_layout(
                    vec2(center_w, row_inner_h),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.style_mut().always_scroll_the_only_direction = true;
                        ui.spacing_mut().scroll.floating = true;
                        ui.spacing_mut().scroll.floating_allocated_width = 0.0;
                        ui.spacing_mut().scroll.floating_width = 4.0;
                        ui.spacing_mut().scroll.bar_width = 8.0;
                        ui.spacing_mut().scroll.bar_inner_margin = 0.0;
                        ui.spacing_mut().scroll.bar_outer_margin = 0.0;
                        ui.spacing_mut().scroll.handle_min_length = 24.0;

                        let scroll_source = egui::containers::scroll_area::ScrollSource {
                            scroll_bar: !is_dnd_active,
                            drag: if is_dnd_active {
                                egui::containers::scroll_area::DragScroll::Never
                            } else {
                                egui::containers::scroll_area::DragScroll::Always
                            },
                            mouse_wheel: !is_dnd_active,
                        };

                        egui::ScrollArea::horizontal()
                            .id_salt(("rule_pipe_scroll", &rule.id))
                            .auto_shrink([false, false])
                            .wheel_scroll_multiplier(vec2(1.5, 1.0))
                            .scroll_source(scroll_source)
                            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                            .show(ui, |ui| {
                                ui.allocate_ui_with_layout(
                                    vec2(0.0, row_inner_h),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.spacing_mut().item_spacing.x = 6.0;
                                        ui.spacing_mut().item_spacing.y = 0.0;
                                        render_track_pipeline(ui, loc, rule, exec_opt, is_test_running, t);
                                    },
                                );
                            });
                    },
                );

                // 5. Far Right Controls: TOGGLE SWITCH (far right) + DELETE + SHARE + CLONE + EDIT + RUN
                ui.allocate_ui_with_layout(
                    vec2(ui.available_width(), row_inner_h),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;

                        // 1. Pause / Resume Toggle Switch (first item in right_to_left = placed at far right)
                        if !is_manual {
                            let mut is_on = rule.is_enabled;
                            let switch_tip = if is_on { loc.rules_btn_pause } else { loc.rules_btn_resume };
                            if widgets::toggle_switch(ui, &mut is_on).on_hover_text(switch_tip).changed() {
                                intents.push(RuleRowIntent::Toggle { id: rule.id.clone(), enabled: is_on });
                            }
                            ui.add_space(4.0);
                        }

                        // 2. Delete
                        if widgets::glyph_button_custom(ui, Glyph::Trash, 26.0, false, loc.rules_btn_delete).clicked() {
                            intents.push(RuleRowIntent::Delete(rule.id.clone()));
                        }

                        // 3. Share
                        if widgets::glyph_button_custom(ui, Glyph::Share, 26.0, false, loc.rules_btn_copy_single_tip).clicked() {
                            intents.push(RuleRowIntent::Share(rule.clone()));
                        }

                        // 3.5. Shortcut
                        if widgets::glyph_button_custom(ui, Glyph::Shortcut, 26.0, false, loc.rules_btn_create_shortcut_tip).clicked() {
                            intents.push(RuleRowIntent::Shortcut(rule.clone()));
                        }

                        // 4. Clone
                        if widgets::glyph_button_custom(ui, Glyph::Clone, 26.0, false, loc.rules_btn_clone).clicked() {
                            intents.push(RuleRowIntent::Clone(rule.clone()));
                        }

                        // 5. Edit
                        if widgets::glyph_button_custom(ui, Glyph::Edit, 26.0, false, loc.rules_btn_edit).clicked() {
                            intents.push(RuleRowIntent::Edit(rule.clone()));
                        }

                        // 6. Run / Stop
                        let is_stoppable = if is_manual { is_running } else { is_test_running };
                        let (run_glyph, run_tip) = if is_stoppable {
                            (Glyph::Stop, loc.rules_btn_stop)
                        } else if is_manual {
                            (Glyph::Play, loc.rules_btn_run_manual)
                        } else {
                            (Glyph::Play, loc.rules_btn_test_run_tip)
                        };
                        if widgets::glyph_button_custom(ui, run_glyph, 26.0, is_stoppable, run_tip).clicked() {
                            if is_stoppable {
                                intents.push(RuleRowIntent::StopRun(rule.id.clone()));
                            } else {
                                intents.push(RuleRowIntent::TestRun(rule.id.clone()));
                            }
                        }
                    },
                );
            },
        );
    });

    (response.response.rect, grip_started)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::state::GuiState;
    use crate::gui::views::rules_view::tests::wait_action;
    use crate::i18n::{self, Language};
    use crate::models::Trigger;

    #[test]
    fn test_track_row_rendering_no_panic_and_fixed_height() {
        let ctx = egui::Context::default();
        crate::gui::theme::install_fonts(&ctx);
        let mut state = GuiState::new();
        let tokens = Tokens::get(&ctx);
        let loc = i18n::get(Language::English);

        let tag_work = crate::models::RuleTag::new("Work").with_color("#3B82F6");
        let rule_with_tags = Rule {
            id: "r_tags".into(),
            name: "Rule with Tags".into(),
            is_enabled: true,
            tags: vec![tag_work],
            actions: vec![wait_action()],
            ..Default::default()
        };

        let rule_no_tags = Rule {
            id: "r_no_tags".into(),
            name: "Rule without Tags".into(),
            is_enabled: false,
            actions: vec![wait_action()],
            ..Default::default()
        };

        let rule_manual = Rule {
            id: "r_manual".into(),
            name: "Manual Rule".into(),
            trigger: Some(Trigger::Manual),
            actions: vec![wait_action()],
            ..Default::default()
        };

        let mut rect_with_tags = Rect::NOTHING;
        let mut rect_no_tags = Rect::NOTHING;
        let mut rect_manual = Rect::NOTHING;
        let mut intents: Vec<RuleRowIntent> = Vec::new();

        let mut out = ctx.run_ui(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.set_width(1000.0);
                let (r1, _) = render_track_row(
                    ui, &mut state.icon_cache, loc, &rule_with_tags, false, false, true, false,
                    None, false, loc.rules_tip_drag_rule, &mut intents, &tokens,
                );
                rect_with_tags = r1;
                let (r2, _) = render_track_row(
                    ui, &mut state.icon_cache, loc, &rule_no_tags, false, false, true, false,
                    None, false, loc.rules_tip_drag_rule, &mut intents, &tokens,
                );
                rect_no_tags = r2;
                let (r3, _) = render_track_row(
                    ui, &mut state.icon_cache, loc, &rule_manual, false, false, true, false,
                    None, false, loc.rules_tip_drag_rule, &mut intents, &tokens,
                );
                rect_manual = r3;
            });
        });
        out.textures_delta.clear();

        assert_eq!(rect_with_tags.height(), rect_no_tags.height(), "Card height must remain strictly fixed and identical regardless of tags presence");
        assert_eq!(rect_with_tags.height(), rect_manual.height(), "Card height must remain strictly fixed and identical for manual rules");
        assert!(rect_with_tags.height() > 0.0);
        assert!(intents.is_empty(), "rendering alone must not mutate rule state");
    }
}
