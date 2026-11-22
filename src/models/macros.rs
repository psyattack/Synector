use serde::{Deserialize, Serialize};

use super::action::{MouseButton, MouseCoordMode};
use super::serde_defaults::*;

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MacroEvent {
    KeyDown {
        key: String,
    },
    KeyUp {
        key: String,
    },
    KeyPress {
        key: String,
    },
    Text {
        text: String,
        #[serde(default = "default_text_delay")]
        delay_ms: u32,
        #[serde(default)]
        organic: bool,
    },
    MouseMove {
        x: i32,
        y: i32,
        #[serde(default)]
        coord_mode: MouseCoordMode,
        #[serde(default)]
        smooth: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u32>,
        #[serde(default)]
        organic: bool,
    },
    MouseDown {
        button: MouseButton,
    },
    MouseUp {
        button: MouseButton,
    },
    MouseClick {
        button: MouseButton,
        #[serde(default = "default_click_count")]
        clicks: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        x: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        y: Option<i32>,
        #[serde(default)]
        coord_mode: MouseCoordMode,
        #[serde(default)]
        smooth: bool,
    },
    MouseWheel {
        delta: i32,
    },
    Delay {
        ms: u32,
        #[serde(default)]
        organic: bool,
        #[serde(default = "default_jitter_percent")]
        jitter_percent: u32,
    },
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum MacroOperation {
    #[default]
    Run,
    Stop,
    Toggle,
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MacroPreset {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub events: Vec<MacroEvent>,
    #[serde(default = "default_repeat_one")]
    pub repeat: u32,
    #[serde(default = "default_speed_one")]
    pub speed: f32,
    #[serde(default)]
    pub infinite: bool,
}

impl MacroPreset {
    pub fn new(id: impl Into<String>, name: impl Into<String>, events: Vec<MacroEvent>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            events,
            repeat: 1,
            speed: 1.0,
            infinite: false,
        }
    }
}

pub fn default_macro_presets() -> Vec<MacroPreset> {
    vec![
        MacroPreset {
            id: "macro_spam_click".to_string(),
            name: "Rapid Clicker".to_string(),
            events: vec![
                MacroEvent::MouseDown { button: MouseButton::Left },
                MacroEvent::Delay { ms: 25, organic: false, jitter_percent: 0 },
                MacroEvent::MouseUp { button: MouseButton::Left },
                MacroEvent::Delay { ms: 25, organic: false, jitter_percent: 0 },
            ],
            repeat: 10,
            speed: 1.0,
            infinite: false,
        },
        MacroPreset {
            id: "macro_copy_paste".to_string(),
            name: "Copy & Paste".to_string(),
            events: vec![
                MacroEvent::KeyDown { key: "Control".to_string() },
                MacroEvent::KeyPress { key: "c".to_string() },
                MacroEvent::KeyUp { key: "Control".to_string() },
                MacroEvent::Delay { ms: 50, organic: false, jitter_percent: 0 },
                MacroEvent::KeyDown { key: "Control".to_string() },
                MacroEvent::KeyPress { key: "v".to_string() },
                MacroEvent::KeyUp { key: "Control".to_string() },
            ],
            repeat: 1,
            speed: 1.0,
            infinite: false,
        },
    ]
}
