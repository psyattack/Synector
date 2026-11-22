use super::overlay::KeyVisualizerLayout;

pub(super) fn default_threshold_duration() -> u64 {
    5
}

pub(super) fn default_sample_rate() -> u32 {
    48000
}

pub(super) fn default_bit_depth() -> u32 {
    24
}

pub(super) fn default_channels_two() -> u32 {
    2
}

pub(super) fn default_idle_sec() -> u32 {
    300
}

pub(super) fn default_repeat_one() -> u32 {
    1
}

pub(super) fn default_speed_one() -> f32 {
    1.0
}

pub(super) fn default_click_count() -> u32 {
    1
}

pub(super) fn default_text_delay() -> u32 {
    20
}

pub(super) fn default_jitter_percent() -> u32 {
    35
}

pub(super) fn default_http_method() -> String {
    "POST".to_string()
}

pub(super) fn default_wait_seconds() -> f64 {
    1.0
}

pub(super) fn default_wait_timeout() -> f64 {
    30.0
}

pub(super) fn default_true() -> bool {
    true
}

/// Serde helper: skip a field holding the `true` default.
pub(super) fn is_true(value: &bool) -> bool {
    *value
}

/// Serde helper: skip a field holding the `false` default.
pub(super) fn is_false(value: &bool) -> bool {
    !*value
}

/// Serde helper: skip a numeric field holding its zero default.
pub(super) fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

/// Serde helper: skip a float field holding its zero default.
pub(super) fn is_zero_f64(value: &f64) -> bool {
    *value == 0.0
}

pub(super) fn default_cooldown() -> f64 {
    0.0
}

pub(super) fn default_overlay_bg() -> [u8; 4] {
    [16, 17, 22, 220]
}

pub(super) fn default_overlay_border() -> [u8; 4] {
    [255, 255, 255, 30]
}

pub(super) fn default_border_width() -> f32 {
    1.0
}

pub(super) fn default_corner_radius() -> f32 {
    8.0
}

pub(super) fn default_key_vis_layout() -> KeyVisualizerLayout {
    KeyVisualizerLayout::WasdMouse
}

pub(super) fn default_graph_sample_count() -> usize { 30 }
pub(super) fn default_graph_fill_color() -> [u8; 4] { [0, 180, 255, 60] }
pub(super) fn default_graph_width() -> f32 { 160.0 }
pub(super) fn default_graph_height() -> f32 { 60.0 }
pub(super) fn default_time_format() -> String { "%H:%M:%S".to_string() }
pub(super) fn default_timer_countdown_ms() -> u64 { 60_000 }
pub(super) fn default_clock_width() -> f32 { 120.0 }
pub(super) fn default_clock_height() -> f32 { 32.0 }
pub(super) fn default_slider_val() -> f32 { 50.0 }
pub(super) fn default_slider_max() -> f32 { 100.0 }
pub(super) fn default_control_width() -> f32 { 120.0 }
pub(super) fn default_control_height() -> f32 { 32.0 }
pub(super) fn default_telemetry_width() -> f32 { 130.0 }
pub(super) fn default_telemetry_height() -> f32 { 28.0 }
pub(super) fn default_media_width() -> f32 { 220.0 }
pub(super) fn default_media_height() -> f32 { 54.0 }
pub(super) fn default_max_keys() -> usize { 4 }
pub(super) fn default_decay_seconds() -> f32 { 2.5 }
pub(super) fn default_key_vis_width() -> f32 { 170.0 }
pub(super) fn default_key_vis_height() -> f32 { 105.0 }

pub(super) fn default_shape_size() -> f32 {
    48.0
}
pub(super) fn default_shape_thickness() -> f32 {
    2.0
}
pub(super) fn default_shape_color() -> [u8; 4] {
    [80, 160, 255, 255]
}
pub(super) fn default_shape_fill_color() -> [u8; 4] {
    [80, 160, 255, 60]
}
pub(super) fn default_shape_corner_radius() -> f32 {
    4.0
}

pub(super) fn default_font_family() -> String {
    "Segoe UI".to_string()
}
pub(super) fn default_font_size() -> f32 {
    13.0
}
pub(super) fn default_white_color() -> [u8; 4] {
    [240, 242, 248, 255]
}
pub(super) fn default_image_size() -> f32 {
    32.0
}
pub(super) fn default_opacity_one() -> f32 {
    1.0
}
pub(super) fn default_crosshair_size() -> f32 {
    14.0
}
pub(super) fn default_crosshair_color() -> [u8; 4] {
    [0, 255, 136, 255]
}
pub(super) fn default_crosshair_thickness() -> f32 {
    2.0
}
pub(super) fn default_crosshair_gap() -> f32 {
    4.0
}
pub(super) fn default_accent_color() -> [u8; 4] {
    [70, 160, 255, 255]
}
pub(super) fn default_dark_color() -> [u8; 4] {
    [30, 32, 40, 220]
}
pub(super) fn default_badge_bg() -> [u8; 4] {
    [70, 160, 255, 45]
}
