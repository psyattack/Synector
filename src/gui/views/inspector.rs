//! Inspector Drawer: Slides in from the right edge with cubic easing,
//! dimmed background scrim, and craft-launcher styling for editing rules
//! and process details.

use egui::{
    Color32, CornerRadius, Id, Order, Pos2, Rect, RichText, Sense, Stroke, Ui, pos2, vec2,
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

mod actions;
mod cards;
mod conditions;
mod dnd;
mod hotkey;
mod pipeline;
mod process;
mod schedule;
mod sections;
mod summary;
mod tags;
mod trigger;

pub(crate) use actions::*;
pub(crate) use cards::*;
pub(crate) use conditions::*;
pub(crate) use dnd::*;
pub(crate) use hotkey::*;
pub(crate) use pipeline::*;
pub(crate) use process::*;
pub(crate) use schedule::*;
pub(crate) use sections::*;
pub(crate) use summary::*;
pub(crate) use tags::*;
pub(crate) use trigger::*;


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

    // Step-test outcomes arrive through the IPC worker on a later frame; the cards read their
    // badge from egui temp data under the id they computed when the user pressed Test.
    for (result_id, value) in state.take_test_step_results() {
        ctx.data_mut(|data| data.insert_temp(result_id, value));
    }

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

    let k = theme::motion::animate_bool(
        ctx,
        Id::unique("inspector-slide"),
        is_open,
        t.metrics().motion.drawer,
    );
    if k <= 0.001 && !is_open {
        return;
    }

    let full = ctx.content_rect();
    let top = (full.top() + t.metrics().header_h).min(full.bottom());
    let bottom = (full.bottom() - t.metrics().footer_h).max(top);
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
            let drawer_corner = t.drawer_corner();
            p.add(theme::shadow(&t).as_shape(panel, drawer_corner));
            p.rect_filled(panel, drawer_corner, t.bg);

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
                    .corner_radius(t.card_corner())
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
                                widgets::hud(ui, loc.inspector_eyebrow_process_details, t.muted);
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
            // One footer height for both skins: the historical 52 px strip, and one content
            // height too. The skin's control row (34 px classic, 38 px modern) used to make the
            // bar's contents different heights, so the strip is centred around the classic row in
            // both generations and the footer reads the same after a skin switch.
            let strip_h = 52.0;
            let footer_control_h = theme::CONTROL_H;
            let footer_margin = ((strip_h - footer_control_h) / 2.0).max(0.0).round() as i8;
            let footer_h = if is_editing_rule {
                strip_h
                    + if save_errors.is_empty() {
                        0.0
                    } else {
                        // Generous: the frame must stay inside this guess, or its bottom edge is
                        // clipped by the strip's square corner.
                        24.0 + save_errors.len() as f32 * 20.0
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
                    // The strip is cut square by the drawer's edge divider: a rounded bottom-left
                    // corner would curl away from that line and leave the corner visibly apart
                    // from the separator.
                    let footer_corner = egui::CornerRadius::ZERO;
                    egui::Frame::new()
                        .fill(t.sunken)
                        // No frame border: the drawer's own edge divider already runs along
                        // the left side, and a full border painted a second, slightly offset
                        // vertical line on top of it. Only the top rule is drawn below.
                        .stroke(Stroke::NONE)
                        .corner_radius(footer_corner)
                        .inner_margin(egui::Margin::symmetric(20, footer_margin))
                        .show(ui, |ui| {
                            let loc = state.loc();
                            // Fill the strip instead of growing from its content, so the frame
                            // ends exactly on the drawer's bottom edge.
                            ui.set_min_height(
                                (footer_rect.height() - f32::from(footer_margin) * 2.0).max(0.0),
                            );
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

                                if widgets::button_with_height(ui, loc.inspector_btn_save_rule, Kind::Primary, 120.0, footer_control_h).clicked() {
                                    // Same contract as `--rules-apply`: validate the draft with the
                                    // shared model validator and keep the editor open on errors.
                                    let mut candidate = rule.clone();
                                    if candidate.name.trim().is_empty() {
                                        candidate.name = loc.rules_default_new_name.to_string();
                                    }
                                    let errors = crate::validation::validate_rules(std::slice::from_ref(&candidate));
                                    if errors.is_empty() {
                                        store_save_errors(ui.ctx(), &rule.id, Vec::new());
                                        state.save_rule(candidate);
                                    } else {
                                        store_save_errors(ui.ctx(), &rule.id, errors);
                                    }
                                }

                                if !state.is_creating_new_rule
                                    && widgets::button_with_height(ui, loc.inspector_btn_clone, Kind::Secondary, 0.0, footer_control_h)
                                        .clicked()
                                {
                                    state.clone_rule(&rule);
                                }

                                if state.is_creating_new_rule
                                    && widgets::button_with_height(ui, loc.inspector_btn_reset, Kind::Secondary, 0.0, footer_control_h)
                                        .clicked()
                                {
                                    state.reset_builder_state();
                                }

                                if widgets::button_with_height(ui, loc.inspector_btn_cancel, Kind::Secondary, 0.0, footer_control_h).clicked() {
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

            // The only border the action strip needs: its top rule. The drawer's edge divider
            // is the left border and the panel edge is the bottom one. A process inspector has
            // no action strip, so it must not get a stray rule at the drawer's bottom edge.
            if footer_h > 0.0 {
                ui.painter()
                    .with_clip_rect(panel)
                    .hline(footer_rect.x_range(), footer_rect.top() + 0.5, t.hairline());
            }

            // One straight rule down the whole drawer, painted last so no footer surface can dim
            // or break it. The strip's square bottom-left corner is cut by this line instead of
            // curling away from it.
            ui.painter()
                .with_clip_rect(panel)
                .vline(panel.left() + 0.5, panel.top()..=panel.bottom(), t.hairline());
        });
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// The literals the conversion pass moved into the catalog must not come back into the
    /// view. Every needle is assembled from fragments so this test cannot match itself.
    #[test]
    fn test_inspector_source_has_no_removed_literals() {
        let source = inspector_module_source();
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

    /// Concatenation of the whole `inspector` module tree. The source guard must scan every
    /// file, and this list must keep matching the `mod` declarations or the guard silently
    /// covers less than the module does.
    fn inspector_module_source() -> String {
        let files = [
            ("inspector.rs", include_str!("inspector.rs")),
            ("inspector/sections.rs", include_str!("inspector/sections.rs")),
            ("inspector/trigger.rs", include_str!("inspector/trigger.rs")),
            ("inspector/conditions.rs", include_str!("inspector/conditions.rs")),
            ("inspector/pipeline.rs", include_str!("inspector/pipeline.rs")),
            ("inspector/cards.rs", include_str!("inspector/cards.rs")),
            ("inspector/dnd.rs", include_str!("inspector/dnd.rs")),
            ("inspector/actions.rs", include_str!("inspector/actions.rs")),
            ("inspector/tags.rs", include_str!("inspector/tags.rs")),
            ("inspector/process.rs", include_str!("inspector/process.rs")),
            ("inspector/hotkey.rs", include_str!("inspector/hotkey.rs")),
            ("inspector/schedule.rs", include_str!("inspector/schedule.rs")),
            ("inspector/summary.rs", include_str!("inspector/summary.rs")),
            ("inspector/actions/process.rs", include_str!("inspector/actions/process.rs")),
            ("inspector/actions/flow.rs", include_str!("inspector/actions/flow.rs")),
            ("inspector/actions/system.rs", include_str!("inspector/actions/system.rs")),
            ("inspector/actions/media.rs", include_str!("inspector/actions/media.rs")),
            ("inspector/actions/network.rs", include_str!("inspector/actions/network.rs")),
            ("inspector/actions/files.rs", include_str!("inspector/actions/files.rs")),
            ("inspector/actions/macro.rs", include_str!("inspector/actions/macro.rs")),
            ("inspector/actions/overlay.rs", include_str!("inspector/actions/overlay.rs")),
        ];

        // Self-check: one entry per `mod` declaration plus this root file, and no empty entry.
        let root_modules = include_str!("inspector.rs")
            .lines()
            .filter(|line| line.starts_with("mod ") && !line.starts_with("mod tests"))
            .count();
        let action_modules = include_str!("inspector/actions.rs")
            .lines()
            .filter(|line| line.starts_with("mod ") && !line.starts_with("mod tests"))
            .count();
        assert_eq!(
            files.len(),
            root_modules + action_modules + 1,
            "the guard file list must match the inspector module tree"
        );
        for (name, source) in &files {
            assert!(!source.is_empty(), "{name} is empty");
        }

        files.iter().map(|(_, source)| *source).collect()
    }
}
