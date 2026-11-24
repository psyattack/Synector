//! Top header bar: the brand wordmark, the centered rules search and the two header dialogs.
//!
//! The header owns the only rules search field. It is deliberately background-free and shorter
//! than a control row: it reads as a title-bar element, not as a form. The analytics and settings
//! dialogs are quiet glyph buttons to its right; the window caption buttons stay at the far right.

use egui::{Rect, Ui, pos2, vec2};
use crate::gui::components::widgets::{self, Glyph};
use crate::gui::state::GuiState;
use crate::gui::theme::Tokens;

/// Width the centered search asks for. It narrows only when the window is too small for the
/// glyphs and the caption buttons to keep their distance.
const SEARCH_W: f32 = 420.0;
/// Minimum width of the header search on a narrow window: below this the field stops being a
/// usable search and the layout would collide with the brand block.
const SEARCH_MIN_W: f32 = 260.0;
/// Width reserved for the brand wordmark on the left.
const BRAND_W: f32 = 260.0;
/// Width reserved for the window caption buttons on the right.
const CAPTION_W: f32 = 140.0;
/// Side of one header glyph button, the icon inside it and the gap between the buttons and the
/// search field. The size is the shared header control height, so the glyphs and the search field
/// match by construction and the header chrome stays the same in both visual generations.
const GLYPH: f32 = widgets::HEADER_SEARCH_H;
const GLYPH_ICON: f32 = 18.0;
const GLYPH_GAP: f32 = 6.0;
/// Breathing room between the glyph pair and the caption buttons.
const CAPTION_GAP: f32 = 12.0;

/// Width of the centered search for this window size. The field is centered on the window, so
/// both the distance to the brand block and the distance to the caption buttons bound it.
fn search_width(full: Rect) -> f32 {
    let center = full.center().x;
    let left_room = (center - (full.left() + BRAND_W)).max(0.0) * 2.0;
    let right_room = (full.right() - CAPTION_W - CAPTION_GAP - center
        - (GLYPH * 2.0 + GLYPH_GAP * 2.0))
        .max(0.0)
        * 2.0;
    SEARCH_W
        .min(left_room)
        .min(right_room)
        .max(SEARCH_MIN_W.min(full.width() * 0.5))
}

pub fn render_header_bar(ui: &mut Ui, state: &mut GuiState) {
    let t = Tokens::get(ui.ctx());
    let h = t.metrics().header_h;
    let full_rect = ui.max_rect();

    let loc = state.loc();

    // 1. Custom window title bar drag & double-click maximize
    widgets::title_bar_drag(ui);

    // 2. Left: Brand Wordmark
    let left_rect = Rect::from_min_size(full_rect.min, vec2(BRAND_W, h));
    ui.scope_builder(egui::UiBuilder::new().max_rect(left_rect), |ui| {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add_space(16.0);
            widgets::wordmark(ui, "SYN", "ECTOR", 16.0, h);
        });
    });

    // 3. Right: Window Caption Buttons
    let right_rect = Rect::from_min_size(
        pos2(full_rect.right() - CAPTION_W, full_rect.top()),
        vec2(CAPTION_W, h),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(right_rect), |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            widgets::window_buttons(ui, h, loc);
        });
    });

    // 4. Center: the rules search, exactly centered where the tabs used to be.
    let search_w = search_width(full_rect);
    let center = full_rect.center().x;
    let search_rect = Rect::from_center_size(
        pos2(center, full_rect.center().y),
        vec2(search_w, h),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(search_rect), |ui| {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            // A dialog can open without a click in the header (the update-available changelog does
            // it from the state sync), and its scrim owns the keyboard then: the field must not
            // keep collecting the typed query behind the scrim, and Escape belongs to the dialog.
            // The two header glyphs surrender the focus themselves; this covers the rest.
            if state.is_modal_open() {
                ui.ctx()
                    .memory_mut(|m| m.surrender_focus(widgets::header_search_id()));
            } else if state.search_overlay_open {
                // Escape closes the results panel first; the next press clears the field. The key
                // is consumed here so the same press can neither close the inspector nor reach
                // the edit.
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                    state.search_overlay_open = false;
                }
            } else if ui.memory(|m| m.has_focus(widgets::header_search_id()))
                && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
            {
                state.search_query.clear();
            }

            let response = widgets::header_search_box(
                ui,
                &mut state.search_query,
                loc.header_search_placeholder,
                search_w,
            );
            if state.search_query.trim().is_empty() {
                state.search_overlay_open = false;
            } else if response.changed() || response.gained_focus() || response.clicked() {
                state.search_overlay_open = true;
            }
        });
    });

    // 5. Right of the search: the analytics and settings dialog buttons. They use the standard
    // framed icon-button chrome with a larger mark; opening a dialog also drops the search focus
    // so typing cannot land in the field hidden under the modal scrim.
    let glyphs_w = GLYPH * 2.0 + GLYPH_GAP;
    let glyphs_rect = Rect::from_min_size(
        pos2(center + search_w * 0.5 + GLYPH_GAP, full_rect.top()),
        vec2(glyphs_w, h),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(glyphs_rect), |ui| {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = GLYPH_GAP;
            let analytics = widgets::glyph_button_outlined(
                ui,
                Glyph::Analytics,
                GLYPH,
                GLYPH_ICON,
                loc.header_analytics_tip,
            );
            let settings = widgets::glyph_button_outlined(
                ui,
                Glyph::Settings,
                GLYPH,
                GLYPH_ICON,
                loc.header_settings_tip,
            );
            if analytics.clicked() {
                ui.ctx()
                    .memory_mut(|m| m.surrender_focus(widgets::header_search_id()));
                state.open_analytics_modal();
            }
            if settings.clicked() {
                ui.ctx()
                    .memory_mut(|m| m.surrender_focus(widgets::header_search_id()));
                state.open_settings_modal();
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_width_respects_the_center_slot() {
        let wide = Rect::from_min_size(pos2(0.0, 0.0), vec2(1180.0, 48.0));
        assert_eq!(search_width(wide), SEARCH_W);

        let narrow = Rect::from_min_size(pos2(0.0, 0.0), vec2(880.0, 48.0));
        let width = search_width(narrow);
        assert!(width >= SEARCH_MIN_W, "was {width}");
        assert!(width < SEARCH_W, "the field must shrink before it collides");

        let center = narrow.center().x;
        let glyphs = GLYPH * 2.0 + GLYPH_GAP;
        assert!(
            center + width * 0.5 + GLYPH_GAP + glyphs <= narrow.right() - CAPTION_W,
            "the glyph pair must stay clear of the caption buttons"
        );
        assert!(
            center - width * 0.5 >= narrow.left() + BRAND_W,
            "the field must stay clear of the brand block"
        );
    }

    /// Typing into the focused header field opens the global results panel; Escape closes it
    /// first and clears the query only on the next press.
    #[test]
    fn typing_opens_the_results_and_escape_closes_then_clears() {
        let ctx = egui::Context::default();
        crate::gui::theme::install_fonts(&ctx);
        crate::gui::theme::apply_theme(
            &ctx,
            crate::config::AppTheme::Dark,
            crate::config::UiSkin::Modern,
            None,
        );
        let mut state = GuiState::new();

        let run = |ctx: &egui::Context,
                   state: &mut GuiState,
                   time: f64,
                   events: Vec<egui::Event>| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        pos2(0.0, 0.0),
                        vec2(1180.0, 48.0),
                    )),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.ctx()
                        .memory_mut(|memory| memory.request_focus(widgets::header_search_id()));
                    ui.set_min_size(vec2(1180.0, 48.0));
                    render_header_bar(ui, state);
                },
            );
            output.textures_delta.clear();
        };

        run(&ctx, &mut state, 0.0, vec![]);
        run(
            &ctx,
            &mut state,
            0.1,
            vec![egui::Event::Text("macro".to_string())],
        );
        assert_eq!(state.search_query, "macro");
        assert!(state.search_overlay_open, "typing must open the results panel");

        run(
            &ctx,
            &mut state,
            0.2,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(!state.search_overlay_open, "Escape closes the results first");
        assert_eq!(state.search_query, "macro", "the query survives the first Escape");

        run(
            &ctx,
            &mut state,
            0.3,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(state.search_query.is_empty(), "the second Escape clears the field");
    }

    #[test]
    fn header_paints_the_rules_search_and_no_tab_labels() {
        let ctx = egui::Context::default();
        crate::gui::theme::install_fonts(&ctx);
        crate::gui::theme::apply_theme(
            &ctx,
            crate::config::AppTheme::Dark,
            crate::config::UiSkin::Modern,
            None,
        );
        let mut state = GuiState::new();
        let loc = crate::i18n::get(crate::i18n::Language::English);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1180.0, 48.0))),
                ..Default::default()
            },
            |ui| {
                ui.set_min_size(vec2(1180.0, 48.0));
                render_header_bar(ui, &mut state);
            },
        );
        output.textures_delta.clear();

        let mut texts = Vec::new();
        for clipped in &output.shapes {
            collect_text(&clipped.shape, &mut texts);
        }
        let hint = loc.header_search_placeholder.to_uppercase();
        assert!(
            texts.iter().any(|text| text == &hint),
            "the header must paint the rules search hint: {texts:?}"
        );
        for tab in ["RULES", "ANALYTICS", "SETTINGS"] {
            assert!(
                !texts.iter().any(|text| text == tab),
                "the {tab} tab label must no longer exist: {texts:?}"
            );
        }
    }

    fn collect_text(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => out.push(text.galley.text().to_string()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect_text(shape, out)),
            _ => {}
        }
    }
}
