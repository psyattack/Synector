//! Automation rules view in the craft-launcher aesthetic:
//! precision segmented cells, hairline cards, numbered section eyebrows,
//! uppercase mono badges, and inverted action buttons.

use egui::{Ui, vec2};
use crate::gui::components::widgets::{self, Kind};
use crate::gui::state::rule_matches_visible_filters;
use crate::gui::state::{GuiState, RuleSortMode};
use crate::gui::theme::{self, Tokens};
use crate::models::Rule;

mod dnd;
mod pipeline;
mod row;
mod summary;
mod tags;

use self::summary::trigger_sort_key;
pub(crate) use self::summary::action_icon_glyph;
// Compatibility re-export: the row and drag-preview modules keep the historical
// `rules_view::default_icon_glyph` path.
pub(crate) use crate::gui::views::rule_summary::default_icon_glyph;

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
        let (pending_reorder, intents) = dnd::render_rule_list(ui, state, &order, loc, &t);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Action, Trigger};

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

    pub(super) fn wait_action() -> Action {
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

    /// Concatenation of the whole `rules_view` module tree plus the shared `rule_summary` module.
    /// The source guard must scan every production file, and this list must keep matching the
    /// `mod` declarations or the guard silently covers less than the module does.
    fn rules_view_module_source() -> String {
        let files = [
            ("rules_view.rs", include_str!("rules_view.rs")),
            ("rules_view/dnd.rs", include_str!("rules_view/dnd.rs")),
            ("rules_view/pipeline.rs", include_str!("rules_view/pipeline.rs")),
            ("rules_view/row.rs", include_str!("rules_view/row.rs")),
            ("rules_view/summary.rs", include_str!("rules_view/summary.rs")),
            ("rules_view/tags.rs", include_str!("rules_view/tags.rs")),
            ("rule_summary.rs", include_str!("rule_summary.rs")),
        ];

        // Self-check: one entry per `mod` declaration plus this root file and the shared module.
        let root_modules = include_str!("rules_view.rs")
            .lines()
            .filter(|line| line.starts_with("mod ") && !line.starts_with("mod tests"))
            .count();
        assert_eq!(
            files.len(),
            root_modules + 2,
            "the guard file list must match the rules_view module tree and its shared rule_summary module"
        );
        for (name, source) in &files {
            assert!(!source.is_empty(), "{name} is empty");
        }

        files.iter().map(|(_, source)| *source).collect()
    }

    /// The literals localized by I18N-GAP must not return to the view. Needles are assembled from
    /// fragments so the scan cannot match its own source.
    #[test]
    fn test_rules_view_source_has_no_removed_literals() {
        let source = rules_view_module_source();
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
}
