use super::{OverlayAnchor, OverlayDefinition};

pub fn compute_overlay_screen_pos_with_anchor(anchor: OverlayAnchor, overlay: &OverlayDefinition, screen_w: f32, screen_h: f32) -> (f32, f32) {
    let w = overlay.rect.width;
    let h = overlay.rect.height;
    let mut rx = overlay.rect.x;
    let mut ry = overlay.rect.y;
    match anchor {
        OverlayAnchor::ScreenCenter => {
            if rx.abs() > screen_w * 0.35 { rx = 0.0; }
            if ry.abs() > screen_h * 0.35 { ry = 0.0; }
            ((screen_w - w) * 0.5 + rx, (screen_h - h) * 0.5 + ry)
        }
        OverlayAnchor::TopLeft => {
            if rx > screen_w * 0.4 { rx = 16.0; }
            if ry > screen_h * 0.4 { ry = 16.0; }
            (rx, ry)
        }
        OverlayAnchor::TopRight => {
            if rx > screen_w * 0.4 { rx = 16.0; }
            if ry > screen_h * 0.4 { ry = 16.0; }
            (screen_w - w - rx, ry)
        }
        OverlayAnchor::BottomLeft => {
            if rx > screen_w * 0.4 { rx = 16.0; }
            if ry > screen_h * 0.4 { ry = 16.0; }
            (rx, (screen_h - h - ry).max(0.0))
        }
        OverlayAnchor::BottomRight => {
            if rx > screen_w * 0.4 { rx = 16.0; }
            if ry > screen_h * 0.4 { ry = 16.0; }
            (screen_w - w - rx, (screen_h - h - ry).max(0.0))
        }
        OverlayAnchor::Custom => (rx, ry),
        OverlayAnchor::WindowRelative => (rx, ry),
    }
}

pub fn compute_overlay_screen_pos(overlay: &OverlayDefinition, screen_w: f32, screen_h: f32) -> (f32, f32) {
    compute_overlay_screen_pos_with_anchor(overlay.anchor, overlay, screen_w, screen_h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_overlay_screen_pos_for_anchors() {
        use crate::models::{OverlayAnchor, OverlayDefinition};

        // 1. Verify compute_overlay_screen_pos for different anchors
        let mut overlay = OverlayDefinition::default_macro_badge();
        overlay.rect.width = 400.0;
        overlay.rect.height = 200.0;
        overlay.rect.x = 20.0;
        overlay.rect.y = 30.0;

        // ScreenCenter: ((1920 - 400) / 2 + 20, (1080 - 200) / 2 + 30) = (780, 470)
        overlay.anchor = OverlayAnchor::ScreenCenter;
        let (pos_x, pos_y) = compute_overlay_screen_pos(&overlay, 1920.0, 1080.0);
        assert_eq!(pos_x, 780.0);
        assert_eq!(pos_y, 470.0);

        // TopLeft: (20, 30)
        overlay.anchor = OverlayAnchor::TopLeft;
        let (pos_x, pos_y) = compute_overlay_screen_pos(&overlay, 1920.0, 1080.0);
        assert_eq!(pos_x, 20.0);
        assert_eq!(pos_y, 30.0);

        // Custom: (20, 30)
        overlay.anchor = OverlayAnchor::Custom;
        let (pos_x, pos_y) = compute_overlay_screen_pos(&overlay, 1920.0, 1080.0);
        assert_eq!(pos_x, 20.0);
        assert_eq!(pos_y, 30.0);

        // TopRight: (1920 - 400 - 20, 30) = (1500, 30)
        overlay.anchor = OverlayAnchor::TopRight;
        let (pos_x, pos_y) = compute_overlay_screen_pos(&overlay, 1920.0, 1080.0);
        assert_eq!(pos_x, 1500.0);
        assert_eq!(pos_y, 30.0);
    }

    #[test]
    fn test_compute_overlay_screen_pos_sanitizes_stale_absolute_values() {
        use crate::models::{OverlayAnchor, OverlayDefinition};

        // 5. Verify anchor coordinate sanitization for stale absolute values
        let mut center_overlay = OverlayDefinition::default_macro_badge();
        center_overlay.anchor = OverlayAnchor::ScreenCenter;
        center_overlay.rect.x = 1200.0; // Stale absolute screen coordinate
        center_overlay.rect.y = 500.0;
        let (pos_x, pos_y) = compute_overlay_screen_pos(&center_overlay, 1920.0, 1080.0);
        // Stale 1200.0 (> 1920 * 0.35 = 672) is treated as 0.0 offset -> exactly centered!
        assert_eq!(pos_x, (1920.0 - center_overlay.rect.width) * 0.5);
        assert_eq!(pos_y, (1080.0 - center_overlay.rect.height) * 0.5);
    }
}
