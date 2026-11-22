use crate::models::{
    CrosshairStyle, OverlayAnchor, OverlayDefinition, OverlayElement, OverlayRect, OverlayStyle,
};

impl OverlayDefinition {
    pub fn default_new_overlay() -> Self {
        Self {
            id: "overlay_custom".to_string(),
            name: "Custom Overlay".to_string(),
            rect: OverlayRect { x: 0.0, y: 0.0, width: 440.0, height: 200.0 },
            anchor: OverlayAnchor::ScreenCenter,
            click_through: false,
            draggable: false,
            opacity: 0.95,
            style: OverlayStyle {
                bg_color: [14, 15, 20, 230],
                border_color: [80, 160, 255, 120],
                border_width: 1.0,
                corner_radius: 8.0,
                glow: true,
            },
            elements: vec![],
            auto_hide_seconds: None,
            target_process: None,
            hide_from_capture: false,
            fade_in_ms: None,
            fade_out_ms: None,
            monitor_index: None,
            close_with_target: false,
        }
    }

    pub fn default_crosshair() -> Self {
        Self {
            id: "overlay_crosshair".to_string(),
            name: "Gaming Crosshair".to_string(),
            rect: OverlayRect { x: 0.0, y: 0.0, width: 48.0, height: 48.0 },
            anchor: OverlayAnchor::ScreenCenter,
            click_through: true,
            draggable: false,
            opacity: 1.0,
            style: OverlayStyle {
                bg_color: [0, 0, 0, 0],
                border_color: [0, 0, 0, 0],
                border_width: 0.0,
                corner_radius: 0.0,
                glow: false,
            },
            elements: vec![
                OverlayElement::Crosshair {
                    id: "cross_1".to_string(),
                    style: CrosshairStyle::Cross,
                    size: 14.0,
                    color: [0, 255, 136, 255],
                    center_dot: true,
                    thickness: 2.0,
                    gap: 4.0,
                    x: 24.0,
                    y: 24.0,
                    rotation: 0.0,
                }
            ],
            auto_hide_seconds: None,
            target_process: None,
            hide_from_capture: false,
            fade_in_ms: None,
            fade_out_ms: None,
            monitor_index: None,
            close_with_target: false,
        }
    }

    pub fn default_macro_badge() -> Self {
        Self {
            id: "overlay_macro_badge".to_string(),
            name: "Macro Status Badge".to_string(),
            rect: OverlayRect { x: 24.0, y: 24.0, width: 220.0, height: 44.0 },
            anchor: OverlayAnchor::TopRight,
            click_through: false,
            draggable: false,
            opacity: 0.95,
            style: OverlayStyle {
                bg_color: [14, 15, 20, 230],
                border_color: [80, 160, 255, 120],
                border_width: 1.0,
                corner_radius: 6.0,
                glow: true,
            },
            elements: vec![
                OverlayElement::Text {
                    id: "txt_badge".to_string(),
                    text: "MACRO ACTIVE".to_string(),
                    font_size: 13.0,
                    color: [80, 160, 255, 255],
                    x: 14.0,
                    y: 12.0,
                    width: None,
                    height: None,
                    lock_aspect: true,
                    bold: true,
                    font_family: "Segoe UI".to_string(),
                    rotation: 0.0,
                    on_click: None,
                },
                OverlayElement::Text {
                    id: "txt_hint".to_string(),
                    text: "[HOLD]".to_string(),
                    font_size: 11.5,
                    color: [160, 170, 190, 255],
                    x: 140.0,
                    y: 13.0,
                    width: None,
                    height: None,
                    lock_aspect: true,
                    bold: true,
                    font_family: "Segoe UI".to_string(),
                    rotation: 0.0,
                    on_click: None,
                }
            ],
            auto_hide_seconds: None,
            target_process: None,
            hide_from_capture: false,
            fade_in_ms: None,
            fade_out_ms: None,
            monitor_index: None,
            close_with_target: false,
        }
    }

    pub fn default_telemetry() -> Self {
        Self {
            id: "overlay_telemetry".to_string(),
            name: "Process Telemetry HUD".to_string(),
            rect: OverlayRect { x: 24.0, y: 80.0, width: 260.0, height: 74.0 },
            anchor: OverlayAnchor::TopRight,
            click_through: false,
            draggable: false,
            opacity: 0.9,
            style: OverlayStyle {
                bg_color: [16, 17, 24, 220],
                border_color: [255, 255, 255, 30],
                border_width: 1.0,
                corner_radius: 6.0,
                glow: false,
            },
            elements: vec![
                OverlayElement::Text {
                    id: "proc_title".to_string(),
                    text: "{ProcessName}".to_string(),
                    font_size: 13.0,
                    color: [255, 255, 255, 255],
                    x: 14.0,
                    y: 12.0,
                    width: None,
                    height: None,
                    lock_aspect: true,
                    bold: true,
                    font_family: "Segoe UI".to_string(),
                    rotation: 0.0,
                    on_click: None,
                },
                OverlayElement::Text {
                    id: "proc_stats".to_string(),
                    text: "Time: {Time} | Active".to_string(),
                    font_size: 11.5,
                    color: [140, 150, 170, 255],
                    x: 14.0,
                    y: 38.0,
                    width: None,
                    height: None,
                    lock_aspect: true,
                    bold: false,
                    font_family: "Segoe UI".to_string(),
                    rotation: 0.0,
                    on_click: None,
                }
            ],
            auto_hide_seconds: None,
            target_process: None,
            hide_from_capture: false,
            fade_in_ms: None,
            fade_out_ms: None,
            monitor_index: None,
            close_with_target: false,
        }
    }
}
