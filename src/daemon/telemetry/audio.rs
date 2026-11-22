use std::time::Duration;

#[cfg(windows)]
use crate::win_com::{Apartment, ComApartment};

/// Reads the default render endpoint's master volume and mute state. `None` means the endpoint
/// could not be queried (no audio device, COM unavailable), which overlays must display as n/a.
#[cfg(windows)]
pub(super) fn get_master_volume_state() -> Option<(u8, bool)> {
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{eMultimedia, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

    let _com = ComApartment::init(Apartment::MultiThreaded);
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.ok()?;
    let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia) }.ok()?;
    let endpoint = unsafe { device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) }.ok()?;
    let scalar = unsafe { endpoint.GetMasterVolumeLevelScalar() }.ok()?;
    let muted = unsafe { endpoint.GetMute() }.map(|flag| flag.as_bool()).unwrap_or(false);
    let volume = (scalar * 100.0).round().clamp(0.0, 100.0) as u8;
    Some((volume, muted))
}

// ---------------- MEDIA HELPER ----------------

/// Volume and media keys are E0-prefixed; they need the extended flag in addition to the scan
/// code so players receive a real WM_APPCOMMAND.
pub(super) fn media_key_is_extended(vk: u8) -> bool {
    matches!(vk, 0xA6..=0xB7)
}

pub fn simulate_media_key(vk: u8) {
    if !crate::daemon::actions::INPUT_INJECTION_ENABLED {
        return;
    }
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
            KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MAPVK_VK_TO_VSC, VIRTUAL_KEY,
        };

        let scan = MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) as u16;
        let mut flags = KEYBD_EVENT_FLAGS(0);
        if scan != 0 {
            flags |= KEYEVENTF_SCANCODE;
        }
        if media_key_is_extended(vk) {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }

        let down = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk as u16),
                    wScan: scan,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let mut up = down;
        up.Anonymous.ki.dwFlags |= KEYEVENTF_KEYUP;

        let _ = SendInput(&[down], std::mem::size_of::<INPUT>() as i32);
        // Hold long enough for DirectInput-style players to register the press.
        std::thread::sleep(Duration::from_millis(35));
        let _ = SendInput(&[up], std::mem::size_of::<INPUT>() as i32);
    }
}

pub fn is_system_audio_playing() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
        use windows::Win32::Media::Audio::{eMultimedia, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
        use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

        let _com = ComApartment::init(Apartment::MultiThreaded);
        let mut peak = 0.0f32;
        if let Ok(enumerator) = unsafe {
            CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
        } {
            if let Ok(device) = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia) } {
                if let Ok(meter) = unsafe { device.Activate::<IAudioMeterInformation>(CLSCTX_ALL, None) } {
                    if let Ok(value) = unsafe { meter.GetPeakValue() } {
                        peak = value;
                    }
                }
            }
        }
        peak > 0.0005
    }
}
