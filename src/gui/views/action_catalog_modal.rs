//! Action Catalog Modal:
//! Studio modal for browsing and selecting rule actions
//! with categorized tabs, instant fuzzy keyword search, and rich capability cards.
//!
//! This file also hosts the shared catalog primitives (`CatalogEntry`, `CatalogCategory`,
//! `CatalogView`, `render_catalog_header`, `render_catalog_body`) that the element catalog
//! reuses. Their intended home is `src/gui/components/catalog_modal.rs`; registering that module
//! would edit `src/gui/components.rs`, which this package does not own, so the shared code stays
//! here for now. The trigger and condition catalogs still carry their own copies and should adopt
//! the same renderer.

use egui::{
    Color32, CornerRadius, FontId, Margin, Rect, RichText, Sense, Stroke,
    pos2, vec2,
};
use crate::gui::components::modal::{Modal, ModalFrame};
use crate::gui::components::widgets::{self, Glyph};
use crate::gui::state::GuiState;
use crate::gui::theme::{self, Tokens};
use crate::i18n::Strings;
use crate::models::{
    Action, PowerPlanType,
    SystemPowerType, WindowActionType,
};

// ============================================================================
// Shared catalog primitives
// ============================================================================

/// Presentation metadata every catalog card exposes to the shared renderer.
///
/// `nestable` is the explicit "can live inside an If/Else branch" flag (U04-010): the badge stays
/// purely presentational, so entries that happen to share a badge no longer share nesting
/// behaviour.
pub(crate) trait CatalogEntry {
    fn category(&self) -> usize;
    fn badge(&self) -> &'static str;
    fn title(&self, loc: &Strings) -> &'static str;
    fn description(&self, loc: &Strings) -> &'static str;
    fn glyph(&self) -> &'static str;
    fn tint(&self, t: &Tokens) -> Color32;
    fn keywords(&self) -> &'static [&'static str];
    fn nestable(&self) -> bool;
}

/// One sidebar row. Index `0` is the "all" bucket; every other index matches `CatalogEntry::category`.
pub(crate) struct CatalogCategory {
    pub index: usize,
    pub glyph: &'static str,
    pub label: fn(&Strings) -> &'static str,
}

/// One catalog dialog's view state: the item table, its categories and the live filter inputs.
pub(crate) struct CatalogView<'a, T: CatalogEntry> {
    pub items: &'a [T],
    pub categories: &'a [CatalogCategory],
    pub selected_category: &'a mut usize,
    pub search_query: &'a mut String,
    /// True while picking a branch action: entries with `nestable == false` are hidden.
    pub nesting: bool,
    pub scroll_id: &'static str,
    pub search_hint: &'a str,
}

/// Case-insensitive match of one entry against an already-lowercased query.
fn catalog_entry_matches(
    entry: &impl CatalogEntry,
    query_lower: &str,
    loc: &Strings,
    nesting: bool,
) -> bool {
    if nesting && !entry.nestable() {
        return false;
    }
    if query_lower.is_empty() {
        return true;
    }
    entry.title(loc).to_lowercase().contains(query_lower)
        || entry.description(loc).to_lowercase().contains(query_lower)
        || entry.badge().to_lowercase().contains(query_lower)
        || entry.keywords().iter().any(|keyword| keyword.to_lowercase().contains(query_lower))
}

/// Indices of the visible entries, in table order. Category and query always apply together, so
/// "type to search" behaves the same in every catalog (U04-005).
pub(crate) fn filter_catalog_entries<T: CatalogEntry>(
    items: &[T],
    selected_category: usize,
    search_query: &str,
    loc: &Strings,
    nesting: bool,
) -> Vec<usize> {
    let query_lower = search_query.trim().to_lowercase();
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            (selected_category == 0 || item.category() == selected_category)
                && catalog_entry_matches(*item, &query_lower, loc, nesting)
        })
        .map(|(idx, _)| idx)
        .collect()
}

/// A catalog click only commits while its modal is open. `Modal` keeps painting (and accepting
/// pointer input on) its contents through the 160 ms close animation, so an unguarded commit
/// inserted the same action twice on a double click (U04-002).
pub(crate) fn catalog_commit_allowed(is_open: bool) -> bool {
    is_open
}

/// Header slice shared by the catalog dialogs: eyebrow label on the left, close button on the right.
pub(crate) fn render_catalog_header(
    frame: &mut ModalFrame<'_>,
    t: &Tokens,
    eyebrow: &str,
    close_tip: &str,
) {
    frame.header(|ui, close_req| {
        ui.horizontal_centered(|ui| {
            ui.add_space(18.0);
            widgets::hud(ui, eyebrow, t.ink);

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(16.0);
                if widgets::glyph_button(ui, Glyph::Close, close_tip).clicked() {
                    *close_req = true;
                }
            });
        });
    });
}

/// Shared catalog body: category sidebar, search row, two-column card grid and empty state.
///
/// Returns the index into `view.items` of the card clicked on this frame.
pub(crate) fn render_catalog_body<T: CatalogEntry>(
    ui: &mut egui::Ui,
    t: &Tokens,
    loc: &Strings,
    view: &mut CatalogView<'_, T>,
) -> Option<usize> {
    let body_rect = ui.max_rect();
    let sidebar_w = 195.0;
    let sidebar_rect = Rect::from_min_size(
        pos2(body_rect.min.x, body_rect.top() + 1.0),
        vec2(sidebar_w, body_rect.height() - 2.0),
    );
    let cards_rect = Rect::from_min_max(
        pos2(sidebar_rect.right() + 1.0, body_rect.top() + 1.0),
        pos2(body_rect.right() - 1.0, body_rect.bottom() - 1.0),
    );

    // Body background surfaces & divider
    let p = ui.painter();
    p.rect_filled(sidebar_rect, CornerRadius::ZERO, t.bg);
    p.rect_filled(cards_rect, CornerRadius::ZERO, t.sunken);

    // Seamless 1px geometric vertical divider without line-cap protrusion (DPI-independent)
    let v_divider = Rect::from_min_max(
        pos2(sidebar_rect.right(), body_rect.top() + 1.0),
        pos2(sidebar_rect.right() + 1.0, body_rect.bottom() - 1.0),
    );
    p.rect_filled(v_divider, CornerRadius::ZERO, t.hairline().color);

    // Left: Categories List
    ui.scope_builder(egui::UiBuilder::new().max_rect(sidebar_rect), |ui| {
        egui::Frame::new()
            .inner_margin(Margin::symmetric(10, 10))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;

                let count_for = |cat_idx: usize| -> usize {
                    view.items
                        .iter()
                        .filter(|item| {
                            (item.nestable() || !view.nesting)
                                && (cat_idx == 0 || item.category() == cat_idx)
                        })
                        .count()
                };

                let categories: Vec<(usize, &'static str, &'static str, usize)> = view
                    .categories
                    .iter()
                    .map(|category| {
                        (
                            category.index,
                            category.glyph,
                            (category.label)(loc),
                            count_for(category.index),
                        )
                    })
                    .collect();

                for (cat_idx, icon, cat_name, count) in categories {
                    let is_active = *view.selected_category == cat_idx;
                    let bg = if is_active { t.accent_ink.gamma_multiply(0.12) } else { Color32::TRANSPARENT };
                    let fg = if is_active { t.accent_ink } else { t.muted };

                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
                    if resp.hovered() && !is_active {
                        ui.painter().rect_filled(rect, CornerRadius::same(4), t.sunken);
                    }
                    if is_active {
                        ui.painter().rect_filled(rect, CornerRadius::same(4), bg);
                    }

                    ui.painter().text(
                        pos2(rect.left() + 10.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        icon,
                        FontId::new(14.0, theme::phosphor()),
                        fg,
                    );

                    ui.painter().text(
                        pos2(rect.left() + 32.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        cat_name,
                        FontId::new(12.5, theme::semibold()),
                        fg,
                    );

                    ui.painter().text(
                        pos2(rect.right() - 8.0, rect.center().y),
                        egui::Align2::RIGHT_CENTER,
                        format!("{count}"),
                        FontId::new(11.0, theme::mono()),
                        t.faint,
                    );

                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        *view.selected_category = cat_idx;
                    }
                }
            });
    });

    let filtered = filter_catalog_entries(
        view.items,
        *view.selected_category,
        view.search_query.as_str(),
        loc,
        view.nesting,
    );

    let mut clicked = None;

    // Right: Cards Grid in a ScrollArea with the Search Bar above
    ui.scope_builder(egui::UiBuilder::new().max_rect(cards_rect), |ui| {
        ui.add_space(10.0);

        // Search box placed directly above cards scroll area
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(10.0);
            let search_w = (ui.available_width() - 10.0).max(60.0);
            ui.allocate_ui_with_layout(
                vec2(search_w, 32.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    widgets::catalog_search_box(ui, view.search_query, view.search_hint);
                },
            );
        });
        ui.add_space(6.0);

        egui::ScrollArea::vertical()
            .id_salt(view.scroll_id)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if filtered.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(60.0);
                        ui.label(
                            RichText::new(egui_phosphor::regular::MAGNIFYING_GLASS)
                                .family(theme::phosphor())
                                .size(32.0)
                                .color(t.faint),
                        );
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(loc.catalog_no_matches)
                                .family(theme::medium())
                                .size(13.0)
                                .color(t.muted),
                        );
                    });
                } else {
                    let total_w = ui.available_width() - 20.0;
                    let col_count = 2;
                    let gap = 10.0;
                    let card_w = ((total_w - gap) / (col_count as f32)).max(160.0);
                    let card_h = 68.0;

                    for chunk in filtered.chunks(col_count) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            ui.add_space(10.0);
                            for (idx, &item_idx) in chunk.iter().enumerate() {
                                if idx > 0 {
                                    ui.add_space(gap);
                                }
                                let item = &view.items[item_idx];
                                let (rect, resp) = ui.allocate_exact_size(vec2(card_w, card_h), Sense::click());
                                let is_hovered = resp.hovered();

                                let bg_col = if is_hovered { t.sunken } else { t.bg };
                                let stroke_col = if is_hovered { t.accent_ink } else { t.line };

                                ui.painter().rect_filled(rect, CornerRadius::same(6), bg_col);
                                ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, stroke_col), egui::StrokeKind::Inside);

                                // Icon Chip on Left
                                let chip_rect = Rect::from_center_size(
                                    pos2(rect.left() + 26.0, rect.center().y),
                                    vec2(34.0, 34.0),
                                );
                                let item_tint = item.tint(t);
                                ui.painter().rect_filled(chip_rect, CornerRadius::same(6), item_tint.gamma_multiply(0.18));
                                ui.painter().text(
                                    chip_rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    item.glyph(),
                                    FontId::new(18.0, theme::phosphor()),
                                    item_tint,
                                );

                                // Text content on Right (vertically centered relative to card & icon chip)
                                let text_left = rect.left() + 52.0;
                                let title_text = item.title(loc);
                                let desc_text = item.description(loc);
                                let desc_w = (rect.right() - text_left - 10.0).max(40.0);

                                let title_galley = ui.painter().layout_no_wrap(
                                    title_text.to_string(),
                                    FontId::new(13.0, theme::semibold()),
                                    t.ink,
                                );
                                let desc_galley = ui.painter().layout(
                                    desc_text.to_string(),
                                    FontId::new(11.0, theme::medium()),
                                    t.muted,
                                    desc_w,
                                );

                                let title_h = title_galley.size().y;
                                let desc_h = desc_galley.size().y;
                                let text_spacing = 3.0;
                                let total_text_h = title_h + text_spacing + desc_h;
                                let text_start_y = (rect.center().y - total_text_h * 0.5).round();

                                ui.painter().galley(pos2(text_left, text_start_y), title_galley, t.ink);
                                ui.painter().galley(pos2(text_left, text_start_y + title_h + text_spacing), desc_galley, t.muted);

                                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                    clicked = Some(item_idx);
                                }
                            }
                        });
                        ui.add_space(6.0);
                    }
                }
                ui.add_space(10.0);
            });
    });

    clicked
}

// ============================================================================
// Action catalog table
// ============================================================================

struct CatalogItem {
    category_idx: usize, // 1: Process, 2: Audio, 3: Automation, 4: System
    badge: &'static str,
    title: fn(&crate::i18n::Strings) -> &'static str,
    description: fn(&crate::i18n::Strings) -> &'static str,
    glyph: &'static str,
    tint: fn(&Tokens) -> Color32,
    keywords: &'static [&'static str],
    /// False only for actions that cannot be nested inside an If/Else branch.
    nestable: bool,
    constructor: fn() -> Action,
}

impl CatalogEntry for CatalogItem {
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
        self.nestable
    }
}
const CATALOG_ITEMS: [CatalogItem; 26] = [
    // 1. Process Management
    CatalogItem {
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
    CatalogItem {
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
    CatalogItem {
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
    CatalogItem {
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
    CatalogItem {
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
    CatalogItem {
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
    CatalogItem {
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
    CatalogItem {
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
    CatalogItem {
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

    // 3. Automation & Logic
    CatalogItem {
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
    CatalogItem {
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
    CatalogItem {
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
    CatalogItem {
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
    CatalogItem {
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
    CatalogItem {
        category_idx: 3,
        badge: "SCRIPT",
        nestable: true,
        title: |loc| loc.action_execute_script_title,
        description: |loc| loc.action_execute_script_desc,
        glyph: egui_phosphor::regular::CODE,
        tint: |t| t.accent_ink,
        keywords: &[
            "powershell", "pwsh", "ps1", "script", "code", "cmd", "batch", "bat", "lua", "terminal", "console",
            "павершелл", "скрипт", "код", "автоматизация", "сценарий", "батник", "терминал", "луа",
        ],
        constructor: || Action::ExecuteScript {
            interpreter: crate::models::ScriptType::PowerShell,
            script: String::new(),
            run_in_window: false,
        },
    },
    CatalogItem {
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
    CatalogItem {
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
    CatalogItem {
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
    CatalogItem {
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
    CatalogItem {
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
    CatalogItem {
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
    CatalogItem {
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
    CatalogItem {
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
    CatalogItem {
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
    CatalogItem {
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
    CatalogItem {
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

pub fn render_action_catalog_modal(ctx: &egui::Context, state: &mut GuiState) {
    let t = Tokens::get(ctx);
    let is_open = state.action_catalog.is_some();

    let active_modal = state
        .action_catalog
        .as_ref()
        .or(state.last_action_catalog.as_ref())
        .cloned();

    let Some(mut modal_state) = active_modal else { return };

    let loc = state.loc();
    let mut chosen_action = None;

    let modal_res = Modal::new("action-catalog-modal", is_open)
        .width_fraction(0.74)
        .height_fraction(0.76)
        .min_size(vec2(720.0, 500.0))
        .max_size(vec2(840.0, 640.0))
        .header_height(46.0)
        .show(ctx, &t, |frame| {
            render_catalog_header(frame, &t, loc.catalog_modal_eyebrow, loc.inspector_btn_cancel);

            frame.body(|ui| {
                let mut view = CatalogView {
                    items: &CATALOG_ITEMS,
                    categories: &ACTION_CATEGORIES,
                    selected_category: &mut modal_state.selected_category,
                    search_query: &mut modal_state.search_query,
                    nesting: modal_state.ifelse_target.is_some(),
                    scroll_id: "action_catalog_cards_scroll",
                    search_hint: loc.catalog_search_hint,
                };
                chosen_action = render_catalog_body(ui, &t, loc, &mut view);
            });
        });

    // `Modal` keeps its card grid clickable through the close animation; only a click on the still
    // open dialog may commit (U04-002).
    if !catalog_commit_allowed(is_open) {
        if modal_res.is_none() {
            state.last_action_catalog = None;
        }
        return;
    }

    if let Some(idx) = chosen_action {
        let action = (CATALOG_ITEMS[idx].constructor)();
        state.add_action_from_catalog(action, modal_state.is_exit_pipeline, modal_state.insert_idx, modal_state.ifelse_target);
    } else if let Some(res) = modal_res {
        if res.close_requested {
            state.close_action_catalog();
        } else if is_open {
            state.action_catalog = Some(modal_state.clone());
            state.last_action_catalog = Some(modal_state);
        }
    } else if !is_open {
        state.last_action_catalog = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
