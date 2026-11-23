//! Drag-and-drop geometry and painters shared by the four drag surfaces.
//!
//! Only the parts that are genuinely identical live here: the row-centre target-slot math, the
//! wheel and edge auto-scroll steps, the animated insertion line and the floating preview card
//! chrome. Each surface keeps its own session payload and identity rules — the rules list resolves
//! a drag by rule id and can be disabled by sort/filter, while the inspector pipelines, the overlay
//! element list and the macro timeline work by index — so no session type is shared.

use egui::{
    pos2, vec2, CornerRadius, Id, LayerId, Order, Painter, Pos2, Rect, Sense, Stroke, Ui,
};
use crate::gui::theme::{self, Tokens};

/// Target slot for a vertical row-centre drag, from the row centre ys collected this frame.
///
/// Before the first centre the slot is `0`; at or after the last centre it is `centers.len()`.
/// An empty list has no row to compare against and yields `0`; callers keep their existing
/// "only assign while non-empty" guard.
pub fn slot_from_pointer(centers: &[f32], pointer_y: f32) -> usize {
    let n = centers.len();
    if n == 0 {
        return 0;
    }
    if pointer_y < centers[0] {
        0
    } else if pointer_y >= centers[n - 1] {
        n
    } else {
        let mut slot = n;
        for (i, pair) in centers.windows(2).enumerate() {
            if pointer_y >= pair[0] && pointer_y < pair[1] {
                slot = i + 1;
                break;
            }
        }
        slot
    }
}

/// One drag-time wheel step: vertical wheel deltas scroll the list and schedule a repaint so the
/// drag keeps following the pointer. The 1.5 multiplier is the same on every surface.
pub fn wheel_scroll(ui: &mut Ui) {
    let wheel_delta_y = ui.input(|i| i.smooth_scroll_delta.y);
    if wheel_delta_y != 0.0 {
        ui.scroll_with_delta(vec2(0.0, wheel_delta_y * 1.5));
        ui.ctx().request_repaint();
    }
}

/// One drag-time edge auto-scroll step. `edge_zone` is the per-surface activation band: the
/// inspector pipeline uses 90 px, the overlay element list and the macro timeline use 70 px.
///
/// The rules list deliberately does not call this: it only routes the wheel while dragging.
pub fn edge_autoscroll(ui: &mut Ui, pointer: Pos2, edge_zone: f32) {
    let viewport = ui.clip_rect();
    let view_top = viewport.top();
    let view_bottom = viewport.bottom();

    // Near top edge: auto-scroll up
    if pointer.y < view_top + edge_zone {
        let factor = ((view_top + edge_zone - pointer.y) / edge_zone).clamp(0.1, 3.0);
        let scroll_speed = 8.0 + 26.0 * factor;
        ui.scroll_with_delta(vec2(0.0, scroll_speed));
        ui.ctx().request_repaint();
    }
    // Near bottom edge: auto-scroll down
    else if pointer.y > view_bottom - edge_zone {
        let factor = ((pointer.y - (view_bottom - edge_zone)) / edge_zone).clamp(0.1, 3.0);
        let scroll_speed = 8.0 + 26.0 * factor;
        ui.scroll_with_delta(vec2(0.0, -scroll_speed));
        ui.ctx().request_repaint();
    }
}

/// Left indicator bead of a drop line.
#[derive(Clone, Copy, Debug)]
pub struct DropBead {
    /// Distance from the slot's left edge.
    pub left: f32,
    pub radius: f32,
    /// Outer halo ring radius; `None` draws no ring.
    pub halo_radius: Option<f32>,
}

/// Per-surface geometry and colours of the animated drop-slot indicator.
///
/// The values stay at each call site: the inspector pipeline is 22 px tall with a step label, the
/// overlay element list is 20 px, and the rules list is 16 px with a zero corner radius.
#[derive(Clone, Copy, Debug)]
pub struct DropLineStyle {
    /// Slot height at full animation.
    pub height: f32,
    /// Animation time in seconds.
    pub anim_secs: f32,
    pub corner: CornerRadius,
    /// Accent background tint at full animation.
    pub fill_alpha: f32,
    pub line_width: f32,
    /// Insets of the line from the slot's left and right edges.
    pub line_left: f32,
    pub line_right: f32,
    pub bead: Option<DropBead>,
}

/// Paints one animated drop-slot indicator.
///
/// `anim_id` is the caller's exact animation id expression: slot identity stays per surface.
/// Returns the slot rect and animation value while the indicator is visible, so a caller that
/// paints a step label (the inspector pipeline) can draw it after the line, as before.
pub fn drop_line(
    ui: &mut Ui,
    anim_id: Id,
    targeted: bool,
    style: &DropLineStyle,
    t: &Tokens,
) -> Option<(Rect, f32)> {
    // A drop indicator is motion like any other: the platform may switch it off entirely.
    let anim =
        crate::gui::theme::motion::animate_bool(ui.ctx(), anim_id, targeted, style.anim_secs);
    if anim <= 0.001 {
        return None;
    }

    let slot_h = style.height * anim;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), slot_h), Sense::hover());
    let p = ui.painter();
    let mid_y = rect.center().y;

    // Background subtle accent tint
    p.rect_filled(
        rect,
        style.corner,
        t.accent_ink.gamma_multiply(style.fill_alpha * anim),
    );

    // Glowing laser line
    p.hline(
        rect.left() + style.line_left..=rect.right() - style.line_right,
        mid_y,
        Stroke::new(style.line_width * anim, t.accent_ink.gamma_multiply(anim)),
    );

    // Left indicator bead with an optional halo
    if let Some(bead) = style.bead {
        let bead_pos = pos2(rect.left() + bead.left, mid_y);
        p.circle_filled(bead_pos, bead.radius * anim, t.accent_ink.gamma_multiply(anim));
        if let Some(halo_radius) = bead.halo_radius {
            p.circle_stroke(
                bead_pos,
                halo_radius * anim,
                Stroke::new(1.0, t.accent_ink.gamma_multiply(0.4 * anim)),
            );
        }
    }

    Some((rect, anim))
}

/// Per-surface chrome of the floating drag preview card.
#[derive(Clone, Copy, Debug)]
pub struct FloatingPreviewStyle {
    pub corner: CornerRadius,
    pub stroke_width: f32,
    /// True for the inspector's elevated card; the overlay and rules previews are flat.
    pub shadow: bool,
}

/// Paints the floating drag preview chrome on the tooltip layer and lets `paint` draw its content.
///
/// `layer_id` is the caller's exact layer id expression, so each surface keeps its own preview
/// layer identity. The closure runs after the fill and stroke, exactly where the content used to
/// be painted.
pub fn floating_preview(
    ui: &Ui,
    layer_id: Id,
    rect: Rect,
    style: &FloatingPreviewStyle,
    t: &Tokens,
    paint: impl FnOnce(&Painter),
) {
    let painter = ui.ctx().layer_painter(LayerId::new(Order::Tooltip, layer_id));

    if style.shadow {
        painter.add(theme::shadow(t).as_shape(rect, style.corner));
    }

    painter.rect_filled(rect, style.corner, t.bg);
    painter.rect_stroke(
        rect,
        style.corner,
        Stroke::new(style.stroke_width, t.accent_ink),
        egui::StrokeKind::Inside,
    );

    paint(&painter);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slot_from_pointer_keeps_row_centre_boundaries() {
        let centers = [10.0, 30.0, 50.0];

        // Before the first centre and after the last one.
        assert_eq!(slot_from_pointer(&centers, 9.9), 0);
        assert_eq!(slot_from_pointer(&centers, 50.0), 3);
        assert_eq!(slot_from_pointer(&centers, 80.0), 3);

        // The boundary belongs to the slot below the earlier row's centre.
        assert_eq!(slot_from_pointer(&centers, 10.0), 1);
        assert_eq!(slot_from_pointer(&centers, 29.9), 1);
        assert_eq!(slot_from_pointer(&centers, 30.0), 2);
        assert_eq!(slot_from_pointer(&centers, 49.9), 2);

        // A single row has only the before-first and after-last slots.
        assert_eq!(slot_from_pointer(&[20.0], 19.9), 0);
        assert_eq!(slot_from_pointer(&[20.0], 20.0), 1);

        // An empty list has no rows to compare against.
        assert_eq!(slot_from_pointer(&[], 42.0), 0);
    }
}
