//! Overlay studio colour controls: the compact colour chip and the element/card colour target
//! mapping used by the inline picker.

use egui::{vec2, Color32, CornerRadius, Rect, RichText, Sense, Stroke, Ui};

use crate::gui::state::ColorPickerTarget;
use crate::gui::theme::{self, Tokens};

pub(super) fn render_color_chip(
    ui: &mut Ui,
    label: &str,
    color: &mut [u8; 4],
    target: ColorPickerTarget,
    active_target: &mut Option<ColorPickerTarget>,
    t: &Tokens,
) {
    let is_active = *active_target == Some(target);
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).family(theme::semibold()).size(10.5).color(t.faint));

        // Compact rounded rectangle swatch (vec2(28.0, 15.0)), no large square, no hex text
        let swatch_size = vec2(28.0, 15.0);
        let id = ui.scope_id().with(("swatch", target, label));
        let (rect, _) = ui.allocate_exact_size(swatch_size, Sense::hover());
        let resp = ui.interact(rect, id, Sense::click());
        let p = ui.painter();

        // 1. Checkerboard pattern for alpha transparency
        let half_w = rect.width() / 2.0;
        let half_h = rect.height() / 2.0;
        p.rect_filled(rect, CornerRadius::same(3), Color32::from_gray(50));
        p.rect_filled(Rect::from_min_size(rect.min, vec2(half_w, half_h)), CornerRadius::ZERO, Color32::from_gray(90));
        p.rect_filled(Rect::from_min_size(rect.min + vec2(half_w, half_h), vec2(half_w, half_h)), CornerRadius::ZERO, Color32::from_gray(90));

        // 2. Color fill with alpha
        let col = Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]);
        p.rect_filled(rect, CornerRadius::same(3), col);

        // 3. Border stroke: accent ink if active, subtle hairline otherwise
        let stroke = if is_active {
            Stroke::new(1.5, t.accent_ink)
        } else if resp.hovered() {
            Stroke::new(1.0, Color32::WHITE)
        } else {
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 30))
        };
        p.rect_stroke(rect, CornerRadius::same(3), stroke, egui::StrokeKind::Inside);

        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            if is_active {
                *active_target = None;
            } else {
                *active_target = Some(target);
            }
        }
    });
}

// ----------------- OVERLAY INSPECTOR PANEL -----------------

/// Colour slots one element panel can edit. Card-scoped targets are deliberately absent: an
/// element colour chip that opened the card editor silently rewrote the overlay style (U02-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ElementColorSlot {
    Primary,
    Secondary,
    Tertiary,
}

/// Builds the element-scoped picker target for one colour slot.
pub(super) fn element_color_target(elem_idx: usize, slot: ElementColorSlot) -> ColorPickerTarget {
    match slot {
        ElementColorSlot::Primary => ColorPickerTarget::ElementPrimary(elem_idx),
        ElementColorSlot::Secondary => ColorPickerTarget::ElementSecondary(elem_idx),
        ElementColorSlot::Tertiary => ColorPickerTarget::ElementTertiary(elem_idx),
    }
}

/// Which colour slot of `elem_idx` the open inline picker belongs to. `None` also covers the
/// card-scoped targets, which only the card panel renders.
pub(super) fn active_element_slot(target: Option<ColorPickerTarget>, elem_idx: usize) -> Option<ElementColorSlot> {
    match target? {
        ColorPickerTarget::ElementPrimary(idx) if idx == elem_idx => Some(ElementColorSlot::Primary),
        ColorPickerTarget::ElementSecondary(idx) if idx == elem_idx => Some(ElementColorSlot::Secondary),
        ColorPickerTarget::ElementTertiary(idx) if idx == elem_idx => Some(ElementColorSlot::Tertiary),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------- U02-001: element colour targets ----------------

    #[test]
    fn test_element_colour_targets_are_element_scoped() {
        for elem_idx in [0usize, 4] {
            for slot in [ElementColorSlot::Primary, ElementColorSlot::Secondary, ElementColorSlot::Tertiary] {
                let target = element_color_target(elem_idx, slot);
                assert!(
                    !matches!(target, ColorPickerTarget::CardBg | ColorPickerTarget::CardBorder),
                    "an element colour chip must never target the card style"
                );
                assert_eq!(active_element_slot(Some(target), elem_idx), Some(slot));
                assert_eq!(active_element_slot(Some(target), elem_idx + 1), None);
            }
        }
        assert_eq!(active_element_slot(Some(ColorPickerTarget::CardBg), 0), None);
        assert_eq!(active_element_slot(Some(ColorPickerTarget::CardBorder), 0), None);
        assert_eq!(active_element_slot(None, 0), None);
    }
}
