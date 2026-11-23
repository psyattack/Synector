//! Drag & drop engine of the rules list: the rule-identity session, the drop-slot resolution and
//! the row loop that hosts the slots.

use egui::{Rect, Ui, vec2};
use crate::gui::components::dnd;
use crate::gui::components::icons::ProcessIconCache;
use crate::gui::state::{GuiState, RuleSortMode};
use crate::gui::theme::{self, Tokens};
use crate::i18n::Strings;
use crate::models::Rule;

use super::row::render_track_row;
use super::{default_icon_glyph, RuleRowIntent};

/// One drag of a rule row. The grabbed rule is tracked by id, not by its position: a daemon sync
/// that inserts, removes or reorders rules mid-drag must not move a different rule than the one
/// the user grabbed (U04-007).
#[derive(Clone, Debug, Default)]
struct RuleDndSession {
    source_id: String,
    target_idx: usize,
}

/// Why drag & drop is unavailable; each reason gets its own grip hint (U04-008).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DndBlock {
    None,
    Sort,
    Filters,
}

fn dnd_block(sort_mode: RuleSortMode, has_tag_filter: bool, has_search: bool) -> DndBlock {
    if sort_mode != RuleSortMode::Default {
        DndBlock::Sort
    } else if has_tag_filter || has_search {
        DndBlock::Filters
    } else {
        DndBlock::None
    }
}

fn dnd_grip_tip(block: DndBlock, loc: &Strings) -> &'static str {
    match block {
        DndBlock::None => loc.rules_tip_drag_rule,
        DndBlock::Sort => loc.rules_tip_dnd_disabled_sort,
        DndBlock::Filters => loc.rules_tip_dnd_disabled_filter,
    }
}

/// Position of the dragged rule in the live list; `None` once it has disappeared from the snapshot.
fn dnd_source_index(session: &RuleDndSession, ids: &[&str]) -> Option<usize> {
    ids.iter().position(|id| *id == session.source_id)
}

/// `(target id, insert after target)` for a drop on `dst_slot`, resolved against the live order.
/// `None` when the move is a no-op or out of range.
fn dnd_drop_target<'a>(ids: &[&'a str], src_idx: usize, dst_slot: usize) -> Option<(Option<&'a str>, bool)> {
    if src_idx >= ids.len() || dst_slot > ids.len() || dst_slot == src_idx || dst_slot == src_idx + 1 {
        return None;
    }
    if dst_slot == ids.len() {
        ids.last().map(|last| (Some(*last), true))
    } else if dst_slot > src_idx {
        Some((Some(ids[dst_slot - 1]), true))
    } else {
        Some((Some(ids[dst_slot]), false))
    }
}

/// Rules list drop line: 16 px, 0.16 s, rail inset 16/16 and a plain 20 px bead. The corner
/// follows the skin, so a rounded generation gets a capsule instead of a rectangle.
fn rule_drop_line_style(t: &Tokens) -> dnd::DropLineStyle {
    dnd::DropLineStyle {
        height: 16.0,
        anim_secs: 0.16,
        corner: t.chip_corner(),
        fill_alpha: 0.08,
        line_width: 2.0,
        line_left: 16.0,
        line_right: 16.0,
        bead: Some(dnd::DropBead {
            left: 20.0,
            radius: 3.0,
            halo_radius: None,
        }),
    }
}

/// Rule drag preview chrome: a flat card with a 1 px accent stroke.
fn rule_preview_style(t: &Tokens) -> dnd::FloatingPreviewStyle {
    dnd::FloatingPreviewStyle {
        corner: t.card_corner(),
        stroke_width: 1.0,
        shadow: false,
    }
}

fn render_rule_drop_slot(
    ui: &mut Ui,
    slot_idx: usize,
    active_dst: Option<usize>,
    active_src: Option<usize>,
    t: &Tokens,
) {
    let is_targeted = if let (Some(dst), Some(src)) = (active_dst, active_src) {
        dst == slot_idx && slot_idx != src && slot_idx != src + 1
    } else {
        false
    };

    let anim_id = ui.scope_id().with(("rule_slot_anim", slot_idx));
    dnd::drop_line(ui, anim_id, is_targeted, &rule_drop_line_style(t), t);
}

/// `(source id, target id, insert after target)` of one committed drag.
type RuleReorderCommit = (String, Option<String>, bool);

/// Renders the rule list and runs the drag & drop session around the row loop.
pub(super) fn render_rule_list(
    ui: &mut Ui,
    state: &mut GuiState,
    order: &[usize],
    loc: &Strings,
    t: &Tokens,
) -> (Option<RuleReorderCommit>, Vec<RuleRowIntent>) {
    let block = dnd_block(
        state.rule_sort_mode,
        state.selected_tag_filter.is_some(),
        !state.search_query.trim().is_empty(),
    );
    let is_dnd_allowed = block == DndBlock::None;
    let grip_tip = dnd_grip_tip(block, loc);

    let dnd_id = egui::Id::unique("rules_view_dnd");
    let mut dnd_session = if is_dnd_allowed {
        ui.data(|d| d.get_temp::<RuleDndSession>(dnd_id))
    } else {
        None
    };
    let pointer_pos = ui.input(|i| i.pointer.hover_pos());
    let pointer_released = ui.input(|i| i.pointer.any_released());

    // Mouse wheel scrolling while dragging items vertically
    if dnd_session.is_some() {
        dnd::wheel_scroll(ui);
    }

    let is_dnd_active = dnd_session.is_some();
    let mut drag_started: Option<(usize, String)> = None;
    let mut current_centers: Vec<f32> = Vec::with_capacity(order.len());
    let mut intents: Vec<RuleRowIntent> = Vec::new();
    let mut pending_reorder: Option<(String, Option<String>, bool)> = None;

    // The row loop holds a shared borrow of the daemon snapshot and a mutable borrow of the
    // icon cache (disjoint fields) and queues every mutation for after the list (U04-011).
    {
        let rules: &[Rule] = state
            .daemon_state
            .as_ref()
            .map(|s| s.config.rules.as_slice())
            .unwrap_or(&[]);
        let selected_rule_id = state.selected_rule_id.clone();
        let exec_states: Vec<Option<crate::ipc::ActiveRuleExecutionState>> = order
            .iter()
            .map(|&idx| state.get_rule_execution_state(&rules[idx]))
            .collect();
        let test_running: Vec<bool> = order
            .iter()
            .map(|&idx| state.active_test_runs.contains_key(&rules[idx].id))
            .collect();
        let rule_ids: Vec<&str> = order.iter().map(|&idx| rules[idx].id.as_str()).collect();
        let active_src = dnd_session.as_ref().and_then(|s| dnd_source_index(s, &rule_ids));
        let active_dst = dnd_session.as_ref().map(|s| s.target_idx);
        let icon_cache: &mut ProcessIconCache = &mut state.icon_cache;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;

                for (position, &rule_idx) in order.iter().enumerate() {
                    // Drop slot indicator before this row
                    if is_dnd_allowed {
                        render_rule_drop_slot(ui, position, active_dst, active_src, t);
                    }

                    let rule = &rules[rule_idx];
                    let is_selected = selected_rule_id.as_deref() == Some(rule.id.as_str());
                    let is_being_dragged = active_src == Some(position);

                    let (row_rect, grip_started) = render_track_row(
                        ui,
                        icon_cache,
                        loc,
                        rule,
                        is_selected,
                        is_being_dragged,
                        is_dnd_allowed,
                        is_dnd_active,
                        exec_states[position],
                        test_running[position],
                        grip_tip,
                        &mut intents,
                        t,
                    );
                    current_centers.push(row_rect.center().y);

                    if grip_started && is_dnd_allowed {
                        drag_started = Some((position, rule.id.clone()));
                    }
                }

                // Trailing drop slot after last row
                if is_dnd_allowed {
                    render_rule_drop_slot(ui, order.len(), active_dst, active_src, t);
                }
            });

        // Update target_idx using fresh current_centers
        if let (Some(session), Some(pointer)) = (&mut dnd_session, pointer_pos) {
            if !current_centers.is_empty() {
                session.target_idx = dnd::slot_from_pointer(&current_centers, pointer.y);
            }
        }

        // Commit reorder on mouse release. Both ends resolve by rule id against the live list,
        // so a snapshot change during the drag cannot move the wrong rule (U04-007).
        if pointer_released {
            if let Some(session) = dnd_session.take() {
                if let Some(src_pos) = dnd_source_index(&session, &rule_ids) {
                    if let Some((target_id, insert_after)) = dnd_drop_target(&rule_ids, src_pos, session.target_idx) {
                        pending_reorder = Some((
                            session.source_id,
                            target_id.map(str::to_string),
                            insert_after,
                        ));
                    }
                }
                ui.data_mut(|d| d.remove_temp::<RuleDndSession>(dnd_id));
            }
        }

        // Handle newly started drag
        if let Some((position, rule_id)) = drag_started {
            dnd_session = Some(RuleDndSession {
                source_id: rule_id,
                target_idx: position,
            });
        }

        // Save session & paint floating preview
        if let Some(session) = &dnd_session {
            ui.data_mut(|d| d.insert_temp(dnd_id, session.clone()));
            ui.ctx().request_repaint();
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);

            if let (Some(pointer), Some(src_pos)) = (pointer_pos, dnd_source_index(session, &rule_ids)) {
                let dragged_rule = &rules[order[src_pos]];
                let float_rect = Rect::from_min_size(pointer + vec2(14.0, 10.0), vec2(180.0, 26.0));
                dnd::floating_preview(
                    ui,
                    egui::Id::unique("rule_drag_preview"),
                    float_rect,
                    &rule_preview_style(t),
                    t,
                    |p| {
                        let preview_text = format!("{} {}", default_icon_glyph(dragged_rule), dragged_rule.name);
                        p.text(
                            float_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            preview_text,
                            egui::FontId::new(11.5, theme::semibold()),
                            t.ink_strong,
                        );
                    },
                );
            }
        }
    }

    (pending_reorder, intents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{self, Language};

    #[test]
    fn test_dnd_source_and_drop_target_resolve_by_rule_id() {
        let mut session = RuleDndSession {
            source_id: "rule-b".into(),
            target_idx: 1,
        };

        // Normal list: the grabbed rule sits where it started.
        assert_eq!(dnd_source_index(&session, &["rule-a", "rule-b", "rule-c"]), Some(1));

        // A daemon sync inserted a rule above the grabbed one: the index follows the id (U04-007).
        let resynced = ["rule-x", "rule-a", "rule-b", "rule-c"];
        assert_eq!(dnd_source_index(&session, &resynced), Some(2));

        // The same live order decides where the drop lands.
        assert_eq!(dnd_drop_target(&resynced, 2, 0), Some((Some("rule-x"), false)));
        assert_eq!(dnd_drop_target(&resynced, 2, 4), Some((Some("rule-c"), true)));
        assert_eq!(dnd_drop_target(&resynced, 0, 2), Some((Some("rule-a"), true)));
        assert_eq!(dnd_drop_target(&resynced, 2, 2), None);
        assert_eq!(dnd_drop_target(&resynced, 2, 3), None, "dropping directly after itself is a no-op");
        assert_eq!(dnd_drop_target(&resynced, 9, 0), None, "a stale index must not reorder anything");
        assert_eq!(dnd_drop_target(&[], 0, 0), None);

        // A rule removed by the sync leaves nothing to grab (and nothing to commit).
        session.target_idx = 3;
        assert_eq!(dnd_source_index(&session, &["rule-a", "rule-c"]), None);
    }

    #[test]
    fn test_dnd_block_reason_selects_its_own_hint() {
        let loc = i18n::get(Language::English);
        let ru = i18n::get(Language::Russian);

        assert_eq!(dnd_block(RuleSortMode::Default, false, false), DndBlock::None);
        assert_eq!(dnd_block(RuleSortMode::NameAsc, false, false), DndBlock::Sort);
        assert_eq!(dnd_block(RuleSortMode::Default, true, false), DndBlock::Filters);
        assert_eq!(dnd_block(RuleSortMode::Default, false, true), DndBlock::Filters);

        assert_eq!(dnd_grip_tip(DndBlock::None, loc), loc.rules_tip_drag_rule);
        assert_eq!(dnd_grip_tip(DndBlock::Sort, loc), loc.rules_tip_dnd_disabled_sort);
        assert_eq!(dnd_grip_tip(DndBlock::Filters, loc), loc.rules_tip_dnd_disabled_filter);
        assert_eq!(dnd_grip_tip(DndBlock::Filters, ru), ru.rules_tip_dnd_disabled_filter);
        assert_ne!(
            dnd_grip_tip(DndBlock::Filters, loc),
            loc.rules_tip_dnd_disabled_sort,
            "a tag filter or search must not be reported as a sort-mode problem (U04-008)"
        );
    }
}
