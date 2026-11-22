use crate::gui::components::text::fill;
use crate::i18n::Strings;
use crate::models::{
    Action, ArchiveFormat, PowerPlanType, RuleCondition, SystemEventType, SystemPowerType, Trigger,
    WindowActionType,
};

pub(super) fn trigger_sort_key(trig: &Option<crate::models::Trigger>) -> usize {
    match trig {
        Some(crate::models::Trigger::Process { .. }) => 0,
        Some(crate::models::Trigger::WindowFocus { .. }) => 1,
        Some(crate::models::Trigger::GlobalHotkey { .. }) => 2,
        Some(crate::models::Trigger::Schedule { .. }) => 3,
        Some(crate::models::Trigger::Webhook { .. }) => 4,
        Some(crate::models::Trigger::FileWatcher { .. }) => 5,
        Some(crate::models::Trigger::ResourceThreshold { .. }) => 6,
        Some(crate::models::Trigger::SystemEvent { .. }) => 7,
        Some(crate::models::Trigger::Manual) | None => 8,
    }
}

/// Localized label of a system event. Every surface that used to `{:?}`-format the enum
/// (condition tooltip and badge, trigger summary) goes through this table.
fn sys_event_label(event: SystemEventType, loc: &Strings) -> &'static str {
    match event {
        SystemEventType::SessionLock => loc.inspector_sys_event_lock,
        SystemEventType::SessionUnlock => loc.inspector_sys_event_unlock,
        SystemEventType::SystemSleep => loc.inspector_sys_event_sleep,
        SystemEventType::SystemResume => loc.inspector_sys_event_resume,
        SystemEventType::PowerSourceAc => loc.inspector_sys_event_ac,
        SystemEventType::PowerSourceBattery => loc.inspector_sys_event_battery,
        SystemEventType::UserIdle => loc.inspector_sys_event_idle,
        SystemEventType::UserActive => loc.inspector_sys_event_active,
        SystemEventType::DeviceConnected => loc.inspector_sys_event_dev_connected,
        SystemEventType::DeviceDisconnected => loc.inspector_sys_event_dev_disconnected,
        SystemEventType::MonitorOn => loc.inspector_sys_event_mon_on,
        SystemEventType::MonitorOff => loc.inspector_sys_event_mon_off,
        SystemEventType::NetworkConnected => loc.inspector_sys_event_net_connected,
        SystemEventType::NetworkDisconnected => loc.inspector_sys_event_net_disconnected,
    }
}

/// Localized label of a system-power action target (`ПИТАНИЕ Выключение`, not `POWER Shutdown`).
fn power_label(power_type: SystemPowerType, loc: &Strings) -> &'static str {
    match power_type {
        SystemPowerType::Lock => loc.inspector_power_lock,
        SystemPowerType::Sleep => loc.inspector_power_sleep,
        SystemPowerType::TurnScreenOff => loc.inspector_power_screen_off,
        SystemPowerType::Restart => loc.inspector_power_restart,
        SystemPowerType::Shutdown => loc.inspector_power_shutdown,
    }
}

/// Localized label of a power plan; the catalog labels already carry the meaning, so the
/// old ` POWER` suffix is gone.
fn power_plan_label(plan: PowerPlanType, loc: &Strings) -> &'static str {
    match plan {
        PowerPlanType::Balanced => loc.inspector_power_plan_balanced,
        PowerPlanType::HighPerformance => loc.inspector_power_plan_high,
        PowerPlanType::PowerSaver => loc.inspector_power_plan_saver,
    }
}

/// Localized label of an archive format; `TarXz` has no inspector cell, so it uses the
/// dedicated chip label instead of its debug name.
fn archive_format_label(format: ArchiveFormat, loc: &Strings) -> &'static str {
    match format {
        ArchiveFormat::Zip => loc.inspector_archive_fmt_zip,
        ArchiveFormat::Tar => loc.inspector_archive_fmt_tar,
        ArchiveFormat::TarGz => loc.inspector_archive_fmt_tar_gz,
        ArchiveFormat::TarXz => loc.rules_brief_archive_tar_xz,
        ArchiveFormat::SevenZip => loc.inspector_archive_fmt_7z,
        ArchiveFormat::Rar => loc.inspector_archive_fmt_rar,
    }
}

/// One chip label per window action, including the two toggle templates that carry the
/// target process. No `WindowActionType` debug name can reach a chip.
fn window_action_label(action: WindowActionType, loc: &Strings) -> &'static str {
    match action {
        WindowActionType::Minimize => loc.rules_brief_window_minimize,
        WindowActionType::Maximize => loc.rules_brief_window_maximize,
        WindowActionType::Restore => loc.rules_brief_window_restore,
        WindowActionType::Hide => loc.rules_brief_window_hide,
        WindowActionType::Show => loc.rules_brief_window_show,
        WindowActionType::Close => loc.rules_brief_window_close,
        WindowActionType::ToggleAlwaysOnTop => loc.rules_brief_window_toggle_on_top,
        WindowActionType::AlwaysOnTop => loc.rules_brief_window_on_top,
        WindowActionType::RemoveAlwaysOnTop => loc.rules_brief_window_remove_on_top,
        WindowActionType::SetTransparency => loc.rules_brief_window_transparency,
        WindowActionType::SnapToPosition => loc.rules_brief_window_snap,
        WindowActionType::ToggleMinimizeRestore => loc.rules_brief_window_toggle_min_restore,
        WindowActionType::ToggleShowHide => loc.rules_brief_window_toggle_show_hide,
    }
}

pub(super) fn format_single_condition_brief(cond: &RuleCondition, loc: &Strings) -> String {
    match cond {
        RuleCondition::ProcessRunning { process_name, must_be_running, .. } => {
            let name = if process_name.is_empty() { loc.rules_trigger_proc } else { process_name.as_str() };
            let prefix = loc.rules_brief_cond_run;
            if *must_be_running { format!("{prefix}{name}") } else { format!("!{prefix}{name}") }
        }
        RuleCondition::WindowFocused { process_name, must_be_focused, .. } => {
            let name = if process_name.is_empty() { loc.rules_trigger_window } else { process_name.as_str() };
            let prefix = loc.rules_brief_cond_focus;
            if *must_be_focused { format!("{prefix}{name}") } else { format!("!{prefix}{name}") }
        }
        RuleCondition::TimeRange { start_time, end_time, .. } => {
            format!("{}-{}", start_time, end_time)
        }
        RuleCondition::Event { event_type, .. } => {
            loc.rules_badge_cond_event.replace("{}", sys_event_label(*event_type, loc))
        }
        RuleCondition::FileWatcher { path, must_exist, .. } => {
            let fname = std::path::Path::new(path)
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| {
                    if path.is_empty() { loc.rules_brief_cond_any_file.to_string() } else { path.clone() }
                });
            let prefix = loc.rules_brief_cond_file;
            if *must_exist { format!("{prefix}{fname}") } else { format!("!{prefix}{fname}") }
        }
        RuleCondition::ResourceThreshold { process_name, cpu_percent, ram_mb, .. } => {
            let name = if process_name.is_empty() { loc.rules_brief_cond_any_res } else { process_name.as_str() };
            let mut limits = Vec::new();
            if let Some(c) = cpu_percent {
                limits.push(loc.rules_brief_cond_cpu.replace("{}", &format!("{c:.0}")));
            }
            if let Some(m) = ram_mb {
                limits.push(loc.rules_brief_cond_ram.replace("{}", &m.to_string()));
            }
            let prefix = loc.rules_brief_cond_res;
            if limits.is_empty() {
                format!("{prefix}{name}")
            } else {
                format!("{prefix}{name}({})", limits.join(","))
            }
        }
        RuleCondition::RuleRunning { rule_id, must_be_running } => {
            let name = if rule_id.is_empty() { loc.rules_brief_cond_any_rule } else { rule_id.as_str() };
            let prefix = loc.rules_brief_cond_rule;
            if *must_be_running { format!("{prefix}{name}") } else { format!("!{prefix}{name}") }
        }
    }
}

pub(super) fn format_single_condition_tooltip(cond: &RuleCondition, idx: usize, loc: &crate::i18n::Strings) -> String {
    let desc = match cond {
        RuleCondition::ProcessRunning { process_name, must_be_running, .. } => {
            let p = if process_name.is_empty() { "?" } else { process_name.as_str() };
            if *must_be_running {
                loc.rules_tip_cond_process_running.replace("{}", p)
            } else {
                loc.rules_tip_cond_process_not_running.replace("{}", p)
            }
        }
        RuleCondition::WindowFocused { process_name, must_be_focused, .. } => {
            let p = if process_name.is_empty() { "?" } else { process_name.as_str() };
            if *must_be_focused {
                loc.rules_tip_cond_window_focused.replace("{}", p)
            } else {
                loc.rules_tip_cond_window_not_focused.replace("{}", p)
            }
        }
        RuleCondition::TimeRange { start_time, end_time, .. } => {
            let filled = loc.rules_tip_cond_time_range.replacen("{}", start_time, 1);
            filled.replacen("{}", end_time, 1)
        }
        RuleCondition::Event { event_type, .. } => {
            loc.rules_tip_cond_event.replace("{}", sys_event_label(*event_type, loc))
        }
        RuleCondition::FileWatcher { path, must_exist, .. } => {
            if *must_exist {
                loc.rules_tip_cond_file_exists.replace("{}", path)
            } else {
                loc.rules_tip_cond_file_missing.replace("{}", path)
            }
        }
        RuleCondition::ResourceThreshold { process_name, cpu_percent, ram_mb, .. } => {
            let p = if process_name.is_empty() { "?" } else { process_name.as_str() };
            fill(
                loc.rules_tip_cond_resource,
                &[p, &format!("{cpu_percent:?}"), &format!("{ram_mb:?}")],
            )
        }
        RuleCondition::RuleRunning { rule_id, must_be_running } => {
            let p = if rule_id.is_empty() { "?" } else { rule_id.as_str() };
            if *must_be_running {
                loc.rules_tip_cond_rule_running.replace("{}", p)
            } else {
                loc.rules_tip_cond_rule_not_running.replace("{}", p)
            }
        }
    };
    format!("{} [{}] {}", loc.inspector_conditions_badge, idx, desc)
}

pub(super) fn format_action_brief(action: &Action, loc: &crate::i18n::Strings) -> String {
    match action {
        Action::StartProcess { file_path, .. } => {
            let fname = std::path::Path::new(file_path).file_name().unwrap_or_default().to_string_lossy();
            format!("{} {}", loc.summary_action_launch, fname)
        }
        Action::KillProcess { process_name, .. } => {
            format!("{} {}", loc.summary_action_kill, process_name)
        }
        Action::AudioControl { target, action_type, volume_level, process_name, left_volume, right_volume } => {
            let target_str = match target {
                crate::models::AudioTarget::Playback => loc.rules_brief_audio_sound.to_string(),
                crate::models::AudioTarget::Microphone => loc.rules_brief_audio_mic.to_string(),
                crate::models::AudioTarget::Process => {
                    loc.rules_brief_audio_proc.replace("{}", process_name.as_deref().unwrap_or("?"))
                }
            };
            match action_type {
                crate::models::AudioVolumeAction::Mute => format!("{} {}", loc.summary_action_mute, target_str),
                crate::models::AudioVolumeAction::Unmute => format!("{} {}", loc.summary_action_unmute, target_str),
                crate::models::AudioVolumeAction::ToggleMute => format!("{} {}", loc.summary_action_toggle, target_str),
                crate::models::AudioVolumeAction::SetVolume => {
                    if let (Some(l), Some(r)) = (left_volume, right_volume) {
                        fill(loc.rules_brief_audio_balance, &[&l.to_string(), &r.to_string(), &target_str])
                    } else {
                        format!("{} {}% {}", loc.summary_action_vol, volume_level, target_str)
                    }
                }
                crate::models::AudioVolumeAction::PlayBeep => loc.rules_brief_audio_beep.to_string(),
            }
        }
        Action::AudioFormat { target, sample_rate, bit_depth, .. } => {
            let t_str = match target {
                crate::models::AudioTarget::Playback | crate::models::AudioTarget::Process => loc.rules_brief_audio_out,
                crate::models::AudioTarget::Microphone => loc.rules_brief_audio_in,
            };
            fill(
                loc.rules_brief_audio_format,
                &[t_str, &sample_rate.to_string(), &bit_depth.to_string()],
            )
        }
        Action::DefaultDevice { target, device_name } => {
            let pfx = match target {
                crate::models::AudioTarget::Playback => loc.rules_brief_audio_dev_output,
                crate::models::AudioTarget::Microphone => loc.rules_brief_audio_dev_mic,
                crate::models::AudioTarget::Process => loc.rules_brief_audio_dev_process,
            };
            format!("{} {}", pfx, device_name)
        }
        Action::ExecuteScript { interpreter, .. } => match interpreter {
            crate::models::ScriptType::PowerShell => loc.inspector_script_type_powershell.to_string(),
            crate::models::ScriptType::Batch => loc.inspector_script_type_cmd.to_string(),
            crate::models::ScriptType::Lua => loc.rules_brief_script_lua.to_string(),
        },
        Action::Macro { events, repeat, speed, infinite, preset_id, operation } => {
            let op_prefix = match operation {
                crate::models::MacroOperation::Run => loc.rules_brief_macro_run,
                crate::models::MacroOperation::Stop => loc.rules_brief_macro_stop,
                crate::models::MacroOperation::Toggle => loc.rules_brief_macro_toggle,
            };
            if *operation == crate::models::MacroOperation::Stop {
                if let Some(pid) = preset_id {
                    loc.rules_brief_macro_stop_preset.replace("{}", pid)
                } else {
                    loc.rules_brief_macro_stop_all.to_string()
                }
            } else if let Some(pid) = preset_id {
                let rep_str = if *infinite {
                    " ∞".to_string()
                } else if *repeat > 1 {
                    format!(" {}x", repeat)
                } else {
                    String::new()
                };
                let spd_str = if (*speed - 1.0).abs() > 0.05 { format!(" @{:.1}x", speed) } else { String::new() };
                format!("{} [{}] {}{}", op_prefix, pid, rep_str, spd_str)
            } else {
                let rep_str = if *infinite {
                    " ∞".to_string()
                } else if *repeat > 1 {
                    format!(" {}x", repeat)
                } else {
                    String::new()
                };
                let spd_str = if (*speed - 1.0).abs() > 0.05 { format!(" @{:.1}x", speed) } else { String::new() };
                let ev_str = loc.rules_brief_macro_events.replace("{}", &events.len().to_string());
                format!("{} ({}){}{}", op_prefix, ev_str, rep_str, spd_str)
            }
        }
        Action::Notification { title, .. } => loc.rules_brief_notify.replace("{}", title),
        Action::SystemPower { power_type } => {
            format!("{} {}", loc.summary_action_power, power_label(*power_type, loc))
        }
        Action::Wait { mode, seconds, condition, .. } => match mode {
            crate::models::WaitMode::Timer => {
                format!("{} {:.1}{}", loc.summary_action_wait, seconds, loc.rules_brief_seconds)
            }
            crate::models::WaitMode::UntilCondition => {
                let desc = match condition {
                    Some(crate::models::WaitCondition::WindowOpened { process_name, .. }) => {
                        loc.rules_brief_wait_window_open.replace("{}", process_name)
                    }
                    Some(crate::models::WaitCondition::WindowClosed { process_name, .. }) => {
                        loc.rules_brief_wait_window_close.replace("{}", process_name)
                    }
                    Some(crate::models::WaitCondition::ProcessStarted { process_name, .. }) => {
                        loc.rules_brief_wait_process_start.replace("{}", process_name)
                    }
                    Some(crate::models::WaitCondition::ProcessExited { process_name, .. }) => {
                        loc.rules_brief_wait_process_exit.replace("{}", process_name)
                    }
                    Some(crate::models::WaitCondition::Hotkey { key_combination }) => {
                        loc.rules_brief_wait_hotkey.replace("{}", key_combination)
                    }
                    Some(crate::models::WaitCondition::File { path, condition }) => {
                        let fname = std::path::Path::new(path).file_name().and_then(|f| f.to_str()).unwrap_or(path);
                        match condition {
                            crate::models::WaitFileCondition::Exists => loc.rules_brief_wait_file_created.replace("{}", fname),
                            crate::models::WaitFileCondition::Deleted => loc.rules_brief_wait_file_deleted.replace("{}", fname),
                        }
                    }
                    Some(crate::models::WaitCondition::RuleStarted { rule_id }) => {
                        loc.rules_brief_wait_rule_start.replace("{}", rule_id)
                    }
                    Some(crate::models::WaitCondition::RuleStopped { rule_id }) => {
                        loc.rules_brief_wait_rule_stop.replace("{}", rule_id)
                    }
                    None => loc.rules_brief_wait_until.to_string(),
                };
                format!("{} [{}]", loc.summary_action_wait, desc)
            }
        },
        Action::Check { condition } => {
            format!("{} {}", loc.summary_action_check, format_single_condition_brief(condition, loc))
        }
        Action::HttpWebhook { .. } => loc.inspector_badge_webhook.to_string(),
        Action::Window { window_action, target_process, .. } => {
            let proc = target_process.as_deref().unwrap_or(loc.rules_brief_active_process);
            fill(window_action_label(*window_action, loc), &[proc])
        }
        Action::ProcessTuning { target_process, .. } => {
            let proc = target_process.as_deref().unwrap_or(loc.rules_brief_active_process);
            loc.rules_brief_process_tuning.replace("{}", proc)
        }
        Action::PowerPlan { plan } => power_plan_label(*plan, loc).to_string(),
        Action::PlaySound { .. } => loc.rules_brief_play_sound.to_string(),
        Action::Firewall { process_name, action } => match action {
            crate::models::FirewallAction::Block => loc.rules_brief_firewall_block.replace("{}", process_name),
            crate::models::FirewallAction::Allow => loc.rules_brief_firewall_allow.replace("{}", process_name),
            crate::models::FirewallAction::Toggle => format!("{} {}", loc.summary_action_toggle, process_name),
        },
        Action::Display { width, height, refresh_rate, monitor_index } => {
            let res = match (width, height) {
                (Some(w), Some(h)) => format!("{}x{}", w, h),
                _ => loc.rules_brief_display_auto.to_string(),
            };
            let mon_prefix = monitor_index
                .map(|m| loc.rules_brief_display_monitor.replace("{}", &m.to_string()))
                .unwrap_or_default();
            match refresh_rate {
                Some(r) => fill(loc.rules_brief_display_refresh, &[&mon_prefix, &res, &r.to_string()]),
                None => fill(loc.rules_brief_display, &[&mon_prefix, &res]),
            }
        }
        Action::IfElse { conditions, then_actions, else_actions, .. } => {
            let cond_summary = conditions
                .first()
                .map(|c| format_single_condition_brief(&c.condition, loc))
                .unwrap_or_else(|| loc.rules_brief_cond_fallback.to_string());
            let count_suffix = if conditions.len() > 1 { format!("+{}", conditions.len() - 1) } else { String::new() };
            fill(
                loc.rules_brief_ifelse,
                &[&cond_summary, &count_suffix, &then_actions.len().to_string(), &else_actions.len().to_string()],
            )
        }
        Action::TrimWorkingSet { target_process, soft_limit_mb, .. } => {
            let proc = target_process.as_deref().unwrap_or(loc.rules_brief_active_process);
            match soft_limit_mb {
                Some(mb) => fill(loc.rules_brief_trim_limit, &[proc, &mb.to_string()]),
                None => loc.rules_brief_trim.replace("{}", proc),
            }
        }
        Action::DiscordRpcSet { details, .. } => {
            if details.is_empty() {
                loc.inspector_badge_discord_rpc_set.to_string()
            } else {
                loc.rules_brief_discord_text.replace("{}", details)
            }
        }
        Action::DiscordRpcClear => loc.inspector_badge_discord_rpc_clear.to_string(),
        Action::Archive { operation, format, destination_path, .. } => {
            let fname = std::path::Path::new(destination_path).file_name().unwrap_or_default().to_string_lossy().into_owned();
            let format_label = archive_format_label(*format, loc);
            match operation {
                crate::models::ArchiveOperation::Pack => fill(loc.rules_brief_archive_pack, &[format_label, &fname]),
                crate::models::ArchiveOperation::Unpack => fill(loc.rules_brief_archive_unpack, &[format_label, &fname]),
            }
        }
        Action::FileSystem { operation, path, .. } => {
            let fname = std::path::Path::new(path).file_name().unwrap_or_default().to_string_lossy().into_owned();
            match operation {
                crate::models::FileSystemOperation::CreateFile => loc.rules_brief_fs_create_file.replace("{}", &fname),
                crate::models::FileSystemOperation::CreateDirectory => loc.rules_brief_fs_create_dir.replace("{}", &fname),
                crate::models::FileSystemOperation::Rename => loc.rules_brief_fs_rename.replace("{}", &fname),
                crate::models::FileSystemOperation::Copy => loc.rules_brief_fs_copy.replace("{}", &fname),
                crate::models::FileSystemOperation::Delete => loc.rules_brief_fs_delete.replace("{}", &fname),
                crate::models::FileSystemOperation::WriteText => loc.rules_brief_fs_write.replace("{}", &fname),
                crate::models::FileSystemOperation::AppendText => loc.rules_brief_fs_append.replace("{}", &fname),
            }
        }
        Action::Overlay { operation, .. } => {
            match operation {
                crate::models::OverlayOperation::Toggle => loc.rules_brief_overlay_toggle.to_string(),
                crate::models::OverlayOperation::Flash => loc.rules_brief_overlay_flash.to_string(),
                crate::models::OverlayOperation::Show => loc.rules_brief_overlay_show.to_string(),
                crate::models::OverlayOperation::Hide => loc.rules_brief_overlay_hide.to_string(),
                crate::models::OverlayOperation::SetText => loc.rules_brief_overlay_set_text.to_string(),
            }
        }
        Action::RuleControl { operation, .. } => match operation {
            crate::models::RuleControlOperation::Call => loc.rules_brief_rule_call.to_string(),
            crate::models::RuleControlOperation::Stop => loc.rules_brief_rule_stop.to_string(),
        },
    }
}

pub(super) fn format_trigger_display(
    trigger: &Option<Trigger>,
    loc: &crate::i18n::Strings,
) -> (&'static str, String) {
    match trigger {
        Some(Trigger::Process { process_name, single_instance, .. }) => {
            let si_suffix = if *single_instance { loc.rules_summary_single_instance } else { "" };
            (
                egui_phosphor::regular::APP_WINDOW,
                format!("{} {}{}", loc.rules_trigger_proc, process_name, si_suffix),
            )
        }
        Some(Trigger::WindowFocus { process_name, on_focus_gained, on_focus_lost }) => {
            let act = match (*on_focus_gained, *on_focus_lost) {
                (true, true) => loc.rules_trigger_focus_blur,
                (true, false) => loc.rules_trigger_focus,
                (false, true) => loc.rules_trigger_blur,
                _ => loc.rules_trigger_focus,
            };
            (
                egui_phosphor::regular::BROWSERS,
                format!("{} {} {}", loc.rules_trigger_window, act, process_name),
            )
        }
        Some(Trigger::ResourceThreshold { process_name, cpu_percent, ram_mb, .. }) => {
            let mut parts = Vec::new();
            if let Some(c) = cpu_percent {
                parts.push(loc.rules_trigger_cpu.replace("{}", &c.to_string()));
            }
            if let Some(m) = ram_mb {
                parts.push(loc.rules_trigger_ram.replace("{}", &m.to_string()));
            }
            (
                egui_phosphor::regular::GAUGE,
                format!("{} ({})", process_name, parts.join(", ")),
            )
        }
        Some(Trigger::GlobalHotkey { key_combination, hold_mode, repeat_while_held }) => {
            let hold_str = if *hold_mode {
                if *repeat_while_held { loc.rules_trigger_hold_loop } else { loc.rules_trigger_hold }
            } else {
                ""
            };
            (
                egui_phosphor::regular::KEYBOARD,
                format!("{}{}", key_combination, hold_str),
            )
        }
        Some(Trigger::FileWatcher { folder_path, .. }) => {
            let name = std::path::Path::new(folder_path).file_name().and_then(|f| f.to_str()).unwrap_or(folder_path);
            (
                egui_phosphor::regular::FOLDER_NOTCH_OPEN,
                format!("{} {}", loc.rules_trigger_folder, name),
            )
        }
        Some(Trigger::Webhook { endpoint_path }) => {
            (
                egui_phosphor::regular::GLOBE,
                format!("/{}", endpoint_path.trim_start_matches('/')),
            )
        }
        Some(Trigger::SystemEvent { event_type, .. }) => {
            (
                egui_phosphor::regular::DESKTOP,
                sys_event_label(*event_type, loc).to_string(),
            )
        }
        Some(Trigger::Schedule { schedule_type, interval_sec, daily_time, day_of_month, target_datetime, .. }) => {
            let text = match schedule_type {
                crate::models::ScheduleType::Interval => {
                    let s = *interval_sec;
                    if s >= 31536000 && s % 31536000 == 0 {
                        loc.rules_summary_every_year.replace("{}", &(s / 31536000).to_string())
                    } else if s >= 2592000 && s % 2592000 == 0 {
                        loc.rules_summary_every_month.replace("{}", &(s / 2592000).to_string())
                    } else if s >= 604800 && s % 604800 == 0 {
                        loc.rules_summary_every_week.replace("{}", &(s / 604800).to_string())
                    } else if s >= 86400 && s % 86400 == 0 {
                        loc.rules_summary_every_day.replace("{}", &(s / 86400).to_string())
                    } else if s >= 3600 && s % 3600 == 0 {
                        loc.rules_summary_every_hour.replace("{}", &(s / 3600).to_string())
                    } else if s >= 60 && s % 60 == 0 {
                        loc.rules_summary_every_minute.replace("{}", &(s / 60).to_string())
                    } else {
                        loc.rules_summary_every_second.replace("{}", &s.to_string())
                    }
                }
                crate::models::ScheduleType::Daily => {
                    loc.rules_summary_trigger_daily.replace("{}", daily_time.as_deref().unwrap_or("12:00"))
                }
                crate::models::ScheduleType::Monthly => {
                    let day_str = day_of_month.unwrap_or(1).to_string();
                    let time_str = daily_time.as_deref().unwrap_or("12:00");
                    let res = loc.rules_summary_trigger_monthly.replacen("{}", &day_str, 1);
                    res.replacen("{}", time_str, 1)
                }
                crate::models::ScheduleType::OneShot => {
                    loc.rules_summary_trigger_once.replace(
                        "{}",
                        target_datetime.as_deref().unwrap_or(loc.rules_summary_trigger_once_unspecified),
                    )
                }
                crate::models::ScheduleType::AppStartup => loc.rules_summary_trigger_startup.to_string(),
                crate::models::ScheduleType::AppShutdown => loc.rules_summary_trigger_shutdown.to_string(),
            };
            (egui_phosphor::regular::CLOCK, text)
        }
        Some(crate::models::Trigger::Manual) | None => (
            egui_phosphor::regular::HAND_TAP,
            loc.rules_badge_manual.to_string(),
        ),
    }
}

pub(crate) fn action_icon_glyph(action: &Action) -> &'static str {
    match action {
        Action::AudioControl { target, .. } => match target {
            crate::models::AudioTarget::Playback => egui_phosphor::regular::SPEAKER_HIGH,
            crate::models::AudioTarget::Microphone => egui_phosphor::regular::MICROPHONE,
            crate::models::AudioTarget::Process => egui_phosphor::regular::FADERS,
        },
        Action::AudioFormat { .. } => egui_phosphor::regular::EQUALIZER,
        Action::DefaultDevice { target, .. } => match target {
            crate::models::AudioTarget::Playback => egui_phosphor::regular::HEADPHONES,
            crate::models::AudioTarget::Microphone => egui_phosphor::regular::MICROPHONE,
            crate::models::AudioTarget::Process => egui_phosphor::regular::FADERS,
        },
        Action::ExecuteScript { interpreter, .. } => match interpreter {
            crate::models::ScriptType::PowerShell | crate::models::ScriptType::Lua => egui_phosphor::regular::CODE,
            crate::models::ScriptType::Batch => egui_phosphor::regular::TERMINAL_WINDOW,
        },
        Action::Macro { .. } => egui_phosphor::regular::KEYBOARD,
        Action::StartProcess { .. } => egui_phosphor::regular::ROCKET,
        Action::KillProcess { .. } => egui_phosphor::regular::STOP,
        Action::Window { .. } => egui_phosphor::regular::APP_WINDOW,
        Action::ProcessTuning { .. } => egui_phosphor::regular::GAUGE,
        Action::PowerPlan { .. } => egui_phosphor::regular::PLUG,
        Action::PlaySound { .. } => egui_phosphor::regular::MUSIC_NOTES,
        Action::Firewall { .. } => egui_phosphor::regular::SHIELD,
        Action::Notification { .. } => egui_phosphor::regular::BELL,
        Action::SystemPower { .. } => egui_phosphor::regular::POWER,
        Action::Wait { .. } => egui_phosphor::regular::HOURGLASS,
        Action::Check { .. } => egui_phosphor::regular::CHECK_CIRCLE,
        Action::HttpWebhook { .. } => egui_phosphor::regular::GLOBE,
        Action::Display { .. } => egui_phosphor::regular::MONITOR,
        Action::IfElse { .. } => egui_phosphor::regular::GIT_FORK,
        Action::TrimWorkingSet { .. } => egui_phosphor::regular::BROOM,
        Action::DiscordRpcSet { .. } | Action::DiscordRpcClear => egui_phosphor::regular::GAME_CONTROLLER,
        Action::Archive { .. } => egui_phosphor::regular::ARCHIVE,
        Action::FileSystem { .. } => egui_phosphor::regular::FOLDER,
        Action::Overlay { .. } => egui_phosphor::regular::FRAME_CORNERS,
        Action::RuleControl { .. } => egui_phosphor::regular::FLOW_ARROW,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{self, Language};

    #[test]
    fn test_format_single_condition_brief_and_tooltip() {
        let loc = i18n::get(Language::English);

        // 1. Process running condition
        let cond1 = RuleCondition::ProcessRunning {
            process_name: "notepad.exe".into(),
            match_mode: crate::models::ProcessMatchMode::Exact,
            must_be_running: true,
        };
        let brief1 = format_single_condition_brief(&cond1, loc);
        assert_eq!(brief1, "RUN:notepad.exe");
        let tip1 = format_single_condition_tooltip(&cond1, 1, loc);
        assert_eq!(
            tip1,
            format!(
                "{} [1] {}",
                loc.inspector_conditions_badge,
                loc.rules_tip_cond_process_running.replace("{}", "notepad.exe")
            )
        );

        // 2. Window focused condition
        let cond2 = RuleCondition::WindowFocused {
            process_name: "code.exe".into(),
            match_mode: crate::models::ProcessMatchMode::Exact,
            must_be_focused: true,
        };
        let brief2 = format_single_condition_brief(&cond2, loc);
        assert_eq!(brief2, "FOCUS:code.exe");
        let tip2 = format_single_condition_tooltip(&cond2, 2, loc);
        assert_eq!(
            tip2,
            format!(
                "{} [2] {}",
                loc.inspector_conditions_badge,
                loc.rules_tip_cond_window_focused.replace("{}", "code.exe")
            )
        );
    }

    #[test]
    fn test_condition_and_action_follow_ui_language() {
        let en = i18n::get(Language::English);
        let ru = i18n::get(Language::Russian);

        let cond = RuleCondition::ProcessRunning {
            process_name: "notepad.exe".into(),
            match_mode: crate::models::ProcessMatchMode::Exact,
            must_be_running: true,
        };
        let tip_en = format_single_condition_tooltip(&cond, 1, en);
        let tip_ru = format_single_condition_tooltip(&cond, 1, ru);
        assert_ne!(tip_en, tip_ru);
        assert!(tip_ru.starts_with(ru.inspector_conditions_badge));
        assert_eq!(
            tip_ru,
            format!(
                "{} [1] {}",
                ru.inspector_conditions_badge,
                ru.rules_tip_cond_process_running.replace("{}", "notepad.exe")
            )
        );

        let event_cond = RuleCondition::Event {
            event_type: SystemEventType::SessionLock,
            idle_duration_sec: 300,
        };
        let badge_en = format_single_condition_brief(&event_cond, en);
        let badge_ru = format_single_condition_brief(&event_cond, ru);
        assert_ne!(badge_en, badge_ru);
        assert_eq!(badge_ru, ru.rules_badge_cond_event.replace("{}", ru.inspector_sys_event_lock));

        let action = Action::Notification {
            title: "Done".into(),
            message: String::new(),
        };
        let brief_en = format_action_brief(&action, en);
        let brief_ru = format_action_brief(&action, ru);
        assert_ne!(brief_en, brief_ru);
        assert_eq!(brief_ru, ru.rules_brief_notify.replace("{}", "Done"));

        let power = Action::SystemPower { power_type: SystemPowerType::Shutdown };
        assert_eq!(
            format_action_brief(&power, ru),
            format!("{} {}", ru.summary_action_power, ru.inspector_power_shutdown)
        );
    }

    #[test]
    fn test_mapped_enums_never_render_debug_names() {
        let loc = i18n::get(Language::Russian);

        for event_type in [
            SystemEventType::SessionLock,
            SystemEventType::SessionUnlock,
            SystemEventType::SystemSleep,
            SystemEventType::SystemResume,
            SystemEventType::PowerSourceAc,
            SystemEventType::PowerSourceBattery,
            SystemEventType::UserIdle,
            SystemEventType::UserActive,
            SystemEventType::DeviceConnected,
            SystemEventType::DeviceDisconnected,
            SystemEventType::MonitorOn,
            SystemEventType::MonitorOff,
            SystemEventType::NetworkConnected,
            SystemEventType::NetworkDisconnected,
        ] {
            let debug = format!("{event_type:?}");
            let label = sys_event_label(event_type, loc);
            let cond = RuleCondition::Event {
                event_type,
                idle_duration_sec: 300,
            };
            let tooltip = format_single_condition_tooltip(&cond, 1, loc);
            assert_eq!(tooltip, format!("{} [1] {}", loc.inspector_conditions_badge, loc.rules_tip_cond_event.replace("{}", label)));
            assert!(!tooltip.contains(&debug), "condition tooltip leaked the Debug enum name {debug}");
            let badge = format_single_condition_brief(&cond, loc);
            assert_eq!(badge, loc.rules_badge_cond_event.replace("{}", label));
            assert!(!badge.contains(&debug), "condition badge leaked the Debug enum name {debug}");
            let (_, trigger) = format_trigger_display(
                &Some(Trigger::SystemEvent {
                    event_type,
                    device_id_filter: None,
                }),
                loc,
            );
            assert_eq!(trigger, label);
            assert!(!trigger.contains(&debug), "trigger summary leaked the Debug enum name {debug}");
        }

        for power_type in [
            SystemPowerType::Lock,
            SystemPowerType::Sleep,
            SystemPowerType::TurnScreenOff,
            SystemPowerType::Restart,
            SystemPowerType::Shutdown,
        ] {
            let debug = format!("{power_type:?}");
            let brief = format_action_brief(&Action::SystemPower { power_type }, loc);
            assert_eq!(brief, format!("{} {}", loc.summary_action_power, power_label(power_type, loc)));
            assert!(!brief.contains(&debug), "power chip leaked the Debug enum name {debug}");
        }

        for plan in [
            PowerPlanType::Balanced,
            PowerPlanType::HighPerformance,
            PowerPlanType::PowerSaver,
        ] {
            let debug = format!("{plan:?}");
            let brief = format_action_brief(&Action::PowerPlan { plan }, loc);
            assert_eq!(brief, power_plan_label(plan, loc));
            assert!(!brief.contains(&debug), "power plan chip leaked the Debug enum name {debug}");
        }

        for format in [
            ArchiveFormat::Zip,
            ArchiveFormat::Tar,
            ArchiveFormat::TarGz,
            ArchiveFormat::TarXz,
            ArchiveFormat::SevenZip,
            ArchiveFormat::Rar,
        ] {
            let debug = format!("{format:?}");
            let brief = format_action_brief(
                &Action::Archive {
                    operation: crate::models::ArchiveOperation::Pack,
                    format,
                    source_path: "C:\\source".into(),
                    destination_path: "C:\\out.bin".into(),
                    compression_level: None,
                    overwrite: true,
                },
                loc,
            );
            assert_eq!(brief, fill(loc.rules_brief_archive_pack, &[archive_format_label(format, loc), "out.bin"]));
            assert!(!brief.contains(&debug), "archive chip leaked the Debug enum name {debug}");
        }

        for action in [
            WindowActionType::Minimize,
            WindowActionType::Maximize,
            WindowActionType::Restore,
            WindowActionType::Hide,
            WindowActionType::Show,
            WindowActionType::Close,
            WindowActionType::ToggleAlwaysOnTop,
            WindowActionType::AlwaysOnTop,
            WindowActionType::RemoveAlwaysOnTop,
            WindowActionType::SetTransparency,
            WindowActionType::SnapToPosition,
            WindowActionType::ToggleMinimizeRestore,
            WindowActionType::ToggleShowHide,
        ] {
            let debug = format!("{action:?}");
            let brief = format_action_brief(
                &Action::Window {
                    target_process: Some("app.exe".into()),
                    window_action: action,
                    x: None,
                    y: None,
                    width: None,
                    height: None,
                    transparency: None,
                },
                loc,
            );
            assert_eq!(brief, fill(window_action_label(action, loc), &["app.exe"]));
            assert!(!brief.contains(&debug), "window chip leaked the Debug enum name {debug}");
        }
    }

    #[test]
    fn test_format_trigger_display_no_prefix() {
        for lang in [Language::English, Language::Russian] {
            let loc = i18n::get(lang);

            let (_, trig_manual) = format_trigger_display(&Some(Trigger::Manual), loc);
            assert!(!trig_manual.starts_with("TRIGGER:"));
            assert!(!trig_manual.starts_with("ТРИГГЕР:"));

            let (_, trig_proc) = format_trigger_display(
                &Some(Trigger::Process {
                    process_name: "notepad.exe".into(),
                    match_mode: crate::models::ProcessMatchMode::Exact,
                    single_instance: false,
                }),
                loc,
            );
            assert!(!trig_proc.starts_with("TRIGGER:"));
            assert!(!trig_proc.starts_with("ТРИГГЕР:"));
            assert!(trig_proc.contains("notepad.exe"));

            // Schedule: Daily
            let (_, trig_daily) = format_trigger_display(
                &Some(Trigger::Schedule {
                    schedule_type: crate::models::ScheduleType::Daily,
                    interval_sec: 0,
                    daily_time: Some("14:30".into()),
                    days_of_week: vec![],
                    day_of_month: None,
                    target_datetime: None,
                }),
                loc,
            );
            assert!(!trig_daily.starts_with("TRIGGER:"));
            assert!(!trig_daily.starts_with("ТРИГГЕР:"));

            // Schedule: OneShot
            let (_, trig_once) = format_trigger_display(
                &Some(Trigger::Schedule {
                    schedule_type: crate::models::ScheduleType::OneShot,
                    interval_sec: 0,
                    daily_time: None,
                    days_of_week: vec![],
                    day_of_month: None,
                    target_datetime: Some("2026-12-31 23:59".into()),
                }),
                loc,
            );
            assert!(!trig_once.starts_with("TRIGGER:"));
            assert!(!trig_once.starts_with("ТРИГГЕР:"));

            // One-shot without a target time: the fallback must follow the UI language too (U16-011).
            let (_, trig_once_unspecified) = format_trigger_display(
                &Some(Trigger::Schedule {
                    schedule_type: crate::models::ScheduleType::OneShot,
                    interval_sec: 0,
                    daily_time: None,
                    days_of_week: vec![],
                    day_of_month: None,
                    target_datetime: None,
                }),
                loc,
            );
            assert_eq!(
                trig_once_unspecified,
                loc.rules_summary_trigger_once
                    .replace("{}", loc.rules_summary_trigger_once_unspecified)
            );
            if lang == Language::Russian {
                assert!(
                    !trig_once_unspecified.contains("SPECIFIED TIME"),
                    "the Russian one-shot summary must not embed an English fallback"
                );
            }

            // Schedule: AppStartup
            let (_, trig_startup) = format_trigger_display(
                &Some(Trigger::Schedule {
                    schedule_type: crate::models::ScheduleType::AppStartup,
                    interval_sec: 0,
                    daily_time: None,
                    days_of_week: vec![],
                    day_of_month: None,
                    target_datetime: None,
                }),
                loc,
            );
            assert!(!trig_startup.starts_with("TRIGGER:"));
            assert!(!trig_startup.starts_with("ТРИГГЕР:"));

            // Schedule: AppShutdown
            let (_, trig_shutdown) = format_trigger_display(
                &Some(Trigger::Schedule {
                    schedule_type: crate::models::ScheduleType::AppShutdown,
                    interval_sec: 0,
                    daily_time: None,
                    days_of_week: vec![],
                    day_of_month: None,
                    target_datetime: None,
                }),
                loc,
            );
            assert!(!trig_shutdown.starts_with("TRIGGER:"));
            assert!(!trig_shutdown.starts_with("ТРИГГЕР:"));
        }
    }

    /// The trigger line and the compact condition chips follow the UI language; only the layout
    /// separators stay in the format strings.
    #[test]
    fn test_trigger_display_and_compact_prefix_follow_ui_language() {
        let en = i18n::get(Language::English);
        let ru = i18n::get(Language::Russian);

        let process = Some(Trigger::Process {
            process_name: "notepad.exe".into(),
            match_mode: crate::models::ProcessMatchMode::Exact,
            single_instance: false,
        });
        let (_, proc_en) = format_trigger_display(&process, en);
        let (_, proc_ru) = format_trigger_display(&process, ru);
        assert_eq!(proc_en, format!("{} {}", en.rules_trigger_proc, "notepad.exe"));
        assert_eq!(proc_ru, format!("{} {}", ru.rules_trigger_proc, "notepad.exe"));
        assert_ne!(proc_en, proc_ru);

        let window = Some(Trigger::WindowFocus {
            process_name: "code.exe".into(),
            on_focus_gained: true,
            on_focus_lost: false,
        });
        let (_, window_en) = format_trigger_display(&window, en);
        assert_eq!(
            window_en,
            format!("{} {} {}", en.rules_trigger_window, en.rules_trigger_focus, "code.exe")
        );
        assert_ne!(window_en, format_trigger_display(&window, ru).1);

        let hotkey = Some(Trigger::GlobalHotkey {
            key_combination: "Ctrl+W".into(),
            hold_mode: true,
            repeat_while_held: false,
        });
        let (_, hotkey_en) = format_trigger_display(&hotkey, en);
        assert_eq!(hotkey_en, format!("Ctrl+W{}", en.rules_trigger_hold));
        assert_ne!(hotkey_en, format_trigger_display(&hotkey, ru).1);

        let running = RuleCondition::ProcessRunning {
            process_name: "notepad.exe".into(),
            match_mode: crate::models::ProcessMatchMode::Exact,
            must_be_running: true,
        };
        let brief_en = format_single_condition_brief(&running, en);
        let brief_ru = format_single_condition_brief(&running, ru);
        assert_eq!(brief_en, format!("{}{}", en.rules_brief_cond_run, "notepad.exe"));
        assert_eq!(brief_ru, format!("{}{}", ru.rules_brief_cond_run, "notepad.exe"));
        assert_ne!(brief_en, brief_ru);

        let not_running = RuleCondition::ProcessRunning {
            process_name: "notepad.exe".into(),
            match_mode: crate::models::ProcessMatchMode::Exact,
            must_be_running: false,
        };
        assert_eq!(
            format_single_condition_brief(&not_running, en),
            format!("!{}{}", en.rules_brief_cond_run, "notepad.exe")
        );

        let resource = RuleCondition::ResourceThreshold {
            process_name: "game.exe".into(),
            cpu_percent: Some(75.0),
            ram_mb: Some(512),
        };
        let res_en = format_single_condition_brief(&resource, en);
        assert_eq!(
            res_en,
            format!(
                "{}{}({},{})",
                en.rules_brief_cond_res,
                "game.exe",
                en.rules_brief_cond_cpu.replace("{}", "75"),
                en.rules_brief_cond_ram.replace("{}", "512"),
            )
        );
        assert_ne!(res_en, format_single_condition_brief(&resource, ru));
    }
}
