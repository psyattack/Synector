use std::collections::HashMap;

use crate::models::Action;
use crate::models::WindowActionType;

#[cfg(windows)]
use windows::core::BOOL;
#[cfg(windows)]
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, WPARAM};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    PostMessageW, EnumWindows, GetForegroundWindow, GetWindowThreadProcessId, ShowWindow,
    SetWindowPos, GetWindowLongW, SetWindowLongW, SetLayeredWindowAttributes,
    GetWindow, GW_OWNER, IsWindowVisible, GetWindowTextLengthW, GetWindowTextW,
    HWND_TOPMOST, HWND_NOTOPMOST, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    SW_HIDE, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, SW_SHOW,
    GWL_EXSTYLE, WS_EX_LAYERED, WS_EX_TOPMOST, WS_EX_TOOLWINDOW, LWA_ALPHA,
    WM_CLOSE,
};

pub(super) fn execute(action: &Action, context: &HashMap<String, String>) -> Result<(), String> {
    match action {
        Action::Window {
            target_process,
            window_action,
            x,
            y,
            width,
            height,
            transparency,
        } => {
            let default_proc = String::new();
            let proc_name = target_process.as_ref().unwrap_or(
                context.get("ProcessName").unwrap_or(&default_proc)
            );
            let target_pid = context.get("ProcessId").and_then(|p| p.parse::<u32>().ok());
            #[cfg(windows)]
            {
                window_control(
                    proc_name,
                    target_pid,
                    *window_action,
                    *x,
                    *y,
                    *width,
                    *height,
                    *transparency,
                )?;
            }
            Ok(())
        }
        _ => unreachable!("window handler received a non-window action"),
    }
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn window_control(
    target_process: &str,
    target_pid: Option<u32>,
    action: WindowActionType,
    x: Option<i32>,
    y: Option<i32>,
    width: Option<i32>,
    height: Option<i32>,
    transparency: Option<u8>,
) -> Result<(), String> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowRect, ShowWindowAsync, IsIconic, SWP_NOACTIVATE, GWL_STYLE, WS_CHILD,
    };

    let target_proc_clean = target_process.trim();
    let mut resolved_pids = Vec::new();
    if let Some(pid) = target_pid {
        if pid > 0 {
            resolved_pids.push(pid);
        }
    }
    if resolved_pids.is_empty() && !target_proc_clean.is_empty() {
        let snap = crate::daemon::watcher::ProcessWatcher::snapshot_process_map();
        let query_name = target_proc_clean.trim_end_matches(".exe").to_lowercase();
        for (pid, name) in snap {
            let proc_base = name.trim_end_matches(".exe").to_lowercase();
            if proc_base == query_name {
                resolved_pids.push(pid);
            }
        }
    }

    struct CandidateWindow {
        hwnd: HWND,
        area: i64,
        is_visible: bool,
    }

    unsafe extern "system" fn enum_wnd(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let data = &mut *(lparam.0 as *mut (Vec<u32>, String, Vec<CandidateWindow>));
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));

        // Skip windows belonging to our own process
        if pid == std::process::id() {
            return BOOL(1);
        }

        // Must NOT be a child window (must be top-level)
        let style = GetWindowLongW(hwnd, GWL_STYLE);
        if (style & (WS_CHILD.0 as i32)) != 0 {
            return BOOL(1);
        }

        // Must NOT be an owned popup/dialog
        if let Ok(owner) = GetWindow(hwnd, GW_OWNER) {
            if !owner.0.is_null() {
                return BOOL(1);
            }
        }

        // Must NOT be a tool window
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE);
        if (ex & (WS_EX_TOOLWINDOW.0 as i32)) != 0 {
            return BOOL(1);
        }

        // Must have non-trivial dimensions
        let mut r = RECT::default();
        if GetWindowRect(hwnd, &mut r).is_err() {
            return BOOL(1);
        }
        let w = (r.right - r.left).max(0);
        let h = (r.bottom - r.top).max(0);
        if w < 20 || h < 20 {
            return BOOL(1);
        }

        let is_vis = IsWindowVisible(hwnd).as_bool();
        let title_len = GetWindowTextLengthW(hwnd);

        // Filter out common Windows internal hidden framework/IME helper windows
        if title_len > 0 {
            let mut buf = [0u16; 128];
            let len = GetWindowTextW(hwnd, &mut buf);
            if len > 0 {
                let title = String::from_utf16_lossy(&buf[..len as usize]);
                if title.starts_with("MSCTFIME")
                    || title.starts_with("Default IME")
                    || title.starts_with("OLEChannelWnd")
                    || title.starts_with("GDI+ Window")
                {
                    return BOOL(1);
                }
            }
        }

        let mut matches = false;
        if !data.0.is_empty() {
            if data.0.contains(&pid) {
                matches = true;
            }
        } else if !data.1.is_empty() {
            if let Some(img) = crate::daemon::watcher::ProcessWatcher::get_process_image_path(pid) {
                let name = std::path::Path::new(&img)
                    .file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or("")
                    .trim_end_matches(".exe");
                if name.eq_ignore_ascii_case(data.1.trim_end_matches(".exe")) {
                    matches = true;
                }
            }
        }

        if matches {
            data.2.push(CandidateWindow {
                hwnd,
                area: (w as i64) * (h as i64),
                is_visible: is_vis,
            });
        }
        BOOL(1)
    }

    let mut query = (resolved_pids, target_proc_clean.to_string(), Vec::<CandidateWindow>::new());
    unsafe {
        let _ = EnumWindows(Some(enum_wnd), LPARAM(&mut query as *mut _ as isize));
    }

    let mut target_hwnds: Vec<HWND> = Vec::new();
    if !query.2.is_empty() {
        // Sort by visible first, then by area descending to target the primary main window
        query.2.sort_by(|a, b| {
            b.is_visible.cmp(&a.is_visible).then_with(|| b.area.cmp(&a.area))
        });
        // Select the primary main application window
        if let Some(main_cand) = query.2.first() {
            target_hwnds.push(main_cand.hwnd);
        }
    } else if target_proc_clean.is_empty() && target_pid.is_none() {
        unsafe {
            let fg = windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow();
            if !fg.0.is_null() {
                let mut fg_pid = 0u32;
                GetWindowThreadProcessId(fg, Some(&mut fg_pid));
                if fg_pid != std::process::id() {
                    target_hwnds.push(fg);
                }
            }
        }
    }

    if target_hwnds.is_empty() {
        return Err(format!("No eligible window found for target '{}'", target_process));
    }

    for hwnd in target_hwnds {
        unsafe {
            match action {
                WindowActionType::Minimize => {
                    let _ = ShowWindowAsync(hwnd, SW_MINIMIZE);
                }
                WindowActionType::Maximize => {
                    let _ = ShowWindowAsync(hwnd, SW_MAXIMIZE);
                }
                WindowActionType::Restore => {
                    let _ = ShowWindowAsync(hwnd, SW_RESTORE);
                }
                WindowActionType::Hide => {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
                WindowActionType::Show => {
                    let _ = ShowWindowAsync(hwnd, SW_SHOW);
                }
                WindowActionType::Close => {
                    let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
                WindowActionType::ToggleMinimizeRestore => {
                    if IsIconic(hwnd).as_bool() {
                        let _ = ShowWindowAsync(hwnd, SW_RESTORE);
                    } else {
                        let _ = ShowWindowAsync(hwnd, SW_MINIMIZE);
                    }
                }
                WindowActionType::ToggleShowHide => {
                    if IsWindowVisible(hwnd).as_bool() {
                        let _ = ShowWindow(hwnd, SW_HIDE);
                    } else {
                        let _ = ShowWindowAsync(hwnd, SW_SHOW);
                    }
                }
                WindowActionType::ToggleAlwaysOnTop => {
                    let ex = GetWindowLongW(hwnd, GWL_EXSTYLE);
                    let is_topmost = (ex & (WS_EX_TOPMOST.0 as i32)) != 0;
                    let flag = if is_topmost { HWND_NOTOPMOST } else { HWND_TOPMOST };
                    let _ = SetWindowPos(hwnd, Some(flag), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                }
                WindowActionType::AlwaysOnTop => {
                    let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                }
                WindowActionType::RemoveAlwaysOnTop => {
                    let _ = SetWindowPos(hwnd, Some(HWND_NOTOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                }
                WindowActionType::SetTransparency => {
                    let ex = GetWindowLongW(hwnd, GWL_EXSTYLE);
                    SetWindowLongW(hwnd, GWL_EXSTYLE, ex | (WS_EX_LAYERED.0 as i32));
                    let alpha = transparency.unwrap_or(255);
                    let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA);
                }
                WindowActionType::SnapToPosition => {
                    let mut current_rect = RECT::default();
                    let _ = GetWindowRect(hwnd, &mut current_rect);
                    let cur_w = (current_rect.right - current_rect.left).max(100);
                    let cur_h = (current_rect.bottom - current_rect.top).max(100);
                    let nx = x.unwrap_or(current_rect.left);
                    let ny = y.unwrap_or(current_rect.top);
                    let nw = width.unwrap_or(cur_w);
                    let nh = height.unwrap_or(cur_h);
                    let _ = SetWindowPos(hwnd, None, nx, ny, nw, nh, SWP_NOZORDER | SWP_NOACTIVATE);
                }
            }
        }
    }

    Ok(())
}

#[cfg(windows)]
pub fn get_foreground_window_pid() -> Option<u32> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let mut fg_pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut fg_pid));
        if fg_pid != 0 {
            Some(fg_pid)
        } else {
            None
        }
    }
}

pub fn has_matching_window(process_name: &str, match_mode: crate::models::ProcessMatchMode) -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{
            EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
            GetWindowLongW, GetWindow, GetWindowRect,
            GWL_STYLE, GWL_EXSTYLE, GW_OWNER, WS_CHILD, WS_EX_TOOLWINDOW,
        };
        use windows::Win32::Foundation::{HWND, LPARAM, RECT};

        let target = process_name.trim().to_lowercase();
        if target.is_empty() {
            return false;
        }

        struct SearchCtx {
            target: String,
            match_mode: crate::models::ProcessMatchMode,
            found: bool,
        }

        unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let ctx = &mut *(lparam.0 as *mut SearchCtx);

            let style = GetWindowLongW(hwnd, GWL_STYLE);
            if (style & (WS_CHILD.0 as i32)) != 0 {
                return BOOL(1);
            }
            if let Ok(owner) = GetWindow(hwnd, GW_OWNER) {
                if !owner.0.is_null() {
                    return BOOL(1);
                }
            }
            let ex = GetWindowLongW(hwnd, GWL_EXSTYLE);
            if (ex & (WS_EX_TOOLWINDOW.0 as i32)) != 0 {
                return BOOL(1);
            }
            let mut r = RECT::default();
            if GetWindowRect(hwnd, &mut r).is_err() {
                return BOOL(1);
            }
            let w = (r.right - r.left).max(0);
            let h = (r.bottom - r.top).max(0);
            if w < 20 || h < 20 {
                return BOOL(1);
            }

            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == 0 || pid == std::process::id() {
                return BOOL(1);
            }

            if let Some(img) = crate::daemon::watcher::ProcessWatcher::get_process_image_path(pid) {
                let name = std::path::Path::new(&img)
                    .file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                if crate::daemon::conditions::matches_process(&ctx.target, &name, ctx.match_mode) {
                    ctx.found = true;
                    return BOOL(0);
                }
            }

            let title_len = GetWindowTextLengthW(hwnd);
            if title_len > 0 {
                let mut buf = [0u16; 256];
                let len = GetWindowTextW(hwnd, &mut buf);
                if len > 0 {
                    let title = String::from_utf16_lossy(&buf[..len as usize]);
                    if crate::daemon::conditions::matches_process(&ctx.target, &title, ctx.match_mode) {
                        ctx.found = true;
                        return BOOL(0);
                    }
                }
            }

            BOOL(1)
        }

        let mut ctx = SearchCtx {
            target,
            match_mode,
            found: false,
        };
        unsafe {
            let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut _ as isize));
        }
        ctx.found
    }
}
