//! Element Catalog Modal:
//! Studio modal for browsing and selecting overlay elements and widgets
//! with categorized tabs, instant fuzzy keyword search, and rich capability cards.
//!
//! The layout, filtering and card rendering live in the shared catalog primitives
//! (`render_catalog_body`), so this file only owns the element table and the commit path.

use egui::vec2;
use crate::gui::components::catalog_modal::{
    CatalogCategory, CatalogChrome, CatalogEntryDef, CatalogModal,
};
use crate::gui::state::GuiState;
use crate::gui::theme::Tokens;
use crate::models::{
    CrosshairStyle, GraphMetric, GraphStyle, InteractiveControlKind, KeyVisualizerLayout,
    OverlayElement, ShapeKind, TelemetryKind, TelemetryStyle,
    TimerMode,
};

/// How one element table row builds its default model value: the id prefix used to allocate a
/// free numeric suffix plus the constructor that receives it.
#[derive(Clone, Copy)]
pub(crate) struct ElementBuild {
    pub id_prefix: &'static str,
    pub make: fn(usize) -> OverlayElement,
}

pub(crate) type ElementEntry = CatalogEntryDef<ElementBuild>;

const CATALOG_ITEMS: [ElementEntry; 10] = [
    // 1. Visual & Display
    CatalogEntryDef {
        category_idx: 1,
        badge: "DISPLAY",
        nestable: true,
        title: |loc| loc.elem_cat_title_text,
        description: |loc| loc.elem_cat_desc_text,
        glyph: egui_phosphor::regular::TEXT_T,
        tint: |t| t.accent_ink,
        keywords: &[
            "text", "label", "string", "font", "title", "caption",
            "текст", "надпись", "метка", "шрифт", "заголовок",
        ],
        constructor: ElementBuild {
            id_prefix: "text",
            make: |id_num| OverlayElement::Text {
                id: format!("text_{id_num}"),
                text: "Text Label".to_string(),
                font_size: 14.0,
                color: [255, 255, 255, 255],
                x: 16.0,
                y: 16.0,
                width: None,
                height: None,
                lock_aspect: false,
                bold: false,
                font_family: "Segoe UI".to_string(),
                rotation: 0.0,
                on_click: None,
            },
        },
    },
    CatalogEntryDef {
        category_idx: 1,
        badge: "IMAGE",
        nestable: true,
        title: |loc| loc.elem_cat_title_image,
        description: |loc| loc.elem_cat_desc_image,
        glyph: egui_phosphor::regular::IMAGE,
        tint: |t| t.accent_ink,
        keywords: &[
            "image", "picture", "photo", "png", "jpg", "ico", "icon", "logo",
            "изображение", "картинка", "фото", "логотип", "иконка",
        ],
        constructor: ElementBuild {
            id_prefix: "image",
            make: |id_num| OverlayElement::Image {
                id: format!("image_{id_num}"),
                path: "assets/app.png".to_string(),
                x: 20.0,
                y: 20.0,
                width: 64.0,
                height: 64.0,
                opacity: 1.0,
                lock_aspect: true,
                rotation: 0.0,
                on_click: None,
            },
        },
    },
    CatalogEntryDef {
        category_idx: 1,
        badge: "SHAPE",
        nestable: true,
        title: |loc| loc.elem_cat_title_shape,
        description: |loc| loc.elem_cat_desc_shape,
        glyph: egui_phosphor::regular::SHAPES,
        tint: |t| t.faint,
        keywords: &[
            "shape", "rect", "rectangle", "circle", "line", "arrow", "geometry",
            "фигура", "прямоугольник", "круг", "линия", "стрелка", "геометрия",
        ],
        constructor: ElementBuild {
            id_prefix: "shape",
            make: |id_num| OverlayElement::Shape {
                id: format!("shape_{id_num}"),
                shape_type: ShapeKind::Rectangle,
                x: 16.0,
                y: 16.0,
                width: 80.0,
                height: 40.0,
                thickness: 2.0,
                color: [255, 255, 255, 200],
                filled: true,
                fill_color: [40, 50, 70, 160],
                corner_radius: 6.0,
                lock_aspect: false,
                rotation: 0.0,
                on_click: None,
            },
        },
    },

    // 2. Interactive & Controls
    CatalogEntryDef {
        category_idx: 2,
        badge: "CONTROL",
        nestable: true,
        title: |loc| loc.elem_cat_title_control,
        description: |loc| loc.elem_cat_desc_control,
        glyph: egui_phosphor::regular::SLIDERS,
        tint: |t| t.accent_ink,
        keywords: &[
            "control", "interactive", "switch", "toggle", "button", "slider", "knob", "checkbox",
            "click", "mute", "action", "volume", "sound", "track", "press", "trigger", "value",
            "управление", "контрол", "интерактивный", "кнопка", "переключатель", "тумблер", "слайдер",
            "ползунок", "клик", "действие", "нажатие", "громкость", "звук", "вкл", "выкл", "значение",
        ],
        constructor: ElementBuild {
            id_prefix: "control",
            make: |id_num| OverlayElement::InteractiveControl {
                id: format!("control_{id_num}"),
                control: InteractiveControlKind::Button {
                    label: "Action".to_string(),
                    icon: None,
                    action: None,
                },
                color: [255, 255, 255, 255],
                bg_color: [35, 40, 50, 200],
                active_color: [37, 99, 235, 220],
                x: 16.0,
                y: 16.0,
                width: 110.0,
                height: 28.0,
                rotation: 0.0,
            },
        },
    },

    // 3. Hardware & Telemetry
    CatalogEntryDef {
        category_idx: 3,
        badge: "GRAPH",
        nestable: true,
        title: |loc| loc.elem_cat_title_graph,
        description: |loc| loc.elem_cat_desc_graph,
        glyph: egui_phosphor::regular::CHART_LINE,
        tint: |t| t.accent_ink,
        keywords: &[
            "graph", "chart", "history", "cpu", "ram", "network", "fps", "histogram",
            "график", "чарт", "история", "память", "сеть", "фпс", "гистограмма",
        ],
        constructor: ElementBuild {
            id_prefix: "graph",
            make: |id_num| OverlayElement::Graph {
                id: format!("graph_{id_num}"),
                metric: GraphMetric::SystemCpu,
                style: GraphStyle::AreaFilled,
                sample_count: 30,
                color: [59, 130, 246, 255],
                fill_color: [37, 99, 235, 90],
                bg_color: [18, 22, 32, 210],
                show_current_val: true,
                x: 16.0,
                y: 16.0,
                width: 160.0,
                height: 65.0,
                rotation: 0.0,
            },
        },
    },
    CatalogEntryDef {
        category_idx: 3,
        badge: "CLOCK",
        nestable: true,
        title: |loc| loc.elem_cat_title_clock,
        description: |loc| loc.elem_cat_desc_clock,
        glyph: egui_phosphor::regular::CLOCK,
        tint: |t| t.warn,
        keywords: &[
            "clock", "time", "stopwatch", "timer", "countdown", "uptime",
            "часы", "время", "секундомер", "таймер", "аптайм",
        ],
        constructor: ElementBuild {
            id_prefix: "clock",
            make: |id_num| OverlayElement::ClockTimer {
                id: format!("clock_{id_num}"),
                mode: TimerMode::Clock,
                format_str: "%H:%M:%S".to_string(),
                countdown_total_ms: 300_000,
                countdown_remaining_ms: 300_000,
                stopwatch_elapsed_ms: 0,
                is_running: false,
                font_size: 16.0,
                color: [255, 255, 255, 255],
                bg_color: [18, 22, 32, 200],
                bold: true,
                font_family: "Inter".to_string(),
                x: 16.0,
                y: 16.0,
                width: 100.0,
                height: 32.0,
                rotation: 0.0,
                on_finish: None,
            },
        },
    },
    CatalogEntryDef {
        category_idx: 3,
        badge: "TELEMETRY",
        nestable: true,
        title: |loc| loc.elem_cat_title_telemetry,
        description: |loc| loc.elem_cat_desc_telemetry,
        glyph: egui_phosphor::regular::ACTIVITY,
        tint: |t| t.good,
        keywords: &[
            "telemetry", "battery", "audio", "mic", "engine", "status", "pulse",
            "телеметрия", "батарея", "звук", "микрофон", "движок", "пульс",
        ],
        constructor: ElementBuild {
            id_prefix: "telemetry",
            make: |id_num| OverlayElement::SystemTelemetry {
                id: format!("telemetry_{id_num}"),
                kind: TelemetryKind::EngineStatus,
                style: TelemetryStyle::Compact,
                color: [255, 255, 255, 255],
                bg_color: [18, 22, 32, 200],
                x: 16.0,
                y: 16.0,
                width: 130.0,
                height: 26.0,
                rotation: 0.0,
                on_click: None,
            },
        },
    },

    // 4. Gaming & Media
    CatalogEntryDef {
        category_idx: 4,
        badge: "KEYS",
        nestable: true,
        title: |loc| loc.elem_cat_title_keys,
        description: |loc| loc.elem_cat_desc_keys,
        glyph: egui_phosphor::regular::KEYBOARD,
        tint: |t| t.accent_ink,
        keywords: &[
            "key", "keyboard", "mouse", "visualizer", "wasd", "rpg", "input",
            "клавиатура", "мышь", "визуализатор", "нажатия", "гейминг",
        ],
        constructor: ElementBuild {
            id_prefix: "keys",
            make: |id_num| OverlayElement::KeyVisualizer {
                id: format!("keys_{id_num}"),
                layout: KeyVisualizerLayout::WasdMouse,
                max_keys: 5,
                decay_seconds: 1.5,
                show_modifiers_only: false,
                color: [255, 255, 255, 255],
                bg_color: [16, 20, 28, 220],
                active_color: [59, 130, 246, 255],
                x: 16.0,
                y: 16.0,
                width: 220.0,
                height: 110.0,
                rotation: 0.0,
            },
        },
    },
    CatalogEntryDef {
        category_idx: 4,
        badge: "RETICLE",
        nestable: true,
        title: |loc| loc.elem_cat_title_crosshair,
        description: |loc| loc.elem_cat_desc_crosshair,
        glyph: egui_phosphor::regular::CROSSHAIR,
        tint: |t| t.good,
        keywords: &[
            "crosshair", "reticle", "aim", "sight", "fps", "gaming", "dot", "cross",
            "прицел", "перекрестие", "точка", "аим", "шутер", "гейминг",
        ],
        constructor: ElementBuild {
            id_prefix: "crosshair",
            make: |id_num| OverlayElement::Crosshair {
                id: format!("crosshair_{id_num}"),
                style: CrosshairStyle::Cross,
                size: 12.0,
                color: [34, 197, 94, 255],
                center_dot: true,
                thickness: 2.0,
                gap: 4.0,
                x: 0.0,
                y: 0.0,
                rotation: 0.0,
            },
        },
    },
    CatalogEntryDef {
        category_idx: 4,
        badge: "MEDIA",
        nestable: true,
        title: |loc| loc.elem_cat_title_media,
        description: |loc| loc.elem_cat_desc_media,
        glyph: egui_phosphor::regular::PLAY_CIRCLE,
        tint: |t| t.accent_ink,
        keywords: &[
            "media", "player", "music", "song", "track", "youtube", "spotify", "equalizer",
            "медиа", "плеер", "музыка", "трек", "песня", "эквалайзер", "спотифай",
        ],
        constructor: ElementBuild {
            id_prefix: "media",
            make: |id_num| OverlayElement::MediaWidget {
                id: format!("media_{id_num}"),
                show_controls: true,
                compact: false,
                color: [255, 255, 255, 255],
                bg_color: [18, 22, 32, 220],
                x: 16.0,
                y: 16.0,
                width: 210.0,
                height: 85.0,
                rotation: 0.0,
            },
        },
    },
];

const ELEMENT_CATEGORIES: [CatalogCategory; 5] = [
    CatalogCategory { index: 0, glyph: egui_phosphor::regular::STACK, label: |loc| loc.element_catalog_cat_all },
    CatalogCategory { index: 1, glyph: egui_phosphor::regular::TEXT_T, label: |loc| loc.element_catalog_cat_visual },
    CatalogCategory { index: 2, glyph: egui_phosphor::regular::TOGGLE_RIGHT, label: |loc| loc.element_catalog_cat_interactive },
    CatalogCategory { index: 3, glyph: egui_phosphor::regular::ACTIVITY, label: |loc| loc.element_catalog_cat_telemetry },
    CatalogCategory { index: 4, glyph: egui_phosphor::regular::GAME_CONTROLLER, label: |loc| loc.element_catalog_cat_gaming_media },
];

/// Next unused `{prefix}_{n}` suffix for a new overlay element id (U04-006).
///
/// `elements.len() + 1` reused an id after any deletion (or for a non-sequential overlay), and the
/// duplicate then persisted into `library.json` and share payloads.
fn next_free_id_number(elements: &[OverlayElement], prefix: &str) -> usize {
    let mut used = std::collections::HashSet::new();
    for element in elements {
        if let Some(number) = element
            .id()
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_prefix('_'))
            .and_then(|digits| digits.parse::<usize>().ok())
        {
            used.insert(number);
        }
    }
    (1..).find(|number| !used.contains(number)).unwrap_or(1)
}

pub(crate) const ELEMENT_CHROME: CatalogChrome = CatalogChrome {
    modal_id: "element-catalog-modal",
    eyebrow: |loc| loc.element_catalog_modal_eyebrow,
    close_tip: |loc| loc.inspector_btn_cancel,
    search_hint: |loc| loc.element_catalog_search_hint,
    scroll_id: "element_catalog_cards_scroll",
    width_fraction: 0.72,
    height_fraction: 0.74,
    min_size: vec2(700.0, 480.0),
    max_size: vec2(820.0, 620.0),
};

/// Commits one catalog card into the open overlay studio: undo snapshot, free id, default
/// construction, smart placement and selection. Extracted from the render loop so the invariant
/// can be unit-tested without a pointer click.
pub(crate) fn commit_element_choice(state: &mut GuiState, item: &ElementEntry) {
    if let Some(studio) = &mut state.overlay_studio {
        studio.push_undo_snapshot();
        let next_id = next_free_id_number(&studio.overlay.elements, item.constructor.id_prefix);
        let mut elem = (item.constructor.make)(next_id);

        // Compute smart placement on card
        let (ew, eh) = crate::gui::components::overlay_renderer::get_element_size(&elem);
        let offset = (studio.overlay.elements.len() as f32 * 18.0) % (studio.overlay.rect.height * 0.4).max(30.0);
        let spawn_x = ((studio.overlay.rect.width - ew) * 0.5).max(12.0);
        let spawn_y = (((studio.overlay.rect.height - eh) * 0.5) + offset).max(12.0);

        if !matches!(elem, OverlayElement::Crosshair { .. }) {
            *elem.x_mut() = spawn_x.round();
            *elem.y_mut() = spawn_y.round();
        }

        studio.overlay.elements.push(elem);
        studio.selected_element_idx = Some(studio.overlay.elements.len() - 1);
    }
    state.close_element_catalog();
}

pub fn render_element_catalog_modal(ctx: &egui::Context, state: &mut GuiState) {
    let t = Tokens::get(ctx);
    let is_open = state.element_catalog.is_some();
    let Some(mut modal_state) = state
        .element_catalog
        .clone()
        .or_else(|| state.last_element_catalog.clone())
    else {
        return;
    };
    let loc = state.loc();

    let outcome = CatalogModal::new(ELEMENT_CHROME, &CATALOG_ITEMS, &ELEMENT_CATEGORIES)
        .show(
            ctx,
            &t,
            loc,
            &mut modal_state.selected_category,
            &mut modal_state.search_query,
            is_open,
        );

    // Element keeps its chosen-before-close ordering and always closes after a successful commit.
    if let Some(idx) = outcome.chosen {
        commit_element_choice(state, &CATALOG_ITEMS[idx]);
    } else if outcome.close_requested {
        state.close_element_catalog();
    } else if is_open {
        state.element_catalog = Some(modal_state.clone());
        state.last_element_catalog = Some(modal_state);
    } else if outcome.fully_closed {
        // Once fully closed the cached state must go away: it was cloned every frame and kept
        // `Modal::show` running for a dialog that could no longer be seen (U04-013).
        state.last_element_catalog = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::components::catalog_modal::{filter_catalog_entries, CatalogEntry};
    use crate::i18n;

    #[test]
    fn test_element_catalog_items_construct_and_categories() {
        assert_eq!(CATALOG_ITEMS.len(), 10);
        let loc = i18n::get(i18n::Language::English);

        for (idx, item) in CATALOG_ITEMS.iter().enumerate() {
            let elem = (item.constructor.make)(idx + 1);
            let (w, h) = crate::gui::components::overlay_renderer::get_element_size(&elem);
            assert!(w > 0.0, "Element {} has invalid width: {}", elem.id(), w);
            assert!(h > 0.0, "Element {} has invalid height: {}", elem.id(), h);

            let title = item.title(loc);
            let desc = item.description(loc);
            assert!(!title.is_empty(), "Title empty for item {}", idx);
            assert!(!desc.is_empty(), "Description empty for item {}", idx);
            assert!((1..=4).contains(&item.category()));
            assert!(item.nestable(), "overlay elements are always nestable");
            assert_eq!(
                elem.id(),
                format!("{}_{}", item.constructor.id_prefix, idx + 1),
                "id_prefix must match the constructor's id for item {}",
                idx
            );
        }
    }

    #[test]
    fn test_next_free_element_id_skips_ids_still_in_use() {
        let text = |n: usize| (CATALOG_ITEMS[0].constructor.make)(n);

        // Sequential ids: the next number follows the list.
        assert_eq!(next_free_id_number(&[text(1), text(2)], "text"), 3);

        // A deleted middle element must not be reused by count-based numbering (U04-006).
        assert_eq!(next_free_id_number(&[text(1), text(3)], "text"), 2);
        assert_eq!(next_free_id_number(&[text(2), text(3)], "text"), 1);

        // Other kinds do not affect this prefix, and an empty studio starts at 1.
        assert_eq!(next_free_id_number(&[], "text"), 1);
        let image = |n: usize| (CATALOG_ITEMS[1].constructor.make)(n);
        assert_eq!(next_free_id_number(&[image(1), image(2), text(1)], "text"), 2);
    }

    #[test]
    fn test_element_catalog_filter_applies_category_and_query_together() {
        let loc = i18n::get(i18n::Language::English);
        let visible = filter_catalog_entries(&CATALOG_ITEMS, 2, "", loc, false);
        assert_eq!(visible.len(), 1, "the interactive category has one element");
        assert_eq!(CATALOG_ITEMS[visible[0]].category(), 2);

        // A query that matches only a different category stays filtered out.
        assert!(filter_catalog_entries(&CATALOG_ITEMS, 2, "crosshair", loc, false).is_empty());
        // Searching the whole table still finds it.
        assert!(!filter_catalog_entries(&CATALOG_ITEMS, 0, "crosshair", loc, false).is_empty());
    }

    #[test]
    fn test_closed_element_catalog_clears_cached_modal_state() {
        let ctx = egui::Context::default();
        crate::gui::theme::install_fonts(&ctx);
        let mut state = GuiState::new();

        let frame = |time: f64, state: &mut GuiState| {
            let mut out = ctx.run_ui(
                egui::RawInput { time: Some(time), ..Default::default() },
                |ui| render_element_catalog_modal(ui.ctx(), state),
            );
            out.textures_delta.clear();
        };

        state.open_element_catalog();
        frame(0.0, &mut state);
        frame(0.4, &mut state);
        assert!(state.last_element_catalog.is_some());

        // Close and let the 160 ms cinematic close animation finish.
        state.close_element_catalog();
        frame(0.6, &mut state);
        frame(0.9, &mut state);
        frame(1.2, &mut state);

        assert!(
            state.last_element_catalog.is_none(),
            "a fully closed element catalog must drop its cached state (U04-013)"
        );
    }

    #[test]
    fn test_overlay_studio_undo_redo_lifecycle() {
        let mut state = GuiState::new();
        state.open_overlay_studio("overlay_custom");
        let studio = state.overlay_studio.as_mut().unwrap();

        assert!(!studio.can_undo());
        assert!(!studio.can_redo());
        let initial_count = studio.overlay.elements.len();

        // Snapshot 1: before adding an element
        studio.push_undo_snapshot();
        assert!(studio.can_undo());
        assert!(!studio.can_redo());

        let elem = (CATALOG_ITEMS[0].constructor.make)(initial_count + 1);
        studio.overlay.elements.push(elem);
        assert_eq!(studio.overlay.elements.len(), initial_count + 1);

        // Undo -> should revert to initial count
        studio.undo();
        assert_eq!(studio.overlay.elements.len(), initial_count);
        assert!(!studio.can_undo());
        assert!(studio.can_redo());

        // Redo -> should restore added element
        studio.redo();
        assert_eq!(studio.overlay.elements.len(), initial_count + 1);
        assert!(studio.can_undo());
        assert!(!studio.can_redo());
    }

    #[test]
    fn test_render_element_catalog_modal_no_crash() {
        let ctx = egui::Context::default();
        crate::gui::theme::install_fonts(&ctx);

        let mut state = GuiState::new();
        state.open_element_catalog();

        for _ in 0..3 {
            let mut out = ctx.run_ui(Default::default(), |ctx| {
                render_element_catalog_modal(ctx, &mut state);
            });
            out.textures_delta.clear();
        }

        state.close_element_catalog();
        for _ in 0..2 {
            let mut out = ctx.run_ui(Default::default(), |ctx| {
                render_element_catalog_modal(ctx, &mut state);
            });
            out.textures_delta.clear();
        }
    }

    #[test]
    fn test_element_catalog_commit_allocates_next_id_and_places() {
        let mut state = GuiState::new();
        state.overlay_studio = Some(crate::gui::state::OverlayStudioState::new(
            crate::models::OverlayDefinition::default_new_overlay(),
            None,
            false,
            String::new(),
        ));
        let text_item = CATALOG_ITEMS
            .iter()
            .find(|item| item.constructor.id_prefix == "text")
            .expect("text element entry");
        let crosshair_item = CATALOG_ITEMS
            .iter()
            .find(|item| item.constructor.id_prefix == "crosshair")
            .expect("crosshair element entry");

        let (rect_w, rect_h) = {
            let studio = state.overlay_studio.as_ref().expect("overlay studio");
            assert!(studio.overlay.elements.is_empty());
            assert!(!studio.can_undo());
            (studio.overlay.rect.width, studio.overlay.rect.height)
        };

        // First commit: free id, smart placement, selection and an undo snapshot.
        commit_element_choice(&mut state, text_item);
        assert!(state.element_catalog.is_none(), "a successful commit closes the catalog");
        {
            let studio = state.overlay_studio.as_ref().expect("overlay studio");
            assert_eq!(studio.overlay.elements.len(), 1);
            let elem = &studio.overlay.elements[0];
            assert_eq!(elem.id(), "text_1");
            let (ew, eh) = crate::gui::components::overlay_renderer::get_element_size(elem);
            assert_eq!(elem.x(), ((rect_w - ew) * 0.5).max(12.0).round());
            assert_eq!(elem.y(), ((rect_h - eh) * 0.5).max(12.0).round());
            assert_eq!(studio.selected_element_idx, Some(0));
            assert!(studio.can_undo(), "the commit must push an undo snapshot");
        }

        // Second commit: the id advances and the stagger offset follows the element count.
        commit_element_choice(&mut state, text_item);
        {
            let studio = state.overlay_studio.as_ref().expect("overlay studio");
            assert_eq!(studio.overlay.elements.len(), 2);
            let elem = &studio.overlay.elements[1];
            assert_eq!(elem.id(), "text_2");
            let (ew, eh) = crate::gui::components::overlay_renderer::get_element_size(elem);
            let offset = 18.0_f32 % (rect_h * 0.4).max(30.0);
            assert_eq!(elem.x(), ((rect_w - ew) * 0.5).max(12.0).round());
            assert_eq!(elem.y(), (((rect_h - eh) * 0.5) + offset).max(12.0).round());
            assert_eq!(studio.selected_element_idx, Some(1));
        }

        // A deleted middle element's id is not reused (U04-006).
        {
            let studio = state.overlay_studio.as_mut().expect("overlay studio");
            studio.overlay.elements.remove(0);
            studio.selected_element_idx = None;
        }
        commit_element_choice(&mut state, text_item);
        {
            let studio = state.overlay_studio.as_ref().expect("overlay studio");
            assert_eq!(studio.overlay.elements.len(), 2);
            assert_eq!(studio.overlay.elements[1].id(), "text_1");
        }

        // Crosshair keeps its authored x/y: smart placement does not apply to it.
        let before = state.overlay_studio.as_ref().unwrap().overlay.elements.len();
        commit_element_choice(&mut state, crosshair_item);
        {
            let studio = state.overlay_studio.as_ref().expect("overlay studio");
            assert_eq!(studio.overlay.elements.len(), before + 1);
            let elem = studio.overlay.elements.last().expect("committed element");
            assert!(matches!(elem, OverlayElement::Crosshair { .. }));
            assert_eq!(elem.id(), "crosshair_1");
            assert_eq!(elem.x(), 0.0);
            assert_eq!(elem.y(), 0.0);
            assert_eq!(studio.selected_element_idx, Some(before));
            assert!(studio.can_undo());
        }

        // The commit remains undoable like any other studio gesture.
        {
            let studio = state.overlay_studio.as_mut().expect("overlay studio");
            studio.undo();
            assert_eq!(studio.overlay.elements.len(), before);
            assert!(studio.can_redo());
            studio.redo();
            assert_eq!(studio.overlay.elements.len(), before + 1);
        }
    }
}
