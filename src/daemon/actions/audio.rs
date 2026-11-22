use std::collections::HashMap;

use super::ActionRunner;
use crate::models::{Action, AudioVolumeAction};

#[cfg(windows)]
use crate::win_com::{Apartment, ComApartment};
#[cfg(windows)]
use super::support::command::{
    powershell_helper_command, ps_single_quoted, run_command_bounded_async, AUDIO_HELPER_TIMEOUT,
    CommandOutcome,
};

pub(super) async fn execute(action: &Action, context: &HashMap<String, String>) -> Result<(), String> {
    match action {
        Action::AudioControl {
            target,
            process_name,
            action_type,
            volume_level,
            left_volume,
            right_volume,
        } => {
            match target {
                crate::models::AudioTarget::Playback => {
                    #[cfg(windows)]
                    {
                        set_audio_volume_or_mute(*action_type, *volume_level, *left_volume, *right_volume)?;
                    }
                }
                crate::models::AudioTarget::Microphone => {
                    #[cfg(windows)]
                    {
                        set_microphone_volume_or_mute(*action_type, *volume_level)?;
                    }
                }
                crate::models::AudioTarget::Process => {
                    let p_name = process_name.as_deref().unwrap_or("");
                    let target_proc = ActionRunner::interpolate_tokens(p_name, context);
                    let target_pid = context.get("ProcessId").and_then(|p| p.parse::<u32>().ok());
                    #[cfg(windows)]
                    {
                        // Retry up to 15 times with 150ms delay (~2.2s total) in case audio session is not yet registered
                        let mut matched = false;
                        for attempt in 0..15 {
                            match set_process_audio(&target_proc, target_pid, *action_type, *volume_level) {
                                Ok(found) => {
                                    if found {
                                        matched = true;
                                        break;
                                    }
                                }
                                Err(e) => return Err(e),
                            }
                            if attempt < 14 {
                                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                            }
                        }
                        if !matched && !target_proc.is_empty() && target_proc != "*" && target_proc != "current" {
                            eprintln!("No audio session found for process '{}'", target_proc);
                        }
                    }
                }
            }
            Ok(())
        }

        Action::AudioFormat {
            target,
            device_name,
            sample_rate,
            bit_depth,
            channels,
        } => {
            let dev_name = device_name.as_ref().map(|d| ActionRunner::interpolate_tokens(d, context));
            #[cfg(windows)]
            {
                set_audio_device_format(*target, *sample_rate, *bit_depth, *channels, dev_name.as_deref()).await?;
            }
            Ok(())
        }

        Action::DefaultDevice { target, device_name } => {
            let name = ActionRunner::interpolate_tokens(device_name, context);
            #[cfg(windows)]
            {
                match target {
                    crate::models::AudioTarget::Playback => {
                        set_default_audio_device(&name).await?;
                    }
                    crate::models::AudioTarget::Microphone => {
                        set_default_microphone_device(&name).await?;
                    }
                    crate::models::AudioTarget::Process => {}
                }
            }
            Ok(())
        }
        _ => unreachable!("audio handler received a non-audio action"),
    }
}

/// Maps the helper script's exit codes to an action error. A stock Windows system does not
/// have the third-party AudioDeviceCmdlets module, and pretending the change happened is
/// worse than reporting it.
#[cfg(windows)]
fn default_device_helper_result(
    out: CommandOutcome,
    what: &str,
    name: &str,
) -> Result<(), String> {
    if out.timed_out {
        return Err(format!(
            "Setting the {what} timed out after {}s",
            AUDIO_HELPER_TIMEOUT.as_secs()
        ));
    }
    match out.exit_code() {
        Some(0) => Ok(()),
        Some(3) => Err(format!(
            "The AudioDeviceCmdlets PowerShell module is not installed; the {what} cannot be changed"
        )),
        Some(4) => Err(format!("No audio device matches '{name}'")),
        code => Err(format!("Setting the {what} failed (exit code {code:?})")),
    }
}

#[cfg(windows)]
async fn set_default_audio_device(name: &str) -> Result<(), String> {
    let ps_cmd = format!(
        r#"$ErrorActionPreference = 'Stop'
$n = '{0}'
if (-not (Get-Command Set-AudioDevice -ErrorAction SilentlyContinue)) {{ exit 3 }}
$dev = Get-AudioDevice -List | Where-Object {{ $_.Name -like "*$n*" }} | Select-Object -First 1
if (-not $dev) {{ exit 4 }}
$dev | Set-AudioDevice
"#,
        ps_single_quoted(name)
    );
    let out = run_command_bounded_async(powershell_helper_command(&ps_cmd), AUDIO_HELPER_TIMEOUT)
        .await?;
    default_device_helper_result(out, "default audio device", name)
}

#[cfg(windows)]
async fn set_default_microphone_device(name: &str) -> Result<(), String> {
    let ps_cmd = format!(
        r#"$ErrorActionPreference = 'Stop'
$n = '{0}'
if (-not (Get-Command Set-AudioDevice -ErrorAction SilentlyContinue)) {{ exit 3 }}
$dev = Get-AudioDevice -List | Where-Object {{ (($_.Type -eq 'Recording') -or ($_.Type -like '*Capture*') -or ($_.Name -like "*$n*")) -and ($_.Name -like "*$n*") }} | Select-Object -First 1
if (-not $dev) {{ exit 4 }}
$dev | Set-AudioDevice
"#,
        ps_single_quoted(name)
    );
    let out = run_command_bounded_async(powershell_helper_command(&ps_cmd), AUDIO_HELPER_TIMEOUT)
        .await?;
    default_device_helper_result(out, "default microphone", name)
}

#[cfg(windows)]
pub(super) fn construct_waveformat_extensible(sample_rate: u32, bit_depth: u16, channels: u16) -> [u8; 40] {
    let mut bytes = [0u8; 40];
    let block_align = channels * (bit_depth / 8);
    let avg_bytes_per_sec = sample_rate * (block_align as u32);
    let channel_mask: u32 = if channels == 1 { 0x4 } else { 0x3 };

    bytes[0..2].copy_from_slice(&0xFFFEu16.to_le_bytes());
    bytes[2..4].copy_from_slice(&channels.to_le_bytes());
    bytes[4..8].copy_from_slice(&sample_rate.to_le_bytes());
    bytes[8..12].copy_from_slice(&avg_bytes_per_sec.to_le_bytes());
    bytes[12..14].copy_from_slice(&block_align.to_le_bytes());
    bytes[14..16].copy_from_slice(&bit_depth.to_le_bytes());
    bytes[16..18].copy_from_slice(&22u16.to_le_bytes());
    bytes[18..20].copy_from_slice(&bit_depth.to_le_bytes());
    bytes[20..24].copy_from_slice(&channel_mask.to_le_bytes());

    let pcm_guid: [u8; 16] = [
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00,
        0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71
    ];
    bytes[24..40].copy_from_slice(&pcm_guid);
    bytes
}

#[cfg(windows)]
fn audio_data_flow(target: crate::models::AudioTarget) -> windows::Win32::Media::Audio::EDataFlow {
    use windows::Win32::Media::Audio::{eCapture, eRender};
    match target {
        crate::models::AudioTarget::Microphone => eCapture,
        _ => eRender,
    }
}

#[cfg(windows)]
fn audio_flow_registry_dir(target: crate::models::AudioTarget) -> &'static str {
    match target {
        crate::models::AudioTarget::Microphone => "Capture",
        _ => "Render",
    }
}

#[cfg(windows)]
fn get_default_audio_endpoint(
    enumerator: &windows::Win32::Media::Audio::IMMDeviceEnumerator,
    target: crate::models::AudioTarget,
) -> Result<windows::Win32::Media::Audio::IMMDevice, String> {
    use windows::Win32::Media::Audio::{eCapture, eCommunications, eConsole, eMultimedia};
    let result = if matches!(target, crate::models::AudioTarget::Microphone) {
        unsafe {
            enumerator
                .GetDefaultAudioEndpoint(eCapture, eConsole)
                .or_else(|_| enumerator.GetDefaultAudioEndpoint(eCapture, eCommunications))
                .or_else(|_| enumerator.GetDefaultAudioEndpoint(eCapture, eMultimedia))
        }
    } else {
        unsafe { enumerator.GetDefaultAudioEndpoint(audio_data_flow(target), eMultimedia) }
    };
    result.map_err(|e| format!("GetDefaultAudioEndpoint failed: {e}"))
}

/// Splits an endpoint id such as `{0.0.1.00000000}.{guid}` into its registry key part.
pub(super) fn endpoint_registry_guid(device_id: &str) -> Option<&str> {
    let guid = device_id.rsplit('.').next()?.trim();
    if guid.starts_with('{') && guid.ends_with('}') {
        Some(guid)
    } else {
        None
    }
}

/// Matches a requested device name against the display names collected from the endpoint
/// registry (friendly name, device description and adapter name).
pub(super) fn device_name_matches(requested: &str, candidates: &[String]) -> bool {
    let requested = requested.trim().to_lowercase();
    if requested.is_empty() {
        return false;
    }
    candidates.iter().any(|candidate| {
        let candidate = candidate.to_lowercase();
        !candidate.is_empty()
            && (candidate.contains(&requested) || requested.contains(&candidate))
    })
}

#[cfg(windows)]
fn read_mmdevice_property(flow_dir: &str, device_id: &str, value_name: &str) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};

    let guid = endpoint_registry_guid(device_id)?;
    let subkey = format!(
        "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\MMDevices\\Audio\\{flow_dir}\\{guid}\\Properties"
    );
    let subkey_wide: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
    let value_wide: Vec<u16> = value_name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut buffer = [0u16; 512];
    let mut size = (buffer.len() * std::mem::size_of::<u16>()) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(subkey_wide.as_ptr()),
            PCWSTR(value_wide.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status != windows::Win32::Foundation::WIN32_ERROR(0) {
        return None;
    }
    let length = ((size as usize) / 2).min(buffer.len());
    let text = String::from_utf16_lossy(&buffer[..length]);
    let text = text.trim_end_matches('\0').trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// Display names for one endpoint, closest to what `Get-AudioDevice -List` reports:
/// the device description with the adapter name in parentheses, plus both parts alone.
#[cfg(windows)]
fn audio_endpoint_display_names(flow_dir: &str, device_id: &str) -> Vec<String> {
    const PKEY_DEVICE_DESC: &str = "{a45c254e-df1c-4efd-8020-67d146a850e0},2";
    const PKEY_ADAPTER_NAME: &str = "{b3f8fa53-0004-438e-9003-51a46e139bfc},6";

    let desc = read_mmdevice_property(flow_dir, device_id, PKEY_DEVICE_DESC);
    let adapter = read_mmdevice_property(flow_dir, device_id, PKEY_ADAPTER_NAME);
    let mut names = Vec::new();
    if let (Some(desc), Some(adapter)) = (&desc, &adapter) {
        names.push(format!("{desc} ({adapter})"));
    }
    if let Some(desc) = desc {
        names.push(desc);
    }
    if let Some(adapter) = adapter {
        names.push(adapter);
    }
    names
}

#[cfg(windows)]
fn find_audio_endpoint_by_name(
    enumerator: &windows::Win32::Media::Audio::IMMDeviceEnumerator,
    target: crate::models::AudioTarget,
    requested: &str,
) -> Result<windows::Win32::Media::Audio::IMMDevice, String> {
    use windows::Win32::Media::Audio::DEVICE_STATE_ACTIVE;

    let collection = unsafe { enumerator.EnumAudioEndpoints(audio_data_flow(target), DEVICE_STATE_ACTIVE) }
        .map_err(|e| format!("EnumAudioEndpoints failed: {e}"))?;
    let count = unsafe { collection.GetCount() }.map_err(|e| format!("GetCount failed: {e}"))?;
    let mut available = Vec::new();
    for index in 0..count {
        let device = match unsafe { collection.Item(index) } {
            Ok(device) => device,
            Err(_) => continue,
        };
        let device_id = match unsafe { device.GetId() } {
            Ok(id) => unsafe { id.to_string() }.unwrap_or_default(),
            Err(_) => continue,
        };
        let names = audio_endpoint_display_names(audio_flow_registry_dir(target), &device_id);
        if device_name_matches(requested, &names) {
            return Ok(device);
        }
        if let Some(name) = names.into_iter().next() {
            available.push(name);
        }
    }
    let mut message = format!("No audio device matches '{requested}'");
    if !available.is_empty() {
        available.truncate(5);
        message.push_str("; available: ");
        message.push_str(&available.join(", "));
    }
    Err(message)
}

/// Runs `f` against a default or explicitly named audio endpoint, owning the COM
/// apartment and enumerator lifetime for the call.
#[cfg(windows)]
fn with_audio_endpoint<T>(
    target: crate::models::AudioTarget,
    device_name: Option<&str>,
    f: impl FnOnce(&windows::Win32::Media::Audio::IMMDevice) -> Result<T, String>,
) -> Result<T, String> {
    use windows::Win32::Media::Audio::{IMMDeviceEnumerator, MMDeviceEnumerator};
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

    let _com = ComApartment::init(Apartment::MultiThreaded);
    let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
        .map_err(|e| format!("CoCreateInstance MMDeviceEnumerator failed: {e}"))?;
    let device = match device_name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => find_audio_endpoint_by_name(&enumerator, target, name)?,
        None => get_default_audio_endpoint(&enumerator, target)?,
    };
    f(&device)
}

#[cfg(windows)]
fn with_default_endpoint_volume<T>(
    target: crate::models::AudioTarget,
    f: impl FnOnce(&windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume) -> Result<T, String>,
) -> Result<T, String> {
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::System::Com::CLSCTX_ALL;

    with_audio_endpoint(target, None, |device| {
        let endpoint: IAudioEndpointVolume = unsafe { device.Activate(CLSCTX_ALL, None) }
            .map_err(|e| format!("Activate IAudioEndpointVolume failed: {e}"))?;
        f(&endpoint)
    })
}

#[cfg(windows)]
fn apply_endpoint_volume_action(
    endpoint: &windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume,
    action_type: AudioVolumeAction,
    volume_level: u8,
    left_volume: Option<u8>,
    right_volume: Option<u8>,
) -> Result<(), String> {
    match action_type {
        AudioVolumeAction::Mute => {
            unsafe { endpoint.SetMute(true, std::ptr::null()) }
                .map_err(|e| format!("SetMute failed: {e}"))
        }
        AudioVolumeAction::Unmute => {
            unsafe { endpoint.SetMute(false, std::ptr::null()) }
                .map_err(|e| format!("SetMute failed: {e}"))
        }
        AudioVolumeAction::ToggleMute => {
            let current = unsafe { endpoint.GetMute() }.map_err(|e| format!("GetMute failed: {e}"))?;
            unsafe { endpoint.SetMute(!current.as_bool(), std::ptr::null()) }
                .map_err(|e| format!("SetMute failed: {e}"))
        }
        AudioVolumeAction::SetVolume => {
            let channel_count = unsafe { endpoint.GetChannelCount() }.unwrap_or(2);
            let mut applied_channel = false;
            if let Some(left) = left_volume {
                if channel_count > 0 {
                    let scalar = (left.min(100) as f32) / 100.0;
                    unsafe {
                        let _ = endpoint.SetChannelVolumeLevelScalar(0, scalar, std::ptr::null());
                    }
                    applied_channel = true;
                }
            }
            if let Some(right) = right_volume {
                if channel_count > 1 {
                    let scalar = (right.min(100) as f32) / 100.0;
                    unsafe {
                        let _ = endpoint.SetChannelVolumeLevelScalar(1, scalar, std::ptr::null());
                    }
                    applied_channel = true;
                }
            }
            if !applied_channel {
                let scalar = (volume_level.min(100) as f32) / 100.0;
                unsafe { endpoint.SetMasterVolumeLevelScalar(scalar, std::ptr::null()) }
                    .map_err(|e| format!("SetMasterVolumeLevelScalar failed: {e}"))?;
            }
            Ok(())
        }
        AudioVolumeAction::PlayBeep => Ok(()),
    }
}

#[cfg(windows)]
async fn set_audio_device_format(
    target: crate::models::AudioTarget,
    sample_rate: u32,
    bit_depth: u32,
    channels: u32,
    device_name: Option<&str>,
) -> Result<(), String> {
    let sample_rate = sample_rate.clamp(8000, 384000);
    let bit_depth = (bit_depth as u16).clamp(8, 32);
    let channels = (channels as u16).clamp(1, 8);

    let wave_bytes = construct_waveformat_extensible(sample_rate, bit_depth, channels);

    let dev_id = with_audio_endpoint(target, device_name, |device| {
        let dev_id_pwstr = unsafe { device.GetId() }.map_err(|e| format!("GetId failed: {e}"))?;
        unsafe { dev_id_pwstr.to_string() }.map_err(|e| format!("DevId parse: {e}"))
    })?;

    let flow_dir = audio_flow_registry_dir(target);
    let byte_list = wave_bytes.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(",");
    let ps_script = format!(
        r#"
                $ErrorActionPreference = 'Stop'
                $id = '{0}'
                $keyPath = "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\{1}\$id\Properties"
                if (-not (Test-Path $keyPath)) {{ exit 4 }}
                $bytes = [byte[]]@({2})
                Set-ItemProperty -Path $keyPath -Name '{{e4870e26-3cc5-4cd2-ba46-56665703c64e}},0' -Value $bytes -Type Binary
                Set-ItemProperty -Path $keyPath -Name '{{e4870e26-3cc5-4cd2-ba46-56665703c64e}},3' -Value $bytes -Type Binary
                Set-ItemProperty -Path $keyPath -Name '{{f19f064d-0824-4d22-8222-99be777a77f0}},0' -Value $bytes -Type Binary
                "#,
        ps_single_quoted(&dev_id),
        flow_dir,
        byte_list
    );

    let out = run_command_bounded_async(powershell_helper_command(&ps_script), AUDIO_HELPER_TIMEOUT)
        .await?;
    if out.timed_out {
        return Err(format!(
            "Audio format helper timed out after {}s",
            AUDIO_HELPER_TIMEOUT.as_secs()
        ));
    }
    match out.exit_code() {
        Some(0) => Ok(()),
        Some(4) => Err("Audio format: the endpoint registry key does not exist".to_string()),
        code => Err(format!("Audio format: helper failed (exit code {code:?})")),
    }
}

#[cfg(windows)]
fn set_audio_volume_or_mute(
    action_type: AudioVolumeAction,
    volume_level: u8,
    left_volume: Option<u8>,
    right_volume: Option<u8>,
) -> Result<(), String> {
    with_default_endpoint_volume(crate::models::AudioTarget::Playback, |endpoint| {
        apply_endpoint_volume_action(endpoint, action_type, volume_level, left_volume, right_volume)
    })
}

pub fn set_master_volume_scalar(scalar: f32) {
    #[cfg(windows)]
    {
        let _ = with_default_endpoint_volume(crate::models::AudioTarget::Playback, |endpoint| {
            unsafe { endpoint.SetMasterVolumeLevelScalar(scalar.clamp(0.0, 1.0), std::ptr::null()) }
                .map_err(|e| format!("SetMasterVolumeLevelScalar failed: {e}"))
        });
    }
}

#[cfg(windows)]
fn set_microphone_volume_or_mute(action_type: AudioVolumeAction, volume_level: u8) -> Result<(), String> {
    with_default_endpoint_volume(crate::models::AudioTarget::Microphone, |endpoint| {
        apply_endpoint_volume_action(endpoint, action_type, volume_level, None, None)
    })
}

#[cfg(windows)]
fn set_process_audio(
    target_process: &str,
    target_pid: Option<u32>,
    action_type: AudioVolumeAction,
    volume_level: u8,
) -> Result<bool, String> {
    use windows::core::Interface;
    use windows::Win32::Media::Audio::{IAudioSessionManager2, IAudioSessionControl2, ISimpleAudioVolume};
    use windows::Win32::System::Com::CLSCTX_ALL;

    with_audio_endpoint(crate::models::AudioTarget::Playback, None, |device| unsafe {
        let session_manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)
            .map_err(|e| format!("Activate IAudioSessionManager2 failed: {e}"))?;

        let session_enumerator = session_manager.GetSessionEnumerator()
            .map_err(|e| format!("GetSessionEnumerator failed: {e}"))?;

        let count = session_enumerator.GetCount()
            .map_err(|e| format!("GetCount failed: {e}"))?;

        let proc_map = crate::daemon::watcher::ProcessWatcher::snapshot_process_map();
        let target_clean = target_process.trim().to_lowercase();
        let is_wildcard_or_current = target_clean == "*" || target_clean == "current" || target_clean.is_empty();

        let mut matched_any = false;

        for i in 0..count {
            let session_control = match session_enumerator.GetSession(i) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let session_control2: IAudioSessionControl2 = match session_control.cast() {
                Ok(c2) => c2,
                Err(_) => continue,
            };

            let pid = match session_control2.GetProcessId() {
                Ok(p) => p,
                Err(_) => continue,
            };

            if pid == 0 {
                continue;
            }

            let matches_pid = target_pid.map(|tp| tp == pid).unwrap_or(false);
            let matches_name = if !target_clean.is_empty() && target_clean != "current" {
                proc_map.get(&pid).map(|name| {
                    name.eq_ignore_ascii_case(&target_clean)
                        || name.trim_end_matches(".exe").eq_ignore_ascii_case(target_clean.trim_end_matches(".exe"))
                }).unwrap_or(false)
            } else {
                false
            };

            if (is_wildcard_or_current && matches_pid) || matches_name || (target_clean == "*" && target_pid.is_none()) {
                let volume: ISimpleAudioVolume = match session_control.cast() {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                match action_type {
                    AudioVolumeAction::Mute => {
                        volume.SetMute(true, std::ptr::null())
                            .map_err(|e| format!("SetMute failed: {e}"))?;
                    }
                    AudioVolumeAction::Unmute => {
                        volume.SetMute(false, std::ptr::null())
                            .map_err(|e| format!("SetMute failed: {e}"))?;
                    }
                    AudioVolumeAction::ToggleMute => {
                        let current = volume.GetMute()
                            .map_err(|e| format!("GetMute failed: {e}"))?;
                        volume.SetMute(!current.as_bool(), std::ptr::null())
                            .map_err(|e| format!("SetMute failed: {e}"))?;
                    }
                    AudioVolumeAction::SetVolume => {
                        let scalar = (volume_level.min(100) as f32) / 100.0;
                        volume.SetMasterVolume(scalar, std::ptr::null())
                            .map_err(|e| format!("SetMasterVolume failed: {e}"))?;
                    }
                    AudioVolumeAction::PlayBeep => {}
                }
                matched_any = true;
            }
        }

        Ok(matched_any)
    })
}
