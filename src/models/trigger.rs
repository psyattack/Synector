use serde::{Deserialize, Serialize};

use super::serde_defaults::*;
use super::validation::{
    days_of_week_are_valid, is_valid_day_of_month, is_valid_hh_mm_ss, is_valid_target_datetime,
};

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
        /// Which lifecycle event of the target fires the rule. The same pipeline runs for
        /// either choice.
        #[serde(default)]
        event: ProcessTriggerEvent,
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

/// Which lifecycle event of the target process fires a `process` trigger.
#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProcessTriggerEvent {
    #[default]
    Started,
    Exited,
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
