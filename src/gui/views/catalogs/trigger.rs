//! Trigger Catalog Modal:
//! Studio modal for browsing and selecting rule triggers
//! with categorized tabs, instant fuzzy keyword search, and rich capability cards.
//!
//! The layout, filtering and card rendering live in the shared catalog primitives
//! (`render_catalog_body`), so this file only owns the trigger table and the commit path.

use egui::vec2;
use crate::gui::components::catalog_modal::{
    CatalogCategory, CatalogChrome, CatalogEntryDef, CatalogModal,
};
use crate::gui::state::GuiState;
use crate::gui::theme::Tokens;
use crate::i18n::Strings;
use crate::models::{
    ProcessMatchMode, ScheduleType, SystemEventType, Trigger,
};

type TriggerEntry = CatalogEntryDef<fn() -> Option<Trigger>>;

/// Default `target_datetime` of a freshly added OneShot trigger: tomorrow at noon in the model's
/// wire format. `Trigger::schedule_errors` rejects an unset timestamp, so the rule must carry a
/// valid one from the moment the catalog creates it; the inspector's date picker still lets the
/// user replace it.
fn default_oneshot_target() -> String {
    let now = chrono::Local::now();
    let day = now.date_naive().succ_opt().unwrap_or_else(|| now.date_naive());
    format!("{} 12:00:00", day.format("%Y-%m-%d"))
}

const TRIGGER_CATALOG_ITEMS: &[TriggerEntry] = &[
    // 0. Manual mode (On Demand)
    CatalogEntryDef {
        category_idx: 3,
        badge: "MANUAL",
        nestable: true,
        title: |loc| loc.catalog_trigger_manual_title,
        description: |loc| loc.catalog_trigger_manual_desc,
        glyph: egui_phosphor::regular::PLAY_CIRCLE,
        tint: |t| t.accent_ink,
        keywords: &["manual", "demand", "run", "button", "click", "ручной", "кнопка", "старт", "клик"],
        constructor: || Some(Trigger::Manual),
    },

    // 1. Process Management
    CatalogEntryDef {
        category_idx: 1,
        badge: "PROCESS",
        nestable: true,
        title: |loc| loc.catalog_trigger_process_title,
        description: |loc| loc.catalog_trigger_process_desc,
        glyph: egui_phosphor::regular::CPU,
        tint: |t| t.good,
        keywords: &["process", "launch", "start", "run", "exe", "app", "exit", "close", "termination", "процесс", "запуск", "старт", "приложение", "завершение", "закрытие", "выход"],
        constructor: || Some(Trigger::Process {
            process_name: String::new(),
            match_mode: ProcessMatchMode::Exact,
            single_instance: false,
            event: crate::models::ProcessTriggerEvent::Started,
        }),
    },
    CatalogEntryDef {
        category_idx: 1,
        badge: "WINDOW",
        nestable: true,
        title: |loc| loc.catalog_trigger_window_title,
        description: |loc| loc.catalog_trigger_window_desc,
        glyph: egui_phosphor::regular::EYE,
        tint: |t| t.accent_ink,
        keywords: &["window", "focus", "foreground", "active", "blur", "окно", "фокус", "активное", "свернуто"],
        constructor: || Some(Trigger::WindowFocus {
            process_name: String::new(),
            on_focus_gained: true,
            on_focus_lost: false,
        }),
    },

    // 2. Schedule & Time
    CatalogEntryDef {
        category_idx: 2,
        badge: "TIMER",
        nestable: true,
        title: |loc| loc.catalog_trigger_interval_title,
        description: |loc| loc.catalog_trigger_interval_desc,
        glyph: egui_phosphor::regular::TIMER,
        tint: |t| t.accent_ink,
        keywords: &["timer", "interval", "repeat", "seconds", "minutes", "hours", "таймер", "интервал", "повтор"],
        constructor: || Some(Trigger::Schedule {
            schedule_type: ScheduleType::Interval,
            interval_sec: 60,
            daily_time: None,
            days_of_week: Vec::new(),
            day_of_month: None,
            target_datetime: None,
        }),
    },
    CatalogEntryDef {
        category_idx: 2,
        badge: "SCHEDULE",
        nestable: true,
        title: |loc| loc.catalog_trigger_daily_title,
        description: |loc| loc.catalog_trigger_daily_desc,
        glyph: egui_phosphor::regular::CALENDAR,
        tint: |t| t.good,
        keywords: &["daily", "time", "clock", "hour", "alarm", "ежедневно", "время", "будильник", "часы"],
        constructor: || Some(Trigger::Schedule {
            schedule_type: ScheduleType::Daily,
            interval_sec: 0,
            daily_time: Some("12:00:00".into()),
            days_of_week: Vec::new(),
            day_of_month: None,
            target_datetime: None,
        }),
    },
    CatalogEntryDef {
        category_idx: 2,
        badge: "ONE-SHOT",
        nestable: true,
        title: |loc| loc.catalog_trigger_oneshot_title,
        description: |loc| loc.catalog_trigger_oneshot_desc,
        glyph: egui_phosphor::regular::CALENDAR_CHECK,
        tint: |t| t.muted,
        keywords: &["oneshot", "date", "calendar", "future", "once", "однократно", "дата", "календарь"],
        constructor: || Some(Trigger::Schedule {
            schedule_type: ScheduleType::OneShot,
            interval_sec: 0,
            daily_time: None,
            days_of_week: Vec::new(),
            day_of_month: None,
            target_datetime: Some(default_oneshot_target()),
        }),
    },
    CatalogEntryDef {
        category_idx: 2,
        badge: "MONTHLY",
        nestable: true,
        title: |loc| loc.catalog_trigger_monthly_title,
        description: |loc| loc.catalog_trigger_monthly_desc,
        glyph: egui_phosphor::regular::CALENDAR_BLANK,
        tint: |t| t.accent_ink,
        keywords: &["monthly", "month", "day", "ежемесячно", "месяц", "число"],
        constructor: || Some(Trigger::Schedule {
            schedule_type: ScheduleType::Monthly,
            interval_sec: 0,
            daily_time: Some("12:00:00".into()),
            days_of_week: Vec::new(),
            day_of_month: Some(1),
            target_datetime: None,
        }),
    },

    // 3. Hardware & System
    CatalogEntryDef {
        category_idx: 3,
        badge: "HOTKEY",
        nestable: true,
        title: |loc| loc.catalog_trigger_hotkey_title,
        description: |loc| loc.catalog_trigger_hotkey_desc,
        glyph: egui_phosphor::regular::COMMAND,
        tint: |t| t.accent_ink,
        keywords: &["hotkey", "shortcut", "keyboard", "mouse", "macro", "клавиша", "хоткей", "сочетание", "бинд"],
        constructor: || Some(Trigger::GlobalHotkey {
            key_combination: "Ctrl+Shift+H".into(),
            hold_mode: false,
            repeat_while_held: false,
        }),
    },
    CatalogEntryDef {
        category_idx: 3,
        badge: "RESOURCE",
        nestable: true,
        title: |loc| loc.catalog_trigger_resource_title,
        description: |loc| loc.catalog_trigger_resource_desc,
        glyph: egui_phosphor::regular::CHART_LINE,
        tint: |t| t.bad,
        keywords: &["resource", "cpu", "ram", "memory", "load", "threshold", "нагрузка", "процессор", "память"],
        constructor: || Some(Trigger::ResourceThreshold {
            process_name: String::new(),
            cpu_percent: Some(80.0),
            ram_mb: None,
            duration_sec: 5,
        }),
    },
    CatalogEntryDef {
        category_idx: 3,
        badge: "FILES",
        nestable: true,
        title: |loc| loc.catalog_trigger_file_title,
        description: |loc| loc.catalog_trigger_file_desc,
        glyph: egui_phosphor::regular::FOLDER_NOTCH_OPEN,
        tint: |t| t.good,
        keywords: &["file", "folder", "directory", "watch", "created", "modified", "файл", "папка", "слежение"],
        constructor: || Some(Trigger::FileWatcher {
            folder_path: String::new(),
            filter_extension: None,
            on_created: true,
            on_modified: false,
            on_deleted: false,
        }),
    },
    CatalogEntryDef {
        category_idx: 3,
        badge: "WEBHOOK",
        nestable: true,
        title: |loc| loc.catalog_trigger_webhook_title,
        description: |loc| loc.catalog_trigger_webhook_desc,
        glyph: egui_phosphor::regular::GLOBE_HEMISPHERE_WEST,
        tint: |t| t.accent_ink,
        keywords: &["webhook", "http", "api", "rest", "curl", "url", "вебхук", "сеть"],
        constructor: || Some(Trigger::Webhook {
            endpoint_path: String::new(),
        }),
    },

    // 4. Lifecycle & System Events
    CatalogEntryDef {
        category_idx: 4,
        badge: "LIFECYCLE",
        nestable: true,
        title: |loc| loc.catalog_trigger_lifecycle_title,
        description: |loc| loc.catalog_trigger_lifecycle_desc,
        glyph: egui_phosphor::regular::ROCKET_LAUNCH,
        tint: |t| t.accent_ink,
        keywords: &["startup", "shutdown", "start", "close", "exit", "boot", "launch", "старт", "запуск", "закрытие", "выход", "завершение"],
        constructor: || Some(Trigger::Schedule {
            schedule_type: ScheduleType::AppStartup,
            interval_sec: 0,
            daily_time: None,
            days_of_week: Vec::new(),
            day_of_month: None,
            target_datetime: None,
        }),
    },
    CatalogEntryDef {
        category_idx: 4,
        badge: "EVENT",
        nestable: true,
        title: |loc| loc.catalog_trigger_lock_title,
        description: |loc| loc.catalog_trigger_lock_desc,
        glyph: egui_phosphor::regular::LOCK,
        tint: |t| t.accent_ink,
        keywords: &["lock", "unlock", "session", "screen", "workstation", "блокировка", "разблокировка", "экран"],
        constructor: || Some(Trigger::SystemEvent {
            event_type: SystemEventType::SessionLock,
            device_id_filter: None,
        }),
    },
    CatalogEntryDef {
        category_idx: 4,
        badge: "POWER",
        nestable: true,
        title: |loc| loc.catalog_trigger_sleep_title,
        description: |loc| loc.catalog_trigger_sleep_desc,
        glyph: egui_phosphor::regular::MOON,
        tint: |t| t.accent_ink,
        keywords: &["sleep", "wake", "suspend", "hibernate", "resume", "сон", "пробуждение", "гибернация"],
        constructor: || Some(Trigger::SystemEvent {
            event_type: SystemEventType::SystemSleep,
            device_id_filter: None,
        }),
    },
    CatalogEntryDef {
        category_idx: 4,
        badge: "POWER",
        nestable: true,
        title: |loc| loc.catalog_trigger_power_title,
        description: |loc| loc.catalog_trigger_power_desc,
        glyph: egui_phosphor::regular::PLUG,
        tint: |t| t.good,
        keywords: &["power", "ac", "wall", "charger", "battery", "laptop", "сеть", "розетка", "зарядка", "батарея"],
        constructor: || Some(Trigger::SystemEvent {
            event_type: SystemEventType::PowerSourceAc,
            device_id_filter: None,
        }),
    },
    CatalogEntryDef {
        category_idx: 4,
        badge: "IDLE",
        nestable: true,
        title: |loc| loc.catalog_trigger_idle_title,
        description: |loc| loc.catalog_trigger_idle_desc,
        glyph: egui_phosphor::regular::HOURGLASS,
        tint: |t| t.muted,
        keywords: &["idle", "afk", "inactive", "away", "active", "input", "бездействие", "неактивность", "активность"],
        constructor: || Some(Trigger::SystemEvent {
            event_type: SystemEventType::UserIdle,
            device_id_filter: None,
        }),
    },
    CatalogEntryDef {
        category_idx: 4,
        badge: "DEVICE",
        nestable: true,
        title: |loc| loc.catalog_trigger_device_title,
        description: |loc| loc.catalog_trigger_device_desc,
        glyph: egui_phosphor::regular::HARD_DRIVES,
        tint: |t| t.good,
        keywords: &["device", "usb", "hardware", "connect", "disconnect", "устройство", "подключение", "отключение"],
        constructor: || Some(Trigger::SystemEvent {
            event_type: SystemEventType::DeviceConnected,
            device_id_filter: None,
        }),
    },
    CatalogEntryDef {
        category_idx: 4,
        badge: "DISPLAY",
        nestable: true,
        title: |loc| loc.catalog_trigger_display_title,
        description: |loc| loc.catalog_trigger_display_desc,
        glyph: egui_phosphor::regular::DESKTOP,
        tint: |t| t.good,
        keywords: &["monitor", "display", "screen", "on", "off", "монитор", "дисплей", "экран"],
        constructor: || Some(Trigger::SystemEvent {
            event_type: SystemEventType::MonitorOn,
            device_id_filter: None,
        }),
    },
    CatalogEntryDef {
        category_idx: 4,
        badge: "NETWORK",
        nestable: true,
        title: |loc| loc.catalog_trigger_network_title,
        description: |loc| loc.catalog_trigger_network_desc,
        glyph: egui_phosphor::regular::WIFI_HIGH,
        tint: |t| t.good,
        keywords: &["network", "wifi", "ethernet", "internet", "online", "offline", "сеть", "интернет", "подключение", "отключение"],
        constructor: || Some(Trigger::SystemEvent {
            event_type: SystemEventType::NetworkConnected,
            device_id_filter: None,
        }),
    },
];

const TRIGGER_CATEGORIES: [CatalogCategory; 5] = [
    CatalogCategory { index: 0, glyph: egui_phosphor::regular::SPARKLE, label: |loc| loc.catalog_cat_all },
    CatalogCategory { index: 1, glyph: egui_phosphor::regular::CPU, label: |loc| loc.catalog_cat_process },
    CatalogCategory { index: 2, glyph: egui_phosphor::regular::CLOCK, label: |loc| loc.catalog_cat_schedule },
    CatalogCategory { index: 3, glyph: egui_phosphor::regular::GEAR, label: |loc| loc.catalog_cat_system },
    CatalogCategory { index: 4, glyph: egui_phosphor::regular::BELL, label: |loc| loc.catalog_cat_events },
];

pub(crate) const TRIGGER_CHROME: CatalogChrome = CatalogChrome {
    modal_id: "trigger-catalog-modal",
    eyebrow: |loc| loc.catalog_trigger_modal_title,
    close_tip: |loc| loc.inspector_btn_cancel,
    search_hint: |loc| loc.catalog_trigger_search_hint,
    scroll_id: "trigger_catalog_cards_scroll",
    width_fraction: 0.74,
    height_fraction: 0.76,
    min_size: vec2(720.0, 500.0),
    max_size: vec2(840.0, 640.0),
};

pub fn render_trigger_catalog_modal(ctx: &egui::Context, state: &mut GuiState) {
    let t = Tokens::get(ctx);
    let is_open = state.trigger_catalog.is_some();
    let Some(mut modal_state) = state
        .trigger_catalog
        .clone()
        .or_else(|| state.last_trigger_catalog.clone())
    else {
        return;
    };
    let loc = state.loc();

    let outcome = CatalogModal::new(TRIGGER_CHROME, TRIGGER_CATALOG_ITEMS, &TRIGGER_CATEGORIES)
        .show(
            ctx,
            &t,
            loc,
            &mut modal_state.selected_category,
            &mut modal_state.search_query,
            is_open,
        );

    // Trigger keeps its close-before-commit ordering: a close request wins over a card click that
    // arrives in the same frame.
    if outcome.close_requested {
        state.close_trigger_catalog();
    } else if let Some(idx) = outcome.chosen {
        state.set_trigger_from_catalog((TRIGGER_CATALOG_ITEMS[idx].constructor)());
    } else if is_open {
        state.trigger_catalog = Some(modal_state.clone());
        state.last_trigger_catalog = Some(modal_state);
    } else if outcome.fully_closed {
        state.last_trigger_catalog = None;
    }
}

/// Titles of the triggers a global header query matches, for the search overlay.
pub(crate) fn search_titles(query: &str, loc: &Strings) -> Vec<&'static str> {
    crate::gui::components::catalog_modal::catalog_search_titles(
        TRIGGER_CATALOG_ITEMS,
        query,
        loc,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::components::catalog_modal::{filter_catalog_entries, CatalogEntry};
    use crate::i18n::{self, Language};

    /// A rule shaped like the GUI's new-rule draft, carrying only the catalog construct.
    fn catalog_rule(trigger: Option<Trigger>) -> crate::models::Rule {
        crate::models::Rule {
            id: "rule_catalog_constructor_test".to_string(),
            name: "Catalog constructor test".to_string(),
            trigger,
            ..crate::models::Rule::default()
        }
    }

    #[test]
    fn test_trigger_catalog_items_and_categories() {
        // Category 1: Processes (Process Launch, Window Focus)
        let cat1_items: Vec<_> = TRIGGER_CATALOG_ITEMS.iter().filter(|i| i.category_idx == 1).collect();
        assert_eq!(cat1_items.len(), 2, "Category 1 should have 2 process triggers");

        // Category 2: Schedule (Interval, Daily, Exact Date, Monthly)
        let cat2_items: Vec<_> = TRIGGER_CATALOG_ITEMS.iter().filter(|i| i.category_idx == 2).collect();
        assert_eq!(cat2_items.len(), 4, "Category 2 should have 4 schedule triggers");

        // Category 3: System / Hardware (Manual, Hotkey, System Load, Folder Watcher, Webhook)
        let cat3_items: Vec<_> = TRIGGER_CATALOG_ITEMS.iter().filter(|i| i.category_idx == 3).collect();
        assert_eq!(cat3_items.len(), 5, "Category 3 should have 5 system/hardware triggers");

        // Category 4: Events & Lifecycle (8 paired items)
        let cat4_items: Vec<_> = TRIGGER_CATALOG_ITEMS.iter().filter(|i| i.category_idx == 4).collect();
        assert_eq!(cat4_items.len(), 8, "Category 4 should have 8 event/lifecycle triggers");

        // Every item's category has a sidebar row, and every row has entries.
        for item in TRIGGER_CATALOG_ITEMS {
            assert!(
                TRIGGER_CATEGORIES.iter().any(|category| category.index == item.category_idx),
                "category {} has no sidebar row",
                item.category_idx
            );
        }
        for category in TRIGGER_CATEGORIES.iter().filter(|category| category.index != 0) {
            assert!(
                TRIGGER_CATALOG_ITEMS.iter().any(|item| item.category_idx == category.index),
                "sidebar row {} matches no entry",
                category.index
            );
        }

        // The shared renderer reads every card field, so none of them may be empty.
        for lang in [Language::English, Language::Russian] {
            let loc = i18n::get(lang);
            for category in &TRIGGER_CATEGORIES {
                assert!(!(category.label)(loc).is_empty(), "empty category label");
                assert!(!category.glyph.is_empty(), "empty category glyph");
            }
            for (idx, item) in TRIGGER_CATALOG_ITEMS.iter().enumerate() {
                assert!(!item.title(loc).is_empty(), "empty title for entry {idx}");
                assert!(!item.description(loc).is_empty(), "empty description for entry {idx}");
                assert!(!item.badge().trim().is_empty(), "empty badge for entry {idx}");
                assert!(!item.glyph().is_empty(), "empty glyph for entry {idx}");
                assert!(!item.keywords().is_empty(), "empty keywords for entry {idx}");
            }
        }

        // Test constructor outputs
        for item in TRIGGER_CATALOG_ITEMS {
            let trig = (item.constructor)();
            match &trig {
                Some(Trigger::Process { .. }) | Some(Trigger::WindowFocus { .. }) => {
                    assert_eq!(item.category_idx, 1);
                }
                Some(Trigger::Schedule { schedule_type, .. }) => {
                    if matches!(schedule_type, ScheduleType::AppStartup | ScheduleType::AppShutdown) {
                        assert_eq!(item.category_idx, 4);
                    } else {
                        assert_eq!(item.category_idx, 2);
                    }
                }
                Some(Trigger::Manual) | Some(Trigger::GlobalHotkey { .. }) | Some(Trigger::ResourceThreshold { .. }) | Some(Trigger::FileWatcher { .. }) | Some(Trigger::Webhook { .. }) => {
                    assert_eq!(item.category_idx, 3);
                }
                Some(Trigger::SystemEvent { .. }) => {
                    assert_eq!(item.category_idx, 4);
                }
                None => {}
            }
        }
    }

    #[test]
    fn test_trigger_catalog_constructors_pass_model_validation() {
        for (idx, item) in TRIGGER_CATALOG_ITEMS.iter().enumerate() {
            let trigger = (item.constructor)().unwrap_or(Trigger::Manual);
            assert!(
                trigger.schedule_errors().is_empty(),
                "entry {idx} builds an invalid schedule payload: {:?}",
                trigger.schedule_errors()
            );

            // Same validator the inspector's Save button runs.
            let errors = crate::validation::validate_rules(std::slice::from_ref(&catalog_rule(Some(trigger.clone()))));
            if matches!(trigger, Trigger::Schedule { .. }) {
                assert!(
                    errors.is_empty(),
                    "entry {idx} must be saveable as constructed: {errors:?}"
                );
            } else {
                // A process/window/folder/hotkey target is user input and cannot be guessed;
                // the constructor must not add any other structural error besides that demand.
                assert!(
                    errors.iter().all(|error| error.contains("needs a non-empty")),
                    "entry {idx} adds an unexpected validation error: {errors:?}"
                );
            }
        }
    }

    #[test]
    fn test_trigger_schedule_defaults_are_valid() {
        let mut schedule_types = std::collections::HashSet::new();
        for item in TRIGGER_CATALOG_ITEMS {
            let Some(Trigger::Schedule {
                schedule_type,
                interval_sec,
                daily_time,
                days_of_week,
                day_of_month,
                target_datetime,
            }) = (item.constructor)()
            else {
                continue;
            };
            assert!(schedule_types.insert(schedule_type), "duplicate {schedule_type:?} entry");
            assert!(
                crate::models::days_of_week_are_valid(&days_of_week),
                "{schedule_type:?} days_of_week must be 1..=7"
            );
            match schedule_type {
                ScheduleType::Interval => {
                    assert!(interval_sec > 0, "Interval needs interval_sec > 0");
                }
                ScheduleType::Daily => {
                    let time = daily_time.as_deref().expect("Daily needs daily_time");
                    assert!(crate::models::is_valid_hh_mm_ss(time), "Daily daily_time {time:?}");
                }
                ScheduleType::OneShot => {
                    let target = target_datetime.as_deref().expect("OneShot needs target_datetime");
                    assert!(crate::models::is_valid_target_datetime(target), "OneShot target {target:?}");
                    let parsed = chrono::NaiveDateTime::parse_from_str(target, "%Y-%m-%d %H:%M:%S")
                        .expect("OneShot default uses the full wire format");
                    assert!(
                        parsed > chrono::Local::now().naive_local(),
                        "OneShot default must be in the future, got {target}"
                    );
                }
                ScheduleType::Monthly => {
                    let time = daily_time.as_deref().expect("Monthly needs daily_time");
                    assert!(crate::models::is_valid_hh_mm_ss(time), "Monthly daily_time {time:?}");
                    let day = day_of_month.expect("Monthly needs day_of_month");
                    assert!(crate::models::is_valid_day_of_month(day), "Monthly day_of_month {day}");
                }
                ScheduleType::AppStartup | ScheduleType::AppShutdown => {}
            }
        }
        for expected in [
            ScheduleType::Interval,
            ScheduleType::Daily,
            ScheduleType::OneShot,
            ScheduleType::Monthly,
            ScheduleType::AppStartup,
        ] {
            assert!(schedule_types.contains(&expected), "no catalog entry constructs {expected:?}");
        }
        assert_eq!(schedule_types.len(), 5);
    }

    #[test]
    fn test_catalog_filter_applies_category_and_query_together() {
        let loc = i18n::get(Language::English);
        let timer_entry = TRIGGER_CATALOG_ITEMS
            .iter()
            .position(|item| item.badge == "TIMER")
            .expect("interval entry");

        // Category-only filtering selects that category...
        let schedule_only = filter_catalog_entries(TRIGGER_CATALOG_ITEMS, 2, "", loc, false);
        assert!(!schedule_only.is_empty());
        assert!(schedule_only.iter().all(|&idx| TRIGGER_CATALOG_ITEMS[idx].category() == 2));

        // ...and a query no longer silently drops the highlighted category (U04-005).
        let schedule_query = filter_catalog_entries(TRIGGER_CATALOG_ITEMS, 2, "timer", loc, false);
        assert_eq!(schedule_query, vec![timer_entry]);
        let process_query = filter_catalog_entries(TRIGGER_CATALOG_ITEMS, 1, "timer", loc, false);
        assert!(
            process_query.is_empty(),
            "the timer entry must not appear under the Processes category"
        );

        // The same query still finds the entry under "all".
        let all_query = filter_catalog_entries(TRIGGER_CATALOG_ITEMS, 0, "timer", loc, false);
        assert!(all_query.contains(&timer_entry));
    }

    #[test]
    fn test_render_trigger_catalog_modal_no_crash() {
        let ctx = egui::Context::default();
        crate::gui::theme::install_fonts(&ctx);

        let mut state = GuiState::new();
        state.open_trigger_catalog();

        for _ in 0..3 {
            let mut out = ctx.run_ui(Default::default(), |ctx| {
                render_trigger_catalog_modal(ctx, &mut state);
            });
            out.textures_delta.clear();
        }

        state.close_trigger_catalog();
        for _ in 0..2 {
            let mut out = ctx.run_ui(Default::default(), |ctx| {
                render_trigger_catalog_modal(ctx, &mut state);
            });
            out.textures_delta.clear();
        }
    }
}
