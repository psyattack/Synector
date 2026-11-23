//! Action Catalog Modal:
//! Studio modal for browsing and selecting rule actions
//! with categorized tabs, instant fuzzy keyword search, and rich capability cards.
//!
//! The shared shell (`CatalogModal`, filtering, header and card grid) lives in
//! `src/gui/components/catalog_modal.rs`; this file owns the action table, its categories,
//! chrome and the pipeline commit adapter.

use egui::vec2;
use crate::gui::components::catalog_modal::{
    CatalogCategory, CatalogChrome, CatalogEntryDef, CatalogModal,
};
use crate::gui::state::GuiState;
use crate::gui::theme::Tokens;
use crate::models::{
    Action, PowerPlanType,
    SystemPowerType, WindowActionType,
};

type ActionEntry = CatalogEntryDef<fn() -> Action>;

// ============================================================================
// Action catalog table
// ============================================================================

const CATALOG_ITEMS: [ActionEntry; 26] = [
    // 1. Process Management
    CatalogEntryDef {
        category_idx: 1,
        badge: "PROCESS",
        nestable: true,
        title: |loc| loc.inspector_menu_launch_process,
        description: |loc| loc.catalog_desc_launch_process,
        glyph: egui_phosphor::regular::ROCKET,
        tint: |t| t.good,
        keywords: &[
            "launch", "start", "run", "exe", "process", "app", "application", "binary",
            "запуск", "старт", "приложение", "программа", "файл", "процесс", "запустить",
        ],
        constructor: || Action::StartProcess {
            file_path: String::new(),
            arguments: String::new(),
            working_directory: None,
            run_as_admin: false,
            only_if_not_running: false,
            run_hidden: false,
            close_on_exit: false,
        },
    },
    CatalogEntryDef {
        category_idx: 1,
        badge: "PROCESS",
        nestable: true,
        title: |loc| loc.inspector_menu_kill_process,
        description: |loc| loc.catalog_desc_kill_process,
        glyph: egui_phosphor::regular::STOP,
        tint: |t| t.bad,
        keywords: &[
            "kill", "terminate", "stop", "close", "end", "taskkill", "force",
            "завершить", "убить", "закрыть", "остановить", "процесс", "снять",
        ],
        constructor: || Action::KillProcess {
            process_name: String::new(),
            kill_tree: true,
            force: true,
        },
    },
    CatalogEntryDef {
        category_idx: 1,
        badge: "WINDOW",
        nestable: true,
        title: |loc| loc.action_window_title,
        description: |loc| loc.action_window_desc,
        glyph: egui_phosphor::regular::APP_WINDOW,
        tint: |t| t.accent_ink,
        keywords: &[
            "window", "minimize", "maximize", "restore", "hide", "show", "transparency", "snap", "always on top",
            "окно", "свернуть", "развернуть", "скрыть", "прозрачность", "позиция", "поверх всех",
        ],
        constructor: || Action::Window {
            target_process: None,
            window_action: WindowActionType::Minimize,
            x: None,
            y: None,
            width: None,
            height: None,
            transparency: None,
        },
    },
    CatalogEntryDef {
        category_idx: 1,
        badge: "TUNING",
        nestable: true,
        title: |loc| loc.action_tuning_title,
        description: |loc| loc.action_tuning_desc,
        glyph: egui_phosphor::regular::CPU,
        tint: |t| t.good,
        keywords: &[
            "tuning", "priority", "affinity", "cpu", "cores",
            "приоритет", "ядра", "тюнинг", "оптимизация", "процесс",
        ],
        constructor: || Action::ProcessTuning {
            target_process: None,
            priority: None,
            affinity_mask: None,
            trim_memory: false,
        },
    },
    CatalogEntryDef {
        category_idx: 1,
        badge: "RAM",
        nestable: true,
        title: |loc| loc.catalog_title_trim_working_set,
        description: |loc| loc.catalog_desc_trim_working_set,
        glyph: egui_phosphor::regular::BROOM,
        tint: |t| t.good,
        keywords: &[
            "trim", "memory", "working set", "ram", "cleanup", "release", "empty",
            "очистка", "память", "рам", "озу", "сброс", "оптимизация", "рабочее множество",
        ],
        constructor: || Action::TrimWorkingSet {
            target_process: None,
            soft_limit_mb: None,
            skip_if_focused: false,
        },
    },

    // 2. Audio & Media
    CatalogEntryDef {
        category_idx: 2,
        badge: "AUDIO",
        nestable: true,
        title: |loc| loc.action_audio_control_title,
        description: |loc| loc.action_audio_control_desc,
        glyph: egui_phosphor::regular::SPEAKER_HIGH,
        tint: |t| t.warn,
        keywords: &[
            "sound", "audio", "volume", "mute", "unmute", "process", "master", "speaker", "microphone", "mic",
            "звук", "громкость", "мутить", "замутить", "динамик", "процесс", "аудио", "микрофон",
        ],
        constructor: || Action::AudioControl {
            target: crate::models::AudioTarget::Playback,
            process_name: None,
            action_type: crate::models::AudioVolumeAction::Mute,
            volume_level: 50,
            left_volume: None,
            right_volume: None,
        },
    },
    CatalogEntryDef {
        category_idx: 2,
        badge: "AUDIO",
        nestable: true,
        title: |loc| loc.action_default_device_title,
        description: |loc| loc.action_default_device_desc,
        glyph: egui_phosphor::regular::HEADPHONES,
        tint: |t| t.warn,
        keywords: &[
            "audio device", "playback", "speakers", "headphones", "sound output", "default device", "microphone", "mic", "recording", "input",
            "устройство", "вывод", "динамики", "наушники", "звук", "воспроизведение", "микрофон", "ввод", "запись",
        ],
        constructor: || Action::DefaultDevice {
            target: crate::models::AudioTarget::Playback,
            device_name: String::new(),
        },
    },
    CatalogEntryDef {
        category_idx: 2,
        badge: "AUDIO",
        nestable: true,
        title: |loc| loc.catalog_title_audio_format,
        description: |loc| loc.catalog_desc_audio_format,
        glyph: egui_phosphor::regular::EQUALIZER,
        tint: |t| t.warn,
        keywords: &[
            "audio format", "sample rate", "hz", "bit", "depth", "channels", "frequency",
            "формат аудио", "частота", "герц", "бит", "каналы", "звук",
        ],
        constructor: || Action::AudioFormat {
            target: crate::models::AudioTarget::Playback,
            device_name: None,
            sample_rate: 48000,
            bit_depth: 24,
            channels: 2,
        },
    },
    CatalogEntryDef {
        category_idx: 2,
        badge: "AUDIO",
        nestable: true,
        title: |loc| loc.action_play_sound_title,
        description: |loc| loc.action_play_sound_desc,
        glyph: egui_phosphor::regular::MUSIC_NOTES,
        tint: |t| t.warn,
        keywords: &[
            "play sound", "audio", "wav", "mp3", "beep", "file", "play", "music", "alert", "effect",
            "звук", "воспроизвести", "аудио", "бип", "сигнал", "мелодия", "файл", "эффект",
        ],
        constructor: || Action::PlaySound {
            file_path: String::new(),
            play_beep: false,
            beep_frequency: None,
            beep_duration_ms: None,
        },
    },

    // 3. Rules & Logic
    CatalogEntryDef {
        category_idx: 3,
        badge: "FLOW",
        nestable: true,
        title: |loc| loc.catalog_title_wait,
        description: |loc| loc.catalog_desc_wait,
        glyph: egui_phosphor::regular::HOURGLASS,
        tint: |t| t.faint,
        keywords: &[
            "wait", "delay", "pause", "sleep", "timeout", "seconds", "hold", "condition",
            "ожидание", "задержка", "пауза", "ждать", "секунды", "таймаут", "время", "условие",
        ],
        constructor: || Action::Wait {
            mode: crate::models::WaitMode::Timer,
            seconds: 1.0,
            condition: None,
            timeout_sec: 30.0,
            timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
        },
    },
    CatalogEntryDef {
        category_idx: 3,
        badge: "LOGIC",
        nestable: true,
        title: |loc| loc.catalog_title_check,
        description: |loc| loc.catalog_desc_check,
        glyph: egui_phosphor::regular::CHECK_CIRCLE,
        tint: |t| t.accent_ink,
        keywords: &[
            "check", "condition", "assert", "gate", "guard", "test", "filter",
            "проверка", "условие", "фильтр", "проверить", "прервать",
        ],
        constructor: || Action::Check {
            condition: crate::models::RuleCondition::default_process(),
        },
    },
    CatalogEntryDef {
        category_idx: 3,
        badge: "LOGIC",
        nestable: false,
        title: |loc| loc.catalog_title_if_else,
        description: |loc| loc.catalog_desc_if_else,
        glyph: egui_phosphor::regular::GIT_FORK,
        tint: |t| t.accent_ink,
        keywords: &[
            "if", "else", "condition", "branch", "logic", "toggle", "check", "flow",
            "если", "иначе", "ветвление", "условие", "логика", "проверка",
        ],
        constructor: || Action::IfElse {
            conditions: vec![crate::models::RuleConditionItem::new(crate::models::RuleCondition::default_process())],
            then_actions: Vec::new(),
            else_if_branches: Vec::new(),
            else_actions: Vec::new(),
        },
    },
    CatalogEntryDef {
        category_idx: 3,
        badge: "RULES",
        nestable: true,
        title: |loc| loc.catalog_title_rule_control,
        description: |loc| loc.catalog_desc_rule_control,
        glyph: egui_phosphor::regular::FLOW_ARROW,
        tint: |t| t.accent_ink,
        keywords: &[
            "rule", "call", "invoke", "run rule", "chain", "stop rule", "subrule", "pipeline", "wait", "compose",
            "правило", "вызов", "вызвать", "запустить правило", "остановить правило", "цепочка", "подправило",
            "пайплайн", "ждать",
        ],
        constructor: || Action::RuleControl {
            rule_id: String::new(),
            operation: crate::models::RuleControlOperation::Call,
            wait: false,
        },
    },
    CatalogEntryDef {
        category_idx: 3,
        badge: "MACRO",
        nestable: true,
        title: |loc| loc.action_macro_title,
        description: |loc| loc.action_macro_desc,
        glyph: egui_phosphor::regular::KEYBOARD,
        tint: |t| t.warn,
        keywords: &[
            "macro", "input", "keyboard", "mouse", "click", "record", "replay", "keys", "type", "hotkey", "cursor",
            "макрос", "клавиатура", "мышь", "клик", "запись", "нажатие", "печать", "хоткей", "курсор", "эмуляция",
        ],
        constructor: || Action::Macro {
            preset_id: None,
            operation: crate::models::MacroOperation::Run,
            events: Vec::new(),
            repeat: 1,
            speed: 1.0,
            infinite: false,
        },
    },
    CatalogEntryDef {
        category_idx: 3,
        badge: "SCRIPT",
        nestable: true,
        title: |loc| loc.action_execute_script_title,
        description: |loc| loc.action_execute_script_desc,
        glyph: egui_phosphor::regular::CODE,
        tint: |t| t.accent_ink,
        keywords: &[
            "powershell", "pwsh", "ps1", "script", "code", "cmd", "batch", "bat", "lua", "terminal", "console",
            "павершелл", "скрипт", "код", "сценарий", "батник", "терминал", "луа",
        ],
        constructor: || Action::ExecuteScript {
            interpreter: crate::models::ScriptType::PowerShell,
            script: String::new(),
            run_in_window: false,
        },
    },
    CatalogEntryDef {
        category_idx: 3,
        badge: "NOTIFY",
        nestable: true,
        title: |loc| loc.inspector_menu_notification,
        description: |loc| loc.catalog_desc_notification,
        glyph: egui_phosphor::regular::BELL,
        tint: |t| t.accent_ink,
        keywords: &[
            "notification", "toast", "alert", "message", "notify", "popup", "banner",
            "уведомление", "сообщение", "алерт", "баннер", "оповещение", "всплывающее",
        ],
        constructor: || Action::Notification {
            title: String::new(),
            message: String::new(),
        },
    },
    CatalogEntryDef {
        category_idx: 3,
        badge: "OVERLAY",
        nestable: true,
        title: |loc| loc.catalog_title_overlay,
        description: |loc| loc.catalog_desc_overlay,
        glyph: egui_phosphor::regular::FRAME_CORNERS,
        tint: |t| t.accent_ink,
        keywords: &[
            "overlay", "hud", "crosshair", "reticle", "widget", "onscreen", "display", "gui", "badge",
            "оверлей", "прицел", "хад", "виджет", "экран", "бейдж", "индикатор",
        ],
        constructor: || {
            static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            let count = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let unique_id = format!("overlay_{:x}_{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos(), count);
            let def = crate::models::OverlayDefinition {
                id: unique_id.clone(),
                ..Default::default()
            };
            Action::Overlay {
                overlay_id: unique_id,
                custom_overlay: Some(Box::new(def)),
                operation: crate::models::OverlayOperation::Toggle,
                override_text: None,
                duration_ms: None,
                close_on_exit: false,
            }
        },
    },
    CatalogEntryDef {
        category_idx: 3,
        badge: "NETWORK",
        nestable: true,
        title: |loc| loc.inspector_menu_webhook,
        description: |loc| loc.catalog_desc_webhook,
        glyph: egui_phosphor::regular::GLOBE,
        tint: |t| t.good,
        keywords: &[
            "webhook", "http", "api", "rest", "post", "get", "put", "request", "url", "json",
            "вебхук", "запрос", "сеть", "интернет", "урл", "апи",
        ],
        constructor: || Action::HttpWebhook {
            url: String::new(),
            method: "POST".into(),
            headers: None,
            payload: None,
        },
    },
    CatalogEntryDef {
        category_idx: 3,
        badge: "DISCORD",
        nestable: true,
        title: |loc| loc.catalog_title_discord_rpc_set,
        description: |loc| loc.catalog_desc_discord_rpc_set,
        glyph: egui_phosphor::regular::GAME_CONTROLLER,
        tint: |t| t.accent_ink,
        keywords: &[
            "discord", "rpc", "rich presence", "activity", "game", "status", "profile",
            "дискорд", "рпц", "статус", "игра", "профиль", "активность",
        ],
        constructor: || Action::DiscordRpcSet {
            app_id: String::new(),
            details: String::new(),
            state: String::new(),
            large_image: None,
            large_text: None,
            small_image: None,
            small_text: None,
            show_elapsed: true,
            buttons: Vec::new(),
            auto_clear_on_process_exit: true,
        },
    },
    CatalogEntryDef {
        category_idx: 3,
        badge: "DISCORD",
        nestable: true,
        title: |loc| loc.catalog_title_discord_rpc_clear,
        description: |loc| loc.catalog_desc_discord_rpc_clear,
        glyph: egui_phosphor::regular::X_CIRCLE,
        tint: |t| t.bad,
        keywords: &[
            "discord", "rpc", "clear", "stop", "reset", "remove status",
            "дискорд", "сброс", "очистить", "удалить статус", "остановить",
        ],
        constructor: || Action::DiscordRpcClear,
    },

    // 4. System & Files
    CatalogEntryDef {
        category_idx: 4,
        badge: "SYSTEM",
        nestable: true,
        title: |loc| loc.inspector_menu_system_power,
        description: |loc| loc.catalog_desc_system_power,
        glyph: egui_phosphor::regular::POWER,
        tint: |t| t.bad,
        keywords: &[
            "power", "lock", "sleep", "shutdown", "restart", "screen", "monitor", "display",
            "питание", "блокировка", "сон", "выключение", "экран", "перезагрузка", "монитор",
        ],
        constructor: || Action::SystemPower {
            power_type: SystemPowerType::Lock,
        },
    },
    CatalogEntryDef {
        category_idx: 4,
        badge: "POWER",
        nestable: true,
        title: |loc| loc.action_power_plan_title,
        description: |loc| loc.action_power_plan_desc,
        glyph: egui_phosphor::regular::BATTERY_CHARGING,
        tint: |t| t.warn,
        keywords: &[
            "power plan", "energy", "high performance", "balanced", "battery saver",
            "питание", "план", "схема", "энергосбережение", "производительность", "аккумулятор",
        ],
        constructor: || Action::PowerPlan {
            plan: PowerPlanType::HighPerformance,
        },
    },
    CatalogEntryDef {
        category_idx: 4,
        badge: "DISPLAY",
        nestable: true,
        title: |loc| loc.action_display_title,
        description: |loc| loc.action_display_desc,
        glyph: egui_phosphor::regular::MONITOR,
        tint: |t| t.accent_ink,
        keywords: &[
            "display", "resolution", "hz", "refresh rate", "monitor", "screen", "fps", "144hz", "1080p", "4k",
            "экран", "разрешение", "герцовка", "монитор", "частота", "дисплей",
        ],
        constructor: || Action::Display {
            width: Some(1920),
            height: Some(1080),
            refresh_rate: Some(144),
            monitor_index: None,
        },
    },
    CatalogEntryDef {
        category_idx: 4,
        badge: "SECURITY",
        nestable: true,
        title: |loc| loc.action_firewall_title,
        description: |loc| loc.action_firewall_desc,
        glyph: egui_phosphor::regular::SHIELD_CHECK,
        tint: |t| t.bad,
        keywords: &[
            "firewall", "block", "network", "internet", "security", "rule", "netsh",
            "файрвол", "брандмауэр", "сеть", "интернет", "трафик", "блокировка",
        ],
        constructor: || Action::Firewall {
            process_name: String::new(),
            action: crate::models::FirewallAction::Block,
        },
    },
    CatalogEntryDef {
        category_idx: 4,
        badge: "FILES",
        nestable: true,
        title: |loc| loc.catalog_title_file_system,
        description: |loc| loc.catalog_desc_file_system,
        glyph: egui_phosphor::regular::FOLDER,
        tint: |t| t.accent_ink,
        keywords: &[
            "file", "folder", "directory", "create", "delete", "rename", "copy", "clone", "write", "text", "append",
            "файл", "папка", "создать", "удалить", "переименовать", "копировать", "клонировать", "записать", "текст",
        ],
        constructor: || Action::FileSystem {
            operation: crate::models::FileSystemOperation::CreateFile,
            path: String::new(),
            target_path: None,
            content: None,
            create_parents: true,
            overwrite: true,
        },
    },
    CatalogEntryDef {
        category_idx: 4,
        badge: "ARCHIVE",
        nestable: true,
        title: |loc| loc.catalog_title_archive,
        description: |loc| loc.catalog_desc_archive,
        glyph: egui_phosphor::regular::ARCHIVE,
        tint: |t| t.warn,
        keywords: &[
            "archive", "zip", "rar", "7z", "tar", "compress", "extract", "unzip", "unpack", "pack",
            "архив", "сжатие", "распаковка", "зип", "рар", "паковать", "распаковать",
        ],
        constructor: || Action::Archive {
            operation: crate::models::ArchiveOperation::Pack,
            format: crate::models::ArchiveFormat::Zip,
            source_path: String::new(),
            destination_path: String::new(),
            compression_level: None,
            overwrite: true,
        },
    },
];

const ACTION_CATEGORIES: [CatalogCategory; 5] = [
    CatalogCategory { index: 0, glyph: egui_phosphor::regular::SPARKLE, label: |loc| loc.catalog_cat_all },
    CatalogCategory { index: 1, glyph: egui_phosphor::regular::CPU, label: |loc| loc.catalog_cat_process },
    CatalogCategory { index: 2, glyph: egui_phosphor::regular::SPEAKER_HIGH, label: |loc| loc.catalog_cat_audio },
    CatalogCategory { index: 3, glyph: egui_phosphor::regular::CODE, label: |loc| loc.catalog_cat_automation },
    CatalogCategory { index: 4, glyph: egui_phosphor::regular::GEAR, label: |loc| loc.catalog_cat_system },
];

pub(crate) const ACTION_CHROME: CatalogChrome = CatalogChrome {
    modal_id: "action-catalog-modal",
    eyebrow: |loc| loc.catalog_modal_eyebrow,
    close_tip: |loc| loc.inspector_btn_cancel,
    search_hint: |loc| loc.catalog_search_hint,
    scroll_id: "action_catalog_cards_scroll",
    width_fraction: 0.74,
    height_fraction: 0.76,
    min_size: vec2(720.0, 500.0),
    max_size: vec2(840.0, 640.0),
};

pub fn render_action_catalog_modal(ctx: &egui::Context, state: &mut GuiState) {
    let t = Tokens::get(ctx);
    let is_open = state.action_catalog.is_some();
    let Some(mut modal_state) = state
        .action_catalog
        .clone()
        .or_else(|| state.last_action_catalog.clone())
    else {
        return;
    };
    let loc = state.loc();

    let outcome = CatalogModal::new(ACTION_CHROME, &CATALOG_ITEMS, &ACTION_CATEGORIES)
        .nesting(modal_state.ifelse_target.is_some())
        .show(
            ctx,
            &t,
            loc,
            &mut modal_state.selected_category,
            &mut modal_state.search_query,
            is_open,
        );

    // Action keeps its chosen-before-close ordering: a card click wins over a close request that
    // arrives in the same frame, and every commit goes through `push_undo_snapshot`.
    if let Some(idx) = outcome.chosen {
        let action = (CATALOG_ITEMS[idx].constructor)();
        state.add_action_from_catalog(
            action,
            modal_state.is_exit_pipeline,
            modal_state.insert_idx,
            modal_state.ifelse_target,
        );
    } else if outcome.close_requested {
        state.close_action_catalog();
    } else if is_open {
        state.action_catalog = Some(modal_state.clone());
        state.last_action_catalog = Some(modal_state);
    } else if outcome.fully_closed {
        state.last_action_catalog = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::components::catalog_modal::{
        catalog_commit_allowed, filter_catalog_entries, CatalogEntry,
    };
    use crate::i18n::{self, Language};

    #[test]
    fn test_action_catalog_entries_are_complete_and_unique() {
        assert_eq!(CATALOG_ITEMS.len(), 26);

        for lang in [Language::English, Language::Russian] {
            let loc = i18n::get(lang);
            for (idx, item) in CATALOG_ITEMS.iter().enumerate() {
                assert!(!item.title(loc).is_empty(), "empty title for entry {idx}");
                assert!(!item.description(loc).is_empty(), "empty description for entry {idx}");
                assert!(!item.badge().trim().is_empty(), "empty badge for entry {idx}");
                assert!(!item.glyph().is_empty(), "empty glyph for entry {idx}");
                assert!(!item.keywords().is_empty(), "empty keywords for entry {idx}");
                assert!((1..=4).contains(&item.category()), "bad category for entry {idx}");
            }
        }

        // One constructor per action kind: a copy-pasted table row is caught here.
        let mut kinds = std::collections::HashSet::new();
        for item in &CATALOG_ITEMS {
            let action = (item.constructor)();
            assert!(kinds.insert(std::mem::discriminant(&action)), "duplicate action kind in catalog");
        }
        assert_eq!(kinds.len(), CATALOG_ITEMS.len());
    }

    #[test]
    fn test_catalog_filter_applies_category_and_query_together() {
        let loc = i18n::get(Language::English);
        let script_idx = CATALOG_ITEMS
            .iter()
            .position(|item| (item.title)(loc) == loc.action_execute_script_title)
            .expect("script entry");

        // Category-only filtering selects that category...
        let audio_only = filter_catalog_entries(&CATALOG_ITEMS, 2, "", loc, false);
        assert!(!audio_only.is_empty());
        assert!(audio_only.iter().all(|&idx| CATALOG_ITEMS[idx].category() == 2));
        assert!(!audio_only.contains(&script_idx));

        // ...and a query no longer silently drops the selected category (U04-005).
        let audio_query = filter_catalog_entries(&CATALOG_ITEMS, 2, "pwsh", loc, false);
        assert!(audio_query.is_empty(), "a script entry must not appear under the Audio category");
        let all_query = filter_catalog_entries(&CATALOG_ITEMS, 0, "pwsh", loc, false);
        assert!(all_query.contains(&script_idx));

        // Search inside the matching category still works.
        let audio_mute = filter_catalog_entries(&CATALOG_ITEMS, 2, "mute", loc, false);
        assert!(!audio_mute.is_empty());
        assert!(audio_mute.iter().all(|&idx| CATALOG_ITEMS[idx].category() == 2));
    }

    #[test]
    fn test_catalog_nesting_only_hides_non_nestable_entries() {
        let loc = i18n::get(Language::English);
        let check_idx = CATALOG_ITEMS
            .iter()
            .position(|item| (item.title)(loc) == loc.catalog_title_check)
            .expect("check entry");
        let if_else_idx = CATALOG_ITEMS
            .iter()
            .position(|item| (item.title)(loc) == loc.catalog_title_if_else)
            .expect("if/else entry");

        let nested = filter_catalog_entries(&CATALOG_ITEMS, 0, "", loc, true);
        assert!(!nested.contains(&if_else_idx), "If/Else must not nest inside a branch");
        assert!(
            nested.contains(&check_idx),
            "Check shares the LOGIC badge but is a valid branch step (U04-010)"
        );
        assert_eq!(nested.len(), CATALOG_ITEMS.len() - 1);

        let top_level = filter_catalog_entries(&CATALOG_ITEMS, 0, "", loc, false);
        assert_eq!(top_level.len(), CATALOG_ITEMS.len());
    }

    #[test]
    fn test_catalog_commit_requires_open_modal() {
        assert!(catalog_commit_allowed(true));
        assert!(
            !catalog_commit_allowed(false),
            "a click during the close animation must not re-insert the chosen entry (U04-002)"
        );
    }

    #[test]
    fn test_render_action_catalog_modal_no_crash() {
        let ctx = egui::Context::default();
        crate::gui::theme::install_fonts(&ctx);

        let mut state = GuiState::new();
        state.open_action_catalog(false, None);

        for _ in 0..3 {
            let mut out = ctx.run_ui(Default::default(), |ctx| {
                render_action_catalog_modal(ctx, &mut state);
            });
            out.textures_delta.clear();
        }

        state.close_action_catalog();
        for _ in 0..2 {
            let mut out = ctx.run_ui(Default::default(), |ctx| {
                render_action_catalog_modal(ctx, &mut state);
            });
            out.textures_delta.clear();
        }
    }
}
