mod overlay_ids;
mod validation;

use serde::{Deserialize, Serialize};

use super::condition::{RuleCondition, RuleConditionItem};
use super::macros::{MacroEvent, MacroOperation};
use super::overlay::OverlayDefinition;
use super::serde_defaults::*;
use super::trigger::ProcessMatchMode;

/// How long a resource created by a rule step stays alive.
///
/// A started process or shown overlay can outlive both the action and the pipeline that created
/// it, so its lifetime is explicit instead of a boolean "close on exit" flag.
#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResourceLifetime {
    /// Leave the process running / the overlay on screen after the rule is done with it.
    #[default]
    Detach,
    /// Stop the process / hide the overlay when the run that created it ends, however it ends.
    UntilRunEnd,
    /// Stop the process / hide the overlay when the process that triggered the run exits.
    /// Only valid for a process-start trigger that supplies a live primary process.
    UntilTriggerProcessExit,
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "$action_type", rename_all = "snake_case")]
pub enum Action {
    #[serde(rename = "start_process")]
    StartProcess {
        file_path: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        arguments: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        working_directory: Option<String>,
        #[serde(default)]
        run_as_admin: bool,
        #[serde(default)]
        only_if_not_running: bool,
        #[serde(default)]
        run_hidden: bool,
        #[serde(default)]
        lifetime: ResourceLifetime,
    },
    #[serde(rename = "kill_process")]
    KillProcess {
        process_name: String,
        #[serde(default)]
        kill_tree: bool,
        #[serde(default)]
        force: bool,
    },
    #[serde(rename = "window")]
    Window {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_process: Option<String>,
        window_action: WindowActionType,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        x: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        y: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        height: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transparency: Option<u8>,
    },
    #[serde(rename = "process_tuning")]
    ProcessTuning {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_process: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        priority: Option<ProcessPriorityLevel>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        affinity_mask: Option<u64>,
        #[serde(default)]
        trim_memory: bool,
    },
    #[serde(rename = "power_plan")]
    PowerPlan {
        plan: PowerPlanType,
    },
    #[serde(rename = "play_sound")]
    PlaySound {
        #[serde(default)]
        file_path: String,
        #[serde(default)]
        play_beep: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        beep_frequency: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        beep_duration_ms: Option<u32>,
    },
    #[serde(rename = "firewall")]
    Firewall {
        process_name: String,
        #[serde(default)]
        action: FirewallAction,
    },
    #[serde(rename = "notification")]
    Notification {
        title: String,
        message: String,
    },
    #[serde(rename = "http_webhook")]
    HttpWebhook {
        url: String,
        #[serde(default = "default_http_method")]
        method: String,
        #[serde(default)]
        headers: Option<std::collections::HashMap<String, String>>,
        #[serde(default)]
        payload: Option<String>,
    },
    #[serde(rename = "audio_control")]
    AudioControl {
        #[serde(default)]
        target: AudioTarget,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        process_name: Option<String>,
        action_type: AudioVolumeAction,
        #[serde(default)]
        volume_level: u8, // 0..100
        #[serde(default, skip_serializing_if = "Option::is_none")]
        left_volume: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        right_volume: Option<u8>,
    },
    #[serde(rename = "audio_format")]
    AudioFormat {
        #[serde(default)]
        target: AudioTarget,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device_name: Option<String>,
        #[serde(default = "default_sample_rate")]
        sample_rate: u32,
        #[serde(default = "default_bit_depth")]
        bit_depth: u32,
        #[serde(default = "default_channels_two")]
        channels: u32,
    },
    #[serde(rename = "default_device")]
    DefaultDevice {
        #[serde(default)]
        target: AudioTarget,
        device_name: String,
    },
    #[serde(rename = "execute_script")]
    ExecuteScript {
        #[serde(default)]
        interpreter: ScriptType,
        script: String,
        #[serde(default)]
        run_in_window: bool,
    },
    #[serde(rename = "macro")]
    Macro {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preset_id: Option<String>,
        #[serde(default)]
        operation: MacroOperation,
        #[serde(default)]
        events: Vec<MacroEvent>,
        #[serde(default = "default_repeat_one")]
        repeat: u32,
        #[serde(default = "default_speed_one")]
        speed: f32,
        #[serde(default)]
        infinite: bool,
    },
    #[serde(rename = "system_power")]
    SystemPower {
        power_type: SystemPowerType,
    },
    #[serde(rename = "wait")]
    Wait {
        #[serde(default)]
        mode: WaitMode,
        #[serde(default = "default_wait_seconds")]
        seconds: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        condition: Option<WaitCondition>,
        #[serde(default = "default_wait_timeout")]
        timeout_sec: f64,
        #[serde(default)]
        timeout_policy: WaitTimeoutPolicy,
    },
    #[serde(rename = "check")]
    Check {
        condition: RuleCondition,
    },
    #[serde(rename = "display")]
    Display {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        height: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        refresh_rate: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        monitor_index: Option<u32>,
    },
    #[serde(rename = "if_else")]
    IfElse {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        conditions: Vec<RuleConditionItem>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        then_actions: Vec<Action>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        else_if_branches: Vec<ElseIfBranch>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        else_actions: Vec<Action>,
    },
    #[serde(rename = "rule_control")]
    RuleControl {
        /// Id of the rule to invoke or stop. Empty means "nothing to do".
        rule_id: String,
        operation: RuleControlOperation,
        /// Wait for the called pipeline to finish before the caller continues. `call` only.
        #[serde(default)]
        wait: bool,
    },
    #[serde(rename = "trim_working_set")]
    TrimWorkingSet {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_process: Option<String>,
        
        #[serde(default, skip_serializing_if = "Option::is_none")]
        soft_limit_mb: Option<u64>,
        
        #[serde(default)]
        skip_if_focused: bool,
    },
    #[serde(rename = "discord_rpc_set")]
    DiscordRpcSet {
        app_id: String,
        #[serde(default)]
        details: String,
        #[serde(default)]
        state: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        large_image: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        large_text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        small_image: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        small_text: Option<String>,
        #[serde(default = "default_true")]
        show_elapsed: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        buttons: Vec<DiscordRpcButton>,
        #[serde(default)]
        auto_clear_on_process_exit: bool,
    },
    #[serde(rename = "discord_rpc_clear")]
    DiscordRpcClear,
    #[serde(rename = "archive")]
    Archive {
        operation: ArchiveOperation,
        #[serde(default)]
        format: ArchiveFormat,
        source_path: String,
        destination_path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        compression_level: Option<u8>,
        #[serde(default = "default_true")]
        overwrite: bool,
    },
    #[serde(rename = "file_system")]
    FileSystem {
        operation: FileSystemOperation,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content: Option<String>,
        #[serde(default = "default_true")]
        create_parents: bool,
        #[serde(default = "default_true")]
        overwrite: bool,
    },
    #[serde(rename = "overlay")]
    Overlay {
        overlay_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        custom_overlay: Option<Box<OverlayDefinition>>,
        operation: OverlayOperation,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        override_text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u32>,
        #[serde(default)]
        lifetime: ResourceLifetime,
    },
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct DiscordRpcButton {
    pub label: String,
    pub url: String,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveOperation {
    #[default]
    Pack,
    Unpack,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveFormat {
    #[default]
    Zip,
    Tar,
    TarGz,
    TarXz,
    SevenZip,
    Rar,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum FileSystemOperation {
    #[default]
    CreateFile,
    CreateDirectory,
    Rename,
    Copy,
    Delete,
    WriteText,
    AppendText,
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ElseIfBranch {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<RuleConditionItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
}

impl ElseIfBranch {
    pub fn new() -> Self {
        Self {
            conditions: vec![RuleConditionItem::new(RuleCondition::default_process())],
            actions: Vec::new(),
        }
    }
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum AudioTarget {
    #[default]
    Playback,
    Microphone,
    Process,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum AudioVolumeAction {
    #[default]
    Mute,
    Unmute,
    ToggleMute,
    SetVolume,
    PlayBeep,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScriptType {
    #[default]
    PowerShell,
    Batch,
    Lua,
}

/// What `Action::RuleControl` does to the target rule: start its pipeline or abort what it is
/// running right now.
#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum RuleControlOperation {
    #[default]
    Call,
    Stop,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum MouseActionType {
    #[default]
    Click,
    DoubleClick,
    Down,
    Up,
    Move,
    Scroll,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    #[default]
    Left,
    Right,
    Middle,
    XButton1,
    XButton2,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum MouseCoordMode {
    #[default]
    Current,
    Absolute,
    Relative,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum FirewallAction {
    #[default]
    Block,
    Allow,
    Toggle,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum WaitMode {
    #[default]
    Timer,
    UntilCondition,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum WaitTimeoutPolicy {
    #[default]
    Abort,
    Continue,
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WaitCondition {
    WindowOpened {
        #[serde(default)]
        process_name: String,
        #[serde(default)]
        match_mode: ProcessMatchMode,
    },
    WindowClosed {
        #[serde(default)]
        process_name: String,
        #[serde(default)]
        match_mode: ProcessMatchMode,
    },
    ProcessStarted {
        #[serde(default)]
        process_name: String,
        #[serde(default)]
        match_mode: ProcessMatchMode,
    },
    ProcessExited {
        #[serde(default)]
        process_name: String,
        #[serde(default)]
        match_mode: ProcessMatchMode,
    },
    Hotkey {
        #[serde(default)]
        key_combination: String,
    },
    File {
        #[serde(default)]
        path: String,
        #[serde(default)]
        condition: WaitFileCondition,
    },
    /// Another rule currently has a running action pipeline.
    RuleStarted {
        #[serde(default)]
        rule_id: String,
    },
    /// Another rule has no running action pipeline.
    RuleStopped {
        #[serde(default)]
        rule_id: String,
    },
}

impl WaitCondition {
    /// Value checks shared by `--rules-apply` and share import. A condition whose key is
    /// empty can never match and would silently burn the whole timeout.
    pub fn validation_errors(&self) -> Vec<&'static str> {
        match self {
            WaitCondition::WindowOpened { process_name, .. }
            | WaitCondition::WindowClosed { process_name, .. }
            | WaitCondition::ProcessStarted { process_name, .. }
            | WaitCondition::ProcessExited { process_name, .. } => {
                if process_name.trim().is_empty() {
                    vec!["wait condition needs a non-empty process_name"]
                } else {
                    Vec::new()
                }
            }
            WaitCondition::Hotkey { key_combination } => {
                if key_combination.trim().is_empty() {
                    vec!["wait condition needs a non-empty key_combination"]
                } else {
                    Vec::new()
                }
            }
            WaitCondition::File { path, .. } => {
                if path.trim().is_empty() {
                    vec!["wait condition needs a non-empty path"]
                } else {
                    Vec::new()
                }
            }
            WaitCondition::RuleStarted { rule_id } | WaitCondition::RuleStopped { rule_id } => {
                if rule_id.trim().is_empty() {
                    vec!["wait condition needs a non-empty rule_id"]
                } else {
                    Vec::new()
                }
            }
        }
    }
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum WaitFileCondition {
    #[default]
    Exists,
    Deleted,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum WindowActionType {
    #[default]
    Minimize,
    Maximize,
    Restore,
    Hide,
    Show,
    Close,
    ToggleAlwaysOnTop,
    AlwaysOnTop,
    RemoveAlwaysOnTop,
    SetTransparency,
    SnapToPosition,
    ToggleMinimizeRestore,
    ToggleShowHide,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProcessPriorityLevel {
    #[default]
    Normal,
    Realtime,
    High,
    AboveNormal,
    BelowNormal,
    Idle,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum PowerPlanType {
    #[default]
    Balanced,
    HighPerformance,
    PowerSaver,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SystemPowerType {
    Lock,
    Sleep,
    TurnScreenOff,
    Restart,
    Shutdown,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum OverlayOperation {
    #[default]
    Show,
    Hide,
    Toggle,
    Flash,
    SetText,
}
