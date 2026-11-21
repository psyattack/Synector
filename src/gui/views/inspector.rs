//! Inspector Drawer: Slides in from the right edge with cubic easing,
//! dimmed background scrim, and craft-launcher styling for editing rules
//! and process details.

use egui::{
    Color32, CornerRadius, Id, LayerId, Order, Pos2, Rect, RichText, Sense, Stroke, Ui, emath::easing, pos2, vec2,
};
use crate::gui::components::script_highlighter::{create_script_layouter, ScriptLanguage};
use crate::gui::components::widgets::{self, Cell, Glyph, Kind};
use crate::gui::state::GuiState;
use crate::gui::theme::{self, Tokens};
use crate::models::{
    Action, ConditionOperator, PowerPlanType, ProcessMatchMode,
    ProcessPriorityLevel, ProcessSnapshotItem, QuotaPeriod, QuotaUnit, Rule, RuleCondition, RuleConditionItem, ScheduleType,
    SystemEventType, SystemPowerType, Trigger, WindowActionType,
};
use chrono::{Datelike, Timelike};
use std::collections::HashMap;
use std::time::Instant;


fn egui_key_to_hotkey_name(key: egui::Key) -> Option<&'static str> {
    match key {
        egui::Key::A => Some("A"),
        egui::Key::B => Some("B"),
        egui::Key::C => Some("C"),
        egui::Key::D => Some("D"),
        egui::Key::E => Some("E"),
        egui::Key::F => Some("F"),
        egui::Key::G => Some("G"),
        egui::Key::H => Some("H"),
        egui::Key::I => Some("I"),
        egui::Key::J => Some("J"),
        egui::Key::K => Some("K"),
        egui::Key::L => Some("L"),
        egui::Key::M => Some("M"),
        egui::Key::N => Some("N"),
        egui::Key::O => Some("O"),
        egui::Key::P => Some("P"),
        egui::Key::Q => Some("Q"),
        egui::Key::R => Some("R"),
        egui::Key::S => Some("S"),
        egui::Key::T => Some("T"),
        egui::Key::U => Some("U"),
        egui::Key::V => Some("V"),
        egui::Key::W => Some("W"),
        egui::Key::X => Some("X"),
        egui::Key::Y => Some("Y"),
        egui::Key::Z => Some("Z"),
        egui::Key::Num0 => Some("0"),
        egui::Key::Num1 => Some("1"),
        egui::Key::Num2 => Some("2"),
        egui::Key::Num3 => Some("3"),
        egui::Key::Num4 => Some("4"),
        egui::Key::Num5 => Some("5"),
        egui::Key::Num6 => Some("6"),
        egui::Key::Num7 => Some("7"),
        egui::Key::Num8 => Some("8"),
        egui::Key::Num9 => Some("9"),
        egui::Key::F1 => Some("F1"),
        egui::Key::F2 => Some("F2"),
        egui::Key::F3 => Some("F3"),
        egui::Key::F4 => Some("F4"),
        egui::Key::F5 => Some("F5"),
        egui::Key::F6 => Some("F6"),
        egui::Key::F7 => Some("F7"),
        egui::Key::F8 => Some("F8"),
        egui::Key::F9 => Some("F9"),
        egui::Key::F10 => Some("F10"),
        egui::Key::F11 => Some("F11"),
        egui::Key::F12 => Some("F12"),
        egui::Key::Enter => Some("Enter"),
        egui::Key::Tab => Some("Tab"),
        egui::Key::Space => Some("Space"),
        egui::Key::Backspace => Some("Backspace"),
        egui::Key::Delete => Some("Delete"),
        egui::Key::Insert => Some("Insert"),
        egui::Key::Home => Some("Home"),
        egui::Key::End => Some("End"),
        egui::Key::PageUp => Some("PageUp"),
        egui::Key::PageDown => Some("PageDown"),
        egui::Key::ArrowUp => Some("Up"),
        egui::Key::ArrowDown => Some("Down"),
        egui::Key::ArrowLeft => Some("Left"),
        egui::Key::ArrowRight => Some("Right"),
        _ => None,
    }
}

fn format_hotkey_combination(key: egui::Key, modifiers: egui::Modifiers) -> Option<String> {
    let key_name = egui_key_to_hotkey_name(key)?;
    let mut parts = Vec::new();
    if modifiers.ctrl || modifiers.command {
        parts.push("Ctrl");
    }
    if modifiers.alt {
        parts.push("Alt");
    }
    if modifiers.shift {
        parts.push("Shift");
    }
    if modifiers.mac_cmd {
        parts.push("Win");
    }
    parts.push(key_name);
    Some(parts.join("+"))
}

/// Snapshot captured at the start of an edit gesture (a drag, a typing burst or a series of
/// clicks) and committed once the gesture stops changing the draft.
#[derive(Clone, Default)]
struct PendingUndo {
    rule_id: String,
    before: Rule,
}

fn pending_undo_id() -> Id {
    Id::unique("inspector-pending-undo")
}

fn clear_pending_undo(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove_temp::<PendingUndo>(pending_undo_id()));
}

/// Commits a pending gesture snapshot immediately, e.g. before Ctrl+Z so the undo lands on
/// the state the gesture started from.
fn flush_pending_undo(ctx: &egui::Context, state: &mut GuiState) {
    if let Some(pending) = ctx.data(|data| data.get_temp::<PendingUndo>(pending_undo_id())) {
        state.push_undo_snapshot_of(&pending.before);
        clear_pending_undo(ctx);
    }
}

/// Frame-level undo bookkeeping.
///
/// The first changed frame records the pre-change rule; later changed frames keep that
/// snapshot. The first frame without changes (the gesture has stopped) pushes it once, so a
/// drag or a typing burst costs one undo entry instead of one per frame.
fn settle_pending_undo(
    ctx: &egui::Context,
    state: &mut GuiState,
    pre_rule: &Rule,
    rule: &Rule,
    undid_or_redid: bool,
) {
    if undid_or_redid {
        // `flush_pending_undo` already ran before the undo/redo.
        return;
    }

    let changed = rule != pre_rule;
    match ctx.data(|data| data.get_temp::<PendingUndo>(pending_undo_id())) {
        Some(pending) if pending.rule_id != rule.id => {
            // The editor switched rules; the old snapshot belongs to a closed draft.
            clear_pending_undo(ctx);
        }
        Some(pending) => {
            if !changed {
                state.push_undo_snapshot_of(&pending.before);
                clear_pending_undo(ctx);
            }
        }
        None if changed => {
            let pending = PendingUndo {
                rule_id: rule.id.clone(),
                before: pre_rule.clone(),
            };
            ctx.data_mut(|data| data.insert_temp(pending_undo_id(), pending));
        }
        None => {}
    }
}

/// Validation errors from the last failed save, kept per rule so switching drafts cannot
/// show another rule's errors.
#[derive(Clone, Default)]
struct SaveValidationErrors {
    rule_id: String,
    messages: Vec<String>,
}

fn save_errors_id() -> Id {
    Id::unique("inspector-save-errors")
}

fn load_save_errors(ctx: &egui::Context, rule_id: &str) -> Vec<String> {
    ctx.data(|data| data.get_temp::<SaveValidationErrors>(save_errors_id()))
        .filter(|errors| errors.rule_id == rule_id)
        .map(|errors| errors.messages)
        .unwrap_or_default()
}

fn store_save_errors(ctx: &egui::Context, rule_id: &str, messages: Vec<String>) {
    if messages.is_empty() {
        ctx.data_mut(|data| data.remove_temp::<SaveValidationErrors>(save_errors_id()));
    } else {
        ctx.data_mut(|data| {
            data.insert_temp(
                save_errors_id(),
                SaveValidationErrors {
                    rule_id: rule_id.to_string(),
                    messages,
                },
            )
        });
    }
}

pub fn render_inspector_drawer(ctx: &egui::Context, state: &mut GuiState) {
    let t = Tokens::get(ctx);

    let is_open = state.editing_rule.is_some() || state.selected_process.is_some();

    // A closed editor must not leave a gesture snapshot or save errors behind for a later rule.
    if state.editing_rule.is_none() {
        clear_pending_undo(ctx);
        ctx.data_mut(|data| data.remove_temp::<SaveValidationErrors>(save_errors_id()));
    }

    // Handle Esc key to close inspector (only if child modals/overlays are NOT open)
    if is_open
        && !state.is_modal_open()
        && ctx.input(|i| i.key_pressed(egui::Key::Escape))
    {
        state.close_editor();
        state.selected_process = None;
    }

    let k = ctx.animate_bool_with_time_and_easing(Id::unique("inspector-slide"), is_open, 0.22, easing::cubic_out);
    if k <= 0.001 && !is_open {
        return;
    }

    let full = ctx.content_rect();
    let top = (full.top() + theme::HEADER_H).min(full.bottom());
    let bottom = (full.bottom() - theme::FOOTER_H).max(top);
    let area = Rect::from_min_max(pos2(full.left(), top), pos2(full.right(), bottom));

    let width = (area.width() * 0.42).clamp(460.0, 720.0).min(area.width());
    let left = area.right() - width * k;

    // 1. Scrim dimming over background (strictly to the left of the drawer)
    let scrim_w = (left - area.left()).max(0.0);
    if scrim_w > 0.0 {
        egui::Area::new(Id::unique("inspector-scrim"))
            .order(Order::Middle)
            .fixed_pos(area.min)
            .show(ctx, |ui| {
                let scrim_size = vec2(scrim_w, area.height());
                let (rect, resp) = ui.allocate_exact_size(scrim_size, Sense::click());
                ui.painter().rect_filled(rect, CornerRadius::ZERO, theme::scrim(&t, k));
                if resp.on_hover_cursor(egui::CursorIcon::Default).clicked() {
                    state.close_editor();
                    state.selected_process = None;
                }
            });
    }

    // 2. Sliding Drawer Area
    egui::Area::new(Id::unique("inspector-drawer"))
        .order(Order::Middle)
        .fixed_pos(pos2(left, area.top()))
        .constrain(false)
        .interactable(!state.is_modal_open())
        .show(ctx, |ui| {
            let panel = Rect::from_min_size(pos2(left, area.top()), vec2(width, area.height()));
            let p = ui.painter();
            p.add(theme::shadow(&t).as_shape(panel, CornerRadius::ZERO));
            p.rect_filled(panel, CornerRadius::ZERO, t.bg);
            p.vline(panel.left() + 0.5, panel.y_range(), Stroke::new(1.0, t.line_strong));

            ui.set_min_size(panel.size());
            ui.set_max_size(panel.size());
            ui.set_width(width);
            ui.set_max_width(width);
            ui.set_clip_rect(panel);

            // 1. Top Header Strip (only for selected_process inspection)
            let mut close_process = false;

            let header_resp = if let Some(proc) = &state.selected_process {
                let proc_name = proc.name.clone();
                let proc_pid = proc.pid;
                Some(egui::Frame::new()
                    .fill(t.sunken)
                    .stroke(t.hairline())
                    .corner_radius(CornerRadius::ZERO)
                    .inner_margin(egui::Margin {
                        left: 20,
                        right: 14,
                        top: 3,
                        bottom: 3,
                    })
                    .show(ui, |ui| {
                        let loc = state.loc();
                        ui.spacing_mut().interact_size.y = 0.0;
                        ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
                        ui.set_width(ui.available_width());
                        ui.set_max_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.spacing_mut().interact_size.y = 0.0;
                            ui.set_width(ui.available_width());
                            ui.vertical(|ui| {
                                ui.spacing_mut().interact_size.y = 0.0;
                                ui.spacing_mut().item_spacing.y = 1.0;
                                widgets::hud(ui, loc.inspector_eyebrow_task_details, t.muted);
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().interact_size.y = 0.0;
                                    ui.spacing_mut().item_spacing.x = 6.0;
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(&proc_name)
                                                .family(theme::semibold())
                                                .size(14.5)
                                                .color(t.ink_strong),
                                        )
                                        .truncate(),
                                    );
                                    widgets::hud(ui, &format!("#{}", proc_pid), t.muted);
                                });
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if widgets::glyph_button(ui, Glyph::Close, loc.inspector_btn_close_tip).clicked() {
                                    close_process = true;
                                }
                            });
                        });
                    }))
            } else {
                None
            };

            if close_process {
                state.selected_process = None;
            }

            let header_bottom = header_resp.map(|r| r.response.rect.bottom()).unwrap_or(panel.top());
            let is_editing_rule = state.editing_rule.is_some();
            let save_errors = state
                .editing_rule
                .as_ref()
                .map(|rule| load_save_errors(ctx, &rule.id))
                .unwrap_or_default();
            // The footer grows with the validation lines so the drawer error list is never clipped.
            let footer_h = if is_editing_rule {
                52.0 + if save_errors.is_empty() {
                    0.0
                } else {
                    18.0 + save_errors.len() as f32 * 15.0
                }
            } else {
                0.0
            };
            let body_rect = Rect::from_min_max(
                pos2(panel.left(), header_bottom),
                pos2(panel.right(), (panel.bottom() - footer_h).max(header_bottom)),
            );
            let footer_rect = Rect::from_min_max(
                pos2(panel.left(), panel.bottom() - footer_h),
                panel.max,
            );

            // 2. Scrollable Body with strictly pinned width and height
            ui.scope_builder(egui::UiBuilder::new().max_rect(body_rect), |ui| {
                ui.set_width(body_rect.width());
                ui.set_max_width(body_rect.width());
                ui.set_height(body_rect.height());
                ui.set_max_height(body_rect.height());

                ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
                egui::ScrollArea::vertical()
                    .id_salt("inspector_scroll_area")
                    .max_width(body_rect.width())
                    .max_height(body_rect.height())
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.set_max_width(ui.available_width());

                        // Keyboard shortcuts for Undo (Ctrl+Z) and Redo (Ctrl+Y or Ctrl+Shift+Z)
                        let (do_undo, do_redo) = ui.input_mut(|i| {
                            let u = i.consume_key(egui::Modifiers::CTRL, egui::Key::Z)
                                || i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z);
                            let r = i.consume_key(egui::Modifiers::CTRL, egui::Key::Y)
                                || i.consume_key(egui::Modifiers::COMMAND, egui::Key::Y)
                                || i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::Z)
                                || i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Z);
                            (u, r)
                        });

                        let mut undid_or_redid = false;
                        if do_undo {
                            flush_pending_undo(ui.ctx(), state);
                            state.undo();
                            undid_or_redid = true;
                        } else if do_redo {
                            flush_pending_undo(ui.ctx(), state);
                            state.redo();
                            undid_or_redid = true;
                        }

                        if let Some(mut rule) = state.editing_rule.clone() {
                            let pre_rule = rule.clone();
                            render_rule_editor(ui, state, &mut rule, &t);
                            if state.editing_rule.is_some() {
                                settle_pending_undo(ui.ctx(), state, &pre_rule, &rule, undid_or_redid);
                                state.editing_rule = Some(rule);
                            }
                        } else if let Some(proc) = state.selected_process.clone() {
                            render_process_inspector(ui, state, &proc, &t);
                        }
                    });
            });

            // 3. Fixed Bottom Action Strip (Save / Cancel / Clone always visible and strictly pinned to drawer bottom)
            if let Some(rule) = state.editing_rule.clone() {
                ui.scope_builder(egui::UiBuilder::new().max_rect(footer_rect), |ui| {
                    egui::Frame::new()
                        .fill(t.sunken)
                        .stroke(t.hairline())
                        .corner_radius(CornerRadius::ZERO)
                        .inner_margin(egui::Margin::symmetric(20, 10))
                        .show(ui, |ui| {
                            let loc = state.loc();
                            ui.set_width(ui.available_width());
                            ui.set_max_width(ui.available_width());
                            if !save_errors.is_empty() {
                                ui.label(
                                    egui::RichText::new(loc.inspector_save_errors_title)
                                        .size(11.0)
                                        .color(t.bad),
                                );
                                for message in &save_errors {
                                    ui.label(
                                        egui::RichText::new(format!("• {message}"))
                                            .size(10.5)
                                            .color(t.bad),
                                    );
                                }
                                ui.add_space(2.0);
                            }
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;

                                if widgets::button(ui, loc.inspector_btn_save_rule, Kind::Primary, 120.0).clicked() {
                                    // Same contract as `--rules-apply`: validate the draft with the
                                    // shared model validator and keep the editor open on errors.
                                    let mut candidate = rule.clone();
                                    if candidate.name.trim().is_empty() {
                                        candidate.name = loc.rules_default_new_name.to_string();
                                    }
                                    let errors = crate::cli::validate_rules(std::slice::from_ref(&candidate));
                                    if errors.is_empty() {
                                        store_save_errors(ui.ctx(), &rule.id, Vec::new());
                                        state.save_rule(candidate);
                                    } else {
                                        store_save_errors(ui.ctx(), &rule.id, errors);
                                    }
                                }

                                if !state.is_creating_new_rule
                                    && widgets::button(ui, loc.inspector_btn_clone, Kind::Secondary, 0.0)
                                        .clicked()
                                {
                                    state.clone_rule(&rule);
                                }

                                if state.is_creating_new_rule
                                    && widgets::button(ui, loc.inspector_btn_reset, Kind::Secondary, 0.0)
                                        .clicked()
                                {
                                    state.reset_builder_state();
                                }

                                if widgets::button(ui, loc.inspector_btn_cancel, Kind::Secondary, 0.0).clicked() {
                                    state.close_editor();
                                }

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if widgets::glyph_button(ui, Glyph::Question, loc.footer_guide_tip).clicked() {
                                        state.show_guide_modal = true;
                                    }
                                });
                            });
                        });
                });
            }
        });
}

fn quota_period_label(period: Option<QuotaPeriod>, loc: &crate::i18n::Strings) -> String {
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

fn quota_unit_label(unit: QuotaUnit, loc: &crate::i18n::Strings) -> &'static str {
    match unit {
        QuotaUnit::Minutes => loc.inspector_quota_unit_minutes,
        QuotaUnit::Hours => loc.inspector_quota_unit_hours,
        QuotaUnit::Days => loc.inspector_quota_unit_days,
        QuotaUnit::Weeks => loc.inspector_quota_unit_weeks,
        QuotaUnit::Months => loc.inspector_quota_unit_months,
    }
}

/// Left-hand form label centred against a standard-height control row.
fn quota_form_label(ui: &mut Ui, text: &str, t: &Tokens) -> egui::Response {
    let row_h = ui.spacing().interact_size.y;
    ui.allocate_ui_with_layout(
        vec2(58.0, row_h),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| widgets::hud(ui, text, t.muted),
    )
    .inner
}

fn render_rule_editor(ui: &mut Ui, state: &mut GuiState, rule: &mut Rule, t: &Tokens) {
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
                                let glyph = crate::gui::views::rules_view::default_icon_glyph(rule);
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

            // 2. Trigger Section
            widgets::eyebrow(ui, "02", loc.inspector_eyebrow_trigger);

            let mut switch_trigger_to_hunter = false;

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

                        let (glyph, title, glyph_color) = match &rule.trigger {
                            Some(Trigger::Process { .. }) => (egui_phosphor::regular::CPU, loc.inspector_trigger_process, t.good),
                            Some(Trigger::WindowFocus { .. }) => (egui_phosphor::regular::APP_WINDOW, loc.inspector_trigger_window, t.accent_ink),
                            Some(Trigger::Schedule { schedule_type, .. }) => match schedule_type {
                                ScheduleType::AppStartup => (egui_phosphor::regular::ROCKET_LAUNCH, loc.inspector_schedule_app_start, t.good),
                                ScheduleType::AppShutdown => (egui_phosphor::regular::POWER, loc.inspector_schedule_app_close, t.warn),
                                ScheduleType::Interval => (egui_phosphor::regular::TIMER, loc.inspector_sched_tab_interval, t.accent_ink),
                                ScheduleType::Daily => (egui_phosphor::regular::CALENDAR, loc.inspector_sched_tab_daily, t.good),
                                ScheduleType::OneShot => (egui_phosphor::regular::CALENDAR_CHECK, loc.inspector_sched_tab_specific, t.muted),
                                ScheduleType::Monthly => (egui_phosphor::regular::CALENDAR_BLANK, loc.inspector_sched_tab_monthly, t.accent_ink),
                            },
                            Some(Trigger::SystemEvent { event_type, .. }) => {
                                let meta = system_event_meta(*event_type, loc, t);
                                (meta.glyph, meta.label, meta.tint)
                            },
                            Some(Trigger::ResourceThreshold { .. }) => (egui_phosphor::regular::CHART_LINE, loc.inspector_trigger_resource, t.warn),
                            Some(Trigger::GlobalHotkey { .. }) => (egui_phosphor::regular::KEYBOARD, loc.inspector_trigger_hotkey, t.accent_ink),
                            Some(Trigger::FileWatcher { .. }) => (egui_phosphor::regular::FILE, loc.inspector_trigger_file, t.accent_ink),
                            Some(Trigger::Webhook { .. }) => (egui_phosphor::regular::GLOBE, loc.inspector_trigger_webhook, t.good),
                            Some(Trigger::Manual) | None => (egui_phosphor::regular::PLAY_CIRCLE, loc.inspector_manual_title, t.accent_ink),
                        };

                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(glyph)
                                    .family(theme::phosphor())
                                    .size(16.0)
                                    .color(glyph_color),
                            );
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(title)
                                    .family(theme::semibold())
                                    .size(12.5)
                                    .color(t.ink),
                            );

                            let exec_opt = state.get_rule_execution_state(rule);
                            let is_running = exec_opt.is_some();
                            let is_test_running = state.active_test_runs.contains_key(&rule.id);
                            let is_stoppable = if is_manual { is_running } else { is_test_running };
                            let run_label = if is_stoppable { loc.rules_btn_stop } else { loc.rules_btn_test_run };
                            let run_kind = if is_stoppable { Kind::Danger } else { Kind::Secondary };

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if widgets::button(ui, loc.inspector_btn_change_trigger, Kind::Secondary, 0.0).clicked() {
                                    state.open_trigger_catalog();
                                }
                                if widgets::button(ui, run_label, run_kind, 0.0).clicked() {
                                    if is_stoppable {
                                        state.stop_rule_execution(&rule.id);
                                    } else {
                                        state.test_run_rule(&rule.id);
                                    }
                                }
                            });
                        });

                        widgets::hrule(ui);

                        if is_manual {
                            render_trigger_info_frame(ui, egui_phosphor::regular::INFO, loc.inspector_manual_desc, t.accent_ink, t);
                        } else {
                            match &mut rule.trigger {
                            Some(Trigger::Process { process_name, match_mode, single_instance }) => {
                                        widgets::hud(ui, loc.inspector_field_target_process, t.muted);
                                        if render_process_name_row(ui, loc, process_name, loc.inspector_field_target_process_hint) {
                                            switch_trigger_to_hunter = true;
                                        }

                                        ui.add_space(6.0);
                                        widgets::hud(ui, loc.inspector_field_match_mode, t.muted);
                                        let match_options = [
                                            (ProcessMatchMode::Exact, Cell::new(loc.inspector_match_tab_exact, "")),
                                            (ProcessMatchMode::Wildcard, Cell::new(loc.inspector_match_tab_wildcard, "")),
                                            (ProcessMatchMode::Regex, Cell::new(loc.inspector_match_tab_regex, "")),
                                        ];
                                        widgets::cells(ui, match_mode, &match_options);

                                        ui.add_space(6.0);
                                        widgets::checkbox(ui, single_instance, loc.inspector_checkbox_single_instance);
                            }

                            Some(Trigger::WindowFocus { process_name, on_focus_gained, on_focus_lost }) => {
                                widgets::hud(ui, loc.inspector_field_target_process, t.muted);
                                if render_process_name_row(ui, loc, process_name, loc.inspector_field_target_process_hint) {
                                    switch_trigger_to_hunter = true;
                                }

                                ui.add_space(4.0);
                                widgets::hud(ui, loc.inspector_field_sys_event_type, t.muted);
                                widgets::checkbox_group(ui, |ui| {
                                    widgets::checkbox_row(ui, |ui| {
                                        if ui.checkbox(on_focus_gained, loc.inspector_focus_gained).changed()
                                            && !*on_focus_gained && !*on_focus_lost
                                        {
                                            *on_focus_lost = true;
                                        }
                                        if ui.checkbox(on_focus_lost, loc.inspector_focus_lost).changed()
                                            && !*on_focus_lost && !*on_focus_gained
                                        {
                                            *on_focus_gained = true;
                                        }
                                    });
                                });
                            }

                            Some(Trigger::Schedule { schedule_type, interval_sec, daily_time, days_of_week, day_of_month, target_datetime }) => {
                                match schedule_type {
                                    ScheduleType::AppStartup | ScheduleType::AppShutdown => {
                                        let mut is_startup = *schedule_type == ScheduleType::AppStartup;
                                        let options = [
                                            (true, Cell::new(loc.inspector_schedule_app_start, "")),
                                            (false, Cell::new(loc.inspector_schedule_app_close, "")),
                                        ];
                                        if widgets::cells(ui, &mut is_startup, &options) {
                                            *schedule_type = if is_startup { ScheduleType::AppStartup } else { ScheduleType::AppShutdown };
                                        }
                                        ui.add_space(4.0);

                                        if *schedule_type == ScheduleType::AppStartup {
                                            render_trigger_info_frame(
                                                ui,
                                                egui_phosphor::regular::ROCKET_LAUNCH,
                                                loc.inspector_schedule_app_start_desc,
                                                t.good,
                                                t,
                                            );
                                        } else {
                                            render_trigger_info_frame(
                                                ui,
                                                egui_phosphor::regular::POWER,
                                                loc.inspector_schedule_app_close_desc,
                                                t.warn,
                                                t,
                                            );
                                        }
                                    }
                                    _ => {
                                        render_schedule_trigger(
                                            ui,
                                            loc,
                                            *schedule_type,
                                            interval_sec,
                                            daily_time,
                                            days_of_week,
                                            day_of_month,
                                            target_datetime,
                                            t,
                                        );
                                    }
                                }
                            }

                            Some(Trigger::SystemEvent { event_type, .. }) => {
                                render_system_event_pair_toggle(ui, event_type, loc, t);
                                let meta = system_event_meta(*event_type, loc, t);
                                render_trigger_info_frame(ui, meta.glyph, meta.description, meta.tint, t);
                            }
                            Some(Trigger::ResourceThreshold { process_name, cpu_percent, ram_mb, duration_sec }) => {
                                widgets::hud(ui, loc.inspector_field_target_process, t.muted);
                                if render_process_name_row(ui, loc, process_name, loc.inspector_hint_process_executable) {
                                    switch_trigger_to_hunter = true;
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

                                ui.add_space(4.0);
                                ui.horizontal_wrapped(|ui| {
                                    widgets::hud(ui, loc.inspector_cond_res_duration, t.muted);
                                    widgets::duration_secs_u64(ui, loc, "trigger_res_duration", duration_sec, 1.0..=3600.0);
                                });
                            }

                            Some(Trigger::GlobalHotkey { key_combination, hold_mode, repeat_while_held }) => {
                                widgets::hud(ui, loc.inspector_field_hotkey_combo, t.muted);
                                let rec_id = ui.scope_id().with("rec_global_hotkey");
                                        let mut is_recording: bool = ui.data_mut(|d| d.get_temp(rec_id).unwrap_or(false));

                                        if is_recording {
                                            ui.ctx().input(|i| {
                                                for event in &i.raw.events {
                                                    match event {
                                                        egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } => {
                                                            if *key == egui::Key::Escape {
                                                                is_recording = false;
                                                                break;
                                                            }
                                                            if let Some(combo) = format_hotkey_combination(*key, *modifiers) {
                                                                *key_combination = combo;
                                                                is_recording = false;
                                                                break;
                                                            }
                                                        }
                                                        egui::Event::PointerButton { button, pressed: true, modifiers, .. } => {
                                                            let btn_str = match button {
                                                                egui::PointerButton::Primary => "LButton",
                                                                egui::PointerButton::Secondary => "RButton",
                                                                egui::PointerButton::Middle => "MButton",
                                                                egui::PointerButton::Extra1 => "M4",
                                                                egui::PointerButton::Extra2 => "M5",
                                                            };
                                                            let mut parts = Vec::new();
                                                            if modifiers.ctrl || modifiers.command { parts.push("Ctrl"); }
                                                            if modifiers.alt { parts.push("Alt"); }
                                                            if modifiers.shift { parts.push("Shift"); }
                                                            parts.push(btn_str);
                                                            *key_combination = parts.join("+");
                                                            is_recording = false;
                                                            break;
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                            });
                                            ui.data_mut(|d| d.insert_temp(rec_id, is_recording));
                                            ui.ctx().request_repaint();
                                        }

                                        ui.horizontal(|ui| {
                                            ui.set_width(ui.available_width());
                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                let rec_label = if is_recording { loc.inspector_hotkey_listening } else { loc.inspector_hotkey_record };
                                                let rec_kind = if is_recording { Kind::Primary } else { Kind::Secondary };
                                                if widgets::button(ui, rec_label, rec_kind, 0.0)
                                                    .on_hover_text(loc.inspector_hotkey_record_tip)
                                                    .clicked()
                                                {
                                                    is_recording = !is_recording;
                                                    ui.data_mut(|d| d.insert_temp(rec_id, is_recording));
                                                }
                                                ui.add(
                                                    egui::TextEdit::singleline(key_combination)
                                                        .hint_text(loc.inspector_hint_hotkey_combo)
                                                        .desired_width(ui.available_width())
                                                        .min_size(vec2(0.0, theme::CONTROL_H))
                                                        .align(egui::Align2::LEFT_CENTER)
                                                        .margin(vec2(10.0, 8.0)),
                                                );
                                            });
                                        });

                                        ui.add_space(4.0);
                                        widgets::checkbox_group(ui, |ui| {
                                            ui.horizontal_wrapped(|ui| {
                                                ui.checkbox(hold_mode, loc.inspector_hotkey_hold_mode)
                                                    .on_hover_text(loc.inspector_hotkey_hold_mode_tip);
                                                if *hold_mode {
                                                    ui.checkbox(repeat_while_held, loc.inspector_hotkey_repeat_while_held)
                                                        .on_hover_text(loc.inspector_hotkey_repeat_while_held_tip);
                                                } else {
                                                    ui.checkbox(repeat_while_held, loc.inspector_hotkey_repeat_in_loop)
                                                        .on_hover_text(loc.inspector_hotkey_repeat_in_loop_tip);
                                                }
                                            });
                                        });
                                    }
                            Some(Trigger::FileWatcher { folder_path, filter_extension, on_created, on_modified, on_deleted }) => {
                                widgets::hud(ui, loc.inspector_field_watch_path, t.muted);
                                ui.horizontal(|ui| {
                                    ui.set_width(ui.available_width());
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if widgets::button(ui, loc.inspector_btn_choose_file, Kind::Secondary, 0.0).clicked() {
                                            if let Some(f) = rfd::FileDialog::new().pick_file() {
                                                *folder_path = f.to_string_lossy().to_string();
                                            }
                                        }
                                        if widgets::button(ui, loc.inspector_btn_choose_folder, Kind::Secondary, 0.0).clicked() {
                                            if let Some(f) = rfd::FileDialog::new().pick_folder() {
                                                *folder_path = f.to_string_lossy().to_string();
                                            }
                                        }
                                        ui.add(
                                            egui::TextEdit::singleline(folder_path)
                                                .hint_text(loc.inspector_hint_watch_path)
                                                .desired_width(ui.available_width())
                                                .min_size(vec2(0.0, theme::CONTROL_H))
                                                .align(egui::Align2::LEFT_CENTER)
                                                .margin(vec2(10.0, 8.0)),
                                        );
                                    });
                                });

                                ui.add_space(4.0);
                                widgets::hud(ui, loc.inspector_field_filter_ext, t.muted);
                                let mut ext = filter_extension.clone().unwrap_or_default();
                                let max_w = ui.available_width().min(240.0);
                                if ui.add(
                                    egui::TextEdit::singleline(&mut ext)
                                        .hint_text(loc.inspector_hint_filter_ext)
                                        .desired_width(max_w)
                                        .min_size(vec2(0.0, theme::CONTROL_H))
                                        .align(egui::Align2::LEFT_CENTER)
                                        .margin(vec2(10.0, 8.0)),
                                ).changed() {
                                    *filter_extension = if ext.trim().is_empty() { None } else { Some(ext.trim().to_string()) };
                                }

                                ui.add_space(4.0);
                                widgets::checkbox_group(ui, |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.checkbox(on_created, loc.inspector_file_event_created);
                                        ui.checkbox(on_modified, loc.inspector_file_event_modified);
                                        ui.checkbox(on_deleted, loc.inspector_file_event_deleted);
                                    });
                                });
                            }

                            Some(Trigger::Webhook { endpoint_path }) => {
                                widgets::hud(ui, loc.inspector_field_webhook_endpoint, t.muted);
                                ui.add(
                                    egui::TextEdit::singleline(endpoint_path)
                                        .hint_text(loc.inspector_hint_webhook_endpoint)
                                        .desired_width(ui.available_width())
                                        .min_size(vec2(0.0, theme::CONTROL_H))
                                        .align(egui::Align2::LEFT_CENTER)
                                        .margin(vec2(10.0, 8.0)),
                                );

                                let endpoint_display = if endpoint_path.trim().is_empty() {
                                    rule.id.as_str()
                                } else {
                                    endpoint_path.as_str()
                                };
                                let full_url = format!("http://127.0.0.1:48123/api/v1/trigger/{}", endpoint_display.trim_start_matches('/'));

                                ui.add_space(6.0);
                                let url_label = egui::RichText::new(&full_url)
                                    .font(egui::FontId::new(12.0, theme::mono()))
                                    .color(t.accent_ink)
                                    .underline();
                                let resp = ui.add(egui::Label::new(url_label).sense(egui::Sense::click()))
                                    .on_hover_text(loc.inspector_webhook_copy_url)
                                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                                if resp.clicked() {
                                    crate::sharing::copy_text_to_clipboard(ui.ctx(), &full_url);
                                    state.set_toast(loc.toast_url_copied);
                                }
                            }

                            Some(Trigger::Manual) | None => {}
                        }
                    }
                });
            });

            if switch_trigger_to_hunter {
                state.open_process_picker(crate::gui::state::ProcessPickerTarget::Trigger);
            }

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

fn render_add_action_button(
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
    p.rect_stroke(rect, CornerRadius::same(2), border, egui::StrokeKind::Inside);

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

#[derive(Clone, Copy, Debug, Default)]
struct DndDragSession {
    source_idx: usize,
    target_idx: usize,
    card_width: f32,
}

/// Action to render as the floating drag preview.
///
/// `source_idx` was captured in an earlier frame; an undo/redo can replace the draft with a
/// shorter snapshot while the session is still alive, so the index is resolved with `get`.
/// A session that no longer fits is dropped instead of panicking on the stale index.
fn dnd_preview_action<'a>(
    actions: &'a [Action],
    session: &mut Option<DndDragSession>,
) -> Option<&'a Action> {
    let source_idx = session.as_ref()?.source_idx;
    let action = actions.get(source_idx);
    if action.is_none() {
        *session = None;
    }
    action
}

fn action_type_badge(action: &Action, loc: &crate::i18n::Strings) -> &'static str {
    match action {
        Action::StartProcess { close_on_exit, .. } => {
            if *close_on_exit {
                loc.inspector_badge_launch_companion
            } else {
                loc.inspector_badge_launch_process
            }
        }
        Action::KillProcess { .. } => loc.inspector_badge_kill_process,
        Action::Notification { .. } => loc.inspector_badge_notification,
        Action::SystemPower { .. } => loc.inspector_badge_system_power,
        Action::Wait { .. } => loc.inspector_badge_wait,
        Action::Check { .. } => loc.inspector_badge_check,
        Action::HttpWebhook { .. } => loc.inspector_badge_webhook,
        Action::Window { .. } => loc.inspector_badge_window,
        Action::ProcessTuning { .. } => loc.inspector_badge_tuning,
        Action::PowerPlan { .. } => loc.inspector_badge_power_plan,
        Action::PlaySound { .. } => loc.inspector_badge_play_sound,
        Action::Firewall { .. } => loc.inspector_badge_firewall,
        Action::Display { .. } => loc.inspector_badge_display,
        Action::AudioControl { .. } => loc.inspector_badge_audio_control,
        Action::DefaultDevice { .. } => loc.inspector_badge_default_device,
        Action::ExecuteScript { .. } => loc.inspector_badge_execute_script,
        Action::Macro { .. } => loc.inspector_badge_macro,
        Action::IfElse { .. } => loc.inspector_badge_if_else,
        Action::AudioFormat { .. } => loc.inspector_badge_audio_format,
        Action::TrimWorkingSet { .. } => loc.inspector_badge_trim_working_set,
        Action::DiscordRpcSet { .. } => loc.inspector_badge_discord_rpc_set,
        Action::DiscordRpcClear => loc.inspector_badge_discord_rpc_clear,
        Action::Archive { .. } => loc.inspector_badge_archive,
        Action::FileSystem { .. } => loc.inspector_badge_file_system,
        Action::Overlay { .. } => loc.inspector_badge_overlay,
        Action::RuleControl { .. } => loc.inspector_badge_rule_control,
    }
}

/// Fills the `{}` placeholders of a catalog template left to right.
///
/// `str::replace` would fill every occurrence with the same value, so templates with more
/// than one placeholder are filled positionally instead.
fn fill(template: &str, args: &[&str]) -> String {
    let mut out = String::new();
    let mut rest = template;
    for arg in args {
        match rest.find("{}") {
            Some(at) => {
                out.push_str(&rest[..at]);
                out.push_str(arg);
                rest = &rest[at + 2..];
            }
            None => break,
        }
    }
    out.push_str(rest);
    out
}

/// Localized window-operation label, shared by the combo box and the summary line.
fn window_action_label(action: &WindowActionType, loc: &crate::i18n::Strings) -> &'static str {
    match action {
        WindowActionType::Minimize => loc.inspector_win_action_minimize,
        WindowActionType::Maximize => loc.inspector_win_action_maximize,
        WindowActionType::Restore => loc.inspector_win_action_restore,
        WindowActionType::ToggleMinimizeRestore => loc.inspector_win_toggle_min_restore,
        WindowActionType::Hide => loc.inspector_win_action_hide,
        WindowActionType::Show => loc.inspector_win_action_show,
        WindowActionType::ToggleShowHide => loc.inspector_win_toggle_show_hide,
        WindowActionType::Close => loc.inspector_win_action_close,
        WindowActionType::AlwaysOnTop => loc.inspector_win_action_always_on_top,
        WindowActionType::RemoveAlwaysOnTop => loc.inspector_win_action_remove_topmost,
        WindowActionType::ToggleAlwaysOnTop => loc.inspector_win_action_toggle_topmost,
        WindowActionType::SetTransparency => loc.inspector_win_action_transparency,
        WindowActionType::SnapToPosition => loc.inspector_win_action_position,
    }
}

/// Localized process-priority label, shared by the combo box and its option rows.
fn priority_label(priority: &Option<ProcessPriorityLevel>, loc: &crate::i18n::Strings) -> &'static str {
    match priority {
        None => loc.inspector_priority_unchanged,
        Some(ProcessPriorityLevel::Idle) => loc.inspector_priority_idle,
        Some(ProcessPriorityLevel::BelowNormal) => loc.inspector_priority_below_normal,
        Some(ProcessPriorityLevel::Normal) => loc.inspector_priority_normal,
        Some(ProcessPriorityLevel::AboveNormal) => loc.inspector_priority_above_normal,
        Some(ProcessPriorityLevel::High) => loc.inspector_priority_high,
        Some(ProcessPriorityLevel::Realtime) => loc.inspector_priority_realtime,
    }
}

/// One-line summary of an action for the floating drag preview. Every word comes from the
/// catalog; only separators, units and the macro markers stay in the format strings.
fn action_summary_preview(action: &Action, loc: &crate::i18n::Strings) -> String {
    match action {
        Action::StartProcess { file_path, .. } => {
            let name = std::path::Path::new(file_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(file_path);
            if name.is_empty() {
                loc.summary_preview_launch_empty.to_string()
            } else {
                fill(loc.summary_preview_launch, &[name])
            }
        }
        Action::KillProcess { process_name, .. } => fill(loc.summary_preview_kill, &[process_name.as_str()]),
        Action::Notification { title, .. } => {
            if title.is_empty() {
                loc.inspector_menu_notification.to_string()
            } else {
                fill(loc.summary_preview_notify, &[title.as_str()])
            }
        }
        Action::SystemPower { power_type } => {
            let power = match power_type {
                SystemPowerType::Lock => loc.inspector_power_lock,
                SystemPowerType::Sleep => loc.inspector_power_sleep,
                SystemPowerType::TurnScreenOff => loc.inspector_power_screen_off,
                SystemPowerType::Restart => loc.inspector_power_restart,
                SystemPowerType::Shutdown => loc.inspector_power_shutdown,
            };
            fill(loc.summary_preview_power, &[power])
        }
        Action::Wait { mode, seconds, condition, timeout_sec, .. } => match mode {
            crate::models::WaitMode::Timer => {
                let seconds_txt = format!("{:.1}", seconds);
                fill(loc.summary_preview_wait_timer, &[seconds_txt.as_str()])
            }
            crate::models::WaitMode::UntilCondition => match condition {
                Some(crate::models::WaitCondition::WindowOpened { process_name, .. }) => {
                    fill(loc.summary_preview_wait_window_open, &[process_name.as_str()])
                }
                Some(crate::models::WaitCondition::WindowClosed { process_name, .. }) => {
                    fill(loc.summary_preview_wait_window_close, &[process_name.as_str()])
                }
                Some(crate::models::WaitCondition::ProcessStarted { process_name, .. }) => {
                    fill(loc.summary_preview_wait_process_start, &[process_name.as_str()])
                }
                Some(crate::models::WaitCondition::ProcessExited { process_name, .. }) => {
                    fill(loc.summary_preview_wait_process_exit, &[process_name.as_str()])
                }
                Some(crate::models::WaitCondition::Hotkey { key_combination }) => {
                    fill(loc.summary_preview_wait_hotkey, &[key_combination.as_str()])
                }
                Some(crate::models::WaitCondition::File { path, condition }) => match condition {
                    crate::models::WaitFileCondition::Exists => fill(loc.summary_preview_wait_file_exists, &[path.as_str()]),
                    crate::models::WaitFileCondition::Deleted => fill(loc.summary_preview_wait_file_deleted, &[path.as_str()]),
                },
                Some(crate::models::WaitCondition::RuleStarted { rule_id }) => {
                    fill(loc.summary_preview_wait_rule_start, &[rule_id.as_str()])
                }
                Some(crate::models::WaitCondition::RuleStopped { rule_id }) => {
                    fill(loc.summary_preview_wait_rule_stop, &[rule_id.as_str()])
                }
                None => {
                    let timeout_txt = format!("{:.0}", timeout_sec);
                    fill(loc.summary_preview_wait_condition, &[timeout_txt.as_str()])
                }
            },
        },
        Action::Check { condition } => match condition {
            crate::models::RuleCondition::ProcessRunning { process_name, must_be_running, .. } => {
                if *must_be_running {
                    fill(loc.summary_preview_check_running, &[process_name.as_str()])
                } else {
                    fill(loc.summary_preview_check_not_running, &[process_name.as_str()])
                }
            }
            crate::models::RuleCondition::WindowFocused { process_name, .. } => {
                fill(loc.summary_preview_check_window, &[process_name.as_str()])
            }
            crate::models::RuleCondition::FileWatcher { path, must_exist, .. } => {
                if *must_exist {
                    fill(loc.summary_preview_check_file_exists, &[path.as_str()])
                } else {
                    fill(loc.summary_preview_check_file_deleted, &[path.as_str()])
                }
            }
            crate::models::RuleCondition::RuleRunning { rule_id, must_be_running } => {
                if *must_be_running {
                    fill(loc.summary_preview_check_rule_running, &[rule_id.as_str()])
                } else {
                    fill(loc.summary_preview_check_rule_stopped, &[rule_id.as_str()])
                }
            }
            _ => loc.catalog_title_check.to_string(),
        },
        Action::HttpWebhook { url, method, .. } => {
            if url.is_empty() {
                loc.inspector_trigger_webhook.to_string()
            } else {
                format!("{} {}", method, url)
            }
        }
        Action::Window { window_action, target_process, .. } => {
            let proc = target_process.as_deref().unwrap_or(loc.summary_preview_active_window);
            match window_action {
                WindowActionType::ToggleMinimizeRestore => fill(loc.summary_preview_window_toggle_min, &[proc]),
                WindowActionType::ToggleShowHide => fill(loc.summary_preview_window_toggle_show, &[proc]),
                other => fill(loc.summary_preview_window, &[window_action_label(other, loc), proc]),
            }
        }
        Action::ProcessTuning { target_process, .. } => {
            let proc = target_process.as_deref().unwrap_or(loc.summary_preview_trigger_process);
            fill(loc.summary_preview_tuning, &[proc])
        }
        Action::PowerPlan { plan } => {
            let plan_label = match plan {
                PowerPlanType::HighPerformance => loc.inspector_power_plan_high,
                PowerPlanType::Balanced => loc.inspector_power_plan_balanced,
                PowerPlanType::PowerSaver => loc.inspector_power_plan_saver,
            };
            fill(loc.summary_preview_power_plan, &[plan_label])
        }
        Action::PlaySound { file_path, play_beep, beep_frequency, beep_duration_ms } => {
            if *play_beep {
                if let Some(f) = beep_frequency {
                    let freq_txt = f.to_string();
                    let duration_txt = beep_duration_ms.unwrap_or(200).to_string();
                    fill(loc.summary_preview_beep, &[freq_txt.as_str(), duration_txt.as_str()])
                } else {
                    loc.inspector_audio_beep.to_string()
                }
            } else {
                let f = std::path::Path::new(file_path).file_name().and_then(|s| s.to_str()).unwrap_or(file_path);
                fill(loc.summary_preview_sound, &[f])
            }
        }
        Action::Firewall { process_name, action } => match action {
            crate::models::FirewallAction::Block => fill(loc.summary_preview_firewall_block, &[process_name.as_str()]),
            crate::models::FirewallAction::Allow => fill(loc.summary_preview_firewall_allow, &[process_name.as_str()]),
            crate::models::FirewallAction::Toggle => fill(loc.summary_preview_firewall_toggle, &[process_name.as_str()]),
        },
        Action::Display { width, height, refresh_rate, monitor_index } => {
            let res = match (width, height) {
                (Some(w), Some(h)) => format!("{}x{}", w, h),
                _ => loc.summary_preview_auto_resolution.to_string(),
            };
            let mon = match monitor_index {
                Some(m) => {
                    let m_txt = m.to_string();
                    format!("{} ", fill(loc.summary_monitor_prefix, &[m_txt.as_str()]))
                }
                None => String::new(),
            };
            match refresh_rate {
                Some(r) => {
                    let rate_txt = r.to_string();
                    fill(loc.summary_preview_display_refresh, &[mon.as_str(), res.as_str(), rate_txt.as_str()])
                }
                None => fill(loc.summary_preview_display, &[mon.as_str(), res.as_str()]),
            }
        }
        Action::AudioControl { target, action_type, volume_level, left_volume, right_volume, process_name } => {
            let target_str = match target {
                crate::models::AudioTarget::Playback => loc.summary_audio_target_playback,
                crate::models::AudioTarget::Microphone => loc.summary_audio_target_mic,
                crate::models::AudioTarget::Process => loc.summary_audio_target_process,
            };
            match action_type {
                crate::models::AudioVolumeAction::Mute => fill(loc.summary_preview_audio_mute, &[target_str]),
                crate::models::AudioVolumeAction::Unmute => fill(loc.summary_preview_audio_unmute, &[target_str]),
                crate::models::AudioVolumeAction::ToggleMute => fill(loc.summary_preview_audio_toggle, &[target_str]),
                crate::models::AudioVolumeAction::SetVolume => {
                    if let (Some(l), Some(r)) = (left_volume, right_volume) {
                        let left_txt = l.to_string();
                        let right_txt = r.to_string();
                        fill(loc.summary_preview_audio_balance, &[target_str, left_txt.as_str(), right_txt.as_str()])
                    } else if let Some(p) = process_name {
                        let volume_txt = volume_level.to_string();
                        fill(
                            loc.summary_preview_audio_process_volume,
                            &[target_str, p.as_str(), volume_txt.as_str()],
                        )
                    } else {
                        let volume_txt = volume_level.to_string();
                        fill(loc.summary_preview_audio_volume, &[target_str, volume_txt.as_str()])
                    }
                }
                crate::models::AudioVolumeAction::PlayBeep => loc.summary_preview_audio_play_beep.to_string(),
            }
        }
        Action::AudioFormat { target, sample_rate, bit_depth, channels, .. } => {
            let target_str = match target {
                crate::models::AudioTarget::Playback | crate::models::AudioTarget::Process => loc.summary_audio_output,
                crate::models::AudioTarget::Microphone => loc.summary_audio_input,
            };
            let ch_str = if *channels == 1 {
                loc.inspector_audio_channels_mono
            } else {
                loc.inspector_audio_channels_stereo
            };
            let rate_txt = sample_rate.to_string();
            let depth_txt = bit_depth.to_string();
            fill(
                loc.summary_preview_audio_format,
                &[target_str, rate_txt.as_str(), depth_txt.as_str(), ch_str],
            )
        }
        Action::DefaultDevice { target, device_name } => {
            let pfx = match target {
                crate::models::AudioTarget::Playback => loc.summary_device_output,
                crate::models::AudioTarget::Microphone => loc.summary_audio_target_mic,
                crate::models::AudioTarget::Process => loc.summary_device_process,
            };
            fill(loc.summary_preview_default_device, &[pfx, device_name.as_str()])
        }
        Action::ExecuteScript { interpreter, script, .. } => {
            let name = match interpreter {
                crate::models::ScriptType::PowerShell => loc.inspector_script_type_powershell,
                crate::models::ScriptType::Batch => loc.inspector_script_type_cmd,
                crate::models::ScriptType::Lua => loc.inspector_script_type_lua,
            };
            let first = script.lines().next().unwrap_or("").trim();
            if first.is_empty() {
                fill(loc.summary_preview_execute, &[name])
            } else {
                format!("{}: {}", name, first.chars().take(35).collect::<String>())
            }
        }
        Action::Macro { preset_id, operation, events, repeat, speed, infinite } => {
            let pfx = match operation {
                crate::models::MacroOperation::Run => loc.inspector_macro_op_run,
                crate::models::MacroOperation::Stop => loc.inspector_macro_op_stop,
                crate::models::MacroOperation::Toggle => loc.inspector_macro_op_toggle,
            };
            if *operation == crate::models::MacroOperation::Stop {
                if let Some(pid) = preset_id {
                    format!("{}: {}", pfx, pid)
                } else {
                    loc.summary_preview_macro_stop_all.to_string()
                }
            } else if let Some(pid) = preset_id {
                let rep_str = if *infinite {
                    " ∞".to_string()
                } else if *repeat > 1 {
                    format!(" x{}", repeat)
                } else {
                    String::new()
                };
                let spd_str = if (*speed - 1.0).abs() > 0.05 { format!(" @{:.1}x", speed) } else { String::new() };
                format!("{}: {}{}{}", pfx, pid, rep_str, spd_str)
            } else {
                let rep_str = if *infinite {
                    " ∞".to_string()
                } else if *repeat > 1 {
                    format!(" x{}", repeat)
                } else {
                    String::new()
                };
                let spd_str = if (*speed - 1.0).abs() > 0.05 { format!(" @{:.1}x", speed) } else { String::new() };
                let count_txt = events.len().to_string();
                let events_txt = fill(loc.summary_preview_macro_events, &[count_txt.as_str()]);
                format!("{}: {}{}{}", pfx, events_txt, rep_str, spd_str)
            }
        }
        Action::IfElse { conditions, then_actions, else_if_branches, else_actions } => {
            let glyph = conditions.first().map(|c| c.condition.icon_glyph()).unwrap_or("❓");
            let count_txt = conditions.len().to_string();
            let then_txt = then_actions.len().to_string();
            let else_txt = else_actions.len().to_string();
            if else_if_branches.is_empty() {
                fill(
                    loc.summary_preview_ifelse,
                    &[glyph, count_txt.as_str(), then_txt.as_str(), else_txt.as_str()],
                )
            } else {
                let elif_txt = else_if_branches.len().to_string();
                fill(
                    loc.summary_preview_ifelse_elif,
                    &[glyph, count_txt.as_str(), then_txt.as_str(), elif_txt.as_str(), else_txt.as_str()],
                )
            }
        }
        Action::TrimWorkingSet { target_process, soft_limit_mb, .. } => {
            let proc = target_process.as_deref().unwrap_or(loc.summary_preview_active_process);
            match soft_limit_mb {
                Some(mb) => {
                    let limit_txt = mb.to_string();
                    fill(loc.summary_preview_trim_limit, &[proc, limit_txt.as_str()])
                }
                None => fill(loc.summary_preview_trim, &[proc]),
            }
        }
        Action::DiscordRpcSet { details, state, .. } => {
            if details.is_empty() && state.is_empty() {
                loc.summary_preview_discord.to_string()
            } else if !details.is_empty() && !state.is_empty() {
                fill(loc.summary_preview_discord_set, &[details.as_str(), state.as_str()])
            } else {
                // No dedicated one-sided template: the badge token is the shared prefix.
                format!("{}: {}{}", loc.inspector_badge_discord_rpc_set, details, state)
            }
        }
        Action::DiscordRpcClear => loc.summary_preview_discord_clear.to_string(),
        Action::Archive { operation, format, destination_path, .. } => {
            let fname = std::path::Path::new(destination_path).file_name().unwrap_or_default().to_string_lossy();
            let op = match operation {
                crate::models::ArchiveOperation::Pack => loc.inspector_archive_op_pack,
                crate::models::ArchiveOperation::Unpack => loc.inspector_archive_op_unpack,
            };
            format!("{} {} -> {}", op, archive_format_label(*format, loc), fname)
        }
        Action::FileSystem { operation, path, .. } => {
            let fname = std::path::Path::new(path).file_name().unwrap_or_default().to_string_lossy();
            let op = match operation {
                crate::models::FileSystemOperation::CreateFile => loc.inspector_fs_op_create_file,
                crate::models::FileSystemOperation::CreateDirectory => loc.inspector_fs_op_create_dir,
                crate::models::FileSystemOperation::Rename => loc.inspector_fs_op_rename,
                crate::models::FileSystemOperation::Copy => loc.inspector_fs_op_copy,
                crate::models::FileSystemOperation::Delete => loc.inspector_fs_op_delete,
                crate::models::FileSystemOperation::WriteText => loc.inspector_fs_op_write_text,
                crate::models::FileSystemOperation::AppendText => loc.inspector_fs_op_append_text,
            };
            format!("{}: {}", op, fname)
        }
        Action::Overlay { overlay_id, operation, override_text, .. } => {
            let op_str = match operation {
                crate::models::OverlayOperation::Show => loc.inspector_overlay_op_show,
                crate::models::OverlayOperation::Hide => loc.inspector_overlay_op_hide,
                crate::models::OverlayOperation::Toggle => loc.summary_overlay_op_toggle,
                crate::models::OverlayOperation::Flash => loc.summary_overlay_op_flash,
                crate::models::OverlayOperation::SetText => loc.inspector_overlay_op_set_text,
            };
            if let Some(txt) = override_text {
                fill(loc.summary_preview_overlay_text, &[op_str, overlay_id.as_str(), txt.as_str()])
            } else {
                fill(loc.summary_preview_overlay, &[op_str, overlay_id.as_str()])
            }
        }
        Action::RuleControl { rule_id, operation, wait } => match operation {
            crate::models::RuleControlOperation::Call => {
                if *wait {
                    fill(loc.summary_preview_rule_call_wait, &[rule_id.as_str()])
                } else {
                    fill(loc.summary_preview_rule_call, &[rule_id.as_str()])
                }
            }
            crate::models::RuleControlOperation::Stop => fill(loc.summary_preview_rule_stop, &[rule_id.as_str()]),
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn render_drop_slot(
    ui: &mut Ui,
    _loc: &crate::i18n::Strings,
    slot_idx: usize,
    active_dst: Option<usize>,
    active_src: Option<usize>,
    pipeline_id: &str,
    prefix: &str,
    t: &Tokens,
) {
    let is_targeted = if let (Some(dst), Some(src)) = (active_dst, active_src) {
        dst == slot_idx && slot_idx != src && slot_idx != src + 1
    } else {
        false
    };

    let anim_id = ui.scope_id().with(("slot_anim", pipeline_id, slot_idx));
    let anim = ui.ctx().animate_bool_with_time_and_easing(anim_id, is_targeted, 0.18, easing::cubic_out);

    if anim > 0.001 {
        let slot_h = 22.0 * anim;
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), slot_h), Sense::hover());
        let p = ui.painter();
        let mid_y = rect.center().y;

        // Background subtle accent tint
        p.rect_filled(
            rect,
            CornerRadius::same(2),
            t.accent_ink.gamma_multiply(0.08 * anim),
        );

        // Glowing laser line
        let line_stroke = Stroke::new(1.8 * anim, t.accent_ink.gamma_multiply(anim));
        p.hline(rect.left() + 24.0..=rect.right() - 8.0, mid_y, line_stroke);

        // Left indicator bead with halo
        let bead_pos = pos2(rect.left() + 12.0, mid_y);
        p.circle_filled(bead_pos, 3.5 * anim, t.accent_ink.gamma_multiply(anim));
        p.circle_stroke(
            bead_pos,
            5.5 * anim,
            Stroke::new(1.0, t.accent_ink.gamma_multiply(0.4 * anim)),
        );

        // Step number label at the right of the drop line
        let step_num = if let Some(src) = active_src {
            if slot_idx > src {
                slot_idx
            } else {
                slot_idx + 1
            }
        } else {
            slot_idx + 1
        };
        let label = format!("{} {:02}", prefix, step_num);
        let text_color = t.accent_ink.gamma_multiply(anim);
        let galley = widgets::caps(ui, &label, 9.0, theme::mono_bold(), 1.0, text_color);
        let text_pos = pos2(rect.right() - galley.size().x - 12.0, mid_y - galley.size().y * 0.5);
        p.galley_with_override_text_color(text_pos, galley, text_color);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_floating_drag_preview(
    ui: &Ui,
    loc: &crate::i18n::Strings,
    action: &Action,
    session: DndDragSession,
    pointer_pos: Option<Pos2>,
    prefix: &str,
    pipeline_id: &str,
    t: &Tokens,
) {
    let Some(pointer) = pointer_pos else { return };

    let card_w = session.card_width.clamp(280.0, 520.0);
    let card_h = 56.0;
    let preview_pos = pos2(
        pointer.x - card_w * 0.5,
        pointer.y - card_h * 0.5,
    );
    let preview_rect = Rect::from_min_size(preview_pos, vec2(card_w, card_h));

    let painter = ui.ctx().layer_painter(LayerId::new(Order::Tooltip, Id::unique(("dnd_float_preview", pipeline_id))));

    // Elevated drop shadow
    painter.add(theme::shadow(t).as_shape(preview_rect, CornerRadius::same(2)));

    // Card background & vibrant border
    painter.rect_filled(preview_rect, CornerRadius::same(2), t.bg);
    painter.rect_stroke(
        preview_rect,
        CornerRadius::same(2),
        Stroke::new(1.5, t.accent_ink),
        egui::StrokeKind::Inside,
    );

    // Top accent stripe
    painter.hline(preview_rect.x_range(), preview_rect.top() + 1.0, Stroke::new(2.0, t.accent_ink));

    // Grip dots on the left of floating card
    let grip_m = pos2(preview_rect.left() + 14.0, preview_rect.center().y);
    for x in [-2.8, 2.8] {
        for y in [-4.0, 0.0, 4.0] {
            painter.circle_filled(grip_m + vec2(x, y), 1.2, t.accent_ink);
        }
    }

    // Step indicator: "STEP 01"
    let step_txt = format!("{} {:02}", prefix, session.source_idx + 1);
    let step_galley = widgets::caps(ui, &step_txt, 11.0, theme::mono_bold(), 1.0, t.accent_ink);
    let step_w = step_galley.size().x;
    painter.galley_with_override_text_color(
        pos2(preview_rect.left() + 26.0, preview_rect.top() + 8.0),
        step_galley,
        t.accent_ink,
    );

    // Type badge
    let type_badge = action_type_badge(action, loc);
    let badge_galley = widgets::caps(ui, type_badge, 10.0, theme::mono(), 1.0, t.ink_strong);
    let badge_rect = Rect::from_min_size(
        pos2(preview_rect.left() + 26.0 + step_w + 8.0, preview_rect.top() + 7.0),
        vec2(badge_galley.size().x + 10.0, 18.0),
    );
    painter.rect_filled(badge_rect, CornerRadius::ZERO, t.sunken);
    painter.rect_stroke(badge_rect, CornerRadius::ZERO, t.hairline(), egui::StrokeKind::Inside);
    painter.galley_with_override_text_color(
        badge_rect.center() - badge_galley.size() * 0.5,
        badge_galley,
        t.ink_strong,
    );

    // Live destination indicator on far right: "➜ STEP 02"
    let dest_num = if session.target_idx > session.source_idx {
        session.target_idx
    } else {
        session.target_idx + 1
    };
    let dest_txt = format!("➜ {} {:02}", prefix, dest_num);
    let dest_galley = widgets::caps(ui, &dest_txt, 10.0, theme::mono_bold(), 1.0, t.accent_ink);
    painter.galley_with_override_text_color(
        pos2(preview_rect.right() - dest_galley.size().x - 12.0, preview_rect.top() + 9.0),
        dest_galley,
        t.accent_ink,
    );

    // Action summary line
    let summary = action_summary_preview(action, loc);
    let summary_galley = ui.painter().layout_no_wrap(
        summary,
        egui::FontId::new(11.0, theme::mono()),
        t.muted,
    );
    painter.galley_with_override_text_color(
        pos2(preview_rect.left() + 26.0, preview_rect.top() + 32.0),
        summary_galley,
        t.muted,
    );
}

/// Renders one drag-and-drop action pipeline. The rule's main/exit pipelines and the nested
/// If/Else branches share it, so drop slots, edge auto-scroll, the floating preview, the
/// infinite-macro warning and unreachable-step dimming behave identically everywhere.
fn render_action_pipeline(
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
        let wheel_delta_y = ui.input(|i| i.smooth_scroll_delta.y);
        if wheel_delta_y != 0.0 {
            ui.scroll_with_delta(vec2(0.0, wheel_delta_y * 1.5));
            ui.ctx().request_repaint();
        }

        // 2. Edge auto-scrolling when pointer is near top or bottom of viewport
        if let Some(pointer) = pointer_pos {
            let viewport = ui.clip_rect();
            // Check if pointer is horizontally within or near the drawer column
            if pointer.x >= viewport.left() - 60.0 && pointer.x <= viewport.right() + 60.0 {
                let edge_zone = 90.0;
                let view_top = viewport.top();
                let view_bottom = viewport.bottom();

                // Near top edge: auto-scroll up
                if pointer.y < view_top + edge_zone {
                    let factor = ((view_top + edge_zone - pointer.y) / edge_zone).clamp(0.1, 3.0);
                    let scroll_speed = 8.0 + 26.0 * factor;
                    ui.scroll_with_delta(vec2(0.0, scroll_speed));
                    ui.ctx().request_repaint();
                }
                // Near bottom edge: auto-scroll down
                else if pointer.y > view_bottom - edge_zone {
                    let factor = ((pointer.y - (view_bottom - edge_zone)) / edge_zone).clamp(0.1, 3.0);
                    let scroll_speed = 8.0 + 26.0 * factor;
                    ui.scroll_with_delta(vec2(0.0, -scroll_speed));
                    ui.ctx().request_repaint();
                }
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
        let n = current_centers.len();
        if n > 0 {
            if pointer.y < current_centers[0] {
                session.target_idx = 0;
            } else if pointer.y >= current_centers[n - 1] {
                session.target_idx = n;
            } else {
                for i in 0..(n - 1) {
                    if pointer.y >= current_centers[i] && pointer.y < current_centers[i + 1] {
                        session.target_idx = i + 1;
                        break;
                    }
                }
            }
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
struct RulePipelineHost<'a> {
    state: &'a mut GuiState,
    t: &'a Tokens,
    is_exit_pipeline: bool,
    exec_opt: Option<crate::ipc::protocol::ActiveRuleExecutionState>,
    switch_to_hunter: Option<usize>,
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
struct PipelineCardOutcome {
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
trait PipelineHost {
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

#[derive(Default)]
struct ActionFieldsResponse {
    pub expand_script: Option<ScriptLanguage>,
    pub expand_macro: bool,
    pub expand_overlay: Option<String>,
    pub expand_overlay_action: Option<usize>,
    pub switch_to_hunter: bool,
    pub open_catalog_nested: Option<crate::gui::state::IfElseCatalogTarget>,
    pub open_condition_catalog: Option<crate::gui::state::ConditionCatalogTarget>,
    pub pick_target: Option<crate::gui::state::ProcessPickerTarget>,
    pub tested_overlay_ids: Vec<String>,
}

fn render_infinite_macro_warning(ui: &mut Ui, state: &GuiState, t: &Tokens) {
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
fn render_action_card(
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
                            let (ok, msg) = match action {
                                Action::Macro { events, speed, .. } => {
                                    if events.is_empty() {
                                        (false, loc.inspector_macro_test_empty.to_string())
                                    } else {
                                        crate::gui::macro_recorder::run_macro_test_safe(events.clone(), *speed);
                                        (true, loc.inspector_macro_testing_actions.replace("{}", &events.len().to_string()))
                                    }
                                }
                                _ => {
                                    let mut ctx = HashMap::new();
                                    ctx.insert("ProcessName".to_string(), "test.exe".to_string());
                                    ctx.insert("Timestamp".to_string(), chrono::Local::now().to_rfc3339());
                                    let res = crate::ipc::IpcClient::call(&crate::ipc::IpcRequest::TestStep {
                                        action: action.clone(),
                                        context: ctx,
                                    });
                                    let ok = matches!(res, Ok(crate::ipc::IpcResponse::TestStepResult(Ok(()))));
                                    let msg = match res {
                                        Ok(crate::ipc::IpcResponse::TestStepResult(Ok(()))) => loc.inspector_test_step_passed.to_string(),
                                        Ok(crate::ipc::IpcResponse::TestStepResult(Err(e))) => loc.inspector_test_step_failed.replace("{}", &e),
                                        Ok(crate::ipc::IpcResponse::Error(e)) => loc.inspector_test_step_error.replace("{}", &e),
                                        Err(e) => loc.inspector_test_step_ipc_error.replace("{}", &e.to_string()),
                                        _ => loc.inspector_test_step_unknown.to_string(),
                                    };
                                    (ok, msg)
                                }
                            };
                            let key = if is_exit_pipeline { idx + 1000 } else { idx };
                            ui.data_mut(|d| d.insert_temp(Id::unique(("test_step_res", key)), (ok, msg, Instant::now())));
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

#[derive(Clone)]
struct RuleChoice {
    id: String,
    name: String,
}

/// Rules offered as targets in the editor, including the rule being edited: a pre-condition
/// "this rule is not running" is a valid way to keep a rule single-instance.
fn rule_choices(state: &GuiState) -> Vec<RuleChoice> {
    let rules: Vec<Rule> = state
        .daemon_state
        .as_ref()
        .map(|daemon| daemon.config.rules.clone())
        .unwrap_or_else(|| {
            crate::config::RulesFile::read_from(&crate::config::AppConfig::config_dir()).rules
        });
    rules
        .into_iter()
        .filter(|rule| !rule.id.is_empty())
        .map(|rule| RuleChoice {
            id: rule.id,
            name: rule.name,
        })
        .collect()
}

fn render_rule_choice_combo(
    ui: &mut Ui,
    choices: &[RuleChoice],
    rule_id: &mut String,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    loc: &crate::i18n::Strings,
    t: &Tokens,
) {
    // An unset target is displayed as the first available rule but selected only by the
    // user: opening a rule must not write into the draft.
    let selected_label = match choices.iter().find(|choice| choice.id == *rule_id) {
        Some(choice) => choice.name.clone(),
        None => match (rule_id.is_empty(), choices.first()) {
            (true, Some(first)) => first.name.clone(),
            (true, None) => loc.inspector_rule_no_targets.to_string(),
            (false, _) => format!("{} ({})", loc.inspector_rule_missing, rule_id),
        },
    };

    widgets::hud(ui, loc.inspector_rule_target, t.muted);
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(RichText::new(selected_label).size(12.0))
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            if choices.is_empty() {
                ui.label(RichText::new(loc.inspector_rule_no_targets).size(12.0).color(t.muted));
            }
            for choice in choices {
                if ui.selectable_label(choice.id == *rule_id, &choice.name).clicked() {
                    *rule_id = choice.id.clone();
                }
            }
        });
}

/// Target-rule picker shared by both modes of `Action::RuleControl`. The edited rule itself is
/// hidden: a rule may stop itself, but offering it as a call target is only a trap.
fn render_rule_target_picker(
    ui: &mut Ui,
    state: &GuiState,
    rule_id: &mut String,
    idx: usize,
    is_exit_pipeline: bool,
    t: &Tokens,
) {
    let editing_id = state
        .editing_rule
        .as_ref()
        .map(|rule| rule.id.as_str())
        .unwrap_or_default();
    let choices: Vec<RuleChoice> = rule_choices(state)
        .into_iter()
        .filter(|choice| choice.id != editing_id)
        .collect();
    render_rule_choice_combo(
        ui,
        &choices,
        rule_id,
        ("rule_control_target", idx, is_exit_pipeline),
        state.loc(),
        t,
    );
}

/// Display label for every `ArchiveFormat` variant. Exhaustively matched so a new
/// model variant cannot be rendered without a label.
fn archive_format_label(format: crate::models::ArchiveFormat, loc: &crate::i18n::Strings) -> &'static str {
    use crate::models::ArchiveFormat;
    match format {
        ArchiveFormat::Zip => loc.inspector_archive_fmt_zip,
        ArchiveFormat::SevenZip => loc.inspector_archive_fmt_7z,
        ArchiveFormat::TarGz => loc.inspector_archive_fmt_tar_gz,
        ArchiveFormat::Tar => loc.inspector_archive_fmt_tar,
        ArchiveFormat::TarXz => loc.inspector_archive_fmt_tar_xz,
        ArchiveFormat::Rar => loc.inspector_archive_fmt_rar,
    }
}

/// Format cell for the archive editor: the label is localized and the tooltip is empty.
fn archive_format_cell(format: crate::models::ArchiveFormat, loc: &crate::i18n::Strings) -> Cell<'static> {
    Cell::new(archive_format_label(format, loc), "")
}

fn archive_format_ext(format: crate::models::ArchiveFormat) -> &'static str {
    use crate::models::ArchiveFormat;
    match format {
        ArchiveFormat::Zip => "zip",
        ArchiveFormat::SevenZip => "7z",
        ArchiveFormat::TarGz => "tar.gz",
        ArchiveFormat::Tar => "tar",
        ArchiveFormat::TarXz => "tar.xz",
        ArchiveFormat::Rar => "rar",
    }
}

/// Order used by the format cells; every variant appears exactly once.
const ARCHIVE_FORMAT_ORDER: [crate::models::ArchiveFormat; 6] = [
    crate::models::ArchiveFormat::Zip,
    crate::models::ArchiveFormat::SevenZip,
    crate::models::ArchiveFormat::TarGz,
    crate::models::ArchiveFormat::Tar,
    crate::models::ArchiveFormat::TarXz,
    crate::models::ArchiveFormat::Rar,
];

/// Shared inputs of the per-action field editors.
struct ActionFieldsCtx<'a> {
    state: &'a GuiState,
    t: &'a Tokens,
    idx: usize,
    is_exit_pipeline: bool,
    default_picker_target: crate::gui::state::ProcessPickerTarget,
    depth: usize,
}

#[allow(clippy::too_many_arguments)]
fn render_action_parameter_fields(
    ui: &mut Ui,
    state: &GuiState,
    action: &mut Action,
    idx: usize,
    is_exit_pipeline: bool,
    default_picker_target: crate::gui::state::ProcessPickerTarget,
    t: &Tokens,
    depth: usize,
) -> ActionFieldsResponse {
    let ctx = ActionFieldsCtx {
        state,
        t,
        idx,
        is_exit_pipeline,
        default_picker_target,
        depth,
    };
    render_action_fields(ui, &ctx, action)
}

/// Short dispatcher: one call per `Action` variant. Each variant keeps its own field
/// function so the layout, widget ids and defaults stay readable and reviewable.
fn render_action_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    action: &mut Action,
) -> ActionFieldsResponse {
    let mut resp = ActionFieldsResponse::default();
    match action {
        Action::StartProcess { .. } => render_start_process_fields(ui, ctx, &mut resp, action),
        Action::KillProcess { .. } => render_kill_process_fields(ui, ctx, &mut resp, action),
        Action::Notification { .. } => render_notification_fields(ui, ctx, &mut resp, action),
        Action::SystemPower { .. } => render_system_power_fields(ui, ctx, &mut resp, action),
        Action::Wait { .. } => render_wait_fields(ui, ctx, &mut resp, action),
        Action::Check { .. } => render_check_fields(ui, ctx, &mut resp, action),
        Action::HttpWebhook { .. } => render_http_webhook_fields(ui, ctx, &mut resp, action),
        Action::Window { .. } => render_window_fields(ui, ctx, &mut resp, action),
        Action::ProcessTuning { .. } => render_process_tuning_fields(ui, ctx, &mut resp, action),
        Action::PowerPlan { .. } => render_power_plan_fields(ui, ctx, &mut resp, action),
        Action::PlaySound { .. } => render_play_sound_fields(ui, ctx, &mut resp, action),
        Action::Firewall { .. } => render_firewall_fields(ui, ctx, &mut resp, action),
        Action::Display { .. } => render_display_fields(ui, ctx, &mut resp, action),
        Action::AudioControl { .. } => render_audio_control_fields(ui, ctx, &mut resp, action),
        Action::AudioFormat { .. } => render_audio_format_fields(ui, ctx, &mut resp, action),
        Action::DefaultDevice { .. } => render_default_device_fields(ui, ctx, &mut resp, action),
        Action::ExecuteScript { .. } => render_execute_script_fields(ui, ctx, &mut resp, action),
        Action::Macro { .. } => render_macro_fields(ui, ctx, &mut resp, action),
        Action::IfElse { .. } => render_if_else_fields(ui, ctx, &mut resp, action),
        Action::TrimWorkingSet { .. } => render_trim_working_set_fields(ui, ctx, &mut resp, action),
        Action::DiscordRpcSet { .. } => render_discord_rpc_set_fields(ui, ctx, &mut resp, action),
        Action::DiscordRpcClear => render_discord_rpc_clear_fields(ui, ctx, &mut resp, action),
        Action::Archive { .. } => render_archive_fields(ui, ctx, &mut resp, action),
        Action::FileSystem { .. } => render_file_system_fields(ui, ctx, &mut resp, action),
        Action::Overlay { .. } => render_overlay_fields(ui, ctx, &mut resp, action),
        Action::RuleControl { .. } => render_rule_control_fields(ui, ctx, &mut resp, action),
    }
    resp
}

fn render_start_process_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    _resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let state = ctx.state;
    let t = ctx.t;
    let is_exit_pipeline = ctx.is_exit_pipeline;
    let Action::StartProcess { file_path, arguments, working_directory, run_as_admin, only_if_not_running, run_hidden, close_on_exit, } = action else { return; };
    widgets::hud(ui, loc.inspector_field_executable_path, t.muted);
    ui.horizontal(|ui| {
        ui.set_width(ui.available_width());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let btn_w = widgets::button_width(ui, loc.settings_btn_browse, 0.0);
            if widgets::button(ui, loc.settings_btn_browse, Kind::Secondary, btn_w).clicked() {
                if let Some(file) = rfd::FileDialog::new()
                    .add_filter(loc.inspector_dialog_filter_executables, &["exe", "bat", "cmd", "lnk", "ps1"])
                    .pick_file()
                {
                    *file_path = file.to_string_lossy().to_string();
                }
            }
            ui.add(
                egui::TextEdit::singleline(file_path)
                    .hint_text(loc.inspector_field_executable_hint)
                    .desired_width(ui.available_width())
                    .min_size(vec2(0.0, theme::CONTROL_H))
                    .align(egui::Align2::LEFT_CENTER)
                    .margin(vec2(10.0, 8.0)),
            );
        });
    });

    widgets::hud(ui, loc.inspector_field_arguments, t.muted);
    ui.add(
        egui::TextEdit::singleline(arguments)
            .hint_text(loc.inspector_field_arguments_hint)
            .desired_width(ui.available_width())
            .min_size(vec2(0.0, theme::CONTROL_H))
            .align(egui::Align2::LEFT_CENTER)
            .margin(vec2(10.0, 8.0)),
    );

    widgets::hud(ui, loc.inspector_field_working_directory, t.muted);
    ui.horizontal(|ui| {
        ui.set_width(ui.available_width());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let btn_w = widgets::button_width(ui, loc.inspector_btn_folder, 0.0);
            if widgets::button(ui, loc.inspector_btn_folder, Kind::Secondary, btn_w).clicked() {
                if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                    *working_directory = Some(folder.to_string_lossy().to_string());
                }
            }
            let mut wd_str = working_directory.clone().unwrap_or_default();
            if ui.add(
                egui::TextEdit::singleline(&mut wd_str)
                    .hint_text(loc.inspector_field_working_directory_hint)
                    .desired_width(ui.available_width())
                    .min_size(vec2(0.0, theme::CONTROL_H))
                    .align(egui::Align2::LEFT_CENTER)
                    .margin(vec2(10.0, 8.0)),
            ).changed() {
                *working_directory = if wd_str.trim().is_empty() { None } else { Some(wd_str.clone()) };
            }
        });
    });

    widgets::checkbox_group(ui, |ui| {
        ui.checkbox(run_hidden, loc.inspector_checkbox_run_hidden);
        ui.checkbox(only_if_not_running, loc.inspector_checkbox_only_if_not_running);
        ui.checkbox(run_as_admin, loc.inspector_checkbox_run_as_admin);
        if !is_exit_pipeline {
            let is_process_trigger = state.editing_rule.as_ref().is_some_and(|r| matches!(r.trigger, Some(Trigger::Process { .. })));
            if !is_process_trigger {
                *close_on_exit = false;
            }
            let resp = ui.add_enabled(
                is_process_trigger,
                egui::Checkbox::new(close_on_exit, loc.inspector_checkbox_close_on_exit),
            );
            if !is_process_trigger {
                resp.on_disabled_hover_text(loc.inspector_companion_disabled_tip);
            } else {
                resp.on_hover_text(loc.inspector_checkbox_close_on_exit_tip);
            }
        }
    });
}

fn render_kill_process_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let default_picker_target = ctx.default_picker_target;
    let Action::KillProcess { process_name, kill_tree, force } = action else { return; };
    widgets::hud(ui, loc.inspector_field_terminate_process_name, t.muted);
    if render_process_name_row(ui, loc, process_name, loc.inspector_field_terminate_hint) {
        resp.pick_target = Some(default_picker_target);
    }

    widgets::checkbox_group(ui, |ui| {
        ui.checkbox(kill_tree, loc.inspector_checkbox_kill_tree);
        ui.checkbox(force, loc.inspector_checkbox_force_terminate);
    });
}

fn render_notification_fields(
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
            .min_size(vec2(0.0, theme::CONTROL_H))
            .align(egui::Align2::LEFT_CENTER)
            .margin(vec2(10.0, 8.0)),
    );

    widgets::hud(ui, loc.inspector_field_notif_message, t.muted);
    ui.add(
        egui::TextEdit::singleline(message)
            .hint_text(loc.inspector_field_notif_message_hint)
            .desired_width(ui.available_width())
            .min_size(vec2(0.0, theme::CONTROL_H))
            .align(egui::Align2::LEFT_CENTER)
            .margin(vec2(10.0, 8.0)),
    );
}

fn render_system_power_fields(
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

fn render_wait_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let state = ctx.state;
    let loc = ctx.state.loc();
    let t = ctx.t;
    let idx = ctx.idx;
    let is_exit_pipeline = ctx.is_exit_pipeline;
    let default_picker_target = ctx.default_picker_target;
    let Action::Wait { mode, seconds, condition, timeout_sec, timeout_policy } = action else { return; };
    widgets::hud(ui, loc.inspector_wait_mode, t.muted);
    let mode_options = [
        (crate::models::WaitMode::Timer, Cell::new(loc.inspector_wait_timer, "")),
        (crate::models::WaitMode::UntilCondition, Cell::new(loc.inspector_wait_until_condition, "")),
    ];
    widgets::cells(ui, mode, &mode_options);

    match mode {
        crate::models::WaitMode::Timer => {
            ui.add_space(4.0);
            widgets::hud(ui, loc.inspector_wait_seconds, t.muted);
            widgets::duration_secs_f64(ui, loc, ("wait_seconds", idx, is_exit_pipeline), seconds, 0.05..=86400.0);
        }
        crate::models::WaitMode::UntilCondition => {
            ui.add_space(4.0);
            widgets::hud(ui, loc.inspector_wait_condition_type, t.muted);

            #[derive(Clone, Copy, PartialEq, Eq)]
            enum WaitCondKind {
                WindowOpened,
                WindowClosed,
                ProcessStarted,
                ProcessExited,
                Hotkey,
                File,
                RuleStarted,
                RuleStopped,
            }

            let mut current_kind = match condition {
                Some(crate::models::WaitCondition::WindowOpened { .. }) => WaitCondKind::WindowOpened,
                Some(crate::models::WaitCondition::WindowClosed { .. }) => WaitCondKind::WindowClosed,
                Some(crate::models::WaitCondition::ProcessStarted { .. }) => WaitCondKind::ProcessStarted,
                Some(crate::models::WaitCondition::ProcessExited { .. }) => WaitCondKind::ProcessExited,
                Some(crate::models::WaitCondition::Hotkey { .. }) => WaitCondKind::Hotkey,
                Some(crate::models::WaitCondition::File { .. }) => WaitCondKind::File,
                Some(crate::models::WaitCondition::RuleStarted { .. }) => WaitCondKind::RuleStarted,
                Some(crate::models::WaitCondition::RuleStopped { .. }) => WaitCondKind::RuleStopped,
                None => WaitCondKind::WindowOpened,
            };

            let combo_id = ui.scope_id().with(idx).with("wait_cond_combo");
            let kind_label = match current_kind {
                WaitCondKind::WindowOpened => loc.inspector_wait_window_opened,
                WaitCondKind::WindowClosed => loc.inspector_wait_window_closed,
                WaitCondKind::ProcessStarted => loc.inspector_wait_process_started,
                WaitCondKind::ProcessExited => loc.inspector_wait_process_exited,
                WaitCondKind::Hotkey => loc.inspector_wait_hotkey,
                WaitCondKind::File => loc.inspector_wait_file,
                WaitCondKind::RuleStarted => loc.inspector_wait_rule_started,
                WaitCondKind::RuleStopped => loc.inspector_wait_rule_stopped,
            };

            let mut changed_kind = false;
            egui::ComboBox::from_id_salt(combo_id)
                .selected_text(kind_label)
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    if ui.selectable_value(&mut current_kind, WaitCondKind::WindowOpened, loc.inspector_wait_window_opened).clicked() { changed_kind = true; }
                    if ui.selectable_value(&mut current_kind, WaitCondKind::WindowClosed, loc.inspector_wait_window_closed).clicked() { changed_kind = true; }
                    if ui.selectable_value(&mut current_kind, WaitCondKind::ProcessStarted, loc.inspector_wait_process_started).clicked() { changed_kind = true; }
                    if ui.selectable_value(&mut current_kind, WaitCondKind::ProcessExited, loc.inspector_wait_process_exited).clicked() { changed_kind = true; }
                    if ui.selectable_value(&mut current_kind, WaitCondKind::Hotkey, loc.inspector_wait_hotkey).clicked() { changed_kind = true; }
                    if ui.selectable_value(&mut current_kind, WaitCondKind::File, loc.inspector_wait_file).clicked() { changed_kind = true; }
                    if ui.selectable_value(&mut current_kind, WaitCondKind::RuleStarted, loc.inspector_wait_rule_started).clicked() { changed_kind = true; }
                    if ui.selectable_value(&mut current_kind, WaitCondKind::RuleStopped, loc.inspector_wait_rule_stopped).clicked() { changed_kind = true; }
                });

            // A missing condition is left untouched until the user picks a kind; an
            // until_condition wait without one is rejected by the shared validator on save.
            if changed_kind {
                *condition = Some(match current_kind {
                    WaitCondKind::WindowOpened => crate::models::WaitCondition::WindowOpened { process_name: String::new(), match_mode: crate::models::ProcessMatchMode::Exact },
                    WaitCondKind::WindowClosed => crate::models::WaitCondition::WindowClosed { process_name: String::new(), match_mode: crate::models::ProcessMatchMode::Exact },
                    WaitCondKind::ProcessStarted => crate::models::WaitCondition::ProcessStarted { process_name: String::new(), match_mode: crate::models::ProcessMatchMode::Exact },
                    WaitCondKind::ProcessExited => crate::models::WaitCondition::ProcessExited { process_name: String::new(), match_mode: crate::models::ProcessMatchMode::Exact },
                    WaitCondKind::Hotkey => crate::models::WaitCondition::Hotkey { key_combination: "Ctrl+Shift+W".into() },
                    WaitCondKind::File => crate::models::WaitCondition::File { path: String::new(), condition: crate::models::WaitFileCondition::Exists },
                    WaitCondKind::RuleStarted => crate::models::WaitCondition::RuleStarted { rule_id: String::new() },
                    WaitCondKind::RuleStopped => crate::models::WaitCondition::RuleStopped { rule_id: String::new() },
                });
            }

            ui.add_space(4.0);
            if let Some(cond) = condition {
                match cond {
                    crate::models::WaitCondition::WindowOpened { process_name, match_mode }
                    | crate::models::WaitCondition::WindowClosed { process_name, match_mode }
                    | crate::models::WaitCondition::ProcessStarted { process_name, match_mode }
                    | crate::models::WaitCondition::ProcessExited { process_name, match_mode } => {
                        widgets::hud(ui, loc.inspector_field_target_process, t.muted);
                        if render_process_name_row(ui, loc, process_name, loc.inspector_hint_wait_window_process) {
                            resp.pick_target = Some(default_picker_target);
                        }

                        ui.add_space(4.0);
                        widgets::hud(ui, loc.inspector_field_match_mode, t.muted);
                        let match_options = [
                            (crate::models::ProcessMatchMode::Exact, Cell::new(loc.inspector_match_mode_exact, "")),
                            (crate::models::ProcessMatchMode::Wildcard, Cell::new(loc.inspector_match_mode_wildcard, "")),
                            (crate::models::ProcessMatchMode::Regex, Cell::new(loc.inspector_match_mode_regex, "")),
                        ];
                        widgets::cells(ui, match_mode, &match_options);
                    }
                    crate::models::WaitCondition::Hotkey { key_combination } => {
                        widgets::hud(ui, loc.inspector_trigger_hotkey, t.muted);
                        ui.add(
                            egui::TextEdit::singleline(key_combination)
                                .hint_text(loc.inspector_hint_wait_hotkey)
                                .desired_width(ui.available_width())
                                .min_size(vec2(0.0, theme::CONTROL_H))
                                .align(egui::Align2::LEFT_CENTER)
                                .margin(vec2(10.0, 8.0)),
                        );
                    }
                    crate::models::WaitCondition::File { path, condition: file_cond } => {
                        widgets::hud(ui, loc.inspector_field_watch_path, t.muted);
                        ui.horizontal(|ui| {
                            ui.set_width(ui.available_width());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let btn_w = widgets::button_width(ui, loc.inspector_btn_choose_file, 0.0);
                                if widgets::button(ui, loc.inspector_btn_choose_file, Kind::Secondary, btn_w).clicked() {
                                    if let Some(f) = rfd::FileDialog::new().pick_file() {
                                        *path = f.to_string_lossy().to_string();
                                    }
                                }
                                ui.add(
                                    egui::TextEdit::singleline(path)
                                        .hint_text(loc.inspector_hint_wait_file_path)
                                        .desired_width(ui.available_width())
                                        .min_size(vec2(0.0, theme::CONTROL_H))
                                        .align(egui::Align2::LEFT_CENTER)
                                        .margin(vec2(10.0, 8.0)),
                                );
                            });
                        });

                        ui.add_space(4.0);
                        let file_opts = [
                            (crate::models::WaitFileCondition::Exists, Cell::new(loc.inspector_wait_file_exists, "")),
                            (crate::models::WaitFileCondition::Deleted, Cell::new(loc.inspector_wait_file_deleted, "")),
                        ];
                        widgets::cells(ui, file_cond, &file_opts);
                    }
                    crate::models::WaitCondition::RuleStarted { rule_id }
                    | crate::models::WaitCondition::RuleStopped { rule_id } => {
                        // A wait cannot observe its own pipeline start or stop:
                        // "started" is already true and "stopped" cannot happen
                        // before the pipeline itself ends.
                        let editing_id = state
                            .editing_rule
                            .as_ref()
                            .map(|rule| rule.id.as_str())
                            .unwrap_or_default();
                        let choices: Vec<RuleChoice> = rule_choices(state)
                            .into_iter()
                            .filter(|choice| choice.id != editing_id)
                            .collect();
                        render_rule_choice_combo(ui, &choices, rule_id, ("wait_rule_target", idx), loc, t);
                    }
                }
            }

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                widgets::hud(ui, loc.inspector_wait_timeout, t.muted);
                widgets::duration_secs_f64(ui, loc, ("wait_timeout", idx, is_exit_pipeline), timeout_sec, 0.0..=3600.0);
                ui.label(egui::RichText::new(loc.inspector_wait_timeout_zero_hint).size(11.0).color(t.muted));
            });

            ui.add_space(4.0);
            widgets::hud(ui, loc.inspector_wait_timeout_policy, t.muted);
            let policy_options = [
                (crate::models::WaitTimeoutPolicy::Abort, Cell::new(loc.inspector_wait_policy_abort, "")),
                (crate::models::WaitTimeoutPolicy::Continue, Cell::new(loc.inspector_wait_policy_continue, "")),
            ];
            widgets::cells(ui, timeout_policy, &policy_options);
        }
    }
}

fn render_check_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let state = ctx.state;
    let t = ctx.t;
    let idx = ctx.idx;
    let default_picker_target = ctx.default_picker_target;
    let Action::Check { condition } = action else { return; };
    widgets::hud(ui, loc.catalog_title_check, t.muted);
    ui.add(
        egui::Label::new(
            egui::RichText::new(loc.inspector_check_desc)
                .size(11.5)
                .color(t.muted),
        ).wrap(),
    );
    ui.add_space(4.0);

    let mut kind = ConditionKind::from_condition(condition);
    let combo_id = ui.scope_id().with(idx).with("check_cond_kind");
    let mut changed_kind = false;
    egui::ComboBox::from_id_salt(combo_id)
        .selected_text(kind.label(loc))
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for k in [
                ConditionKind::Process,
                ConditionKind::Window,
                ConditionKind::Time,
                ConditionKind::Event,
                ConditionKind::Resource,
                ConditionKind::FileWatcher,
                ConditionKind::Rule,
            ] {
                if ui.selectable_value(&mut kind, k, k.label(loc)).clicked() {
                    changed_kind = true;
                }
            }
        });

    if changed_kind {
        *condition = match kind {
            ConditionKind::Process => RuleCondition::default_process(),
            ConditionKind::Window => RuleCondition::WindowFocused { process_name: String::new(), match_mode: ProcessMatchMode::Exact, must_be_focused: true },
            ConditionKind::Time => RuleCondition::TimeRange { start_time: "09:00".into(), end_time: "18:00".into(), days_of_week: vec![1, 2, 3, 4, 5] },
            ConditionKind::Event => RuleCondition::Event { event_type: SystemEventType::SessionLock, idle_duration_sec: 300 },
            ConditionKind::Resource => RuleCondition::ResourceThreshold { process_name: String::new(), cpu_percent: Some(80.0), ram_mb: None },
            ConditionKind::FileWatcher => RuleCondition::FileWatcher { path: String::new(), filter_extension: None, must_exist: true },
            ConditionKind::Rule => RuleCondition::RuleRunning { rule_id: String::new(), must_be_running: true },
        };
    }

    ui.add_space(4.0);
    render_single_condition_editor(ui, loc, condition, idx, || {
        resp.pick_target = Some(default_picker_target);
    }, t, state);
}

fn render_http_webhook_fields(
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

fn render_window_fields(
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

fn render_process_tuning_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let idx = ctx.idx;
    let default_picker_target = ctx.default_picker_target;
    let Action::ProcessTuning { target_process, priority, affinity_mask: _, trim_memory: _, } = action else { return; };
    widgets::hud(ui, loc.inspector_field_target_process, t.muted);
    if render_optional_process_name_row(ui, loc, target_process, loc.inspector_hint_tuning_process) {
        resp.pick_target = Some(default_picker_target);
    }

    widgets::hud(ui, loc.inspector_field_priority, t.muted);
    let combo_id = ui.scope_id().with(idx).with("pri_combo");
    egui::ComboBox::from_id_salt(combo_id)
        .selected_text(priority_label(priority, loc))
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            ui.selectable_value(priority, None, loc.inspector_priority_unchanged);
            ui.selectable_value(priority, Some(ProcessPriorityLevel::Idle), loc.inspector_priority_idle);
            ui.selectable_value(priority, Some(ProcessPriorityLevel::BelowNormal), loc.inspector_priority_below_normal);
            ui.selectable_value(priority, Some(ProcessPriorityLevel::Normal), loc.inspector_priority_normal);
            ui.selectable_value(priority, Some(ProcessPriorityLevel::AboveNormal), loc.inspector_priority_above_normal);
            ui.selectable_value(priority, Some(ProcessPriorityLevel::High), loc.inspector_priority_high);
            ui.selectable_value(priority, Some(ProcessPriorityLevel::Realtime), loc.inspector_priority_realtime);
        });
}

fn render_power_plan_fields(
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

fn render_play_sound_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    _resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let idx = ctx.idx;
    let is_exit_pipeline = ctx.is_exit_pipeline;
    let Action::PlaySound { file_path, play_beep, beep_frequency, beep_duration_ms } = action else { return; };
    let mode_options = [
        (false, Cell::new(loc.inspector_sound_mode_file, "")),
        (true, Cell::new(loc.inspector_sound_mode_beep, "")),
    ];
    widgets::cells(ui, play_beep, &mode_options);
    ui.add_space(4.0);

    if *play_beep {
        let mut is_custom = beep_frequency.is_some();
        let tone_options = [
            (false, Cell::new(loc.inspector_beep_alert, loc.inspector_beep_alert_tip)),
            (true, Cell::new(loc.inspector_beep_custom, loc.inspector_beep_custom_tip)),
        ];
        if widgets::cells(ui, &mut is_custom, &tone_options) {
            if is_custom {
                *beep_frequency = Some(750);
                *beep_duration_ms = Some(200);
            } else {
                *beep_frequency = None;
                *beep_duration_ms = None;
            }
        }

        if is_custom {
            ui.add_space(4.0);
            let mut freq = beep_frequency.unwrap_or(750);
            let mut dur = beep_duration_ms.unwrap_or(200);

            widgets::hud(ui, loc.inspector_sound_beep_frequency, t.muted);
            if ui.add(egui::Slider::new(&mut freq, 100..=5000).suffix(" Hz")).changed() {
                *beep_frequency = Some(freq);
            }

            ui.add_space(4.0);
            widgets::hud(ui, loc.inspector_sound_beep_duration, t.muted);
            if widgets::duration_millis_u32(ui, loc, ("beep_duration", idx, is_exit_pipeline), &mut dur, 50.0..=3000.0).changed() {
                *beep_duration_ms = Some(dur);
            }
        }
    } else {
        widgets::hud(ui, loc.inspector_field_audio_file_path, t.muted);
        ui.horizontal(|ui| {
            ui.set_width(ui.available_width());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let btn_w = widgets::button_width(ui, loc.settings_btn_browse, 0.0);
                if widgets::button(ui, loc.settings_btn_browse, Kind::Secondary, btn_w).clicked() {
                    if let Some(file) = rfd::FileDialog::new()
                        .add_filter(loc.inspector_dialog_filter_audio, &["wav", "mp3", "wma", "m4a", "ogg"])
                        .pick_file()
                    {
                        *file_path = file.to_string_lossy().to_string();
                    }
                }
                ui.add(
                    egui::TextEdit::singleline(file_path)
                        .hint_text(loc.inspector_hint_sound_file)
                        .desired_width(ui.available_width())
                        .min_size(vec2(0.0, theme::CONTROL_H))
                        .align(egui::Align2::LEFT_CENTER)
                        .margin(vec2(10.0, 8.0)),
                );
            });
        });
    }
}

fn render_firewall_fields(
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

fn render_display_fields(
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

fn render_audio_control_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let default_picker_target = ctx.default_picker_target;
    let Action::AudioControl { target, process_name, action_type, volume_level, left_volume, right_volume, } = action else { return; };
    widgets::hud(ui, loc.inspector_audio_target_label, t.muted);
    let target_options = [
        (crate::models::AudioTarget::Playback, Cell::new(
            loc.inspector_audio_target_playback,
            loc.inspector_audio_target_playback_tip,
        )),
        (crate::models::AudioTarget::Microphone, Cell::new(
            loc.inspector_audio_target_mic,
            loc.inspector_audio_target_mic_tip,
        )),
        (crate::models::AudioTarget::Process, Cell::new(
            loc.inspector_audio_target_process,
            loc.inspector_audio_target_process_tip,
        )),
    ];
    widgets::cells(ui, target, &target_options);

    if *target == crate::models::AudioTarget::Process {
        ui.add_space(4.0);
        widgets::hud(ui, loc.inspector_field_target_process, t.muted);
        if render_optional_process_name_row(ui, loc, process_name, loc.inspector_hint_action_process) {
            resp.pick_target = Some(default_picker_target);
        }
    }

    ui.add_space(4.0);
    widgets::hud(ui, loc.inspector_field_audio_action_type, t.muted);
    let audio_options = [
        (crate::models::AudioVolumeAction::Mute, Cell::new(
            loc.inspector_cell_mute,
            loc.inspector_audio_mute,
        )),
        (crate::models::AudioVolumeAction::Unmute, Cell::new(
            loc.inspector_cell_unmute,
            loc.inspector_audio_unmute,
        )),
        (crate::models::AudioVolumeAction::ToggleMute, Cell::new(
            loc.inspector_cell_toggle,
            loc.inspector_audio_toggle_mute,
        )),
        (crate::models::AudioVolumeAction::SetVolume, Cell::new(
            loc.inspector_cell_volume,
            loc.inspector_audio_set_volume,
        )),
    ];
    widgets::cells(ui, action_type, &audio_options);

    if *action_type == crate::models::AudioVolumeAction::SetVolume {
        ui.add_space(4.0);
        if *target == crate::models::AudioTarget::Playback {
            let mut separate = left_volume.is_some() || right_volume.is_some();
            if ui.checkbox(&mut separate, loc.inspector_audio_separate_channels).changed() {
                if separate {
                    *left_volume = Some(*volume_level);
                    *right_volume = Some(*volume_level);
                } else {
                    *left_volume = None;
                    *right_volume = None;
                }
            }

            if separate {
                let mut l = left_volume.unwrap_or(*volume_level);
                let mut r = right_volume.unwrap_or(*volume_level);
                ui.add_space(2.0);
                widgets::hud(ui, loc.inspector_audio_left_channel, t.muted);
                if ui.add(egui::Slider::new(&mut l, 0..=100).suffix("%")).changed() {
                    *left_volume = Some(l);
                }
                ui.add_space(2.0);
                widgets::hud(ui, loc.inspector_audio_right_channel, t.muted);
                if ui.add(egui::Slider::new(&mut r, 0..=100).suffix("%")).changed() {
                    *right_volume = Some(r);
                }
            } else {
                widgets::hud(ui, loc.inspector_field_volume_level, t.muted);
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(volume_level, 0..=100).suffix("%"));
                });
            }
        } else {
            widgets::hud(ui, loc.inspector_field_volume_level, t.muted);
            ui.horizontal(|ui| {
                ui.add(egui::Slider::new(volume_level, 0..=100).suffix("%"));
            });
        }
    }
}

fn render_audio_format_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    _resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let Action::AudioFormat { target, device_name: _, sample_rate, bit_depth, channels, } = action else { return; };
    widgets::hud(ui, loc.inspector_audio_target_label, t.muted);
    let target_options = [
        (crate::models::AudioTarget::Playback, Cell::new(loc.inspector_audio_target_playback, loc.inspector_audio_target_playback_tip)),
        (crate::models::AudioTarget::Microphone, Cell::new(loc.inspector_audio_target_mic, loc.inspector_audio_target_mic_tip)),
    ];
    widgets::cells(ui, target, &target_options);

    ui.add_space(4.0);
    widgets::hud(ui, loc.inspector_audio_format_sample_rate, t.muted);
    let rate_options = [
        (44100, Cell::new(loc.inspector_audio_rate_44100, loc.inspector_audio_rate_44100_tip)),
        (48000, Cell::new(loc.inspector_audio_rate_48000, loc.inspector_audio_rate_48000_tip)),
        (96000, Cell::new(loc.inspector_audio_rate_96000, loc.inspector_audio_rate_96000_tip)),
        (192000, Cell::new(loc.inspector_audio_rate_192000, loc.inspector_audio_rate_192000_tip)),
    ];
    widgets::cells(ui, sample_rate, &rate_options);

    ui.add_space(4.0);
    widgets::hud(ui, loc.inspector_audio_format_bit_depth, t.muted);
    let depth_options = [
        (16, Cell::new(loc.inspector_audio_depth_16, loc.inspector_audio_depth_16_tip)),
        (24, Cell::new(loc.inspector_audio_depth_24, loc.inspector_audio_depth_24_tip)),
        (32, Cell::new(loc.inspector_audio_depth_32, loc.inspector_audio_depth_32_tip)),
    ];
    widgets::cells(ui, bit_depth, &depth_options);

    ui.add_space(4.0);
    widgets::hud(ui, loc.inspector_audio_format_channels, t.muted);
    let ch_options = [
        (2, Cell::new(loc.inspector_audio_channels_stereo, loc.inspector_audio_channels_stereo_tip)),
        (1, Cell::new(loc.inspector_audio_channels_mono, loc.inspector_audio_channels_mono_tip)),
    ];
    widgets::cells(ui, channels, &ch_options);
}

fn render_default_device_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    _resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let Action::DefaultDevice { target, device_name } = action else { return; };
    widgets::hud(ui, loc.inspector_audio_target_label, t.muted);
    let dev_target_options = [
        (crate::models::AudioTarget::Playback, Cell::new(
            loc.inspector_audio_dev_playback,
            loc.inspector_audio_dev_playback_tip,
        )),
        (crate::models::AudioTarget::Microphone, Cell::new(
            loc.inspector_audio_dev_mic,
            loc.inspector_audio_dev_mic_tip,
        )),
    ];
    widgets::cells(ui, target, &dev_target_options);

    ui.add_space(4.0);
    let dev_hint = match target {
        crate::models::AudioTarget::Playback => loc.inspector_hint_device_playback,
        crate::models::AudioTarget::Microphone => loc.inspector_hint_device_mic,
        crate::models::AudioTarget::Process => loc.inspector_hint_device_process,
    };
    widgets::hud(ui, loc.inspector_field_device_name, t.muted);
    ui.add(
        egui::TextEdit::singleline(device_name)
            .hint_text(dev_hint)
            .desired_width(ui.available_width())
            .min_size(vec2(0.0, theme::CONTROL_H))
            .align(egui::Align2::LEFT_CENTER)
            .margin(vec2(10.0, 8.0)),
    );
}

fn render_execute_script_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let Action::ExecuteScript { interpreter, script, run_in_window } = action else { return; };
    ui.horizontal(|ui| {
        widgets::hud(ui, loc.inspector_script_interpreter, t.muted);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::glyph_button(ui, Glyph::Expand, loc.inspector_script_expand_tip).clicked() {
                resp.expand_script = Some(match interpreter {
                    crate::models::ScriptType::PowerShell => ScriptLanguage::PowerShell,
                    crate::models::ScriptType::Batch => ScriptLanguage::Batch,
                    crate::models::ScriptType::Lua => ScriptLanguage::Lua,
                });
            }
        });
    });

    let script_options = [
        (crate::models::ScriptType::PowerShell, Cell::new(loc.inspector_script_type_powershell, loc.inspector_script_type_powershell_tip)),
        (crate::models::ScriptType::Batch, Cell::new(loc.inspector_script_type_cmd, loc.inspector_script_type_cmd_tip)),
        (crate::models::ScriptType::Lua, Cell::new(loc.inspector_script_type_lua, loc.inspector_script_type_lua_tip)),
    ];
    widgets::cells(ui, interpreter, &script_options);

    ui.add_space(4.0);
    let lang = match interpreter {
        crate::models::ScriptType::PowerShell => ScriptLanguage::PowerShell,
        crate::models::ScriptType::Batch => ScriptLanguage::Batch,
        crate::models::ScriptType::Lua => ScriptLanguage::Lua,
    };
    let mut layouter = create_script_layouter(lang, *t, 12.0);
    ui.add(
        egui::TextEdit::multiline(script)
            .hint_text(loc.inspector_script_placeholder)
            .font(egui::FontId::new(12.0, theme::mono()))
            .desired_width(ui.available_width())
            .desired_rows(4)
            .margin(vec2(8.0, 6.0))
            .layouter(&mut layouter),
    );

    if *interpreter != crate::models::ScriptType::Lua {
        widgets::checkbox(ui, run_in_window, loc.inspector_checkbox_visible_console);
    }
}

fn render_macro_fields(
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

fn render_if_else_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let state = ctx.state;
    let idx = ctx.idx;
    let is_exit_pipeline = ctx.is_exit_pipeline;
    let loc = ctx.state.loc();
    let t = ctx.t;
    let depth = ctx.depth;
    let Action::IfElse { conditions, then_actions, else_if_branches, else_actions } = action else { return; };
    if depth > 0 {
        ui.label(egui::RichText::new(loc.inspector_ifelse_nested_not_supported).color(t.muted));
    } else {
        // An empty condition list is rendered as-is; the "add condition" button below is
        // the user action that fills it, so opening the rule does not mutate the draft.
        // ── 1. PRIMARY IF BLOCK ──
        ui.horizontal(|ui| {
            widgets::hud(ui, loc.inspector_ifelse_then_badge, t.good);
        });

        let mut remove_if_cond_idx = None;
        let mut change_if_cond_idx = None;
        let total_if_conds = conditions.len();
        for (c_idx, c_item) in conditions.iter_mut().enumerate() {
            if c_idx > 0 {
                render_condition_operator_connector(ui, &mut c_item.operator, loc, t);
            }
            let can_remove = total_if_conds > 1;
            let (remove_clicked, change_clicked) = render_condition_card(
                ui,
                c_idx,
                c_item,
                loc,
                t,
                true,
                can_remove,
                || {
                    resp.pick_target = Some(crate::gui::state::ProcessPickerTarget::ActionCondition {
                        action_idx: idx,
                        cond_idx: c_idx,
                        is_exit: is_exit_pipeline,
                    });
                },
                state,
            );
            if remove_clicked && can_remove {
                remove_if_cond_idx = Some(c_idx);
            }
            if change_clicked {
                change_if_cond_idx = Some(c_idx);
            }
        }
        if let Some(c_idx) = remove_if_cond_idx {
            if c_idx < conditions.len() {
                conditions.remove(c_idx);
            }
        }
        if let Some(c_idx) = change_if_cond_idx {
            resp.open_condition_catalog = Some(crate::gui::state::ConditionCatalogTarget::ChangeActionIfElse {
                action_idx: idx,
                cond_idx: c_idx,
                is_exit: is_exit_pipeline,
            });
        }

        if render_add_condition_button(ui, loc, t) {
            resp.open_condition_catalog = Some(crate::gui::state::ConditionCatalogTarget::ActionIfElse {
                action_idx: idx,
                is_exit: is_exit_pipeline,
            });
        }

        ui.add_space(6.0);

        // THEN branch actions
        ui.horizontal(|ui| {
            widgets::hud(ui, loc.inspector_ifelse_then_execute, t.good);
        });

        let then_branch_salt = ifelse_branch_salt(IfElseBranchSalt::Then);
        render_subactions_branch_pipeline(
            ui,
            state,
            then_actions,
            idx,
            &then_branch_salt,
            loc.inspector_ifelse_then_badge,
            is_exit_pipeline,
            BranchPickerKind::Then,
            loc.inspector_ifelse_add_then,
            t,
            depth + 1,
            resp,
            Some(crate::gui::state::IfElseCatalogTarget::Then(idx)),
        );

        // ── 2. ELSE IF BRANCHES ──
        let mut remove_branch_idx = None;
        for (b_idx, branch) in else_if_branches.iter_mut().enumerate() {
            ui.add_space(8.0);
            widgets::hrule(ui);

            ui.horizontal(|ui| {
                let b_lbl = format!("{} {:02}", loc.inspector_ifelse_elseif_badge, b_idx + 1);
                widgets::hud(ui, &b_lbl, t.warn);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::glyph_button(ui, Glyph::Close, loc.inspector_ifelse_remove_branch_tip).clicked() {
                        remove_branch_idx = Some(b_idx);
                    }
                });
            });

            let mut remove_elif_cond_idx = None;
            let mut change_elif_cond_idx = None;
            let total_elif_conds = branch.conditions.len();
            for (c_idx, c_item) in branch.conditions.iter_mut().enumerate() {
                if c_idx > 0 {
                    render_condition_operator_connector(ui, &mut c_item.operator, loc, t);
                }
                let can_remove = total_elif_conds > 1;
                let (remove_clicked, change_clicked) = render_condition_card(
                    ui,
                    c_idx,
                    c_item,
                    loc,
                    t,
                    true,
                    can_remove,
                    || {
                        resp.pick_target = Some(crate::gui::state::ProcessPickerTarget::ActionElseIfCondition {
                            action_idx: idx,
                            branch_idx: b_idx,
                            cond_idx: c_idx,
                            is_exit: is_exit_pipeline,
                        });
                    },
                    state,
                );
                if remove_clicked && can_remove {
                    remove_elif_cond_idx = Some(c_idx);
                }
                if change_clicked {
                    change_elif_cond_idx = Some(c_idx);
                }
            }
            if let Some(c_idx) = remove_elif_cond_idx {
                if c_idx < branch.conditions.len() {
                    branch.conditions.remove(c_idx);
                }
            }
            if let Some(c_idx) = change_elif_cond_idx {
                resp.open_condition_catalog = Some(crate::gui::state::ConditionCatalogTarget::ChangeActionElseIf {
                    action_idx: idx,
                    branch_idx: b_idx,
                    cond_idx: c_idx,
                    is_exit: is_exit_pipeline,
                });
            }

            if render_add_condition_button(ui, loc, t) {
                resp.open_condition_catalog = Some(crate::gui::state::ConditionCatalogTarget::ActionElseIf {
                    action_idx: idx,
                    branch_idx: b_idx,
                    is_exit: is_exit_pipeline,
                });
            }

            ui.add_space(6.0);

            // Branch actions header
            let branch_salt = ifelse_branch_salt(IfElseBranchSalt::ElseIf(b_idx));
            let branch_lbl = loc.inspector_ifelse_elif_badge.replace("{:02}", &format!("{:02}", b_idx + 1));
            ui.horizontal(|ui| {
                widgets::hud(ui, loc.inspector_ifelse_then_execute, t.warn);
            });

            render_subactions_branch_pipeline(
                ui,
                state,
                &mut branch.actions,
                idx,
                &branch_salt,
                &branch_lbl,
                is_exit_pipeline,
                BranchPickerKind::ElseIf(b_idx),
                loc.inspector_ifelse_add_then,
                t,
                depth + 1,
                resp,
                Some(crate::gui::state::IfElseCatalogTarget::ElseIf {
                    parent_idx: idx,
                    branch_idx: b_idx,
                }),
            );
        }
        if let Some(b_idx) = remove_branch_idx {
            if b_idx < else_if_branches.len() {
                else_if_branches.remove(b_idx);
            }
        }

        // Full-width button to add an Else If branch with separator above and below
        ui.add_space(8.0);
        widgets::hrule(ui);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            widgets::hud(ui, loc.inspector_ifelse_elseif_badge, t.warn);
        });
        ui.add_space(4.0);
        if render_full_width_action_button(ui, loc.inspector_ifelse_add_elseif, "", t) {
            else_if_branches.push(crate::models::ElseIfBranch::new());
        }
        ui.add_space(6.0);
        widgets::hrule(ui);
        ui.add_space(8.0);

        // ── 3. ELSE BRANCH ──
        ui.horizontal(|ui| {
            widgets::hud(ui, loc.inspector_ifelse_else_badge, t.bad);
        });

        let else_branch_salt = ifelse_branch_salt(IfElseBranchSalt::Else);
        render_subactions_branch_pipeline(
            ui,
            state,
            else_actions,
            idx,
            &else_branch_salt,
            loc.inspector_ifelse_else_badge,
            is_exit_pipeline,
            BranchPickerKind::Else,
            loc.inspector_ifelse_add_else,
            t,
            depth + 1,
            resp,
            Some(crate::gui::state::IfElseCatalogTarget::Else(idx)),
        );
    }
}

fn render_trim_working_set_fields(
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

fn render_discord_rpc_set_fields(
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
            .min_size(vec2(0.0, theme::CONTROL_H))
            .margin(vec2(10.0, 8.0)),
    );
    ui.add_space(4.0);

    widgets::hud(ui, loc.inspector_discord_details, t.muted);
    ui.add(
        egui::TextEdit::singleline(details)
            .hint_text(loc.inspector_hint_discord_details)
            .desired_width(ui.available_width())
            .min_size(vec2(0.0, theme::CONTROL_H))
            .margin(vec2(10.0, 8.0)),
    );
    ui.add_space(4.0);

    widgets::hud(ui, loc.inspector_discord_state, t.muted);
    ui.add(
        egui::TextEdit::singleline(rpc_state)
            .hint_text(loc.inspector_hint_discord_state)
            .desired_width(ui.available_width())
            .min_size(vec2(0.0, theme::CONTROL_H))
            .margin(vec2(10.0, 8.0)),
    );
    ui.add_space(4.0);

    widgets::hud(ui, loc.inspector_discord_large_image, t.muted);
    let mut l_img = large_image.clone().unwrap_or_default();
    if ui.add(
        egui::TextEdit::singleline(&mut l_img)
            .hint_text(loc.inspector_hint_discord_large_image)
            .desired_width(ui.available_width())
            .min_size(vec2(0.0, theme::CONTROL_H))
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
            .min_size(vec2(0.0, theme::CONTROL_H))
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
            .min_size(vec2(0.0, theme::CONTROL_H))
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
            .min_size(vec2(0.0, theme::CONTROL_H))
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
                        .min_size(vec2(0.0, theme::CONTROL_H))
                        .align(egui::Align2::LEFT_CENTER)
                        .margin(vec2(8.0, 6.0)),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut btn.label)
                        .hint_text(loc.inspector_discord_btn_label)
                        .desired_width(label_w)
                        .min_size(vec2(0.0, theme::CONTROL_H))
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

fn render_discord_rpc_clear_fields(
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
        .corner_radius(CornerRadius::ZERO)
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

fn render_archive_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    _resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let Action::Archive { operation, format, source_path, destination_path, overwrite, .. } = action else { return; };
    let is_pack = matches!(operation, crate::models::ArchiveOperation::Pack);
    let ext = archive_format_ext(*format);

    widgets::hud(ui, loc.inspector_archive_operation, t.muted);
    let op_options = [
        (crate::models::ArchiveOperation::Pack, Cell::new(loc.inspector_archive_op_pack, "")),
        (crate::models::ArchiveOperation::Unpack, Cell::new(loc.inspector_archive_op_unpack, "")),
    ];
    widgets::cells(ui, operation, &op_options);
    ui.add_space(4.0);

    widgets::hud(ui, loc.inspector_archive_format, t.muted);
    let fmt_options = ARCHIVE_FORMAT_ORDER.map(|f| (f, archive_format_cell(f, loc)));
    widgets::cells(ui, format, &fmt_options);
    ui.add_space(4.0);

    widgets::hud(ui, loc.inspector_archive_source, t.muted);
    ui.horizontal(|ui| {
        ui.set_width(ui.available_width());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if is_pack {
                if widgets::button(ui, loc.inspector_cond_file_btn_folder, Kind::Secondary, 0.0).clicked() {
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        *source_path = folder.to_string_lossy().to_string();
                    }
                }
                if widgets::button(ui, loc.inspector_cond_file_btn_file, Kind::Secondary, 0.0).clicked() {
                    if let Some(file) = rfd::FileDialog::new().pick_file() {
                        *source_path = file.to_string_lossy().to_string();
                    }
                }
            } else {
                let btn_w = widgets::button_width(ui, loc.settings_btn_browse, 0.0);
                if widgets::button(ui, loc.settings_btn_browse, Kind::Secondary, btn_w).clicked() {
                    if let Some(file) = rfd::FileDialog::new()
                        .add_filter(loc.inspector_dialog_filter_archive, &["zip", "7z", "rar", "tar", "gz", "tgz", "xz", "bz2"])
                        .add_filter(loc.inspector_dialog_filter_all, &["*"])
                        .pick_file()
                    {
                        *source_path = file.to_string_lossy().to_string();
                    }
                }
            }
            let src_hint = if is_pack {
                loc.inspector_hint_archive_source_pack
            } else {
                loc.inspector_hint_archive_source_unpack
            };
            ui.add(
                egui::TextEdit::singleline(source_path)
                    .hint_text(src_hint)
                    .desired_width(ui.available_width())
                    .min_size(vec2(0.0, theme::CONTROL_H))
                    .align(egui::Align2::LEFT_CENTER)
                    .margin(vec2(10.0, 8.0)),
            );
        });
    });
    ui.add_space(4.0);

    widgets::hud(ui, loc.inspector_archive_destination, t.muted);
    ui.horizontal(|ui| {
        ui.set_width(ui.available_width());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let btn_w = widgets::button_width(ui, loc.settings_btn_browse, 0.0);
            if widgets::button(ui, loc.settings_btn_browse, Kind::Secondary, btn_w).clicked() {
                if is_pack {
                    let filter_label = loc.inspector_dialog_filter_archive_typed.replace("{}", &ext.to_uppercase());
                    let default_name = format!("archive.{}", ext);
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter(&filter_label, &[ext])
                        .set_file_name(&default_name)
                        .save_file()
                    {
                        let mut p_str = p.to_string_lossy().to_string();
                        let suffix = format!(".{}", ext);
                        if !p_str.to_lowercase().ends_with(&suffix) {
                            p_str.push_str(&suffix);
                        }
                        *destination_path = p_str;
                    }
                } else if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                    *destination_path = folder.to_string_lossy().to_string();
                }
            }
            let dst_hint = if is_pack {
                loc.inspector_hint_archive_dest_pack
            } else {
                loc.inspector_hint_archive_dest_unpack
            };
            ui.add(
                egui::TextEdit::singleline(destination_path)
                    .hint_text(dst_hint)
                    .desired_width(ui.available_width())
                    .min_size(vec2(0.0, theme::CONTROL_H))
                    .align(egui::Align2::LEFT_CENTER)
                    .margin(vec2(10.0, 8.0)),
            );
        });
    });
    ui.add_space(4.0);

    widgets::checkbox(ui, overwrite, loc.inspector_archive_overwrite);
}

fn render_file_system_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    _resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let t = ctx.t;
    let idx = ctx.idx;
    let Action::FileSystem { operation, path, target_path, content, create_parents, overwrite, } = action else { return; };
    widgets::hud(ui, loc.inspector_fs_operation, t.muted);
    let combo_id = ui.scope_id().with(idx).with("fs_op_combo");
    let op_text = match operation {
        crate::models::FileSystemOperation::CreateFile => loc.inspector_fs_op_create_file,
        crate::models::FileSystemOperation::CreateDirectory => loc.inspector_fs_op_create_dir,
        crate::models::FileSystemOperation::Rename => loc.inspector_fs_op_rename,
        crate::models::FileSystemOperation::Copy => loc.inspector_fs_op_copy,
        crate::models::FileSystemOperation::Delete => loc.inspector_fs_op_delete,
        crate::models::FileSystemOperation::WriteText => loc.inspector_fs_op_write_text,
        crate::models::FileSystemOperation::AppendText => loc.inspector_fs_op_append_text,
    };
    egui::ComboBox::from_id_salt(combo_id)
        .selected_text(op_text)
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            ui.selectable_value(operation, crate::models::FileSystemOperation::CreateFile, loc.inspector_fs_op_create_file);
            ui.selectable_value(operation, crate::models::FileSystemOperation::CreateDirectory, loc.inspector_fs_op_create_dir);
            ui.selectable_value(operation, crate::models::FileSystemOperation::Rename, loc.inspector_fs_op_rename);
            ui.selectable_value(operation, crate::models::FileSystemOperation::Copy, loc.inspector_fs_op_copy);
            ui.selectable_value(operation, crate::models::FileSystemOperation::Delete, loc.inspector_fs_op_delete);
            ui.selectable_value(operation, crate::models::FileSystemOperation::WriteText, loc.inspector_fs_op_write_text);
            ui.selectable_value(operation, crate::models::FileSystemOperation::AppendText, loc.inspector_fs_op_append_text);
        });
    ui.add_space(4.0);

    let fs_op = *operation;
    widgets::hud(ui, loc.inspector_fs_path, t.muted);
    ui.horizontal(|ui| {
        ui.set_width(ui.available_width());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            match fs_op {
                crate::models::FileSystemOperation::CreateDirectory => {
                    let btn_w = widgets::button_width(ui, loc.settings_btn_browse, 0.0);
                    if widgets::button(ui, loc.settings_btn_browse, Kind::Secondary, btn_w).clicked() {
                        if let Some(p) = rfd::FileDialog::new().pick_folder() {
                            *path = p.to_string_lossy().to_string();
                        }
                    }
                }
                crate::models::FileSystemOperation::CreateFile => {
                    let btn_w = widgets::button_width(ui, loc.settings_btn_browse, 0.0);
                    if widgets::button(ui, loc.settings_btn_browse, Kind::Secondary, btn_w).clicked() {
                        if let Some(p) = rfd::FileDialog::new().save_file() {
                            *path = p.to_string_lossy().to_string();
                        }
                    }
                }
                _ => {
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
                }
            }
            let p_hint = match fs_op {
                crate::models::FileSystemOperation::CreateDirectory => loc.inspector_hint_fs_new_folder,
                crate::models::FileSystemOperation::CreateFile => loc.inspector_hint_fs_new_file,
                _ => loc.inspector_hint_fs_path,
            };
            ui.add(
                egui::TextEdit::singleline(path)
                    .hint_text(p_hint)
                    .desired_width(ui.available_width())
                    .min_size(vec2(0.0, theme::CONTROL_H))
                    .align(egui::Align2::LEFT_CENTER)
                    .margin(vec2(10.0, 8.0)),
            );
        });
    });
    ui.add_space(4.0);

    if matches!(operation, crate::models::FileSystemOperation::Rename | crate::models::FileSystemOperation::Copy) {
        widgets::hud(ui, loc.inspector_fs_target_path, t.muted);
        let mut tgt = target_path.clone().unwrap_or_default();
        ui.horizontal(|ui| {
            ui.set_width(ui.available_width());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::button(ui, loc.inspector_cond_file_btn_folder, Kind::Secondary, 0.0).clicked() {
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        *target_path = Some(folder.to_string_lossy().to_string());
                    }
                }
                if widgets::button(ui, loc.inspector_cond_file_btn_file, Kind::Secondary, 0.0).clicked() {
                    if let Some(file) = rfd::FileDialog::new().pick_file() {
                        *target_path = Some(file.to_string_lossy().to_string());
                    }
                }
                if ui.add(
                    egui::TextEdit::singleline(&mut tgt)
                        .hint_text(loc.inspector_hint_fs_target_path)
                        .desired_width(ui.available_width())
                        .min_size(vec2(0.0, theme::CONTROL_H))
                        .align(egui::Align2::LEFT_CENTER)
                        .margin(vec2(10.0, 8.0)),
                ).changed() {
                    *target_path = if tgt.trim().is_empty() { None } else { Some(tgt.clone()) };
                }
            });
        });
        ui.add_space(4.0);
    }

    if matches!(operation, crate::models::FileSystemOperation::WriteText | crate::models::FileSystemOperation::AppendText) {
        widgets::hud(ui, loc.inspector_fs_content, t.muted);
        let mut text_body = content.clone().unwrap_or_default();
        if ui.add(
            egui::TextEdit::multiline(&mut text_body)
                .hint_text(loc.inspector_hint_fs_content)
                .desired_width(ui.available_width())
                .desired_rows(4)
                .margin(vec2(10.0, 8.0)),
        ).changed() {
            *content = Some(text_body);
        }
        ui.add_space(4.0);
    }

    widgets::checkbox_group(ui, |ui| {
        ui.checkbox(create_parents, loc.inspector_fs_create_parents);
        ui.checkbox(overwrite, loc.inspector_fs_overwrite);
    });
}

fn render_overlay_fields(
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

        egui::ComboBox::from_id_salt(("overlay_preset_select", idx))
            .selected_text(RichText::new(current_label).size(12.0))
            .width((ui.available_width() - if !is_custom { 120.0 } else { 0.0 }).max(140.0))
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
        p.rect_filled(rect, CornerRadius::same(2), bg);
        p.rect_stroke(rect, CornerRadius::same(2), Stroke::new(1.0, border_color), egui::StrokeKind::Inside);

        // Icon chip on left
        let icon_box = Rect::from_min_size(rect.min + vec2(8.0, 7.0), vec2(30.0, 30.0));
        let icon_bg = if is_overlay_showing || hovered { t.accent_ink.gamma_multiply(0.22) } else { t.accent_ink.gamma_multiply(0.12) };
        p.rect_filled(icon_box, CornerRadius::same(2), icon_bg);
        p.text(
            icon_box.center(),
            egui::Align2::CENTER_CENTER,
            egui_phosphor::regular::FRAME_CORNERS,
            egui::FontId::new(18.0, theme::phosphor()),
            t.accent_ink,
        );

        // Middle title
        let text_left = icon_box.right() + 10.0;
        let title_pos = pos2(text_left, rect.center().y);
        let btn_title = if is_custom {
            loc.inspector_overlay_studio_title.to_string()
        } else {
            let matched_name = display_overlay
                .map(|o| o.name.as_str())
                .unwrap_or(overlay_id.as_str());
            format!("{} ({})", loc.inspector_overlay_studio_title, matched_name)
        };
        p.text(
            title_pos,
            egui::Align2::LEFT_CENTER,
            btn_title,
            egui::FontId::new(13.0, theme::semibold()),
            if is_overlay_showing || hovered { t.accent_ink } else { t.ink_strong },
        );

        // Action arrow pill on right
        let arrow_pos = pos2(rect.right() - 14.0, rect.center().y);
        let (status_text, status_color) = if is_overlay_showing {
            (loc.inspector_badge_executing_now, t.accent_ink)
        } else if hovered {
            (loc.inspector_overlay_studio_open, t.accent_ink)
        } else {
            (loc.inspector_overlay_studio_open, t.faint)
        };
        p.text(
            arrow_pos,
            egui::Align2::RIGHT_CENTER,
            status_text,
            egui::FontId::new(11.0, theme::mono_bold()),
            status_color,
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
                .min_size(vec2(0.0, theme::CONTROL_H))
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
                    .min_size(vec2(0.0, theme::CONTROL_H))
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

fn render_rule_control_fields(
    ui: &mut Ui,
    ctx: &ActionFieldsCtx<'_>,
    _resp: &mut ActionFieldsResponse,
    action: &mut Action,
) {
    let loc = ctx.state.loc();
    let state = ctx.state;
    let t = ctx.t;
    let idx = ctx.idx;
    let is_exit_pipeline = ctx.is_exit_pipeline;
    let Action::RuleControl { rule_id, operation, wait } = action else { return; };
    widgets::hud(ui, loc.inspector_rule_operation, t.muted);
    let operation_options = [
        (
            crate::models::RuleControlOperation::Call,
            widgets::Cell::new(loc.inspector_rule_op_call, loc.inspector_rule_op_call_desc),
        ),
        (
            crate::models::RuleControlOperation::Stop,
            widgets::Cell::new(loc.inspector_rule_op_stop, loc.inspector_rule_op_stop_desc),
        ),
    ];
    widgets::cells(ui, operation, &operation_options);

    ui.add_space(4.0);
    render_rule_target_picker(ui, state, rule_id, idx, is_exit_pipeline, t);

    if *operation == crate::models::RuleControlOperation::Call {
        ui.add_space(4.0);
        widgets::checkbox_group(ui, |ui| {
            ui.checkbox(wait, loc.inspector_rule_wait)
                .on_hover_text(loc.inspector_rule_wait_tip);
        });
    } else {
        // A stopped rule is never awaited: the mode switch must not leave a stale flag.
        *wait = false;
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConditionKind {
    Process,
    Window,
    Time,
    Event,
    Resource,
    FileWatcher,
    Rule,
}

impl ConditionKind {
    fn from_condition(c: &RuleCondition) -> Self {
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

    fn label(&self, loc: &crate::i18n::Strings) -> &'static str {
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
fn render_condition_card(
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
        .corner_radius(CornerRadius::ZERO)
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

fn render_condition_operator_connector(
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
        cells_w += (galley.size().x + 24.0).max(theme::CONTROL_H);
    }

    let side_w = ((avail_w - cells_w - 12.0) * 0.5).max(12.0);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;

        let (rect_left, _) = ui.allocate_exact_size(vec2(side_w, theme::CONTROL_H), Sense::hover());
        ui.painter().line_segment(
            [pos2(rect_left.left(), rect_left.center().y), pos2(rect_left.right(), rect_left.center().y)],
            Stroke::new(1.0, t.muted.gamma_multiply(0.25)),
        );

        widgets::cells(ui, op, &op_options);

        let rem_w = (ui.available_width() - 2.0).max(0.0);
        if rem_w > 0.0 {
            let (rect_right, _) = ui.allocate_exact_size(vec2(rem_w, theme::CONTROL_H), Sense::hover());
            ui.painter().line_segment(
                [pos2(rect_right.left(), rect_right.center().y), pos2(rect_right.right(), rect_right.center().y)],
                Stroke::new(1.0, t.muted.gamma_multiply(0.25)),
            );
        }
    });
    ui.add_space(2.0);
}

fn render_full_width_action_button(
    ui: &mut Ui,
    label: &str,
    tip: &str,
    t: &Tokens,
) -> bool {
    widgets::full_width_action_button(ui, label, tip, t)
}

fn render_add_condition_button(
    ui: &mut Ui,
    loc: &crate::i18n::Strings,
    t: &Tokens,
) -> bool {
    render_full_width_action_button(ui, loc.inspector_conditions_btn_add, "", t)
}

fn render_single_condition_editor(
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
                        .min_size(vec2(0.0, theme::CONTROL_H))
                        .align(egui::Align2::LEFT_CENTER)
                        .margin(vec2(8.0, 6.0)),
                );
                widgets::hud(ui, loc.inspector_cond_time_to, t.muted);
                ui.add(
                    egui::TextEdit::singleline(end_time)
                        .hint_text(loc.inspector_hint_cond_time_to)
                        .desired_width(65.0)
                        .min_size(vec2(0.0, theme::CONTROL_H))
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
                            .min_size(vec2(0.0, theme::CONTROL_H))
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
                    .min_size(vec2(0.0, theme::CONTROL_H))
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

fn render_conditions_section(
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

#[allow(clippy::too_many_arguments)]
fn render_subaction_card(
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
                            let (ok, msg) = match sub_act {
                                Action::Macro { events, speed, .. } => {
                                    if events.is_empty() {
                                        (false, loc.inspector_macro_test_empty.to_string())
                                    } else {
                                        crate::gui::macro_recorder::run_macro_test_safe(events.clone(), *speed);
                                        (true, loc.inspector_macro_testing_actions.replace("{}", &events.len().to_string()))
                                    }
                                }
                                _ => {
                                    let mut ctx = std::collections::HashMap::new();
                                    ctx.insert("ProcessName".to_string(), "test.exe".to_string());
                                    ctx.insert("Timestamp".to_string(), chrono::Local::now().to_rfc3339());
                                    let res = crate::ipc::IpcClient::call(&crate::ipc::IpcRequest::TestStep {
                                        action: sub_act.clone(),
                                        context: ctx,
                                    });
                                    let ok = matches!(res, Ok(crate::ipc::IpcResponse::TestStepResult(Ok(()))));
                                    let msg = match res {
                                        Ok(crate::ipc::IpcResponse::TestStepResult(Ok(()))) => loc.inspector_test_step_passed.to_string(),
                                        Ok(crate::ipc::IpcResponse::TestStepResult(Err(e))) => loc.inspector_test_step_failed.replace("{}", &e),
                                        Ok(crate::ipc::IpcResponse::Error(e)) => loc.inspector_test_step_error.replace("{}", &e),
                                        Err(e) => loc.inspector_test_step_ipc_error.replace("{}", &e.to_string()),
                                        _ => loc.inspector_test_step_unknown.to_string(),
                                    };
                                    (ok, msg)
                                }
                            };
                            let sub_test_key = ui.scope_id().with(("sub_test_res", parent_idx, branch_salt, sub_idx));
                            ui.data_mut(|d| d.insert_temp(sub_test_key, (ok, msg, std::time::Instant::now())));
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

#[derive(Clone, Copy)]
enum BranchPickerKind {
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
enum IfElseBranchSalt {
    Then,
    Else,
    ElseIf(usize),
}

fn ifelse_branch_salt(branch: IfElseBranchSalt) -> String {
    match branch {
        IfElseBranchSalt::Then => "then".to_string(),
        IfElseBranchSalt::Else => "else".to_string(),
        IfElseBranchSalt::ElseIf(branch_idx) => format!("elif_{branch_idx}"),
    }
}

/// Host for a nested If/Else branch: cards are the compact sub-action cards and every
/// outcome is folded back into the parent editor's response.
struct NestedPipelineHost<'a> {
    state: &'a GuiState,
    resp: &'a mut ActionFieldsResponse,
    t: &'a Tokens,
    parent_idx: usize,
    branch_salt: &'a str,
    branch_label: &'a str,
    picker_kind: BranchPickerKind,
    is_exit_pipeline: bool,
    depth: usize,
    add_btn_label: &'a str,
    catalog_target: Option<crate::gui::state::IfElseCatalogTarget>,
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
fn render_subactions_branch_pipeline(
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
/// Sends the force-terminate request and reports the daemon's answer. Returns `true` only
/// when the daemon confirmed `IpcResponse::Ok`, so the caller can clear the selection.
fn terminate_process(state: &mut GuiState, proc: &ProcessSnapshotItem) -> bool {
    let loc = state.loc();
    match crate::ipc::IpcClient::call(&crate::ipc::IpcRequest::KillProcess {
        pid: proc.pid,
        force: true,
    }) {
        Ok(crate::ipc::IpcResponse::Ok) => {
            state.set_toast(format!("{} {} #{}", loc.toast_process_terminated, proc.name, proc.pid));
            true
        }
        Ok(crate::ipc::IpcResponse::Error(error)) => {
            crate::logging::warn("GUI", &format!("KillProcess #{} was refused: {error}", proc.pid));
            state.set_toast(loc.toast_request_failed.replace("{}", &error));
            false
        }
        Ok(_) => {
            crate::logging::warn("GUI", &format!("KillProcess #{} returned an unexpected response", proc.pid));
            state.set_toast(loc.toast_request_failed_unexpected.to_string());
            false
        }
        Err(error) => {
            crate::logging::warn("GUI", &format!("KillProcess #{} failed: {error}", proc.pid));
            state.set_toast(loc.toast_request_failed.replace("{}", &error));
            false
        }
    }
}

fn render_process_inspector(ui: &mut Ui, state: &mut GuiState, proc: &ProcessSnapshotItem, t: &Tokens) {
    let loc = state.loc();
    let scrollbar_w = ui.spacing().scroll.bar_width + ui.spacing().scroll.bar_inner_margin + ui.spacing().scroll.bar_outer_margin;
    let right_m = (20.0 - scrollbar_w).max(0.0).round() as i8;

    egui::Frame::new()
        .inner_margin(egui::Margin {
            left: 20,
            right: right_m,
            top: 8,
            bottom: 8,
        })
        .show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 12.0;

            // Window Title
            widgets::hud(ui, loc.inspector_field_window_title, t.muted);
            let wtitle = if proc.window_title.is_empty() {
                loc.inspector_no_active_window
            } else {
                &proc.window_title
            };
            ui.label(
                egui::RichText::new(wtitle)
                    .family(theme::medium())
                    .size(13.0)
                    .color(t.ink),
            );

            // RAM Working Set
            let mb = proc.memory_bytes as f64 / 1_048_576.0;
            widgets::hud(ui, loc.inspector_field_working_set_ram, t.muted);
            ui.horizontal_wrapped(|ui| {
                widgets::badge(ui, &format!("{:.1} MB", mb), t.good);
                widgets::hud(ui, &format!("{} {}", proc.memory_bytes, loc.inspector_bytes_suffix), t.faint);
            });

            widgets::hrule(ui);

            widgets::eyebrow(ui, "02", loc.inspector_eyebrow_quick_actions);

            if widgets::button(ui, loc.inspector_btn_create_automation, Kind::Secondary, 0.0).clicked() {
                state.apply_process_to_draft_or_new_rule(&proc.name);
            }

            widgets::hrule(ui);

            // Terminating a process is destructive and cannot be undone: the first click only
            // arms the confirmation, and the selection is cleared only on a confirmed kill.
            let confirm_id = Id::unique(("inspector_terminate_confirm", proc.pid));
            let awaiting_confirm: bool = ui.data(|d| d.get_temp(confirm_id).unwrap_or(false));
            if awaiting_confirm {
                ui.label(egui::RichText::new(loc.inspector_terminate_confirm).size(11.5).color(t.warn));
                ui.horizontal(|ui| {
                    if widgets::button(ui, loc.inspector_btn_terminate_process, Kind::Danger, 0.0).clicked() {
                        ui.data_mut(|d| d.remove_temp::<bool>(confirm_id));
                        if terminate_process(state, proc) {
                            state.selected_process = None;
                        }
                    }
                    if widgets::button(ui, loc.inspector_btn_cancel, Kind::Secondary, 0.0).clicked() {
                        ui.data_mut(|d| d.remove_temp::<bool>(confirm_id));
                    }
                });
            } else if widgets::button(ui, loc.inspector_btn_terminate_process, Kind::Danger, 0.0).clicked() {
                ui.data_mut(|d| d.insert_temp(confirm_id, true));
            }
        });
    });
}

fn render_trigger_info_frame(ui: &mut Ui, glyph: &'static str, desc: &str, tint: Color32, t: &Tokens) {
    egui::Frame::new()
        .fill(t.bg)
        .stroke(t.hairline())
        .corner_radius(CornerRadius::ZERO)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.label(
                    egui::RichText::new(glyph)
                        .family(theme::phosphor())
                        .size(18.0)
                        .color(tint),
                );
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(desc)
                            .size(11.5)
                            .color(t.ink_strong),
                    ).wrap(),
                );
            });
        });
}

/// Glyph, short label, long description and accent tint of one system event.
#[derive(Clone, Copy)]
struct SystemEventMeta {
    glyph: &'static str,
    label: &'static str,
    description: &'static str,
    tint: Color32,
}

/// Single source of truth for the system-event presentation: the trigger header, the
/// trigger info card and the condition card all read this table.
fn system_event_meta(event_type: SystemEventType, loc: &crate::i18n::Strings, t: &Tokens) -> SystemEventMeta {
    match event_type {
        SystemEventType::SessionLock => SystemEventMeta {
            glyph: egui_phosphor::regular::LOCK,
            label: loc.inspector_sys_event_lock,
            description: loc.inspector_sys_event_desc_lock,
            tint: t.accent_ink,
        },
        SystemEventType::SessionUnlock => SystemEventMeta {
            glyph: egui_phosphor::regular::LOCK_OPEN,
            label: loc.inspector_sys_event_unlock,
            description: loc.inspector_sys_event_desc_unlock,
            tint: t.accent_ink,
        },
        SystemEventType::SystemSleep => SystemEventMeta {
            glyph: egui_phosphor::regular::MOON,
            label: loc.inspector_sys_event_sleep,
            description: loc.inspector_sys_event_desc_sleep,
            tint: t.accent_ink,
        },
        SystemEventType::SystemResume => SystemEventMeta {
            glyph: egui_phosphor::regular::SUN,
            label: loc.inspector_sys_event_resume,
            description: loc.inspector_sys_event_desc_resume,
            tint: t.good,
        },
        SystemEventType::PowerSourceAc => SystemEventMeta {
            glyph: egui_phosphor::regular::PLUG,
            label: loc.inspector_sys_event_ac,
            description: loc.inspector_sys_event_desc_ac,
            tint: t.good,
        },
        SystemEventType::PowerSourceBattery => SystemEventMeta {
            glyph: egui_phosphor::regular::BATTERY_CHARGING,
            label: loc.inspector_sys_event_battery,
            description: loc.inspector_sys_event_desc_battery,
            tint: t.warn,
        },
        SystemEventType::UserIdle => SystemEventMeta {
            glyph: egui_phosphor::regular::HOURGLASS,
            label: loc.inspector_sys_event_idle,
            description: loc.inspector_sys_event_desc_idle,
            tint: t.muted,
        },
        SystemEventType::UserActive => SystemEventMeta {
            glyph: egui_phosphor::regular::ACTIVITY,
            label: loc.inspector_sys_event_active,
            description: loc.inspector_sys_event_desc_active,
            tint: t.good,
        },
        SystemEventType::DeviceConnected => SystemEventMeta {
            glyph: egui_phosphor::regular::HARD_DRIVES,
            label: loc.inspector_sys_event_dev_connected,
            description: loc.inspector_sys_event_desc_dev_connected,
            tint: t.good,
        },
        SystemEventType::DeviceDisconnected => SystemEventMeta {
            glyph: egui_phosphor::regular::PLUG,
            label: loc.inspector_sys_event_dev_disconnected,
            description: loc.inspector_sys_event_desc_dev_disconnected,
            tint: t.warn,
        },
        SystemEventType::MonitorOn => SystemEventMeta {
            glyph: egui_phosphor::regular::DESKTOP,
            label: loc.inspector_sys_event_mon_on,
            description: loc.inspector_sys_event_desc_mon_on,
            tint: t.good,
        },
        SystemEventType::MonitorOff => SystemEventMeta {
            glyph: egui_phosphor::regular::MOON_STARS,
            label: loc.inspector_sys_event_mon_off,
            description: loc.inspector_sys_event_desc_mon_off,
            tint: t.muted,
        },
        SystemEventType::NetworkConnected => SystemEventMeta {
            glyph: egui_phosphor::regular::WIFI_HIGH,
            label: loc.inspector_sys_event_net_connected,
            description: loc.inspector_sys_event_desc_net_connected,
            tint: t.good,
        },
        SystemEventType::NetworkDisconnected => SystemEventMeta {
            glyph: egui_phosphor::regular::WIFI_SLASH,
            label: loc.inspector_sys_event_net_disconnected,
            description: loc.inspector_sys_event_desc_net_disconnected,
            tint: t.bad,
        },
    }
}

/// Mutually exclusive system-event pairs offered as one two-cell toggle.
const SYSTEM_EVENT_PAIRS: [(SystemEventType, SystemEventType); 7] = [
    (SystemEventType::SessionLock, SystemEventType::SessionUnlock),
    (SystemEventType::SystemSleep, SystemEventType::SystemResume),
    (SystemEventType::PowerSourceAc, SystemEventType::PowerSourceBattery),
    (SystemEventType::UserIdle, SystemEventType::UserActive),
    (SystemEventType::DeviceConnected, SystemEventType::DeviceDisconnected),
    (SystemEventType::MonitorOn, SystemEventType::MonitorOff),
    (SystemEventType::NetworkConnected, SystemEventType::NetworkDisconnected),
];

/// Two-cell toggle for the pair an event belongs to. Unpaired events render nothing.
fn render_system_event_pair_toggle(
    ui: &mut Ui,
    event_type: &mut SystemEventType,
    loc: &crate::i18n::Strings,
    t: &Tokens,
) -> bool {
    let Some((first, second)) = SYSTEM_EVENT_PAIRS
        .iter()
        .copied()
        .find(|(first, second)| *event_type == *first || *event_type == *second)
    else {
        return false;
    };

    let mut is_first = *event_type == first;
    let options = [
        (true, Cell::new(system_event_meta(first, loc, t).label, "")),
        (false, Cell::new(system_event_meta(second, loc, t).label, "")),
    ];
    let changed = widgets::cells(ui, &mut is_first, &options);
    if changed {
        *event_type = if is_first { first } else { second };
    }
    ui.add_space(4.0);
    changed
}

/// Toggle one weekday in a days-of-week list. An empty list means "every day", so the
/// first toggle expands it to all seven days before removing the clicked one.
fn toggle_day_of_week(days_of_week: &mut Vec<u8>, day: u8) {
    if days_of_week.is_empty() {
        *days_of_week = (1..=7).collect();
        days_of_week.retain(|&d| d != day);
    } else if days_of_week.contains(&day) {
        days_of_week.retain(|&d| d != day);
    } else {
        days_of_week.push(day);
        days_of_week.sort_unstable();
    }
}

/// Weekday selector shared by the schedule trigger and the time-range condition.
fn render_days_of_week_selector(ui: &mut Ui, loc: &crate::i18n::Strings, days_of_week: &mut Vec<u8>) {
    let dow_names = [
        (1, loc.inspector_dow_mon),
        (2, loc.inspector_dow_tue),
        (3, loc.inspector_dow_wed),
        (4, loc.inspector_dow_thu),
        (5, loc.inspector_dow_fri),
        (6, loc.inspector_dow_sat),
        (7, loc.inspector_dow_sun),
    ];

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        for (val, label) in dow_names {
            let is_on = days_of_week.is_empty() || days_of_week.contains(&val);
            if widgets::button(ui, label, if is_on { Kind::Primary } else { Kind::Secondary }, 0.0).clicked() {
                toggle_day_of_week(days_of_week, val);
            }
        }
        let all_active = days_of_week.is_empty() || days_of_week.len() == 7;
        if widgets::button(ui, loc.inspector_btn_all_days, if all_active { Kind::Primary } else { Kind::Secondary }, 0.0).clicked() {
            days_of_week.clear();
        }
    });
}

fn render_process_picker(ui: &mut Ui, loc: &crate::i18n::Strings) -> bool {
    widgets::button(ui, loc.inspector_pick_running, Kind::Secondary, 0.0)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// One process-name row: right-aligned picker button followed by the editable name.
/// Returns `true` when the picker was clicked. The caller draws its own HUD label.
fn render_process_name_row(
    ui: &mut Ui,
    loc: &crate::i18n::Strings,
    process_name: &mut String,
    hint: &str,
) -> bool {
    let mut picked = false;
    ui.horizontal(|ui| {
        ui.set_width(ui.available_width());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            picked = render_process_picker(ui, loc);
            ui.add(
                egui::TextEdit::singleline(process_name)
                    .hint_text(hint)
                    .desired_width(ui.available_width())
                    .min_size(vec2(0.0, theme::CONTROL_H))
                    .align(egui::Align2::LEFT_CENTER)
                    .margin(vec2(10.0, 8.0)),
            );
        });
    });
    picked
}

/// Optional-string variant of [`render_process_name_row`]: an empty (or whitespace-only)
/// name clears the field, and the draft is written only when the text actually changed.
fn render_optional_process_name_row(
    ui: &mut Ui,
    loc: &crate::i18n::Strings,
    process_name: &mut Option<String>,
    hint: &str,
) -> bool {
    let mut buffer = process_name.clone().unwrap_or_default();
    let mut picked = false;
    ui.horizontal(|ui| {
        ui.set_width(ui.available_width());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            picked = render_process_picker(ui, loc);
            if ui.add(
                egui::TextEdit::singleline(&mut buffer)
                    .hint_text(hint)
                    .desired_width(ui.available_width())
                    .min_size(vec2(0.0, theme::CONTROL_H))
                    .align(egui::Align2::LEFT_CENTER)
                    .margin(vec2(10.0, 8.0)),
            ).changed() {
                let trimmed = buffer.trim();
                *process_name = if trimmed.is_empty() { None } else { Some(trimmed.to_string()) };
            }
        });
    });
    picked
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

fn render_datetime_picker(
    ui: &mut Ui,
    loc: &crate::i18n::Strings,
    target_datetime: &mut Option<String>,
    t: &Tokens,
    id_salt: &str,
) {
    let now = chrono::Local::now();
    // An unset timestamp is rendered as tomorrow noon but is written into the draft only
    // once the user actually picks a day or edits a field; opening a rule must not dirty it.
    let fallback_day = now.date_naive().succ_opt().unwrap_or(now.date_naive());

    let (mut sel_year, mut sel_month, mut sel_day, mut sel_hour, mut sel_min, mut sel_sec) = {
        let dt_str = target_datetime.as_deref().unwrap_or("");
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(dt_str, "%Y-%m-%d %H:%M:%S") {
            (dt.year(), dt.month(), dt.day(), dt.hour(), dt.minute(), dt.second())
        } else if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(&format!("{}:00", dt_str), "%Y-%m-%d %H:%M:%S") {
            (dt.year(), dt.month(), dt.day(), dt.hour(), dt.minute(), 0)
        } else {
            (fallback_day.year(), fallback_day.month(), fallback_day.day(), 12, 0, 0)
        }
    };

    let view_month_id = ui.scope_id().with(id_salt).with("cal_view_month");
    let view_year_id = ui.scope_id().with(id_salt).with("cal_view_year");
    let mut view_month: u32 = ui.data_mut(|d| d.get_temp(view_month_id).unwrap_or(sel_month));
    let mut view_year: i32 = ui.data_mut(|d| d.get_temp(view_year_id).unwrap_or(sel_year));

    widgets::hud(ui, loc.inspector_mini_calendar, t.muted);
    ui.horizontal(|ui| {
        if widgets::button(ui, "◀", Kind::Secondary, 24.0).clicked() {
            if view_month == 1 {
                view_month = 12;
                view_year -= 1;
            } else {
                view_month -= 1;
            }
            ui.data_mut(|d| {
                d.insert_temp(view_month_id, view_month);
                d.insert_temp(view_year_id, view_year);
            });
        }

        let month_name = if (1..=12).contains(&view_month) {
            loc.inspector_cal_month_names[(view_month - 1) as usize]
        } else {
            ""
        };
        ui.label(egui::RichText::new(format!("{} {}", month_name, view_year)).strong());

        if widgets::button(ui, "▶", Kind::Secondary, 24.0).clicked() {
            if view_month == 12 {
                view_month = 1;
                view_year += 1;
            } else {
                view_month += 1;
            }
            ui.data_mut(|d| {
                d.insert_temp(view_month_id, view_month);
                d.insert_temp(view_year_id, view_year);
            });
        }
    });

    if let Some(first_day) = chrono::NaiveDate::from_ymd_opt(view_year, view_month, 1) {
        let start_col = first_day.weekday().number_from_monday() as usize;
        let days_in_cur_month = days_in_month(view_year, view_month);

        let two_digit_w = (widgets::caps(ui, "00", theme::HUD_SIZE, theme::mono_bold(), theme::BUTTON_SPACING, t.ink).size().x + 28.0).max(44.0);

        egui::Grid::new(ui.scope_id().with(id_salt).with("mini_cal_grid"))
            .spacing(vec2(3.0, 3.0))
            .show(ui, |ui| {
                for d in loc.inspector_cal_day_names {
                    let (rect, _) = ui.allocate_exact_size(vec2(two_digit_w, 18.0), Sense::hover());
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        d,
                        egui::FontId::new(11.0, theme::mono()),
                        t.muted,
                    );
                }
                ui.end_row();

                let mut col = 1;
                for _ in 1..start_col {
                    ui.allocate_exact_size(vec2(two_digit_w, theme::CONTROL_H), Sense::hover());
                    col += 1;
                }

                for day in 1..=days_in_cur_month {
                    let is_selected = sel_year == view_year && sel_month == view_month && sel_day == day;
                    let day_str = format!("{}", day);
                    let btn = widgets::button(
                        ui,
                        &day_str,
                        if is_selected { Kind::Primary } else { Kind::Secondary },
                        two_digit_w,
                    );
                    if btn.clicked() {
                        sel_year = view_year;
                        sel_month = view_month;
                        sel_day = day;
                        *target_datetime = Some(format!(
                            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                            sel_year, sel_month, sel_day, sel_hour, sel_min, sel_sec
                        ));
                    }
                    col += 1;
                    if col > 7 {
                        ui.end_row();
                        col = 1;
                    }
                }
                if col != 1 {
                    ui.end_row();
                }
            });
    }

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        widgets::hud(ui, loc.inspector_execution_time, t.muted);
        let h_ch = ui.add(
            egui::DragValue::new(&mut sel_hour)
                .range(0..=23)
                .speed(0.1)
                .suffix(" h"),
        ).changed();
        ui.label(":");
        let m_ch = ui.add(
            egui::DragValue::new(&mut sel_min)
                .range(0..=59)
                .speed(0.1)
                .suffix(" m"),
        ).changed();
        ui.label(":");
        let s_ch = ui.add(
            egui::DragValue::new(&mut sel_sec)
                .range(0..=59)
                .speed(0.1)
                .suffix(" s"),
        ).changed();

        if h_ch || m_ch || s_ch {
            *target_datetime = Some(format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                sel_year, sel_month, sel_day, sel_hour, sel_min, sel_sec
            ));
        }
    });
}

/// Interval presets as one table: the label, the value it sets and its tooltip.
/// Keeping them together means a label change can no longer silently drop the tip.
fn interval_presets(loc: &crate::i18n::Strings) -> [(&'static str, u64, &'static str); 7] {
    [
        ("1S", 1, loc.inspector_preset_tip_1s),
        ("1M", 60, loc.inspector_preset_tip_1m),
        ("1H", 3600, loc.inspector_preset_tip_1h),
        ("1D", 86400, loc.inspector_preset_tip_1d),
        ("1W", 604800, loc.inspector_preset_tip_1w),
        ("1MO", 2592000, loc.inspector_preset_tip_1mo),
        ("1Y", 31536000, loc.inspector_preset_tip_1y),
    ]
}

#[allow(clippy::too_many_arguments)]
fn render_schedule_trigger(
    ui: &mut Ui,
    loc: &crate::i18n::Strings,
    schedule_type: ScheduleType,
    interval_sec: &mut u64,
    daily_time: &mut Option<String>,
    days_of_week: &mut Vec<u8>,
    day_of_month: &mut Option<u8>,
    target_datetime: &mut Option<String>,
    t: &Tokens,
) {
    match schedule_type {
        ScheduleType::Interval => {
            widgets::hud(ui, loc.inspector_preset_intervals, t.muted);

            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                for (label, secs, tip) in interval_presets(loc) {
                    let on = *interval_sec == secs;
                    let btn = widgets::button(ui, label, if on { Kind::Primary } else { Kind::Secondary }, 0.0)
                        .on_hover_text(tip);
                    if btn.clicked() {
                        *interval_sec = secs;
                    }
                }
            });

            ui.add_space(4.0);
            widgets::hud(ui, loc.inspector_custom_interval, t.muted);

            let s = *interval_sec;
            let mut hours = s / 3600;
            let mut mins = (s % 3600) / 60;
            let mut secs = s % 60;

            ui.horizontal_wrapped(|ui| {
                let h_ch = ui.add(
                    egui::DragValue::new(&mut hours)
                        .range(0..=u64::MAX / 3600)
                        .speed(0.2)
                        .suffix(" h"),
                ).changed();
                ui.label(":");
                let m_ch = ui.add(
                    egui::DragValue::new(&mut mins)
                        .range(0..=59)
                        .speed(0.2)
                        .suffix(" m"),
                ).changed();
                ui.label(":");
                let s_ch = ui.add(
                    egui::DragValue::new(&mut secs)
                        .range(0..=59)
                        .speed(0.2)
                        .suffix(" s"),
                ).changed();

                if h_ch || m_ch || s_ch {
                    let total = hours
                        .saturating_mul(3600)
                        .saturating_add(mins.saturating_mul(60))
                        .saturating_add(secs);
                    *interval_sec = if total == 0 { 1 } else { total };
                }
            });

            ui.add_space(8.0);
            widgets::hrule(ui);
            let mut has_start_time = target_datetime.is_some();
            if ui.checkbox(&mut has_start_time, loc.inspector_checkbox_first_execution).changed() {
                if has_start_time {
                    let now = chrono::Local::now();
                    *target_datetime = Some(format!(
                        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                        now.year(), now.month(), now.day(), now.hour(), now.minute(), now.second()
                    ));
                } else {
                    *target_datetime = None;
                }
            }

            if has_start_time {
                ui.add_space(4.0);
                render_datetime_picker(ui, loc, target_datetime, t, "interval_cal");
            }
        }

        ScheduleType::Daily => {
            widgets::hud(ui, loc.inspector_active_days_of_week, t.muted);
            render_days_of_week_selector(ui, loc, days_of_week);

            ui.add_space(4.0);

            let cur_str = daily_time.as_deref().unwrap_or("12:00:00");
            let (mut hour, mut min, mut sec) = {
                let parts: Vec<&str> = cur_str.split(':').collect();
                let h = parts.first().and_then(|s| s.parse::<u32>().ok()).unwrap_or(12);
                let m = parts.get(1).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
                let s = parts.get(2).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
                (h, m, s)
            };

            widgets::hud(ui, loc.inspector_daily_execution_time, t.muted);
            ui.horizontal_wrapped(|ui| {
                let h_ch = ui.add(
                    egui::DragValue::new(&mut hour)
                        .range(0..=23)
                        .speed(0.1)
                        .suffix(" h"),
                ).changed();
                ui.label(":");
                let m_ch = ui.add(
                    egui::DragValue::new(&mut min)
                        .range(0..=59)
                        .speed(0.1)
                        .suffix(" m"),
                ).changed();
                ui.label(":");
                let s_ch = ui.add(
                    egui::DragValue::new(&mut sec)
                        .range(0..=59)
                        .speed(0.1)
                        .suffix(" s"),
                ).changed();

                if h_ch || m_ch || s_ch {
                    *daily_time = Some(format!("{:02}:{:02}:{:02}", hour, min, sec));
                }
            });
        }

        ScheduleType::Monthly => {
            widgets::hud(ui, loc.inspector_sched_day_of_month, t.muted);
            // Unset renders as day 1; the value is stored only when the user changes it.
            let mut cur_day = day_of_month.unwrap_or(1);
            if ui.add(
                egui::DragValue::new(&mut cur_day)
                    .range(1..=31)
                    .speed(0.1)
                    .suffix(" d"),
            ).changed() {
                *day_of_month = Some(cur_day.clamp(1, 31));
            }

            ui.add_space(8.0);

            let cur_str = daily_time.as_deref().unwrap_or("12:00:00");
            let (mut hour, mut min, mut sec) = {
                let parts: Vec<&str> = cur_str.split(':').collect();
                let h = parts.first().and_then(|s| s.parse::<u32>().ok()).unwrap_or(12);
                let m = parts.get(1).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
                let s = parts.get(2).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
                (h, m, s)
            };

            widgets::hud(ui, loc.inspector_daily_execution_time, t.muted);
            ui.horizontal_wrapped(|ui| {
                let h_ch = ui.add(
                    egui::DragValue::new(&mut hour)
                        .range(0..=23)
                        .speed(0.1)
                        .suffix(" h"),
                ).changed();
                ui.label(":");
                let m_ch = ui.add(
                    egui::DragValue::new(&mut min)
                        .range(0..=59)
                        .speed(0.1)
                        .suffix(" m"),
                ).changed();
                ui.label(":");
                let s_ch = ui.add(
                    egui::DragValue::new(&mut sec)
                        .range(0..=59)
                        .speed(0.1)
                        .suffix(" s"),
                ).changed();

                if h_ch || m_ch || s_ch {
                    *daily_time = Some(format!("{:02}:{:02}:{:02}", hour, min, sec));
                }
            });
        }

        ScheduleType::OneShot => {
            render_datetime_picker(ui, loc, target_datetime, t, "oneshot_cal");
        }

        _ => {}
    }
}

fn render_inspector_tag_section(
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
            let color = tag.parse_color(t.accent_ink);
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
                            let c = ext.parse_color(t.accent_ink);
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
                        let c = crate::models::RuleTag::new("").with_color(hex).parse_color(t.accent_ink);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::state::{GuiState, ProcessPickerTarget};
    use crate::models::RuleControlOperation;

    #[test]
    fn test_rule_control_action_editor_renders() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);

        // The picker lists rules from the daemon snapshot or, when there is none, from disk.
        let target = Rule {
            id: "rule_target".into(),
            name: "Target Rule".into(),
            ..Default::default()
        };
        crate::config::RulesFile::save_rules_to(
            &crate::config::AppConfig::config_dir(),
            std::slice::from_ref(&target),
        )
        .expect("write the isolated test rules file");

        let state = GuiState::new();
        let mut action = Action::RuleControl {
            rule_id: String::new(),
            operation: RuleControlOperation::Call,
            wait: true,
        };

        for operation in [RuleControlOperation::Call, RuleControlOperation::Stop] {
            if let Action::RuleControl { operation: current, .. } = &mut action {
                *current = operation;
            }
            for _ in 0..3 {
                let mut out = ctx.run_ui(Default::default(), |ctx| {
                    let t = Tokens::get(ctx);
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let _ = render_action_parameter_fields(
                            ui,
                            &state,
                            &mut action,
                            0,
                            false,
                            ProcessPickerTarget::Action(0),
                            &t,
                            0,
                        );
                    });
                });
                out.textures_delta.clear();
            }
        }

        match action {
            Action::RuleControl { rule_id, wait, .. } => {
                assert_eq!(rule_id, "", "rendering must not auto-select a target rule");
                assert!(!wait, "the wait flag is cleared when the operation is stop");
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_drag_preview_drops_a_stale_source_index() {
        let actions = vec![
            Action::Notification { title: "a".into(), message: String::new() },
            Action::Notification { title: "b".into(), message: String::new() },
        ];

        // An undo can replace the draft with a shorter snapshot while the drag session is
        // still alive in egui's temporary data. The stale index must drop the session, not
        // index past the end of the vector.
        let mut stale = Some(DndDragSession { source_idx: 5, target_idx: 5, card_width: 300.0 });
        assert!(dnd_preview_action(&actions, &mut stale).is_none());
        assert!(stale.is_none(), "the stale session is dropped");

        let mut valid = Some(DndDragSession { source_idx: 1, target_idx: 0, card_width: 300.0 });
        assert!(dnd_preview_action(&actions, &mut valid).is_some());
        assert!(valid.is_some(), "a fitting session survives");
    }

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
                        false,
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

    #[test]
    fn test_archive_format_cells_cover_every_variant() {
        let loc = crate::i18n::get(crate::i18n::Language::English);
        let formats: Vec<crate::models::ArchiveFormat> = ARCHIVE_FORMAT_ORDER.to_vec();
        assert_eq!(formats.len(), 6, "one cell per ArchiveFormat variant");
        let distinct: std::collections::HashSet<_> = ARCHIVE_FORMAT_ORDER.iter().collect();
        assert_eq!(distinct.len(), formats.len(), "no variant is listed twice");
        assert!(formats.contains(&crate::models::ArchiveFormat::TarXz));

        for format in formats {
            let cell = archive_format_cell(format, loc);
            assert!(!cell.label.is_empty(), "{format:?} has no label");
            assert!(!archive_format_ext(format).is_empty(), "{format:?} has no extension");
        }
    }

    #[test]
    fn test_interval_preset_tips_travel_with_the_entry() {
        let loc = crate::i18n::get(crate::i18n::Language::English);
        let presets = interval_presets(loc);
        assert_eq!(presets.len(), 7);
        for (label, seconds, tip) in presets {
            assert!(!label.is_empty());
            assert!(seconds > 0);
            assert!(!tip.is_empty(), "preset {label} lost its tooltip");
        }
        assert_eq!(presets[1], ("1M", 60, loc.inspector_preset_tip_1m));
    }

    #[test]
    fn test_day_of_week_toggle_expands_all_days_first() {
        let mut days: Vec<u8> = Vec::new();
        toggle_day_of_week(&mut days, 3);
        assert_eq!(days, vec![1, 2, 4, 5, 6, 7]);
        toggle_day_of_week(&mut days, 3);
        assert_eq!(days, vec![1, 2, 3, 4, 5, 6, 7]);
        toggle_day_of_week(&mut days, 3);
        assert_eq!(days, vec![1, 2, 4, 5, 6, 7]);
    }

    #[test]
    fn test_undo_gesture_pushes_one_snapshot_per_edit() {
        let ctx = egui::Context::default();
        let mut state = GuiState::new();
        let mut rule = Rule {
            id: "gesture_rule".into(),
            name: "Gesture".into(),
            ..Default::default()
        };
        state.editing_rule = Some(rule.clone());

        // Three changed frames of one drag must not fill the history with clones.
        for value in [1.0, 2.0, 3.0] {
            let pre_rule = rule.clone();
            rule.cooldown_seconds = value;
            settle_pending_undo(&ctx, &mut state, &pre_rule, &rule, false);
        }
        assert!(state.undo_stack.is_empty(), "no snapshot while the gesture is moving");

        // The first frame without a change commits the pre-gesture state once.
        let pre_rule = rule.clone();
        settle_pending_undo(&ctx, &mut state, &pre_rule, &rule, false);
        assert_eq!(state.undo_stack.len(), 1);
        assert_eq!(state.undo_stack[0].cooldown_seconds, 0.0);
        settle_pending_undo(&ctx, &mut state, &pre_rule, &rule, false);
        assert_eq!(state.undo_stack.len(), 1, "no second snapshot after the gesture");
    }

    #[test]
    fn test_undo_flushes_the_pending_gesture_before_undoing() {
        let ctx = egui::Context::default();
        let mut state = GuiState::new();
        let pre_rule = Rule {
            id: "gesture_rule".into(),
            name: "Gesture".into(),
            ..Default::default()
        };
        let mut rule = pre_rule.clone();
        rule.cooldown_seconds = 5.0;
        state.editing_rule = Some(rule.clone());

        settle_pending_undo(&ctx, &mut state, &pre_rule, &rule, false);
        flush_pending_undo(&ctx, &mut state);
        state.undo();

        assert_eq!(
            state.editing_rule.as_ref().map(|r| r.cooldown_seconds),
            Some(0.0),
            "Ctrl+Z mid-gesture returns to the pre-gesture state"
        );
    }

    /// Every summary word comes from the catalog: the same action must read differently in EN
    /// and RU, with no unfilled template placeholder and no raw `{:?}` enum name.
    #[test]
    fn test_action_summary_preview_is_localized() {
        let en = crate::i18n::get(crate::i18n::Language::English);
        let ru = crate::i18n::get(crate::i18n::Language::Russian);

        let cases = [
            Action::SystemPower { power_type: SystemPowerType::Shutdown },
            Action::PowerPlan { plan: PowerPlanType::HighPerformance },
            Action::Notification { title: "Build".into(), message: String::new() },
            Action::PlaySound {
                file_path: String::new(),
                play_beep: true,
                beep_frequency: Some(750),
                beep_duration_ms: None,
            },
            Action::Wait {
                mode: crate::models::WaitMode::Timer,
                seconds: 2.5,
                condition: None,
                timeout_sec: 0.0,
                timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
            },
            Action::Display {
                width: Some(1920),
                height: Some(1080),
                refresh_rate: Some(144),
                monitor_index: Some(2),
            },
            Action::AudioFormat {
                target: crate::models::AudioTarget::Microphone,
                device_name: None,
                sample_rate: 48000,
                bit_depth: 24,
                channels: 2,
            },
            Action::IfElse {
                conditions: Vec::new(),
                then_actions: vec![],
                else_if_branches: Vec::new(),
                else_actions: Vec::new(),
            },
            Action::RuleControl { rule_id: "rule_a".into(), operation: RuleControlOperation::Call, wait: true },
            Action::DiscordRpcClear,
        ];

        for action in &cases {
            let en_text = action_summary_preview(action, en);
            let ru_text = action_summary_preview(action, ru);
            assert!(!en_text.is_empty(), "{action:?} has an empty EN summary");
            assert!(!ru_text.is_empty(), "{action:?} has an empty RU summary");
            assert!(!en_text.contains("{}"), "unfilled placeholder in {en_text}");
            assert!(!ru_text.contains("{}"), "unfilled placeholder in {ru_text}");
            assert_ne!(en_text, ru_text, "{action:?} renders the same text in both languages");
            assert!(!en_text.contains("HighPerformance"), "raw enum name leaked: {en_text}");
            assert!(!ru_text.contains("HighPerformance"), "raw enum name leaked: {ru_text}");
        }

        // The enum labels are the ones the parameter editor shows, not their Debug names.
        let power = Action::SystemPower { power_type: SystemPowerType::Shutdown };
        assert_eq!(
            action_summary_preview(&power, en),
            en.summary_preview_power.replace("{}", en.inspector_power_shutdown)
        );
        let plan = Action::PowerPlan { plan: PowerPlanType::HighPerformance };
        assert_eq!(
            action_summary_preview(&plan, en),
            en.summary_preview_power_plan.replace("{}", en.inspector_power_plan_high)
        );
    }

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

    /// The literals the conversion pass moved into the catalog must not come back into the
    /// view. Every needle is assembled from fragments so this test cannot match itself.
    #[test]
    fn test_inspector_source_has_no_removed_literals() {
        let source = include_str!("inspector.rs");
        let removed = [
            concat!("URL copied to ", "clipboard"),
            concat!("Gained ", "Focus"),
            concat!("Lost ", "Focus"),
            concat!("Step ", "passed"),
            concat!("Unknown ", "result"),
            concat!("Confirm ", "termination"),
            concat!("Rule is not ", "valid:"),
            concat!("WINDOW ", "OPERATION"),
            concat!("HOTKEY ", "COMBINATION"),
            concat!("HIGH ", "PERFORMANCE"),
            concat!("POWER ", "SAVER"),
            concat!("WINDOWS ", "ALERT"),
            concat!("Default Windows notification ", "alert beep"),
            concat!("Application .exe or ", "window title"),
            concat!("Content to write... Tokens like ", "{ProcessName}"),
            concat!("\"REC", "ORD\""),
            concat!("● LISTEN", "ING..."),
            concat!("(0 = ", "infinite)"),
            concat!("\"Monitor", " #\""),
            concat!("\"W:", " \""),
            concat!("\"H:", " \""),
            concat!("Pres", "et: "),
            concat!("\"Pres", "et\""),
            concat!("\"TH", "EN\""),
            concat!("\"EL", "SE\""),
            concat!("EL", "IF {:02}"),
            concat!("\"PA", "SS\""),
            concat!("\"FA", "IL\""),
            concat!("Select Rule ", "Icon"),
            concat!("Image or ", "Exe"),
            concat!("Execu", "tables"),
            concat!("Audio ", "File"),
            concat!("Archive ", "Files"),
            concat!("All ", "Files"),
            concat!("{} ", "Archive"),
            concat!("sub_card_ghost\", parent_idx, ", "branch_label"),
            concat!("sub_test_res\", parent_idx, ", "branch_label"),
        ];
        for literal in removed {
            assert!(!source.contains(literal), "hardcoded UI literal is back: {literal}");
        }
    }
}
