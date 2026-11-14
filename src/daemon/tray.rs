use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Mutex as TokioMutex;
use crate::daemon::engine::DaemonEngine;

pub static RUNNING: AtomicBool = AtomicBool::new(true);
static WM_TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static GLOBAL_ICON_PATH: Mutex<Option<String>> = Mutex::new(None);
static TRAY_HWND: AtomicIsize = AtomicIsize::new(0);
static HIDE_TRAY_ICON: AtomicBool = AtomicBool::new(false);
static IS_MASKED: AtomicBool = AtomicBool::new(false);

static AVAILABLE_UPDATE: Mutex<Option<crate::updater::ReleaseInfo>> = Mutex::new(None);
static GLOBAL_ENGINE: Mutex<Option<Arc<TokioMutex<DaemonEngine>>>> = Mutex::new(None);
static GLOBAL_RT: Mutex<Option<tokio::runtime::Handle>> = Mutex::new(None);

pub fn set_available_update(info: Option<crate::updater::ReleaseInfo>) {
    if let Ok(mut lock) = AVAILABLE_UPDATE.lock() {
        *lock = info;
    }
}

pub fn get_available_update() -> Option<crate::updater::ReleaseInfo> {
    AVAILABLE_UPDATE.lock().ok().and_then(|l| l.clone())
}

static IS_UPDATING: AtomicBool = AtomicBool::new(false);
static UPDATE_PROGRESS_PCT: AtomicU32 = AtomicU32::new(0);
static UPDATE_TASK_HANDLE: Mutex<Option<tokio::task::AbortHandle>> = Mutex::new(None);

pub fn set_updating(updating: bool, progress: Option<f32>) {
    IS_UPDATING.store(updating, Ordering::SeqCst);
    let pct = progress.map(|p| (p * 100.0).clamp(0.0, 100.0) as u32).unwrap_or(0);
    UPDATE_PROGRESS_PCT.store(pct, Ordering::SeqCst);
}

pub fn is_updating() -> bool {
    IS_UPDATING.load(Ordering::SeqCst)
}

#[allow(dead_code)]
pub fn get_update_progress_pct() -> u32 {
    UPDATE_PROGRESS_PCT.load(Ordering::SeqCst)
}

pub fn abort_active_update() {
    if let Ok(mut lock) = UPDATE_TASK_HANDLE.lock() {
        if let Some(handle) = lock.take() {
            handle.abort();
        }
    }
    set_updating(false, None);
    crate::updater::clean_temp_update_files();
    crate::daemon::overlay_manager::hide_update_progress_overlay();
}

#[cfg(windows)]
use windows::core::{w, PCWSTR, PWSTR};
#[cfg(windows)]
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    CreateDIBSection, DeleteObject, GetDC, ReleaseDC, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HBITMAP,
};
#[cfg(windows)]
use windows::Win32::UI::Shell::{
    ExtractIconExW, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DispatchMessageW,
    GetCursorPos, GetMessageW, GetSystemMetrics, InsertMenuItemW, LoadIconW, LoadImageW,
    PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SetForegroundWindow,
    SetMenuDefaultItem, TrackPopupMenu, TranslateMessage, HICON, IDI_APPLICATION, IMAGE_ICON,
    LR_LOADFROMFILE, MENUITEMINFOW, MFS_DISABLED, MFT_SEPARATOR, MIIM_BITMAP, MIIM_FTYPE, MIIM_ID,
    MIIM_STATE, MIIM_STRING, SM_CXSMICON, SM_CYSMICON, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RIGHTBUTTON,
    WM_APP, WM_COMMAND, WM_DESTROY, WM_LBUTTONDBLCLK, WM_NULL, WM_RBUTTONUP, WNDCLASSW,
};
#[cfg(windows)]
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};

const WM_TRAYICON: u32 = WM_APP + 1;
const WM_UPDATE_ICON: u32 = WM_APP + 2;
const IDM_OPEN: usize = 1001;
const IDM_UPDATE: usize = 1002;
const IDM_EXIT: usize = 1003;

#[cfg(windows)]
pub fn load_tray_icon(custom_path: Option<&str>) -> HICON {
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
                                return HICON(handle.0);
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
                            return hicon_small;
                        }
                    }
                }
            }
        }

        // 2. Try loading from current executable (which has app.ico embedded at resource ID 1)
        if let Ok(hinst) = GetModuleHandleW(None) {
            if let Ok(icon) = LoadIconW(Some(hinst.into()), PCWSTR(1 as _)) {
                if !icon.is_invalid() {
                    return icon;
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
                return hicon_small;
            }
        }

        // 3. Fallback
        LoadIconW(None, IDI_APPLICATION).unwrap_or_default()
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
        IS_MASKED.store(is_masked, Ordering::SeqCst);
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

            let hicon = load_tray_icon(tray_icon_path.as_deref());

            let mut tip = [0u16; 128];
            let tip_str = if is_masked { "Windows Service Host\0" } else { "Synector\0" };
            let tip_text = tip_str.encode_utf16().collect::<Vec<_>>();
            tip[..tip_text.len().min(128)].copy_from_slice(&tip_text[..tip_text.len().min(128)]);

            let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = 1;
            nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
            nid.uCallbackMessage = WM_TRAYICON;
            nid.hIcon = hicon;
            nid.szTip = tip;

            let msg_id = RegisterWindowMessageW(w!("TaskbarCreated"));
            WM_TASKBAR_CREATED.store(msg_id, Ordering::SeqCst);
            TRAY_HWND.store(hwnd.0 as isize, Ordering::SeqCst);

            let config = crate::config::AppConfig::load_from_file(crate::config::AppConfig::config_path());
            let is_dark = config.theme != crate::config::AppTheme::Light;
            apply_menu_theme(Some(hwnd), is_dark);

            if !hide_tray_icon {
                let _ = Shell_NotifyIconW(NIM_ADD, &nid);
            }

            Ok(Self { _hwnd: hwnd, nid })
        }

        #[cfg(not(windows))]
        Ok(Self {})
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
    unsafe {
        let raw = TRAY_HWND.load(Ordering::SeqCst);
        if raw != 0 {
            let hwnd = HWND(raw as *mut _);
            let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = 1;
            if visible {
                let icon_path = GLOBAL_ICON_PATH.lock().ok().and_then(|l| l.clone());
                let hicon = load_tray_icon(icon_path.as_deref());
                let mut tip = [0u16; 128];
                let tip_str = if IS_MASKED.load(Ordering::SeqCst) { "Windows Service Host\0" } else { "Synector\0" };
                let tip_text = tip_str.encode_utf16().collect::<Vec<_>>();
                tip[..tip_text.len().min(128)].copy_from_slice(&tip_text[..tip_text.len().min(128)]);
                nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
                nid.uCallbackMessage = WM_TRAYICON;
                nid.hIcon = hicon;
                nid.szTip = tip;
                let _ = Shell_NotifyIconW(NIM_ADD, &nid);
            } else {
                let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            }
        }
    }
}

#[cfg(windows)]
unsafe extern "system" fn tray_wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let taskbar_msg = WM_TASKBAR_CREATED.load(Ordering::SeqCst);
    if taskbar_msg != 0 && msg == taskbar_msg {
        if !HIDE_TRAY_ICON.load(Ordering::SeqCst) {
            let icon_path = GLOBAL_ICON_PATH.lock().ok().and_then(|l| l.clone());
            let hicon = load_tray_icon(icon_path.as_deref());
            let mut tip = [0u16; 128];
            let tip_str = if IS_MASKED.load(Ordering::SeqCst) { "Windows Service Host\0" } else { "Synector\0" };
            let tip_text = tip_str.encode_utf16().collect::<Vec<_>>();
            tip[..tip_text.len().min(128)].copy_from_slice(&tip_text[..tip_text.len().min(128)]);

            let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = 1;
            nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
            nid.uCallbackMessage = WM_TRAYICON;
            nid.hIcon = hicon;
            nid.szTip = tip;
            let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        }
        return LRESULT(0);
    }

    match msg {
        WM_UPDATE_ICON => {
            if !HIDE_TRAY_ICON.load(Ordering::SeqCst) {
                let icon_path = GLOBAL_ICON_PATH.lock().ok().and_then(|l| l.clone());
                let hicon = load_tray_icon(icon_path.as_deref());
                let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
                nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
                nid.hWnd = hwnd;
                nid.uID = 1;
                nid.uFlags = NIF_ICON;
                nid.hIcon = hicon;
                let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
            }
            LRESULT(0)
        }

        WM_TRAYICON => {
            let event = lparam.0 as u32;
            if event == WM_LBUTTONDBLCLK {
                spawn_or_show_gui();
            } else if event == WM_RBUTTONUP {
                let config = crate::config::AppConfig::load_from_file(crate::config::AppConfig::config_path());
                let is_dark = config.theme != crate::config::AppTheme::Light;
                apply_menu_theme(Some(hwnd), is_dark);

                let loc = crate::i18n::get(config.language);
                let mut open_w: Vec<u16> = loc.tray_menu_open.encode_utf16().chain(std::iter::once(0)).collect();
                let mut exit_w: Vec<u16> = loc.tray_menu_exit.encode_utf16().chain(std::iter::once(0)).collect();

                let available_update = get_available_update();
                let icon_size = GetSystemMetrics(SM_CXSMICON)
                    .max(GetSystemMetrics(SM_CYSMICON))
                    .max(16) as u32;
                let hbmp_open = generate_open_menu_bitmap(icon_size, is_dark);
                let hbmp_update = if available_update.is_some() {
                    generate_update_menu_bitmap(icon_size, is_dark)
                } else {
                    None
                };
                let hbmp_exit = generate_exit_menu_bitmap(icon_size, is_dark);

                if let Ok(hmenu) = CreatePopupMenu() {
                    let mut menu_idx = 0;

                    // 1. Open Synector (Bold default item with app window / dashboard icon)
                    let mut mask_open = MIIM_ID | MIIM_STRING;
                    if hbmp_open.is_some() {
                        mask_open |= MIIM_BITMAP;
                    }
                    let item_open = MENUITEMINFOW {
                        cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
                        fMask: mask_open,
                        wID: IDM_OPEN as u32,
                        dwTypeData: PWSTR(open_w.as_mut_ptr()),
                        cch: open_w.len() as u32,
                        hbmpItem: hbmp_open.unwrap_or_default(),
                        ..Default::default()
                    };
                    let _ = InsertMenuItemW(hmenu, menu_idx, true, &item_open);
                    let _ = SetMenuDefaultItem(hmenu, IDM_OPEN as u32, 0);
                    menu_idx += 1;

                    // 2. Update to vX.X.X / Updating... (If available or updating, placed directly under Open)
                    let mut update_w: Option<Vec<u16>> = None;
                    let is_upd = is_updating();
                    if is_upd || available_update.is_some() {
                        let update_label = if is_upd {
                            loc.tray_menu_updating.to_string()
                        } else if let Some(ref info) = available_update {
                            loc.tray_menu_update.replace("{}", &info.version)
                        } else {
                            loc.tray_menu_updating.to_string()
                        };
                        let mut w_str: Vec<u16> = update_label.encode_utf16().chain(std::iter::once(0)).collect();
                        let mut mask_upd = MIIM_ID | MIIM_STRING;
                        if is_upd {
                            mask_upd |= MIIM_STATE;
                        }
                        if hbmp_update.is_some() {
                            mask_upd |= MIIM_BITMAP;
                        }
                        let item_update = MENUITEMINFOW {
                            cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
                            fMask: mask_upd,
                            fState: if is_upd { MFS_DISABLED } else { Default::default() },
                            wID: IDM_UPDATE as u32,
                            dwTypeData: PWSTR(w_str.as_mut_ptr()),
                            cch: w_str.len() as u32,
                            hbmpItem: hbmp_update.unwrap_or_default(),
                            ..Default::default()
                        };
                        let _ = InsertMenuItemW(hmenu, menu_idx, true, &item_update);
                        update_w = Some(w_str);
                        menu_idx += 1;
                    }

                    // 3. Separator
                    let sep = MENUITEMINFOW {
                        cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
                        fMask: MIIM_FTYPE,
                        fType: MFT_SEPARATOR,
                        ..Default::default()
                    };
                    let _ = InsertMenuItemW(hmenu, menu_idx, true, &sep);
                    menu_idx += 1;

                    // 4. Exit (with power/exit icon)
                    let mut mask_exit = MIIM_ID | MIIM_STRING;
                    if hbmp_exit.is_some() {
                        mask_exit |= MIIM_BITMAP;
                    }
                    let item_exit = MENUITEMINFOW {
                        cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
                        fMask: mask_exit,
                        wID: IDM_EXIT as u32,
                        dwTypeData: PWSTR(exit_w.as_mut_ptr()),
                        cch: exit_w.len() as u32,
                        hbmpItem: hbmp_exit.unwrap_or_default(),
                        ..Default::default()
                    };
                    let _ = InsertMenuItemW(hmenu, menu_idx, true, &item_exit);

                    let mut pt = POINT::default();
                    let _ = GetCursorPos(&mut pt);
                    let _ = SetForegroundWindow(hwnd);
                    let _ = TrackPopupMenu(
                        hmenu,
                        TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RIGHTBUTTON,
                        pt.x,
                        pt.y,
                        None,
                        hwnd,
                        None,
                    );
                    let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
                    let _ = DestroyMenu(hmenu);
                    drop(update_w);
                }

                if let Some(bmp) = hbmp_open {
                    let _ = DeleteObject(bmp.into());
                }
                if let Some(bmp) = hbmp_update {
                    let _ = DeleteObject(bmp.into());
                }
                if let Some(bmp) = hbmp_exit {
                    let _ = DeleteObject(bmp.into());
                }
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

pub fn start_update_download(show_overlay: bool) {
    let update_info = match get_available_update() {
        Some(info) => info,
        None => return,
    };

    if is_updating() {
        return;
    }

    set_updating(true, Some(0.0));
    let version = update_info.version.clone();

    if show_overlay {
        crate::daemon::overlay_manager::show_update_progress_overlay(&version);
    }

    if let Ok(lock) = GLOBAL_ENGINE.lock() {
        if let Some(engine) = lock.as_ref() {
            if let Ok(rt_lock) = GLOBAL_RT.lock() {
                if let Some(rt) = rt_lock.as_ref() {
                    let eng_clone = Arc::clone(engine);
                    let asset_url = update_info.asset_url.clone();
                    let ver = version.clone();
                    let handle = rt.spawn(async move {
                        {
                            let mut eng = eng_clone.lock().await;
                            eng.is_downloading_update = true;
                            eng.update_download_progress = 0.0;
                            eng.update_status = Some("Connecting...".to_string());
                        }

                        let eng_progress = Arc::clone(&eng_clone);
                        let ver_for_overlay = ver.clone();
                        let res = crate::updater::download_and_install_daemon(&asset_url, move |pct| {
                            set_updating(true, Some(pct));
                            let gui_open = crate::single_instance::is_gui_running();
                            let active = crate::daemon::overlay_manager::get_active_overlay_ids();
                            if !gui_open {
                                if !active.contains("overlay_update_progress") {
                                    crate::daemon::overlay_manager::show_update_progress_overlay(&ver_for_overlay);
                                }
                            } else if active.contains("overlay_update_progress") {
                                crate::daemon::overlay_manager::hide_update_progress_overlay();
                            }
                            let eng_p = Arc::clone(&eng_progress);
                            tokio::spawn(async move {
                                let mut eng = eng_p.lock().await;
                                eng.update_download_progress = pct;
                                eng.update_status = Some(format!("{:.0}%", pct * 100.0));
                            });
                        }).await;

                        match res {
                            Ok(()) => {
                                // Successfully applied update; apply_update_and_restart exits process
                            }
                            Err(err) => {
                                eprintln!("[DAEMON] Update error: {err}");
                                set_updating(false, None);
                                crate::updater::clean_temp_update_files();
                                let mut eng = eng_clone.lock().await;
                                eng.is_downloading_update = false;
                                eng.update_status = Some(format!("Error: {err}"));
                                eng.log("ERROR", "UPDATER", &format!("Update failed: {err}"));
                                tokio::time::sleep(tokio::time::Duration::from_millis(4000)).await;
                                crate::daemon::overlay_manager::hide_update_progress_overlay();
                            }
                        }
                    });

                    if let Ok(mut h_lock) = UPDATE_TASK_HANDLE.lock() {
                        *h_lock = Some(handle.abort_handle());
                    }
                }
            }
        }
    }
}

pub fn trigger_update_flow(silent: bool) {
    if is_updating() {
        return;
    }

    if get_available_update().is_none() {
        return;
    }

    if crate::single_instance::is_gui_running() {
        // 1. If GUI is open
        if !silent {
            crate::single_instance::focus_existing_gui_window();
            if let Ok(lock) = GLOBAL_ENGINE.lock() {
                if let Some(engine) = lock.as_ref() {
                    if let Ok(rt_lock) = GLOBAL_RT.lock() {
                        if let Some(rt) = rt_lock.as_ref() {
                            let eng_clone = Arc::clone(engine);
                            rt.spawn(async move {
                                let mut eng = eng_clone.lock().await;
                                eng.pending_open_update_modal = true;
                            });
                        }
                    }
                }
            }
        } else {
            start_update_download(false);
        }
    } else {
        // 2. If GUI is closed and only daemon is running:
        start_update_download(!silent);
    }
}

fn handle_tray_update_click() {
    trigger_update_flow(false);
}

#[cfg(windows)]
pub fn apply_menu_theme(hwnd: Option<HWND>, is_dark: bool) {
    unsafe {
        // uxtheme.dll ordinals:
        // 135: SetPreferredAppMode (0 = Default, 1 = AllowDark, 2 = ForceDark, 3 = ForceLight)
        // 136: FlushMenuThemes
        // 133: AllowDarkModeForWindow
        if let Ok(uxtheme) = LoadLibraryW(w!("uxtheme.dll")) {
            let mode = if is_dark { 2 } else { 3 };
            let set_preferred_app_mode: Option<unsafe extern "system" fn(i32) -> i32> =
                std::mem::transmute(GetProcAddress(uxtheme, windows::core::PCSTR(135 as _)));
            if let Some(set_mode) = set_preferred_app_mode {
                set_mode(mode);
            }

            let flush_menu_themes: Option<unsafe extern "system" fn()> =
                std::mem::transmute(GetProcAddress(uxtheme, windows::core::PCSTR(136 as _)));
            if let Some(flush) = flush_menu_themes {
                flush();
            }

            if let Some(h) = hwnd {
                let allow_dark_for_window: Option<unsafe extern "system" fn(HWND, bool) -> bool> =
                    std::mem::transmute(GetProcAddress(uxtheme, windows::core::PCSTR(133 as _)));
                if let Some(allow_wnd) = allow_dark_for_window {
                    let _ = allow_wnd(h, is_dark);
                }
            }
        }
    }
}

#[cfg(windows)]
unsafe fn create_pargb_bitmap(width: i32, height: i32, rgba_pixels: &[u8]) -> Option<HBITMAP> {
    if width <= 0 || height <= 0 || rgba_pixels.len() < (width * height * 4) as usize {
        return None;
    }

    let hdc_screen = GetDC(None);
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut bits_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
    let hbmp = CreateDIBSection(Some(hdc_screen), &bmi, DIB_RGB_COLORS, &mut bits_ptr, None, 0).ok()?;
    ReleaseDC(None, hdc_screen);

    if bits_ptr.is_null() {
        let _ = DeleteObject(hbmp.into());
        return None;
    }

    let dest = std::slice::from_raw_parts_mut(bits_ptr as *mut u8, (width * height * 4) as usize);

    #[allow(clippy::chunks_exact_to_as_chunks)]
    for (src, dst) in rgba_pixels.chunks_exact(4).zip(dest.chunks_exact_mut(4)) {
        let r = src[0] as u32;
        let g = src[1] as u32;
        let b = src[2] as u32;
        let a = src[3] as u32;

        if a == 0 {
            dst[0] = 0;
            dst[1] = 0;
            dst[2] = 0;
            dst[3] = 0;
        } else if a == 255 {
            dst[0] = b as u8;
            dst[1] = g as u8;
            dst[2] = r as u8;
            dst[3] = 255;
        } else {
            dst[0] = ((b * a + 127) / 255) as u8;
            dst[1] = ((g * a + 127) / 255) as u8;
            dst[2] = ((r * a + 127) / 255) as u8;
            dst[3] = a as u8;
        }
    }

    Some(hbmp)
}

#[cfg(windows)]
fn dist_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let vx = b.0 - a.0;
    let vy = b.1 - a.1;
    let wx = p.0 - a.0;
    let wy = p.1 - a.1;
    let c1 = wx * vx + wy * vy;
    if c1 <= 0.0 {
        return (wx * wx + wy * wy).sqrt();
    }
    let c2 = vx * vx + vy * vy;
    if c2 <= c1 {
        let dx = p.0 - b.0;
        let dy = p.1 - b.1;
        return (dx * dx + dy * dy).sqrt();
    }
    let t = c1 / c2;
    let qx = a.0 + t * vx;
    let qy = a.1 + t * vy;
    let dx = p.0 - qx;
    let dy = p.1 - qy;
    (dx * dx + dy * dy).sqrt()
}

#[cfg(windows)]
fn sd_rounded_rect(p: (f32, f32), min_pt: (f32, f32), max_pt: (f32, f32), r: f32) -> f32 {
    let cx = (min_pt.0 + max_pt.0) * 0.5;
    let cy = (min_pt.1 + max_pt.1) * 0.5;
    let hx = (max_pt.0 - min_pt.0) * 0.5 - r;
    let hy = (max_pt.1 - min_pt.1) * 0.5 - r;
    let qx = (p.0 - cx).abs() - hx;
    let qy = (p.1 - cy).abs() - hy;
    let ox = qx.max(0.0);
    let oy = qy.max(0.0);
    let out_d = (ox * ox + oy * oy).sqrt();
    let in_d = qx.max(qy).min(0.0);
    out_d + in_d - r
}

#[cfg(windows)]
fn generate_open_menu_bitmap(size: u32, is_dark: bool) -> Option<HBITMAP> {
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let s = size as f32;

    let min_pt = (s * 0.14, s * 0.16);
    let max_pt = (s * 0.86, s * 0.84);
    let r = (s * 0.08).clamp(1.0, 2.0);
    let hw = (s * 0.045).clamp(0.65, 1.0);
    let y_hdr = s * 0.38;

    // Monochrome theme adaptation: crisp white on dark, crisp black on light
    let (r_col, g_col, b_col) = if is_dark {
        (245u8, 245u8, 250u8) // Crisp white
    } else {
        (25u8, 25u8, 30u8)    // Crisp black/charcoal
    };

    let offsets = [(-0.25f32, -0.25f32), (0.25, -0.25), (-0.25, 0.25), (0.25, 0.25)];

    for y in 0..size {
        for x in 0..size {
            let mut cov = 0.0f32;
            for (ox, oy) in offsets {
                let p = (x as f32 + 0.5 + ox, y as f32 + 0.5 + oy);

                let d_frame = sd_rounded_rect(p, min_pt, max_pt, r).abs() - hw;
                let d_hdr = dist_to_segment(p, (min_pt.0 + hw, y_hdr), (max_pt.0 - hw, y_hdr)) - hw;

                let min_d = d_frame.min(d_hdr);
                cov += (0.5 - min_d).clamp(0.0, 1.0);
            }
            cov *= 0.25;

            if cov > 0.015 {
                let idx = ((y * size + x) * 4) as usize;
                let a_byte = (cov * 255.0) as u8;
                rgba[idx] = r_col;
                rgba[idx + 1] = g_col;
                rgba[idx + 2] = b_col;
                rgba[idx + 3] = a_byte;
            }
        }
    }

    unsafe { create_pargb_bitmap(size as i32, size as i32, &rgba) }
}

#[cfg(windows)]
fn generate_exit_menu_bitmap(size: u32, is_dark: bool) -> Option<HBITMAP> {
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let s = size as f32;
    let cx = s * 0.5;
    let cy = s * 0.5;
    let r = s * 0.32;
    let hw = (s * 0.055).clamp(0.85, 1.4);

    let rad_left = (-130.0f32).to_radians();
    let rad_right = (-50.0f32).to_radians();
    let p_left = (cx + r * rad_left.cos(), cy + r * rad_left.sin());
    let p_right = (cx + r * rad_right.cos(), cy + r * rad_right.sin());

    // Monochrome theme adaptation: crisp white on dark, crisp black on light
    let (r_col, g_col, b_col) = if is_dark {
        (245u8, 245u8, 250u8) // Crisp white
    } else {
        (25u8, 25u8, 30u8)    // Crisp black/charcoal
    };

    let offsets = [(-0.25f32, -0.25f32), (0.25, -0.25), (-0.25, 0.25), (0.25, 0.25)];

    for y in 0..size {
        for x in 0..size {
            let mut cov = 0.0f32;
            for (ox, oy) in offsets {
                let p = (x as f32 + 0.5 + ox, y as f32 + 0.5 + oy);
                let dx = p.0 - cx;
                let dy = p.1 - cy;
                let dist_from_c = (dx * dx + dy * dy).sqrt();
                let angle_deg = dy.atan2(dx).to_degrees();

                let in_top_gap = angle_deg < -50.0 && angle_deg > -130.0;
                let d_arc = if in_top_gap {
                    let d_left = ((p.0 - p_left.0).powi(2) + (p.1 - p_left.1).powi(2)).sqrt() - hw;
                    let d_right = ((p.0 - p_right.0).powi(2) + (p.1 - p_right.1).powi(2)).sqrt() - hw;
                    d_left.min(d_right)
                } else {
                    (dist_from_c - r).abs() - hw
                };

                let d_line = dist_to_segment(p, (cx, cy - s * 0.38), (cx, cy - s * 0.04)) - hw;
                let min_d = d_arc.min(d_line);
                cov += (0.5 - min_d).clamp(0.0, 1.0);
            }
            cov *= 0.25;

            if cov > 0.015 {
                let idx = ((y * size + x) * 4) as usize;
                let a_byte = (cov * 255.0) as u8;
                rgba[idx] = r_col;
                rgba[idx + 1] = g_col;
                rgba[idx + 2] = b_col;
                rgba[idx + 3] = a_byte;
            }
        }
    }

    unsafe { create_pargb_bitmap(size as i32, size as i32, &rgba) }
}

#[cfg(windows)]
pub fn generate_update_menu_bitmap(size: u32, is_dark: bool) -> Option<HBITMAP> {
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let s = size as f32;
    let cx = s * 0.5;
    let cy = s * 0.5;
    let hw = (s * 0.055).clamp(0.8, 1.3);

    // Accent emerald green color for update action
    let (r_col, g_col, b_col) = if is_dark {
        (0u8, 215u8, 120u8)
    } else {
        (0u8, 140u8, 80u8)
    };

    let p_shaft_top = (cx, cy - s * 0.30);
    let p_shaft_bot = (cx, cy + s * 0.14);
    let p_tip = (cx, cy + s * 0.14);
    let p_left = (cx - s * 0.22, cy - s * 0.04);
    let p_right = (cx + s * 0.22, cy - s * 0.04);
    let p_tray_left = (cx - s * 0.28, cy + s * 0.32);
    let p_tray_right = (cx + s * 0.28, cy + s * 0.32);

    let offsets = [(-0.25f32, -0.25f32), (0.25, -0.25), (-0.25, 0.25), (0.25, 0.25)];

    for y in 0..size {
        for x in 0..size {
            let mut cov = 0.0f32;
            for (ox, oy) in offsets {
                let p = (x as f32 + 0.5 + ox, y as f32 + 0.5 + oy);
                let d_shaft = dist_to_segment(p, p_shaft_top, p_shaft_bot) - hw;
                let d_head1 = dist_to_segment(p, p_left, p_tip) - hw;
                let d_head2 = dist_to_segment(p, p_right, p_tip) - hw;
                let d_tray = dist_to_segment(p, p_tray_left, p_tray_right) - hw;

                let min_d = d_shaft.min(d_head1).min(d_head2).min(d_tray);
                cov += (0.5 - min_d).clamp(0.0, 1.0);
            }
            cov *= 0.25;

            if cov > 0.015 {
                let idx = ((y * size + x) * 4) as usize;
                let a_byte = (cov * 255.0) as u8;
                rgba[idx] = r_col;
                rgba[idx + 1] = g_col;
                rgba[idx + 2] = b_col;
                rgba[idx + 3] = a_byte;
            }
        }
    }

    unsafe { create_pargb_bitmap(size as i32, size as i32, &rgba) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn test_tray_menu_bitmaps_creation_and_cleanup() {
        for is_dark in [true, false] {
            for size in [16, 20, 24, 32] {
                let open_bmp = generate_open_menu_bitmap(size, is_dark);
                assert!(open_bmp.is_some(), "Open menu bitmap should be generated successfully for size {size}, dark={is_dark}");
                if let Some(bmp) = open_bmp {
                    unsafe {
                        let _ = windows::Win32::Graphics::Gdi::DeleteObject(bmp.into());
                    }
                }

                let update_bmp = generate_update_menu_bitmap(size, is_dark);
                assert!(update_bmp.is_some(), "Update menu bitmap should be generated successfully for size {size}, dark={is_dark}");
                if let Some(bmp) = update_bmp {
                    unsafe {
                        let _ = windows::Win32::Graphics::Gdi::DeleteObject(bmp.into());
                    }
                }

                let exit_bmp = generate_exit_menu_bitmap(size, is_dark);
                assert!(exit_bmp.is_some(), "Exit menu bitmap should be generated successfully for size {size}, dark={is_dark}");
                if let Some(bmp) = exit_bmp {
                    unsafe {
                        let _ = windows::Win32::Graphics::Gdi::DeleteObject(bmp.into());
                    }
                }
            }
        }
    }

    #[test]
    fn test_tray_available_update_state() {
        set_available_update(None);
        assert!(get_available_update().is_none());

        let release = crate::updater::ReleaseInfo {
            version: "3.0.0".to_string(),
            tag_name: "v3.0.0".to_string(),
            name: "v3.0.0".to_string(),
            body: "".to_string(),
            html_url: "".to_string(),
            asset_url: "".to_string(),
            asset_name: "".to_string(),
            asset_size: 100,
        };

        set_available_update(Some(release.clone()));
        assert_eq!(get_available_update(), Some(release));

        set_available_update(None);
        assert!(get_available_update().is_none());
    }

    #[test]
    fn test_tray_updating_state() {
        set_updating(false, None);
        assert!(!is_updating());
        assert_eq!(get_update_progress_pct(), 0);

        set_updating(true, Some(0.42));
        assert!(is_updating());
        assert_eq!(get_update_progress_pct(), 42);

        set_updating(false, None);
        assert!(!is_updating());
    }
}
