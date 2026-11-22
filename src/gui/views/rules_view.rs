//! Automation rules view in the craft-launcher aesthetic:
//! precision segmented cells, hairline cards, numbered section eyebrows,
//! uppercase mono badges, and inverted action buttons.

use egui::{CornerRadius, pos2, Rect, Sense, Stroke, Ui, vec2};
use crate::gui::components::dnd;
use crate::gui::components::icons::ProcessIconCache;
use crate::gui::components::text::fill;
use crate::gui::components::widgets::{self, Glyph, Kind};
use crate::gui::state::{GuiState, RuleSortMode};
use crate::gui::theme::{self, Tokens};
use crate::i18n::Strings;
use crate::models::{
    Action, ArchiveFormat, ConditionOperator, PowerPlanType, Rule, RuleCondition, SystemEventType,
    SystemPowerType, Trigger, WindowActionType,
};

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

/// State mutation requested by one rule row this frame. Rows render from a shared borrow of the
/// daemon snapshot plus the icon cache, so mutations are queued and applied once the list is done
/// (U04-011); the rule clone happens only when the user actually clicks.
enum RuleRowIntent {
    Edit(Rule),
    Toggle { id: String, enabled: bool },
    Delete(String),
    Share(Rule),
    Shortcut(Rule),
    Clone(Rule),
    TestRun(String),
    StopRun(String),
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

/// True when a rule matches the tag filter and search query — the same predicate
/// `GuiState::visible_rules` applies to the "copy all" export (U05-013).
fn rule_matches_visible_filters(rule: &Rule, tag_filter: Option<&str>, needle_lower: &str) -> bool {
    if let Some(tag_name) = tag_filter {
        if !rule.tags.iter().any(|t| t.name.eq_ignore_ascii_case(tag_name)) {
            return false;
        }
    }
    if needle_lower.is_empty() {
        return true;
    }
    rule.name.to_lowercase().contains(needle_lower)
        || rule.tags.iter().any(|t| t.name.to_lowercase().contains(needle_lower))
}

/// Rule indices shown by the view, in display order: filter first, then the active sort mode.
/// The list itself keeps indices so the row loop never clones rule payloads (U04-011).
fn visible_rule_order(
    rules: &[Rule],
    tag_filter: Option<&str>,
    search_query: &str,
    sort_mode: RuleSortMode,
) -> Vec<usize> {
    let needle = search_query.trim().to_lowercase();
    let mut order: Vec<usize> = rules
        .iter()
        .enumerate()
        .filter(|(_, rule)| rule_matches_visible_filters(rule, tag_filter, &needle))
        .map(|(idx, _)| idx)
        .collect();

    match sort_mode {
        RuleSortMode::Default => {
            // Keep manual user drag & drop ordering
        }
        RuleSortMode::NameAsc => {
            order.sort_by_key(|&idx| rules[idx].name.to_lowercase());
        }
        RuleSortMode::NameDesc => {
            order.sort_by_key(|&idx| std::cmp::Reverse(rules[idx].name.to_lowercase()));
        }
        RuleSortMode::TriggerType => {
            order.sort_by_key(|&idx| trigger_sort_key(&rules[idx].trigger));
        }
        RuleSortMode::ActionCount => {
            order.sort_by_key(|&idx| std::cmp::Reverse(rules[idx].actions.len() + rules[idx].on_exit_actions.len()));
        }
        RuleSortMode::Status => {
            order.sort_by_key(|&idx| (!rules[idx].is_enabled, rules[idx].name.to_lowercase()));
        }
    }

    order
}

pub fn render_rules_view(ui: &mut Ui, state: &mut GuiState) {
    let t = Tokens::get(ui.ctx());
    let loc = state.loc();

    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 12.0;

        let total = state.daemon_state.as_ref().map_or(0, |s| s.config.rules.len());

        // 1. Top Bar: Tag Filter ComboBox + Sort ComboBox + Search Box + Import/Export/New Rule
        ui.horizontal(|ui| {
            // Tag Filter dropdown (clean text without leading glyph)
            let all_tags = state.all_distinct_tags();
            let selected_tag = state.selected_tag_filter.as_ref().and_then(|t_name| {
                all_tags.iter().find(|t| t.name.eq_ignore_ascii_case(t_name))
            });

            let current_tag_label = if let Some(tag) = selected_tag {
                tag.name.clone()
            } else {
                loc.rules_tag_filter_all.to_string()
            };

            egui::ComboBox::from_id_salt("rules_tag_filter_combo")
                .selected_text(&current_tag_label)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(state.selected_tag_filter.is_none(), loc.rules_tag_filter_all).clicked() {
                        state.selected_tag_filter = None;
                    }
                    if !all_tags.is_empty() {
                        ui.separator();
                        for tag in &all_tags {
                            let is_sel = state.selected_tag_filter.as_deref() == Some(&tag.name);
                            if ui.selectable_label(is_sel, &tag.name).clicked() {
                                state.selected_tag_filter = Some(tag.name.clone());
                            }
                        }
                    }
                })
                .response
                .widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::ComboBox, true, loc.rules_tag_filter_accessible)
                });

            ui.add_space(6.0);

            // Sort dropdown (clean text without leading glyph)
            let current_sort_label = match state.rule_sort_mode {
                RuleSortMode::Default => loc.rules_sort_default,
                RuleSortMode::NameAsc => loc.rules_sort_name_asc,
                RuleSortMode::NameDesc => loc.rules_sort_name_desc,
                RuleSortMode::TriggerType => loc.rules_sort_trigger,
                RuleSortMode::ActionCount => loc.rules_sort_actions,
                RuleSortMode::Status => loc.rules_sort_status,
            };

            egui::ComboBox::from_id_salt("rules_sort_mode_combo")
                .selected_text(current_sort_label)
                .show_ui(ui, |ui| {
                    let sort_options = [
                        (RuleSortMode::Default, loc.rules_sort_default),
                        (RuleSortMode::NameAsc, loc.rules_sort_name_asc),
                        (RuleSortMode::NameDesc, loc.rules_sort_name_desc),
                        (RuleSortMode::TriggerType, loc.rules_sort_trigger),
                        (RuleSortMode::ActionCount, loc.rules_sort_actions),
                        (RuleSortMode::Status, loc.rules_sort_status),
                    ];
                    for (mode, label) in sort_options {
                        if ui.selectable_label(state.rule_sort_mode == mode, label).clicked() {
                            state.rule_sort_mode = mode;
                        }
                    }
                })
                .response
                .widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::ComboBox, true, loc.rules_sort_accessible)
                });

            ui.add_space(6.0);

            // Search box (pinned to right of combo-boxes)
            ui.add(
                egui::TextEdit::singleline(&mut state.search_query)
                    .hint_text(loc.rules_search_placeholder)
                    .desired_width(180.0)
                    .min_size(vec2(0.0, theme::CONTROL_H))
                    .align(egui::Align2::LEFT_CENTER)
                    .margin(vec2(8.0, 7.0)),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Primary "+ NEW AUTOMATION" Button
                if widgets::button(ui, loc.rules_btn_new, Kind::Primary, 0.0).clicked() {
                    state.start_new_rule();
                }

                ui.add_space(4.0);

                // Export all automations to clipboard
                if widgets::button(ui, loc.rules_btn_copy_all, Kind::Secondary, 0.0)
                    .on_hover_text(loc.rules_btn_copy_all_tip)
                    .clicked()
                {
                    state.copy_all_rules_to_clipboard(ui.ctx());
                }

                ui.add_space(4.0);

                // Import automations from clipboard
                if widgets::button(ui, loc.rules_btn_import, Kind::Secondary, 0.0)
                    .on_hover_text(loc.rules_btn_import_tip)
                    .clicked()
                {
                    state.import_automations_from_clipboard(ui.ctx());
                }
            });
        });

        // 2. Filter and sort rules. Only indices are kept: the snapshot is borrowed in place.
        let order = {
            let rules: &[Rule] = state
                .daemon_state
                .as_ref()
                .map(|s| s.config.rules.as_slice())
                .unwrap_or(&[]);
            visible_rule_order(
                rules,
                state.selected_tag_filter.as_deref(),
                &state.search_query,
                state.rule_sort_mode,
            )
        };

        if order.is_empty() {
            ui.add_space(48.0);
            ui.vertical_centered(|ui| {
                if total == 0 {
                    widgets::hud(ui, loc.rules_empty_title, t.muted);
                    ui.add_space(12.0);
                    if widgets::button(ui, loc.rules_btn_create_first, Kind::Secondary, 0.0).clicked() {
                        state.start_new_rule();
                    }
                } else {
                    widgets::hud(ui, loc.rules_no_filtered_matches, t.muted);
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(loc.rules_no_filtered_matches_tip).color(t.faint));
                }
            });
            return;
        }

        // 3. Streamlined Track-List with Drag & Drop Reordering & Horizontal Wheel Routing
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
                            render_rule_drop_slot(ui, position, active_dst, active_src, &t);
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
                            &t,
                        );
                        current_centers.push(row_rect.center().y);

                        if grip_started && is_dnd_allowed {
                            drag_started = Some((position, rule.id.clone()));
                        }
                    }

                    // Trailing drop slot after last row
                    if is_dnd_allowed {
                        render_rule_drop_slot(ui, order.len(), active_dst, active_src, &t);
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
                        &RULE_PREVIEW_STYLE,
                        &t,
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

        if let Some((source_id, target_id, insert_after)) = pending_reorder {
            state.reorder_rule_by_id(&source_id, target_id.as_deref(), insert_after);
        }

        for intent in intents {
            match intent {
                RuleRowIntent::Edit(rule) => state.edit_rule(rule),
                RuleRowIntent::Toggle { id, enabled } => state.toggle_rule(&id, enabled),
                RuleRowIntent::Delete(id) => state.delete_rule(&id),
                RuleRowIntent::Share(rule) => state.copy_single_rule_to_clipboard(ui.ctx(), &rule),
                RuleRowIntent::Shortcut(rule) => state.create_shortcut_dialog(&rule),
                RuleRowIntent::Clone(rule) => state.clone_rule(&rule),
                RuleRowIntent::TestRun(id) => state.test_run_rule(&id),
                RuleRowIntent::StopRun(id) => state.stop_rule_execution(&id),
            }
        }
    });
}

/// Rules list drop line: 16 px, 0.16 s, rail inset 16/16 and a plain 20 px bead.
const RULE_DROP_LINE_STYLE: dnd::DropLineStyle = dnd::DropLineStyle {
    height: 16.0,
    anim_secs: 0.16,
    corner: CornerRadius::ZERO,
    fill_alpha: 0.08,
    line_width: 2.0,
    line_left: 16.0,
    line_right: 16.0,
    bead: Some(dnd::DropBead {
        left: 20.0,
        radius: 3.0,
        halo_radius: None,
    }),
};

/// Rule drag preview chrome: a flat card with a 1 px accent stroke.
const RULE_PREVIEW_STYLE: dnd::FloatingPreviewStyle = dnd::FloatingPreviewStyle {
    corner: CornerRadius::same(2),
    stroke_width: 1.0,
    shadow: false,
};

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
    dnd::drop_line(ui, anim_id, is_targeted, &RULE_DROP_LINE_STYLE, t);
}

#[allow(clippy::too_many_arguments)]
fn render_track_row(
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

fn render_track_pipeline(
    ui: &mut Ui,
    loc: &Strings,
    rule: &Rule,
    exec_opt: Option<crate::ipc::ActiveRuleExecutionState>,
    is_test_running: bool,
    t: &Tokens,
) {
    ui.spacing_mut().item_spacing.x = 6.0;

    // 0. Live RUNNING Badge before Trigger
    let is_running = exec_opt.is_some() || is_test_running;
    if is_running {
        let running_text = if let Some(exec) = &exec_opt {
            if exec.active_count > 1 {
                format!("{} {}x", loc.rules_badge_running, exec.active_count)
            } else {
                loc.rules_badge_running.to_string()
            }
        } else {
            loc.rules_badge_running.to_string()
        };
        widgets::badge(ui, &running_text, t.good);
    }

    // 1. Trigger Chip (No "TRIGGER:" prefix! Clean glyph and detail)
    let (trig_glyph, trig_label) = format_trigger_display(&rule.trigger, loc);
    let trig_chip = format!("{} {}", trig_glyph, trig_label);
    let galley = widgets::caps(ui, &trig_chip, 9.5, theme::mono(), 0.5, t.accent_ink);
    let chip_w = galley.size().x + 12.0;
    let (chip_rect, _chip_resp) = ui.allocate_exact_size(vec2(chip_w, 22.0), Sense::hover());
    ui.painter().rect(chip_rect, CornerRadius::ZERO, t.sunken, t.hairline(), egui::StrokeKind::Inside);
    widgets::paint_centred(ui, chip_rect, galley, t.accent_ink);

    // 2. Pre-conditions (if any) - NO arrow before IF
    if !rule.conditions.is_empty() {
        widgets::hud(ui, loc.rules_summary_cond_if, t.faint);
        for (c_idx, item) in rule.conditions.iter().enumerate() {
            if c_idx > 0 {
                let op_str = match item.operator {
                    ConditionOperator::And => loc.inspector_cond_op_and,
                    ConditionOperator::Or => loc.inspector_cond_op_or,
                };
                widgets::hud(ui, op_str, t.faint);
            }
            let label = format_single_condition_brief(&item.condition, loc);
            let tip = format_single_condition_tooltip(&item.condition, c_idx + 1, loc);
            widgets::badge(ui, &label, t.faint).on_hover_text(tip);
        }
    }

    // 3. Arrow divider to actions
    widgets::hud(ui, "➔", t.faint);

    // 4. Action Steps (No step numbering! Active highlight with ▶ prefix!)
    if rule.actions.is_empty() {
        widgets::hud(ui, loc.rules_summary_no_actions, t.muted);
    } else {
        for (a_idx, action) in rule.actions.iter().enumerate() {
            if a_idx > 0 {
                widgets::hud(ui, "➔", t.faint);
            }

            let is_current = exec_opt.is_some_and(|e| !e.is_exit && e.current_step == a_idx);
            render_action_chip(ui, action, is_current, t.ink, loc, t);
        }
    }

    // 5. On-Exit Actions (if any)
    if !rule.on_exit_actions.is_empty() {
        widgets::hud(ui, "➔", t.faint);
        widgets::hud(ui, loc.rules_summary_on_exit, t.accent_ink);

        for (e_idx, action) in rule.on_exit_actions.iter().enumerate() {
            if e_idx > 0 {
                widgets::hud(ui, "➔", t.faint);
            }

            let is_current = exec_opt.is_some_and(|e| e.is_exit && e.current_step == e_idx);
            render_action_chip(ui, action, is_current, t.faint, loc, t);
        }
    }

    // 6. Companion mode
    let has_companion = rule.actions.iter().any(|a| matches!(a, Action::StartProcess { close_on_exit: true, .. }));
    if has_companion {
        widgets::badge(ui, loc.rules_summary_bound_satellite, t.good);
    }
}

/// One action-step chip; the main and on-exit pipelines differ only in the non-current ink (U04-009).
fn render_action_chip(
    ui: &mut Ui,
    action: &Action,
    is_current: bool,
    ink: egui::Color32,
    loc: &Strings,
    t: &Tokens,
) {
    let brief = format_action_brief(action, loc);
    let icon = action_icon_glyph(action);

    let (chip_fill, chip_stroke, chip_ink, font) = if is_current {
        (
            t.accent.gamma_multiply(0.35),
            Stroke::new(1.5, t.accent_ink),
            t.accent_ink,
            theme::mono_bold(),
        )
    } else {
        (t.sunken, t.hairline(), ink, theme::mono())
    };

    let step_label = format_step_chip(icon, &brief, is_current);
    let galley = widgets::caps(ui, &step_label, 9.5, font, 0.5, chip_ink);
    let chip_w = galley.size().x + 12.0;
    let (chip_rect, chip_resp) = ui.allocate_exact_size(vec2(chip_w, 22.0), Sense::hover());
    ui.painter().rect(chip_rect, CornerRadius::ZERO, chip_fill, chip_stroke, egui::StrokeKind::Inside);
    widgets::paint_centred(ui, chip_rect, galley, chip_ink);

    if is_current {
        chip_resp.on_hover_text(loc.rules_summary_step_active);
    }
}

/// `▶ icon brief` for the running step, `icon brief` otherwise.
fn format_step_chip(icon: &str, brief: &str, is_current: bool) -> String {
    if is_current {
        format!("▶ {} {}", icon, brief)
    } else {
        format!("{} {}", icon, brief)
    }
}

fn measure_tags_width(ui: &egui::Ui, tags: &[crate::models::RuleTag]) -> f32 {
    let font_text = egui::FontId::new(9.5, theme::mono());
    let font_icon = egui::FontId::new(10.0, theme::phosphor());
    let mut total_w = 0.0;
    let pad_x = 5.0;
    let more_badge_w = 22.0;
    for (i, tag) in tags.iter().enumerate() {
        if i >= 3 {
            total_w += more_badge_w + 3.0;
            break;
        }
        let text_w = ui.painter().layout_no_wrap(tag.name.clone(), font_text.clone(), egui::Color32::WHITE).size().x;
        let icon_w = if let Some(ico) = &tag.icon {
            if !ico.is_empty() {
                ui.painter().layout_no_wrap(ico.clone(), font_icon.clone(), egui::Color32::WHITE).size().x + 3.5
            } else {
                0.0
            }
        } else {
            0.0
        };
        let pill_w = (pad_x + icon_w + text_w + pad_x).max(18.0);
        total_w += pill_w + 3.0;
    }
    total_w
}

fn render_card_tags_at(
    ui: &mut Ui,
    tags: &[crate::models::RuleTag],
    start_x: f32,
    top_y: f32,
    max_w: f32,
    rule_id: &str,
    t: &Tokens,
) {
    let font_text = egui::FontId::new(9.5, theme::mono());
    let font_icon = egui::FontId::new(10.0, theme::phosphor());

    let mut curr_x = start_x;
    let pill_h = 17.0;
    let total_tags = tags.len();
    let mut shown_count = 0;

    for (i, tag) in tags.iter().enumerate() {
        let tag_color = theme::tag_color(tag, t.accent_ink);
        let has_ico = tag.icon.as_ref().is_some_and(|ico| !ico.is_empty());

        let text_galley = ui.painter().layout_no_wrap(tag.name.clone(), font_text.clone(), tag_color);
        let (icon_galley, icon_w) = if let Some(ico) = &tag.icon {
            if !ico.is_empty() {
                let g = ui.painter().layout_no_wrap(ico.clone(), font_icon.clone(), tag_color);
                let w = g.size().x;
                (Some(g), w)
            } else {
                (None, 0.0)
            }
        } else {
            (None, 0.0)
        };

        let pad_x = 5.0;
        let gap = if has_ico { 3.5 } else { 0.0 };
        let pill_w = (pad_x + icon_w + gap + text_galley.size().x + pad_x).max(18.0);
        let more_badge_w = 22.0;

        let has_more = i + 1 < total_tags;
        let needed_w = if has_more { pill_w + 3.0 + more_badge_w } else { pill_w };

        if curr_x - start_x + needed_w > max_w && shown_count > 0 {
            break;
        }

        let pill_rect = Rect::from_min_size(pos2(curr_x, top_y), vec2(pill_w, pill_h));
        let _resp = ui.interact(pill_rect, ui.scope_id().with(("card_tag", rule_id, i)), Sense::hover());

        ui.painter().rect(
            pill_rect,
            CornerRadius::same(3),
            tag_color.gamma_multiply(0.18),
            Stroke::new(0.8, tag_color.gamma_multiply(0.50)),
            egui::StrokeKind::Inside,
        );

        let mut draw_x = pill_rect.left() + pad_x;
        if let Some(ig) = icon_galley {
            let icon_pos = pos2(draw_x, pill_rect.center().y - ig.size().y / 2.0);
            ui.painter().galley(icon_pos, ig, tag_color);
            draw_x += icon_w + gap;
        }

        let text_pos = pos2(draw_x, pill_rect.center().y - text_galley.size().y / 2.0);
        ui.painter().galley(text_pos, text_galley, tag_color);

        // Tautological tooltip removed: tag.name is already fully legible on the pill

        curr_x += pill_w + 3.0;
        shown_count += 1;
    }

    let hidden_count = total_tags.saturating_sub(shown_count);
    if hidden_count > 0 {
        let more_text = format!("+{}", hidden_count);
        let more_galley = ui.painter().layout_no_wrap(more_text, egui::FontId::new(8.5, theme::mono()), t.muted);
        let more_w = more_galley.size().x + 8.0;
        let more_rect = Rect::from_min_size(pos2(curr_x, top_y), vec2(more_w, pill_h));
        let more_resp = ui.interact(more_rect, ui.scope_id().with(("card_tag_more", rule_id)), Sense::hover());

        ui.painter().rect(
            more_rect,
            CornerRadius::same(3),
            t.sunken,
            Stroke::new(0.8, t.line),
            egui::StrokeKind::Inside,
        );
        let more_pos = pos2(more_rect.center().x - more_galley.size().x / 2.0, more_rect.center().y - more_galley.size().y / 2.0);
        ui.painter().galley(more_pos, more_galley, t.muted);

        let hidden_names: Vec<&str> = tags.iter().skip(shown_count).map(|t| t.name.as_str()).collect();
        more_resp.on_hover_text(hidden_names.join(", "));
    }
}

fn trigger_sort_key(trig: &Option<crate::models::Trigger>) -> usize {
    match trig {
        Some(crate::models::Trigger::Process { .. }) => 0,
        Some(crate::models::Trigger::WindowFocus { .. }) => 1,
        Some(crate::models::Trigger::GlobalHotkey { .. }) => 2,
        Some(crate::models::Trigger::Schedule { .. }) => 3,
        Some(crate::models::Trigger::Webhook { .. }) => 4,
        Some(crate::models::Trigger::FileWatcher { .. }) => 5,
        Some(crate::models::Trigger::ResourceThreshold { .. }) => 6,
        Some(crate::models::Trigger::SystemEvent { .. }) => 7,
        Some(crate::models::Trigger::Manual) | None => 8,
    }
}

/// Localized label of a system event. Every surface that used to `{:?}`-format the enum
/// (condition tooltip and badge, trigger summary) goes through this table.
fn sys_event_label(event: SystemEventType, loc: &Strings) -> &'static str {
    match event {
        SystemEventType::SessionLock => loc.inspector_sys_event_lock,
        SystemEventType::SessionUnlock => loc.inspector_sys_event_unlock,
        SystemEventType::SystemSleep => loc.inspector_sys_event_sleep,
        SystemEventType::SystemResume => loc.inspector_sys_event_resume,
        SystemEventType::PowerSourceAc => loc.inspector_sys_event_ac,
        SystemEventType::PowerSourceBattery => loc.inspector_sys_event_battery,
        SystemEventType::UserIdle => loc.inspector_sys_event_idle,
        SystemEventType::UserActive => loc.inspector_sys_event_active,
        SystemEventType::DeviceConnected => loc.inspector_sys_event_dev_connected,
        SystemEventType::DeviceDisconnected => loc.inspector_sys_event_dev_disconnected,
        SystemEventType::MonitorOn => loc.inspector_sys_event_mon_on,
        SystemEventType::MonitorOff => loc.inspector_sys_event_mon_off,
        SystemEventType::NetworkConnected => loc.inspector_sys_event_net_connected,
        SystemEventType::NetworkDisconnected => loc.inspector_sys_event_net_disconnected,
    }
}

/// Localized label of a system-power action target (`ПИТАНИЕ Выключение`, not `POWER Shutdown`).
fn power_label(power_type: SystemPowerType, loc: &Strings) -> &'static str {
    match power_type {
        SystemPowerType::Lock => loc.inspector_power_lock,
        SystemPowerType::Sleep => loc.inspector_power_sleep,
        SystemPowerType::TurnScreenOff => loc.inspector_power_screen_off,
        SystemPowerType::Restart => loc.inspector_power_restart,
        SystemPowerType::Shutdown => loc.inspector_power_shutdown,
    }
}

/// Localized label of a power plan; the catalog labels already carry the meaning, so the
/// old ` POWER` suffix is gone.
fn power_plan_label(plan: PowerPlanType, loc: &Strings) -> &'static str {
    match plan {
        PowerPlanType::Balanced => loc.inspector_power_plan_balanced,
        PowerPlanType::HighPerformance => loc.inspector_power_plan_high,
        PowerPlanType::PowerSaver => loc.inspector_power_plan_saver,
    }
}

/// Localized label of an archive format; `TarXz` has no inspector cell, so it uses the
/// dedicated chip label instead of its debug name.
fn archive_format_label(format: ArchiveFormat, loc: &Strings) -> &'static str {
    match format {
        ArchiveFormat::Zip => loc.inspector_archive_fmt_zip,
        ArchiveFormat::Tar => loc.inspector_archive_fmt_tar,
        ArchiveFormat::TarGz => loc.inspector_archive_fmt_tar_gz,
        ArchiveFormat::TarXz => loc.rules_brief_archive_tar_xz,
        ArchiveFormat::SevenZip => loc.inspector_archive_fmt_7z,
        ArchiveFormat::Rar => loc.inspector_archive_fmt_rar,
    }
}

/// One chip label per window action, including the two toggle templates that carry the
/// target process. No `WindowActionType` debug name can reach a chip.
fn window_action_label(action: WindowActionType, loc: &Strings) -> &'static str {
    match action {
        WindowActionType::Minimize => loc.rules_brief_window_minimize,
        WindowActionType::Maximize => loc.rules_brief_window_maximize,
        WindowActionType::Restore => loc.rules_brief_window_restore,
        WindowActionType::Hide => loc.rules_brief_window_hide,
        WindowActionType::Show => loc.rules_brief_window_show,
        WindowActionType::Close => loc.rules_brief_window_close,
        WindowActionType::ToggleAlwaysOnTop => loc.rules_brief_window_toggle_on_top,
        WindowActionType::AlwaysOnTop => loc.rules_brief_window_on_top,
        WindowActionType::RemoveAlwaysOnTop => loc.rules_brief_window_remove_on_top,
        WindowActionType::SetTransparency => loc.rules_brief_window_transparency,
        WindowActionType::SnapToPosition => loc.rules_brief_window_snap,
        WindowActionType::ToggleMinimizeRestore => loc.rules_brief_window_toggle_min_restore,
        WindowActionType::ToggleShowHide => loc.rules_brief_window_toggle_show_hide,
    }
}

fn format_single_condition_brief(cond: &RuleCondition, loc: &Strings) -> String {
    match cond {
        RuleCondition::ProcessRunning { process_name, must_be_running, .. } => {
            let name = if process_name.is_empty() { loc.rules_trigger_proc } else { process_name.as_str() };
            let prefix = loc.rules_brief_cond_run;
            if *must_be_running { format!("{prefix}{name}") } else { format!("!{prefix}{name}") }
        }
        RuleCondition::WindowFocused { process_name, must_be_focused, .. } => {
            let name = if process_name.is_empty() { loc.rules_trigger_window } else { process_name.as_str() };
            let prefix = loc.rules_brief_cond_focus;
            if *must_be_focused { format!("{prefix}{name}") } else { format!("!{prefix}{name}") }
        }
        RuleCondition::TimeRange { start_time, end_time, .. } => {
            format!("{}-{}", start_time, end_time)
        }
        RuleCondition::Event { event_type, .. } => {
            loc.rules_badge_cond_event.replace("{}", sys_event_label(*event_type, loc))
        }
        RuleCondition::FileWatcher { path, must_exist, .. } => {
            let fname = std::path::Path::new(path)
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| {
                    if path.is_empty() { loc.rules_brief_cond_any_file.to_string() } else { path.clone() }
                });
            let prefix = loc.rules_brief_cond_file;
            if *must_exist { format!("{prefix}{fname}") } else { format!("!{prefix}{fname}") }
        }
        RuleCondition::ResourceThreshold { process_name, cpu_percent, ram_mb, .. } => {
            let name = if process_name.is_empty() { loc.rules_brief_cond_any_res } else { process_name.as_str() };
            let mut limits = Vec::new();
            if let Some(c) = cpu_percent {
                limits.push(loc.rules_brief_cond_cpu.replace("{}", &format!("{c:.0}")));
            }
            if let Some(m) = ram_mb {
                limits.push(loc.rules_brief_cond_ram.replace("{}", &m.to_string()));
            }
            let prefix = loc.rules_brief_cond_res;
            if limits.is_empty() {
                format!("{prefix}{name}")
            } else {
                format!("{prefix}{name}({})", limits.join(","))
            }
        }
        RuleCondition::RuleRunning { rule_id, must_be_running } => {
            let name = if rule_id.is_empty() { loc.rules_brief_cond_any_rule } else { rule_id.as_str() };
            let prefix = loc.rules_brief_cond_rule;
            if *must_be_running { format!("{prefix}{name}") } else { format!("!{prefix}{name}") }
        }
    }
}

fn format_single_condition_tooltip(cond: &RuleCondition, idx: usize, loc: &crate::i18n::Strings) -> String {
    let desc = match cond {
        RuleCondition::ProcessRunning { process_name, must_be_running, .. } => {
            let p = if process_name.is_empty() { "?" } else { process_name.as_str() };
            if *must_be_running {
                loc.rules_tip_cond_process_running.replace("{}", p)
            } else {
                loc.rules_tip_cond_process_not_running.replace("{}", p)
            }
        }
        RuleCondition::WindowFocused { process_name, must_be_focused, .. } => {
            let p = if process_name.is_empty() { "?" } else { process_name.as_str() };
            if *must_be_focused {
                loc.rules_tip_cond_window_focused.replace("{}", p)
            } else {
                loc.rules_tip_cond_window_not_focused.replace("{}", p)
            }
        }
        RuleCondition::TimeRange { start_time, end_time, .. } => {
            let filled = loc.rules_tip_cond_time_range.replacen("{}", start_time, 1);
            filled.replacen("{}", end_time, 1)
        }
        RuleCondition::Event { event_type, .. } => {
            loc.rules_tip_cond_event.replace("{}", sys_event_label(*event_type, loc))
        }
        RuleCondition::FileWatcher { path, must_exist, .. } => {
            if *must_exist {
                loc.rules_tip_cond_file_exists.replace("{}", path)
            } else {
                loc.rules_tip_cond_file_missing.replace("{}", path)
            }
        }
        RuleCondition::ResourceThreshold { process_name, cpu_percent, ram_mb, .. } => {
            let p = if process_name.is_empty() { "?" } else { process_name.as_str() };
            fill(
                loc.rules_tip_cond_resource,
                &[p, &format!("{cpu_percent:?}"), &format!("{ram_mb:?}")],
            )
        }
        RuleCondition::RuleRunning { rule_id, must_be_running } => {
            let p = if rule_id.is_empty() { "?" } else { rule_id.as_str() };
            if *must_be_running {
                loc.rules_tip_cond_rule_running.replace("{}", p)
            } else {
                loc.rules_tip_cond_rule_not_running.replace("{}", p)
            }
        }
    };
    format!("{} [{}] {}", loc.inspector_conditions_badge, idx, desc)
}

fn format_action_brief(action: &Action, loc: &crate::i18n::Strings) -> String {
    match action {
        Action::StartProcess { file_path, .. } => {
            let fname = std::path::Path::new(file_path).file_name().unwrap_or_default().to_string_lossy();
            format!("{} {}", loc.summary_action_launch, fname)
        }
        Action::KillProcess { process_name, .. } => {
            format!("{} {}", loc.summary_action_kill, process_name)
        }
        Action::AudioControl { target, action_type, volume_level, process_name, left_volume, right_volume } => {
            let target_str = match target {
                crate::models::AudioTarget::Playback => loc.rules_brief_audio_sound.to_string(),
                crate::models::AudioTarget::Microphone => loc.rules_brief_audio_mic.to_string(),
                crate::models::AudioTarget::Process => {
                    loc.rules_brief_audio_proc.replace("{}", process_name.as_deref().unwrap_or("?"))
                }
            };
            match action_type {
                crate::models::AudioVolumeAction::Mute => format!("{} {}", loc.summary_action_mute, target_str),
                crate::models::AudioVolumeAction::Unmute => format!("{} {}", loc.summary_action_unmute, target_str),
                crate::models::AudioVolumeAction::ToggleMute => format!("{} {}", loc.summary_action_toggle, target_str),
                crate::models::AudioVolumeAction::SetVolume => {
                    if let (Some(l), Some(r)) = (left_volume, right_volume) {
                        fill(loc.rules_brief_audio_balance, &[&l.to_string(), &r.to_string(), &target_str])
                    } else {
                        format!("{} {}% {}", loc.summary_action_vol, volume_level, target_str)
                    }
                }
                crate::models::AudioVolumeAction::PlayBeep => loc.rules_brief_audio_beep.to_string(),
            }
        }
        Action::AudioFormat { target, sample_rate, bit_depth, .. } => {
            let t_str = match target {
                crate::models::AudioTarget::Playback | crate::models::AudioTarget::Process => loc.rules_brief_audio_out,
                crate::models::AudioTarget::Microphone => loc.rules_brief_audio_in,
            };
            fill(
                loc.rules_brief_audio_format,
                &[t_str, &sample_rate.to_string(), &bit_depth.to_string()],
            )
        }
        Action::DefaultDevice { target, device_name } => {
            let pfx = match target {
                crate::models::AudioTarget::Playback => loc.rules_brief_audio_dev_output,
                crate::models::AudioTarget::Microphone => loc.rules_brief_audio_dev_mic,
                crate::models::AudioTarget::Process => loc.rules_brief_audio_dev_process,
            };
            format!("{} {}", pfx, device_name)
        }
        Action::ExecuteScript { interpreter, .. } => match interpreter {
            crate::models::ScriptType::PowerShell => loc.inspector_script_type_powershell.to_string(),
            crate::models::ScriptType::Batch => loc.inspector_script_type_cmd.to_string(),
            crate::models::ScriptType::Lua => loc.rules_brief_script_lua.to_string(),
        },
        Action::Macro { events, repeat, speed, infinite, preset_id, operation } => {
            let op_prefix = match operation {
                crate::models::MacroOperation::Run => loc.rules_brief_macro_run,
                crate::models::MacroOperation::Stop => loc.rules_brief_macro_stop,
                crate::models::MacroOperation::Toggle => loc.rules_brief_macro_toggle,
            };
            if *operation == crate::models::MacroOperation::Stop {
                if let Some(pid) = preset_id {
                    loc.rules_brief_macro_stop_preset.replace("{}", pid)
                } else {
                    loc.rules_brief_macro_stop_all.to_string()
                }
            } else if let Some(pid) = preset_id {
                let rep_str = if *infinite {
                    " ∞".to_string()
                } else if *repeat > 1 {
                    format!(" {}x", repeat)
                } else {
                    String::new()
                };
                let spd_str = if (*speed - 1.0).abs() > 0.05 { format!(" @{:.1}x", speed) } else { String::new() };
                format!("{} [{}] {}{}", op_prefix, pid, rep_str, spd_str)
            } else {
                let rep_str = if *infinite {
                    " ∞".to_string()
                } else if *repeat > 1 {
                    format!(" {}x", repeat)
                } else {
                    String::new()
                };
                let spd_str = if (*speed - 1.0).abs() > 0.05 { format!(" @{:.1}x", speed) } else { String::new() };
                let ev_str = loc.rules_brief_macro_events.replace("{}", &events.len().to_string());
                format!("{} ({}){}{}", op_prefix, ev_str, rep_str, spd_str)
            }
        }
        Action::Notification { title, .. } => loc.rules_brief_notify.replace("{}", title),
        Action::SystemPower { power_type } => {
            format!("{} {}", loc.summary_action_power, power_label(*power_type, loc))
        }
        Action::Wait { mode, seconds, condition, .. } => match mode {
            crate::models::WaitMode::Timer => {
                format!("{} {:.1}{}", loc.summary_action_wait, seconds, loc.rules_brief_seconds)
            }
            crate::models::WaitMode::UntilCondition => {
                let desc = match condition {
                    Some(crate::models::WaitCondition::WindowOpened { process_name, .. }) => {
                        loc.rules_brief_wait_window_open.replace("{}", process_name)
                    }
                    Some(crate::models::WaitCondition::WindowClosed { process_name, .. }) => {
                        loc.rules_brief_wait_window_close.replace("{}", process_name)
                    }
                    Some(crate::models::WaitCondition::ProcessStarted { process_name, .. }) => {
                        loc.rules_brief_wait_process_start.replace("{}", process_name)
                    }
                    Some(crate::models::WaitCondition::ProcessExited { process_name, .. }) => {
                        loc.rules_brief_wait_process_exit.replace("{}", process_name)
                    }
                    Some(crate::models::WaitCondition::Hotkey { key_combination }) => {
                        loc.rules_brief_wait_hotkey.replace("{}", key_combination)
                    }
                    Some(crate::models::WaitCondition::File { path, condition }) => {
                        let fname = std::path::Path::new(path).file_name().and_then(|f| f.to_str()).unwrap_or(path);
                        match condition {
                            crate::models::WaitFileCondition::Exists => loc.rules_brief_wait_file_created.replace("{}", fname),
                            crate::models::WaitFileCondition::Deleted => loc.rules_brief_wait_file_deleted.replace("{}", fname),
                        }
                    }
                    Some(crate::models::WaitCondition::RuleStarted { rule_id }) => {
                        loc.rules_brief_wait_rule_start.replace("{}", rule_id)
                    }
                    Some(crate::models::WaitCondition::RuleStopped { rule_id }) => {
                        loc.rules_brief_wait_rule_stop.replace("{}", rule_id)
                    }
                    None => loc.rules_brief_wait_until.to_string(),
                };
                format!("{} [{}]", loc.summary_action_wait, desc)
            }
        },
        Action::Check { condition } => {
            format!("{} {}", loc.summary_action_check, format_single_condition_brief(condition, loc))
        }
        Action::HttpWebhook { .. } => loc.inspector_badge_webhook.to_string(),
        Action::Window { window_action, target_process, .. } => {
            let proc = target_process.as_deref().unwrap_or(loc.rules_brief_active_process);
            fill(window_action_label(*window_action, loc), &[proc])
        }
        Action::ProcessTuning { target_process, .. } => {
            let proc = target_process.as_deref().unwrap_or(loc.rules_brief_active_process);
            loc.rules_brief_process_tuning.replace("{}", proc)
        }
        Action::PowerPlan { plan } => power_plan_label(*plan, loc).to_string(),
        Action::PlaySound { .. } => loc.rules_brief_play_sound.to_string(),
        Action::Firewall { process_name, action } => match action {
            crate::models::FirewallAction::Block => loc.rules_brief_firewall_block.replace("{}", process_name),
            crate::models::FirewallAction::Allow => loc.rules_brief_firewall_allow.replace("{}", process_name),
            crate::models::FirewallAction::Toggle => format!("{} {}", loc.summary_action_toggle, process_name),
        },
        Action::Display { width, height, refresh_rate, monitor_index } => {
            let res = match (width, height) {
                (Some(w), Some(h)) => format!("{}x{}", w, h),
                _ => loc.rules_brief_display_auto.to_string(),
            };
            let mon_prefix = monitor_index
                .map(|m| loc.rules_brief_display_monitor.replace("{}", &m.to_string()))
                .unwrap_or_default();
            match refresh_rate {
                Some(r) => fill(loc.rules_brief_display_refresh, &[&mon_prefix, &res, &r.to_string()]),
                None => fill(loc.rules_brief_display, &[&mon_prefix, &res]),
            }
        }
        Action::IfElse { conditions, then_actions, else_actions, .. } => {
            let cond_summary = conditions
                .first()
                .map(|c| format_single_condition_brief(&c.condition, loc))
                .unwrap_or_else(|| loc.rules_brief_cond_fallback.to_string());
            let count_suffix = if conditions.len() > 1 { format!("+{}", conditions.len() - 1) } else { String::new() };
            fill(
                loc.rules_brief_ifelse,
                &[&cond_summary, &count_suffix, &then_actions.len().to_string(), &else_actions.len().to_string()],
            )
        }
        Action::TrimWorkingSet { target_process, soft_limit_mb, .. } => {
            let proc = target_process.as_deref().unwrap_or(loc.rules_brief_active_process);
            match soft_limit_mb {
                Some(mb) => fill(loc.rules_brief_trim_limit, &[proc, &mb.to_string()]),
                None => loc.rules_brief_trim.replace("{}", proc),
            }
        }
        Action::DiscordRpcSet { details, .. } => {
            if details.is_empty() {
                loc.inspector_badge_discord_rpc_set.to_string()
            } else {
                loc.rules_brief_discord_text.replace("{}", details)
            }
        }
        Action::DiscordRpcClear => loc.inspector_badge_discord_rpc_clear.to_string(),
        Action::Archive { operation, format, destination_path, .. } => {
            let fname = std::path::Path::new(destination_path).file_name().unwrap_or_default().to_string_lossy().into_owned();
            let format_label = archive_format_label(*format, loc);
            match operation {
                crate::models::ArchiveOperation::Pack => fill(loc.rules_brief_archive_pack, &[format_label, &fname]),
                crate::models::ArchiveOperation::Unpack => fill(loc.rules_brief_archive_unpack, &[format_label, &fname]),
            }
        }
        Action::FileSystem { operation, path, .. } => {
            let fname = std::path::Path::new(path).file_name().unwrap_or_default().to_string_lossy().into_owned();
            match operation {
                crate::models::FileSystemOperation::CreateFile => loc.rules_brief_fs_create_file.replace("{}", &fname),
                crate::models::FileSystemOperation::CreateDirectory => loc.rules_brief_fs_create_dir.replace("{}", &fname),
                crate::models::FileSystemOperation::Rename => loc.rules_brief_fs_rename.replace("{}", &fname),
                crate::models::FileSystemOperation::Copy => loc.rules_brief_fs_copy.replace("{}", &fname),
                crate::models::FileSystemOperation::Delete => loc.rules_brief_fs_delete.replace("{}", &fname),
                crate::models::FileSystemOperation::WriteText => loc.rules_brief_fs_write.replace("{}", &fname),
                crate::models::FileSystemOperation::AppendText => loc.rules_brief_fs_append.replace("{}", &fname),
            }
        }
        Action::Overlay { operation, .. } => {
            match operation {
                crate::models::OverlayOperation::Toggle => loc.rules_brief_overlay_toggle.to_string(),
                crate::models::OverlayOperation::Flash => loc.rules_brief_overlay_flash.to_string(),
                crate::models::OverlayOperation::Show => loc.rules_brief_overlay_show.to_string(),
                crate::models::OverlayOperation::Hide => loc.rules_brief_overlay_hide.to_string(),
                crate::models::OverlayOperation::SetText => loc.rules_brief_overlay_set_text.to_string(),
            }
        }
        Action::RuleControl { operation, .. } => match operation {
            crate::models::RuleControlOperation::Call => loc.rules_brief_rule_call.to_string(),
            crate::models::RuleControlOperation::Stop => loc.rules_brief_rule_stop.to_string(),
        },
    }
}

fn format_trigger_display(
    trigger: &Option<Trigger>,
    loc: &crate::i18n::Strings,
) -> (&'static str, String) {
    match trigger {
        Some(Trigger::Process { process_name, single_instance, .. }) => {
            let si_suffix = if *single_instance { loc.rules_summary_single_instance } else { "" };
            (
                egui_phosphor::regular::APP_WINDOW,
                format!("{} {}{}", loc.rules_trigger_proc, process_name, si_suffix),
            )
        }
        Some(Trigger::WindowFocus { process_name, on_focus_gained, on_focus_lost }) => {
            let act = match (*on_focus_gained, *on_focus_lost) {
                (true, true) => loc.rules_trigger_focus_blur,
                (true, false) => loc.rules_trigger_focus,
                (false, true) => loc.rules_trigger_blur,
                _ => loc.rules_trigger_focus,
            };
            (
                egui_phosphor::regular::BROWSERS,
                format!("{} {} {}", loc.rules_trigger_window, act, process_name),
            )
        }
        Some(Trigger::ResourceThreshold { process_name, cpu_percent, ram_mb, .. }) => {
            let mut parts = Vec::new();
            if let Some(c) = cpu_percent {
                parts.push(loc.rules_trigger_cpu.replace("{}", &c.to_string()));
            }
            if let Some(m) = ram_mb {
                parts.push(loc.rules_trigger_ram.replace("{}", &m.to_string()));
            }
            (
                egui_phosphor::regular::GAUGE,
                format!("{} ({})", process_name, parts.join(", ")),
            )
        }
        Some(Trigger::GlobalHotkey { key_combination, hold_mode, repeat_while_held }) => {
            let hold_str = if *hold_mode {
                if *repeat_while_held { loc.rules_trigger_hold_loop } else { loc.rules_trigger_hold }
            } else {
                ""
            };
            (
                egui_phosphor::regular::KEYBOARD,
                format!("{}{}", key_combination, hold_str),
            )
        }
        Some(Trigger::FileWatcher { folder_path, .. }) => {
            let name = std::path::Path::new(folder_path).file_name().and_then(|f| f.to_str()).unwrap_or(folder_path);
            (
                egui_phosphor::regular::FOLDER_NOTCH_OPEN,
                format!("{} {}", loc.rules_trigger_folder, name),
            )
        }
        Some(Trigger::Webhook { endpoint_path }) => {
            (
                egui_phosphor::regular::GLOBE,
                format!("/{}", endpoint_path.trim_start_matches('/')),
            )
        }
        Some(Trigger::SystemEvent { event_type, .. }) => {
            (
                egui_phosphor::regular::DESKTOP,
                sys_event_label(*event_type, loc).to_string(),
            )
        }
        Some(Trigger::Schedule { schedule_type, interval_sec, daily_time, day_of_month, target_datetime, .. }) => {
            let text = match schedule_type {
                crate::models::ScheduleType::Interval => {
                    let s = *interval_sec;
                    if s >= 31536000 && s % 31536000 == 0 {
                        loc.rules_summary_every_year.replace("{}", &(s / 31536000).to_string())
                    } else if s >= 2592000 && s % 2592000 == 0 {
                        loc.rules_summary_every_month.replace("{}", &(s / 2592000).to_string())
                    } else if s >= 604800 && s % 604800 == 0 {
                        loc.rules_summary_every_week.replace("{}", &(s / 604800).to_string())
                    } else if s >= 86400 && s % 86400 == 0 {
                        loc.rules_summary_every_day.replace("{}", &(s / 86400).to_string())
                    } else if s >= 3600 && s % 3600 == 0 {
                        loc.rules_summary_every_hour.replace("{}", &(s / 3600).to_string())
                    } else if s >= 60 && s % 60 == 0 {
                        loc.rules_summary_every_minute.replace("{}", &(s / 60).to_string())
                    } else {
                        loc.rules_summary_every_second.replace("{}", &s.to_string())
                    }
                }
                crate::models::ScheduleType::Daily => {
                    loc.rules_summary_trigger_daily.replace("{}", daily_time.as_deref().unwrap_or("12:00"))
                }
                crate::models::ScheduleType::Monthly => {
                    let day_str = day_of_month.unwrap_or(1).to_string();
                    let time_str = daily_time.as_deref().unwrap_or("12:00");
                    let res = loc.rules_summary_trigger_monthly.replacen("{}", &day_str, 1);
                    res.replacen("{}", time_str, 1)
                }
                crate::models::ScheduleType::OneShot => {
                    loc.rules_summary_trigger_once.replace(
                        "{}",
                        target_datetime.as_deref().unwrap_or(loc.rules_summary_trigger_once_unspecified),
                    )
                }
                crate::models::ScheduleType::AppStartup => loc.rules_summary_trigger_startup.to_string(),
                crate::models::ScheduleType::AppShutdown => loc.rules_summary_trigger_shutdown.to_string(),
            };
            (egui_phosphor::regular::CLOCK, text)
        }
        Some(crate::models::Trigger::Manual) | None => (
            egui_phosphor::regular::HAND_TAP,
            loc.rules_badge_manual.to_string(),
        ),
    }
}

/// Icon shown for one condition row. Presentation lives in the GUI, not in the model.
pub(crate) fn condition_icon_glyph(condition: &RuleCondition) -> &'static str {
    match condition {
        RuleCondition::ProcessRunning { .. } => egui_phosphor::regular::CPU,
        RuleCondition::WindowFocused { .. } => egui_phosphor::regular::APP_WINDOW,
        RuleCondition::TimeRange { .. } => egui_phosphor::regular::CLOCK,
        RuleCondition::Event { .. } => egui_phosphor::regular::BELL,
        RuleCondition::ResourceThreshold { .. } => egui_phosphor::regular::CHART_LINE,
        RuleCondition::FileWatcher { .. } => egui_phosphor::regular::FILE,
        RuleCondition::RuleRunning { .. } => egui_phosphor::regular::PLAY,
    }
}

/// Icon shown for a rule card: the first action decides, then the trigger, then a gear.
/// This is the single action-to-glyph table; `models` deliberately carries no presentation.
pub(crate) fn default_icon_glyph(rule: &Rule) -> &'static str {
    if let Some(action) = rule.actions.first() {
        return action_icon_glyph(action);
    }
    match &rule.trigger {
        Some(Trigger::Manual) => egui_phosphor::regular::PLAY,
        Some(Trigger::Process { .. }) => egui_phosphor::regular::CPU,
        Some(Trigger::WindowFocus { .. }) => egui_phosphor::regular::EYE,
        Some(Trigger::ResourceThreshold { .. }) => egui_phosphor::regular::CHART_LINE,
        Some(Trigger::GlobalHotkey { .. }) => egui_phosphor::regular::COMMAND,
        Some(Trigger::SystemEvent { .. }) => egui_phosphor::regular::LIGHTNING,
        Some(Trigger::Schedule { .. }) => egui_phosphor::regular::CALENDAR,
        Some(Trigger::FileWatcher { .. }) => egui_phosphor::regular::FOLDER_NOTCH_OPEN,
        Some(Trigger::Webhook { .. }) => egui_phosphor::regular::GLOBE_HEMISPHERE_WEST,
        None => egui_phosphor::regular::GEAR,
    }
}

fn action_icon_glyph(action: &Action) -> &'static str {
    match action {
        Action::AudioControl { target, .. } => match target {
            crate::models::AudioTarget::Playback => egui_phosphor::regular::SPEAKER_HIGH,
            crate::models::AudioTarget::Microphone => egui_phosphor::regular::MICROPHONE,
            crate::models::AudioTarget::Process => egui_phosphor::regular::FADERS,
        },
        Action::AudioFormat { .. } => egui_phosphor::regular::EQUALIZER,
        Action::DefaultDevice { target, .. } => match target {
            crate::models::AudioTarget::Playback => egui_phosphor::regular::HEADPHONES,
            crate::models::AudioTarget::Microphone => egui_phosphor::regular::MICROPHONE,
            crate::models::AudioTarget::Process => egui_phosphor::regular::FADERS,
        },
        Action::ExecuteScript { interpreter, .. } => match interpreter {
            crate::models::ScriptType::PowerShell | crate::models::ScriptType::Lua => egui_phosphor::regular::CODE,
            crate::models::ScriptType::Batch => egui_phosphor::regular::TERMINAL_WINDOW,
        },
        Action::Macro { .. } => egui_phosphor::regular::KEYBOARD,
        Action::StartProcess { .. } => egui_phosphor::regular::ROCKET,
        Action::KillProcess { .. } => egui_phosphor::regular::STOP,
        Action::Window { .. } => egui_phosphor::regular::APP_WINDOW,
        Action::ProcessTuning { .. } => egui_phosphor::regular::GAUGE,
        Action::PowerPlan { .. } => egui_phosphor::regular::PLUG,
        Action::PlaySound { .. } => egui_phosphor::regular::MUSIC_NOTES,
        Action::Firewall { .. } => egui_phosphor::regular::SHIELD,
        Action::Notification { .. } => egui_phosphor::regular::BELL,
        Action::SystemPower { .. } => egui_phosphor::regular::POWER,
        Action::Wait { .. } => egui_phosphor::regular::HOURGLASS,
        Action::Check { .. } => egui_phosphor::regular::CHECK_CIRCLE,
        Action::HttpWebhook { .. } => egui_phosphor::regular::GLOBE,
        Action::Display { .. } => egui_phosphor::regular::MONITOR,
        Action::IfElse { .. } => egui_phosphor::regular::GIT_FORK,
        Action::TrimWorkingSet { .. } => egui_phosphor::regular::BROOM,
        Action::DiscordRpcSet { .. } | Action::DiscordRpcClear => egui_phosphor::regular::GAME_CONTROLLER,
        Action::Archive { .. } => egui_phosphor::regular::ARCHIVE,
        Action::FileSystem { .. } => egui_phosphor::regular::FOLDER,
        Action::Overlay { .. } => egui_phosphor::regular::FRAME_CORNERS,
        Action::RuleControl { .. } => egui_phosphor::regular::FLOW_ARROW,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{self, Language};

    fn state_with_rules(rules: Vec<Rule>) -> GuiState {
        let mut state = GuiState::new();
        state.daemon_state = Some(crate::ipc::DaemonState {
            autorun_enabled: false,
            uptime_secs: 0,
            daemon_memory_bytes: 0,
            active_rules_count: 0,
            total_rules_count: 0,
            config: crate::config::AppConfig {
                rules,
                ..Default::default()
            },
            logs: vec![],
            active_rule_executions: std::collections::HashMap::new(),
            active_overlay_ids: std::collections::HashSet::new(),
            active_test_rule_ids: std::collections::HashSet::new(),
            analytics: Default::default(),
            available_update: None,
            update_progress: None,
            update_status: None,
            pending_open_update_modal: false,
            pending_auto_start_update: false,
        });
        state
    }

    fn wait_action() -> Action {
        Action::Wait {
            mode: crate::models::WaitMode::Timer,
            seconds: 1.0,
            condition: None,
            timeout_sec: 30.0,
            timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
        }
    }

    fn sample_rules() -> Vec<Rule> {
        use crate::models::RuleTag;

        let tag_work = RuleTag::new("Work").with_color("#3B82F6");
        let tag_dev = RuleTag::new("Dev").with_color("#10B981");

        let r1 = Rule {
            id: "r1".into(),
            name: "Beta Work".into(),
            is_enabled: true,
            tags: vec![tag_work.clone()],
            trigger: Some(Trigger::Process {
                process_name: "notepad.exe".into(),
                match_mode: crate::models::ProcessMatchMode::Exact,
                single_instance: false,
            }),
            actions: vec![wait_action()],
            ..Default::default()
        };

        let r2 = Rule {
            id: "r2".into(),
            name: "Alpha Dev".into(),
            is_enabled: false,
            tags: vec![tag_dev.clone()],
            actions: vec![wait_action(), wait_action()],
            ..Default::default()
        };

        let r3 = Rule {
            id: "r3".into(),
            name: "Gamma Work & Dev".into(),
            is_enabled: true,
            tags: vec![tag_work, tag_dev],
            trigger: Some(Trigger::Manual),
            ..Default::default()
        };

        vec![r1, r2, r3]
    }

    fn ids_of<'a>(rules: &'a [Rule], order: &[usize]) -> Vec<&'a str> {
        order.iter().map(|&idx| rules[idx].id.as_str()).collect()
    }

    #[test]
    fn test_format_single_condition_brief_and_tooltip() {
        let loc = i18n::get(Language::English);

        // 1. Process running condition
        let cond1 = RuleCondition::ProcessRunning {
            process_name: "notepad.exe".into(),
            match_mode: crate::models::ProcessMatchMode::Exact,
            must_be_running: true,
        };
        let brief1 = format_single_condition_brief(&cond1, loc);
        assert_eq!(brief1, "RUN:notepad.exe");
        let tip1 = format_single_condition_tooltip(&cond1, 1, loc);
        assert_eq!(
            tip1,
            format!(
                "{} [1] {}",
                loc.inspector_conditions_badge,
                loc.rules_tip_cond_process_running.replace("{}", "notepad.exe")
            )
        );

        // 2. Window focused condition
        let cond2 = RuleCondition::WindowFocused {
            process_name: "code.exe".into(),
            match_mode: crate::models::ProcessMatchMode::Exact,
            must_be_focused: true,
        };
        let brief2 = format_single_condition_brief(&cond2, loc);
        assert_eq!(brief2, "FOCUS:code.exe");
        let tip2 = format_single_condition_tooltip(&cond2, 2, loc);
        assert_eq!(
            tip2,
            format!(
                "{} [2] {}",
                loc.inspector_conditions_badge,
                loc.rules_tip_cond_window_focused.replace("{}", "code.exe")
            )
        );
    }

    #[test]
    fn test_condition_and_action_follow_ui_language() {
        let en = i18n::get(Language::English);
        let ru = i18n::get(Language::Russian);

        let cond = RuleCondition::ProcessRunning {
            process_name: "notepad.exe".into(),
            match_mode: crate::models::ProcessMatchMode::Exact,
            must_be_running: true,
        };
        let tip_en = format_single_condition_tooltip(&cond, 1, en);
        let tip_ru = format_single_condition_tooltip(&cond, 1, ru);
        assert_ne!(tip_en, tip_ru);
        assert!(tip_ru.starts_with(ru.inspector_conditions_badge));
        assert_eq!(
            tip_ru,
            format!(
                "{} [1] {}",
                ru.inspector_conditions_badge,
                ru.rules_tip_cond_process_running.replace("{}", "notepad.exe")
            )
        );

        let event_cond = RuleCondition::Event {
            event_type: SystemEventType::SessionLock,
            idle_duration_sec: 300,
        };
        let badge_en = format_single_condition_brief(&event_cond, en);
        let badge_ru = format_single_condition_brief(&event_cond, ru);
        assert_ne!(badge_en, badge_ru);
        assert_eq!(badge_ru, ru.rules_badge_cond_event.replace("{}", ru.inspector_sys_event_lock));

        let action = Action::Notification {
            title: "Done".into(),
            message: String::new(),
        };
        let brief_en = format_action_brief(&action, en);
        let brief_ru = format_action_brief(&action, ru);
        assert_ne!(brief_en, brief_ru);
        assert_eq!(brief_ru, ru.rules_brief_notify.replace("{}", "Done"));

        let power = Action::SystemPower { power_type: SystemPowerType::Shutdown };
        assert_eq!(
            format_action_brief(&power, ru),
            format!("{} {}", ru.summary_action_power, ru.inspector_power_shutdown)
        );
    }

    #[test]
    fn test_mapped_enums_never_render_debug_names() {
        let loc = i18n::get(Language::Russian);

        for event_type in [
            SystemEventType::SessionLock,
            SystemEventType::SessionUnlock,
            SystemEventType::SystemSleep,
            SystemEventType::SystemResume,
            SystemEventType::PowerSourceAc,
            SystemEventType::PowerSourceBattery,
            SystemEventType::UserIdle,
            SystemEventType::UserActive,
            SystemEventType::DeviceConnected,
            SystemEventType::DeviceDisconnected,
            SystemEventType::MonitorOn,
            SystemEventType::MonitorOff,
            SystemEventType::NetworkConnected,
            SystemEventType::NetworkDisconnected,
        ] {
            let debug = format!("{event_type:?}");
            let label = sys_event_label(event_type, loc);
            let cond = RuleCondition::Event {
                event_type,
                idle_duration_sec: 300,
            };
            let tooltip = format_single_condition_tooltip(&cond, 1, loc);
            assert_eq!(tooltip, format!("{} [1] {}", loc.inspector_conditions_badge, loc.rules_tip_cond_event.replace("{}", label)));
            assert!(!tooltip.contains(&debug), "condition tooltip leaked the Debug enum name {debug}");
            let badge = format_single_condition_brief(&cond, loc);
            assert_eq!(badge, loc.rules_badge_cond_event.replace("{}", label));
            assert!(!badge.contains(&debug), "condition badge leaked the Debug enum name {debug}");
            let (_, trigger) = format_trigger_display(
                &Some(Trigger::SystemEvent {
                    event_type,
                    device_id_filter: None,
                }),
                loc,
            );
            assert_eq!(trigger, label);
            assert!(!trigger.contains(&debug), "trigger summary leaked the Debug enum name {debug}");
        }

        for power_type in [
            SystemPowerType::Lock,
            SystemPowerType::Sleep,
            SystemPowerType::TurnScreenOff,
            SystemPowerType::Restart,
            SystemPowerType::Shutdown,
        ] {
            let debug = format!("{power_type:?}");
            let brief = format_action_brief(&Action::SystemPower { power_type }, loc);
            assert_eq!(brief, format!("{} {}", loc.summary_action_power, power_label(power_type, loc)));
            assert!(!brief.contains(&debug), "power chip leaked the Debug enum name {debug}");
        }

        for plan in [
            PowerPlanType::Balanced,
            PowerPlanType::HighPerformance,
            PowerPlanType::PowerSaver,
        ] {
            let debug = format!("{plan:?}");
            let brief = format_action_brief(&Action::PowerPlan { plan }, loc);
            assert_eq!(brief, power_plan_label(plan, loc));
            assert!(!brief.contains(&debug), "power plan chip leaked the Debug enum name {debug}");
        }

        for format in [
            ArchiveFormat::Zip,
            ArchiveFormat::Tar,
            ArchiveFormat::TarGz,
            ArchiveFormat::TarXz,
            ArchiveFormat::SevenZip,
            ArchiveFormat::Rar,
        ] {
            let debug = format!("{format:?}");
            let brief = format_action_brief(
                &Action::Archive {
                    operation: crate::models::ArchiveOperation::Pack,
                    format,
                    source_path: "C:\\source".into(),
                    destination_path: "C:\\out.bin".into(),
                    compression_level: None,
                    overwrite: true,
                },
                loc,
            );
            assert_eq!(brief, fill(loc.rules_brief_archive_pack, &[archive_format_label(format, loc), "out.bin"]));
            assert!(!brief.contains(&debug), "archive chip leaked the Debug enum name {debug}");
        }

        for action in [
            WindowActionType::Minimize,
            WindowActionType::Maximize,
            WindowActionType::Restore,
            WindowActionType::Hide,
            WindowActionType::Show,
            WindowActionType::Close,
            WindowActionType::ToggleAlwaysOnTop,
            WindowActionType::AlwaysOnTop,
            WindowActionType::RemoveAlwaysOnTop,
            WindowActionType::SetTransparency,
            WindowActionType::SnapToPosition,
            WindowActionType::ToggleMinimizeRestore,
            WindowActionType::ToggleShowHide,
        ] {
            let debug = format!("{action:?}");
            let brief = format_action_brief(
                &Action::Window {
                    target_process: Some("app.exe".into()),
                    window_action: action,
                    x: None,
                    y: None,
                    width: None,
                    height: None,
                    transparency: None,
                },
                loc,
            );
            assert_eq!(brief, fill(window_action_label(action, loc), &["app.exe"]));
            assert!(!brief.contains(&debug), "window chip leaked the Debug enum name {debug}");
        }
    }

    #[test]
    fn test_format_trigger_display_no_prefix() {
        for lang in [Language::English, Language::Russian] {
            let loc = i18n::get(lang);

            let (_, trig_manual) = format_trigger_display(&Some(Trigger::Manual), loc);
            assert!(!trig_manual.starts_with("TRIGGER:"));
            assert!(!trig_manual.starts_with("ТРИГГЕР:"));

            let (_, trig_proc) = format_trigger_display(
                &Some(Trigger::Process {
                    process_name: "notepad.exe".into(),
                    match_mode: crate::models::ProcessMatchMode::Exact,
                    single_instance: false,
                }),
                loc,
            );
            assert!(!trig_proc.starts_with("TRIGGER:"));
            assert!(!trig_proc.starts_with("ТРИГГЕР:"));
            assert!(trig_proc.contains("notepad.exe"));

            // Schedule: Daily
            let (_, trig_daily) = format_trigger_display(
                &Some(Trigger::Schedule {
                    schedule_type: crate::models::ScheduleType::Daily,
                    interval_sec: 0,
                    daily_time: Some("14:30".into()),
                    days_of_week: vec![],
                    day_of_month: None,
                    target_datetime: None,
                }),
                loc,
            );
            assert!(!trig_daily.starts_with("TRIGGER:"));
            assert!(!trig_daily.starts_with("ТРИГГЕР:"));

            // Schedule: OneShot
            let (_, trig_once) = format_trigger_display(
                &Some(Trigger::Schedule {
                    schedule_type: crate::models::ScheduleType::OneShot,
                    interval_sec: 0,
                    daily_time: None,
                    days_of_week: vec![],
                    day_of_month: None,
                    target_datetime: Some("2026-12-31 23:59".into()),
                }),
                loc,
            );
            assert!(!trig_once.starts_with("TRIGGER:"));
            assert!(!trig_once.starts_with("ТРИГГЕР:"));

            // One-shot without a target time: the fallback must follow the UI language too (U16-011).
            let (_, trig_once_unspecified) = format_trigger_display(
                &Some(Trigger::Schedule {
                    schedule_type: crate::models::ScheduleType::OneShot,
                    interval_sec: 0,
                    daily_time: None,
                    days_of_week: vec![],
                    day_of_month: None,
                    target_datetime: None,
                }),
                loc,
            );
            assert_eq!(
                trig_once_unspecified,
                loc.rules_summary_trigger_once
                    .replace("{}", loc.rules_summary_trigger_once_unspecified)
            );
            if lang == Language::Russian {
                assert!(
                    !trig_once_unspecified.contains("SPECIFIED TIME"),
                    "the Russian one-shot summary must not embed an English fallback"
                );
            }

            // Schedule: AppStartup
            let (_, trig_startup) = format_trigger_display(
                &Some(Trigger::Schedule {
                    schedule_type: crate::models::ScheduleType::AppStartup,
                    interval_sec: 0,
                    daily_time: None,
                    days_of_week: vec![],
                    day_of_month: None,
                    target_datetime: None,
                }),
                loc,
            );
            assert!(!trig_startup.starts_with("TRIGGER:"));
            assert!(!trig_startup.starts_with("ТРИГГЕР:"));

            // Schedule: AppShutdown
            let (_, trig_shutdown) = format_trigger_display(
                &Some(Trigger::Schedule {
                    schedule_type: crate::models::ScheduleType::AppShutdown,
                    interval_sec: 0,
                    daily_time: None,
                    days_of_week: vec![],
                    day_of_month: None,
                    target_datetime: None,
                }),
                loc,
            );
            assert!(!trig_shutdown.starts_with("TRIGGER:"));
            assert!(!trig_shutdown.starts_with("ТРИГГЕР:"));
        }
    }

    #[test]
    fn test_visible_rule_order_filters_and_sorts() {
        let rules = sample_rules();

        // Tag filter
        let work = visible_rule_order(&rules, Some("Work"), "", RuleSortMode::Default);
        assert_eq!(ids_of(&rules, &work), ["r1", "r3"]);
        let dev = visible_rule_order(&rules, Some("Dev"), "", RuleSortMode::Default);
        assert_eq!(ids_of(&rules, &dev), ["r2", "r3"]);

        // Search (rule name or tag name, case-insensitive)
        let alpha = visible_rule_order(&rules, None, "ALPHA", RuleSortMode::Default);
        assert_eq!(ids_of(&rules, &alpha), ["r2"]);
        let search_work = visible_rule_order(&rules, None, " work ", RuleSortMode::Default);
        assert_eq!(ids_of(&rules, &search_work), ["r1", "r3"]);

        // Sort by name
        let name_asc = visible_rule_order(&rules, None, "", RuleSortMode::NameAsc);
        assert_eq!(ids_of(&rules, &name_asc), ["r2", "r1", "r3"]);
        let name_desc = visible_rule_order(&rules, None, "", RuleSortMode::NameDesc);
        assert_eq!(ids_of(&rules, &name_desc), ["r3", "r1", "r2"]);

        // Sort by trigger type (process first, manual/unknown last)
        let by_trigger = visible_rule_order(&rules, None, "", RuleSortMode::TriggerType);
        assert_eq!(ids_of(&rules, &by_trigger), ["r1", "r2", "r3"]);

        // Sort by action count (most actions first)
        let by_actions = visible_rule_order(&rules, None, "", RuleSortMode::ActionCount);
        assert_eq!(ids_of(&rules, &by_actions), ["r2", "r1", "r3"]);

        // Sort by status (enabled first, then by name)
        let by_status = visible_rule_order(&rules, None, "", RuleSortMode::Status);
        assert_eq!(ids_of(&rules, &by_status), ["r1", "r3", "r2"]);
    }

    #[test]
    fn test_view_order_matches_visible_rules_export() {
        let rules = sample_rules();
        let mut state = state_with_rules(rules.clone());

        for (tag, query) in [
            (None, ""),
            (Some("Work"), ""),
            (Some("Dev"), ""),
            (None, "alpha"),
            (None, "work"),
            (Some("Work"), "gamma"),
            (Some("Dev"), "nomatch"),
        ] {
            state.selected_tag_filter = tag.map(str::to_string);
            state.search_query = query.to_string();

            let order = {
                let rules: &[Rule] = state.daemon_state.as_ref().unwrap().config.rules.as_slice();
                visible_rule_order(
                    rules,
                    state.selected_tag_filter.as_deref(),
                    &state.search_query,
                    RuleSortMode::Default,
                )
            };
            let view_ids: Vec<String> = order
                .iter()
                .map(|&idx| state.daemon_state.as_ref().unwrap().config.rules[idx].id.clone())
                .collect();
            let export_ids: Vec<String> = state.visible_rules().into_iter().map(|r| r.id).collect();

            assert_eq!(
                view_ids, export_ids,
                "the rules view and the copy-all export must share one filter (tag={tag:?}, query={query:?})"
            );
        }
    }

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

    /// The trigger line and the compact condition chips follow the UI language; only the layout
    /// separators stay in the format strings.
    #[test]
    fn test_trigger_display_and_compact_prefix_follow_ui_language() {
        let en = i18n::get(Language::English);
        let ru = i18n::get(Language::Russian);

        let process = Some(Trigger::Process {
            process_name: "notepad.exe".into(),
            match_mode: crate::models::ProcessMatchMode::Exact,
            single_instance: false,
        });
        let (_, proc_en) = format_trigger_display(&process, en);
        let (_, proc_ru) = format_trigger_display(&process, ru);
        assert_eq!(proc_en, format!("{} {}", en.rules_trigger_proc, "notepad.exe"));
        assert_eq!(proc_ru, format!("{} {}", ru.rules_trigger_proc, "notepad.exe"));
        assert_ne!(proc_en, proc_ru);

        let window = Some(Trigger::WindowFocus {
            process_name: "code.exe".into(),
            on_focus_gained: true,
            on_focus_lost: false,
        });
        let (_, window_en) = format_trigger_display(&window, en);
        assert_eq!(
            window_en,
            format!("{} {} {}", en.rules_trigger_window, en.rules_trigger_focus, "code.exe")
        );
        assert_ne!(window_en, format_trigger_display(&window, ru).1);

        let hotkey = Some(Trigger::GlobalHotkey {
            key_combination: "Ctrl+W".into(),
            hold_mode: true,
            repeat_while_held: false,
        });
        let (_, hotkey_en) = format_trigger_display(&hotkey, en);
        assert_eq!(hotkey_en, format!("Ctrl+W{}", en.rules_trigger_hold));
        assert_ne!(hotkey_en, format_trigger_display(&hotkey, ru).1);

        let running = RuleCondition::ProcessRunning {
            process_name: "notepad.exe".into(),
            match_mode: crate::models::ProcessMatchMode::Exact,
            must_be_running: true,
        };
        let brief_en = format_single_condition_brief(&running, en);
        let brief_ru = format_single_condition_brief(&running, ru);
        assert_eq!(brief_en, format!("{}{}", en.rules_brief_cond_run, "notepad.exe"));
        assert_eq!(brief_ru, format!("{}{}", ru.rules_brief_cond_run, "notepad.exe"));
        assert_ne!(brief_en, brief_ru);

        let not_running = RuleCondition::ProcessRunning {
            process_name: "notepad.exe".into(),
            match_mode: crate::models::ProcessMatchMode::Exact,
            must_be_running: false,
        };
        assert_eq!(
            format_single_condition_brief(&not_running, en),
            format!("!{}{}", en.rules_brief_cond_run, "notepad.exe")
        );

        let resource = RuleCondition::ResourceThreshold {
            process_name: "game.exe".into(),
            cpu_percent: Some(75.0),
            ram_mb: Some(512),
        };
        let res_en = format_single_condition_brief(&resource, en);
        assert_eq!(
            res_en,
            format!(
                "{}{}({},{})",
                en.rules_brief_cond_res,
                "game.exe",
                en.rules_brief_cond_cpu.replace("{}", "75"),
                en.rules_brief_cond_ram.replace("{}", "512"),
            )
        );
        assert_ne!(res_en, format_single_condition_brief(&resource, ru));
    }

    /// The literals localized by I18N-GAP must not return to the view. Needles are assembled from
    /// fragments so the scan cannot match its own source.
    #[test]
    fn test_rules_view_source_has_no_removed_literals() {
        let source = include_str!("rules_view.rs");
        let removed = [
            concat!("Clear the tag filter and ", "search to reorder"),
            concat!("PROC ", "{}{}"),
            concat!("WINDOW ", "{} {}"),
            concat!("\"FOC", "US/BLUR\""),
            concat!("\"FOC", "US\""),
            concat!("\"BL", "UR\""),
            concat!("\" [HO", "LD LOOP]\""),
            concat!("\" [HO", "LD]\""),
            concat!("FOLDER ", "{}"),
            concat!("CPU > ", "{}%"),
            concat!("RAM > ", "{}MB"),
            concat!("\"RU", "N:{}\""),
            concat!("\"!RU", "N:{}\""),
            concat!("\"FOC", "US:{}\""),
            concat!("\"!FOC", "US:{}\""),
            concat!("\"FI", "LE:{}\""),
            concat!("\"!FI", "LE:{}\""),
            concat!("\"RE", "S:{}\""),
            concat!("\"RE", "S:{}({})\""),
            concat!("\"RU", "LE:{}\""),
            concat!("\"!RU", "LE:{}\""),
            concat!("CPU>", "{:.0}%"),
            concat!("RAM>", "{}MB"),
            concat!("\"PR", "OC\""),
            concat!("\"WIN", "DOW\""),
            concat!("\"FI", "LE\""),
            concat!("\"RE", "S\""),
            concat!("\"RU", "LE\""),
        ];
        for literal in removed {
            assert!(!source.contains(literal), "hardcoded UI literal is back: {literal}");
        }
    }

    #[test]
    fn test_action_step_chip_marks_only_the_current_step() {
        let icon = egui_phosphor::regular::ROCKET;
        assert_eq!(format_step_chip(icon, "LAUNCH app", false), format!("{} LAUNCH app", icon));
        assert_eq!(format_step_chip(icon, "LAUNCH app", true), format!("▶ {} LAUNCH app", icon));
    }

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
