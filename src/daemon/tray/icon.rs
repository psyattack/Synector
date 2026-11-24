use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Mutex as TokioMutex;
use crate::daemon::engine::DaemonEngine;

use super::{
    abort_active_update, apply_menu_theme, handle_tray_update_click, is_updating, show_tray_menu,
    IDM_EXIT, IDM_OPEN, IDM_UPDATE,
};

#[cfg(windows)]
use windows::core::{w, PCWSTR};
#[cfg(windows)]
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
#[cfg(windows)]
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
#[cfg(windows)]
use windows::Win32::UI::Shell::{
    ExtractIconExW, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyIcon, DispatchMessageW, GetMessageW, GetWindowLongPtrW,
    LoadIconW, LoadImageW, PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW,
    SetWindowLongPtrW, TranslateMessage, GWLP_USERDATA, HICON, IDI_APPLICATION, IMAGE_ICON, LR_LOADFROMFILE,
    WM_APP, WM_COMMAND, WM_DESTROY, WM_LBUTTONDBLCLK, WM_RBUTTONUP, WNDCLASSW,
};

pub static RUNNING: AtomicBool = AtomicBool::new(true);
static WM_TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static GLOBAL_ICON_PATH: Mutex<Option<String>> = Mutex::new(None);
static TRAY_HWND: AtomicIsize = AtomicIsize::new(0);
static HIDE_TRAY_ICON: AtomicBool = AtomicBool::new(false);

pub(super) static GLOBAL_ENGINE: Mutex<Option<Arc<TokioMutex<DaemonEngine>>>> = Mutex::new(None);
pub(super) static GLOBAL_RT: Mutex<Option<tokio::runtime::Handle>> = Mutex::new(None);

pub(super) const WM_TRAYICON: u32 = WM_APP + 1;
const WM_UPDATE_ICON: u32 = WM_APP + 2;
const WM_TRAY_VISIBILITY: u32 = WM_APP + 3;

/// An icon handle together with its ownership: `LoadIconW` returns shared handles that must not
/// be destroyed, while `LoadImageW`/`ExtractIconExW` return caller-owned handles that must.
#[cfg(windows)]
pub(super) struct TrayIconHandle {
    handle: HICON,
    owned: bool,
}

#[cfg(windows)]
impl TrayIconHandle {
    pub(super) fn shared(handle: HICON) -> Self {
        Self {
            handle,
            owned: false,
        }
    }

    pub(super) fn owned(handle: HICON) -> Self {
        Self {
            handle,
            owned: true,
        }
    }

    #[cfg(test)]
    pub(super) fn is_owned(&self) -> bool {
        self.owned
    }
}

#[cfg(windows)]
impl Drop for TrayIconHandle {
    fn drop(&mut self) {
        if self.owned && !self.handle.is_invalid() {
            unsafe {
                let _ = DestroyIcon(self.handle);
            }
        }
    }
}

#[cfg(windows)]
fn load_tray_icon(custom_path: Option<&str>) -> TrayIconHandle {
    unsafe {
        // 1. If custom path provided in config
        if let Some(path) = custom_path {
            let trimmed = path.trim();
            if !trimmed.is_empty() {
                let expanded = expand_path(trimmed);
                let p = std::path::Path::new(&expanded);
                if p.exists() {
                    let path_w: Vec<u16> = expanded.encode_utf16().chain(std::iter::once(0)).collect();
                    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();

                    if ext == "ico" {
                        if let Ok(handle) = LoadImageW(
                            None,
                            PCWSTR(path_w.as_ptr()),
                            IMAGE_ICON,
                            16,
                            16,
                            LR_LOADFROMFILE,
                        ) {
                            if !handle.is_invalid() {
                                return TrayIconHandle::owned(HICON(handle.0));
                            }
                        }
                    } else if ext == "exe" || ext == "dll" {
                        let mut hicon_small = HICON::default();
                        let count = ExtractIconExW(
                            PCWSTR(path_w.as_ptr()),
                            0,
                            None,
                            Some(&mut hicon_small),
                            1,
                        );
                        if count > 0 && !hicon_small.is_invalid() {
                            return TrayIconHandle::owned(hicon_small);
                        }
                    }
                }
            }
        }

        // 2. Try loading from current executable (which has app.ico embedded at resource ID 1)
        if let Ok(hinst) = GetModuleHandleW(None) {
            if let Ok(icon) = LoadIconW(Some(hinst.into()), PCWSTR(1 as _)) {
                if !icon.is_invalid() {
                    return TrayIconHandle::shared(icon);
                }
            }
        }

        if let Ok(exe) = std::env::current_exe() {
            let exe_w: Vec<u16> = exe.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
            let mut hicon_small = HICON::default();
            let count = ExtractIconExW(
                PCWSTR(exe_w.as_ptr()),
                0,
                None,
                Some(&mut hicon_small),
                1,
            );
            if count > 0 && !hicon_small.is_invalid() {
                return TrayIconHandle::owned(hicon_small);
            }
        }

        // 3. Fallback
        TrayIconHandle::shared(LoadIconW(None, IDI_APPLICATION).unwrap_or_default())
    }
}

fn expand_path(path: &str) -> String {
    let mut result = path.to_string();
    if result.contains('%') {
        for (key, val) in std::env::vars() {
            let placeholder = format!("%{}%", key);
            result = result.replace(&placeholder, &val);
        }
    }
    result
}

/// Tray state owned by the window: the current icon handle and the tooltip identity. The pointer
/// is stored in the window's user data and dropped by `TrayManager`.
#[cfg(windows)]
struct TrayIconState {
    masked: bool,
    icon: TrayIconHandle,
}

#[cfg(windows)]
unsafe fn tray_state(hwnd: HWND) -> Option<&'static mut TrayIconState> {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut TrayIconState;
    ptr.as_mut()
}

/// Builds the single `NOTIFYICONDATAW` shape used by add, modify and delete calls.
#[cfg(windows)]
pub(super) fn build_tray_icon_data(hwnd: HWND, masked: bool, icon: HICON) -> NOTIFYICONDATAW {
    let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = WM_TRAYICON;
    nid.hIcon = icon;
    let tip_str = if masked {
        "Windows Service Host\0"
    } else {
        "Synector\0"
    };
    let tip_text = tip_str.encode_utf16().collect::<Vec<_>>();
    let tip_len = tip_text.len().min(128);
    nid.szTip[..tip_len].copy_from_slice(&tip_text[..tip_len]);
    nid
}

#[cfg(windows)]
unsafe fn reload_tray_icon() -> TrayIconHandle {
    let icon_path = GLOBAL_ICON_PATH.lock().ok().and_then(|l| l.clone());
    load_tray_icon(icon_path.as_deref())
}

/// Adds the tray icon with a freshly loaded handle (startup, TaskbarCreated, re-show).
#[cfg(windows)]
unsafe fn add_tray_icon(hwnd: HWND, state: &mut TrayIconState) {
    let icon = reload_tray_icon();
    let nid = build_tray_icon_data(hwnd, state.masked, icon.handle);
    let _ = Shell_NotifyIconW(NIM_ADD, &nid);
    state.icon = icon;
}

/// Replaces the icon of an existing tray item (settings change). The previous handle is destroyed
/// after the shell accepted the new one.
#[cfg(windows)]
unsafe fn modify_tray_icon(hwnd: HWND, state: &mut TrayIconState) {
    let icon = reload_tray_icon();
    let nid = build_tray_icon_data(hwnd, state.masked, icon.handle);
    if Shell_NotifyIconW(NIM_MODIFY, &nid).as_bool() {
        state.icon = icon;
    }
}

#[cfg(windows)]
unsafe fn delete_tray_icon(hwnd: HWND, state: &TrayIconState) {
    let nid = build_tray_icon_data(hwnd, state.masked, state.icon.handle);
    let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
}

pub struct TrayManager {
    #[cfg(windows)]
    _hwnd: HWND,
    #[cfg(windows)]
    nid: NOTIFYICONDATAW,
}

impl TrayManager {
    pub fn new(
        tray_icon_path: Option<String>,
        hide_tray_icon: bool,
        masked: bool,
        engine: Arc<TokioMutex<DaemonEngine>>,
        rt_handle: tokio::runtime::Handle,
    ) -> Result<Self, String> {
        if let Ok(mut lock) = GLOBAL_ENGINE.lock() {
            *lock = Some(engine);
        }
        if let Ok(mut lock) = GLOBAL_RT.lock() {
            *lock = Some(rt_handle);
        }
        let is_masked = masked || std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().eq_ignore_ascii_case("WindowsServiceHost.exe"))).unwrap_or(false);
        HIDE_TRAY_ICON.store(hide_tray_icon, Ordering::SeqCst);
        if let Ok(mut lock) = GLOBAL_ICON_PATH.lock() {
            *lock = tray_icon_path.clone();
        }

        #[cfg(windows)]
        unsafe {
            let class_name = if is_masked {
                w!("WindowsServiceHostClass")
            } else {
                w!("SynectorTrayClass")
            };
            let wnd_class = WNDCLASSW {
                lpfnWndProc: Some(tray_wnd_proc),
                lpszClassName: class_name,
                ..Default::default()
            };
            RegisterClassW(&wnd_class);

            let window_title = if is_masked {
                w!("Windows Service Host")
            } else {
                w!("SynectorTrayWindow")
            };

            let hwnd = CreateWindowExW(
                windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE(0),
                class_name,
                window_title,
                windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(0),
                0, 0, 0, 0,
                None,
                None,
                None,
                None,
            ).map_err(|e| format!("CreateWindowEx failed: {e}"))?;

            let icon = load_tray_icon(tray_icon_path.as_deref());
            let hicon = icon.handle;
            let state = Box::new(TrayIconState {
                masked: is_masked,
                icon,
            });
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);

            let nid = build_tray_icon_data(hwnd, is_masked, hicon);

            let msg_id = RegisterWindowMessageW(w!("TaskbarCreated"));
            WM_TASKBAR_CREATED.store(msg_id, Ordering::SeqCst);
            TRAY_HWND.store(hwnd.0 as isize, Ordering::SeqCst);

            let config = crate::config::Settings::read();
            let is_dark = config.effective_theme() != crate::config::AppTheme::Light;
            apply_menu_theme(Some(hwnd), is_dark);

            if !hide_tray_icon {
                let _ = Shell_NotifyIconW(NIM_ADD, &nid);
            }

            Ok(Self { _hwnd: hwnd, nid })
        }
    }

    pub fn run_message_loop(&self) {
        #[cfg(windows)]
        unsafe {
            let mut msg = std::mem::zeroed();
            while RUNNING.load(Ordering::SeqCst) {
                let ret = GetMessageW(&mut msg, None, 0, 0);
                if ret.0 <= 0 {
                    break;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

impl Drop for TrayManager {
    fn drop(&mut self) {
        if crate::single_instance::is_gui_running() {
            crate::single_instance::close_existing_gui_window();
        }
        TRAY_HWND.store(0, Ordering::SeqCst);
        #[cfg(windows)]
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.nid);
            // Takes back the state boxed in `new`, dropping the last icon handle with it.
            let state = GetWindowLongPtrW(self._hwnd, GWLP_USERDATA) as *mut TrayIconState;
            if !state.is_null() {
                SetWindowLongPtrW(self._hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(state));
            }
        }
    }
}

pub fn update_tray_icon(new_path: Option<String>) {
    if let Ok(mut lock) = GLOBAL_ICON_PATH.lock() {
        *lock = new_path;
    }
    #[cfg(windows)]
    {
        let raw = TRAY_HWND.load(Ordering::SeqCst);
        if raw != 0 {
            unsafe {
                let hwnd = HWND(raw as *mut _);
                let _ = PostMessageW(Some(hwnd), WM_UPDATE_ICON, WPARAM(0), LPARAM(0));
            }
        }
    }
}

pub fn set_tray_visible(visible: bool) {
    HIDE_TRAY_ICON.store(!visible, Ordering::SeqCst);
    #[cfg(windows)]
    {
        let raw = TRAY_HWND.load(Ordering::SeqCst);
        if raw != 0 {
            unsafe {
                let hwnd = HWND(raw as *mut _);
                let _ = PostMessageW(
                    Some(hwnd),
                    WM_TRAY_VISIBILITY,
                    WPARAM(usize::from(visible)),
                    LPARAM(0),
                );
            }
        }
    }
}

#[cfg(windows)]
unsafe extern "system" fn tray_wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let taskbar_msg = WM_TASKBAR_CREATED.load(Ordering::SeqCst);
    if taskbar_msg != 0 && msg == taskbar_msg {
        if !HIDE_TRAY_ICON.load(Ordering::SeqCst) {
            if let Some(state) = tray_state(hwnd) {
                add_tray_icon(hwnd, state);
            }
        }
        return LRESULT(0);
    }

    match msg {
        WM_UPDATE_ICON => {
            if !HIDE_TRAY_ICON.load(Ordering::SeqCst) {
                if let Some(state) = tray_state(hwnd) {
                    modify_tray_icon(hwnd, state);
                }
            }
            LRESULT(0)
        }

        WM_TRAY_VISIBILITY => {
            if let Some(state) = tray_state(hwnd) {
                if wparam.0 != 0 {
                    add_tray_icon(hwnd, state);
                } else {
                    delete_tray_icon(hwnd, state);
                }
            }
            LRESULT(0)
        }

        WM_TRAYICON => {
            let event = lparam.0 as u32;
            if event == WM_LBUTTONDBLCLK {
                spawn_or_show_gui();
            } else if event == WM_RBUTTONUP {
                show_tray_menu(hwnd);
            }
            LRESULT(0)
        }

        WM_COMMAND => {
            let id = wparam.0;
            match id {
                IDM_OPEN => spawn_or_show_gui(),
                IDM_UPDATE => {
                    if !is_updating() {
                        handle_tray_update_click();
                    }
                }
                IDM_EXIT => {
                    abort_active_update();
                    if crate::single_instance::is_gui_running() {
                        crate::single_instance::close_existing_gui_window();
                    }
                    RUNNING.store(false, Ordering::SeqCst);
                    PostQuitMessage(0);
                }
                _ => {}
            }
            LRESULT(0)
        }

        WM_DESTROY => {
            if crate::single_instance::is_gui_running() {
                crate::single_instance::close_existing_gui_window();
            }
            PostQuitMessage(0);
            LRESULT(0)
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn spawn_or_show_gui() {
    if crate::single_instance::focus_existing_gui_window() {
        return;
    }

    if let Ok(exe_path) = std::env::current_exe() {
        let mut cmd = std::process::Command::new(exe_path);
        cmd.arg("--gui");
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
        let _ = cmd.spawn();
    }
}
