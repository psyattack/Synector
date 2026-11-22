#[cfg(windows)]
use super::macro_action::{calculate_sleep_ms, random_range};

#[cfg(windows)]
use windows::Win32::Foundation::POINT;
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, MOUSEINPUT, MOUSE_EVENT_FLAGS,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP,
    MOUSEEVENTF_WHEEL, MOUSEEVENTF_MOVE,
    VIRTUAL_KEY, VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RETURN, VK_ESCAPE, VK_TAB, VK_SPACE,
    VK_F1, VK_F2, VK_F3, VK_F4, VK_F5, VK_F6, VK_F7, VK_F8, VK_F9, VK_F10, VK_F11, VK_F12,
    VK_BACK, VK_DELETE, VK_INSERT, VK_HOME, VK_END, VK_PRIOR, VK_NEXT, VK_UP, VK_DOWN, VK_LEFT,
    VK_RIGHT, VK_CAPITAL, VK_SNAPSHOT,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, SetCursorPos};

/// Physical input emitters (SendInput, SetCursorPos, media keys) are disabled while the
/// crate is built for unit tests, so `cargo test` can exercise macros, hold tracking and
/// cursor math without moving the developer's mouse or typing into the focused window.
pub(crate) const INPUT_INJECTION_ENABLED: bool = !cfg!(test);

#[cfg(windows)]
pub(super) fn send_mouse_input(flags: MOUSE_EVENT_FLAGS, mouse_data: u32) {
    if !INPUT_INJECTION_ENABLED {
        return;
    }
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: mouse_data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        let _ = SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

#[cfg(windows)]
pub(super) async fn smooth_mouse_move(
    target_x: i32,
    target_y: i32,
    duration_ms: u32,
    organic: bool,
) {
    if !INPUT_INJECTION_ENABLED {
        return;
    }
    let mut start_pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut start_pt);
    }
    let sx = start_pt.x as f64;
    let sy = start_pt.y as f64;
    let tx = target_x as f64;
    let ty = target_y as f64;

    let total_dist = (tx - sx).hypot(ty - sy).max(1.0);
    if total_dist < 2.0 {
        unsafe {
            let _ = SetCursorPos(target_x, target_y);
        }
        return;
    }

    let base_dur = duration_ms.max(20) as f64;
    let total_duration = if organic {
        let factor = 1.0 + (random_range(0, 30) as f64 - 15.0) / 100.0;
        base_dur * factor
    } else {
        base_dur
    };

    let step_interval_ms = 8.0;
    let steps = (total_duration / step_interval_ms).round().max(4.0) as usize;

    let (p1x, p1y, p2x, p2y) = if organic {
        let vx = tx - sx;
        let vy = ty - sy;
        let nx = -vy / total_dist;
        let ny = vx / total_dist;
        let sign = if random_range(0, 1) == 0 { 1.0 } else { -1.0 };
        let dev = total_dist * (random_range(10, 25) as f64 / 100.0) * sign;

        let p1x = sx + vx * 0.3 + nx * dev;
        let p1y = sy + vy * 0.3 + ny * dev;
        let p2x = sx + vx * 0.7 + nx * (dev * 0.7);
        let p2y = sy + vy * 0.7 + ny * (dev * 0.7);
        (p1x, p1y, p2x, p2y)
    } else {
        (sx, sy, tx, ty)
    };

    for i in 1..=steps {
        let raw_t = i as f64 / steps as f64;
        let t = if raw_t < 0.5 {
            2.0 * raw_t * raw_t
        } else {
            1.0 - (-2.0 * raw_t + 2.0).powi(2) / 2.0
        };

        let (cur_x, cur_y) = if organic {
            let u = 1.0 - t;
            let tt = t * t;
            let uu = u * u;
            let uuu = uu * u;
            let ttt = tt * t;

            let bx = uuu * sx + 3.0 * uu * t * p1x + 3.0 * u * tt * p2x + ttt * tx;
            let by = uuu * sy + 3.0 * uu * t * p1y + 3.0 * u * tt * p2y + ttt * ty;

            if i < steps.saturating_sub(2) {
                let jx = (random_range(0, 2) as i32 - 1) as f64;
                let jy = (random_range(0, 2) as i32 - 1) as f64;
                (bx + jx, by + jy)
            } else {
                (bx, by)
            }
        } else {
            let lx = sx + (tx - sx) * t;
            let ly = sy + (ty - sy) * t;
            (lx, ly)
        };

        unsafe {
            let _ = SetCursorPos(cur_x.round() as i32, cur_y.round() as i32);
        }
        tokio::time::sleep(std::time::Duration::from_millis(step_interval_ms as u64)).await;
    }

    unsafe {
        let _ = SetCursorPos(target_x, target_y);
    }
}

#[cfg(windows)]
pub(super) fn send_mouse_relative_step(dx: i32, dy: i32) {
    if dx == 0 && dy == 0 || !INPUT_INJECTION_ENABLED {
        return;
    }
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: 0,
                dwFlags: MOUSEEVENTF_MOVE,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        let _ = SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

#[cfg(windows)]
pub(super) async fn smooth_relative_mouse_move(
    total_dx: i32,
    total_dy: i32,
    duration_ms: u32,
    organic: bool,
) {
    if total_dx == 0 && total_dy == 0 || !INPUT_INJECTION_ENABLED {
        return;
    }

    let dur_ms = duration_ms.max(10);
    // For very brief or tiny movements, send immediately without slicing
    if dur_ms < 18 || (total_dx.abs() < 3 && total_dy.abs() < 3) {
        send_mouse_relative_step(total_dx, total_dy);
        return;
    }

    let base_dur = dur_ms as f64;
    let total_duration_ms = if organic {
        let factor = 1.0 + (random_range(0, 20) as f64 - 10.0) / 100.0;
        base_dur * factor
    } else {
        base_dur
    };
    let total_secs = total_duration_ms / 1000.0;

    let start = std::time::Instant::now();
    let mut sent_x = 0i32;
    let mut sent_y = 0i32;

    loop {
        let elapsed = start.elapsed().as_secs_f64();
        let t = (elapsed / total_secs).min(1.0);

        let progress = if organic {
            // Smoothstep curve for human-like organic motion
            t * t * (3.0 - 2.0 * t)
        } else {
            // Linear velocity curve - exactly matches in-game mouse sensor dynamics
            t
        };

        let target_x = (total_dx as f64 * progress).round() as i32;
        let target_y = (total_dy as f64 * progress).round() as i32;

        let step_dx = target_x - sent_x;
        let step_dy = target_y - sent_y;

        if step_dx != 0 || step_dy != 0 {
            send_mouse_relative_step(step_dx, step_dy);
            sent_x += step_dx;
            sent_y += step_dy;
        }

        if t >= 1.0 {
            break;
        }

        let remaining_ms = (total_secs - elapsed) * 1000.0;
        if remaining_ms > 5.0 {
            tokio::time::sleep(std::time::Duration::from_millis(3)).await;
        } else {
            std::thread::yield_now();
        }
    }

    // Deliver any remaining integer ticks to guarantee 0 cumulative drift
    let rem_x = total_dx - sent_x;
    let rem_y = total_dy - sent_y;
    if rem_x != 0 || rem_y != 0 {
        send_mouse_relative_step(rem_x, rem_y);
    }
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
pub(super) async fn mouse_control(
    action_type: crate::models::MouseActionType,
    button: crate::models::MouseButton,
    coord_mode: crate::models::MouseCoordMode,
    x: Option<i32>,
    y: Option<i32>,
    smooth_move: bool,
    move_duration_ms: Option<u32>,
    organic_move: bool,
    scroll_delta: Option<i32>,
    clicks: Option<u32>,
    click_delay_ms: Option<u32>,
    organic_click_delay: bool,
) -> Result<(), String> {
    match coord_mode {
        crate::models::MouseCoordMode::Absolute => {
            let target_x = x.unwrap_or(0);
            let target_y = y.unwrap_or(0);
            if smooth_move {
                smooth_mouse_move(target_x, target_y, move_duration_ms.unwrap_or(250), organic_move).await;
            } else {
                if INPUT_INJECTION_ENABLED {
                    unsafe {
                        let _ = SetCursorPos(target_x, target_y);
                    }
                }
                let sleep_d = move_duration_ms.unwrap_or(16);
                if sleep_d > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(sleep_d as u64)).await;
                }
            }
        }
        crate::models::MouseCoordMode::Relative => {
            let dx = x.unwrap_or(0);
            let dy = y.unwrap_or(0);
            if smooth_move {
                smooth_relative_mouse_move(dx, dy, move_duration_ms.unwrap_or(200), organic_move).await;
            } else {
                send_mouse_relative_step(dx, dy);
                let sleep_d = move_duration_ms.unwrap_or(16);
                if sleep_d > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(sleep_d as u64)).await;
                }
            }
        }
        crate::models::MouseCoordMode::Current => {}
    }

    let (down_flag, up_flag, mouse_data) = match button {
        crate::models::MouseButton::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, 0),
        crate::models::MouseButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, 0),
        crate::models::MouseButton::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, 0),
        crate::models::MouseButton::XButton1 => (MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, 1),
        crate::models::MouseButton::XButton2 => (MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, 2),
    };

    let base_delay = click_delay_ms.unwrap_or(35);

    match action_type {
        crate::models::MouseActionType::Move => {
            // Movement handled above
        }
        crate::models::MouseActionType::Click => {
            let count = clicks.unwrap_or(1).clamp(1, 100);
            for i in 0..count {
                if let Ok(mut lock) = ACTIVE_HELD_MOUSE_BUTTONS.lock() {
                    lock.insert(button);
                }
                send_mouse_input(down_flag, mouse_data);
                let hold_ms = calculate_sleep_ms(base_delay.clamp(10, 500), organic_click_delay, 30, None);
                tokio::time::sleep(std::time::Duration::from_millis(hold_ms)).await;
                send_mouse_input(up_flag, mouse_data);
                if let Ok(mut lock) = ACTIVE_HELD_MOUSE_BUTTONS.lock() {
                    lock.remove(&button);
                }

                if i + 1 < count {
                    let interval_ms = calculate_sleep_ms((base_delay * 2).clamp(20, 2000), organic_click_delay, 35, None);
                    tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;
                }
            }
        }
        crate::models::MouseActionType::DoubleClick => {
            if let Ok(mut lock) = ACTIVE_HELD_MOUSE_BUTTONS.lock() {
                lock.insert(button);
            }
            send_mouse_input(down_flag, mouse_data);
            let hold1 = calculate_sleep_ms(base_delay.clamp(10, 300), organic_click_delay, 30, None);
            tokio::time::sleep(std::time::Duration::from_millis(hold1)).await;
            send_mouse_input(up_flag, mouse_data);
            if let Ok(mut lock) = ACTIVE_HELD_MOUSE_BUTTONS.lock() {
                lock.remove(&button);
            }

            let pause = calculate_sleep_ms((base_delay * 2).clamp(30, 400), organic_click_delay, 30, None);
            tokio::time::sleep(std::time::Duration::from_millis(pause)).await;

            if let Ok(mut lock) = ACTIVE_HELD_MOUSE_BUTTONS.lock() {
                lock.insert(button);
            }
            send_mouse_input(down_flag, mouse_data);
            let hold2 = calculate_sleep_ms(base_delay.clamp(10, 300), organic_click_delay, 30, None);
            tokio::time::sleep(std::time::Duration::from_millis(hold2)).await;
            send_mouse_input(up_flag, mouse_data);
            if let Ok(mut lock) = ACTIVE_HELD_MOUSE_BUTTONS.lock() {
                lock.remove(&button);
            }
        }
        crate::models::MouseActionType::Down => {
            if let Ok(mut lock) = ACTIVE_HELD_MOUSE_BUTTONS.lock() {
                lock.insert(button);
            }
            send_mouse_input(down_flag, mouse_data);
        }
        crate::models::MouseActionType::Up => {
            if let Ok(mut lock) = ACTIVE_HELD_MOUSE_BUTTONS.lock() {
                lock.remove(&button);
            }
            send_mouse_input(up_flag, mouse_data);
        }
        crate::models::MouseActionType::Scroll => {
            let delta = scroll_delta.unwrap_or(120);
            send_mouse_input(MOUSEEVENTF_WHEEL, delta as u32);
        }
    }
    Ok(())
}

#[cfg(windows)]
pub(super) async fn simulate_text_typing(text: &str, delay_ms: u32, organic: bool, jitter_percent: u32) -> Result<(), String> {
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    let mut remaining = text;
    while !remaining.is_empty() {
        if remaining.starts_with('{') {
            if let Some(end_brace) = remaining.find('}') {
                let token = remaining[1..end_brace].trim();
                let lower = token.to_lowercase();
                if let Some(stripped) = lower.strip_prefix("delay ") {
                    if let Ok(ms) = stripped.trim().parse::<u64>() {
                        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                    }
                    remaining = &remaining[end_brace + 1..];
                    continue;
                }
            }
        }

        let mut chars = remaining.chars();
        let ch = chars.next().unwrap();
        remaining = chars.as_str();

        if ch == '\r' {
            continue;
        }
        if ch == '\n' {
            send_key_event(VK_RETURN, false);
            tokio::time::sleep(std::time::Duration::from_millis(35)).await;
            send_key_event(VK_RETURN, true);
        } else {
            send_char_event(ch);
        }
        let sleep = calculate_sleep_ms(delay_ms, organic, jitter_percent, Some(ch));
        if sleep > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(sleep)).await;
        }
    }
    Ok(())
}

#[cfg(windows)]
pub(super) fn parse_virtual_key(s: &str) -> Option<VIRTUAL_KEY> {
    let clean = s.trim().trim_matches(|c| c == '{' || c == '}');
    let upper = clean.to_uppercase();
    match upper.as_str() {
        "CTRL" | "CONTROL" | "LCTRL" | "RCTRL" => Some(VK_CONTROL),
        "SHIFT" | "LSHIFT" | "RSHIFT" => Some(VK_SHIFT),
        "ALT" | "MENU" | "LALT" | "RALT" => Some(VK_MENU),
        "WIN" | "LWIN" | "RWIN" | "WINDOWS" => Some(VK_LWIN),
        "ENTER" | "RETURN" => Some(VK_RETURN),
        "ESC" | "ESCAPE" => Some(VK_ESCAPE),
        "TAB" => Some(VK_TAB),
        "SPACE" | "SPACEBAR" => Some(VK_SPACE),
        "BACK" | "BACKSPACE" => Some(VK_BACK),
        "DEL" | "DELETE" => Some(VK_DELETE),
        "INS" | "INSERT" => Some(VK_INSERT),
        "HOME" => Some(VK_HOME),
        "END" => Some(VK_END),
        "PAGEUP" | "PGUP" | "PRIOR" => Some(VK_PRIOR),
        "PAGEDOWN" | "PGDN" | "NEXT" => Some(VK_NEXT),
        "UP" => Some(VK_UP),
        "DOWN" => Some(VK_DOWN),
        "LEFT" => Some(VK_LEFT),
        "RIGHT" => Some(VK_RIGHT),
        "CAPS" | "CAPSLOCK" => Some(VK_CAPITAL),
        "PRTSC" | "PRINTSCREEN" => Some(VK_SNAPSHOT),
        "F1" => Some(VK_F1),
        "F2" => Some(VK_F2),
        "F3" => Some(VK_F3),
        "F4" => Some(VK_F4),
        "F5" => Some(VK_F5),
        "F6" => Some(VK_F6),
        "F7" => Some(VK_F7),
        "F8" => Some(VK_F8),
        "F9" => Some(VK_F9),
        "F10" => Some(VK_F10),
        "F11" => Some(VK_F11),
        "F12" => Some(VK_F12),
        // Numpad
        "NUMPAD0" | "NUM0" => Some(VIRTUAL_KEY(0x60)),
        "NUMPAD1" | "NUM1" => Some(VIRTUAL_KEY(0x61)),
        "NUMPAD2" | "NUM2" => Some(VIRTUAL_KEY(0x62)),
        "NUMPAD3" | "NUM3" => Some(VIRTUAL_KEY(0x63)),
        "NUMPAD4" | "NUM4" => Some(VIRTUAL_KEY(0x64)),
        "NUMPAD5" | "NUM5" => Some(VIRTUAL_KEY(0x65)),
        "NUMPAD6" | "NUM6" => Some(VIRTUAL_KEY(0x66)),
        "NUMPAD7" | "NUM7" => Some(VIRTUAL_KEY(0x67)),
        "NUMPAD8" | "NUM8" => Some(VIRTUAL_KEY(0x68)),
        "NUMPAD9" | "NUM9" => Some(VIRTUAL_KEY(0x69)),
        "MULTIPLY" | "NUMPADMULTIPLY" => Some(VIRTUAL_KEY(0x6A)),
        "ADD" | "NUMPADADD" => Some(VIRTUAL_KEY(0x6B)),
        "SUBTRACT" | "NUMPADSUBTRACT" => Some(VIRTUAL_KEY(0x6D)),
        "DECIMAL" | "NUMPADDECIMAL" => Some(VIRTUAL_KEY(0x6E)),
        "DIVIDE" | "NUMPADDIVIDE" => Some(VIRTUAL_KEY(0x6F)),
        // Punctuation and symbols
        ";" | "SEMICOLON" => Some(VIRTUAL_KEY(0xBA)),
        "=" | "PLUS" | "EQUAL" | "EQUALS" => Some(VIRTUAL_KEY(0xBB)),
        "," | "COMMA" => Some(VIRTUAL_KEY(0xBC)),
        "-" | "MINUS" | "DASH" => Some(VIRTUAL_KEY(0xBD)),
        "." | "PERIOD" | "DOT" => Some(VIRTUAL_KEY(0xBE)),
        "/" | "SLASH" => Some(VIRTUAL_KEY(0xBF)),
        "`" | "~" | "TILDE" | "BACKTICK" => Some(VIRTUAL_KEY(0xC0)),
        "[" | "{" | "LBRACKET" => Some(VIRTUAL_KEY(0xDB)),
        "\\" | "|" | "BACKSLASH" => Some(VIRTUAL_KEY(0xDC)),
        "]" | "}" | "RBRACKET" => Some(VIRTUAL_KEY(0xDD)),
        "'" | "\"" | "QUOTE" => Some(VIRTUAL_KEY(0xDE)),
        s if s.to_uppercase().starts_with("VK_0X") => {
            let hex = &s[5..];
            u16::from_str_radix(hex, 16).ok().map(VIRTUAL_KEY)
        }
        s if s.len() == 1 => {
            let ch = s.chars().next().unwrap();
            if ch.is_ascii_alphanumeric() {
                Some(VIRTUAL_KEY(ch.to_ascii_uppercase() as u16))
            } else {
                match ch {
                    ';' => Some(VIRTUAL_KEY(0xBA)),
                    '=' => Some(VIRTUAL_KEY(0xBB)),
                    ',' => Some(VIRTUAL_KEY(0xBC)),
                    '-' => Some(VIRTUAL_KEY(0xBD)),
                    '.' => Some(VIRTUAL_KEY(0xBE)),
                    '/' => Some(VIRTUAL_KEY(0xBF)),
                    '`' => Some(VIRTUAL_KEY(0xC0)),
                    '[' => Some(VIRTUAL_KEY(0xDB)),
                    '\\' => Some(VIRTUAL_KEY(0xDC)),
                    ']' => Some(VIRTUAL_KEY(0xDD)),
                    '\'' => Some(VIRTUAL_KEY(0xDE)),
                    _ => None,
                }
            }
        }
        _ => None,
    }
}

#[cfg(windows)]
pub(super) static ACTIVE_HELD_KEYS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<u16>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashSet::new()));

#[cfg(windows)]
pub(super) static ACTIVE_HELD_MOUSE_BUTTONS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<crate::models::MouseButton>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashSet::new()));

#[cfg(windows)]
pub(super) fn send_key_event_raw(vk: VIRTUAL_KEY, is_up: bool) {
    if !INPUT_INJECTION_ENABLED {
        return;
    }
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        MapVirtualKeyW, MAPVK_VK_TO_VSC, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE,
        VK_DIVIDE, VK_LWIN, VK_RCONTROL, VK_RMENU, VK_RWIN,
    };
    let scan = unsafe { MapVirtualKeyW(vk.0 as u32, MAPVK_VK_TO_VSC) } as u16;
    let mut flags = windows::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS(0);
    if is_up {
        flags |= KEYEVENTF_KEYUP;
    }
    if scan != 0 {
        flags |= KEYEVENTF_SCANCODE;
    }
    match vk {
        VK_UP | VK_DOWN | VK_LEFT | VK_RIGHT | VK_INSERT | VK_DELETE | VK_HOME | VK_END
        | VK_PRIOR | VK_NEXT | VK_SNAPSHOT | VK_DIVIDE | VK_RCONTROL | VK_RMENU | VK_LWIN | VK_RWIN => {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        _ => {}
    }
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        let _ = SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

#[cfg(windows)]
pub(super) fn send_key_event(vk: VIRTUAL_KEY, is_up: bool) {
    if let Ok(mut lock) = ACTIVE_HELD_KEYS.lock() {
        if is_up {
            lock.remove(&vk.0);
        } else {
            lock.insert(vk.0);
        }
    }
    send_key_event_raw(vk, is_up);
}

/// Immediately and physically releases all keyboard keys and mouse buttons that were simulated
/// in the down state by macros, ensuring game characters and inputs stop immediately upon cancel.
pub fn release_all_simulated_inputs() {
    #[cfg(windows)]
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            VIRTUAL_KEY, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_XUP,
        };

        let keys_to_release: Vec<u16> = {
            if let Ok(mut lock) = ACTIVE_HELD_KEYS.lock() {
                lock.drain().collect()
            } else {
                Vec::new()
            }
        };

        for vk_code in keys_to_release {
            send_key_event_raw(VIRTUAL_KEY(vk_code), true);
        }

        let buttons_to_release: Vec<crate::models::MouseButton> = {
            if let Ok(mut lock) = ACTIVE_HELD_MOUSE_BUTTONS.lock() {
                lock.drain().collect()
            } else {
                Vec::new()
            }
        };

        for btn in buttons_to_release {
            let (up_flag, mouse_data) = match btn {
                crate::models::MouseButton::Left => (MOUSEEVENTF_LEFTUP, 0),
                crate::models::MouseButton::Right => (MOUSEEVENTF_RIGHTUP, 0),
                crate::models::MouseButton::Middle => (MOUSEEVENTF_MIDDLEUP, 0),
                crate::models::MouseButton::XButton1 => (MOUSEEVENTF_XUP, 1),
                crate::models::MouseButton::XButton2 => (MOUSEEVENTF_XUP, 2),
            };
            send_mouse_input(up_flag, mouse_data);
        }
    }
}

#[cfg(windows)]
pub(super) fn send_char_event(ch: char) {
    if !INPUT_INJECTION_ENABLED {
        return;
    }
    let mut buf = [0u16; 2];
    let encoded = ch.encode_utf16(&mut buf);
    for &code in encoded.iter() {
        let input_down = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: code,
                    dwFlags: KEYEVENTF_UNICODE,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let input_up = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: code,
                    dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        unsafe {
            SendInput(&[input_down, input_up], std::mem::size_of::<INPUT>() as i32);
        }
    }
}
