//! Action badges, labels and the one-line drag-preview summaries.

use super::*;
use crate::gui::components::text::fill;

pub(crate) fn action_type_badge(action: &Action, loc: &crate::i18n::Strings) -> &'static str {
    match action {
        Action::StartProcess { close_on_exit, .. } => {
            if *close_on_exit {
                loc.inspector_badge_launch_companion
            } else {
                loc.inspector_badge_launch_process
            }
        }
        Action::KillProcess { .. } => loc.inspector_badge_kill_process,
        Action::Notification { .. } => loc.inspector_badge_notification,
        Action::SystemPower { .. } => loc.inspector_badge_system_power,
        Action::Wait { .. } => loc.inspector_badge_wait,
        Action::Check { .. } => loc.inspector_badge_check,
        Action::HttpWebhook { .. } => loc.inspector_badge_webhook,
        Action::Window { .. } => loc.inspector_badge_window,
        Action::ProcessTuning { .. } => loc.inspector_badge_tuning,
        Action::PowerPlan { .. } => loc.inspector_badge_power_plan,
        Action::PlaySound { .. } => loc.inspector_badge_play_sound,
        Action::Firewall { .. } => loc.inspector_badge_firewall,
        Action::Display { .. } => loc.inspector_badge_display,
        Action::AudioControl { .. } => loc.inspector_badge_audio_control,
        Action::DefaultDevice { .. } => loc.inspector_badge_default_device,
        Action::ExecuteScript { .. } => loc.inspector_badge_execute_script,
        Action::Macro { .. } => loc.inspector_badge_macro,
        Action::IfElse { .. } => loc.inspector_badge_if_else,
        Action::AudioFormat { .. } => loc.inspector_badge_audio_format,
        Action::TrimWorkingSet { .. } => loc.inspector_badge_trim_working_set,
        Action::DiscordRpcSet { .. } => loc.inspector_badge_discord_rpc_set,
        Action::DiscordRpcClear => loc.inspector_badge_discord_rpc_clear,
        Action::Archive { .. } => loc.inspector_badge_archive,
        Action::FileSystem { .. } => loc.inspector_badge_file_system,
        Action::Overlay { .. } => loc.inspector_badge_overlay,
        Action::RuleControl { .. } => loc.inspector_badge_rule_control,
    }
}

/// Localized window-operation label, shared by the combo box and the summary line.
pub(crate) fn window_action_label(action: &WindowActionType, loc: &crate::i18n::Strings) -> &'static str {
    match action {
        WindowActionType::Minimize => loc.inspector_win_action_minimize,
        WindowActionType::Maximize => loc.inspector_win_action_maximize,
        WindowActionType::Restore => loc.inspector_win_action_restore,
        WindowActionType::ToggleMinimizeRestore => loc.inspector_win_toggle_min_restore,
        WindowActionType::Hide => loc.inspector_win_action_hide,
        WindowActionType::Show => loc.inspector_win_action_show,
        WindowActionType::ToggleShowHide => loc.inspector_win_toggle_show_hide,
        WindowActionType::Close => loc.inspector_win_action_close,
        WindowActionType::AlwaysOnTop => loc.inspector_win_action_always_on_top,
        WindowActionType::RemoveAlwaysOnTop => loc.inspector_win_action_remove_topmost,
        WindowActionType::ToggleAlwaysOnTop => loc.inspector_win_action_toggle_topmost,
        WindowActionType::SetTransparency => loc.inspector_win_action_transparency,
        WindowActionType::SnapToPosition => loc.inspector_win_action_position,
    }
}

/// Localized process-priority label, shared by the combo box and its option rows.
pub(crate) fn priority_label(priority: &Option<ProcessPriorityLevel>, loc: &crate::i18n::Strings) -> &'static str {
    match priority {
        None => loc.inspector_priority_unchanged,
        Some(ProcessPriorityLevel::Idle) => loc.inspector_priority_idle,
        Some(ProcessPriorityLevel::BelowNormal) => loc.inspector_priority_below_normal,
        Some(ProcessPriorityLevel::Normal) => loc.inspector_priority_normal,
        Some(ProcessPriorityLevel::AboveNormal) => loc.inspector_priority_above_normal,
        Some(ProcessPriorityLevel::High) => loc.inspector_priority_high,
        Some(ProcessPriorityLevel::Realtime) => loc.inspector_priority_realtime,
    }
}

/// One-line summary of an action for the floating drag preview. Every word comes from the
/// catalog; only separators, units and the macro markers stay in the format strings.
pub(crate) fn action_summary_preview(action: &Action, loc: &crate::i18n::Strings) -> String {
    match action {
        Action::StartProcess { file_path, .. } => {
            let name = std::path::Path::new(file_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(file_path);
            if name.is_empty() {
                loc.summary_preview_launch_empty.to_string()
            } else {
                fill(loc.summary_preview_launch, &[name])
            }
        }
        Action::KillProcess { process_name, .. } => fill(loc.summary_preview_kill, &[process_name.as_str()]),
        Action::Notification { title, .. } => {
            if title.is_empty() {
                loc.inspector_menu_notification.to_string()
            } else {
                fill(loc.summary_preview_notify, &[title.as_str()])
            }
        }
        Action::SystemPower { power_type } => {
            let power = match power_type {
                SystemPowerType::Lock => loc.inspector_power_lock,
                SystemPowerType::Sleep => loc.inspector_power_sleep,
                SystemPowerType::TurnScreenOff => loc.inspector_power_screen_off,
                SystemPowerType::Restart => loc.inspector_power_restart,
                SystemPowerType::Shutdown => loc.inspector_power_shutdown,
            };
            fill(loc.summary_preview_power, &[power])
        }
        Action::Wait { mode, seconds, condition, timeout_sec, .. } => match mode {
            crate::models::WaitMode::Timer => {
                let seconds_txt = format!("{:.1}", seconds);
                fill(loc.summary_preview_wait_timer, &[seconds_txt.as_str()])
            }
            crate::models::WaitMode::UntilCondition => match condition {
                Some(crate::models::WaitCondition::WindowOpened { process_name, .. }) => {
                    fill(loc.summary_preview_wait_window_open, &[process_name.as_str()])
                }
                Some(crate::models::WaitCondition::WindowClosed { process_name, .. }) => {
                    fill(loc.summary_preview_wait_window_close, &[process_name.as_str()])
                }
                Some(crate::models::WaitCondition::ProcessStarted { process_name, .. }) => {
                    fill(loc.summary_preview_wait_process_start, &[process_name.as_str()])
                }
                Some(crate::models::WaitCondition::ProcessExited { process_name, .. }) => {
                    fill(loc.summary_preview_wait_process_exit, &[process_name.as_str()])
                }
                Some(crate::models::WaitCondition::Hotkey { key_combination }) => {
                    fill(loc.summary_preview_wait_hotkey, &[key_combination.as_str()])
                }
                Some(crate::models::WaitCondition::File { path, condition }) => match condition {
                    crate::models::WaitFileCondition::Exists => fill(loc.summary_preview_wait_file_exists, &[path.as_str()]),
                    crate::models::WaitFileCondition::Deleted => fill(loc.summary_preview_wait_file_deleted, &[path.as_str()]),
                },
                Some(crate::models::WaitCondition::RuleStarted { rule_id }) => {
                    fill(loc.summary_preview_wait_rule_start, &[rule_id.as_str()])
                }
                Some(crate::models::WaitCondition::RuleStopped { rule_id }) => {
                    fill(loc.summary_preview_wait_rule_stop, &[rule_id.as_str()])
                }
                None => {
                    let timeout_txt = format!("{:.0}", timeout_sec);
                    fill(loc.summary_preview_wait_condition, &[timeout_txt.as_str()])
                }
            },
        },
        Action::Check { condition } => match condition {
            crate::models::RuleCondition::ProcessRunning { process_name, must_be_running, .. } => {
                if *must_be_running {
                    fill(loc.summary_preview_check_running, &[process_name.as_str()])
                } else {
                    fill(loc.summary_preview_check_not_running, &[process_name.as_str()])
                }
            }
            crate::models::RuleCondition::WindowFocused { process_name, .. } => {
                fill(loc.summary_preview_check_window, &[process_name.as_str()])
            }
            crate::models::RuleCondition::FileWatcher { path, must_exist, .. } => {
                if *must_exist {
                    fill(loc.summary_preview_check_file_exists, &[path.as_str()])
                } else {
                    fill(loc.summary_preview_check_file_deleted, &[path.as_str()])
                }
            }
            crate::models::RuleCondition::RuleRunning { rule_id, must_be_running } => {
                if *must_be_running {
                    fill(loc.summary_preview_check_rule_running, &[rule_id.as_str()])
                } else {
                    fill(loc.summary_preview_check_rule_stopped, &[rule_id.as_str()])
                }
            }
            _ => loc.catalog_title_check.to_string(),
        },
        Action::HttpWebhook { url, method, .. } => {
            if url.is_empty() {
                loc.inspector_trigger_webhook.to_string()
            } else {
                format!("{} {}", method, url)
            }
        }
        Action::Window { window_action, target_process, .. } => {
            let proc = target_process.as_deref().unwrap_or(loc.summary_preview_active_window);
            match window_action {
                WindowActionType::ToggleMinimizeRestore => fill(loc.summary_preview_window_toggle_min, &[proc]),
                WindowActionType::ToggleShowHide => fill(loc.summary_preview_window_toggle_show, &[proc]),
                other => fill(loc.summary_preview_window, &[window_action_label(other, loc), proc]),
            }
        }
        Action::ProcessTuning { target_process, .. } => {
            let proc = target_process.as_deref().unwrap_or(loc.summary_preview_trigger_process);
            fill(loc.summary_preview_tuning, &[proc])
        }
        Action::PowerPlan { plan } => {
            let plan_label = match plan {
                PowerPlanType::HighPerformance => loc.inspector_power_plan_high,
                PowerPlanType::Balanced => loc.inspector_power_plan_balanced,
                PowerPlanType::PowerSaver => loc.inspector_power_plan_saver,
            };
            fill(loc.summary_preview_power_plan, &[plan_label])
        }
        Action::PlaySound { file_path, play_beep, beep_frequency, beep_duration_ms } => {
            if *play_beep {
                if let Some(f) = beep_frequency {
                    let freq_txt = f.to_string();
                    let duration_txt = beep_duration_ms.unwrap_or(200).to_string();
                    fill(loc.summary_preview_beep, &[freq_txt.as_str(), duration_txt.as_str()])
                } else {
                    loc.inspector_audio_beep.to_string()
                }
            } else {
                let f = std::path::Path::new(file_path).file_name().and_then(|s| s.to_str()).unwrap_or(file_path);
                fill(loc.summary_preview_sound, &[f])
            }
        }
        Action::Firewall { process_name, action } => match action {
            crate::models::FirewallAction::Block => fill(loc.summary_preview_firewall_block, &[process_name.as_str()]),
            crate::models::FirewallAction::Allow => fill(loc.summary_preview_firewall_allow, &[process_name.as_str()]),
            crate::models::FirewallAction::Toggle => fill(loc.summary_preview_firewall_toggle, &[process_name.as_str()]),
        },
        Action::Display { width, height, refresh_rate, monitor_index } => {
            let res = match (width, height) {
                (Some(w), Some(h)) => format!("{}x{}", w, h),
                _ => loc.summary_preview_auto_resolution.to_string(),
            };
            let mon = match monitor_index {
                Some(m) => {
                    let m_txt = m.to_string();
                    format!("{} ", fill(loc.summary_monitor_prefix, &[m_txt.as_str()]))
                }
                None => String::new(),
            };
            match refresh_rate {
                Some(r) => {
                    let rate_txt = r.to_string();
                    fill(loc.summary_preview_display_refresh, &[mon.as_str(), res.as_str(), rate_txt.as_str()])
                }
                None => fill(loc.summary_preview_display, &[mon.as_str(), res.as_str()]),
            }
        }
        Action::AudioControl { target, action_type, volume_level, left_volume, right_volume, process_name } => {
            let target_str = match target {
                crate::models::AudioTarget::Playback => loc.summary_audio_target_playback,
                crate::models::AudioTarget::Microphone => loc.summary_audio_target_mic,
                crate::models::AudioTarget::Process => loc.summary_audio_target_process,
            };
            match action_type {
                crate::models::AudioVolumeAction::Mute => fill(loc.summary_preview_audio_mute, &[target_str]),
                crate::models::AudioVolumeAction::Unmute => fill(loc.summary_preview_audio_unmute, &[target_str]),
                crate::models::AudioVolumeAction::ToggleMute => fill(loc.summary_preview_audio_toggle, &[target_str]),
                crate::models::AudioVolumeAction::SetVolume => {
                    if let (Some(l), Some(r)) = (left_volume, right_volume) {
                        let left_txt = l.to_string();
                        let right_txt = r.to_string();
                        fill(loc.summary_preview_audio_balance, &[target_str, left_txt.as_str(), right_txt.as_str()])
                    } else if let Some(p) = process_name {
                        let volume_txt = volume_level.to_string();
                        fill(
                            loc.summary_preview_audio_process_volume,
                            &[target_str, p.as_str(), volume_txt.as_str()],
                        )
                    } else {
                        let volume_txt = volume_level.to_string();
                        fill(loc.summary_preview_audio_volume, &[target_str, volume_txt.as_str()])
                    }
                }
                crate::models::AudioVolumeAction::PlayBeep => loc.summary_preview_audio_play_beep.to_string(),
            }
        }
        Action::AudioFormat { target, sample_rate, bit_depth, channels, .. } => {
            let target_str = match target {
                crate::models::AudioTarget::Playback | crate::models::AudioTarget::Process => loc.summary_audio_output,
                crate::models::AudioTarget::Microphone => loc.summary_audio_input,
            };
            let ch_str = if *channels == 1 {
                loc.inspector_audio_channels_mono
            } else {
                loc.inspector_audio_channels_stereo
            };
            let rate_txt = sample_rate.to_string();
            let depth_txt = bit_depth.to_string();
            fill(
                loc.summary_preview_audio_format,
                &[target_str, rate_txt.as_str(), depth_txt.as_str(), ch_str],
            )
        }
        Action::DefaultDevice { target, device_name } => {
            let pfx = match target {
                crate::models::AudioTarget::Playback => loc.summary_device_output,
                crate::models::AudioTarget::Microphone => loc.summary_audio_target_mic,
                crate::models::AudioTarget::Process => loc.summary_device_process,
            };
            fill(loc.summary_preview_default_device, &[pfx, device_name.as_str()])
        }
        Action::ExecuteScript { interpreter, script, .. } => {
            let name = match interpreter {
                crate::models::ScriptType::PowerShell => loc.inspector_script_type_powershell,
                crate::models::ScriptType::Batch => loc.inspector_script_type_cmd,
                crate::models::ScriptType::Lua => loc.inspector_script_type_lua,
            };
            let first = script.lines().next().unwrap_or("").trim();
            if first.is_empty() {
                fill(loc.summary_preview_execute, &[name])
            } else {
                format!("{}: {}", name, first.chars().take(35).collect::<String>())
            }
        }
        Action::Macro { preset_id, operation, events, repeat, speed, infinite } => {
            let pfx = match operation {
                crate::models::MacroOperation::Run => loc.inspector_macro_op_run,
                crate::models::MacroOperation::Stop => loc.inspector_macro_op_stop,
                crate::models::MacroOperation::Toggle => loc.inspector_macro_op_toggle,
            };
            if *operation == crate::models::MacroOperation::Stop {
                if let Some(pid) = preset_id {
                    format!("{}: {}", pfx, pid)
                } else {
                    loc.summary_preview_macro_stop_all.to_string()
                }
            } else if let Some(pid) = preset_id {
                let rep_str = if *infinite {
                    " ∞".to_string()
                } else if *repeat > 1 {
                    format!(" x{}", repeat)
                } else {
                    String::new()
                };
                let spd_str = if (*speed - 1.0).abs() > 0.05 { format!(" @{:.1}x", speed) } else { String::new() };
                format!("{}: {}{}{}", pfx, pid, rep_str, spd_str)
            } else {
                let rep_str = if *infinite {
                    " ∞".to_string()
                } else if *repeat > 1 {
                    format!(" x{}", repeat)
                } else {
                    String::new()
                };
                let spd_str = if (*speed - 1.0).abs() > 0.05 { format!(" @{:.1}x", speed) } else { String::new() };
                let count_txt = events.len().to_string();
                let events_txt = fill(loc.summary_preview_macro_events, &[count_txt.as_str()]);
                format!("{}: {}{}{}", pfx, events_txt, rep_str, spd_str)
            }
        }
        Action::IfElse { conditions, then_actions, else_if_branches, else_actions } => {
            let glyph = conditions.first().map(|c| crate::gui::views::rule_summary::condition_icon_glyph(&c.condition)).unwrap_or("❓");
            let count_txt = conditions.len().to_string();
            let then_txt = then_actions.len().to_string();
            let else_txt = else_actions.len().to_string();
            if else_if_branches.is_empty() {
                fill(
                    loc.summary_preview_ifelse,
                    &[glyph, count_txt.as_str(), then_txt.as_str(), else_txt.as_str()],
                )
            } else {
                let elif_txt = else_if_branches.len().to_string();
                fill(
                    loc.summary_preview_ifelse_elif,
                    &[glyph, count_txt.as_str(), then_txt.as_str(), elif_txt.as_str(), else_txt.as_str()],
                )
            }
        }
        Action::TrimWorkingSet { target_process, soft_limit_mb, .. } => {
            let proc = target_process.as_deref().unwrap_or(loc.summary_preview_active_process);
            match soft_limit_mb {
                Some(mb) => {
                    let limit_txt = mb.to_string();
                    fill(loc.summary_preview_trim_limit, &[proc, limit_txt.as_str()])
                }
                None => fill(loc.summary_preview_trim, &[proc]),
            }
        }
        Action::DiscordRpcSet { details, state, .. } => {
            if details.is_empty() && state.is_empty() {
                loc.summary_preview_discord.to_string()
            } else if !details.is_empty() && !state.is_empty() {
                fill(loc.summary_preview_discord_set, &[details.as_str(), state.as_str()])
            } else {
                // No dedicated one-sided template: the badge token is the shared prefix.
                format!("{}: {}{}", loc.inspector_badge_discord_rpc_set, details, state)
            }
        }
        Action::DiscordRpcClear => loc.summary_preview_discord_clear.to_string(),
        Action::Archive { operation, format, destination_path, .. } => {
            let fname = std::path::Path::new(destination_path).file_name().unwrap_or_default().to_string_lossy();
            let op = match operation {
                crate::models::ArchiveOperation::Pack => loc.inspector_archive_op_pack,
                crate::models::ArchiveOperation::Unpack => loc.inspector_archive_op_unpack,
            };
            format!("{} {} -> {}", op, archive_format_label(*format, loc), fname)
        }
        Action::FileSystem { operation, path, .. } => {
            let fname = std::path::Path::new(path).file_name().unwrap_or_default().to_string_lossy();
            let op = match operation {
                crate::models::FileSystemOperation::CreateFile => loc.inspector_fs_op_create_file,
                crate::models::FileSystemOperation::CreateDirectory => loc.inspector_fs_op_create_dir,
                crate::models::FileSystemOperation::Rename => loc.inspector_fs_op_rename,
                crate::models::FileSystemOperation::Copy => loc.inspector_fs_op_copy,
                crate::models::FileSystemOperation::Delete => loc.inspector_fs_op_delete,
                crate::models::FileSystemOperation::WriteText => loc.inspector_fs_op_write_text,
                crate::models::FileSystemOperation::AppendText => loc.inspector_fs_op_append_text,
            };
            format!("{}: {}", op, fname)
        }
        Action::Overlay { overlay_id, operation, override_text, .. } => {
            let op_str = match operation {
                crate::models::OverlayOperation::Show => loc.inspector_overlay_op_show,
                crate::models::OverlayOperation::Hide => loc.inspector_overlay_op_hide,
                crate::models::OverlayOperation::Toggle => loc.summary_overlay_op_toggle,
                crate::models::OverlayOperation::Flash => loc.summary_overlay_op_flash,
                crate::models::OverlayOperation::SetText => loc.inspector_overlay_op_set_text,
            };
            if let Some(txt) = override_text {
                fill(loc.summary_preview_overlay_text, &[op_str, overlay_id.as_str(), txt.as_str()])
            } else {
                fill(loc.summary_preview_overlay, &[op_str, overlay_id.as_str()])
            }
        }
        Action::RuleControl { rule_id, operation, wait } => match operation {
            crate::models::RuleControlOperation::Call => {
                if *wait {
                    fill(loc.summary_preview_rule_call_wait, &[rule_id.as_str()])
                } else {
                    fill(loc.summary_preview_rule_call, &[rule_id.as_str()])
                }
            }
            crate::models::RuleControlOperation::Stop => fill(loc.summary_preview_rule_stop, &[rule_id.as_str()]),
        },
    }
}

#[derive(Clone)]
pub(crate) struct RuleChoice {
    pub(crate) id: String,
    pub(crate) name: String,
}

/// Rules offered as targets in the editor, including the rule being edited: a pre-condition
/// "this rule is not running" is a valid way to keep a rule single-instance.
pub(crate) fn rule_choices(state: &GuiState) -> Vec<RuleChoice> {
    let rules: Vec<Rule> = state
        .daemon_state
        .as_ref()
        .map(|daemon| daemon.config.rules.clone())
        .unwrap_or_else(|| {
            crate::config::RulesFile::read_from(&crate::config::AppConfig::config_dir()).rules
        });
    rules
        .into_iter()
        .filter(|rule| !rule.id.is_empty())
        .map(|rule| RuleChoice {
            id: rule.id,
            name: rule.name,
        })
        .collect()
}

pub(crate) fn render_rule_choice_combo(
    ui: &mut Ui,
    choices: &[RuleChoice],
    rule_id: &mut String,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    loc: &crate::i18n::Strings,
    t: &Tokens,
) {
    // An unset target is displayed as the first available rule but selected only by the
    // user: opening a rule must not write into the draft.
    let selected_label = match choices.iter().find(|choice| choice.id == *rule_id) {
        Some(choice) => choice.name.clone(),
        None => match (rule_id.is_empty(), choices.first()) {
            (true, Some(first)) => first.name.clone(),
            (true, None) => loc.inspector_rule_no_targets.to_string(),
            (false, _) => format!("{} ({})", loc.inspector_rule_missing, rule_id),
        },
    };

    widgets::hud(ui, loc.inspector_rule_target, t.muted);
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(RichText::new(selected_label).size(12.0))
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            if choices.is_empty() {
                ui.label(RichText::new(loc.inspector_rule_no_targets).size(12.0).color(t.muted));
            }
            for choice in choices {
                if ui.selectable_label(choice.id == *rule_id, &choice.name).clicked() {
                    *rule_id = choice.id.clone();
                }
            }
        });
}

/// Target-rule picker shared by both modes of `Action::RuleControl`. The edited rule itself is
/// hidden: a rule may stop itself, but offering it as a call target is only a trap.
pub(crate) fn render_rule_target_picker(
    ui: &mut Ui,
    state: &GuiState,
    rule_id: &mut String,
    idx: usize,
    t: &Tokens,
) {
    let editing_id = state
        .editing_rule
        .as_ref()
        .map(|rule| rule.id.as_str())
        .unwrap_or_default();
    let choices: Vec<RuleChoice> = rule_choices(state)
        .into_iter()
        .filter(|choice| choice.id != editing_id)
        .collect();
    render_rule_choice_combo(
        ui,
        &choices,
        rule_id,
        ("rule_control_target", idx),
        state.loc(),
        t,
    );
}

/// Display label for every `ArchiveFormat` variant. Exhaustively matched so a new
/// model variant cannot be rendered without a label.
pub(crate) fn archive_format_label(format: crate::models::ArchiveFormat, loc: &crate::i18n::Strings) -> &'static str {
    use crate::models::ArchiveFormat;
    match format {
        ArchiveFormat::Zip => loc.inspector_archive_fmt_zip,
        ArchiveFormat::SevenZip => loc.inspector_archive_fmt_7z,
        ArchiveFormat::TarGz => loc.inspector_archive_fmt_tar_gz,
        ArchiveFormat::Tar => loc.inspector_archive_fmt_tar,
        ArchiveFormat::TarXz => loc.inspector_archive_fmt_tar_xz,
        ArchiveFormat::Rar => loc.inspector_archive_fmt_rar,
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    use crate::models::RuleControlOperation;


    /// Every summary word comes from the catalog: the same action must read differently in EN
    /// and RU, with no unfilled template placeholder and no raw `{:?}` enum name.
    #[test]
    fn test_action_summary_preview_is_localized() {
        let en = crate::i18n::get(crate::i18n::Language::English);
        let ru = crate::i18n::get(crate::i18n::Language::Russian);

        let cases = [
            Action::SystemPower { power_type: SystemPowerType::Shutdown },
            Action::PowerPlan { plan: PowerPlanType::HighPerformance },
            Action::Notification { title: "Build".into(), message: String::new() },
            Action::PlaySound {
                file_path: String::new(),
                play_beep: true,
                beep_frequency: Some(750),
                beep_duration_ms: None,
            },
            Action::Wait {
                mode: crate::models::WaitMode::Timer,
                seconds: 2.5,
                condition: None,
                timeout_sec: 0.0,
                timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
            },
            Action::Display {
                width: Some(1920),
                height: Some(1080),
                refresh_rate: Some(144),
                monitor_index: Some(2),
            },
            Action::AudioFormat {
                target: crate::models::AudioTarget::Microphone,
                device_name: None,
                sample_rate: 48000,
                bit_depth: 24,
                channels: 2,
            },
            Action::IfElse {
                conditions: Vec::new(),
                then_actions: vec![],
                else_if_branches: Vec::new(),
                else_actions: Vec::new(),
            },
            Action::RuleControl { rule_id: "rule_a".into(), operation: RuleControlOperation::Call, wait: true },
            Action::DiscordRpcClear,
        ];

        for action in &cases {
            let en_text = action_summary_preview(action, en);
            let ru_text = action_summary_preview(action, ru);
            assert!(!en_text.is_empty(), "{action:?} has an empty EN summary");
            assert!(!ru_text.is_empty(), "{action:?} has an empty RU summary");
            assert!(!en_text.contains("{}"), "unfilled placeholder in {en_text}");
            assert!(!ru_text.contains("{}"), "unfilled placeholder in {ru_text}");
            assert_ne!(en_text, ru_text, "{action:?} renders the same text in both languages");
            assert!(!en_text.contains("HighPerformance"), "raw enum name leaked: {en_text}");
            assert!(!ru_text.contains("HighPerformance"), "raw enum name leaked: {ru_text}");
        }

        // The enum labels are the ones the parameter editor shows, not their Debug names.
        let power = Action::SystemPower { power_type: SystemPowerType::Shutdown };
        assert_eq!(
            action_summary_preview(&power, en),
            en.summary_preview_power.replace("{}", en.inspector_power_shutdown)
        );
        let plan = Action::PowerPlan { plan: PowerPlanType::HighPerformance };
        assert_eq!(
            action_summary_preview(&plan, en),
            en.summary_preview_power_plan.replace("{}", en.inspector_power_plan_high)
        );
    }
}
