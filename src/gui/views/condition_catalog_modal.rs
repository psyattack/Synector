//! Condition Catalog Modal:
//! Studio modal for browsing and selecting rule pre-conditions
//! with categorized tabs, instant fuzzy keyword search, and rich capability cards.
//!
//! The layout, filtering and card rendering live in the shared catalog primitives
//! (`render_catalog_body`), so this file only owns the condition table and the commit path.

use egui::{vec2, Color32};
use crate::gui::components::modal::Modal;
use crate::gui::state::GuiState;
use crate::gui::theme::Tokens;
use crate::i18n::Strings;
use crate::models::{
    ProcessMatchMode, RuleCondition, SystemEventType,
};
use super::action_catalog_modal::{
    catalog_commit_allowed, render_catalog_body, render_catalog_header, CatalogCategory,
    CatalogEntry, CatalogView,
};

struct ConditionCatalogItem {
    category_idx: usize, // 1: Processes, 2: System, 3: Time, 4: Events, 5: Rules
    badge: &'static str,
    title: fn(&Strings) -> &'static str,
    description: fn(&Strings) -> &'static str,
    glyph: &'static str,
    tint: fn(&Tokens) -> Color32,
    keywords: &'static [&'static str],
    constructor: fn() -> RuleCondition,
}

impl CatalogEntry for ConditionCatalogItem {
    fn category(&self) -> usize {
        self.category_idx
    }

    fn badge(&self) -> &'static str {
        self.badge
    }

    fn title(&self, loc: &Strings) -> &'static str {
        (self.title)(loc)
    }

    fn description(&self, loc: &Strings) -> &'static str {
        (self.description)(loc)
    }

    fn glyph(&self) -> &'static str {
        self.glyph
    }

    fn tint(&self, t: &Tokens) -> Color32 {
        (self.tint)(t)
    }

    fn keywords(&self) -> &'static [&'static str] {
        self.keywords
    }

    fn nestable(&self) -> bool {
        // Conditions are not branch actions, so the catalog has no nesting mode.
        true
    }
}

const CONDITION_CATALOG_ITEMS: &[ConditionCatalogItem] = &[
    // 1. Process & Window
    ConditionCatalogItem {
        category_idx: 1,
        badge: "PROCESS",
        title: |loc| loc.catalog_cond_process_title,
        description: |loc| loc.catalog_cond_process_desc,
        glyph: egui_phosphor::regular::CPU,
        tint: |t| t.good,
        keywords: &["process", "running", "closed", "state", "status", "exe", "app", "процесс", "запущен", "закрыт", "состояние", "работает"],
        constructor: || RuleCondition::ProcessRunning {
            process_name: String::new(),
            match_mode: ProcessMatchMode::Exact,
            must_be_running: true,
        },
    },
    ConditionCatalogItem {
        category_idx: 1,
        badge: "WINDOW",
        title: |loc| loc.catalog_cond_window_title,
        description: |loc| loc.catalog_cond_window_desc,
        glyph: egui_phosphor::regular::EYE,
        tint: |t| t.accent_ink,
        keywords: &["window", "focus", "active", "foreground", "background", "blur", "окно", "фокус", "активно", "фон"],
        constructor: || RuleCondition::WindowFocused {
            process_name: String::new(),
            match_mode: ProcessMatchMode::Exact,
            must_be_focused: true,
        },
    },

    // 2. Hardware & Resource
    ConditionCatalogItem {
        category_idx: 2,
        badge: "RESOURCE",
        title: |loc| loc.catalog_cond_resource_title,
        description: |loc| loc.catalog_cond_resource_desc,
        glyph: egui_phosphor::regular::CHART_LINE,
        tint: |t| t.bad,
        keywords: &["resource", "cpu", "ram", "memory", "load", "threshold", "нагрузка", "процессор", "память"],
        constructor: || RuleCondition::ResourceThreshold {
            // `resource_threshold_errors` rejects an empty target: without one the condition
            // silently degrades to "process is running". Explorer exists in every desktop
            // session, so the freshly added condition is coherent and immediately saveable;
            // the inspector's process picker replaces it with the user's target.
            process_name: "explorer.exe".to_string(),
            cpu_percent: Some(80.0),
            ram_mb: None,
        },
    },
    ConditionCatalogItem {
        category_idx: 2,
        badge: "FILES",
        title: |loc| loc.catalog_cond_file_title,
        description: |loc| loc.catalog_cond_file_desc,
        glyph: egui_phosphor::regular::FOLDER_NOTCH_OPEN,
        tint: |t| t.good,
        keywords: &["file", "folder", "directory", "path", "exists", "файл", "папка", "слежение"],
        constructor: || RuleCondition::FileWatcher {
            path: String::new(),
            filter_extension: None,
            must_exist: true,
        },
    },

    // 3. Time & Schedule
    ConditionCatalogItem {
        category_idx: 3,
        badge: "TIME",
        title: |loc| loc.catalog_cond_time_title,
        description: |loc| loc.catalog_cond_time_desc,
        glyph: egui_phosphor::regular::CLOCK,
        tint: |t| t.good,
        keywords: &["time", "hours", "days", "window", "schedule", "время", "часы", "дни", "интервал"],
        constructor: || RuleCondition::TimeRange {
            start_time: "09:00".into(),
            end_time: "18:00".into(),
            days_of_week: vec![1, 2, 3, 4, 5],
        },
    },

    // 4. System Events & States
    ConditionCatalogItem {
        category_idx: 4,
        badge: "EVENT",
        title: |loc| loc.catalog_cond_lock_title,
        description: |loc| loc.catalog_cond_lock_desc,
        glyph: egui_phosphor::regular::LOCK,
        tint: |t| t.accent_ink,
        keywords: &["locked", "unlocked", "lock", "screen", "session", "блокировка", "разблокировка", "экран"],
        constructor: || RuleCondition::Event {
            event_type: SystemEventType::SessionLock,
            idle_duration_sec: 0,
        },
    },
    ConditionCatalogItem {
        category_idx: 4,
        badge: "POWER",
        title: |loc| loc.catalog_cond_sleep_title,
        description: |loc| loc.catalog_cond_sleep_desc,
        glyph: egui_phosphor::regular::MOON,
        tint: |t| t.accent_ink,
        keywords: &["sleep", "wake", "suspend", "hibernate", "resume", "сон", "пробуждение", "гибернация"],
        constructor: || RuleCondition::Event {
            event_type: SystemEventType::SystemSleep,
            idle_duration_sec: 0,
        },
    },
    ConditionCatalogItem {
        category_idx: 4,
        badge: "POWER",
        title: |loc| loc.catalog_cond_power_title,
        description: |loc| loc.catalog_cond_power_desc,
        glyph: egui_phosphor::regular::PLUG,
        tint: |t| t.good,
        keywords: &["power", "ac", "wall", "charger", "battery", "laptop", "сеть", "розетка", "зарядка", "батарея"],
        constructor: || RuleCondition::Event {
            event_type: SystemEventType::PowerSourceAc,
            idle_duration_sec: 0,
        },
    },
    ConditionCatalogItem {
        category_idx: 4,
        badge: "IDLE",
        title: |loc| loc.catalog_cond_idle_title,
        description: |loc| loc.catalog_cond_idle_desc,
        glyph: egui_phosphor::regular::HOURGLASS,
        tint: |t| t.muted,
        keywords: &["idle", "afk", "inactive", "away", "active", "input", "бездействие", "неактивность", "активность"],
        constructor: || RuleCondition::Event {
            event_type: SystemEventType::UserIdle,
            idle_duration_sec: 300,
        },
    },
    ConditionCatalogItem {
        category_idx: 4,
        badge: "DISPLAY",
        title: |loc| loc.catalog_cond_display_title,
        description: |loc| loc.catalog_cond_display_desc,
        glyph: egui_phosphor::regular::DESKTOP,
        tint: |t| t.good,
        keywords: &["monitor", "display", "screen", "on", "off", "монитор", "дисплей", "экран"],
        constructor: || RuleCondition::Event {
            event_type: SystemEventType::MonitorOn,
            idle_duration_sec: 0,
        },
    },
    ConditionCatalogItem {
        category_idx: 4,
        badge: "DEVICE",
        title: |loc| loc.catalog_cond_device_title,
        description: |loc| loc.catalog_cond_device_desc,
        glyph: egui_phosphor::regular::HARD_DRIVES,
        tint: |t| t.good,
        keywords: &["device", "usb", "hardware", "connect", "disconnect", "устройство", "подключение", "отключение"],
        constructor: || RuleCondition::Event {
            event_type: SystemEventType::DeviceConnected,
            idle_duration_sec: 0,
        },
    },
    ConditionCatalogItem {
        category_idx: 4,
        badge: "NETWORK",
        title: |loc| loc.catalog_cond_network_title,
        description: |loc| loc.catalog_cond_network_desc,
        glyph: egui_phosphor::regular::WIFI_HIGH,
        tint: |t| t.good,
        keywords: &["network", "wifi", "ethernet", "internet", "online", "offline", "сеть", "интернет", "подключение", "отключение"],
        constructor: || RuleCondition::Event {
            event_type: SystemEventType::NetworkConnected,
            idle_duration_sec: 0,
        },
    },

    // 5. Rules
    ConditionCatalogItem {
        category_idx: 5,
        badge: "RULE",
        title: |loc| loc.catalog_cond_rule_title,
        description: |loc| loc.catalog_cond_rule_desc,
        glyph: egui_phosphor::regular::PLAY,
        tint: |t| t.accent_ink,
        keywords: &[
            "rule", "running", "started", "stopped", "pipeline", "activity", "правило", "запущено",
            "остановлено", "выполняется", "пайплайн",
        ],
        constructor: || RuleCondition::RuleRunning {
            rule_id: String::new(),
            must_be_running: true,
        },
    },
];

const CONDITION_CATEGORIES: [CatalogCategory; 6] = [
    CatalogCategory { index: 0, glyph: egui_phosphor::regular::SPARKLE, label: |loc| loc.catalog_cat_all },
    CatalogCategory { index: 1, glyph: egui_phosphor::regular::CPU, label: |loc| loc.catalog_cat_process },
    CatalogCategory { index: 2, glyph: egui_phosphor::regular::HARD_DRIVE, label: |loc| loc.catalog_cat_system },
    CatalogCategory { index: 3, glyph: egui_phosphor::regular::CLOCK, label: |loc| loc.catalog_cat_time },
    CatalogCategory { index: 4, glyph: egui_phosphor::regular::LIGHTNING, label: |loc| loc.catalog_cat_events },
    CatalogCategory { index: 5, glyph: egui_phosphor::regular::PLAY, label: |loc| loc.catalog_cat_rules },
];

pub fn render_condition_catalog_modal(ctx: &egui::Context, state: &mut GuiState) {
    let t = Tokens::get(ctx);
    let is_open = state.condition_catalog.is_some();
    let active_modal = state
        .condition_catalog
        .as_ref()
        .or(state.last_condition_catalog.as_ref())
        .cloned();

    let Some(mut modal_state) = active_modal else { return };
    let loc = state.loc();
    let mut chosen_condition: Option<RuleCondition> = None;

    let modal_res = Modal::new("condition-catalog-modal", is_open)
        .width_fraction(0.74)
        .height_fraction(0.76)
        .min_size(vec2(720.0, 500.0))
        .max_size(vec2(840.0, 640.0))
        .header_height(46.0)
        .show(ctx, &t, |frame| {
            render_catalog_header(frame, &t, loc.catalog_condition_modal_title, loc.inspector_btn_cancel);

            frame.body(|ui| {
                let mut view = CatalogView {
                    items: CONDITION_CATALOG_ITEMS,
                    categories: &CONDITION_CATEGORIES,
                    selected_category: &mut modal_state.selected_category,
                    search_query: &mut modal_state.search_query,
                    nesting: false,
                    scroll_id: "condition_catalog_cards_scroll",
                    search_hint: loc.catalog_condition_search_hint,
                };
                if let Some(idx) = render_catalog_body(ui, &t, loc, &mut view) {
                    chosen_condition = Some((CONDITION_CATALOG_ITEMS[idx].constructor)());
                }
            });
        });

    // `Modal` keeps its card grid clickable through the close animation; only a click on the
    // still open dialog may commit (U04-002).
    if !catalog_commit_allowed(is_open) {
        if modal_res.is_none() {
            state.last_condition_catalog = None;
        }
        return;
    }

    if let Some(cond) = chosen_condition {
        state.apply_condition_from_catalog(cond);
    } else if let Some(res) = modal_res {
        if res.close_requested {
            state.close_condition_catalog();
        } else if is_open {
            state.condition_catalog = Some(modal_state.clone());
            state.last_condition_catalog = Some(modal_state);
        }
    } else if !is_open {
        state.last_condition_catalog = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::views::action_catalog_modal::filter_catalog_entries;
    use crate::i18n::{self, Language};
    use crate::models::RuleConditionItem;

    /// A rule shaped like the GUI's new-rule draft, carrying only the catalog construct.
    fn catalog_rule(conditions: Vec<RuleCondition>) -> crate::models::Rule {
        crate::models::Rule {
            id: "rule_catalog_constructor_test".to_string(),
            name: "Catalog constructor test".to_string(),
            conditions: conditions.into_iter().map(RuleConditionItem::new).collect(),
            ..crate::models::Rule::default()
        }
    }

    #[test]
    fn test_condition_catalog_items_and_categories() {
        // Category 1: Processes (Process State, Window Focus)
        let cat1_items: Vec<_> = CONDITION_CATALOG_ITEMS.iter().filter(|i| i.category_idx == 1).collect();
        assert_eq!(cat1_items.len(), 2, "Category 1 should have exactly 2 combined items");
        assert_eq!(cat1_items[0].badge, "PROCESS");
        assert_eq!(cat1_items[1].badge, "WINDOW");

        // Category 4: Events (7 stateful system events from Trigger library)
        let cat4_items: Vec<_> = CONDITION_CATALOG_ITEMS.iter().filter(|i| i.category_idx == 4).collect();
        assert_eq!(cat4_items.len(), 7, "Category 4 should have 7 system events");

        // Category 5: Rules (other rule's pipeline activity)
        let cat5_items: Vec<_> = CONDITION_CATALOG_ITEMS.iter().filter(|i| i.category_idx == 5).collect();
        assert_eq!(cat5_items.len(), 1, "Category 5 should have the rule activity condition");
        assert_eq!(cat5_items[0].badge, "RULE");

        // Every item's category has a sidebar row, and every row has entries.
        for item in CONDITION_CATALOG_ITEMS {
            assert!(
                CONDITION_CATEGORIES.iter().any(|category| category.index == item.category_idx),
                "category {} has no sidebar row",
                item.category_idx
            );
        }
        for category in CONDITION_CATEGORIES.iter().filter(|category| category.index != 0) {
            assert!(
                CONDITION_CATALOG_ITEMS.iter().any(|item| item.category_idx == category.index),
                "sidebar row {} matches no entry",
                category.index
            );
        }

        // The shared renderer reads every card field, so none of them may be empty.
        for lang in [Language::English, Language::Russian] {
            let loc = i18n::get(lang);
            for category in &CONDITION_CATEGORIES {
                assert!(!(category.label)(loc).is_empty(), "empty category label");
                assert!(!category.glyph.is_empty(), "empty category glyph");
            }
            for (idx, item) in CONDITION_CATALOG_ITEMS.iter().enumerate() {
                assert!(!item.title(loc).is_empty(), "empty title for entry {idx}");
                assert!(!item.description(loc).is_empty(), "empty description for entry {idx}");
                assert!(!item.badge().trim().is_empty(), "empty badge for entry {idx}");
                assert!(!item.glyph().is_empty(), "empty glyph for entry {idx}");
                assert!(!item.keywords().is_empty(), "empty keywords for entry {idx}");
            }
        }

        // Test constructor outputs
        for item in CONDITION_CATALOG_ITEMS {
            let cond = (item.constructor)();
            match &cond {
                RuleCondition::ProcessRunning { .. } => assert_eq!(item.category_idx, 1),
                RuleCondition::WindowFocused { .. } => assert_eq!(item.category_idx, 1),
                RuleCondition::ResourceThreshold { .. } | RuleCondition::FileWatcher { .. } => {
                    assert_eq!(item.category_idx, 2);
                }
                RuleCondition::TimeRange { .. } => assert_eq!(item.category_idx, 3),
                RuleCondition::Event { .. } => assert_eq!(item.category_idx, 4),
                RuleCondition::RuleRunning { .. } => assert_eq!(item.category_idx, 5),
            }
        }
    }

    #[test]
    fn test_condition_catalog_constructors_pass_model_validation() {
        for (idx, item) in CONDITION_CATALOG_ITEMS.iter().enumerate() {
            let condition = (item.constructor)();
            assert!(
                condition.validation_errors().is_empty(),
                "entry {idx} builds an invalid condition: {:?}",
                condition.validation_errors()
            );

            // Same validator the inspector's Save button runs; it walks the rule conditions.
            let errors = crate::validation::validate_rules(std::slice::from_ref(&catalog_rule(vec![condition])));
            assert!(
                errors.is_empty(),
                "entry {idx} must be saveable as constructed: {errors:?}"
            );
        }
    }

    #[test]
    fn test_resource_condition_defaults_are_saveable() {
        let item = CONDITION_CATALOG_ITEMS
            .iter()
            .find(|item| item.badge == "RESOURCE")
            .expect("resource condition entry");
        match (item.constructor)() {
            RuleCondition::ResourceThreshold {
                process_name,
                cpu_percent,
                ram_mb,
            } => {
                // U01-012: the model rejects an empty target, so the constructor must supply one.
                assert!(
                    !process_name.trim().is_empty(),
                    "resource threshold needs a default process_name"
                );
                assert!(
                    cpu_percent.is_some() || ram_mb.is_some(),
                    "resource threshold needs cpu_percent or ram_mb"
                );
            }
            other => panic!("RESOURCE entry constructed {other:?}"),
        }
    }

    #[test]
    fn test_catalog_filter_applies_category_and_query_together() {
        let loc = i18n::get(Language::English);
        let resource_entry = CONDITION_CATALOG_ITEMS
            .iter()
            .position(|item| item.badge == "RESOURCE")
            .expect("resource entry");

        // Category-only filtering selects that category...
        let system_only = filter_catalog_entries(CONDITION_CATALOG_ITEMS, 2, "", loc, false);
        assert!(!system_only.is_empty());
        assert!(system_only.iter().all(|&idx| CONDITION_CATALOG_ITEMS[idx].category() == 2));

        // ...and a query no longer silently drops the highlighted category (U04-005).
        let system_query = filter_catalog_entries(CONDITION_CATALOG_ITEMS, 2, "cpu", loc, false);
        assert_eq!(system_query, vec![resource_entry]);
        let process_query = filter_catalog_entries(CONDITION_CATALOG_ITEMS, 1, "cpu", loc, false);
        assert!(
            process_query.is_empty(),
            "the system-load entry must not appear under the Processes category"
        );

        // The same query still finds the entry under "all".
        let all_query = filter_catalog_entries(CONDITION_CATALOG_ITEMS, 0, "cpu", loc, false);
        assert!(all_query.contains(&resource_entry));
    }

    #[test]
    fn test_render_condition_catalog_modal_no_crash() {
        let ctx = egui::Context::default();
        crate::gui::theme::install_fonts(&ctx);

        let mut state = GuiState::new();
        state.open_condition_catalog();

        for _ in 0..3 {
            let mut out = ctx.run_ui(Default::default(), |ctx| {
                render_condition_catalog_modal(ctx, &mut state);
            });
            out.textures_delta.clear();
        }

        state.close_condition_catalog();
        for _ in 0..2 {
            let mut out = ctx.run_ui(Default::default(), |ctx| {
                render_condition_catalog_modal(ctx, &mut state);
            });
            out.textures_delta.clear();
        }
    }

    /// U04-002 (CATALOG-FOLLOWUP): `Modal` keeps the card grid clickable through the 160 ms
    /// close animation, so the condition commit must be behind the same gate the action catalog
    /// uses. The test module itself names these symbols, so the scan stops at `#[cfg(test)]`.
    #[test]
    fn test_condition_catalog_commit_is_gated_while_the_modal_closes() {
        let source = include_str!("condition_catalog_modal.rs");
        let production = source
            .split_once("#[cfg(test)]")
            .map(|(production, _)| production)
            .unwrap_or(source);
        let gate = production
            .find("catalog_commit_allowed(is_open)")
            .expect("the condition commit path must use the shared close-animation gate");
        let commit = production
            .find("state.apply_condition_from_catalog")
            .expect("the condition commit call must stay in the commit path");
        assert!(
            gate < commit,
            "the gate must run before the chosen condition is applied (U04-002)"
        );
    }
}
