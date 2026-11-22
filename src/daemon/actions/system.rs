use std::collections::HashMap;
use std::process::Command;

use super::support::command::{
    apply_no_window, run_command_bounded_async, run_system_tool, UTILITY_TIMEOUT,
};
use super::ActionRunner;
use crate::models::{Action, PowerPlanType, SystemPowerType};

#[cfg(windows)]
use super::support::command::{powershell_helper_command, ps_single_quoted};
#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
#[cfg(windows)]
use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_FILENAME};
#[cfg(windows)]
use windows::Win32::System::Shutdown::LockWorkStation;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    SendMessageTimeoutW, SMTO_ABORTIFHUNG, HWND_BROADCAST, WM_SYSCOMMAND,
};

pub(super) async fn execute(action: &Action, context: &HashMap<String, String>) -> Result<(), String> {
    match action {
        Action::PowerPlan { plan } => {
            set_power_plan(*plan).await?;
            Ok(())
        }

        Action::PlaySound { file_path, play_beep, beep_frequency, beep_duration_ms } => {
            if *play_beep {
                #[cfg(windows)]
                unsafe {
                    if let Some(freq) = beep_frequency {
                        let f = (*freq).clamp(37, 32767);
                        let d = beep_duration_ms.unwrap_or(200).clamp(10, 10000);
                        let _ = windows::Win32::System::Diagnostics::Debug::Beep(f, d);
                    } else {
                        let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
                            windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_STYLE(0),
                        );
                    }
                }
            } else {
                let path = ActionRunner::interpolate_tokens(file_path, context);
                play_sound_file(&path)?;
            }
            Ok(())
        }

        Action::Notification { title, message } => {
            let t = ActionRunner::interpolate_tokens(title, context);
            let m = ActionRunner::interpolate_tokens(message, context);

            #[cfg(windows)]
            {
                let escaped_title = ps_single_quoted(&t);
                let escaped_msg = ps_single_quoted(&m);
                let ps_script = format!(
                    "Add-Type -AssemblyName System.Windows.Forms; \
                         $notify = New-Object System.Windows.Forms.NotifyIcon; \
                         $notify.Icon = [System.Drawing.SystemIcons]::Information; \
                         $notify.Visible = $true; \
                         $notify.ShowBalloonTip(4000, '{escaped_title}', '{escaped_msg}', [System.Windows.Forms.ToolTipIcon]::Info); \
                         Start-Sleep -Milliseconds 2500; \
                         $notify.Dispose()"
                );

                let out = run_command_bounded_async(
                    powershell_helper_command(&ps_script),
                    UTILITY_TIMEOUT,
                )
                .await?;
                if out.timed_out {
                    return Err(format!(
                        "Notification helper timed out after {}s",
                        UTILITY_TIMEOUT.as_secs()
                    ));
                }
                if !out.succeeded() {
                    return Err(format!(
                        "Notification failed (exit code {:?})",
                        out.exit_code()
                    ));
                }
            }
            Ok(())
        }

        Action::SystemPower { power_type } => {
            match power_type {
                SystemPowerType::Lock => {
                    #[cfg(windows)]
                    unsafe {
                        let _ = LockWorkStation();
                    }
                }
                SystemPowerType::TurnScreenOff => {
                    #[cfg(windows)]
                    unsafe {
                        const SC_MONITORPOWER: usize = 0xF170;
                        // 2 = turn monitor off. SendMessageTimeoutW + ABORTIFHUNG keeps
                        // one hung top-level window from stalling the rule pipeline.
                        let mut result = 0usize;
                        let _ = SendMessageTimeoutW(
                            HWND_BROADCAST,
                            WM_SYSCOMMAND,
                            WPARAM(SC_MONITORPOWER),
                            LPARAM(2),
                            SMTO_ABORTIFHUNG,
                            1000,
                            Some(&mut result),
                        );
                    }
                }
                SystemPowerType::Sleep => {
                    run_system_tool("rundll32.exe", &["powrprof.dll,SetSuspendState", "0,1,0"]).await?;
                }
                SystemPowerType::Restart => {
                    run_system_tool("shutdown", &["/r", "/t", "0"]).await?;
                }
                SystemPowerType::Shutdown => {
                    run_system_tool("shutdown", &["/s", "/t", "0"]).await?;
                }
            }
            Ok(())
        }

        Action::Display { width, height, refresh_rate, monitor_index } => {
            #[cfg(windows)]
            {
                change_display_settings(*width, *height, *refresh_rate, *monitor_index)?;
            }
            Ok(())
        }
        _ => unreachable!("system handler received a non-system action"),
    }
}

pub(super) async fn execute_discord(action: &Action, context: &HashMap<String, String>) -> Result<(), String> {
    match action {
        Action::DiscordRpcSet {
            app_id,
            details,
            state,
            large_image,
            large_text,
            small_image,
            small_text,
            show_elapsed,
            buttons,
            ..
        } => {
            let app_id_str = ActionRunner::interpolate_tokens(app_id, context);
            let details_str = ActionRunner::interpolate_tokens(details, context);
            let state_str = ActionRunner::interpolate_tokens(state, context);
            let large_image_str = large_image.as_ref().map(|s| ActionRunner::interpolate_tokens(s, context));
            let large_text_str = large_text.as_ref().map(|s| ActionRunner::interpolate_tokens(s, context));
            let small_image_str = small_image.as_ref().map(|s| ActionRunner::interpolate_tokens(s, context));
            let small_text_str = small_text.as_ref().map(|s| ActionRunner::interpolate_tokens(s, context));
            let buttons_interp: Vec<crate::models::DiscordRpcButton> = buttons
                .iter()
                .map(|b| crate::models::DiscordRpcButton {
                    label: ActionRunner::interpolate_tokens(&b.label, context),
                    url: ActionRunner::interpolate_tokens(&b.url, context),
                })
                .collect();

            crate::daemon::discord_rpc::update_presence(
                &app_id_str,
                &details_str,
                &state_str,
                large_image_str.as_deref(),
                large_text_str.as_deref(),
                small_image_str.as_deref(),
                small_text_str.as_deref(),
                *show_elapsed,
                &buttons_interp,
            ).await?;
            Ok(())
        }

        Action::DiscordRpcClear => {
            crate::daemon::discord_rpc::clear_presence().await?;
            Ok(())
        }
        _ => unreachable!("discord handler received a non-discord action"),
    }
}

#[cfg(windows)]
fn change_display_settings(
    width: Option<u32>,
    height: Option<u32>,
    refresh_rate: Option<u32>,
    monitor_index: Option<u32>,
) -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::{
        ChangeDisplaySettingsExW, EnumDisplayDevicesW, EnumDisplaySettingsW, CDS_UPDATEREGISTRY,
        DEVMODEW, DISPLAY_DEVICEW, DISP_CHANGE_SUCCESSFUL, DM_DISPLAYFREQUENCY, DM_PELSHEIGHT,
        DM_PELSWIDTH, ENUM_CURRENT_SETTINGS,
    };

    unsafe {
        let mut dm: DEVMODEW = std::mem::zeroed();
        dm.dmSize = std::mem::size_of::<DEVMODEW>() as u16;

        let dev_name_buf: Option<Vec<u16>> = if let Some(m_idx) = monitor_index {
            if m_idx > 0 {
                let mut dd: DISPLAY_DEVICEW = std::mem::zeroed();
                dd.cb = std::mem::size_of::<DISPLAY_DEVICEW>() as u32;
                let target_idx = m_idx - 1;
                if EnumDisplayDevicesW(None, target_idx, &mut dd, 0).as_bool() {
                    Some(dd.DeviceName.to_vec())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let dev_ptr = dev_name_buf.as_ref().map(|v| PCWSTR(v.as_ptr())).unwrap_or(PCWSTR::null());

        if !EnumDisplaySettingsW(dev_ptr, ENUM_CURRENT_SETTINGS, &mut dm).as_bool() {
            return Err(format!("Failed to query current display settings for monitor {:?}", monitor_index));
        }

        if let (Some(w), Some(h)) = (width, height) {
            dm.dmPelsWidth = w;
            dm.dmPelsHeight = h;
            dm.dmFields |= DM_PELSWIDTH | DM_PELSHEIGHT;
        }

        if let Some(hz) = refresh_rate {
            dm.dmDisplayFrequency = hz;
            dm.dmFields |= DM_DISPLAYFREQUENCY;
        }

        let res = ChangeDisplaySettingsExW(dev_ptr, Some(&dm), None, CDS_UPDATEREGISTRY, None);
        if res == DISP_CHANGE_SUCCESSFUL {
            Ok(())
        } else {
            Err(format!("ChangeDisplaySettings failed with status {:?}", res))
        }
    }
}

async fn set_power_plan(plan: PowerPlanType) -> Result<(), String> {
    let guid = match plan {
        PowerPlanType::HighPerformance => "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c",
        PowerPlanType::Balanced => "381b4222-f694-41f0-9685-ff5bb577002f",
        PowerPlanType::PowerSaver => "a1841308-3541-4fab-bc81-f71556f20b4a",
    };
    run_system_tool("powercfg", &["/setactive", guid]).await
}

#[cfg(windows)]
extern "system" {
    fn mciSendStringW(
        lpstrcommand: windows::core::PCWSTR,
        lpstrreturnstring: windows::core::PWSTR,
        ucmbreturn: u32,
        hwndcallback: windows::Win32::Foundation::HWND,
    ) -> u32;
}

fn play_sound_file(path: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        let lower = path.to_lowercase();
        if lower.ends_with(".wav") {
            let path_w: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
            unsafe {
                let _ = PlaySoundW(PCWSTR(path_w.as_ptr()), None, SND_FILENAME | SND_ASYNC);
            }
        } else {
            let close_cmd: Vec<u16> = "close ph_media\0".encode_utf16().collect();
            let open_cmd_str = format!("open \"{}\" alias ph_media\0", path);
            let open_cmd: Vec<u16> = open_cmd_str.encode_utf16().collect();
            let play_cmd: Vec<u16> = "play ph_media from 0\0".encode_utf16().collect();
            unsafe {
                let _ = mciSendStringW(PCWSTR(close_cmd.as_ptr()), windows::core::PWSTR::null(), 0, HWND(std::ptr::null_mut()));
                let res = mciSendStringW(PCWSTR(open_cmd.as_ptr()), windows::core::PWSTR::null(), 0, HWND(std::ptr::null_mut()));
                if res == 0 {
                    let _ = mciSendStringW(PCWSTR(play_cmd.as_ptr()), windows::core::PWSTR::null(), 0, HWND(std::ptr::null_mut()));
                } else {
                    let path_w: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
                    let _ = PlaySoundW(PCWSTR(path_w.as_ptr()), None, SND_FILENAME | SND_ASYNC);
                }
            }
        }
    }
    Ok(())
}

pub(super) async fn set_firewall_rule(proc_name: &str, action: crate::models::FirewallAction) -> Result<(), String> {
    let rule_name = format!("Synector_{}", proc_name);
    let exe_path = crate::daemon::watcher::ProcessWatcher::find_exe_path_by_name(proc_name)
        .unwrap_or_else(|| proc_name.to_string());
    let should_block = match action {
        crate::models::FirewallAction::Block => true,
        crate::models::FirewallAction::Allow => false,
        crate::models::FirewallAction::Toggle => !super::is_firewall_rule_active(&rule_name).await,
    };
    let mut cmd = Command::new("netsh");
    if should_block {
        cmd.args([
            "advfirewall", "firewall", "add", "rule",
            &format!("name={}", rule_name),
            "dir=out",
            "action=block",
            &format!("program={}", exe_path),
        ]);
    } else {
        cmd.args(["advfirewall", "firewall", "delete", "rule", &format!("name={}", rule_name)]);
    }
    apply_no_window(&mut cmd);
    let out = run_command_bounded_async(cmd, UTILITY_TIMEOUT).await?;
    if out.timed_out {
        return Err(format!("netsh timed out after {}s", UTILITY_TIMEOUT.as_secs()));
    }
    if !out.succeeded() {
        return Err(format!("netsh failed with exit code {:?}", out.exit_code()));
    }
    Ok(())
}

pub async fn is_firewall_rule_active(rule_name: &str) -> bool {
    let mut cmd = Command::new("netsh");
    cmd.args(["advfirewall", "firewall", "show", "rule", &format!("name={}", rule_name)]);
    apply_no_window(&mut cmd);
    run_command_bounded_async(cmd, UTILITY_TIMEOUT)
        .await
        .map(|out| out.succeeded())
        .unwrap_or(false)
}
