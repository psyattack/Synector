//! Header global search overlay.
//!
//! Typing in the header field opens a dimmed results panel under it. Results are grouped by the
//! surface they can be applied to, with a count and a few previews: rules stay filtered behind the
//! panel, settings and the activity log open with the query already in their own search field, and
//! the rule-builder catalogs (actions, triggers, conditions) open over a draft rule with the same
//! query. The overlay is an `Area` pair, not a `Modal`: the header field stays visible and
//! editable while the page below is dimmed and click-to-dismiss.

use egui::{Align2, FontId, Order, Sense, pos2, vec2};
use crate::gui::components::widgets;
use crate::gui::state::{ConditionCatalogTarget, GuiState, rule_matches_visible_filters};
use crate::gui::theme::{self, Tokens};
use crate::gui::views::analytics_modal::{log_matches_query, LOG_SCAN_LIMIT};
use crate::gui::views::presets::preset_matches_query;
use crate::i18n::Strings;

/// Width the results panel asks for. It is wider than the search field so a group title, its
/// count and an example line fit without wrapping.
const PANEL_W: f32 = 560.0;
/// Gap between the header and the panel.
const PANEL_GAP: f32 = 8.0;
/// Shortest the panel may become for a one-line result set.
const PANEL_MIN_H: f32 = 220.0;
/// Previews shown under one group title.
const EXAMPLE_LIMIT: usize = 3;
/// Long log previews are cut, so one message cannot push the other groups out of the panel.
const LOG_PREVIEW_CHARS: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SearchTarget {
    Rules,
    Settings,
    ActivityLog,
    Actions,
    Triggers,
    Conditions,
    Overlays,
    Macros,
}

struct SearchGroup {
    target: SearchTarget,
    title: &'static str,
    count: usize,
    examples: Vec<String>,
    /// The id of the first match: an overlay preset for [`SearchTarget::Overlays`], a macro
    /// preset for [`SearchTarget::Macros`].
    payload: Option<String>,
}

/// Renders the overlay when the header field holds a query. Called after the panels and the
/// inspector so the scrim sits above them, and before the dialogs, which own the foreground.
///
/// `collect_groups` is the one list of surfaces a query can reach: rules, settings, the activity
/// log, the rule-builder catalogs (actions, triggers, conditions) and the overlay and macro preset
/// libraries. A new search field elsewhere in the window must add its group and its target here,
/// otherwise it stays invisible to the header search.
pub fn render_global_search(ctx: &egui::Context, state: &mut GuiState) {
    let open = state.search_overlay_open
        && !state.search_query.trim().is_empty()
        && !state.is_modal_open();
    if !open {
        state.search_overlay_open = false;
        // Drop the last measurement so reopening with a different result set does not paint one
        // frame at the previous height.
        ctx.data_mut(|data| data.remove_temp::<f32>(measured_height_id()));
        return;
    }

    let t = Tokens::get(ctx);
    let loc = state.loc();
    let full = ctx.content_rect();
    let header_h = t.metrics().header_h;
    let groups = collect_groups(state, loc);

    // 1. Scrim over everything below the header. The header itself stays usable, so the query can
    // be edited while the results are open; a click on the dimmed page dismisses the overlay.
    let mut scrim_clicked = false;
    let scrim_layer = egui::LayerId::new(Order::Middle, egui::Id::unique("global-search-scrim"));
    ctx.move_to_top(scrim_layer);
    egui::Area::new(scrim_layer.id)
        .order(scrim_layer.order)
        .fixed_pos(pos2(full.left(), full.top() + header_h))
        .show(ctx, |ui| {
            let height = (full.height() - header_h).max(0.0);
            let (rect, response) =
                ui.allocate_exact_size(vec2(full.width(), height), Sense::click());
            ui.painter().rect_filled(
                rect,
                egui::CornerRadius::ZERO,
                theme::scrim(&t, 1.0),
            );
            scrim_clicked = response.clicked();
        });

    // 2. Results panel, pinned under the header and centered like the search field.
    let panel_w = PANEL_W.min(full.width() - 32.0).max(320.0);
    let panel_pos = pos2(
        full.center().x - panel_w * 0.5,
        full.top() + header_h + PANEL_GAP,
    );
    // The panel stops above the footer, like the inspector scrim, so a clamped result set never
    // covers the status bar.
    let panel_max_h = (full.height() - header_h - PANEL_GAP - t.metrics().footer_h)
        .max(PANEL_MIN_H);
    // The panel follows the result set: one frame after the content is measured it takes exactly
    // the content height, clamped between the minimum and the window's available height. The
    // stored measurement is what makes resizing work without a second layout pass.
    let measured_id = measured_height_id();
    let measured = ctx.data(|data| data.get_temp::<f32>(measured_id));
    let panel_h = measured
        .unwrap_or(PANEL_MIN_H)
        .clamp(PANEL_MIN_H, panel_max_h);
    let panel_layer = egui::LayerId::new(Order::Middle, egui::Id::unique("global-search-panel"));
    ctx.move_to_top(panel_layer);

    let query = state.search_query.trim().to_string();
    let mut applied: Option<(SearchTarget, Option<String>)> = None;
    egui::Area::new(panel_layer.id)
        .order(panel_layer.order)
        .fixed_pos(panel_pos)
        .show(ctx, |ui| {
            // Allocate the full panel rect and paint the card chrome by hand: relying on the
            // frame's auto-size collapsed the surface to the current result set, which made the
            // panel small and jumpy between queries.
            let (panel_rect, _) = ui.allocate_exact_size(vec2(panel_w, panel_h), Sense::hover());
            ui.painter()
                .add(theme::shadow(&t).as_shape(panel_rect, t.card_corner()));
            ui.painter().rect(
                panel_rect,
                t.card_corner(),
                t.raised(),
                t.hairline(),
                egui::StrokeKind::Inside,
            );

            let inner = panel_rect.shrink2(vec2(8.0, 8.0));
            let mut content_height = None;
            ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
                ui.set_width(inner.width());
                ui.set_height(inner.height());
                let scroll = egui::ScrollArea::vertical()
                    .id_salt("global-search-results")
                    .max_height(inner.height())
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_width(inner.width());
                        if groups.is_empty() {
                            ui.add_space(6.0);
                            widgets::hud(
                                ui,
                                &loc.search_no_results.replace("{}", &query),
                                t.muted,
                            );
                            ui.add_space(6.0);
                            return;
                        }
                        for (index, group) in groups.iter().enumerate() {
                            if index > 0 {
                                widgets::hrule(ui);
                            }
                            if render_group(ui, group, &t) {
                                applied = Some((group.target, group.payload.clone()));
                            }
                        }
                    });
                content_height = Some(scroll.content_size.y);
            });

            if let Some(content) = content_height {
                let next = (content + 16.0).clamp(PANEL_MIN_H, panel_max_h);
                let changed = measured.is_none_or(|old| (old - next).abs() > 0.5);
                ui.ctx().data_mut(|data| data.insert_temp(measured_id, next));
                if changed {
                    // Apply the measured height on the next pass; this is one extra frame per
                    // result-set size change, not a per-frame layout loop.
                    ui.ctx().request_repaint();
                }
            }
        });

    if let Some((target, overlay_id)) = applied {
        state.search_overlay_open = false;
        apply_target(state, target, overlay_id);
        // A target that opens another surface must not leave the keyboard focus in the header
        // field behind its scrim. The rules target is the one that keeps working in the field, so
        // the click that applied it hands the focus back: egui surrenders focus on any click
        // outside the edit, which would otherwise leave the query in an inactive field.
        let search_id = crate::gui::components::widgets::header_search_id();
        if target == SearchTarget::Rules {
            ctx.memory_mut(|memory| memory.request_focus(search_id));
        } else {
            ctx.memory_mut(|memory| memory.surrender_focus(search_id));
        }
    } else if scrim_clicked {
        state.search_overlay_open = false;
    }
}

/// One clickable group: the localized category, its count and up to three previews. The whole row
/// is the target, so a click anywhere in the group applies it.
fn render_group(ui: &mut egui::Ui, group: &SearchGroup, t: &Tokens) -> bool {
    let width = ui.available_width();
    // Compact rows: more groups fit on the fixed panel before it has to scroll.
    let height = 28.0 + group.examples.len() as f32 * 13.0 + 6.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, t.inset_corner(), t.hover_fill());
    }
    painter.text(
        pos2(rect.left() + 10.0, rect.top() + 13.0),
        Align2::LEFT_CENTER,
        group.title,
        FontId::new(10.0, theme::mono_bold()),
        t.ink_strong,
    );
    painter.text(
        pos2(rect.right() - 10.0, rect.top() + 13.0),
        Align2::RIGHT_CENTER,
        group.count.to_string(),
        FontId::new(10.0, theme::mono_bold()),
        t.accent_ink,
    );
    let example_painter = painter.with_clip_rect(rect.shrink2(vec2(10.0, 0.0)));
    for (index, example) in group.examples.iter().enumerate() {
        example_painter.text(
            pos2(rect.left() + 10.0, rect.top() + 28.0 + index as f32 * 13.0),
            Align2::LEFT_CENTER,
            example,
            FontId::new(9.0, theme::mono()),
            t.faint,
        );
    }
    response.clicked()
}

fn collect_groups(state: &GuiState, loc: &Strings) -> Vec<SearchGroup> {
    let needle = state.search_query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut groups = Vec::new();
    let daemon = state.daemon_state.as_ref();

    // 1. Rules: the header field already filters the list behind the panel.
    let rules = daemon.map(|daemon| daemon.config.rules.as_slice()).unwrap_or(&[]);
    let rule_names: Vec<String> = rules
        .iter()
        .filter(|rule| {
            rule_matches_visible_filters(rule, state.selected_tag_filter.as_deref(), &needle)
        })
        .map(|rule| rule.name.clone())
        .collect();
    if !rule_names.is_empty() {
        groups.push(SearchGroup {
            target: SearchTarget::Rules,
            title: loc.search_group_rules,
            count: rule_names.len(),
            examples: previews(rule_names),
            payload: None,
        });
    }

    // 2. Settings: the dialog opens with its own search filtered by the same query.
    let settings = crate::gui::views::settings_modal::search_settings(&needle, loc);
    if !settings.is_empty() {
        groups.push(SearchGroup {
            target: SearchTarget::Settings,
            title: loc.search_group_settings,
            count: settings.len(),
            examples: settings
                .iter()
                .take(EXAMPLE_LIMIT)
                .map(|(title, _)| title.to_string())
                .collect(),
            payload: None,
        });
    }

    // 3. Activity log: the analytics dialog opens with its log filter set. The predicate and the
    // scan cap come from the log section itself, so the promised count is what the stream shows.
    let logs = daemon.map(|daemon| daemon.logs.as_slice()).unwrap_or(&[]);
    let mut log_count = 0;
    let mut log_examples = Vec::new();
    for log in logs.iter().take(LOG_SCAN_LIMIT) {
        if log_matches_query(log, &needle) {
            log_count += 1;
            if log_examples.len() < EXAMPLE_LIMIT {
                log_examples.push(crate::gui::components::text::fill(
                    loc.search_activity_preview,
                    &[
                        log.level.as_str(),
                        log.category.as_str(),
                        &truncate(&log.message, LOG_PREVIEW_CHARS),
                    ],
                ));
            }
        }
    }
    if log_count > 0 {
        groups.push(SearchGroup {
            target: SearchTarget::ActivityLog,
            title: loc.search_group_activity,
            count: log_count,
            examples: log_examples,
            payload: None,
        });
    }

    // 4. Rule-builder catalogs. Applying one opens a draft rule when none is being edited and
    // then the catalog with the query in its own search field.
    let actions = crate::gui::views::catalogs::action::search_titles(&needle, loc);
    if !actions.is_empty() {
        groups.push(SearchGroup {
            target: SearchTarget::Actions,
            title: loc.search_group_actions,
            count: actions.len(),
            examples: actions.iter().take(EXAMPLE_LIMIT).map(|title| title.to_string()).collect(),
            payload: None,
        });
    }
    let triggers = crate::gui::views::catalogs::trigger::search_titles(&needle, loc);
    if !triggers.is_empty() {
        groups.push(SearchGroup {
            target: SearchTarget::Triggers,
            title: loc.search_group_triggers,
            count: triggers.len(),
            examples: triggers.iter().take(EXAMPLE_LIMIT).map(|title| title.to_string()).collect(),
            payload: None,
        });
    }
    let conditions = crate::gui::views::catalogs::condition::search_titles(&needle, loc);
    if !conditions.is_empty() {
        groups.push(SearchGroup {
            target: SearchTarget::Conditions,
            title: loc.search_group_conditions,
            count: conditions.len(),
            examples: conditions.iter().take(EXAMPLE_LIMIT).map(|title| title.to_string()).collect(),
            payload: None,
        });
    }

    // 5. Overlay presets: applying opens the studio for the first match with the preset picker
    // filtered by the same query. The preset predicate is the picker's own.
    let overlays = daemon.map(|daemon| daemon.config.overlays.as_slice()).unwrap_or(&[]);
    let matches: Vec<&crate::models::OverlayDefinition> = overlays
        .iter()
        .filter(|overlay| preset_matches_query(&overlay.name, &overlay.id, &needle))
        .collect();
    if !matches.is_empty() {
        groups.push(SearchGroup {
            target: SearchTarget::Overlays,
            title: loc.search_group_overlays,
            count: matches.len(),
            examples: matches
                .iter()
                .take(EXAMPLE_LIMIT)
                .map(|overlay| overlay.name.clone())
                .collect(),
            payload: Some(matches[0].id.clone()),
        });
    }

    // 6. Macro presets: applying opens a draft rule with a Macro action carrying the first
    // matching preset and the Macro Studio's preset picker filtered by the same query.
    let macros = daemon.map(|daemon| daemon.config.macro_presets.as_slice()).unwrap_or(&[]);
    let matches: Vec<&crate::models::MacroPreset> = macros
        .iter()
        .filter(|preset| preset_matches_query(&preset.name, &preset.id, &needle))
        .collect();
    if !matches.is_empty() {
        groups.push(SearchGroup {
            target: SearchTarget::Macros,
            title: loc.search_group_macros,
            count: matches.len(),
            examples: matches
                .iter()
                .take(EXAMPLE_LIMIT)
                .map(|preset| preset.name.clone())
                .collect(),
            payload: Some(matches[0].id.clone()),
        });
    }

    groups
}

fn apply_target(state: &mut GuiState, target: SearchTarget, payload: Option<String>) {
    let query = state.search_query.trim().to_string();
    match target {
        // The rules list is already filtered by the header field; closing the overlay is the
        // whole action, and the query deliberately stays in the field.
        SearchTarget::Rules => {}
        SearchTarget::Settings => {
            state.settings_search_query = query;
            state.search_query.clear();
            state.open_settings_modal();
        }
        SearchTarget::ActivityLog => {
            // The count promised by the group is the whole stream, so the level chip resets with
            // the query instead of keeping a filter the result did not account for.
            state.analytics_log_level = crate::gui::state::AnalyticsLogFilter::All;
            state.analytics_log_filter = query;
            state.search_query.clear();
            state.open_analytics_modal();
        }
        SearchTarget::Actions => {
            state.search_query.clear();
            state.ensure_rule_draft();
            state.open_action_catalog(false, None);
            if let Some(catalog) = &mut state.action_catalog {
                catalog.search_query = query;
            }
        }
        SearchTarget::Triggers => {
            state.search_query.clear();
            state.ensure_rule_draft();
            state.open_trigger_catalog();
            if let Some(catalog) = &mut state.trigger_catalog {
                catalog.search_query = query;
            }
        }
        SearchTarget::Conditions => {
            state.search_query.clear();
            state.ensure_rule_draft();
            state.open_condition_catalog_with_target(ConditionCatalogTarget::RulePrecondition);
            if let Some(catalog) = &mut state.condition_catalog {
                catalog.search_query = query;
            }
        }
        SearchTarget::Overlays => {
            state.search_query.clear();
            if let Some(id) = payload {
                // Open the definition the group matched, taken from the daemon's library: the
                // studio's own opener reads the config library cache and would silently fall back
                // to a blank overlay when the entry is not there.
                let matched = state
                    .daemon_state
                    .as_ref()
                    .and_then(|daemon| {
                        daemon
                            .config
                            .overlays
                            .iter()
                            .find(|overlay| overlay.id == id)
                            .cloned()
                    });
                match matched {
                    Some(overlay) => state.open_overlay_studio_with(overlay),
                    None => state.open_overlay_studio(&id),
                }
                if let Some(studio) = &mut state.overlay_studio {
                    studio.show_presets_popover = true;
                    studio.preset_search_query = query;
                }
            }
        }
        SearchTarget::Macros => apply_macro_preset(state, payload, query),
    }
}

/// Opens the Macro Studio on a draft rule whose first action is the matched preset, with the
/// preset picker filtered by the same query. A macro preset has no standalone editor, so this is
/// the equivalent of "open the library on that search value" for the macro library.
fn apply_macro_preset(state: &mut GuiState, payload: Option<String>, query: String) {
    let Some(preset_id) = payload else {
        return;
    };
    let Some(preset) = state
        .daemon_state
        .as_ref()
        .and_then(|daemon| {
            daemon
                .config
                .macro_presets
                .iter()
                .find(|preset| preset.id == preset_id)
        })
        .cloned()
    else {
        return;
    };

    state.search_query.clear();
    state.ensure_rule_draft();
    state.push_undo_snapshot();
    let step_idx = state
        .editing_rule
        .as_ref()
        .map_or(0, |rule| rule.actions.len());
    if let Some(rule) = &mut state.editing_rule {
        rule.actions.push(crate::models::Action::Macro {
            preset_id: Some(preset.id),
            operation: Default::default(),
            events: preset.events,
            repeat: preset.repeat,
            speed: preset.speed,
            infinite: preset.infinite,
        });
    }
    state.open_macro_editor(step_idx, false);
    state.show_macro_presets_modal = true;
    state.macro_preset_search_query = query;
}

fn previews(mut values: Vec<String>) -> Vec<String> {
    values.truncate(EXAMPLE_LIMIT);
    values
}

/// Key of the measured content height in egui's temporary data.
fn measured_height_id() -> egui::Id {
    egui::Id::unique("global-search-content-h")
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::DaemonState;
    use crate::models::{LogItem, MacroPreset, OverlayDefinition, Rule};
    use std::collections::{HashMap, HashSet};

    fn search_state(query: &str) -> GuiState {
        let mut config = crate::config::AppConfig::default();
        config.rules.push(Rule {
            id: "rule_macro".into(),
            name: "Macro Launcher".into(),
            ..Default::default()
        });
        // Replace the bundled library instead of appending to it, so the counts are deterministic.
        config.overlays = vec![OverlayDefinition {
            id: "overlay_macro".into(),
            name: "Macro Crosshair".into(),
            ..Default::default()
        }];
        config.macro_presets = vec![MacroPreset::new("preset_macro", "Macro Fast", vec![])];

        let mut state = GuiState::new();
        state.daemon_state = Some(DaemonState {
            autorun_enabled: true,
            uptime_secs: 60,
            daemon_memory_bytes: 1024,
            active_rules_count: 1,
            total_rules_count: 1,
            config,
            logs: vec![LogItem {
                timestamp: "12:00:00".into(),
                level: "INFO".into(),
                category: "RULE".into(),
                message: "Macro playback finished".into(),
            }],
            active_rule_executions: HashMap::new(),
            active_overlay_ids: HashSet::new(),
            active_test_rule_ids: HashSet::new(),
            available_update: None,
            update_progress: None,
            update_status: None,
            pending_open_update_modal: false,
            pending_auto_start_update: false,
        });
        state.search_query = query.to_string();
        state
    }

    #[test]
    fn groups_are_collected_per_surface_with_counts() {
        let state = search_state("macro");
        let loc = crate::i18n::get(crate::i18n::Language::English);
        let groups = collect_groups(&state, loc);

        let group = |target: SearchTarget| groups.iter().find(|group| group.target == target);
        assert_eq!(group(SearchTarget::Rules).map(|group| group.count), Some(1));
        assert_eq!(
            group(SearchTarget::ActivityLog).map(|group| group.count),
            Some(1)
        );
        assert_eq!(group(SearchTarget::Overlays).map(|group| group.count), Some(1));
        assert_eq!(group(SearchTarget::Macros).map(|group| group.count), Some(1));
        assert!(
            group(SearchTarget::Actions).is_some(),
            "the macro action entry must be found"
        );
        assert!(
            group(SearchTarget::Settings).is_none(),
            "no setting mentions a macro"
        );
        assert_eq!(
            group(SearchTarget::Overlays).and_then(|group| group.payload.clone()),
            Some("overlay_macro".into()),
            "the overlay group opens the first matching preset"
        );
    }

    #[test]
    fn applying_settings_transfers_the_query_and_opens_the_dialog() {
        let mut state = search_state("system login");
        apply_target(&mut state, SearchTarget::Settings, None);
        assert!(state.show_settings_modal);
        assert_eq!(state.settings_search_query, "system login");
        assert!(state.search_query.is_empty(), "the header field is handed over");
    }

    #[test]
    fn applying_an_action_opens_a_draft_and_the_catalog_with_the_query() {
        let mut state = search_state("launch");
        apply_target(&mut state, SearchTarget::Actions, None);
        assert!(state.editing_rule.is_some(), "a draft is created when none is edited");
        let catalog = state.action_catalog.as_ref().expect("the action catalog opened");
        assert_eq!(catalog.search_query, "launch");
        assert!(state.search_query.is_empty());
    }

    #[test]
    fn applying_a_trigger_and_a_condition_targets_the_catalog_search() {
        let mut state = search_state("hotkey");
        apply_target(&mut state, SearchTarget::Triggers, None);
        assert_eq!(
            state.trigger_catalog.as_ref().map(|catalog| catalog.search_query.as_str()),
            Some("hotkey")
        );

        let mut state = search_state("cpu");
        apply_target(&mut state, SearchTarget::Conditions, None);
        assert_eq!(
            state.condition_catalog.as_ref().map(|catalog| catalog.search_query.as_str()),
            Some("cpu")
        );
        assert!(state.editing_rule.is_some());
    }

    #[test]
    fn applying_an_overlay_opens_the_studio_and_the_filtered_preset_picker() {
        let mut state = search_state("macro");
        apply_target(&mut state, SearchTarget::Overlays, Some("overlay_macro".into()));
        let studio = state.overlay_studio.as_ref().expect("the overlay studio opened");
        assert!(studio.show_presets_popover);
        assert_eq!(studio.preset_search_query, "macro");
    }

    #[test]
    fn applying_a_macro_preset_opens_the_studio_on_a_macro_action() {
        let mut state = search_state("macro");
        apply_target(&mut state, SearchTarget::Macros, Some("preset_macro".into()));

        assert!(state.macro_editor.is_some(), "the Macro Studio opens");
        assert!(state.show_macro_presets_modal);
        assert_eq!(state.macro_preset_search_query, "macro");
        assert!(state.search_query.is_empty());
        let rule = state.editing_rule.as_ref().expect("a draft rule exists");
        match rule.actions.first() {
            Some(crate::models::Action::Macro { preset_id, .. }) => {
                assert_eq!(preset_id.as_deref(), Some("preset_macro"));
            }
            other => panic!("expected a Macro action, got {other:?}"),
        }

        // Teardown without the close path, which would queue a preset save against the config.
        state.macro_editor = None;
        state.show_macro_presets_modal = false;
        crate::gui::macro_recorder::set_studio_active(false);
    }

    #[test]
    fn the_panel_renders_every_group_without_panicking() {
        let ctx = egui::Context::default();
        crate::gui::theme::install_fonts(&ctx);
        let mut state = search_state("macro");
        state.search_overlay_open = true;
        let loc = crate::i18n::get(crate::i18n::Language::English);

        // The overlay lives in Areas, which egui drains on the following pass.
        let mut output = None;
        for time in [0.0, 0.1] {
            let mut frame = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 760.0),
                    )),
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    render_global_search(ui.ctx(), &mut state);
                },
            );
            frame.textures_delta.clear();
            output = Some(frame);
        }
        let output = output.expect("two frames were rendered");

        let mut texts = Vec::new();
        for clipped in &output.shapes {
            collect_text(&clipped.shape, &mut texts);
        }
        for needle in [
            loc.search_group_rules,
            loc.search_group_activity,
            loc.search_group_overlays,
            loc.search_group_macros,
        ] {
            assert!(
                texts.iter().any(|text| text == needle),
                "{needle:?} was not painted: {texts:?}"
            );
        }

    }

    /// The panel height follows the result set: a broader query grows the surface, and neither
    /// case leaves the [minimum, available] band.
    #[test]
    fn panel_height_adapts_to_the_result_set() {
        let height_for = |query: &str| {
            let ctx = egui::Context::default();
            crate::gui::theme::install_fonts(&ctx);
            let mut state = search_state(query);
            state.search_overlay_open = true;
            let mut height = 0.0_f32;
            for frame in 0..4 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1180.0, 760.0),
                        )),
                        time: Some(frame as f64 * 0.1),
                        ..Default::default()
                    },
                    |ui| {
                        render_global_search(ui.ctx(), &mut state);
                    },
                );
                output.textures_delta.clear();
                for clipped in &output.shapes {
                    if let egui::Shape::Rect(rect) = &clipped.shape {
                        if rect.rect.width() > 500.0
                            && rect.rect.width() < 800.0
                            && rect.rect.height() >= PANEL_MIN_H - 1.0
                        {
                            height = height.max(rect.rect.height());
                        }
                    }
                }
            }
            height
        };

        let narrow = height_for("macro");
        let broad = height_for("e");
        let max = 760.0 - 48.0 - 8.0 - 24.0;
        assert!(narrow >= PANEL_MIN_H - 1.0, "narrow panel stayed tiny: {narrow}");
        assert!(
            broad > narrow + 10.0,
            "a broader result set must grow the panel: broad {broad}, narrow {narrow}"
        );
        assert!(broad <= max + 1.0, "the panel outgrew the window: {broad} > {max}");
    }

    fn collect_text(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => out.push(text.galley.text().to_string()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect_text(shape, out)),
            _ => {}
        }
    }

    #[test]
    fn previews_are_bounded() {
        let previews = previews(vec![
            "one".into(),
            "two".into(),
            "three".into(),
            "four".into(),
        ]);
        assert_eq!(previews.len(), EXAMPLE_LIMIT);
        assert_eq!(previews[0], "one");
    }

    #[test]
    fn long_text_is_truncated_by_characters_not_bytes() {
        let text = "Привет мир, это достаточно длинное сообщение";
        let cut = truncate(text, 10);
        assert_eq!(cut.chars().count(), 11, "ten characters plus the ellipsis");
        assert!(cut.ends_with('…'));
        assert_eq!(truncate("short", 10), "short");
    }
}
