//! Overlay studio geometry: rotation helpers, crosshair coordinate bounds, the card/element
//! clamp and the monitor-dependent card projection helpers.
//!
//! Everything here is a pure value-in/value-out helper over the overlay model; the studio view
//! keeps the `Ui` rendering.

use egui::{vec2, Pos2, Rect};

use super::get_element_size;
use crate::models::{OverlayAnchor, OverlayDefinition, OverlayElement};

pub(super) fn rotate_vec(v: egui::Vec2, angle_rad: f32) -> egui::Vec2 {
    let cos = angle_rad.cos();
    let sin = angle_rad.sin();
    vec2(v.x * cos - v.y * sin, v.x * sin + v.y * cos)
}

pub(super) fn rotate_point(p: egui::Pos2, center: egui::Pos2, angle_rad: f32) -> egui::Pos2 {
    let offset = p - center;
    center + rotate_vec(offset, angle_rad)
}

pub(super) fn get_rotated_aabb(center: egui::Pos2, half_w: f32, half_h: f32, angle_rad: f32) -> egui::Rect {
    let cos = angle_rad.cos().abs();
    let sin = angle_rad.sin().abs();
    let bounding_half_w = half_w * cos + half_h * sin;
    let bounding_half_h = half_w * sin + half_h * cos;
    Rect::from_center_size(center, vec2(bounding_half_w * 2.0, bounding_half_h * 2.0))
}

/// Crosshair `x`/`y` use `0` — and anything negative — as the sentinel for "centre of the
/// card": the renderer and the canvas hit-test both branch on `> 0.0`. Every editing path must
/// therefore stay at 1 px or more, or the crosshair teleports to the card centre (U02-004).
pub(super) const CROSSHAIR_MIN_COORD: f32 = 1.0;

/// Bounds an edited crosshair coordinate may take on one axis.
pub(super) fn crosshair_coord_range(dimension: f32) -> std::ops::RangeInclusive<f32> {
    CROSSHAIR_MIN_COORD..=dimension.max(CROSSHAIR_MIN_COORD)
}

/// Clamps one crosshair coordinate into `crosshair_coord_range`.
pub(super) fn clamp_crosshair_coord(value: f32, dimension: f32) -> f32 {
    let range = crosshair_coord_range(dimension);
    value.clamp(*range.start(), *range.end())
}

/// Mirrors `get_element_size` for Text so the clamp and the canvas hit-test agree on the box.
pub(super) fn text_size_estimate(text: &str, font_size: f32) -> (f32, f32) {
    let char_len = (text.chars().count() as f32).max(1.0);
    let est_w = (char_len * (font_size * 0.6).max(8.0) + 12.0).max(30.0);
    let est_h = (font_size + 6.0).max(18.0);
    (est_w, est_h)
}

/// Card/element geometry that [`clamp_elements_to_canvas`] actually operates on.
///
/// The clamp rewrites element sizes and positions, and the studio persists those, so it must
/// run only when this geometry changed. Colours, labels and font sizes are deliberately not
/// part of the signature: the old unconditional per-frame call used to shrink a long Text
/// label's font permanently (U02-003).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct OverlayGeometry {
    card_w: f32,
    card_h: f32,
    card_x: f32,
    card_y: f32,
    elements: Vec<[f32; 4]>,
}

pub(super) fn overlay_geometry(overlay: &OverlayDefinition) -> OverlayGeometry {
    OverlayGeometry {
        card_w: overlay.rect.width,
        card_h: overlay.rect.height,
        card_x: overlay.rect.x,
        card_y: overlay.rect.y,
        elements: overlay
            .elements
            .iter()
            .map(|elem| {
                let (w, h) = get_element_size(elem);
                [elem.x(), elem.y(), w, h]
            })
            .collect(),
    }
}

pub fn clamp_elements_to_canvas(overlay: &mut OverlayDefinition) {
    let canvas_w = overlay.rect.width.max(20.0);
    let canvas_h = overlay.rect.height.max(20.0);

    for elem in &mut overlay.elements {
        match elem {
            OverlayElement::Image { width, height, lock_aspect, x, y, .. } => {
                if *lock_aspect {
                    let w = *width;
                    let h = (*height).max(1.0);
                    if w > canvas_w || h > canvas_h {
                        let scale = (canvas_w / w).min(canvas_h / h);
                        *width = (w * scale).round().clamp(8.0, canvas_w);
                        *height = (h * scale).round().clamp(8.0, canvas_h);
                    } else {
                        *width = w.clamp(8.0, canvas_w);
                        *height = h.clamp(8.0, canvas_h);
                    }
                } else {
                    *width = width.clamp(8.0, canvas_w);
                    *height = height.clamp(8.0, canvas_h);
                }
                let max_x = (canvas_w - *width).max(0.0);
                let max_y = (canvas_h - *height).max(0.0);
                *x = x.clamp(0.0, max_x);
                *y = y.clamp(0.0, max_y);
            }
            OverlayElement::Shape { width, height, lock_aspect, x, y, .. } => {
                if *lock_aspect {
                    let w = *width;
                    let h = (*height).max(1.0);
                    if w > canvas_w || h > canvas_h {
                        let scale = (canvas_w / w).min(canvas_h / h);
                        *width = (w * scale).round().clamp(4.0, canvas_w);
                        *height = (h * scale).round().clamp(4.0, canvas_h);
                    } else {
                        *width = w.clamp(4.0, canvas_w);
                        *height = h.clamp(4.0, canvas_h);
                    }
                } else {
                    *width = width.clamp(4.0, canvas_w);
                    *height = height.clamp(4.0, canvas_h);
                }
                let max_x = (canvas_w - *width).max(0.0);
                let max_y = (canvas_h - *height).max(0.0);
                *x = x.clamp(0.0, max_x);
                *y = y.clamp(0.0, max_y);
            }
            OverlayElement::Crosshair { size, gap, x, y, .. } => {
                let max_dim = canvas_w.min(canvas_h);
                if (*size + *gap) * 2.0 > max_dim {
                    *size = ((max_dim / 2.0) - *gap).clamp(2.0, 128.0);
                }
                *x = clamp_crosshair_coord(*x, canvas_w);
                *y = clamp_crosshair_coord(*y, canvas_h);
            }
            OverlayElement::Text { font_size, text, x, y, .. } => {
                // `font_size` is a user choice and is never rewritten here: the old arm only
                // lowered it, so a long label shrank permanently, deleting characters never
                // restored it, and the shrunken value was persisted on close (U02-003).
                // Overflowing text is clipped by the card painter instead.
                let (est_w, est_h) = text_size_estimate(text, *font_size);
                let max_x = (canvas_w - est_w).max(0.0);
                let max_y = (canvas_h - est_h).max(0.0);
                *x = x.clamp(0.0, max_x);
                *y = y.clamp(0.0, max_y);
            }
            OverlayElement::Graph { width, height, x, y, .. }
            | OverlayElement::ClockTimer { width, height, x, y, .. }
            | OverlayElement::InteractiveControl { width, height, x, y, .. }
            | OverlayElement::SystemTelemetry { width, height, x, y, .. }
            | OverlayElement::MediaWidget { width, height, x, y, .. }
            | OverlayElement::KeyVisualizer { width, height, x, y, .. } => {
                *width = width.clamp(10.0, canvas_w);
                *height = height.clamp(4.0, canvas_h);
                let max_x = (canvas_w - *width).max(0.0);
                let max_y = (canvas_h - *height).max(0.0);
                *x = x.clamp(0.0, max_x);
                *y = y.clamp(0.0, max_y);
            }
        }
    }
}

pub fn resolve_overlay_monitor(
    overlay: &OverlayDefinition,
    monitors: &[crate::models::MonitorInfo],
) -> crate::models::MonitorInfo {
    overlay
        .monitor_index
        .and_then(|idx| monitors.get(idx))
        .or_else(|| monitors.iter().find(|m| m.is_primary))
        .or_else(|| monitors.first())
        .cloned()
        .unwrap_or_else(|| crate::models::MonitorInfo {
            index: 0,
            name: "Primary Display".to_string(),
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            is_primary: true,
        })
}

pub fn max_card_dimensions(overlay: &OverlayDefinition, screen_w: f32, screen_h: f32) -> (f32, f32) {
    let rx = overlay.rect.x.max(0.0);
    let ry = overlay.rect.y.max(0.0);
    match overlay.anchor {
        OverlayAnchor::ScreenCenter => {
            let mw = (screen_w - rx * 2.0).clamp(20.0, screen_w);
            let mh = (screen_h - ry * 2.0).clamp(20.0, screen_h);
            (mw, mh)
        }
        OverlayAnchor::TopLeft | OverlayAnchor::TopRight | OverlayAnchor::BottomLeft | OverlayAnchor::BottomRight => {
            ((screen_w - rx).clamp(20.0, screen_w), (screen_h - ry).clamp(20.0, screen_h))
        }
        OverlayAnchor::Custom => {
            ((screen_w - overlay.rect.x).clamp(20.0, screen_w), (screen_h - overlay.rect.y).clamp(20.0, screen_h))
        }
        OverlayAnchor::WindowRelative => (screen_w, screen_h),
    }
}

pub fn clamp_card_to_monitor(overlay: &mut OverlayDefinition, screen_w: f32, screen_h: f32) {
    let (max_w, max_h) = max_card_dimensions(overlay, screen_w, screen_h);
    overlay.rect.width = overlay.rect.width.clamp(20.0, max_w);
    overlay.rect.height = overlay.rect.height.clamp(20.0, max_h);
    if overlay.anchor == OverlayAnchor::Custom {
        overlay.rect.x = overlay.rect.x.clamp(0.0, (screen_w - overlay.rect.width).max(0.0));
        overlay.rect.y = overlay.rect.y.clamp(0.0, (screen_h - overlay.rect.height).max(0.0));
    }
}

/// Canvas-space centre of one element. Crosshair `x`/`y` are absolute offsets from the card
/// corner and treat 0 as the "centre of the card" sentinel; every other kind is laid out from
/// its top-left corner. Shared by the hit-test, the selection handles and the selection
/// highlight so the copies cannot drift apart (U02-013).
pub(super) fn element_center_on_canvas(card_canvas_rect: Rect, elem: &OverlayElement, zoom: f32) -> Pos2 {
    match elem {
        OverlayElement::Crosshair { x, y, .. } => {
            if *x > 0.0 && *y > 0.0 {
                card_canvas_rect.min + vec2(*x * zoom, *y * zoom)
            } else {
                card_canvas_rect.center()
            }
        }
        _ => {
            let (raw_w, raw_h) = get_element_size(elem);
            card_canvas_rect.min + vec2((elem.x() + raw_w * 0.5) * zoom, (elem.y() + raw_h * 0.5) * zoom)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::components::text::fill;
    use crate::gui::state::GuiState;
    use egui::{pos2, Vec2};

    #[test]
    fn test_canvas_element_clamping_and_image_fit() {
        use crate::models::{OverlayDefinition, OverlayElement};

        let mut overlay = OverlayDefinition::default_crosshair();
        overlay.rect.width = 100.0;
        overlay.rect.height = 100.0;

        // 1. Proportional Image (lock_aspect: true)
        overlay.elements.push(OverlayElement::Image {
            id: "img_proportional".to_string(),
            path: "".to_string(),
            x: 50.0,
            y: 50.0,
            width: 200.0,
            height: 100.0, // 2:1 aspect ratio
            opacity: 1.0,
            lock_aspect: true,
            rotation: 0.0,
            on_click: None,
        });

        // 2. Freeform Image (lock_aspect: false)
        overlay.elements.push(OverlayElement::Image {
            id: "img_freeform".to_string(),
            path: "".to_string(),
            x: 80.0,
            y: 80.0,
            width: 150.0,
            height: 250.0,
            opacity: 1.0,
            lock_aspect: false,
            rotation: 0.0,
            on_click: None,
        });

        // 3. Interactive control overflowing canvas
        overlay.elements.push(OverlayElement::InteractiveControl {
            id: "ctrl_1".to_string(),
            control: crate::models::InteractiveControlKind::default(),
            color: [255, 255, 255, 255],
            bg_color: [0, 0, 0, 255],
            active_color: [0, 255, 0, 255],
            x: 90.0,
            y: 90.0,
            width: 300.0,
            height: 50.0,
            rotation: 0.0,
        });

        // 4. Text with large font overflowing canvas
        overlay.elements.push(OverlayElement::Text {
            id: "txt_large".to_string(),
            text: "FPS: 144".to_string(),
            font_size: 40.0,
            color: [255, 255, 255, 255],
            x: 40.0,
            y: 40.0,
            width: None,
            height: None,
            lock_aspect: true,
            bold: false,
            font_family: "Segoe UI".to_string(),
            rotation: 0.0,
            on_click: None,
        });

        // 5. Standard Text scaling
        overlay.elements.push(OverlayElement::Text {
            id: "txt_scaled".to_string(),
            text: "Title".to_string(),
            font_size: 20.0,
            color: [255, 255, 255, 255],
            x: 60.0,
            y: 60.0,
            width: None,
            height: None,
            lock_aspect: false,
            bold: true,
            font_family: "Segoe UI".to_string(),
            rotation: 0.0,
            on_click: None,
        });

        // 6. Proportional Shape with locked aspect ratio
        overlay.elements.push(OverlayElement::Shape {
            id: "shape_clamp".to_string(),
            shape_type: crate::models::ShapeKind::Rectangle,
            x: 80.0,
            y: 80.0,
            width: 200.0,
            height: 100.0, // 2:1 aspect ratio
            thickness: 2.0,
            color: [255, 255, 255, 255],
            filled: false,
            fill_color: [0, 0, 0, 0],
            corner_radius: 0.0,
            lock_aspect: true,
            rotation: 0.0,
            on_click: None,
        });

        clamp_elements_to_canvas(&mut overlay);

        // Verify Image 1: proportional shrink (width <= 100, height <= 50, x + width <= 100, y + height <= 100)
        if let OverlayElement::Image { width, height, x, y, .. } = &overlay.elements[1] {
            assert!(*width <= 100.0);
            assert!(*height <= 50.0);
            assert!(*x >= 0.0 && *x + *width <= 100.0);
            assert!(*y >= 0.0 && *y + *height <= 100.0);
        } else {
            panic!("Expected Image element");
        }

        // Verify Image 2: freeform shrink (width <= 100, height <= 100)
        if let OverlayElement::Image { width, height, x, y, .. } = &overlay.elements[2] {
            assert!(*width <= 100.0);
            assert!(*height <= 100.0);
            assert!(*x >= 0.0 && *x + *width <= 100.0);
            assert!(*y >= 0.0 && *y + *height <= 100.0);
        } else {
            panic!("Expected Image element");
        }

        // Verify InteractiveControl: width <= 100, height <= 100, coordinates clamped
        if let OverlayElement::InteractiveControl { width, height, x, y, .. } = &overlay.elements[3] {
            assert!(*width <= 100.0);
            assert!(*height <= 100.0);
            assert!(*x >= 0.0 && *x + *width <= 100.0);
            assert!(*y >= 0.0 && *y + *height <= 100.0);
        } else {
            panic!("Expected InteractiveControl element");
        }

        // Verify Text: the clamp never rewrites font_size (U02-003). An oversized label is moved
        // to the card origin and clipped by the card painter at render time instead.
        if let OverlayElement::Text { font_size, x, y, .. } = &overlay.elements[4] {
            assert_eq!(*font_size, 40.0, "the clamp must not shrink the user's font");
            assert_eq!(*x, 0.0);
            let (_, eh) = text_size_estimate("FPS: 144", *font_size);
            assert!(*y >= 0.0 && *y + eh <= 100.0);
        } else {
            panic!("Expected Text element");
        }

        // Verify Standard Text 5: clamped to canvas
        if let OverlayElement::Text { font_size, x, y, text, .. } = &overlay.elements[5] {
            let (ew, eh) = get_element_size(&OverlayElement::Text {
                id: "".to_string(),
                text: text.clone(),
                font_size: *font_size,
                color: [0; 4],
                x: 0.0,
                y: 0.0,
                width: None,
                height: None,
                lock_aspect: false,
                bold: false,
                font_family: "".to_string(),
                rotation: 0.0,
                on_click: None,
            });
            assert!(*x >= 0.0 && *x + ew <= 100.0);
            assert!(*y >= 0.0 && *y + eh <= 100.0);
        } else {
            panic!("Expected Scaled Text element");
        }

        // Verify Proportional Shape 6: clamped to canvas with preserved aspect ratio (width <= 100, height <= 50)
        if let OverlayElement::Shape { width, height, x, y, .. } = &overlay.elements[6] {
            assert!(*width <= 100.0);
            assert!(*height <= 50.0);
            assert!(*x >= 0.0 && *x + *width <= 100.0);
            assert!(*y >= 0.0 && *y + *height <= 100.0);
        } else {
            panic!("Expected Shape element");
        }
    }

    #[test]
    fn test_monitor_artboard_and_card_aspect_ratio_lock() {
        use crate::models::{OverlayAnchor, OverlayDefinition};

        let mut overlay = OverlayDefinition::default_macro_badge();
        overlay.rect.width = 400.0;
        overlay.rect.height = 200.0;

        // 2. Verify Card Aspect Ratio Lock calculation
        let card_lock_aspect = true;
        let aspect = overlay.rect.width / overlay.rect.height; // 400 / 200 = 2.0
        assert_eq!(aspect, 2.0);

        // Resizing width from 400 to 600 maintains aspect ratio -> height = 300
        let new_w = 600.0;
        let new_h = if card_lock_aspect {
            (new_w / aspect).round()
        } else {
            overlay.rect.height
        };
        assert_eq!(new_h, 300.0);

        // 3. Verify OverlayStudioState default camera pan & zoom, centered default card
        let mut state = GuiState::new();
        state.open_overlay_studio("overlay_custom");
        let studio = state.overlay_studio.as_ref().unwrap();
        assert!(!studio.card_lock_aspect);
        assert_eq!(studio.canvas_pan, egui::Vec2::ZERO);
        assert_eq!(studio.canvas_zoom, None);
        assert_eq!(studio.overlay.anchor, OverlayAnchor::ScreenCenter);
        assert_eq!(studio.overlay.rect.width, 440.0);
        assert_eq!(studio.overlay.rect.height, 200.0);

        // Also verify preset macro badge retains its dedicated top-right geometry
        state.open_overlay_studio("overlay_macro_badge");
        let studio_badge = state.overlay_studio.as_ref().unwrap();
        assert_eq!(studio_badge.overlay.anchor, OverlayAnchor::TopRight);
        assert_eq!(studio_badge.overlay.rect.width, 220.0);
        assert_eq!(studio_badge.overlay.rect.height, 44.0);

        // 7. Verify 4-way auto-panning calculation logic
        let canvas_viewport = Rect::from_min_size(pos2(100.0, 100.0), vec2(800.0, 600.0));
        let edge_zone = 70.0;

        // Pointer near right edge -> pan step X is negative (slides camera right)
        let ptr_right = pos2(canvas_viewport.right() - 20.0, canvas_viewport.center().y);
        let factor_r = ((ptr_right.x - (canvas_viewport.right() - edge_zone)) / edge_zone).clamp(0.0, 1.5);
        let pan_x = -(3.0 + 8.0 * factor_r);
        assert!(pan_x < -3.0);

        // Pointer near left edge -> pan step X is positive (slides camera left)
        let ptr_left = pos2(canvas_viewport.left() + 20.0, canvas_viewport.center().y);
        let factor_l = ((canvas_viewport.left() + edge_zone - ptr_left.x) / edge_zone).clamp(0.0, 1.5);
        let pan_l = 3.0 + 8.0 * factor_l;
        assert!(pan_l > 3.0);

        // Pointer near bottom edge -> pan step Y is negative (slides camera down)
        let ptr_bottom = pos2(canvas_viewport.center().x, canvas_viewport.bottom() - 15.0);
        let factor_b = ((ptr_bottom.y - (canvas_viewport.bottom() - edge_zone)) / edge_zone).clamp(0.0, 1.5);
        let pan_y = -(3.0 + 8.0 * factor_b);
        assert!(pan_y < -3.0);

        // Pointer near top edge -> pan step Y is positive (slides camera up)
        let ptr_top = pos2(canvas_viewport.center().x, canvas_viewport.top() + 15.0);
        let factor_t = ((canvas_viewport.top() + edge_zone - ptr_top.y) / edge_zone).clamp(0.0, 1.5);
        let pan_t = 3.0 + 8.0 * factor_t;
        assert!(pan_t > 3.0);

        // Safeguard: when an element is selected (selected_element_idx is Some), camera auto-pan is disabled
        let selected_element_idx = Some(0usize);
        let pan_step = if selected_element_idx.is_none() {
            vec2(pan_x, pan_y)
        } else {
            Vec2::ZERO
        };
        assert_eq!(pan_step, Vec2::ZERO);

        // 4. Verify max_card_dimensions and clamp_card_to_monitor
        let mut custom_overlay = OverlayDefinition::default_macro_badge();
        custom_overlay.anchor = OverlayAnchor::Custom;
        custom_overlay.rect.x = 1500.0;
        custom_overlay.rect.y = 800.0;
        custom_overlay.rect.width = 600.0;
        custom_overlay.rect.height = 400.0;
        let (max_w, max_h) = max_card_dimensions(&custom_overlay, 1920.0, 1080.0);
        assert_eq!(max_w, 420.0); // 1920 - 1500
        assert_eq!(max_h, 280.0); // 1080 - 800
        clamp_card_to_monitor(&mut custom_overlay, 1920.0, 1080.0);
        assert_eq!(custom_overlay.rect.width, 420.0);
        assert_eq!(custom_overlay.rect.height, 280.0);

        // 6. Verify calibration synchronization matching: when last_synced_pos matches, no overwrite occurs
        let mut studio = state.overlay_studio.unwrap();
        studio.is_calibrating = true;
        studio.overlay.rect.x = 750.0;
        studio.overlay.rect.y = 350.0;
        studio.last_synced_pos = Some((750.0, 350.0));
        let daemon_pos = (750.0, 350.0);
        let pos_differs_from_last_synced = studio.last_synced_pos.is_none_or(|(lx, ly)| (lx - daemon_pos.0).abs() > 0.5 || (ly - daemon_pos.1).abs() > 0.5);
        assert!(!pos_differs_from_last_synced, "Position matching last_synced_pos must not trigger overwrite");
    }

    #[test]
    fn test_multimonitor_canvas_support_and_card_clamping() {
        use crate::models::{MonitorInfo, MonitorOrientation, OverlayDefinition};

        // Create a 2-monitor virtual desktop setup:
        // Monitor 0: Primary Landscape 2560x1440 at (0, 0)
        // Monitor 1: Secondary Portrait 1080x1920 at (-1080, -383)
        let monitors = vec![
            MonitorInfo {
                index: 0,
                name: "Primary Display".to_string(),
                x: 0,
                y: 0,
                width: 2560,
                height: 1440,
                is_primary: true,
            },
            MonitorInfo {
                index: 1,
                name: "Portrait Display".to_string(),
                x: -1080,
                y: -383,
                width: 1080,
                height: 1920,
                is_primary: false,
            },
        ];

        // 1. Verify monitor orientation detection
        assert_eq!(monitors[0].orientation(), MonitorOrientation::Landscape);
        assert!(!monitors[0].is_portrait());
        assert_eq!(monitors[1].orientation(), MonitorOrientation::Portrait);
        assert!(monitors[1].is_portrait());

        // 2. Verify resolve_overlay_monitor
        let mut overlay = OverlayDefinition::default_new_overlay();
        assert_eq!(overlay.monitor_index, None);
        let resolved = resolve_overlay_monitor(&overlay, &monitors);
        assert_eq!(resolved.index, 0);
        assert!(resolved.is_primary);

        // Switch to secondary monitor (index 1)
        overlay.monitor_index = Some(1);
        let resolved_sec = resolve_overlay_monitor(&overlay, &monitors);
        assert_eq!(resolved_sec.index, 1);
        assert_eq!(resolved_sec.width, 1080);
        assert_eq!(resolved_sec.height, 1920);

        // Invalid index falls back to primary
        overlay.monitor_index = Some(99);
        let resolved_fallback = resolve_overlay_monitor(&overlay, &monitors);
        assert_eq!(resolved_fallback.index, 0);

        // 3. Verify Virtual Desktop Bounding Box calculation
        let min_x = monitors.iter().map(|m| m.x).min().unwrap_or(0);
        let min_y = monitors.iter().map(|m| m.y).min().unwrap_or(0);
        let max_x = monitors.iter().map(|m| m.x + m.width as i32).max().unwrap_or(1920);
        let max_y = monitors.iter().map(|m| m.y + m.height as i32).max().unwrap_or(1080);
        assert_eq!(min_x, -1080);
        assert_eq!(min_y, -383);
        assert_eq!(max_x, 2560);
        assert_eq!(max_y, 1537); // -383 + 1920 = 1537

        let desktop_w = (max_x - min_x) as f32; // 3640
        let desktop_h = (max_y - min_y) as f32; // 1920
        assert_eq!(desktop_w, 3640.0);
        assert_eq!(desktop_h, 1920.0);

        // 4. Verify strict clamping within active monitor (preventing crossing to other monitors during drag)
        // Set overlay on Monitor 1 (Portrait 1080x1920)
        overlay.monitor_index = Some(1);
        overlay.rect.width = 400.0;
        overlay.rect.height = 300.0;
        let mon1 = &monitors[1];

        // Attempt to drag card beyond right edge of portrait monitor (e.g. 1200.0 px)
        let dragged_rel_x = 1200.0f32;
        let clamped_rel_x = dragged_rel_x.clamp(0.0, (mon1.width as f32 - overlay.rect.width).max(0.0));
        assert_eq!(clamped_rel_x, 680.0); // 1080 - 400 = 680

        // Attempt to drag card below bottom edge of portrait monitor (e.g. 2500.0 px)
        let dragged_rel_y = 2500.0f32;
        let clamped_rel_y = dragged_rel_y.clamp(0.0, (mon1.height as f32 - overlay.rect.height).max(0.0));
        assert_eq!(clamped_rel_y, 1620.0); // 1920 - 300 = 1620

        // Attempt to drag card into negative coordinates (crossing left or top)
        let dragged_neg_x = -150.0f32;
        let clamped_neg_x = dragged_neg_x.clamp(0.0, (mon1.width as f32 - overlay.rect.width).max(0.0));
        assert_eq!(clamped_neg_x, 0.0);

        // Verify card cannot cross monitor boundaries
        assert!(clamped_rel_x + overlay.rect.width <= mon1.width as f32);
        assert!(clamped_rel_y + overlay.rect.height <= mon1.height as f32);
        assert!(clamped_neg_x >= 0.0);

        // 5. Verify Canvas Display Header badge label formatting (catalog value + fill helper)
        let loc = crate::i18n::get(crate::i18n::Language::English);
        let mon0_label = fill(
            loc.overlay_studio_canvas_display,
            &[&monitors[0].width.to_string(), &monitors[0].height.to_string()],
        );
        assert_eq!(mon0_label, "DISPLAY 2560×1440");
        let mon1_label = fill(
            loc.overlay_studio_canvas_display,
            &[&monitors[1].width.to_string(), &monitors[1].height.to_string()],
        );
        assert_eq!(mon1_label, "DISPLAY 1080×1920");

        // 6. Verify Overlay Header Pill text formatting (no glyph, no brackets)
        let pill_text = fill(
            loc.overlay_studio_canvas_overlay,
            &[
                &(overlay.rect.width as i32).to_string(),
                &(overlay.rect.height as i32).to_string(),
            ],
        );
        assert_eq!(pill_text, "OVERLAY 400×300");

        // 7. Verify Image & Card boundary clipping prevents bleeding into left settings panel
        let left_panel_rect = Rect::from_min_max(pos2(0.0, 50.0), pos2(400.0, 800.0));
        let canvas_rect = Rect::from_min_max(pos2(400.0, 50.0), pos2(1200.0, 800.0));
        // Panned card overlapping left panel (left edge at 200.0, crossing x=400.0)
        let panned_card = Rect::from_min_max(pos2(200.0, 100.0), pos2(700.0, 500.0));
        let effective_clip = panned_card.intersect(canvas_rect);
        assert_eq!(effective_clip.min.x, 400.0, "Clip rect must never extend left of canvas boundary");
        assert!(!effective_clip.intersects(left_panel_rect.shrink(0.1)), "Clip rect must not intersect left settings panel");
        assert!(effective_clip.is_positive());
    }

    // ---------------- U02-004: crosshair sentinel ----------------

    #[test]
    fn test_crosshair_coordinates_never_hit_the_centre_sentinel() {
        let range = crosshair_coord_range(100.0);
        assert_eq!(*range.start(), 1.0);
        assert_eq!(*range.end(), 100.0);
        assert_eq!(clamp_crosshair_coord(0.0, 100.0), 1.0);
        assert_eq!(clamp_crosshair_coord(-25.0, 100.0), 1.0);
        assert_eq!(clamp_crosshair_coord(42.0, 100.0), 42.0);
        assert_eq!(clamp_crosshair_coord(500.0, 100.0), 100.0);

        // A legacy overlay saved with the sentinel / a negative value is clamped out of it.
        let mut overlay = OverlayDefinition::default_crosshair();
        overlay.rect.width = 120.0;
        overlay.rect.height = 90.0;
        if let OverlayElement::Crosshair { x, y, .. } = &mut overlay.elements[0] {
            *x = 0.0;
            *y = -12.0;
        }
        clamp_elements_to_canvas(&mut overlay);
        if let OverlayElement::Crosshair { x, y, .. } = &overlay.elements[0] {
            assert!(*x >= 1.0);
            assert!(*y >= 1.0);
        }
    }

    // ---------------- U02-003: no per-frame clamping ----------------

    #[test]
    fn test_text_font_size_is_not_clamped_by_preview() {
        let mut overlay = OverlayDefinition::default_crosshair();
        overlay.rect.width = 80.0;
        overlay.rect.height = 40.0;
        overlay.elements = vec![OverlayElement::Text {
            id: "label".to_string(),
            text: "A very long label that cannot possibly fit".to_string(),
            font_size: 48.0,
            color: [255, 255, 255, 255],
            x: 0.0,
            y: 0.0,
            width: None,
            height: None,
            lock_aspect: true,
            bold: false,
            font_family: "Segoe UI".to_string(),
            rotation: 0.0,
            on_click: None,
        }];
        clamp_elements_to_canvas(&mut overlay);
        if let OverlayElement::Text { font_size, .. } = &overlay.elements[0] {
            assert_eq!(*font_size, 48.0, "clamping must never rewrite the user's font size");
        } else {
            panic!("expected a Text element");
        }

        // The clamp is idempotent once the geometry was applied.
        let mut once = overlay.clone();
        clamp_elements_to_canvas(&mut once);
        assert_eq!(overlay, once);
    }

    #[test]
    fn test_overlay_geometry_ignores_presentation_only_edits() {
        let overlay = OverlayDefinition::default_crosshair();
        let geometry = overlay_geometry(&overlay);

        let mut recoloured = overlay.clone();
        recoloured.style.bg_color = [1, 2, 3, 4];
        recoloured.opacity = 0.4;
        assert_eq!(
            overlay_geometry(&recoloured),
            geometry,
            "colour/opacity edits must not trigger the geometry clamp"
        );

        let mut moved = overlay.clone();
        *moved.elements[0].x_mut() = 32.0;
        assert_ne!(overlay_geometry(&moved), geometry, "a geometry edit must trigger the clamp");
    }
}
