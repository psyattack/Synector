use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};
use serde::{Deserialize, Serialize};

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "$trigger_type", rename_all = "snake_case")]
pub enum Trigger {
    #[serde(rename = "manual")]
    Manual,
    #[serde(rename = "process")]
    Process {
        process_name: String,
        #[serde(default)]
        match_mode: ProcessMatchMode,
        #[serde(default)]
        single_instance: bool,
    },
    #[serde(rename = "window_focus")]
    WindowFocus {
        process_name: String,
        #[serde(default = "default_true")]
        on_focus_gained: bool,
        #[serde(default)]
        on_focus_lost: bool,
    },
    #[serde(rename = "resource")]
    ResourceThreshold {
        process_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cpu_percent: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ram_mb: Option<u64>,
        #[serde(default = "default_threshold_duration")]
        duration_sec: u64,
    },
    #[serde(rename = "hotkey")]
    GlobalHotkey {
        key_combination: String,
        #[serde(default)]
        hold_mode: bool,
        #[serde(default)]
        repeat_while_held: bool,
    },
    #[serde(rename = "system")]
    SystemEvent {
        event_type: SystemEventType,
        #[serde(default)]
        device_id_filter: Option<String>,
    },
    #[serde(rename = "schedule")]
    Schedule {
        schedule_type: ScheduleType,
        #[serde(default)]
        interval_sec: u64,
        #[serde(default)]
        daily_time: Option<String>, // "HH:MM:SS"
        #[serde(default)]
        days_of_week: Vec<u8>,      // 1 = Monday .. 7 = Sunday
        #[serde(default, skip_serializing_if = "Option::is_none")]
        day_of_month: Option<u8>,   // 1..31 for Monthly schedule
        #[serde(default)]
        target_datetime: Option<String>,
    },
    #[serde(rename = "file_watcher")]
    FileWatcher {
        folder_path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter_extension: Option<String>,
        #[serde(default = "default_true")]
        on_created: bool,
        #[serde(default)]
        on_modified: bool,
        #[serde(default)]
        on_deleted: bool,
    },
    #[serde(rename = "webhook")]
    Webhook {
        #[serde(default)]
        endpoint_path: String,
    },
}

fn default_threshold_duration() -> u64 {
    5
}

impl Trigger {
    /// Structural problems inside a schedule payload, shared by `--rules-apply`, share import
    /// and the GUI save path. Empty for every other trigger.
    ///
    /// Each schedule type has its own mandatory fields: a Daily or OneShot rule without its
    /// time used to apply cleanly and then never fire, which looked like a broken engine.
    pub fn schedule_errors(&self) -> Vec<&'static str> {
        let Trigger::Schedule {
            schedule_type,
            interval_sec,
            daily_time,
            days_of_week,
            day_of_month,
            target_datetime,
        } = self
        else {
            return Vec::new();
        };
        let mut errors = Vec::new();
        if !days_of_week_are_valid(days_of_week) {
            errors.push("days_of_week must list days between 1 (Monday) and 7 (Sunday)");
        }
        let daily_time_ok = |daily_time: &Option<String>, errors: &mut Vec<&'static str>| match daily_time {
            Some(time) if is_valid_hh_mm_ss(time) => {}
            Some(_) => errors.push(
                "daily_time must be a zero-padded \"HH:MM\" or \"HH:MM:SS\" between 00:00 and 23:59",
            ),
            None => errors.push("this schedule needs daily_time"),
        };
        match schedule_type {
            ScheduleType::Interval => {
                if *interval_sec == 0 {
                    errors.push("interval schedule needs interval_sec > 0");
                }
                if let Some(target) = target_datetime {
                    if !is_valid_target_datetime(target) {
                        errors.push(
                            "target_datetime must be a valid \"YYYY-MM-DD HH:MM[:SS]\" timestamp",
                        );
                    }
                }
            }
            ScheduleType::Daily => {
                daily_time_ok(daily_time, &mut errors);
            }
            ScheduleType::OneShot => match target_datetime {
                Some(target) if is_valid_target_datetime(target) => {}
                Some(_) => errors.push(
                    "one_shot schedule needs target_datetime as \"YYYY-MM-DD HH:MM[:SS]\"",
                ),
                None => errors.push("one_shot schedule needs target_datetime"),
            },
            ScheduleType::Monthly => {
                daily_time_ok(daily_time, &mut errors);
                match day_of_month {
                    Some(day) if is_valid_day_of_month(*day) => {}
                    _ => errors.push("monthly schedule needs day_of_month between 1 and 31"),
                }
            }
            ScheduleType::AppStartup | ScheduleType::AppShutdown => {}
        }
        errors
    }
}


#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProcessMatchMode {
    #[default]
    Exact,
    Wildcard,
    Regex,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SystemEventType {
    SessionLock,
    SessionUnlock,
    SystemSleep,
    SystemResume,
    PowerSourceAc,
    PowerSourceBattery,
    UserIdle,
    UserActive,
    DeviceConnected,
    DeviceDisconnected,
    MonitorOn,
    MonitorOff,
    NetworkConnected,
    NetworkDisconnected,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleType {
    #[default]
    AppStartup,
    AppShutdown,
    Interval,
    Daily,
    OneShot,
    Monthly,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConditionOperator {
    #[default]
    And,
    Or,
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(from = "RuleConditionItemHelper", into = "RuleConditionItemHelper")]
pub struct RuleConditionItem {
    pub operator: ConditionOperator,
    pub condition: RuleCondition,
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
enum RuleConditionItemHelper {
    Item {
        #[serde(default)]
        operator: ConditionOperator,
        condition: RuleCondition,
    },
    Bare(RuleCondition),
}

impl From<RuleConditionItemHelper> for RuleConditionItem {
    fn from(helper: RuleConditionItemHelper) -> Self {
        match helper {
            RuleConditionItemHelper::Item { operator, condition } => Self { operator, condition },
            RuleConditionItemHelper::Bare(condition) => Self {
                operator: ConditionOperator::And,
                condition,
            },
        }
    }
}

impl From<RuleConditionItem> for RuleConditionItemHelper {
    fn from(item: RuleConditionItem) -> Self {
        RuleConditionItemHelper::Item {
            operator: item.operator,
            condition: item.condition,
        }
    }
}

impl RuleConditionItem {
    pub fn new(condition: RuleCondition) -> Self {
        Self {
            operator: ConditionOperator::And,
            condition,
        }
    }

    pub fn with_operator(operator: ConditionOperator, condition: RuleCondition) -> Self {
        Self {
            operator,
            condition,
        }
    }
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "$condition_type", rename_all = "snake_case")]
pub enum RuleCondition {
    #[serde(rename = "process_running")]
    ProcessRunning {
        process_name: String,
        #[serde(default)]
        match_mode: ProcessMatchMode,
        #[serde(default = "default_true")]
        must_be_running: bool,
    },
    #[serde(rename = "window_focused")]
    WindowFocused {
        process_name: String,
        #[serde(default)]
        match_mode: ProcessMatchMode,
        #[serde(default = "default_true")]
        must_be_focused: bool,
    },
    #[serde(rename = "time_range")]
    TimeRange {
        start_time: String, // "HH:MM"
        end_time: String,   // "HH:MM"
        #[serde(default)]
        days_of_week: Vec<u8>, // 1 = Monday .. 7 = Sunday
    },
    #[serde(rename = "event")]
    Event {
        event_type: SystemEventType,
        #[serde(default = "default_idle_sec")]
        idle_duration_sec: u32,
    },
    #[serde(rename = "resource")]
    ResourceThreshold {
        process_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cpu_percent: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ram_mb: Option<u64>,
    },
    #[serde(rename = "file_watcher")]
    FileWatcher {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter_extension: Option<String>,
        #[serde(default = "default_true")]
        must_exist: bool,
    },
    /// Whether another rule has a running action pipeline right now.
    #[serde(rename = "rule_running")]
    RuleRunning {
        /// Id of the observed rule. An empty id never satisfies the condition.
        rule_id: String,
        #[serde(default = "default_true")]
        must_be_running: bool,
    },
}

impl RuleCondition {
    pub fn default_process() -> Self {
        Self::ProcessRunning {
            process_name: String::new(),
            match_mode: ProcessMatchMode::Exact,
            must_be_running: true,
        }
    }

    pub fn icon_glyph(&self) -> &'static str {
        match self {
            Self::ProcessRunning { .. } => egui_phosphor::regular::CPU,
            Self::WindowFocused { .. } => egui_phosphor::regular::APP_WINDOW,
            Self::TimeRange { .. } => egui_phosphor::regular::CLOCK,
            Self::Event { .. } => egui_phosphor::regular::BELL,
            Self::ResourceThreshold { .. } => egui_phosphor::regular::CHART_LINE,
            Self::FileWatcher { .. } => egui_phosphor::regular::FILE,
            Self::RuleRunning { .. } => egui_phosphor::regular::PLAY,
        }
    }

    /// Shared value checks used by `--rules-apply` and share import.
    pub fn validation_errors(&self) -> Vec<&'static str> {
        match self {
            RuleCondition::TimeRange {
                start_time,
                end_time,
                days_of_week,
            } => {
                let mut errors = Vec::new();
                if !start_time.is_empty() && !is_valid_hh_mm(start_time) {
                    errors.push("time_range start_time must be a zero-padded HH:MM between 00:00 and 23:59");
                }
                if !end_time.is_empty() && !is_valid_hh_mm(end_time) {
                    errors.push("time_range end_time must be a zero-padded HH:MM between 00:00 and 23:59");
                }
                if !days_of_week_are_valid(days_of_week) {
                    errors.push("days_of_week must list days between 1 (Monday) and 7 (Sunday)");
                }
                errors
            }
            RuleCondition::ResourceThreshold {
                process_name,
                cpu_percent,
                ram_mb,
                ..
            } => resource_threshold_errors(process_name, *cpu_percent, *ram_mb),
            _ => Vec::new(),
        }
    }
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
        close_on_exit: bool,
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
        close_on_exit: bool,
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

pub fn default_sample_rate() -> u32 {
    48000
}

pub fn default_bit_depth() -> u32 {
    24
}

pub fn default_channels_two() -> u32 {
    2
}

pub fn default_idle_sec() -> u32 {
    300
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

pub fn default_repeat_one() -> u32 {
    1
}

pub fn default_speed_one() -> f32 {
    1.0
}

pub fn default_click_count() -> u32 {
    1
}

pub fn default_text_delay() -> u32 {
    20
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum MacroOperation {
    #[default]
    Run,
    Stop,
    Toggle,
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

fn default_jitter_percent() -> u32 {
    35
}

fn default_http_method() -> String {
    "POST".to_string()
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

pub fn default_wait_seconds() -> f64 {
    1.0
}

pub fn default_wait_timeout() -> f64 {
    30.0
}

/// Longest wait a rule may express, in seconds. A day-long pipeline is already an authoring
/// mistake, and an unbounded `seconds` is what makes `Duration::from_secs_f64` panic.
pub const MAX_WAIT_SECONDS: f64 = 86_400.0;

/// Fallible conversion of a rule-authored second count into a [`std::time::Duration`].
///
/// A rule file is untrusted input and `Duration::from_secs_f64` panics on negative,
/// non-finite and huge values. Every runtime use of `seconds`/`timeout_sec` goes through
/// this helper so a hand-edited rule becomes a normal action error instead of a dead task.
pub fn duration_from_secs(seconds: f64) -> Result<std::time::Duration, String> {
    if !seconds.is_finite() {
        return Err("duration must be a finite number of seconds".to_string());
    }
    if seconds < 0.0 {
        return Err("duration must not be negative".to_string());
    }
    if seconds > MAX_WAIT_SECONDS {
        return Err(format!(
            "duration must not exceed {MAX_WAIT_SECONDS:.0} seconds"
        ));
    }
    Ok(std::time::Duration::from_secs_f64(seconds))
}

fn parse_two_digits(part: &str) -> Option<u8> {
    if part.len() != 2 || !part.as_bytes().iter().all(u8::is_ascii_digit) {
        return None;
    }
    part.parse().ok()
}

/// True for a zero-padded `HH:MM` between 00:00 and 23:59.
pub fn is_valid_hh_mm(value: &str) -> bool {
    let Some((hour, minute)) = value.split_once(':') else {
        return false;
    };
    matches!(
        (parse_two_digits(hour), parse_two_digits(minute)),
        (Some(hour), Some(minute)) if hour < 24 && minute < 60
    )
}

/// True for a zero-padded `HH:MM` or `HH:MM:SS` between 00:00:00 and 23:59:59.
pub fn is_valid_hh_mm_ss(value: &str) -> bool {
    let parts: Vec<&str> = value.split(':').collect();
    match parts.as_slice() {
        [hour, minute] => matches!(
            (parse_two_digits(hour), parse_two_digits(minute)),
            (Some(hour), Some(minute)) if hour < 24 && minute < 60
        ),
        [hour, minute, second] => matches!(
            (parse_two_digits(hour), parse_two_digits(minute), parse_two_digits(second)),
            (Some(hour), Some(minute), Some(second)) if hour < 24 && minute < 60 && second < 60
        ),
        _ => false,
    }
}

/// True for a local datetime in the wire format `YYYY-MM-DD HH:MM[:SS]`.
pub fn is_valid_target_datetime(value: &str) -> bool {
    chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S").is_ok()
        || chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M").is_ok()
}

/// True when every weekday number is in the ISO range 1 (Monday) ..= 7 (Sunday).
pub fn days_of_week_are_valid(days: &[u8]) -> bool {
    days.iter().all(|day| (1..=7).contains(day))
}

/// True when a monthly schedule day is in 1 ..= 31.
pub fn is_valid_day_of_month(day: u8) -> bool {
    (1..=31).contains(&day)
}

/// Shared value checks for the resource trigger and the resource condition. Both need a
/// process and at least one threshold; without one they would silently degrade to
/// "process is running", which `process_running` already expresses.
pub fn resource_threshold_errors(
    process_name: &str,
    cpu_percent: Option<f32>,
    ram_mb: Option<u64>,
) -> Vec<&'static str> {
    let mut errors = Vec::new();
    if process_name.trim().is_empty() {
        errors.push("resource threshold needs a non-empty process_name");
    }
    if cpu_percent.is_none() && ram_mb.is_none() {
        errors.push("resource threshold needs cpu_percent or ram_mb");
    }
    if let Some(cpu) = cpu_percent {
        if !cpu.is_finite() || cpu < 0.0 {
            errors.push("cpu_percent must be a finite non-negative number");
        }
    }
    if ram_mb == Some(0) {
        errors.push("ram_mb must be greater than 0");
    }
    errors
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

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
pub struct RuleTag {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

/// Parses a tag colour written as `#RRGGBB` or `RRGGBB`. Only six ASCII hex digits are
/// accepted; anything else returns `None` so callers can fall back or reject the rule.
pub fn parse_hex_color(value: &str) -> Option<[u8; 3]> {
    let clean = value.trim().trim_start_matches('#');
    if clean.len() != 6 || !clean.is_ascii() {
        return None;
    }
    let rgb = u32::from_str_radix(clean, 16).ok()?;
    Some([(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8])
}

impl RuleTag {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            color: None,
            icon: None,
        }
    }

    pub fn with_color(mut self, color: impl Into<String>) -> Self {
        self.color = Some(color.into());
        self
    }

    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn parse_color(&self, fallback: egui::Color32) -> egui::Color32 {
        self.color
            .as_deref()
            .and_then(parse_hex_color)
            .map(|[r, g, b]| egui::Color32::from_rgb(r, g, b))
            .unwrap_or(fallback)
    }
}

/// Unit used by a custom quota window.
#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum QuotaUnit {
    #[default]
    Minutes,
    Hours,
    Days,
    Weeks,
    Months,
}

impl QuotaUnit {
    pub const ALL: [QuotaUnit; 5] = [
        QuotaUnit::Minutes,
        QuotaUnit::Hours,
        QuotaUnit::Days,
        QuotaUnit::Weeks,
        QuotaUnit::Months,
    ];

    /// Fixed length of one unit in seconds. Calendar months have no fixed length and are
    /// added as calendar months by [`QuotaPeriod::custom_window_end`].
    pub fn fixed_seconds(self) -> Option<u64> {
        match self {
            QuotaUnit::Minutes => Some(60),
            QuotaUnit::Hours => Some(3_600),
            QuotaUnit::Days => Some(86_400),
            QuotaUnit::Weeks => Some(604_800),
            QuotaUnit::Months => None,
        }
    }
}

/// Reset period of a rule's execution quota.
#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum QuotaPeriod {
    /// Reset at the start of every clock minute.
    Minute,
    /// Reset at the start of every clock hour.
    Hour,
    /// Reset at local midnight.
    Day,
    /// Reset at Monday 00:00 of the ISO week.
    Week,
    /// Reset on the 1st day of the month at 00:00.
    Month,
    /// Fixed-length window anchored at the first execution after the previous window lapsed.
    Custom {
        /// Window length in `unit`s.
        value: u32,
        #[serde(default)]
        unit: QuotaUnit,
    },
}

impl QuotaPeriod {
    /// Largest allowed length of a custom window, in `unit`s. Matches the GUI clamp so the
    /// CLI, share import and the editor accept exactly the same range.
    pub const MAX_CUSTOM_WINDOW_UNITS: u32 = 100_000;

    /// Rejects custom window lengths that are zero or absurdly large, which would overflow
    /// the calendar arithmetic and silently break the reset.
    pub fn validation_error(self) -> Option<&'static str> {
        match self {
            QuotaPeriod::Custom { value, .. }
                if !(1..=Self::MAX_CUSTOM_WINDOW_UNITS).contains(&value) =>
            {
                Some("a custom quota window length must be between 1 and 100000")
            }
            _ => None,
        }
    }

    /// End of a custom window that started at `start`; calendar months are added as
    /// calendar months (Jan 31 + 1 month = Feb 28/29), other units as fixed time.
    ///
    /// Lengths are clamped into the accepted range first, and the arithmetic is checked:
    /// `None` means the end cannot be represented and the window must be treated as lapsed.
    pub fn custom_window_end(self, start: DateTime<Local>) -> Option<DateTime<Local>> {
        match self {
            QuotaPeriod::Custom { value, unit } => {
                let value = value.clamp(1, Self::MAX_CUSTOM_WINDOW_UNITS);
                match unit.fixed_seconds() {
                    Some(seconds) => {
                        let total = i64::try_from(seconds).ok()?.checked_mul(i64::from(value))?;
                        start.checked_add_signed(chrono::Duration::seconds(total))
                    }
                    None => start.checked_add_months(chrono::Months::new(value)),
                }
            }
            _ => None,
        }
    }
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Rule {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub is_enabled: bool,
    #[serde(default = "default_cooldown", skip_serializing_if = "is_zero_f64")]
    pub cooldown_seconds: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_executions: Option<u64>,
    /// Reset period for `max_executions`. `None` counts executions for the lifetime of
    /// the rule (historical behavior), `Some(_)` limits them per period.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_period: Option<QuotaPeriod>,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub execution_count: u64,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub stop_on_error: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<Trigger>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<RuleConditionItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_exit_actions: Vec<Action>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_icon_path: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub auto_process_icon: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_executed_at: Option<String>,
    /// Runtime anchor of a custom quota window, local `%Y-%m-%d %H:%M:%S`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_window_start: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<RuleTag>,
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            description: String::new(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            max_executions: None,
            quota_period: None,
            execution_count: 0,
            stop_on_error: true,
            trigger: Some(Trigger::Manual),
            conditions: Vec::new(),
            actions: Vec::new(),
            on_exit_actions: Vec::new(),
            custom_icon_path: None,
            auto_process_icon: false,
            last_executed_at: None,
            quota_window_start: None,
            tags: Vec::new(),
        }
    }
}

impl Action {
    pub fn refresh_overlay_ids(&mut self) {
        match self {
            Action::Overlay { overlay_id, custom_overlay, .. } => {
                if custom_overlay.is_some() {
                    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
                    let count = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let fresh_id = format!(
                        "overlay_{:x}_{:x}",
                        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos(),
                        count
                    );
                    *overlay_id = fresh_id.clone();
                    if let Some(def) = custom_overlay.as_deref_mut() {
                        def.id = fresh_id;
                    }
                }
            }
            Action::IfElse { then_actions, else_if_branches, else_actions, .. } => {
                for a in then_actions {
                    a.refresh_overlay_ids();
                }
                for b in else_if_branches {
                    for a in &mut b.actions {
                        a.refresh_overlay_ids();
                    }
                }
                for a in else_actions {
                    a.refresh_overlay_ids();
                }
            }
            _ => {}
        }
    }

    /// Makes the two overlay identities of an [`Action::Overlay`] agree: the embedded
    /// definition takes the action's `overlay_id` (or donates its id when that is empty).
    ///
    /// The runner shows the embedded definition while the engine binds `close_on_exit`
    /// under the action's id; a mismatch between them leaks the overlay. Called on every
    /// path that accepts a rule definition (config load, share import, CLI apply).
    pub fn normalize_overlay_ids(&mut self) {
        match self {
            Action::Overlay {
                overlay_id,
                custom_overlay,
                ..
            } => {
                if let Some(definition) = custom_overlay.as_deref_mut() {
                    if overlay_id.trim().is_empty() {
                        *overlay_id = definition.id.clone();
                    } else {
                        definition.id = overlay_id.clone();
                    }
                }
            }
            Action::IfElse {
                then_actions,
                else_if_branches,
                else_actions,
                ..
            } => {
                for action in then_actions {
                    action.normalize_overlay_ids();
                }
                for branch in else_if_branches {
                    for action in &mut branch.actions {
                        action.normalize_overlay_ids();
                    }
                }
                for action in else_actions {
                    action.normalize_overlay_ids();
                }
            }
            _ => {}
        }
    }

    pub fn duplicate_with_fresh_ids(&self) -> Self {
        let mut cloned = self.clone();
        cloned.normalize_overlay_ids();
        cloned.refresh_overlay_ids();
        cloned
    }

    /// Value checks for one action, shared by `--rules-apply` and share import. Nested
    /// if/else pipelines are walked by the callers.
    pub fn validation_errors(&self) -> Vec<String> {
        let mut errors = Vec::new();
        match self {
            Action::Wait {
                mode,
                condition,
                seconds,
                timeout_sec,
                ..
            } => {
                if *mode == WaitMode::UntilCondition {
                    match condition {
                        Some(condition) => {
                            errors.extend(condition.validation_errors().into_iter().map(str::to_string));
                        }
                        None => errors.push("an until_condition wait needs a condition".to_string()),
                    }
                }
                if let Err(err) = duration_from_secs(*seconds) {
                    errors.push(format!("wait seconds: {err}"));
                }
                if let Err(err) = duration_from_secs(*timeout_sec) {
                    errors.push(format!("wait timeout_sec: {err}"));
                }
            }
            Action::Check { condition } => {
                errors.extend(condition.validation_errors().into_iter().map(str::to_string));
            }
            _ => {}
        }
        errors
    }
}

impl Rule {
    pub fn clone_with_new_id(&self) -> Self {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let count = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let new_id = format!(
            "rule_{:x}_{:x}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
            count
        );
        let new_name = generate_clone_name(&self.name);

        let mut cloned = self.clone();
        cloned.id = new_id;
        cloned.name = new_name;
        cloned.is_enabled = false;
        cloned.clear_quota_runtime();
        cloned.stop_on_error = self.stop_on_error;
        cloned.custom_icon_path = self.custom_icon_path.clone();
        cloned.auto_process_icon = self.auto_process_icon;

        // Ensure cloned overlay actions get their own fresh unique IDs so they don't collide with original rule
        cloned.normalize();
        for action in cloned.actions.iter_mut() {
            action.refresh_overlay_ids();
        }
        for action in cloned.on_exit_actions.iter_mut() {
            action.refresh_overlay_ids();
        }

        cloned
    }

    /// Clears the daemon-owned quota counters and the custom-window anchor.
    pub fn clear_quota_runtime(&mut self) {
        self.execution_count = 0;
        self.last_executed_at = None;
        self.quota_window_start = None;
    }

    /// True when `self` redefines the quota relative to `previous`: the reset period changed,
    /// or a limit was enabled where there was none. Changing only the numeric limit keeps
    /// the running counter, which is the documented update semantics.
    pub fn quota_redefined_from(&self, previous: &Rule) -> bool {
        self.quota_period != previous.quota_period
            || (previous.max_executions.is_none() && self.max_executions.is_some())
    }

    /// Clears the quota counter and the custom-window anchor but keeps `last_executed_at`,
    /// which schedule triggers use for deduplication.
    pub fn reset_quota_progress(&mut self) {
        self.execution_count = 0;
        self.quota_window_start = None;
    }

    /// Description of the first tag colour that is not `#RRGGBB`, for apply/import rejection.
    pub fn tag_color_error(&self) -> Option<String> {
        self.tags.iter().find_map(|tag| {
            let color = tag.color.as_deref()?;
            if parse_hex_color(color).is_some() {
                None
            } else {
                Some(format!(
                    "tag \"{}\" has colour \"{}\"; expected #RRGGBB",
                    tag.name, color
                ))
            }
        })
    }

    /// Normalizes every action definition of the rule (currently overlay ids).
    pub fn normalize(&mut self) {
        for action in self.actions.iter_mut().chain(self.on_exit_actions.iter_mut()) {
            action.normalize_overlay_ids();
        }
    }

    fn parsed_timestamp(stamp: Option<&str>) -> Option<DateTime<Local>> {
        let naive = chrono::NaiveDateTime::parse_from_str(stamp?, "%Y-%m-%d %H:%M:%S").ok()?;
        Local.from_local_datetime(&naive).single()
    }

    fn last_executed_local(&self) -> Option<DateTime<Local>> {
        Self::parsed_timestamp(self.last_executed_at.as_deref())
    }

    fn quota_window_start_local(&self) -> Option<DateTime<Local>> {
        Self::parsed_timestamp(self.quota_window_start.as_deref())
    }

    /// Identifier of the calendar bucket of `period` that contains `at`.
    fn calendar_bucket(period: QuotaPeriod, at: DateTime<Local>) -> Option<String> {
        match period {
            QuotaPeriod::Minute => Some(at.format("%Y-%m-%d %H:%M").to_string()),
            QuotaPeriod::Hour => Some(at.format("%Y-%m-%d %H").to_string()),
            QuotaPeriod::Day => Some(at.format("%Y-%m-%d").to_string()),
            QuotaPeriod::Week => Some(at.format("%G-W%V").to_string()),
            QuotaPeriod::Month => Some(at.format("%Y-%m").to_string()),
            QuotaPeriod::Custom { .. } => None,
        }
    }

    /// True when the stored counter still belongs to the quota window containing `now`.
    fn quota_window_is_current(&self, now: DateTime<Local>) -> bool {
        match self.quota_period {
            None => true,
            Some(period @ QuotaPeriod::Custom { .. }) => {
                let Some(start) = self.quota_window_start_local() else {
                    return false;
                };
                let Some(end) = period.custom_window_end(start) else {
                    // An unrepresentable window end (calendar overflow or a corrupt value)
                    // is treated as lapsed so the quota cannot get stuck forever.
                    return false;
                };
                now < end
            }
            Some(period) => match self.last_executed_local() {
                Some(last) => Self::calendar_bucket(period, last) == Self::calendar_bucket(period, now),
                None => false,
            },
        }
    }

    /// Executions counted against the quota in the period containing `now`. A lapsed
    /// window reads as zero even before the stored counter is reset.
    pub fn quota_used(&self, now: DateTime<Local>) -> u64 {
        if self.quota_window_is_current(now) {
            self.execution_count
        } else {
            0
        }
    }

    /// Remaining executions in the current period; `None` when the rule is unlimited.
    pub fn quota_remaining(&self, now: DateTime<Local>) -> Option<u64> {
        let limit = self.max_executions?;
        Some(limit.saturating_sub(self.quota_used(now)))
    }

    pub fn quota_exhausted(&self, now: DateTime<Local>) -> bool {
        self.max_executions
            .is_some_and(|limit| self.quota_used(now) >= limit)
    }

    /// Drops the counter of a lapsed period. Returns `true` when the counters changed,
    /// so the daemon can persist the rollover.
    pub fn normalize_quota(&mut self, now: DateTime<Local>) -> bool {
        if self.quota_period.is_none() || self.quota_window_is_current(now) {
            return false;
        }
        let changed = self.execution_count != 0 || self.quota_window_start.is_some();
        self.execution_count = 0;
        self.quota_window_start = None;
        changed
    }

    /// Anchors a custom window at the moment its first execution starts.
    pub fn begin_quota_window(&mut self, now: DateTime<Local>) -> bool {
        if !matches!(self.quota_period, Some(QuotaPeriod::Custom { .. })) || self.quota_window_start.is_some() {
            return false;
        }
        self.quota_window_start = Some(now.format("%Y-%m-%d %H:%M:%S").to_string());
        true
    }

    /// The next moment the quota counter resets, when that moment is known.
    pub fn quota_next_reset(&self, now: DateTime<Local>) -> Option<DateTime<Local>> {
        match self.quota_period? {
            period @ QuotaPeriod::Custom { .. } => {
                let start = self.quota_window_start_local()?;
                let end = period.custom_window_end(start)?;
                (end > now).then_some(end)
            }
            period => {
                let midnight = now.date_naive().and_hms_opt(0, 0, 0)?;
                let start_of_day = Local.from_local_datetime(&midnight).single()?;
                let next = match period {
                    QuotaPeriod::Minute => {
                        let start_of_minute = Local
                            .from_local_datetime(&now.date_naive().and_hms_opt(now.hour(), now.minute(), 0)?)
                            .single()?;
                        start_of_minute + chrono::Duration::minutes(1)
                    }
                    QuotaPeriod::Hour => start_of_day + chrono::Duration::hours(now.hour() as i64 + 1),
                    QuotaPeriod::Day => start_of_day + chrono::Duration::days(1),
                    QuotaPeriod::Week => {
                        let monday =
                            start_of_day - chrono::Duration::days(now.weekday().num_days_from_monday() as i64);
                        monday + chrono::Duration::weeks(1)
                    }
                    QuotaPeriod::Month => {
                        let (year, month) = if now.month() == 12 {
                            (now.year() + 1, 1)
                        } else {
                            (now.year(), now.month() + 1)
                        };
                        let naive = chrono::NaiveDate::from_ymd_opt(year, month, 1)?.and_hms_opt(0, 0, 0)?;
                        Local.from_local_datetime(&naive).single()?
                    }
                    QuotaPeriod::Custom { .. } => return None,
                };
                Some(next)
            }
        }
    }
}

pub fn generate_clone_name(name: &str) -> String {
    if let Some(base) = name.strip_suffix(" (Copy)") {
        format!("{} (Copy 2)", base)
    } else if let Some((base, num_str)) = name.rsplit_once(" (Copy ") {
        if let Some(num_trimmed) = num_str.strip_suffix(')') {
            if let Ok(n) = num_trimmed.parse::<u32>() {
                format!("{} (Copy {})", base, n + 1)
            } else {
                format!("{} (Copy)", name)
            }
        } else {
            format!("{} (Copy)", name)
        }
    } else {
        format!("{} (Copy)", name)
    }
}

fn default_true() -> bool {
    true
}

/// Serde helper: skip a field holding the `true` default.
fn is_true(value: &bool) -> bool {
    *value
}

/// Serde helper: skip a field holding the `false` default.
fn is_false(value: &bool) -> bool {
    !*value
}

/// Serde helper: skip a numeric field holding its zero default.
fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

/// Serde helper: skip a float field holding its zero default.
fn is_zero_f64(value: &f64) -> bool {
    *value == 0.0
}

fn default_cooldown() -> f64 {
    0.0
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProcessSnapshotItem {
    pub pid: u32,
    pub name: String,
    pub window_title: String,
    pub memory_bytes: u64,
    pub cpu_percent: f32,
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LogItem {
    pub timestamp: String,
    pub level: String,
    pub category: String,
    pub message: String,
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct AnalyticsSummary {
    #[serde(default)]
    pub total_executions: u64,
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

pub fn default_overlay_bg() -> [u8; 4] {
    [16, 17, 22, 220]
}

pub fn default_overlay_border() -> [u8; 4] {
    [255, 255, 255, 30]
}

pub fn default_border_width() -> f32 {
    1.0
}

pub fn default_corner_radius() -> f32 {
    8.0
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

pub fn default_key_vis_layout() -> KeyVisualizerLayout {
    KeyVisualizerLayout::WasdMouse
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

pub fn default_graph_sample_count() -> usize { 30 }
pub fn default_graph_fill_color() -> [u8; 4] { [0, 180, 255, 60] }
pub fn default_graph_width() -> f32 { 160.0 }
pub fn default_graph_height() -> f32 { 60.0 }
pub fn default_time_format() -> String { "%H:%M:%S".to_string() }
pub fn default_timer_countdown_ms() -> u64 { 60_000 }
pub fn default_clock_width() -> f32 { 120.0 }
pub fn default_clock_height() -> f32 { 32.0 }
pub fn default_slider_val() -> f32 { 50.0 }
pub fn default_slider_max() -> f32 { 100.0 }
pub fn default_control_width() -> f32 { 120.0 }
pub fn default_control_height() -> f32 { 32.0 }
pub fn default_telemetry_width() -> f32 { 130.0 }
pub fn default_telemetry_height() -> f32 { 28.0 }
pub fn default_media_width() -> f32 { 220.0 }
pub fn default_media_height() -> f32 { 54.0 }
pub fn default_max_keys() -> usize { 4 }
pub fn default_decay_seconds() -> f32 { 2.5 }
pub fn default_key_vis_width() -> f32 { 170.0 }
pub fn default_key_vis_height() -> f32 { 105.0 }

pub fn default_shape_size() -> f32 {
    48.0
}
pub fn default_shape_thickness() -> f32 {
    2.0
}
pub fn default_shape_color() -> [u8; 4] {
    [80, 160, 255, 255]
}
pub fn default_shape_fill_color() -> [u8; 4] {
    [80, 160, 255, 60]
}
pub fn default_shape_corner_radius() -> f32 {
    4.0
}

pub fn default_font_family() -> String {
    "Segoe UI".to_string()
}
pub fn default_font_size() -> f32 {
    13.0
}
pub fn default_white_color() -> [u8; 4] {
    [240, 242, 248, 255]
}
pub fn default_image_size() -> f32 {
    32.0
}
pub fn default_opacity_one() -> f32 {
    1.0
}
pub fn default_crosshair_size() -> f32 {
    14.0
}
pub fn default_crosshair_color() -> [u8; 4] {
    [0, 255, 136, 255]
}
pub fn default_crosshair_thickness() -> f32 {
    2.0
}
pub fn default_crosshair_gap() -> f32 {
    4.0
}
pub fn default_accent_color() -> [u8; 4] {
    [70, 160, 255, 255]
}
pub fn default_dark_color() -> [u8; 4] {
    [30, 32, 40, 220]
}
pub fn default_badge_bg() -> [u8; 4] {
    [70, 160, 255, 45]
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

// ---------------------------------------------------------------------------
// Strict JSON key checking
// ---------------------------------------------------------------------------

/// One key in a rule document that is not part of the rule model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownRuleKey {
    /// JSON path of the object carrying the key, for example `rule.actions[0]`.
    pub path: String,
    /// The key itself.
    pub key: String,
}

impl std::fmt::Display for UnknownRuleKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown key \"{}\" in {}", self.key, self.path)
    }
}

/// Rejects unknown keys in one rule object before it is deserialized.
///
/// Serde silently drops unknown keys, so a typo such as `max_execution` used to apply and
/// run with the default. This pass walks the raw JSON against the same key sets the types
/// declare and reports every unknown key with its path. Callers decide the policy:
/// `--rules-apply` and share import refuse the rule, the daemon load logs and skips it.
pub fn validate_rule_keys(value: &serde_json::Value) -> Result<(), Vec<UnknownRuleKey>> {
    let mut errors = Vec::new();
    check_rule_keys(value, "rule", &mut errors);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn check_keys(
    object: &serde_json::Map<String, serde_json::Value>,
    allowed: &[&str],
    path: &str,
    errors: &mut Vec<UnknownRuleKey>,
) {
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            errors.push(UnknownRuleKey {
                path: path.to_string(),
                key: key.clone(),
            });
        }
    }
}

fn variant_keys<'a>(
    tag: &str,
    table: &'a [(&'static str, &'static [&'static str])],
) -> Option<&'a [&'static str]> {
    table
        .iter()
        .find(|(name, _)| *name == tag)
        .map(|(_, keys)| *keys)
}

fn check_rule_keys(value: &serde_json::Value, path: &str, errors: &mut Vec<UnknownRuleKey>) {
    let Some(object) = value.as_object() else {
        return;
    };
    check_keys(object, RULE_KEYS, path, errors);
    if let Some(trigger) = object.get("trigger") {
        check_trigger_keys(trigger, &format!("{path}.trigger"), errors);
    }
    if let Some(conditions) = object.get("conditions") {
        check_condition_items(conditions, &format!("{path}.conditions"), errors);
    }
    for field in ["actions", "on_exit_actions"] {
        if let Some(actions) = object.get(field) {
            check_action_list(actions, &format!("{path}.{field}"), errors);
        }
    }
    if let Some(tags) = object.get("tags").and_then(serde_json::Value::as_array) {
        for (index, tag) in tags.iter().enumerate() {
            let Some(tag) = tag.as_object() else { continue };
            check_keys(tag, TAG_KEYS, &format!("{path}.tags[{index}]"), errors);
        }
    }
    if let Some(period) = object.get("quota_period") {
        check_quota_period(period, &format!("{path}.quota_period"), errors);
    }
}

fn check_quota_period(value: &serde_json::Value, path: &str, errors: &mut Vec<UnknownRuleKey>) {
    let Some(object) = value.as_object() else {
        return;
    };
    check_keys(object, QUOTA_PERIOD_KEYS, path, errors);
    if let Some(custom) = object.get("custom").and_then(serde_json::Value::as_object) {
        check_keys(custom, CUSTOM_QUOTA_KEYS, &format!("{path}.custom"), errors);
    }
}

fn check_trigger_keys(value: &serde_json::Value, path: &str, errors: &mut Vec<UnknownRuleKey>) {
    let Some(object) = value.as_object() else {
        return;
    };
    let Some(tag) = object.get("$trigger_type").and_then(serde_json::Value::as_str) else {
        return;
    };
    if let Some(allowed) = variant_keys(tag, TRIGGER_VARIANTS) {
        check_keys(object, allowed, path, errors);
    }
}

fn check_condition_items(value: &serde_json::Value, path: &str, errors: &mut Vec<UnknownRuleKey>) {
    let Some(items) = value.as_array() else {
        return;
    };
    for (index, item) in items.iter().enumerate() {
        let item_path = format!("{path}[{index}]");
        let Some(object) = item.as_object() else {
            continue;
        };
        if object.contains_key("condition") {
            check_keys(object, CONDITION_ITEM_KEYS, &item_path, errors);
            if let Some(condition) = object.get("condition") {
                check_condition_keys(condition, &format!("{item_path}.condition"), errors);
            }
        } else {
            check_condition_keys(item, &item_path, errors);
        }
    }
}

fn check_condition_keys(value: &serde_json::Value, path: &str, errors: &mut Vec<UnknownRuleKey>) {
    let Some(object) = value.as_object() else {
        return;
    };
    let Some(tag) = object.get("$condition_type").and_then(serde_json::Value::as_str) else {
        return;
    };
    if let Some(allowed) = variant_keys(tag, CONDITION_VARIANTS) {
        check_keys(object, allowed, path, errors);
    }
}

fn check_action_list(value: &serde_json::Value, path: &str, errors: &mut Vec<UnknownRuleKey>) {
    let Some(actions) = value.as_array() else {
        return;
    };
    for (index, action) in actions.iter().enumerate() {
        check_action(action, &format!("{path}[{index}]"), errors);
    }
}

fn check_action(value: &serde_json::Value, path: &str, errors: &mut Vec<UnknownRuleKey>) {
    let Some(object) = value.as_object() else {
        return;
    };
    let Some(tag) = object.get("$action_type").and_then(serde_json::Value::as_str) else {
        return;
    };
    if let Some(allowed) = variant_keys(tag, ACTION_VARIANTS) {
        check_keys(object, allowed, path, errors);
    }
    match tag {
        "if_else" => {
            if let Some(items) = object.get("conditions") {
                check_condition_items(items, &format!("{path}.conditions"), errors);
            }
            if let Some(items) = object.get("then_actions") {
                check_action_list(items, &format!("{path}.then_actions"), errors);
            }
            if let Some(branches) = object
                .get("else_if_branches")
                .and_then(serde_json::Value::as_array)
            {
                for (index, branch) in branches.iter().enumerate() {
                    let branch_path = format!("{path}.else_if_branches[{index}]");
                    let Some(branch) = branch.as_object() else {
                        continue;
                    };
                    check_keys(branch, ELSE_IF_KEYS, &branch_path, errors);
                    if let Some(items) = branch.get("conditions") {
                        check_condition_items(items, &format!("{branch_path}.conditions"), errors);
                    }
                    if let Some(items) = branch.get("actions") {
                        check_action_list(items, &format!("{branch_path}.actions"), errors);
                    }
                }
            }
            if let Some(items) = object.get("else_actions") {
                check_action_list(items, &format!("{path}.else_actions"), errors);
            }
        }
        "wait" => {
            if let Some(condition) = object.get("condition") {
                check_wait_condition(condition, &format!("{path}.condition"), errors);
            }
        }
        "check" => {
            if let Some(condition) = object.get("condition") {
                check_condition_keys(condition, &format!("{path}.condition"), errors);
            }
        }
        "macro" => {
            if let Some(events) = object.get("events").and_then(serde_json::Value::as_array) {
                for (index, event) in events.iter().enumerate() {
                    let event_path = format!("{path}.events[{index}]");
                    let Some(event) = event.as_object() else {
                        continue;
                    };
                    if let Some(tag) = event.get("type").and_then(serde_json::Value::as_str) {
                        if let Some(allowed) = variant_keys(tag, MACRO_EVENT_VARIANTS) {
                            check_keys(event, allowed, &event_path, errors);
                        }
                    }
                }
            }
        }
        "overlay" => {
            if let Some(overlay) = object.get("custom_overlay") {
                check_overlay_definition(overlay, &format!("{path}.custom_overlay"), errors);
            }
        }
        _ => {}
    }
}

fn check_wait_condition(value: &serde_json::Value, path: &str, errors: &mut Vec<UnknownRuleKey>) {
    let Some(object) = value.as_object() else {
        return;
    };
    let Some(tag) = object.get("type").and_then(serde_json::Value::as_str) else {
        return;
    };
    if let Some(allowed) = variant_keys(tag, WAIT_CONDITION_VARIANTS) {
        check_keys(object, allowed, path, errors);
    }
}

fn check_overlay_definition(value: &serde_json::Value, path: &str, errors: &mut Vec<UnknownRuleKey>) {
    let Some(object) = value.as_object() else {
        return;
    };
    check_keys(object, OVERLAY_DEFINITION_KEYS, path, errors);
    if let Some(rect) = object.get("rect").and_then(serde_json::Value::as_object) {
        check_keys(rect, OVERLAY_RECT_KEYS, &format!("{path}.rect"), errors);
    }
    if let Some(style) = object.get("style").and_then(serde_json::Value::as_object) {
        check_keys(style, OVERLAY_STYLE_KEYS, &format!("{path}.style"), errors);
    }
    if let Some(elements) = object.get("elements").and_then(serde_json::Value::as_array) {
        for (index, element) in elements.iter().enumerate() {
            let element_path = format!("{path}.elements[{index}]");
            let Some(element) = element.as_object() else {
                continue;
            };
            let Some(tag) = element.get("type").and_then(serde_json::Value::as_str) else {
                continue;
            };
            if let Some(allowed) = variant_keys(tag, OVERLAY_ELEMENT_VARIANTS) {
                check_keys(element, allowed, &element_path, errors);
            }
            if tag == "interactive_control" {
                if let Some(control) = element.get("control").and_then(serde_json::Value::as_object) {
                    let control_path = format!("{element_path}.control");
                    if let Some(kind) = control.get("type").and_then(serde_json::Value::as_str) {
                        if let Some(allowed) = variant_keys(kind, INTERACTIVE_CONTROL_VARIANTS) {
                            check_keys(control, allowed, &control_path, errors);
                        }
                    }
                    for action_field in ["on_enable", "on_disable", "action"] {
                        if let Some(action) = control.get(action_field) {
                            check_overlay_click_action(
                                action,
                                &format!("{control_path}.{action_field}"),
                                errors,
                            );
                        }
                    }
                }
            }
            for action_field in ["on_click", "on_finish"] {
                if let Some(action) = element.get(action_field) {
                    check_overlay_click_action(
                        action,
                        &format!("{element_path}.{action_field}"),
                        errors,
                    );
                }
            }
        }
    }
}

fn check_overlay_click_action(
    value: &serde_json::Value,
    path: &str,
    errors: &mut Vec<UnknownRuleKey>,
) {
    let Some(object) = value.as_object() else {
        return;
    };
    check_keys(object, OVERLAY_CLICK_ACTION_KEYS, path, errors);
    let Some(tag) = object.get("type").and_then(serde_json::Value::as_str) else {
        return;
    };
    let payload_keys: &[&str] = match tag {
        "trigger_rule" | "toggle_rule" => &["rule_id"],
        "kill_process" | "trim_memory" => &["process_name"],
        _ => return,
    };
    if let Some(payload) = object.get("payload").and_then(serde_json::Value::as_object) {
        check_keys(payload, payload_keys, &format!("{path}.payload"), errors);
    }
}

const RULE_KEYS: &[&str] = &[
    "id",
    "name",
    "description",
    "is_enabled",
    "cooldown_seconds",
    "max_executions",
    "quota_period",
    "execution_count",
    "stop_on_error",
    "trigger",
    "conditions",
    "actions",
    "on_exit_actions",
    "custom_icon_path",
    "auto_process_icon",
    "last_executed_at",
    "quota_window_start",
    "tags",
];

const QUOTA_PERIOD_KEYS: &[&str] = &["custom"];
const CUSTOM_QUOTA_KEYS: &[&str] = &["value", "unit"];
const TAG_KEYS: &[&str] = &["name", "color", "icon"];
const CONDITION_ITEM_KEYS: &[&str] = &["operator", "condition"];
const ELSE_IF_KEYS: &[&str] = &["conditions", "actions"];

const TRIGGER_VARIANTS: &[(&str, &[&str])] = &[
    ("manual", &["$trigger_type"]),
    (
        "process",
        &["$trigger_type", "process_name", "match_mode", "single_instance"],
    ),
    (
        "window_focus",
        &[
            "$trigger_type",
            "process_name",
            "on_focus_gained",
            "on_focus_lost",
        ],
    ),
    (
        "resource",
        &[
            "$trigger_type",
            "process_name",
            "cpu_percent",
            "ram_mb",
            "duration_sec",
        ],
    ),
    (
        "hotkey",
        &[
            "$trigger_type",
            "key_combination",
            "hold_mode",
            "repeat_while_held",
        ],
    ),
    (
        "system",
        &["$trigger_type", "event_type", "device_id_filter"],
    ),
    (
        "schedule",
        &[
            "$trigger_type",
            "schedule_type",
            "interval_sec",
            "daily_time",
            "days_of_week",
            "day_of_month",
            "target_datetime",
        ],
    ),
    (
        "file_watcher",
        &[
            "$trigger_type",
            "folder_path",
            "filter_extension",
            "on_created",
            "on_modified",
            "on_deleted",
        ],
    ),
    ("webhook", &["$trigger_type", "endpoint_path"]),
];

const CONDITION_VARIANTS: &[(&str, &[&str])] = &[
    (
        "process_running",
        &[
            "$condition_type",
            "process_name",
            "match_mode",
            "must_be_running",
        ],
    ),
    (
        "window_focused",
        &[
            "$condition_type",
            "process_name",
            "match_mode",
            "must_be_focused",
        ],
    ),
    (
        "time_range",
        &["$condition_type", "start_time", "end_time", "days_of_week"],
    ),
    (
        "event",
        &["$condition_type", "event_type", "idle_duration_sec"],
    ),
    (
        "resource",
        &["$condition_type", "process_name", "cpu_percent", "ram_mb"],
    ),
    (
        "file_watcher",
        &["$condition_type", "path", "filter_extension", "must_exist"],
    ),
    (
        "rule_running",
        &["$condition_type", "rule_id", "must_be_running"],
    ),
];

const ACTION_VARIANTS: &[(&str, &[&str])] = &[
    (
        "start_process",
        &[
            "$action_type",
            "file_path",
            "arguments",
            "working_directory",
            "run_as_admin",
            "only_if_not_running",
            "run_hidden",
            "close_on_exit",
        ],
    ),
    (
        "kill_process",
        &["$action_type", "process_name", "kill_tree", "force"],
    ),
    (
        "window",
        &[
            "$action_type",
            "target_process",
            "window_action",
            "x",
            "y",
            "width",
            "height",
            "transparency",
        ],
    ),
    (
        "process_tuning",
        &["$action_type", "target_process", "priority", "affinity_mask", "trim_memory"],
    ),
    ("power_plan", &["$action_type", "plan"]),
    (
        "play_sound",
        &["$action_type", "file_path", "play_beep", "beep_frequency", "beep_duration_ms"],
    ),
    ("firewall", &["$action_type", "process_name", "action"]),
    ("notification", &["$action_type", "title", "message"]),
    (
        "http_webhook",
        &["$action_type", "url", "method", "headers", "payload"],
    ),
    (
        "audio_control",
        &[
            "$action_type",
            "target",
            "process_name",
            "action_type",
            "volume_level",
            "left_volume",
            "right_volume",
        ],
    ),
    (
        "audio_format",
        &["$action_type", "target", "device_name", "sample_rate", "bit_depth", "channels"],
    ),
    ("default_device", &["$action_type", "target", "device_name"]),
    (
        "execute_script",
        &["$action_type", "interpreter", "script", "run_in_window"],
    ),
    (
        "macro",
        &["$action_type", "preset_id", "operation", "events", "repeat", "speed", "infinite"],
    ),
    ("system_power", &["$action_type", "power_type"]),
    (
        "wait",
        &["$action_type", "mode", "seconds", "condition", "timeout_sec", "timeout_policy"],
    ),
    ("check", &["$action_type", "condition"]),
    (
        "display",
        &["$action_type", "width", "height", "refresh_rate", "monitor_index"],
    ),
    (
        "if_else",
        &["$action_type", "conditions", "then_actions", "else_if_branches", "else_actions"],
    ),
    (
        "rule_control",
        &["$action_type", "rule_id", "operation", "wait"],
    ),
    (
        "trim_working_set",
        &["$action_type", "target_process", "soft_limit_mb", "skip_if_focused"],
    ),
    (
        "discord_rpc_set",
        &[
            "$action_type",
            "app_id",
            "details",
            "state",
            "large_image",
            "large_text",
            "small_image",
            "small_text",
            "show_elapsed",
            "buttons",
            "auto_clear_on_process_exit",
        ],
    ),
    ("discord_rpc_clear", &["$action_type"]),
    (
        "archive",
        &["$action_type", "operation", "format", "source_path", "destination_path", "compression_level", "overwrite"],
    ),
    (
        "file_system",
        &["$action_type", "operation", "path", "target_path", "content", "create_parents", "overwrite"],
    ),
    (
        "overlay",
        &["$action_type", "overlay_id", "custom_overlay", "operation", "override_text", "duration_ms", "close_on_exit"],
    ),
];

const WAIT_CONDITION_VARIANTS: &[(&str, &[&str])] = &[
    (
        "window_opened",
        &["type", "process_name", "match_mode"],
    ),
    (
        "window_closed",
        &["type", "process_name", "match_mode"],
    ),
    (
        "process_started",
        &["type", "process_name", "match_mode"],
    ),
    (
        "process_exited",
        &["type", "process_name", "match_mode"],
    ),
    ("hotkey", &["type", "key_combination"]),
    ("file", &["type", "path", "condition"]),
    ("rule_started", &["type", "rule_id"]),
    ("rule_stopped", &["type", "rule_id"]),
];

const MACRO_EVENT_VARIANTS: &[(&str, &[&str])] = &[
    ("key_down", &["type", "key"]),
    ("key_up", &["type", "key"]),
    ("key_press", &["type", "key"]),
    ("text", &["type", "text", "delay_ms", "organic"]),
    (
        "mouse_move",
        &["type", "x", "y", "coord_mode", "smooth", "duration_ms", "organic"],
    ),
    ("mouse_down", &["type", "button"]),
    ("mouse_up", &["type", "button"]),
    (
        "mouse_click",
        &["type", "button", "clicks", "x", "y", "coord_mode", "smooth"],
    ),
    ("mouse_wheel", &["type", "delta"]),
    ("delay", &["type", "ms", "organic", "jitter_percent"]),
];

const OVERLAY_DEFINITION_KEYS: &[&str] = &[
    "id",
    "name",
    "rect",
    "anchor",
    "click_through",
    "draggable",
    "opacity",
    "style",
    "elements",
    "auto_hide_seconds",
    "target_process",
    "hide_from_capture",
    "fade_in_ms",
    "fade_out_ms",
    "monitor_index",
    "close_with_target",
];
const OVERLAY_RECT_KEYS: &[&str] = &["x", "y", "width", "height"];
const OVERLAY_STYLE_KEYS: &[&str] = &[
    "bg_color",
    "border_color",
    "border_width",
    "corner_radius",
    "glow",
];
const OVERLAY_CLICK_ACTION_KEYS: &[&str] = &["type", "payload"];

const OVERLAY_ELEMENT_VARIANTS: &[(&str, &[&str])] = &[
    (
        "text",
        &["type", "id", "text", "font_size", "color", "x", "y", "width", "height", "lock_aspect", "bold", "font_family", "rotation", "on_click"],
    ),
    (
        "image",
        &["type", "id", "path", "x", "y", "width", "height", "opacity", "lock_aspect", "rotation", "on_click"],
    ),
    (
        "crosshair",
        &["type", "id", "style", "size", "color", "center_dot", "thickness", "gap", "x", "y", "rotation"],
    ),
    (
        "shape",
        &["type", "id", "shape_type", "x", "y", "width", "height", "thickness", "color", "filled", "fill_color", "corner_radius", "lock_aspect", "rotation", "on_click"],
    ),
    (
        "graph",
        &["type", "id", "metric", "style", "sample_count", "color", "fill_color", "bg_color", "show_current_val", "x", "y", "width", "height", "rotation"],
    ),
    (
        "clock_timer",
        &["type", "id", "mode", "format_str", "countdown_total_ms", "countdown_remaining_ms", "stopwatch_elapsed_ms", "is_running", "font_size", "color", "bg_color", "bold", "font_family", "x", "y", "width", "height", "rotation", "on_finish"],
    ),
    (
        "interactive_control",
        &["type", "id", "control", "color", "bg_color", "active_color", "x", "y", "width", "height", "rotation"],
    ),
    (
        "system_telemetry",
        &["type", "id", "kind", "style", "color", "bg_color", "x", "y", "width", "height", "rotation", "on_click"],
    ),
    (
        "media_widget",
        &["type", "id", "show_controls", "compact", "color", "bg_color", "x", "y", "width", "height", "rotation"],
    ),
    (
        "key_visualizer",
        &["type", "id", "layout", "max_keys", "decay_seconds", "show_modifiers_only", "color", "bg_color", "active_color", "x", "y", "width", "height", "rotation"],
    ),
];

const INTERACTIVE_CONTROL_VARIANTS: &[(&str, &[&str])] = &[
    ("button", &["type", "label", "icon", "action"]),
    (
        "toggle_switch",
        &["type", "label", "is_active", "bind_rule_id", "on_enable", "on_disable"],
    ),
    ("slider", &["type", "label", "value", "min", "max", "bind_target"]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rule_tag_serde_and_color_parsing() {
        let tag = RuleTag::new("Productivity")
            .with_color("#10B981")
            .with_icon("⚡");

        let json = serde_json::to_string(&tag).unwrap();
        let decoded: RuleTag = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.name, "Productivity");
        assert_eq!(decoded.color.as_deref(), Some("#10B981"));
        assert_eq!(decoded.icon.as_deref(), Some("⚡"));

        let color = decoded.parse_color(egui::Color32::WHITE);
        assert_eq!(color, egui::Color32::from_rgb(0x10, 0xB9, 0x81));

        let invalid_color_tag = RuleTag::new("Bad").with_color("not_a_color");
        let fallback = egui::Color32::from_rgb(1, 2, 3);
        assert_eq!(invalid_color_tag.parse_color(fallback), fallback);
    }

    #[test]
    fn test_generate_clone_name() {
        assert_eq!(generate_clone_name("Discord Mute"), "Discord Mute (Copy)");
        assert_eq!(generate_clone_name("Discord Mute (Copy)"), "Discord Mute (Copy 2)");
        assert_eq!(generate_clone_name("Discord Mute (Copy 2)"), "Discord Mute (Copy 3)");
        assert_eq!(generate_clone_name("Discord Mute (Copy 99)"), "Discord Mute (Copy 100)");
    }

    #[test]
    fn test_rule_clone_with_new_id() {
        let original = Rule {
            id: "rule_orig_123".into(),
            name: "Game Optimizer".into(),
            description: "Some memo".into(),
            is_enabled: true,
            cooldown_seconds: 1.5,
            max_executions: Some(10),
            quota_period: Some(QuotaPeriod::Day),
            execution_count: 42,
            stop_on_error: true,
            trigger: Some(Trigger::Process {
                process_name: "game.exe".into(),
                match_mode: ProcessMatchMode::Exact,
                single_instance: true,
            }),
            actions: vec![Action::Notification {
                title: "Alert".into(),
                message: "Starting".into(),
            }],
            on_exit_actions: vec![],
            custom_icon_path: Some("C:\\game.exe".into()),
            auto_process_icon: true,
            conditions: vec![],
            last_executed_at: None,
            quota_window_start: Some("2026-11-19 08:00:00".into()),
            tags: vec![RuleTag::new("Gaming").with_color("#3B82F6").with_icon("🎮")],
        };

        let cloned = original.clone_with_new_id();
        assert_ne!(cloned.id, original.id);
        assert_eq!(cloned.name, "Game Optimizer (Copy)");
        assert_eq!(cloned.description, "Some memo");
        assert_eq!(cloned.tags, original.tags);
        assert!(!cloned.is_enabled); // Paused on clone
        assert_eq!(cloned.cooldown_seconds, 1.5);
        assert_eq!(cloned.max_executions, Some(10));
        assert_eq!(cloned.quota_period, Some(QuotaPeriod::Day));
        assert_eq!(cloned.trigger, original.trigger);
        assert_eq!(cloned.actions, original.actions);
        assert_eq!(cloned.execution_count, 0); // reset to 0
        assert!(cloned.quota_window_start.is_none()); // runtime anchor reset
        assert_eq!(cloned.custom_icon_path, Some("C:\\game.exe".into()));
        assert!(cloned.auto_process_icon);
        assert!(cloned.stop_on_error);
    }

    #[test]
    fn test_action_duplicate_with_fresh_ids() {
        let overlay_act = Action::Overlay {
            overlay_id: "overlay_initial_123".to_string(),
            custom_overlay: Some(Box::new(OverlayDefinition {
                id: "overlay_initial_123".to_string(),
                ..Default::default()
            })),
            operation: OverlayOperation::Toggle,
            override_text: None,
            duration_ms: None,
            close_on_exit: false,
        };

        let duplicated = overlay_act.duplicate_with_fresh_ids();
        if let Action::Overlay { overlay_id, custom_overlay, .. } = duplicated {
            assert_ne!(overlay_id, "overlay_initial_123");
            assert_eq!(custom_overlay.as_ref().map(|o| o.id.as_str()), Some(overlay_id.as_str()));
        } else {
            panic!("Expected Action::Overlay");
        }
    }

    #[test]
    fn test_new_actions_and_conditions_serde() {
        let rule = Rule {
            id: "rule_advanced".into(),
            name: "Industrial Workstation Setup".into(),
            description: "Applies windows, tuning and scripts".into(),
            is_enabled: true,
            cooldown_seconds: 1.0,
            max_executions: Some(5),
            quota_period: Some(QuotaPeriod::Week),
            execution_count: 2,
            stop_on_error: true,
            trigger: Some(Trigger::Webhook {
                endpoint_path: "studio_start".into(),
            }),
            actions: vec![
                Action::Window {
                    target_process: Some("Code.exe".into()),
                    window_action: WindowActionType::Maximize,
                    x: None,
                    y: None,
                    width: None,
                    height: None,
                    transparency: Some(240),
                },
                Action::ProcessTuning {
                    target_process: Some("Code.exe".into()),
                    priority: Some(ProcessPriorityLevel::High),
                    affinity_mask: Some(0xFF),
                    trim_memory: true,
                },
                Action::Macro {
                    preset_id: None,
                    operation: MacroOperation::Run,
                    events: vec![
                        MacroEvent::KeyDown { key: "Shift".into() },
                        MacroEvent::KeyPress { key: "P".into() },
                        MacroEvent::KeyUp { key: "Shift".into() },
                    ],
                    repeat: 1,
                    speed: 1.0,
                    infinite: false,
                },
                Action::PowerPlan {
                    plan: PowerPlanType::HighPerformance,
                },
                Action::PlaySound {
                    file_path: "C:\\Windows\\Media\\chimes.wav".into(),
                    play_beep: false,
                    beep_frequency: None,
                    beep_duration_ms: None,
                },
                Action::ExecuteScript {
                    interpreter: ScriptType::Lua,
                    script: "ph.log('info', 'Workstation setup complete!')".into(),
                    run_in_window: false,
                },
                Action::Firewall {
                    process_name: "telemetry.exe".into(),
                    action: FirewallAction::Block,
                },
                Action::Display {
                    width: Some(1920),
                    height: Some(1080),
                    refresh_rate: Some(144),
                    monitor_index: Some(1),
                },
                Action::DefaultDevice {
                    target: AudioTarget::Playback,
                    device_name: "Headphones".into(),
                },
                Action::DefaultDevice {
                    target: AudioTarget::Microphone,
                    device_name: "Microphone".into(),
                },
                Action::IfElse {
                    conditions: vec![RuleConditionItem::new(RuleCondition::ProcessRunning {
                        process_name: "game.exe".into(),
                        match_mode: ProcessMatchMode::Exact,
                        must_be_running: true,
                    })],
                    then_actions: vec![Action::Wait {
                        mode: WaitMode::Timer,
                        seconds: 1.0,
                        condition: None,
                        timeout_sec: 30.0,
                        timeout_policy: WaitTimeoutPolicy::Abort,
                    }],
                    else_if_branches: vec![ElseIfBranch {
                        conditions: vec![RuleConditionItem::new(RuleCondition::Event {
                            event_type: SystemEventType::PowerSourceAc,
                            idle_duration_sec: 300,
                        })],
                        actions: vec![Action::Wait {
                            mode: WaitMode::Timer,
                            seconds: 2.0,
                            condition: None,
                            timeout_sec: 30.0,
                            timeout_policy: WaitTimeoutPolicy::Abort,
                        }],
                    }],
                    else_actions: vec![],
                },
            ],
            on_exit_actions: vec![],
            custom_icon_path: None,
            auto_process_icon: true,
            conditions: vec![
                RuleConditionItem::new(RuleCondition::WindowFocused {
                    process_name: "game.exe".into(),
                    match_mode: ProcessMatchMode::Exact,
                    must_be_focused: true,
                }),
                RuleConditionItem::with_operator(
                    ConditionOperator::Or,
                    RuleCondition::FileWatcher {
                        path: "C:\\log.txt".into(),
                        filter_extension: None,
                        must_exist: true,
                    },
                ),
            ],
            tags: vec![],
            last_executed_at: None,
            quota_window_start: None,
        };

        let json = serde_json::to_string(&rule).expect("Serialization must succeed");
        let decoded: Rule = serde_json::from_str(&json).expect("Deserialization must succeed");

        assert_eq!(decoded.id, rule.id);
        assert_eq!(decoded.max_executions, Some(5));
        assert_eq!(decoded.quota_period, Some(QuotaPeriod::Week));
        assert_eq!(decoded.execution_count, 2);
        assert!(decoded.stop_on_error);
        assert_eq!(decoded.actions.len(), 11);
        assert_eq!(decoded.conditions.len(), 2);
        assert_eq!(decoded, rule);
    }

    #[test]
    fn test_unified_actions_serde() {
        let unified_actions = vec![
            Action::AudioControl {
                target: AudioTarget::Playback,
                process_name: None,
                action_type: AudioVolumeAction::SetVolume,
                volume_level: 75,
                left_volume: Some(70),
                right_volume: Some(80),
            },
            Action::AudioFormat {
                target: AudioTarget::Playback,
                device_name: None,
                sample_rate: 48000,
                bit_depth: 24,
                channels: 2,
            },
            Action::DefaultDevice {
                target: AudioTarget::Microphone,
                device_name: "USB Mic".into(),
            },
            Action::ExecuteScript {
                interpreter: ScriptType::PowerShell,
                script: "Get-Process".into(),
                run_in_window: true,
            },
            Action::Macro {
                preset_id: None,
                operation: MacroOperation::Run,
                events: vec![
                    MacroEvent::KeyDown { key: "Shift".into() },
                    MacroEvent::KeyPress { key: "A".into() },
                    MacroEvent::KeyUp { key: "Shift".into() },
                    MacroEvent::Text { text: "Hello".into(), delay_ms: 20, organic: false },
                    MacroEvent::MouseMove {
                        x: 100,
                        y: 200,
                        smooth: true,
                        duration_ms: Some(150),
                        organic: true,
                        coord_mode: MouseCoordMode::Absolute,
                    },
                    MacroEvent::MouseClick {
                        button: MouseButton::Left,
                        clicks: 2,
                        x: None,
                        y: None,
                        coord_mode: MouseCoordMode::Absolute,
                        smooth: false,
                    },
                    MacroEvent::Delay {
                        ms: 250,
                        organic: true,
                        jitter_percent: 30,
                    },
                ],
                repeat: 3,
                speed: 1.5,
                infinite: false,
            },
        ];

        let json = serde_json::to_string(&unified_actions).expect("Must serialize");
        let decoded: Vec<Action> = serde_json::from_str(&json).expect("Must deserialize");
        assert_eq!(decoded, unified_actions);
    }

    #[test]
    fn test_new_v23_actions_serde() {
        let actions = vec![
            Action::TrimWorkingSet {
                target_process: Some("game.exe".to_string()),
                soft_limit_mb: Some(256),
                skip_if_focused: true,
            },
            Action::DiscordRpcSet {
                app_id: "123456789".to_string(),
                details: "Playing Game".to_string(),
                state: "In Mission".to_string(),
                large_image: Some("logo".to_string()),
                large_text: Some("Game Logo".to_string()),
                small_image: Some("status".to_string()),
                small_text: Some("Online".to_string()),
                show_elapsed: true,
                buttons: vec![DiscordRpcButton {
                    label: "Join".to_string(),
                    url: "https://example.com".to_string(),
                }],
                auto_clear_on_process_exit: true,
            },
            Action::DiscordRpcClear,
            Action::Archive {
                operation: ArchiveOperation::Pack,
                format: ArchiveFormat::Zip,
                source_path: "C:\\source".to_string(),
                destination_path: "C:\\out.zip".to_string(),
                compression_level: Some(6),
                overwrite: true,
            },
            Action::FileSystem {
                operation: FileSystemOperation::CreateFile,
                path: "C:\\file.txt".to_string(),
                target_path: None,
                content: Some("text".to_string()),
                create_parents: true,
                overwrite: true,
            },
        ];

        let json = serde_json::to_string(&actions).expect("Must serialize");
        let decoded: Vec<Action> = serde_json::from_str(&json).expect("Must deserialize");
        assert_eq!(decoded, actions);
    }

    #[test]
    fn test_trim_working_set_serde_options_and_backcompat() {
        // Backwards compatibility: legacy JSON without soft_limit_mb and skip_if_focused
        let legacy_json = r#"{"$action_type":"trim_working_set","target_process":"notepad.exe"}"#;
        let action: Action = serde_json::from_str(legacy_json).expect("Must deserialize legacy format");
        assert_eq!(
            action,
            Action::TrimWorkingSet {
                target_process: Some("notepad.exe".to_string()),
                soft_limit_mb: None,
                skip_if_focused: false,
            }
        );

        // Serialization with None soft_limit_mb omits it
        let serialized = serde_json::to_string(&action).expect("Must serialize");
        assert!(!serialized.contains("soft_limit_mb"));
        assert!(serialized.contains(r#""skip_if_focused":false"#));

        // Full roundtrip with soft_limit_mb and skip_if_focused: true
        let full_action = Action::TrimWorkingSet {
            target_process: Some("game.exe".to_string()),
            soft_limit_mb: Some(512),
            skip_if_focused: true,
        };
        let full_json = serde_json::to_string(&full_action).expect("Must serialize");
        assert!(full_json.contains(r#""soft_limit_mb":512"#));
        assert!(full_json.contains(r#""skip_if_focused":true"#));
        let full_decoded: Action = serde_json::from_str(&full_json).expect("Must deserialize");
        assert_eq!(full_decoded, full_action);
    }

    #[test]
    fn test_manual_trigger_serde() {
        let trigger = Trigger::Manual;
        let serialized = serde_json::to_string(&trigger).unwrap();
        assert_eq!(serialized, r#"{"$trigger_type":"manual"}"#);
        let deserialized: Trigger = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, Trigger::Manual);
    }

    #[test]
    fn test_global_hotkey_hold_mode_and_repeat_serde() {
        // Test optional field default: json without repeat_while_held defaults to false
        let json = r#"{"$trigger_type":"hotkey","key_combination":"Ctrl+Shift+H","hold_mode":true}"#;
        let trigger: Trigger = serde_json::from_str(json).unwrap();
        assert_eq!(
            trigger,
            Trigger::GlobalHotkey {
                key_combination: "Ctrl+Shift+H".to_string(),
                hold_mode: true,
                repeat_while_held: false,
            }
        );

        // Test roundtrip with repeat_while_held: true
        let trigger_loop = Trigger::GlobalHotkey {
            key_combination: "M4".to_string(),
            hold_mode: true,
            repeat_while_held: true,
        };
        let serialized = serde_json::to_string(&trigger_loop).unwrap();
        let deserialized: Trigger = serde_json::from_str(&serialized).unwrap();
        assert_eq!(trigger_loop, deserialized);
    }

    #[test]
    fn test_precondition_resource_and_event_serde() {
        // 1. ResourceThreshold roundtrip
        let res_cond = RuleCondition::ResourceThreshold {
            process_name: "game.exe".to_string(),
            cpu_percent: Some(75.5),
            ram_mb: Some(2048),
        };
        let serialized = serde_json::to_string(&res_cond).unwrap();
        let deserialized: RuleCondition = serde_json::from_str(&serialized).unwrap();
        assert_eq!(res_cond, deserialized);

        // 2. Event condition roundtrip
        let event_cond = RuleCondition::Event {
            event_type: SystemEventType::SessionLock,
            idle_duration_sec: 300,
        };
        let serialized = serde_json::to_string(&event_cond).unwrap();
        let deserialized: RuleCondition = serde_json::from_str(&serialized).unwrap();
        assert_eq!(event_cond, deserialized);

        // 3. FileWatcher condition roundtrip
        let file_cond = RuleCondition::FileWatcher {
            path: "C:\\test.log".to_string(),
            filter_extension: Some("log".to_string()),
            must_exist: true,
        };
        let serialized = serde_json::to_string(&file_cond).unwrap();
        let deserialized: RuleCondition = serde_json::from_str(&serialized).unwrap();
        assert_eq!(file_cond, deserialized);
    }

    #[test]
    fn test_rule_activity_condition_and_wait_serde() {
        let condition = RuleCondition::RuleRunning {
            rule_id: "rule_game_mode".to_string(),
            must_be_running: false,
        };
        let json = serde_json::to_string(&condition).unwrap();
        assert!(json.contains("\"$condition_type\":\"rule_running\""), "{json}");
        let decoded: RuleCondition = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, condition);

        // Omitted must_be_running defaults to true, matching the other state conditions.
        let defaulted: RuleCondition =
            serde_json::from_str(r#"{"$condition_type":"rule_running","rule_id":"rule_a"}"#).unwrap();
        assert_eq!(
            defaulted,
            RuleCondition::RuleRunning {
                rule_id: "rule_a".to_string(),
                must_be_running: true,
            }
        );

        for wait in [
            WaitCondition::RuleStarted {
                rule_id: "rule_game_mode".into(),
            },
            WaitCondition::RuleStopped {
                rule_id: "rule_game_mode".into(),
            },
        ] {
            let action = Action::Wait {
                mode: WaitMode::UntilCondition,
                seconds: 0.0,
                condition: Some(wait),
                timeout_sec: 30.0,
                timeout_policy: WaitTimeoutPolicy::Abort,
            };
            let json = serde_json::to_string(&action).unwrap();
            let decoded: Action = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, action);
        }
    }

    #[test]
    fn test_overlay_models_and_action_serde() {
        let action = Action::Overlay {
            overlay_id: "overlay_crosshair".to_string(),
            custom_overlay: None,
            operation: OverlayOperation::Toggle,
            override_text: Some("Aim Active".to_string()),
            duration_ms: Some(3000),
            close_on_exit: true,
        };
        let json = serde_json::to_string(&action).expect("Serialize Action::Overlay");
        let decoded: Action = serde_json::from_str(&json).expect("Deserialize Action::Overlay");
        assert_eq!(decoded, action);

        // Test with custom embedded overlay
        let custom_action = Action::Overlay {
            overlay_id: "custom_rule_hud".to_string(),
            custom_overlay: Some(Box::new(OverlayDefinition::default_crosshair())),
            operation: OverlayOperation::Show,
            override_text: None,
            duration_ms: None,
            close_on_exit: false,
        };
        let custom_json = serde_json::to_string(&custom_action).unwrap();
        let custom_decoded: Action = serde_json::from_str(&custom_json).unwrap();
        assert_eq!(custom_decoded, custom_action);

        let mut overlay = OverlayDefinition::default_crosshair();
        overlay.monitor_index = Some(1);
        overlay.close_with_target = true;
        let json_ov = serde_json::to_string(&overlay).expect("Serialize OverlayDefinition");
        let decoded_ov: OverlayDefinition = serde_json::from_str(&json_ov).expect("Deserialize OverlayDefinition");
        assert_eq!(decoded_ov, overlay);
        assert_eq!(decoded_ov.monitor_index, Some(1));
        assert!(decoded_ov.close_with_target);

        // Test MonitorInfo orientation
        let mon_landscape = MonitorInfo {
            index: 0,
            name: "Display 1".to_string(),
            x: 0,
            y: 0,
            width: 2560,
            height: 1440,
            is_primary: true,
        };
        assert_eq!(mon_landscape.orientation(), MonitorOrientation::Landscape);
        assert!(!mon_landscape.is_portrait());

        let mon_portrait = MonitorInfo {
            index: 1,
            name: "Display 2".to_string(),
            x: -1080,
            y: 0,
            width: 1080,
            height: 1920,
            is_primary: false,
        };
        assert_eq!(mon_portrait.orientation(), MonitorOrientation::Portrait);
        assert!(mon_portrait.is_portrait());

        let macro_ov = OverlayDefinition::default_macro_badge();
        let json_macro = serde_json::to_string(&macro_ov).unwrap();
        let decoded_macro: OverlayDefinition = serde_json::from_str(&json_macro).unwrap();
        assert_eq!(decoded_macro, macro_ov);

        // Test Image element defaults: lock_aspect defaults to true
        let raw_img_json = r#"{"type": "image", "path": "test.png"}"#;
        let img_elem: OverlayElement = serde_json::from_str(raw_img_json).unwrap();
        if let OverlayElement::Image { lock_aspect, .. } = img_elem {
            assert!(lock_aspect, "lock_aspect must default to true");
        } else {
            panic!("Expected Image element");
        }

        // Test fade_in_ms and fade_out_ms serde
        let mut fade_overlay = OverlayDefinition::default_crosshair();
        fade_overlay.fade_in_ms = Some(250);
        fade_overlay.fade_out_ms = Some(350);
        let fade_json = serde_json::to_string(&fade_overlay).unwrap();
        assert!(fade_json.contains("\"fade_in_ms\":250"));
        assert!(fade_json.contains("\"fade_out_ms\":350"));
        let decoded_fade: OverlayDefinition = serde_json::from_str(&fade_json).unwrap();
        assert_eq!(decoded_fade.fade_in_ms, Some(250));
        assert_eq!(decoded_fade.fade_out_ms, Some(350));

        // When omitted, both must default to None and click_through to false
        let raw_no_fade = r#"{"id":"test","name":"Test"}"#;
        let decoded_no_fade: OverlayDefinition = serde_json::from_str(raw_no_fade).unwrap();
        assert_eq!(decoded_no_fade.fade_in_ms, None);
        assert_eq!(decoded_no_fade.fade_out_ms, None);
        assert!(!decoded_no_fade.click_through, "click_through must default to false");
        assert!(!OverlayDefinition::default_new_overlay().click_through, "new overlay click_through must be false by default");
    }

    #[test]
    fn test_overlay_interactive_and_telemetry_elements() {
        let elements = vec![
            OverlayElement::Graph {
                id: "g1".to_string(),
                metric: GraphMetric::SystemCpu,
                style: GraphStyle::AreaFilled,
                sample_count: 50,
                color: [0, 255, 128, 255],
                fill_color: [0, 255, 128, 60],
                bg_color: [20, 20, 30, 200],
                show_current_val: true,
                x: 10.0,
                y: 20.0,
                width: 140.0,
                height: 60.0,
                rotation: 0.0,
            },
            OverlayElement::ClockTimer {
                id: "c1".to_string(),
                mode: TimerMode::Countdown,
                format_str: "%H:%M".to_string(),
                countdown_total_ms: 30000,
                countdown_remaining_ms: 25000,
                stopwatch_elapsed_ms: 0,
                is_running: true,
                font_size: 16.0,
                color: [255, 255, 255, 255],
                bg_color: [0, 0, 0, 180],
                bold: true,
                font_family: "Segoe UI".to_string(),
                x: 15.0,
                y: 25.0,
                width: 100.0,
                height: 30.0,
                rotation: 45.0,
                on_finish: Some(OverlayClickAction::CloseOverlay),
            },
            OverlayElement::InteractiveControl {
                id: "ctrl1".to_string(),
                control: InteractiveControlKind::Button {
                    label: "Mute".to_string(),
                    icon: None,
                    action: Some(OverlayClickAction::ToggleMute),
                },
                color: [255, 255, 255, 255],
                bg_color: [40, 45, 55, 220],
                active_color: [0, 200, 100, 255],
                x: 5.0,
                y: 5.0,
                width: 80.0,
                height: 28.0,
                rotation: 0.0,
            },
            OverlayElement::SystemTelemetry {
                id: "tel1".to_string(),
                kind: TelemetryKind::Battery,
                style: TelemetryStyle::Badge,
                color: [0, 220, 130, 255],
                bg_color: [25, 30, 40, 220],
                x: 50.0,
                y: 60.0,
                width: 90.0,
                height: 24.0,
                rotation: 0.0,
                on_click: None,
            },
            OverlayElement::MediaWidget {
                id: "m1".to_string(),
                show_controls: true,
                compact: false,
                color: [240, 240, 250, 255],
                bg_color: [20, 24, 32, 200],
                x: 0.0,
                y: 0.0,
                width: 180.0,
                height: 52.0,
                rotation: 0.0,
            },
            OverlayElement::KeyVisualizer {
                id: "k1".to_string(),
                layout: KeyVisualizerLayout::WasdMouse,
                max_keys: 5,
                decay_seconds: 2.0,
                show_modifiers_only: true,
                color: [255, 255, 255, 255],
                bg_color: [35, 40, 50, 220],
                active_color: [0, 180, 255, 255],
                x: 100.0,
                y: 100.0,
                width: 170.0,
                height: 105.0,
                rotation: 0.0,
            },
        ];

        for mut elem in elements {
            assert!(!elem.id().is_empty());
            assert_eq!(elem.x(), elem.x());
            assert_eq!(elem.y(), elem.y());
            *elem.id_mut() = format!("{}_cloned", elem.id());
            assert!(elem.id().ends_with("_cloned"));

            let json = serde_json::to_string(&elem).expect("Serialize element");
            let decoded: OverlayElement = serde_json::from_str(&json).expect("Deserialize element");
            assert_eq!(decoded.id(), elem.id());
            assert_eq!(decoded.x(), elem.x());
            assert_eq!(decoded.y(), elem.y());
            assert_eq!(decoded.rotation(), elem.rotation());
        }
    }

    #[test]
    fn test_key_visualizer_layouts_and_backcompat() {
        let layouts = [
            KeyVisualizerLayout::WasdMouse,
            KeyVisualizerLayout::WasdOnly,
            KeyVisualizerLayout::ActionRpg,
            KeyVisualizerLayout::MouseOnly,
            KeyVisualizerLayout::TextStream,
        ];
        for layout in layouts {
            let elem = OverlayElement::KeyVisualizer {
                id: "k".into(),
                layout,
                max_keys: 4,
                decay_seconds: 1.5,
                show_modifiers_only: false,
                color: [255, 255, 255, 255],
                bg_color: [0, 0, 0, 200],
                active_color: [0, 200, 255, 255],
                x: 10.0,
                y: 10.0,
                width: 170.0,
                height: 105.0,
                rotation: 0.0,
            };
            let json = serde_json::to_string(&elem).unwrap();
            let decoded: OverlayElement = serde_json::from_str(&json).unwrap();
            if let OverlayElement::KeyVisualizer { layout: dec_layout, .. } = decoded {
                assert_eq!(dec_layout, layout);
            } else {
                panic!("Expected KeyVisualizer");
            }
        }

        // Backward compatibility: legacy JSON without layout and active_color
        let legacy_json = r#"{"type":"key_visualizer","id":"legacy","max_keys":3,"decay_seconds":1.0,"show_modifiers_only":false,"color":[255,255,255,255],"bg_color":[0,0,0,200],"x":0.0,"y":0.0,"width":160.0,"height":32.0,"rotation":0.0}"#;
        let decoded: OverlayElement = serde_json::from_str(legacy_json).unwrap();
        if let OverlayElement::KeyVisualizer { layout, active_color, .. } = decoded {
            assert_eq!(layout, KeyVisualizerLayout::WasdMouse);
            assert_eq!(active_color, default_accent_color());
        } else {
            panic!("Expected KeyVisualizer");
        }
    }

    #[test]
    fn test_wait_check_toggle_serde() {
        let wait_timer = Action::Wait {
            mode: WaitMode::Timer,
            seconds: 3.5,
            condition: None,
            timeout_sec: 10.0,
            timeout_policy: WaitTimeoutPolicy::Continue,
        };
        let json_timer = serde_json::to_string(&wait_timer).unwrap();
        let dec_timer: Action = serde_json::from_str(&json_timer).unwrap();
        assert_eq!(dec_timer, wait_timer);

        let wait_until = Action::Wait {
            mode: WaitMode::UntilCondition,
            seconds: 0.0,
            condition: Some(WaitCondition::WindowOpened {
                process_name: "notepad.exe".into(),
                match_mode: ProcessMatchMode::Exact,
            }),
            timeout_sec: 45.0,
            timeout_policy: WaitTimeoutPolicy::Abort,
        };
        let json_until = serde_json::to_string(&wait_until).unwrap();
        let dec_until: Action = serde_json::from_str(&json_until).unwrap();
        assert_eq!(dec_until, wait_until);

        let check_act = Action::Check {
            condition: RuleCondition::ProcessRunning {
                process_name: "game.exe".into(),
                match_mode: ProcessMatchMode::Exact,
                must_be_running: true,
            },
        };
        let json_check = serde_json::to_string(&check_act).unwrap();
        let dec_check: Action = serde_json::from_str(&json_check).unwrap();
        assert_eq!(dec_check, check_act);

        let fw_toggle = Action::Firewall {
            process_name: "app.exe".into(),
            action: FirewallAction::Toggle,
        };
        let json_fw = serde_json::to_string(&fw_toggle).unwrap();
        let dec_fw: Action = serde_json::from_str(&json_fw).unwrap();
        assert_eq!(dec_fw, fw_toggle);

        let win_min_restore = Action::Window {
            target_process: Some("app.exe".into()),
            window_action: WindowActionType::ToggleMinimizeRestore,
            x: None,
            y: None,
            width: None,
            height: None,
            transparency: None,
        };
        let json_wmr = serde_json::to_string(&win_min_restore).unwrap();
        let dec_wmr: Action = serde_json::from_str(&json_wmr).unwrap();
        assert_eq!(dec_wmr, win_min_restore);

        let win_show_hide = Action::Window {
            target_process: Some("app.exe".into()),
            window_action: WindowActionType::ToggleShowHide,
            x: None,
            y: None,
            width: None,
            height: None,
            transparency: None,
        };
        let json_wsh = serde_json::to_string(&win_show_hide).unwrap();
        let dec_wsh: Action = serde_json::from_str(&json_wsh).unwrap();
        assert_eq!(dec_wsh, win_show_hide);
    }

    #[test]
    fn test_macro_presets_and_operations_serde() {
        let preset = MacroPreset {
            id: "preset_1".into(),
            name: "Test Macro".into(),
            events: vec![
                MacroEvent::KeyDown { key: "A".into() },
                MacroEvent::KeyUp { key: "A".into() },
            ],
            repeat: 3,
            speed: 1.5,
            infinite: false,
        };
        let preset_json = serde_json::to_string(&preset).unwrap();
        let dec_preset: MacroPreset = serde_json::from_str(&preset_json).unwrap();
        assert_eq!(dec_preset.id, "preset_1");
        assert_eq!(dec_preset.name, "Test Macro");
        assert_eq!(dec_preset.events.len(), 2);
        assert_eq!(dec_preset.repeat, 3);
        assert_eq!(dec_preset.speed, 1.5);
        assert!(!dec_preset.infinite);

        let macro_run_action = Action::Macro {
            events: vec![],
            repeat: 1,
            speed: 1.0,
            infinite: false,
            preset_id: Some("preset_1".into()),
            operation: MacroOperation::Run,
        };
        let json_run = serde_json::to_string(&macro_run_action).unwrap();
        let dec_run: Action = serde_json::from_str(&json_run).unwrap();
        assert_eq!(dec_run, macro_run_action);

        let macro_stop_action = Action::Macro {
            events: vec![],
            repeat: 1,
            speed: 1.0,
            infinite: false,
            preset_id: Some("preset_1".into()),
            operation: MacroOperation::Stop,
        };
        let json_stop = serde_json::to_string(&macro_stop_action).unwrap();
        let dec_stop: Action = serde_json::from_str(&json_stop).unwrap();
        assert_eq!(dec_stop, macro_stop_action);

        let macro_toggle_action = Action::Macro {
            events: vec![],
            repeat: 1,
            speed: 1.0,
            infinite: false,
            preset_id: Some("preset_1".into()),
            operation: MacroOperation::Toggle,
        };
        let json_toggle = serde_json::to_string(&macro_toggle_action).unwrap();
        let dec_toggle: Action = serde_json::from_str(&json_toggle).unwrap();
        assert_eq!(dec_toggle, macro_toggle_action);

        let overlay_set_text = Action::Overlay {
            overlay_id: "overlay_1".into(),
            operation: OverlayOperation::SetText,
            custom_overlay: None,
            override_text: Some("Dynamic notification text".into()),
            duration_ms: None,
            close_on_exit: false,
        };
        let json_set_text = serde_json::to_string(&overlay_set_text).unwrap();
        let dec_set_text: Action = serde_json::from_str(&json_set_text).unwrap();
        assert_eq!(dec_set_text, overlay_set_text);

        // Verify refresh_overlay_ids preserves preset reference (custom_overlay == None)
        let mut cloned_action = overlay_set_text.clone();
        cloned_action.refresh_overlay_ids();
        assert_eq!(cloned_action, overlay_set_text);
    }

    fn local_at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(year, month, day, hour, minute, 0)
            .single()
            .expect("valid local time")
    }

    #[test]
    fn test_calendar_quota_periods_roll_over() {
        // Minute: 14:59:xx belongs to the previous bucket, 15:00 starts a new one.
        let mut minutely = Rule {
            max_executions: Some(2),
            quota_period: Some(QuotaPeriod::Minute),
            execution_count: 2,
            last_executed_at: Some("2026-11-19 14:59:10".into()),
            ..Default::default()
        };
        assert!(minutely.quota_exhausted(local_at(2026, 11, 19, 14, 59)));
        assert_eq!(
            minutely.quota_next_reset(local_at(2026, 11, 19, 14, 59)),
            Some(local_at(2026, 11, 19, 15, 0))
        );
        assert!(!minutely.quota_exhausted(local_at(2026, 11, 19, 15, 0)));
        assert!(minutely.normalize_quota(local_at(2026, 11, 19, 15, 0)));
        assert_eq!(minutely.execution_count, 0);

        // Hour: 14:59 belongs to the previous bucket, 15:00 starts a new one.
        let mut hourly = Rule {
            max_executions: Some(2),
            quota_period: Some(QuotaPeriod::Hour),
            execution_count: 2,
            last_executed_at: Some("2026-11-19 14:10:00".into()),
            ..Default::default()
        };
        assert!(hourly.quota_exhausted(local_at(2026, 11, 19, 14, 59)));
        assert!(!hourly.quota_exhausted(local_at(2026, 11, 19, 15, 0)));
        assert!(hourly.normalize_quota(local_at(2026, 11, 19, 15, 0)));
        assert_eq!(hourly.execution_count, 0);

        // Day: local midnight is the boundary.
        let mut daily = Rule {
            max_executions: Some(3),
            quota_period: Some(QuotaPeriod::Day),
            execution_count: 3,
            last_executed_at: Some("2026-11-18 22:00:00".into()),
            ..Default::default()
        };
        assert!(daily.quota_exhausted(local_at(2026, 11, 18, 23, 59)));
        assert_eq!(daily.quota_used(local_at(2026, 11, 19, 0, 1)), 0);
        assert!(daily.normalize_quota(local_at(2026, 11, 19, 0, 1)));
        // The last-run stamp stays: daily and monthly schedules deduplicate on it.
        assert_eq!(daily.last_executed_at.as_deref(), Some("2026-11-18 22:00:00"));

        // Week: Monday 00:00 starts a new ISO week (2026-11-16 is a Monday).
        let weekly = Rule {
            max_executions: Some(1),
            quota_period: Some(QuotaPeriod::Week),
            execution_count: 1,
            last_executed_at: Some("2026-11-16 09:00:00".into()),
            ..Default::default()
        };
        assert!(weekly.quota_exhausted(local_at(2026, 11, 22, 23, 0)));
        assert!(!weekly.quota_exhausted(local_at(2026, 11, 23, 0, 1)));

        // Month: the 1st at 00:00 is the boundary.
        let monthly = Rule {
            max_executions: Some(1),
            quota_period: Some(QuotaPeriod::Month),
            execution_count: 1,
            last_executed_at: Some("2026-11-30 23:00:00".into()),
            ..Default::default()
        };
        assert!(monthly.quota_exhausted(local_at(2026, 11, 30, 23, 59)));
        assert!(!monthly.quota_exhausted(local_at(2026, 12, 1, 0, 0)));
    }

    #[test]
    fn test_custom_quota_window_anchors_at_first_execution() {
        let mut rule = Rule {
            max_executions: Some(2),
            quota_period: Some(QuotaPeriod::Custom { value: 90, unit: QuotaUnit::Minutes }),
            ..Default::default()
        };
        let start = local_at(2026, 11, 19, 10, 0);
        assert!(rule.begin_quota_window(start));
        assert_eq!(rule.quota_window_start.as_deref(), Some("2026-11-19 10:00:00"));
        rule.execution_count = 2;
        rule.last_executed_at = Some("2026-11-19 10:30:00".into());

        assert!(rule.quota_exhausted(local_at(2026, 11, 19, 11, 0)));
        assert_eq!(
            rule.quota_next_reset(local_at(2026, 11, 19, 11, 0)),
            Some(local_at(2026, 11, 19, 11, 30))
        );
        assert!(rule.quota_exhausted(local_at(2026, 11, 19, 11, 29)));
        // The window lapses exactly on its end, and the counter rolls over lazily.
        assert!(!rule.quota_exhausted(local_at(2026, 11, 19, 11, 30)));
        assert!(rule.normalize_quota(local_at(2026, 11, 19, 11, 30)));
        assert_eq!(rule.execution_count, 0);
        assert!(rule.quota_window_start.is_none());

        // The next execution opens a fresh window instead of inheriting the old one.
        let next = local_at(2026, 11, 19, 13, 0);
        assert!(rule.begin_quota_window(next));
        assert_eq!(rule.quota_window_start.as_deref(), Some("2026-11-19 13:00:00"));
        assert_eq!(rule.quota_remaining(next), Some(2));
    }

    #[test]
    fn test_custom_quota_window_supports_calendar_months() {
        let mut rule = Rule {
            max_executions: Some(1),
            quota_period: Some(QuotaPeriod::Custom { value: 1, unit: QuotaUnit::Months }),
            ..Default::default()
        };
        let start = local_at(2026, 11, 19, 10, 0);
        assert!(rule.begin_quota_window(start));
        rule.execution_count = 1;
        rule.last_executed_at = Some("2026-11-19 10:05:00".into());

        // One calendar month from the window start, not a fixed 30-day span.
        assert_eq!(
            rule.quota_next_reset(local_at(2026, 12, 1, 0, 0)),
            Some(local_at(2026, 12, 19, 10, 0))
        );
        assert!(rule.quota_exhausted(local_at(2026, 12, 18, 23, 0)));
        assert!(!rule.quota_exhausted(local_at(2026, 12, 19, 10, 0)));

        // Month-end clamping: Jan 31 + 1 month lands on the last day of February.
        let mut clamped = Rule {
            quota_period: Some(QuotaPeriod::Custom { value: 1, unit: QuotaUnit::Months }),
            ..Default::default()
        };
        assert!(clamped.begin_quota_window(local_at(2027, 1, 31, 9, 0)));
        assert_eq!(
            clamped.quota_next_reset(local_at(2027, 2, 1, 0, 0)),
            Some(local_at(2027, 2, 28, 9, 0))
        );
    }

    #[test]
    fn test_lifetime_quota_never_resets() {
        let mut rule = Rule {
            max_executions: Some(1),
            quota_period: None,
            execution_count: 1,
            last_executed_at: Some("2020-01-01 00:00:00".into()),
            ..Default::default()
        };
        assert!(rule.quota_exhausted(local_at(2026, 11, 19, 12, 0)));
        assert!(!rule.normalize_quota(local_at(2026, 11, 19, 12, 0)));
        assert_eq!(rule.execution_count, 1);
        assert_eq!(rule.quota_remaining(local_at(2026, 11, 19, 12, 0)), Some(0));
    }

    #[test]
    fn test_quota_period_serde() {
        // Rules without the field keep the historical lifetime quota.
        let legacy: Rule = serde_json::from_str(r#"{"id":"r","name":"R","max_executions":5}"#).unwrap();
        assert_eq!(legacy.quota_period, None);
        assert_eq!(legacy.quota_window_start, None);

        let custom = Rule {
            id: "r".into(),
            name: "R".into(),
            max_executions: Some(2),
            quota_period: Some(QuotaPeriod::Custom { value: 2, unit: QuotaUnit::Days }),
            ..Default::default()
        };
        let json = serde_json::to_string(&custom).unwrap();
        assert!(
            json.contains(r#""quota_period":{"custom":{"value":2,"unit":"days"}}"#),
            "{json}"
        );
        // The custom-window anchor is runtime state, never part of the definition.
        assert!(!json.contains("quota_window_start"));
        let decoded: Rule = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.quota_period, custom.quota_period);

        // Calendar variants serialize as bare snake_case strings.
        let daily: Rule = serde_json::from_str(r#"{"id":"r","name":"R","quota_period":"day"}"#).unwrap();
        assert_eq!(daily.quota_period, Some(QuotaPeriod::Day));
        // A custom window without a unit defaults to minutes.
        let minutes: Rule =
            serde_json::from_str(r#"{"id":"r","name":"R","quota_period":{"custom":{"value":30}}}"#).unwrap();
        assert_eq!(
            minutes.quota_period,
            Some(QuotaPeriod::Custom { value: 30, unit: QuotaUnit::Minutes })
        );
    }

    #[test]
    fn test_duration_from_secs_rejects_invalid_values() {
        assert_eq!(
            duration_from_secs(1.5).expect("valid").as_millis(),
            1_500
        );
        for invalid in [-1.0, f64::NAN, f64::INFINITY, MAX_WAIT_SECONDS + 1.0, 1e300] {
            assert!(
                duration_from_secs(invalid).is_err(),
                "{invalid} must be rejected"
            );
        }
    }

    #[test]
    fn test_custom_quota_window_rejects_invalid_and_lapses_on_overflow() {
        for value in [0, QuotaPeriod::MAX_CUSTOM_WINDOW_UNITS + 1] {
            assert!(
                QuotaPeriod::Custom { value, unit: QuotaUnit::Minutes }
                    .validation_error()
                    .is_some(),
                "value {value} must be rejected"
            );
        }
        assert!(QuotaPeriod::Custom { value: 1, unit: QuotaUnit::Minutes }
            .validation_error()
            .is_none());
        assert!(QuotaPeriod::Day.validation_error().is_none());

        // An unrepresentable window end must read as a lapsed window, never as "still counting".
        let extreme = Local
            .with_ymd_and_hms(262_140, 1, 1, 0, 0, 0)
            .single()
            .expect("chrono supports this year");
        assert!(QuotaPeriod::Custom {
            value: QuotaPeriod::MAX_CUSTOM_WINDOW_UNITS,
            unit: QuotaUnit::Months,
        }
        .custom_window_end(extreme)
        .is_none());

        // A corrupt anchor is likewise not a current window.
        let mut rule = Rule {
            max_executions: Some(1),
            quota_period: Some(QuotaPeriod::Custom {
                value: 1,
                unit: QuotaUnit::Days,
            }),
            execution_count: 1,
            quota_window_start: Some("not a timestamp".into()),
            ..Default::default()
        };
        let now = local_at(2026, 11, 19, 12, 0);
        assert_eq!(rule.quota_used(now), 0);
        assert!(!rule.quota_exhausted(now));
        assert!(rule.normalize_quota(now));
    }

    #[test]
    fn test_parse_hex_color_rejects_non_ascii_tag_colour() {
        assert_eq!(parse_hex_color("#10B981"), Some([0x10, 0xB9, 0x81]));
        assert_eq!(parse_hex_color("10b981"), Some([0x10, 0xB9, 0x81]));
        for invalid in ["a\u{e9}\u{20ac}", "12345", "#12345G", "1234567"] {
            assert_eq!(parse_hex_color(invalid), None, "{invalid}");
        }

        let fallback = egui::Color32::from_rgb(1, 2, 3);
        let broken = RuleTag::new("Broken").with_color("a\u{e9}\u{20ac}");
        assert_eq!(broken.parse_color(fallback), fallback);

        let mut rule = Rule {
            tags: vec![broken],
            ..Default::default()
        };
        assert!(rule.tag_color_error().is_some());
        rule.tags = vec![RuleTag::new("Good").with_color("#10B981")];
        assert!(rule.tag_color_error().is_none());
    }

    #[test]
    fn test_quota_redefinition_helpers() {
        let lifetime = Rule::default();
        let mut defined = lifetime.clone();
        defined.max_executions = Some(5);
        assert!(defined.quota_redefined_from(&lifetime));

        // Changing only the numeric limit keeps the running counter.
        let mut same = defined.clone();
        same.max_executions = Some(9);
        assert!(!same.quota_redefined_from(&defined));

        let mut periodic = same.clone();
        periodic.quota_period = Some(QuotaPeriod::Day);
        assert!(periodic.quota_redefined_from(&same));

        let mut progress = defined.clone();
        progress.execution_count = 4;
        progress.last_executed_at = Some("2026-11-19 10:00:00".into());
        progress.quota_window_start = Some("2026-11-19 09:00:00".into());
        progress.reset_quota_progress();
        assert_eq!(progress.execution_count, 0);
        assert!(progress.quota_window_start.is_none());
        // Schedules deduplicate on this stamp, so it must survive a quota reset.
        assert_eq!(
            progress.last_executed_at.as_deref(),
            Some("2026-11-19 10:00:00")
        );
    }

    #[test]
    fn test_overlay_ids_normalize_across_load_and_nesting() {
        let mut action = Action::Overlay {
            overlay_id: "action_id".into(),
            custom_overlay: Some(Box::new(OverlayDefinition {
                id: "definition_id".into(),
                ..Default::default()
            })),
            operation: OverlayOperation::Show,
            override_text: None,
            duration_ms: None,
            close_on_exit: true,
        };
        action.normalize_overlay_ids();
        if let Action::Overlay {
            overlay_id,
            custom_overlay,
            ..
        } = &action
        {
            assert_eq!(overlay_id, "action_id");
            assert_eq!(
                custom_overlay.as_ref().map(|definition| definition.id.as_str()),
                Some("action_id")
            );
        } else {
            panic!("expected an overlay action");
        }

        // An empty action id adopts the embedded definition id instead.
        let mut empty = Action::Overlay {
            overlay_id: String::new(),
            custom_overlay: Some(Box::new(OverlayDefinition {
                id: "embedded".into(),
                ..Default::default()
            })),
            operation: OverlayOperation::Show,
            override_text: None,
            duration_ms: None,
            close_on_exit: false,
        };
        empty.normalize_overlay_ids();
        if let Action::Overlay { overlay_id, .. } = &empty {
            assert_eq!(overlay_id, "embedded");
        }

        let mut nested = Rule {
            actions: vec![Action::IfElse {
                conditions: Vec::new(),
                then_actions: vec![action],
                else_if_branches: Vec::new(),
                else_actions: Vec::new(),
            }],
            ..Default::default()
        };
        nested.normalize();
        if let Action::IfElse { then_actions, .. } = &nested.actions[0] {
            if let Action::Overlay { custom_overlay, .. } = &then_actions[0] {
                assert_eq!(
                    custom_overlay.as_ref().map(|definition| definition.id.as_str()),
                    Some("action_id")
                );
            } else {
                panic!("expected a nested overlay action");
            }
        } else {
            panic!("expected an if_else action");
        }
    }

    #[test]
    fn test_validate_rule_keys_rejects_typos() {
        let rule = Rule {
            id: "r".into(),
            name: "R".into(),
            actions: vec![Action::StartProcess {
                file_path: "app.exe".into(),
                arguments: String::new(),
                working_directory: None,
                run_as_admin: false,
                only_if_not_running: false,
                run_hidden: false,
                close_on_exit: false,
            }],
            tags: vec![RuleTag::new("tag")],
            ..Default::default()
        };
        let mut value = serde_json::to_value(&rule).expect("serialize");
        assert!(validate_rule_keys(&value).is_ok());

        value["max_execution"] = serde_json::json!(5);
        value["actions"][0]["max_execution"] = serde_json::json!(5);
        value["tags"][0]["colour"] = serde_json::json!("#fff");
        let errors = validate_rule_keys(&value).expect_err("typos must be reported");
        let rendered = errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ");
        assert!(rendered.contains("max_execution"), "{rendered}");
        assert!(rendered.contains("actions[0]"), "{rendered}");
        assert!(rendered.contains("tags[0]"), "{rendered}");
    }

    #[test]
    fn test_schedule_per_type_validation() {
        let schedule = |schedule_type: ScheduleType,
                        interval_sec: u64,
                        daily_time: Option<&str>,
                        days_of_week: Vec<u8>,
                        day_of_month: Option<u8>,
                        target_datetime: Option<&str>| Trigger::Schedule {
            schedule_type,
            interval_sec,
            daily_time: daily_time.map(str::to_string),
            days_of_week,
            day_of_month,
            target_datetime: target_datetime.map(str::to_string),
        };

        assert!(schedule(ScheduleType::Daily, 0, None, vec![], None, None)
            .schedule_errors()
            .iter()
            .any(|error| error.contains("daily_time")));
        assert!(!schedule(ScheduleType::Daily, 0, Some("9:00"), vec![], None, None)
            .schedule_errors()
            .is_empty());
        assert!(schedule(ScheduleType::Daily, 0, Some("09:00"), vec![], None, None)
            .schedule_errors()
            .is_empty());
        assert!(schedule(ScheduleType::Daily, 0, Some("09:00:00"), vec![], None, None)
            .schedule_errors()
            .is_empty());
        assert!(!schedule(ScheduleType::Interval, 0, None, vec![], None, None)
            .schedule_errors()
            .is_empty());
        assert!(!schedule(ScheduleType::OneShot, 0, None, vec![], None, None)
            .schedule_errors()
            .is_empty());
        assert!(schedule(
            ScheduleType::OneShot,
            0,
            None,
            vec![],
            None,
            Some("2026-12-31 23:59")
        )
        .schedule_errors()
        .is_empty());
        assert!(!schedule(ScheduleType::Monthly, 0, Some("12:00:00"), vec![], None, None)
            .schedule_errors()
            .is_empty());
        assert!(schedule(
            ScheduleType::Monthly,
            0,
            Some("12:00:00"),
            vec![],
            Some(1),
            None
        )
        .schedule_errors()
        .is_empty());
        assert!(!schedule(ScheduleType::Daily, 0, Some("09:00"), vec![0], None, None)
            .schedule_errors()
            .is_empty());
    }

    #[test]
    fn test_time_of_day_and_day_validators() {
        assert!(is_valid_hh_mm("00:00"));
        assert!(is_valid_hh_mm("23:59"));
        assert!(!is_valid_hh_mm("9:00"));
        assert!(!is_valid_hh_mm("24:00"));
        assert!(!is_valid_hh_mm("23:60"));
        assert!(!is_valid_hh_mm("23:5"));
        assert!(is_valid_hh_mm_ss("23:59:59"));
        assert!(!is_valid_hh_mm_ss("23:59:60"));
        assert!(days_of_week_are_valid(&[1, 7]));
        assert!(!days_of_week_are_valid(&[0]));
        assert!(!days_of_week_are_valid(&[8]));
        assert!(is_valid_day_of_month(31));
        assert!(!is_valid_day_of_month(0));
        assert!(!is_valid_day_of_month(32));
    }

    #[test]
    fn test_resource_threshold_and_wait_validation() {
        assert_eq!(resource_threshold_errors("", None, None).len(), 2);
        assert_eq!(resource_threshold_errors("game.exe", None, None).len(), 1);
        assert!(resource_threshold_errors("game.exe", Some(80.0), None).is_empty());

        let missing_condition = Action::Wait {
            mode: WaitMode::UntilCondition,
            seconds: 0.0,
            condition: None,
            timeout_sec: 30.0,
            timeout_policy: WaitTimeoutPolicy::Abort,
        };
        assert!(missing_condition
            .validation_errors()
            .iter()
            .any(|error| error.contains("needs a condition")));

        let negative_timer = Action::Wait {
            mode: WaitMode::Timer,
            seconds: -5.0,
            condition: None,
            timeout_sec: 30.0,
            timeout_policy: WaitTimeoutPolicy::Abort,
        };
        assert!(!negative_timer.validation_errors().is_empty());

        let empty_wait = WaitCondition::ProcessStarted {
            process_name: String::new(),
            match_mode: ProcessMatchMode::Exact,
        };
        assert!(!empty_wait.validation_errors().is_empty());

        let bad_range = RuleCondition::TimeRange {
            start_time: "09:00".into(),
            end_time: "25:00".into(),
            days_of_week: vec![1, 2],
        };
        assert!(!bad_range.validation_errors().is_empty());
    }
}
