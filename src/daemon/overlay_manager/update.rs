use crate::models::OverlayDefinition;

use super::supervisor::{flash_overlay, hide_overlay_sync, show_overlay};

pub fn show_update_notification(version: &str) {
    let mut overlay = OverlayDefinition::default_new_overlay();
    overlay.id = "overlay_update_notification".to_string();
    overlay.name = "Update Notification".to_string();
    overlay.anchor = crate::models::OverlayAnchor::BottomRight;
    overlay.rect = crate::models::OverlayRect {
        x: 24.0,
        y: 24.0,
        width: 355.0,
        height: 86.0,
    };
    overlay.fade_in_ms = Some(300);
    overlay.fade_out_ms = Some(400);
    overlay.auto_hide_seconds = Some(6.0);
    overlay.click_through = false;
    flash_overlay(&overlay, 6000, Some(version.to_string()));
}

pub fn show_update_progress_overlay(version: &str) {
    let mut overlay = OverlayDefinition::default_new_overlay();
    overlay.id = "overlay_update_progress".to_string();
    overlay.name = "Update Progress".to_string();
    overlay.anchor = crate::models::OverlayAnchor::BottomRight;
    overlay.rect = crate::models::OverlayRect {
        x: 24.0,
        y: 24.0,
        width: 355.0,
        height: 88.0,
    };
    overlay.fade_in_ms = Some(250);
    overlay.fade_out_ms = Some(350);
    overlay.auto_hide_seconds = None;
    overlay.click_through = true;
    show_overlay(&overlay, false, Some(version.to_string()));
}

pub fn hide_update_progress_overlay() {
    hide_overlay_sync("overlay_update_progress");
}
