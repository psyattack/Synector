//! Overlay studio calibration: the live on-screen position sync, the preview window toggle and
//! the real-time write-back of the studio overlay into the edited rule.

use crate::gui::state::{GuiState, OverlayStudioState};
use crate::models::{compute_overlay_screen_pos, OverlayAnchor, OverlayDefinition};

use super::clamp_elements_to_canvas;
use super::geometry::{overlay_geometry, OverlayGeometry};

/// Runs [`clamp_elements_to_canvas`] only when the card/element geometry it rewrites actually
/// changed. The old unconditional per-frame call let a long Text label shrink its own font
/// permanently (U02-003); the signature is cached per overlay id in egui memory.
pub(super) fn clamp_studio_geometry_if_changed(ctx: &egui::Context, overlay: &mut OverlayDefinition) {
    let geometry_id = egui::Id::unique(("overlay_studio_geometry", overlay.id.as_str()));
    let cached: Option<OverlayGeometry> = ctx.data(|d| d.get_temp(geometry_id));
    if cached.as_ref() == Some(&overlay_geometry(overlay)) {
        return;
    }
    clamp_elements_to_canvas(overlay);
    ctx.data_mut(|d| d.insert_temp(geometry_id, overlay_geometry(overlay)));
}

/// Synchronizes live on-screen calibration coordinates from the daemon. Polls only while the
/// pointer is idle so an in-progress drag is never overwritten, and never blocks the frame: the
/// request is queued and the answer is applied on a later frame. Returns `true` when the overlay
/// position changed.
pub(super) fn sync_calibration_position(
    ctx: &egui::Context,
    state: &mut GuiState,
    studio: &mut OverlayStudioState,
    active_mon: &crate::models::MonitorInfo,
) -> bool {
    if !studio.is_calibrating || ctx.input(|i| i.pointer.any_down()) {
        return false;
    }
    state.request_overlay_position(&studio.overlay.id);
    let Some((abs_x, abs_y)) = state.overlay_position_for(&studio.overlay.id) else {
        return false;
    };

    let rel_x = (abs_x - active_mon.x as f32).clamp(0.0, (active_mon.width as f32 - studio.overlay.rect.width).max(0.0));
    let rel_y = (abs_y - active_mon.y as f32).clamp(0.0, (active_mon.height as f32 - studio.overlay.rect.height).max(0.0));
    let pos_differs_from_current = (studio.overlay.rect.x - rel_x).abs() > 0.5 || (studio.overlay.rect.y - rel_y).abs() > 0.5;
    let pos_differs_from_last_synced = studio.last_synced_pos.is_none_or(|(lx, ly)| (lx - abs_x).abs() > 0.5 || (ly - abs_y).abs() > 0.5);
    if !(pos_differs_from_current && pos_differs_from_last_synced) {
        return false;
    }

    studio.overlay.anchor = OverlayAnchor::Custom;
    studio.overlay.rect.x = rel_x;
    studio.overlay.rect.y = rel_y;
    studio.last_synced_pos = Some((abs_x, abs_y));
    ctx.request_repaint();
    true
}

/// Shows or hides the live calibration window that mirrors the studio card on screen. The
/// preview requests are fire-and-forget: the studio's own copy renders immediately.
pub(super) fn apply_calibration_toggle(
    state: &mut GuiState,
    studio: &mut OverlayStudioState,
    active_mon: &crate::models::MonitorInfo,
) {
    studio.is_calibrating = !studio.is_calibrating;
    if studio.is_calibrating {
        let (csx, csy) = compute_overlay_screen_pos(&studio.overlay, active_mon.width as f32, active_mon.height as f32);
        let abs_x = active_mon.x as f32 + csx;
        let abs_y = active_mon.y as f32 + csy;
        studio.last_synced_pos = Some((abs_x, abs_y));
        state.request_show_overlay_preview(studio.overlay.clone(), true);
    } else {
        studio.last_synced_pos = None;
        state.request_hide_overlay_preview(&studio.overlay.id);
    }
}

/// Re-sends the edited overlay to the calibration window after a visible change.
pub(super) fn push_calibration_preview(
    state: &mut GuiState,
    studio: &mut OverlayStudioState,
    initial_overlay: &OverlayDefinition,
    active_mon: &crate::models::MonitorInfo,
) {
    if !studio.is_calibrating || &studio.overlay == initial_overlay {
        return;
    }
    let (csx, csy) = compute_overlay_screen_pos(&studio.overlay, active_mon.width as f32, active_mon.height as f32);
    let abs_x = active_mon.x as f32 + csx;
    let abs_y = active_mon.y as f32 + csy;
    studio.last_synced_pos = Some((abs_x, abs_y));
    state.request_show_overlay_preview(studio.overlay.clone(), true);
}

/// Real-time auto-save of the studio overlay into the rule action being edited.
pub(super) fn persist_studio_into_rule(state: &mut GuiState, studio: &OverlayStudioState) {
    let Some(target_idx) = studio.action_target_idx else { return };
    let Some(rule) = state.editing_rule.as_mut() else { return };
    let actions = if studio.is_exit_pipeline {
        &mut rule.on_exit_actions
    } else {
        &mut rule.actions
    };
    if let Some(crate::models::Action::Overlay { custom_overlay, overlay_id, .. }) = actions.get_mut(target_idx) {
        if custom_overlay.is_some() {
            *custom_overlay = Some(Box::new(studio.overlay.clone()));
        }
        *overlay_id = studio.overlay.id.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::OverlayElement;

    #[test]
    fn test_clamp_runs_only_when_geometry_changes() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ctx| {
            let mut overlay = OverlayDefinition::default_crosshair();
            overlay.rect.width = 200.0;
            overlay.rect.height = 200.0;
            overlay.elements.push(OverlayElement::Text {
                id: "label".to_string(),
                text: "Label".to_string(),
                font_size: 24.0,
                color: [255, 255, 255, 255],
                x: 50.0,
                y: 150.0,
                width: None,
                height: None,
                lock_aspect: true,
                bold: false,
                font_family: "Segoe UI".to_string(),
                rotation: 0.0,
                on_click: None,
            });
            clamp_studio_geometry_if_changed(ctx, &mut overlay);
            assert_eq!(overlay.elements[1].x(), 50.0, "the element already fits");

            // Shrinking the card is a real geometry change and must clamp.
            overlay.rect.width = 80.0;
            clamp_studio_geometry_if_changed(ctx, &mut overlay);
            assert!(overlay.elements[1].x() <= 80.0);

            // Typing more text is presentation-only: the font size survives the next frames.
            if let OverlayElement::Text { text, .. } = &mut overlay.elements[1] {
                text.push_str(" with a much longer caption");
            }
            clamp_studio_geometry_if_changed(ctx, &mut overlay);
            clamp_studio_geometry_if_changed(ctx, &mut overlay);
            if let OverlayElement::Text { font_size, .. } = &overlay.elements[1] {
                assert_eq!(*font_size, 24.0);
            }
        });
        out.textures_delta.clear();
    }
}
