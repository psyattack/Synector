//! Overlay Studio modal shell: the studio and presets modal entry points, the public geometry
//! re-exports and the submodules that make up the studio.

use egui::{pos2, vec2, CornerRadius, Rect};

use crate::gui::components::modal::{render_modal_header, Modal, ModalHeader};
use crate::gui::components::widgets::{self, Kind};
use crate::gui::state::GuiState;
use crate::gui::theme::Tokens;

mod calibration;
mod canvas;
mod colors;
mod geometry;
mod history;
mod inspector;
mod presets;

pub use crate::gui::components::overlay_renderer::get_element_size;

use self::calibration::{
    apply_calibration_toggle, clamp_studio_geometry_if_changed, persist_studio_into_rule,
    push_calibration_preview, sync_calibration_position,
};
use self::canvas::render_overlay_canvas;
use self::history::{
    handle_studio_element_shortcuts, read_studio_history_shortcuts,
    render_inspector_with_edit_undo, StudioHistoryAction,
};

pub use self::geometry::{
    clamp_card_to_monitor, clamp_elements_to_canvas, max_card_dimensions, resolve_overlay_monitor,
};
pub use self::presets::render_overlay_presets_modal;

pub fn render_overlay_modal(ctx: &egui::Context, state: &mut GuiState) {
    let is_open = state.overlay_studio.is_some();
    let active_studio = state
        .overlay_studio
        .as_ref()
        .or(state.last_overlay_studio.as_ref())
        .cloned();

    let Some(mut studio) = active_studio else { return };
    // Continually request repaint for live telemetry, clock, and graph animations
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
    clamp_studio_geometry_if_changed(ctx, &mut studio.overlay);
    let t = Tokens::get(ctx);
    let loc = state.loc();
    let mut initial_overlay = studio.overlay.clone();

    let mut toggle_calibration = false;

    let monitors = crate::daemon::overlay_manager::get_system_monitors();
    let active_mon = resolve_overlay_monitor(&studio.overlay, &monitors);

    if sync_calibration_position(ctx, state, &mut studio, &active_mon) {
        initial_overlay = studio.overlay.clone();
    }

    let rules_list: Vec<(String, String)> = state
        .daemon_state
        .as_ref()
        .map(|s| s.config.rules.iter().map(|r| (r.id.clone(), r.name.clone())).collect())
        .unwrap_or_default();

    let mut open_element_catalog = false;

    // Studio keyboard shortcuts are processed before UI rendering so a key is consumed in the
    // frame it is pressed.
    let is_typing = ctx.text_edit_focused();
    if let Some(action) = read_studio_history_shortcuts(ctx, is_open, is_typing) {
        match action {
            StudioHistoryAction::Undo => studio.undo(),
            StudioHistoryAction::Redo => studio.redo(),
        }
        ctx.request_repaint();
    }
    handle_studio_element_shortcuts(ctx, &mut studio, is_open, is_typing);

    let modal_res = Modal::new("overlay-studio-modal", is_open)
        .width_fraction(0.92)
        .height_fraction(0.92)
        .min_size(vec2(920.0, 600.0))
        .max_size(vec2(1360.0, 940.0))
        .header_height(50.0)
        .close_on_escape(studio.active_color_target.is_none())
        .show(ctx, &t, |frame| {
            // ---------------- 1. HEADER ----------------
            frame.header(|ui, close_req| {
                render_modal_header(
                    ui,
                    close_req,
                    &t,
                    ModalHeader {
                        glyph: Some(egui_phosphor::regular::FRAME_CORNERS),
                        glyph_size: 16.0,
                        close_tip: loc.modal_tip_close,
                        left_pad: 18.0,
                        glyph_pad: 6.0,
                        right_pad: 16.0,
                        item_spacing_x: None,
                    },
                    |ui| { widgets::hud(ui, loc.overlay_studio_title, t.ink); },
                    |ui| {
                        // Calibrate Position on Screen Button
                        ui.add_space(8.0);
                        let cal_text = if studio.is_calibrating {
                            loc.overlay_studio_btn_calibrate_hide
                        } else {
                            loc.overlay_studio_btn_calibrate
                        };
                        let cal_kind = if studio.is_calibrating { Kind::Danger } else { Kind::Secondary };
                        if widgets::button(ui, cal_text, cal_kind, 0.0).clicked() {
                            toggle_calibration = true;
                        }

                        // Presets Button
                        ui.add_space(8.0);
                        let presets_kind = if studio.show_presets_popover { Kind::Primary } else { Kind::Secondary };
                        if widgets::button(ui, loc.overlay_studio_presets_btn, presets_kind, 0.0).clicked() {
                            studio.show_presets_popover = !studio.show_presets_popover;
                            if studio.show_presets_popover && studio.preset_save_name.trim().is_empty() {
                                studio.preset_save_name = studio.overlay.name.clone();
                            }
                        }
                    },
                );
            });

            // ---------------- 2. BODY: SPLIT LAYOUT ----------------
            frame.body(|ui| {
                let body_rect = ui.max_rect();
                let left_w = (body_rect.width() * 0.44).clamp(380.0, 520.0);
                let left_rect = Rect::from_min_size(body_rect.min, vec2(left_w, body_rect.height()));
                let right_rect = Rect::from_min_max(
                    pos2(left_rect.right() + 1.0, body_rect.min.y),
                    body_rect.max,
                );

                // Vertical Divider between panels
                let v_divider = Rect::from_min_max(
                    pos2(left_rect.right(), body_rect.top() + 1.0),
                    pos2(left_rect.right() + 1.0, body_rect.bottom() - 1.0),
                );
                ui.painter().rect_filled(v_divider, CornerRadius::ZERO, t.hairline().color);

                // LEFT PANEL: Overlay Settings & Elements Inspector
                ui.scope_builder(egui::UiBuilder::new().max_rect(left_rect), |ui| {
                    ui.set_width(left_rect.width());
                    ui.set_height(left_rect.height());
                    ui.set_clip_rect(left_rect);

                    egui::Frame::new()
                        .inner_margin(egui::Margin::same(14))
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt("overlay_studio_left_scroll")
                                .auto_shrink([false, false])
                                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                                .show(ui, |ui| {
                                    render_inspector_with_edit_undo(
                                        ui,
                                        &mut studio,
                                        &monitors,
                                        &rules_list,
                                        &mut open_element_catalog,
                                        &t,
                                        loc,
                                    );
                                });
                        });
                });

                // RIGHT PANEL: Canvas Preview
                ui.scope_builder(egui::UiBuilder::new().max_rect(right_rect), |ui| {
                    ui.set_width(right_rect.width());
                    ui.set_height(right_rect.height());
                    ui.set_clip_rect(right_rect);

                    egui::Frame::new()
                        .inner_margin(egui::Margin::same(14))
                        .show(ui, |ui| {
                            render_overlay_canvas(
                                ui,
                                &mut studio,
                                &t,
                                loc,
                            );
                        });
                });
            });
        });

    if modal_res.is_some() {
        if modal_res.as_ref().is_some_and(|r| r.close_requested) {
            state.close_overlay_studio();
        } else if is_open {
            if toggle_calibration {
                apply_calibration_toggle(state, &mut studio, &active_mon);
            }

            push_calibration_preview(state, &mut studio, &initial_overlay, &active_mon);

            persist_studio_into_rule(state, &studio);

            state.overlay_studio = Some(studio);
        }
    } else if !is_open {
        state.last_overlay_studio = None;
    }

    if open_element_catalog {
        state.open_element_catalog();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::components::text::fill;
    use crate::gui::theme;

    #[test]
    fn test_render_overlay_modal_no_crash() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);

        let presets = ["overlay_crosshair", "overlay_macro_badge", "overlay_telemetry", "unknown_custom"];
        for preset in presets {
            let mut state = GuiState::new();
            state.open_overlay_studio(preset);

            for _ in 0..5 {
                let mut out = ctx.run_ui(Default::default(), |ctx| {
                    render_overlay_modal(ctx, &mut state);
                });
                out.textures_delta.clear();
            }

            state.close_overlay_studio();
            for _ in 0..5 {
                let mut out = ctx.run_ui(Default::default(), |ctx| {
                    render_overlay_modal(ctx, &mut state);
                });
                out.textures_delta.clear();
            }
        }
    }

    #[test]
    fn test_overlay_elements_with_fonts_and_coordinates() {
        use crate::models::{OverlayDefinition, OverlayElement};

        let mut overlay = OverlayDefinition::default_crosshair();
        assert_eq!(overlay.elements.len(), 1);
        if let OverlayElement::Crosshair { x, y, .. } = overlay.elements[0] {
            assert_eq!(x, 24.0);
            assert_eq!(y, 24.0);
        } else {
            panic!("Expected crosshair element");
        }

        // Test text font family & click action
        overlay.elements.push(OverlayElement::Text {
            id: "txt_1".to_string(),
            text: "FPS: 144".to_string(),
            font_size: 14.0,
            font_family: "Impact".to_string(),
            bold: false,
            color: [255, 255, 255, 255],
            x: 10.0,
            y: 10.0,
            width: None,
            height: None,
            lock_aspect: true,
            rotation: 0.0,
            on_click: Some(crate::models::OverlayClickAction::CloseOverlay),
        });

        // Test image element
        overlay.elements.push(OverlayElement::Image {
            id: "img_1".to_string(),
            path: "C:/test/sample.png".to_string(),
            x: 5.0,
            y: 5.0,
            width: 32.0,
            height: 32.0,
            opacity: 0.9,
            lock_aspect: true,
            rotation: 0.0,
            on_click: None,
        });

        // Test shape element
        overlay.elements.push(OverlayElement::Shape {
            id: "shape_1".to_string(),
            shape_type: crate::models::ShapeKind::Rectangle,
            x: 20.0,
            y: 20.0,
            width: 40.0,
            height: 40.0,
            thickness: 2.0,
            color: [255, 0, 0, 255],
            filled: true,
            fill_color: [0, 255, 0, 100],
            corner_radius: 6.0,
            lock_aspect: true,
            rotation: 0.0,
            on_click: None,
        });

        assert_eq!(overlay.elements.len(), 4);
    }

    // ---------------- U02-012: studio strings come from the catalog ----------------

    #[test]
    fn test_studio_catalog_values_drive_the_rendered_labels() {
        let en = crate::i18n::get(crate::i18n::Language::English);
        let ru = crate::i18n::get(crate::i18n::Language::Russian);

        // Field prefixes and units carry their own separator/space in the catalog.
        assert_eq!(en.overlay_studio_field_screen_x, "Screen X: ");
        assert_eq!(ru.overlay_studio_field_screen_x, "Экран X: ");
        assert_eq!(en.overlay_studio_field_w, "W: ");
        assert_eq!(ru.overlay_studio_field_w, "Ш: ");
        assert_eq!(en.overlay_studio_unit_px, " px");
        assert_eq!(en.overlay_studio_unit_pt, " pt");

        // Colour chips and palette swatches are localised.
        assert_eq!(en.overlay_studio_chip_bg, "BG:");
        assert_eq!(ru.overlay_studio_chip_bg, "ФОН:");
        assert_eq!(en.overlay_studio_chip_text, "TEXT:");
        assert_eq!(ru.overlay_studio_chip_text, "ТЕКСТ:");
        assert_ne!(en.overlay_studio_palette_cyan, ru.overlay_studio_palette_cyan);
        assert_ne!(en.overlay_studio_crosshair_cross, ru.overlay_studio_crosshair_cross);

        // Badges, control defaults and the preset fallback are catalog values, not literals.
        assert_ne!(en.overlay_studio_badge_crosshair, ru.overlay_studio_badge_crosshair);
        assert_eq!(en.overlay_studio_control_default_action, "Action");
        assert_eq!(ru.overlay_studio_control_default_action, "Действие");
        assert_eq!(en.overlay_default_preset_name, "Custom Overlay");
        assert_eq!(ru.overlay_default_preset_name, "Свой оверлей");
        assert_eq!(en.overlay_studio_on_click_toggle, "Toggle");
        assert_eq!(ru.overlay_studio_on_click_toggle, "Переключить");
        assert_eq!(en.overlay_studio_file_filter_image, "Image");
        assert_eq!(ru.overlay_studio_file_filter_image, "Изображение");

        // Preset save/delete failure toasts.
        assert_eq!(
            en.presets_save_invalid_name,
            "Preset name needs at least one letter or digit"
        );
        assert_eq!(en.presets_save_failed, "Could not save the preset; see the log");
        assert_eq!(
            en.presets_delete_failed,
            "Could not delete the preset; see the log"
        );
        assert_ne!(en.presets_save_failed, ru.presets_save_failed);
        assert_ne!(en.presets_delete_failed, ru.presets_delete_failed);
        assert_ne!(en.presets_save_invalid_name, ru.presets_save_invalid_name);

        // Multi-placeholder canvas labels are filled positionally.
        assert_eq!(
            fill(en.overlay_studio_canvas_display, &["1920", "1080"]),
            "DISPLAY 1920×1080"
        );
        assert_eq!(
            fill(ru.overlay_studio_canvas_display, &["1920", "1080"]),
            "МОНИТОР 1920×1080"
        );
        assert_eq!(fill(en.overlay_studio_canvas_overlay, &["400", "300"]), "OVERLAY 400×300");
        assert_eq!(
            fill(en.overlay_studio_canvas_coords, &["12", "34", "56", "78", "90"]),
            "X: 12 Y: 34 | 56×78 px | 90°"
        );
    }

    /// The production part of one studio source file: everything before its `#[cfg(test)]`
    /// module, with comment lines dropped.
    fn production_part(source: &'static str) -> String {
        let production = source
            .split_once("#[cfg(test)]")
            .map(|(production, _)| production)
            .unwrap_or(source);
        production
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Concatenation of every production file the studio guard scans: the module tree (plus the
    /// shared colour picker and preset write contract) and the shared preset picker. The list is
    /// checked against the `mod` declarations of this file, so a new submodule cannot silently
    /// escape the guard.
    fn overlay_module_source() -> String {
        let files = [
            ("overlay_modal.rs", production_part(include_str!("overlay_modal.rs"))),
            ("overlay_modal/calibration.rs", production_part(include_str!("overlay_modal/calibration.rs"))),
            ("overlay_modal/canvas.rs", production_part(include_str!("overlay_modal/canvas.rs"))),
            ("overlay_modal/colors.rs", production_part(include_str!("overlay_modal/colors.rs"))),
            ("overlay_modal/geometry.rs", production_part(include_str!("overlay_modal/geometry.rs"))),
            ("overlay_modal/history.rs", production_part(include_str!("overlay_modal/history.rs"))),
            ("overlay_modal/inspector.rs", production_part(include_str!("overlay_modal/inspector.rs"))),
            ("overlay_modal/presets.rs", production_part(include_str!("overlay_modal/presets.rs"))),
            ("views/presets.rs", production_part(include_str!("presets.rs"))),
            ("components/color_picker.rs", production_part(include_str!("../components/color_picker.rs"))),
            ("state/library_write.rs", production_part(include_str!("../state/library_write.rs"))),
        ];

        // Self-check: every `mod` declared by the studio root has its file in the list, and the
        // list adds no unlisted submodule.
        let declared: Vec<&str> = include_str!("overlay_modal.rs")
            .lines()
            .filter_map(|line| line.strip_prefix("mod "))
            .filter_map(|rest| rest.strip_suffix(';'))
            .filter(|name| *name != "tests")
            .collect();
        let studio_files: Vec<&str> = files
            .iter()
            .map(|(path, _)| *path)
            .filter(|path| path.starts_with("overlay_modal"))
            .collect();
        assert_eq!(
            studio_files.len(),
            declared.len() + 1,
            "the guard file list must match the overlay_modal module tree"
        );
        for name in declared {
            let path = format!("overlay_modal/{name}.rs");
            assert!(
                studio_files.contains(&path.as_str()),
                "the guard file list is missing {path}"
            );
        }

        files
            .iter()
            .map(|(_, source)| source.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Guards the conversion: the English studio literals replaced by `Strings` fields must not
    /// come back into production code. The studio is a module tree, so the scan concatenates
    /// every production file of it (plus the shared colour picker, the preset write contract and
    /// the shared preset picker); each file contributes only the part before its `#[cfg(test)]`
    /// module, comment lines are dropped, and the needle list below is the one the single-file
    /// guard used.
    #[test]
    fn test_no_hardcoded_studio_english_literals_remain() {
        let production = overlay_module_source();

        let denied = [
            // field prefixes / units
            r#""W: ""#,
            r#""H: ""#,
            r#""X: ""#,
            r#""Y: ""#,
            r#""Screen X: ""#,
            r#""Screen Y: ""#,
            r#""Width: ""#,
            r#""Height: ""#,
            r#""Border: ""#,
            r#""Radius: ""#,
            r#""Size: ""#,
            r#""Thick: ""#,
            r#""Gap: ""#,
            r#""Min: ""#,
            r#""Max: ""#,
            r#"" px""#,
            r#"" pt""#,
            // colour chip labels
            r#""BG:""#,
            r#""BORDER:""#,
            r#""COLOR:""#,
            r#""TEXT COLOR:""#,
            r#""TEXT:""#,
            r#""FILL:""#,
            r#""LINE:""#,
            r#""ACTIVE:""#,
            r#""GLOW:""#,
            // palette swatch names
            r#""Cyan""#,
            r#""Green""#,
            r#""Red""#,
            r#""Amber""#,
            r#""White""#,
            r#""Dark""#,
            r#""Clear""#,
            // crosshair styles and clock panel
            r#""Cross""#,
            r#""T-Shape""#,
            r#""Circle""#,
            r#""Dot""#,
            r#""Font Size""#,
            r#""Bold""#,
            // control kind defaults
            r#""Action""#,
            r#""Volume""#,
            r#""Button""#,
            r#""Toggle""#,
            r#""Slider""#,
            // element badges
            r#""CROSSHAIR""#,
            r#""TEXT""#,
            r#""IMAGE""#,
            r#""SQUARE""#,
            r#""CIRCLE""#,
            r#""ARROW""#,
            r#""LINE""#,
            r#""GRAPH""#,
            r#""CLOCK""#,
            r#""CONTROL""#,
            r#""STATUS""#,
            r#""MEDIA""#,
            r#""KEYS""#,
            // presets, filter, on-click and canvas labels
            r#""Custom Overlay""#,
            r#""Preset name needs at least one letter or digit""#,
            r#""Could not save the preset; see the log""#,
            r#""Could not delete the preset; see the log""#,
            r#""Image""#,
            r#""DISPLAY {}×{}""#,
            r#""OVERLAY {}×{}""#,
            r#""RULE_CLICK_TOGGLE_LABEL""#,
        ];
        for literal in denied {
            assert!(
                !production.contains(literal),
                "hardcoded overlay-studio string {literal} must come from Strings"
            );
        }
    }
}
