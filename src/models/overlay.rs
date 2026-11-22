mod defaults;
mod geometry;

pub use geometry::*;

use serde::{Deserialize, Serialize};

use super::serde_defaults::*;

#[derive(schemars::JsonSchema, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorInfo {
    pub index: usize,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub is_primary: bool,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitorOrientation {
    Landscape,
    Portrait,
}

impl MonitorInfo {
    pub fn orientation(&self) -> MonitorOrientation {
        if self.height > self.width {
            MonitorOrientation::Portrait
        } else {
            MonitorOrientation::Landscape
        }
    }

    pub fn is_portrait(&self) -> bool {
        self.height > self.width
    }
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum OverlayAnchor {
    #[default]
    ScreenCenter,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Custom,
    WindowRelative,
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OverlayRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Default for OverlayRect {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 260.0,
            height: 70.0,
        }
    }
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OverlayStyle {
    #[serde(default = "default_overlay_bg")]
    pub bg_color: [u8; 4],
    #[serde(default = "default_overlay_border")]
    pub border_color: [u8; 4],
    #[serde(default = "default_border_width")]
    pub border_width: f32,
    #[serde(default = "default_corner_radius")]
    pub corner_radius: f32,
    #[serde(default)]
    pub glow: bool,
}

impl Default for OverlayStyle {
    fn default() -> Self {
        Self {
            bg_color: default_overlay_bg(),
            border_color: default_overlay_border(),
            border_width: default_border_width(),
            corner_radius: default_corner_radius(),
            glow: false,
        }
    }
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum ShapeKind {
    #[default]
    Rectangle,
    Circle,
    Arrow,
    Line,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum CrosshairStyle {
    #[default]
    Cross,
    Dot,
    Circle,
    TShaped,
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "payload")]
pub enum OverlayClickAction {
    TriggerRule { rule_id: String },
    ToggleRule { rule_id: String },
    CloseOverlay,
    KillProcess { process_name: String },
    TrimMemory { process_name: String },
    ToggleMute,
    MediaPlayPause,
    MediaNext,
    MediaPrev,
    ResetTimer,
    ToggleTimer,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum GraphMetric {
    #[default]
    SystemCpu,
    ProcessCpu,
    SystemRam,
    ProcessRam,
    Fps,
    Battery,
    Network,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum GraphStyle {
    #[default]
    Line,
    AreaFilled,
    Histogram,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TimerMode {
    #[default]
    Clock,
    Stopwatch,
    Countdown,
    Uptime,
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InteractiveControlKind {
    Button {
        #[serde(default)]
        label: String,
        #[serde(default)]
        icon: Option<String>,
        #[serde(default)]
        action: Option<OverlayClickAction>,
    },
    ToggleSwitch {
        #[serde(default)]
        label: String,
        #[serde(default)]
        is_active: bool,
        #[serde(default)]
        bind_rule_id: Option<String>,
        #[serde(default)]
        on_enable: Option<OverlayClickAction>,
        #[serde(default)]
        on_disable: Option<OverlayClickAction>,
    },
    Slider {
        #[serde(default)]
        label: String,
        #[serde(default = "default_slider_val")]
        value: f32,
        #[serde(default)]
        min: f32,
        #[serde(default = "default_slider_max")]
        max: f32,
        #[serde(default)]
        bind_target: SliderBindTarget,
    },
}

impl Default for InteractiveControlKind {
    fn default() -> Self {
        InteractiveControlKind::Button {
            label: "Action".to_string(),
            icon: None,
            action: None,
        }
    }
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SliderBindTarget {
    #[default]
    MasterVolume,
    ProcessVolume,
    OverlayOpacity,
    CustomVariable,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryKind {
    #[default]
    Battery,
    AudioMicStatus,
    EngineStatus,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryStyle {
    #[default]
    Badge,
    Compact,
    Detailed,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyVisualizerLayout {
    WasdMouse,
    WasdOnly,
    ActionRpg,
    MouseOnly,
    TextStream,
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OverlayElement {
    Text {
        #[serde(default)]
        id: String,
        text: String,
        #[serde(default = "default_font_size")]
        font_size: f32,
        #[serde(default = "default_white_color")]
        color: [u8; 4],
        #[serde(default)]
        x: f32,
        #[serde(default)]
        y: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        height: Option<f32>,
        #[serde(default)]
        lock_aspect: bool,
        #[serde(default)]
        bold: bool,
        #[serde(default = "default_font_family")]
        font_family: String,
        #[serde(default)]
        rotation: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on_click: Option<OverlayClickAction>,
    },
    Image {
        #[serde(default)]
        id: String,
        path: String,
        #[serde(default)]
        x: f32,
        #[serde(default)]
        y: f32,
        #[serde(default = "default_image_size")]
        width: f32,
        #[serde(default = "default_image_size")]
        height: f32,
        #[serde(default = "default_opacity_one")]
        opacity: f32,
        #[serde(default = "default_true")]
        lock_aspect: bool,
        #[serde(default)]
        rotation: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on_click: Option<OverlayClickAction>,
    },
    Crosshair {
        #[serde(default)]
        id: String,
        #[serde(default)]
        style: CrosshairStyle,
        #[serde(default = "default_crosshair_size")]
        size: f32,
        #[serde(default = "default_crosshair_color")]
        color: [u8; 4],
        #[serde(default = "default_true")]
        center_dot: bool,
        #[serde(default = "default_crosshair_thickness")]
        thickness: f32,
        #[serde(default = "default_crosshair_gap")]
        gap: f32,
        #[serde(default)]
        x: f32,
        #[serde(default)]
        y: f32,
        #[serde(default)]
        rotation: f32,
    },
    Shape {
        #[serde(default)]
        id: String,
        #[serde(default)]
        shape_type: ShapeKind,
        #[serde(default)]
        x: f32,
        #[serde(default)]
        y: f32,
        #[serde(default = "default_shape_size")]
        width: f32,
        #[serde(default = "default_shape_size")]
        height: f32,
        #[serde(default = "default_shape_thickness")]
        thickness: f32,
        #[serde(default = "default_shape_color")]
        color: [u8; 4],
        #[serde(default)]
        filled: bool,
        #[serde(default = "default_shape_fill_color")]
        fill_color: [u8; 4],
        #[serde(default = "default_shape_corner_radius")]
        corner_radius: f32,
        #[serde(default = "default_true")]
        lock_aspect: bool,
        #[serde(default)]
        rotation: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on_click: Option<OverlayClickAction>,
    },
    Graph {
        #[serde(default)]
        id: String,
        #[serde(default)]
        metric: GraphMetric,
        #[serde(default)]
        style: GraphStyle,
        #[serde(default = "default_graph_sample_count")]
        sample_count: usize,
        #[serde(default = "default_accent_color")]
        color: [u8; 4],
        #[serde(default = "default_graph_fill_color")]
        fill_color: [u8; 4],
        #[serde(default = "default_dark_color")]
        bg_color: [u8; 4],
        #[serde(default = "default_true")]
        show_current_val: bool,
        #[serde(default)]
        x: f32,
        #[serde(default)]
        y: f32,
        #[serde(default = "default_graph_width")]
        width: f32,
        #[serde(default = "default_graph_height")]
        height: f32,
        #[serde(default)]
        rotation: f32,
    },
    ClockTimer {
        #[serde(default)]
        id: String,
        #[serde(default)]
        mode: TimerMode,
        #[serde(default = "default_time_format")]
        format_str: String,
        #[serde(default = "default_timer_countdown_ms")]
        countdown_total_ms: u64,
        #[serde(default = "default_timer_countdown_ms")]
        countdown_remaining_ms: u64,
        #[serde(default)]
        stopwatch_elapsed_ms: u64,
        #[serde(default)]
        is_running: bool,
        #[serde(default = "default_font_size")]
        font_size: f32,
        #[serde(default = "default_white_color")]
        color: [u8; 4],
        #[serde(default = "default_badge_bg")]
        bg_color: [u8; 4],
        #[serde(default = "default_true")]
        bold: bool,
        #[serde(default = "default_font_family")]
        font_family: String,
        #[serde(default)]
        x: f32,
        #[serde(default)]
        y: f32,
        #[serde(default = "default_clock_width")]
        width: f32,
        #[serde(default = "default_clock_height")]
        height: f32,
        #[serde(default)]
        rotation: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on_finish: Option<OverlayClickAction>,
    },
    InteractiveControl {
        #[serde(default)]
        id: String,
        #[serde(default)]
        control: InteractiveControlKind,
        #[serde(default = "default_white_color")]
        color: [u8; 4],
        #[serde(default = "default_badge_bg")]
        bg_color: [u8; 4],
        #[serde(default = "default_accent_color")]
        active_color: [u8; 4],
        #[serde(default)]
        x: f32,
        #[serde(default)]
        y: f32,
        #[serde(default = "default_control_width")]
        width: f32,
        #[serde(default = "default_control_height")]
        height: f32,
        #[serde(default)]
        rotation: f32,
    },
    SystemTelemetry {
        #[serde(default)]
        id: String,
        #[serde(default)]
        kind: TelemetryKind,
        #[serde(default)]
        style: TelemetryStyle,
        #[serde(default = "default_white_color")]
        color: [u8; 4],
        #[serde(default = "default_badge_bg")]
        bg_color: [u8; 4],
        #[serde(default)]
        x: f32,
        #[serde(default)]
        y: f32,
        #[serde(default = "default_telemetry_width")]
        width: f32,
        #[serde(default = "default_telemetry_height")]
        height: f32,
        #[serde(default)]
        rotation: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on_click: Option<OverlayClickAction>,
    },
    MediaWidget {
        #[serde(default)]
        id: String,
        #[serde(default = "default_true")]
        show_controls: bool,
        #[serde(default)]
        compact: bool,
        #[serde(default = "default_white_color")]
        color: [u8; 4],
        #[serde(default = "default_badge_bg")]
        bg_color: [u8; 4],
        #[serde(default)]
        x: f32,
        #[serde(default)]
        y: f32,
        #[serde(default = "default_media_width")]
        width: f32,
        #[serde(default = "default_media_height")]
        height: f32,
        #[serde(default)]
        rotation: f32,
    },
    KeyVisualizer {
        #[serde(default)]
        id: String,
        #[serde(default = "default_key_vis_layout")]
        layout: KeyVisualizerLayout,
        #[serde(default = "default_max_keys")]
        max_keys: usize,
        #[serde(default = "default_decay_seconds")]
        decay_seconds: f32,
        #[serde(default)]
        show_modifiers_only: bool,
        #[serde(default = "default_white_color")]
        color: [u8; 4],
        #[serde(default = "default_badge_bg")]
        bg_color: [u8; 4],
        #[serde(default = "default_accent_color")]
        active_color: [u8; 4],
        #[serde(default)]
        x: f32,
        #[serde(default)]
        y: f32,
        #[serde(default = "default_key_vis_width")]
        width: f32,
        #[serde(default = "default_key_vis_height")]
        height: f32,
        #[serde(default)]
        rotation: f32,
    },
}

impl OverlayElement {
    pub fn id(&self) -> &str {
        match self {
            OverlayElement::Text { id, .. }
            | OverlayElement::Crosshair { id, .. }
            | OverlayElement::Image { id, .. }
            | OverlayElement::Shape { id, .. }
            | OverlayElement::Graph { id, .. }
            | OverlayElement::ClockTimer { id, .. }
            | OverlayElement::InteractiveControl { id, .. }
            | OverlayElement::SystemTelemetry { id, .. }
            | OverlayElement::MediaWidget { id, .. }
            | OverlayElement::KeyVisualizer { id, .. } => id.as_str(),
        }
    }

    pub fn id_mut(&mut self) -> &mut String {
        match self {
            OverlayElement::Text { id, .. }
            | OverlayElement::Crosshair { id, .. }
            | OverlayElement::Image { id, .. }
            | OverlayElement::Shape { id, .. }
            | OverlayElement::Graph { id, .. }
            | OverlayElement::ClockTimer { id, .. }
            | OverlayElement::InteractiveControl { id, .. }
            | OverlayElement::SystemTelemetry { id, .. }
            | OverlayElement::MediaWidget { id, .. }
            | OverlayElement::KeyVisualizer { id, .. } => id,
        }
    }

    pub fn rotation(&self) -> f32 {
        match self {
            OverlayElement::Text { rotation, .. }
            | OverlayElement::Crosshair { rotation, .. }
            | OverlayElement::Image { rotation, .. }
            | OverlayElement::Shape { rotation, .. }
            | OverlayElement::Graph { rotation, .. }
            | OverlayElement::ClockTimer { rotation, .. }
            | OverlayElement::InteractiveControl { rotation, .. }
            | OverlayElement::SystemTelemetry { rotation, .. }
            | OverlayElement::MediaWidget { rotation, .. }
            | OverlayElement::KeyVisualizer { rotation, .. } => *rotation,
        }
    }

    pub fn rotation_mut(&mut self) -> &mut f32 {
        match self {
            OverlayElement::Text { rotation, .. }
            | OverlayElement::Crosshair { rotation, .. }
            | OverlayElement::Image { rotation, .. }
            | OverlayElement::Shape { rotation, .. }
            | OverlayElement::Graph { rotation, .. }
            | OverlayElement::ClockTimer { rotation, .. }
            | OverlayElement::InteractiveControl { rotation, .. }
            | OverlayElement::SystemTelemetry { rotation, .. }
            | OverlayElement::MediaWidget { rotation, .. }
            | OverlayElement::KeyVisualizer { rotation, .. } => rotation,
        }
    }

    pub fn x(&self) -> f32 {
        match self {
            OverlayElement::Text { x, .. }
            | OverlayElement::Crosshair { x, .. }
            | OverlayElement::Image { x, .. }
            | OverlayElement::Shape { x, .. }
            | OverlayElement::Graph { x, .. }
            | OverlayElement::ClockTimer { x, .. }
            | OverlayElement::InteractiveControl { x, .. }
            | OverlayElement::SystemTelemetry { x, .. }
            | OverlayElement::MediaWidget { x, .. }
            | OverlayElement::KeyVisualizer { x, .. } => *x,
        }
    }

    pub fn y(&self) -> f32 {
        match self {
            OverlayElement::Text { y, .. }
            | OverlayElement::Crosshair { y, .. }
            | OverlayElement::Image { y, .. }
            | OverlayElement::Shape { y, .. }
            | OverlayElement::Graph { y, .. }
            | OverlayElement::ClockTimer { y, .. }
            | OverlayElement::InteractiveControl { y, .. }
            | OverlayElement::SystemTelemetry { y, .. }
            | OverlayElement::MediaWidget { y, .. }
            | OverlayElement::KeyVisualizer { y, .. } => *y,
        }
    }

    pub fn x_mut(&mut self) -> &mut f32 {
        match self {
            OverlayElement::Text { x, .. }
            | OverlayElement::Crosshair { x, .. }
            | OverlayElement::Image { x, .. }
            | OverlayElement::Shape { x, .. }
            | OverlayElement::Graph { x, .. }
            | OverlayElement::ClockTimer { x, .. }
            | OverlayElement::InteractiveControl { x, .. }
            | OverlayElement::SystemTelemetry { x, .. }
            | OverlayElement::MediaWidget { x, .. }
            | OverlayElement::KeyVisualizer { x, .. } => x,
        }
    }

    pub fn y_mut(&mut self) -> &mut f32 {
        match self {
            OverlayElement::Text { y, .. }
            | OverlayElement::Crosshair { y, .. }
            | OverlayElement::Image { y, .. }
            | OverlayElement::Shape { y, .. }
            | OverlayElement::Graph { y, .. }
            | OverlayElement::ClockTimer { y, .. }
            | OverlayElement::InteractiveControl { y, .. }
            | OverlayElement::SystemTelemetry { y, .. }
            | OverlayElement::MediaWidget { y, .. }
            | OverlayElement::KeyVisualizer { y, .. } => y,
        }
    }
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OverlayDefinition {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub rect: OverlayRect,
    #[serde(default)]
    pub anchor: OverlayAnchor,
    #[serde(default)]
    pub click_through: bool,
    #[serde(default)]
    pub draggable: bool,
    #[serde(default = "default_opacity_one")]
    pub opacity: f32,
    #[serde(default)]
    pub style: OverlayStyle,
    #[serde(default)]
    pub elements: Vec<OverlayElement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_hide_seconds: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_process: Option<String>,
    #[serde(default)]
    pub hide_from_capture: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_in_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_out_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor_index: Option<usize>,
    #[serde(default)]
    pub close_with_target: bool,
}

impl Default for OverlayDefinition {
    fn default() -> Self {
        Self::default_new_overlay()
    }
}
