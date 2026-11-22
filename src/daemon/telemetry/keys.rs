use std::sync::Mutex;
use std::time::Instant;

#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

// ---------------- REAL KEY VISUALIZER ----------------

static LAST_KEYSTROKE: Mutex<Option<(Instant, String)>> = Mutex::new(None);

#[cfg(windows)]
#[inline]
pub fn is_vk_pressed(vk: i32) -> bool {
    unsafe { GetAsyncKeyState(vk) as u16 & 0x8000 != 0 }
}

#[cfg(not(windows))]
#[inline]
pub fn is_vk_pressed(_vk: i32) -> bool {
    false
}

pub fn get_active_or_recent_keys(max_keys: usize, decay_sec: f32) -> String {
    #[cfg(windows)]
    {
        let mut pressed: Vec<String> = Vec::new();
        unsafe {
            if GetAsyncKeyState(0x11) as u16 & 0x8000 != 0 { pressed.push("CTRL".to_string()); }
            if GetAsyncKeyState(0x10) as u16 & 0x8000 != 0 { pressed.push("SHIFT".to_string()); }
            if GetAsyncKeyState(0x12) as u16 & 0x8000 != 0 { pressed.push("ALT".to_string()); }
            if GetAsyncKeyState(0x5B) as u16 & 0x8000 != 0 { pressed.push("WIN".to_string()); }
            if GetAsyncKeyState(0x20) as u16 & 0x8000 != 0 { pressed.push("SPACE".to_string()); }
            if GetAsyncKeyState(0x0D) as u16 & 0x8000 != 0 { pressed.push("ENTER".to_string()); }
            if GetAsyncKeyState(0x1B) as u16 & 0x8000 != 0 { pressed.push("ESC".to_string()); }
            if GetAsyncKeyState(0x09) as u16 & 0x8000 != 0 { pressed.push("TAB".to_string()); }

            // Letter keys A-Z (0x41 ..= 0x5A)
            for vk in 0x41u8..=0x5Au8 {
                if GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0 {
                    pressed.push(format!("{}", vk as char));
                }
            }

            // Function keys F1-F12 (0x70 ..= 0x7B)
            for vk in 0x70u8..=0x7Bu8 {
                if GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0 {
                    let num = vk - 0x70 + 1;
                    pressed.push(format!("F{}", num));
                }
            }
        }

        let now = Instant::now();
        if !pressed.is_empty() {
            let key_str = pressed.iter().take(max_keys).map(|k| format!("[{}]", k)).collect::<Vec<_>>().join(" + ");
            if let Ok(mut lock) = LAST_KEYSTROKE.lock() {
                *lock = Some((now, key_str.clone()));
            }
            return key_str;
        }

        if let Ok(lock) = LAST_KEYSTROKE.lock() {
            if let Some((time, ref key_str)) = *lock {
                if now.duration_since(time).as_secs_f32() <= decay_sec {
                    return key_str.clone();
                }
            }
        }

        "[KEYS IDLE]".to_string()
    }
    #[cfg(not(windows))]
    {
        let _ = (max_keys, decay_sec);
        "[KEYS IDLE]".to_string()
    }
}
