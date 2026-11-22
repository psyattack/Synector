use std::collections::{HashMap, HashSet};
use std::process::{Child, Command};
use std::sync::{Arc, Mutex, OnceLock};

use crate::models::{MonitorInfo, OverlayDefinition};

pub struct ActiveOverlayProcess {
    pub id: String,
    pub child: Option<Child>,
    pub close_with_target: bool,
    pub target_process: String,
    pub fade_out_ms: Option<u32>,
}

static ACTIVE_PROCESSES: OnceLock<Arc<Mutex<HashMap<String, ActiveOverlayProcess>>>> = OnceLock::new();
type CalibratedPositionsMap = Arc<Mutex<HashMap<String, (f32, f32)>>>;
static CALIBRATED_POSITIONS: OnceLock<CalibratedPositionsMap> = OnceLock::new();
static OVERLAY_TEXTS: OnceLock<Arc<Mutex<HashMap<String, String>>>> = OnceLock::new();

fn active_map() -> &'static Arc<Mutex<HashMap<String, ActiveOverlayProcess>>> {
    ACTIVE_PROCESSES.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

fn calibrated_map() -> &'static Arc<Mutex<HashMap<String, (f32, f32)>>> {
    CALIBRATED_POSITIONS.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

fn texts_map() -> &'static Arc<Mutex<HashMap<String, String>>> {
    OVERLAY_TEXTS.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

pub fn get_overlay_text(id: &str) -> Option<String> {
    texts_map().lock().unwrap().get(id).cloned()
}

fn live_text_dir() -> std::path::PathBuf {
    crate::config::AppConfig::config_dir().join("overlay_text")
}

/// Path of the live-text file for `id`. Overlay windows poll this file so a
/// `SetText` update repaints the label instead of restarting the process.
pub fn live_overlay_text_path(id: &str) -> std::path::PathBuf {
    use sha2::{Digest, Sha256};
    let slug: String = id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .take(48)
        .collect();
    let digest = Sha256::digest(id.as_bytes());
    let suffix = digest[..4].iter().map(|b| format!("{b:02x}")).collect::<String>();
    live_text_dir().join(format!("{slug}-{suffix}.txt"))
}

fn write_live_overlay_text(id: &str, text: &str) {
    static WARN_ONCE: std::sync::Once = std::sync::Once::new();
    let path = live_overlay_text_path(id);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut tmp_name = path.as_os_str().to_os_string();
    tmp_name.push(format!(".{}.tmp", std::process::id()));
    let tmp = std::path::PathBuf::from(tmp_name);
    let written = std::fs::write(&tmp, text).is_ok()
        && match std::fs::rename(&tmp, &path) {
            Ok(()) => true,
            Err(_) => {
                let _ = std::fs::remove_file(&tmp);
                false
            }
        };
    if !written {
        WARN_ONCE.call_once(|| {
            crate::logging::warn(
                "OVERLAY",
                "Cannot publish live overlay text; SetText updates will not be shown",
            );
        });
    }
}

/// Latest live text published for `id`, read by the overlay window. `None`
/// means no text has been published for this overlay yet.
pub fn live_overlay_text(id: &str) -> Option<String> {
    std::fs::read_to_string(live_overlay_text_path(id)).ok()
}

pub fn update_overlay_text(id_or_name: &str, text: &str) {
    let config = crate::config::Library::shared();
    let resolved_id = config
        .find_overlay(id_or_name)
        .map(|o| o.id.clone())
        .unwrap_or_else(|| id_or_name.to_string());
    texts_map().lock().unwrap().insert(resolved_id.clone(), text.to_string());
    // Publish the new label for the running window; it polls this file, so no
    // kill/respawn (and no flicker or GPU startup storm) per text frame.
    write_live_overlay_text(&resolved_id, text);
}

/// Returns the live text only when it differs from `current`, so the overlay
/// window can swap it without re-reading every frame.
pub fn live_overlay_text_if_changed(id: &str, current: Option<&str>) -> Option<String> {
    let text = live_overlay_text(id)?;
    if current == Some(text.as_str()) {
        None
    } else {
        Some(text)
    }
}

pub fn init() {
    let _ = active_map();
    let _ = calibrated_map();
    let _ = texts_map();
}

pub fn show_overlay(overlay: &OverlayDefinition, calibration: bool, override_text: Option<String>) {
    let effective_text = override_text.or_else(|| get_overlay_text(&overlay.id));
    if let Some(ref txt) = effective_text {
        texts_map().lock().unwrap().insert(overlay.id.clone(), txt.clone());
        write_live_overlay_text(&overlay.id, txt);
    }
    let b64 = crate::ipc::overlay_wire::encode_overlay_b64(overlay);

    let mut lock = active_map().lock().unwrap();
    if let Some(mut existing) = lock.remove(&overlay.id) {
        if let Some(mut ch) = existing.child.take() {
            let _ = ch.kill();
        }
    }

    let child = if cfg!(test) {
        None
    } else {
        let exe = std::env::current_exe().unwrap_or_default();
        let mut cmd = Command::new(&exe);
        cmd.arg(crate::arg_spellings::OVERLAY_WINDOW).arg(&overlay.id);

        if calibration {
            cmd.arg(crate::arg_spellings::CALIBRATION);
        }
        if let Some(txt) = effective_text {
            cmd.arg(crate::arg_spellings::OVERRIDE_TEXT).arg(txt);
        }
        if let Some(b64_str) = b64 {
            cmd.arg(crate::arg_spellings::OVERLAY_DATA).arg(b64_str);
        }
        if !calibration {
            if let Some(s) = overlay.auto_hide_seconds {
                if s > 0.0 {
                    cmd.arg(crate::arg_spellings::DURATION)
                        .arg(((s * 1000.0).round() as u32).to_string());
                }
            }
        }

        spawn_overlay_process(&overlay.id, cmd)
    };

    if child.is_none() && !cfg!(test) {
        // Never register an overlay that has no process behind it: a failed spawn
        // must not be reported as active.
        return;
    }

    lock.insert(
        overlay.id.clone(),
        ActiveOverlayProcess {
            id: overlay.id.clone(),
            child,
            close_with_target: overlay.close_with_target,
            target_process: overlay.target_process.clone().unwrap_or_default(),
            fade_out_ms: overlay.fade_out_ms,
        },
    );
}

/// Spawns an overlay window and logs a failure with the overlay id. Spawn errors
/// used to be swallowed to `None`, leaving a permanently "active" overlay that
/// nothing could see.
fn spawn_overlay_process(overlay_id: &str, command: Command) -> Option<Child> {
    let mut command = command;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    match command.spawn() {
        Ok(child) => {
            #[cfg(windows)]
            job::assign_overlay_child(&child);
            Some(child)
        }
        Err(err) => {
            crate::logging::error(
                "OVERLAY",
                &format!("Failed to spawn overlay window '{overlay_id}': {err}"),
            );
            None
        }
    }
}

/// Kill-on-close Job Object for overlay children: Windows terminates every
/// overlay process when the daemon handle set closes, including a daemon crash
/// or forced kill that never runs the shutdown path.
#[cfg(windows)]
mod job {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use std::sync::OnceLock;

    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS: i32 = 9;

    #[repr(C)]
    #[derive(Default)]
    struct JobObjectBasicLimitInformation {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct IoCounters {
        read_operation_count: u64,
        write_operation_count: u64,
        other_operation_count: u64,
        read_transfer_count: u64,
        write_transfer_count: u64,
        other_transfer_count: u64,
    }

    #[repr(C)]
    #[derive(Default)]
    struct JobObjectExtendedLimitInformation {
        basic_limit_information: JobObjectBasicLimitInformation,
        io_info: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> *mut c_void;
        fn SetInformationJobObject(
            job: *mut c_void,
            info_class: i32,
            info: *const c_void,
            length: u32,
        ) -> i32;
        fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }

    fn overlay_job() -> Option<*mut c_void> {
        static JOB: OnceLock<usize> = OnceLock::new();
        let handle = *JOB.get_or_init(|| unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return 0;
            }
            let mut info = JobObjectExtendedLimitInformation::default();
            info.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let configured = SetInformationJobObject(
                job,
                JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
                &info as *const _ as *const c_void,
                std::mem::size_of::<JobObjectExtendedLimitInformation>() as u32,
            );
            if configured == 0 {
                let _ = CloseHandle(job);
                return 0;
            }
            job as usize
        });
        if handle == 0 { None } else { Some(handle as *mut c_void) }
    }

    /// Assigns a freshly spawned overlay to the daemon's job. The handle is
    /// deliberately kept open for the process lifetime; closing it (process
    /// exit) is what triggers the kill-on-close contract.
    pub fn assign_overlay_child(child: &Child) {
        if let Some(job) = overlay_job() {
            unsafe {
                let _ = AssignProcessToJobObject(job, child.as_raw_handle());
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_overlay_job_exists_and_is_shared() {
            let job = overlay_job().expect("kill-on-close job object must be created");
            assert!(!job.is_null());
            assert_eq!(overlay_job(), Some(job), "one job object per daemon process");
        }
    }
}

#[cfg(windows)]
pub fn find_overlay_hwnd(id: &str, pid: Option<u32>) -> Option<windows::Win32::Foundation::HWND> {
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, FindWindowW, GetWindowThreadProcessId};
    use windows::core::PCWSTR;

    let title_wide: Vec<u16> = format!("Synector Overlay [{id}]\0").encode_utf16().collect();
    let found = unsafe { FindWindowW(None, PCWSTR(title_wide.as_ptr())) };
    if let Ok(h) = found {
        if !h.is_invalid() {
            return Some(h);
        }
    }

    if let Some(target_pid) = pid {
        struct SearchContext {
            pid: u32,
            hwnd: Option<HWND>,
        }
        unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
            let ctx = &mut *(lparam.0 as *mut SearchContext);
            let mut win_pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut win_pid));
            if win_pid == ctx.pid {
                ctx.hwnd = Some(hwnd);
                return windows::core::BOOL(0);
            }
            windows::core::BOOL(1)
        }
        let mut ctx = SearchContext {
            pid: target_pid,
            hwnd: None,
        };
        unsafe {
            let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut _ as isize));
        }
        return ctx.hwnd;
    }
    None
}

pub fn hide_overlay(id: &str) {
    let mut lock = active_map().lock().unwrap();
    if let Some(mut proc) = lock.remove(id) {
        let fade_ms = proc.fade_out_ms.unwrap_or(0);
        let pid = proc.child.as_ref().map(|c| c.id());

        #[cfg(windows)]
        let signaled = if fade_ms > 0 {
            if let Some(hwnd) = find_overlay_hwnd(id, pid) {
                unsafe {
                    use windows::Win32::Foundation::{LPARAM, WPARAM};
                    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
                    let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
                true
            } else {
                false
            }
        } else {
            false
        };

        #[cfg(not(windows))]
        let signaled = false;

        if let Some(mut ch) = proc.child.take() {
            if signaled && fade_ms > 0 {
                std::thread::spawn(move || {
                    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(fade_ms as u64 + 400);
                    while std::time::Instant::now() < deadline {
                        if let Ok(Some(_)) = ch.try_wait() {
                            return;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(25));
                    }
                    let _ = ch.kill();
                });
            } else {
                let _ = ch.kill();
            }
        }
    }
}

pub fn toggle_overlay(overlay: &OverlayDefinition, override_text: Option<String>) {
    if get_active_overlay_ids().contains(&overlay.id) {
        hide_overlay(&overlay.id);
    } else {
        show_overlay(overlay, false, override_text);
    }
}

pub fn flash_overlay(overlay: &OverlayDefinition, duration_ms: u32, override_text: Option<String>) {
    let effective_text = override_text.or_else(|| get_overlay_text(&overlay.id));
    if let Some(ref txt) = effective_text {
        texts_map().lock().unwrap().insert(overlay.id.clone(), txt.clone());
        write_live_overlay_text(&overlay.id, txt);
    }
    let b64 = crate::ipc::overlay_wire::encode_overlay_b64(overlay);

    let mut lock = active_map().lock().unwrap();
    if let Some(mut existing) = lock.remove(&overlay.id) {
        if let Some(mut ch) = existing.child.take() {
            let _ = ch.kill();
        }
    }

    let child = if cfg!(test) {
        None
    } else {
        let exe = std::env::current_exe().unwrap_or_default();
        let mut cmd = Command::new(&exe);
        cmd.arg(crate::arg_spellings::OVERLAY_WINDOW).arg(&overlay.id);
        cmd.arg(crate::arg_spellings::DURATION)
            .arg(duration_ms.to_string());

        if let Some(txt) = effective_text {
            cmd.arg(crate::arg_spellings::OVERRIDE_TEXT).arg(txt);
        }
        if let Some(b64_str) = b64 {
            cmd.arg(crate::arg_spellings::OVERLAY_DATA).arg(b64_str);
        }

        spawn_overlay_process(&overlay.id, cmd)
    };

    if child.is_none() && !cfg!(test) {
        return;
    }

    lock.insert(
        overlay.id.clone(),
        ActiveOverlayProcess {
            id: overlay.id.clone(),
            child,
            close_with_target: overlay.close_with_target,
            target_process: overlay.target_process.clone().unwrap_or_default(),
            fade_out_ms: overlay.fade_out_ms,
        },
    );
}

pub fn hide_all() {
    let to_hide: Vec<String> = {
        let lock = active_map().lock().unwrap();
        lock.keys().cloned().collect()
    };
    for id in to_hide {
        hide_overlay(&id);
    }
}

pub fn hide_overlay_sync(id: &str) {
    let mut lock = active_map().lock().unwrap();
    if let Some(mut proc) = lock.remove(id) {
        let fade_ms = proc.fade_out_ms.unwrap_or(0);
        let pid = proc.child.as_ref().map(|c| c.id());

        #[cfg(windows)]
        let signaled = if fade_ms > 0 {
            if let Some(hwnd) = find_overlay_hwnd(id, pid) {
                unsafe {
                    use windows::Win32::Foundation::{LPARAM, WPARAM};
                    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
                    let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
                true
            } else {
                false
            }
        } else {
            false
        };

        #[cfg(not(windows))]
        let signaled = false;

        if let Some(mut ch) = proc.child.take() {
            if signaled && fade_ms > 0 {
                let deadline = std::time::Instant::now() + std::time::Duration::from_millis(fade_ms as u64 + 150);
                let mut exited = false;
                while std::time::Instant::now() < deadline {
                    if let Ok(Some(_)) = ch.try_wait() {
                        exited = true;
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                if !exited {
                    let _ = ch.kill();
                    let _ = ch.wait();
                }
            } else {
                let _ = ch.kill();
                let _ = ch.wait();
            }
        }
    }
}

pub fn hide_all_sync() {
    let to_hide: Vec<String> = {
        let lock = active_map().lock().unwrap();
        lock.keys().cloned().collect()
    };
    for id in to_hide {
        hide_overlay_sync(&id);
    }
}

pub fn show_update_notification(version: &str) {
    let mut overlay = OverlayDefinition::default_new_overlay();
    overlay.id = "overlay_update_notification".to_string();
    overlay.name = "Update Notification".to_string();
    overlay.anchor = crate::models::OverlayAnchor::BottomRight;
    overlay.rect = crate::models::OverlayRect {
        x: 24.0,
        y: 24.0,
        width: 355.0,
        height: 86.0,
    };
    overlay.fade_in_ms = Some(300);
    overlay.fade_out_ms = Some(400);
    overlay.auto_hide_seconds = Some(6.0);
    overlay.click_through = false;
    flash_overlay(&overlay, 6000, Some(version.to_string()));
}

pub fn show_update_progress_overlay(version: &str) {
    let mut overlay = OverlayDefinition::default_new_overlay();
    overlay.id = "overlay_update_progress".to_string();
    overlay.name = "Update Progress".to_string();
    overlay.anchor = crate::models::OverlayAnchor::BottomRight;
    overlay.rect = crate::models::OverlayRect {
        x: 24.0,
        y: 24.0,
        width: 355.0,
        height: 88.0,
    };
    overlay.fade_in_ms = Some(250);
    overlay.fade_out_ms = Some(350);
    overlay.auto_hide_seconds = None;
    overlay.click_through = true;
    show_overlay(&overlay, false, Some(version.to_string()));
}

pub fn hide_update_progress_overlay() {
    hide_overlay_sync("overlay_update_progress");
}

pub fn get_active_overlay_ids() -> HashSet<String> {
    let mut lock = active_map().lock().unwrap();
    let mut dead = Vec::new();

    for (id, proc) in lock.iter_mut() {
        if let Some(ch) = &mut proc.child {
            if let Ok(Some(_)) = ch.try_wait() {
                dead.push(id.clone());
            }
        }
    }

    for id in dead {
        lock.remove(&id);
    }

    lock.keys().cloned().collect()
}

pub fn get_calibrated_position(id: &str) -> Option<(f32, f32)> {
    calibrated_map().lock().ok()?.get(id).copied()
}

pub fn save_calibrated_position(id: &str, x: f32, y: f32) {
    if let Ok(mut lock) = calibrated_map().lock() {
        lock.insert(id.to_string(), (x, y));
    }
}



pub fn check_process_terminated(process_name: &str) {
    let clean_target = std::path::Path::new(process_name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(process_name);

    let to_close: Vec<String> = {
        let lock = active_map().lock().unwrap();
        lock.values()
            .filter(|p| {
                if !p.close_with_target {
                    return false;
                }
                let clean_p = std::path::Path::new(&p.target_process)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&p.target_process);
                clean_p.eq_ignore_ascii_case(clean_target)
            })
            .map(|p| p.id.clone())
            .collect()
    };

    for id in to_close {
        hide_overlay(&id);
    }
}

/// Effective DPI of a display device; `96` when it cannot be resolved (for
/// example on a headless session). The process manifest opts into PerMonitorV2,
/// so `GetDeviceCaps` reports the real per-monitor DPI. Cached briefly because
/// overlay windows query the monitor list every frame.
#[cfg(windows)]
fn monitor_dpi(device_name: &str) -> u32 {
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::{CreateDCW, DeleteDC, GetDeviceCaps, LOGPIXELSX};

    const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(30);
    static DPI_CACHE: OnceLock<Mutex<HashMap<String, (u32, std::time::Instant)>>> = OnceLock::new();
    let cache = DPI_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(cache) = cache.lock() {
        if let Some((dpi, sampled_at)) = cache.get(device_name) {
            if sampled_at.elapsed() < CACHE_TTL {
                return *dpi;
            }
        }
    }

    let device: Vec<u16> = device_name.encode_utf16().chain(std::iter::once(0)).collect();
    let device_ptr = if device_name.is_empty() {
        PCWSTR::null()
    } else {
        PCWSTR(device.as_ptr())
    };
    let dpi = unsafe {
        let dc = CreateDCW(PCWSTR::null(), device_ptr, PCWSTR::null(), None);
        if dc.is_invalid() {
            96
        } else {
            let dpi = GetDeviceCaps(Some(dc), LOGPIXELSX);
            let _ = DeleteDC(dc);
            u32::try_from(dpi).ok().filter(|dpi| *dpi >= 48).unwrap_or(96)
        }
    };
    if let Ok(mut cache) = cache.lock() {
        cache.insert(device_name.to_string(), (dpi, std::time::Instant::now()));
    }
    dpi
}

/// Logical scale factor of a display device (`1.0` at 96 DPI). Used by the
/// overlay window to report its position in the same logical space as
/// `MonitorInfo` and `OverlayRect`.
#[cfg(windows)]
pub(crate) fn monitor_scale(device_name: &str) -> f32 {
    monitor_dpi(device_name) as f32 / 96.0
}

/// Converts physical monitor pixels into the logical points that
/// `egui::ViewportBuilder::with_position` / `with_inner_size` expect. Without
/// this, anchors and secondary-monitor offsets are wrong above 100% scaling.
fn monitor_bounds_to_logical(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    dpi: u32,
) -> (i32, i32, u32, u32) {
    if dpi <= 96 {
        return (x, y, width, height);
    }
    let scale = f64::from(dpi) / 96.0;
    let logical_x = (f64::from(x) / scale).round() as i32;
    let logical_y = (f64::from(y) / scale).round() as i32;
    let logical_w = ((f64::from(width) / scale).round() as u32).max(1);
    let logical_h = ((f64::from(height) / scale).round() as u32).max(1);
    (logical_x, logical_y, logical_w, logical_h)
}

#[cfg(windows)]
pub fn get_system_monitors() -> Vec<MonitorInfo> {
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
    };
    use windows::Win32::Foundation::{LPARAM, RECT};
    use windows::core::BOOL;

    unsafe extern "system" fn enum_proc(
        hmonitor: HMONITOR,
        _hdc: HDC,
        _lprect: *mut RECT,
        lparam: LPARAM,
    ) -> BOOL {
        let list = &mut *(lparam.0 as *mut Vec<MonitorInfo>);
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(hmonitor, &mut info as *mut MONITORINFOEXW as *mut _).as_bool() {
            let rect = info.monitorInfo.rcMonitor;
            let is_primary = (info.monitorInfo.dwFlags & 1) != 0;
            let w = (rect.right - rect.left).max(0) as u32;
            let h = (rect.bottom - rect.top).max(0) as u32;
            let raw_name = String::from_utf16_lossy(&info.szDevice).trim_matches('\0').to_string();
            let dpi = monitor_dpi(&raw_name);
            let (logical_x, logical_y, logical_w, logical_h) =
                monitor_bounds_to_logical(rect.left, rect.top, w, h, dpi);
            let name = if raw_name.is_empty() {
                format!("Display {}", list.len() + 1)
            } else {
                raw_name
            };
            if w > 0 && h > 0 {
                list.push(MonitorInfo {
                    index: 0,
                    name,
                    x: logical_x,
                    y: logical_y,
                    width: logical_w,
                    height: logical_h,
                    is_primary,
                });
            }
        }
        BOOL(1)
    }

    let mut list: Vec<MonitorInfo> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(enum_proc),
            LPARAM(&mut list as *mut _ as isize),
        );
    }

    if list.is_empty() {
        use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
        let w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
        let h = unsafe { GetSystemMetrics(SM_CYSCREEN) };
        let (sw, sh) = if w > 0 && h > 0 { (w as u32, h as u32) } else { (1920, 1080) };
        let (_, _, logical_w, logical_h) = monitor_bounds_to_logical(0, 0, sw, sh, monitor_dpi(""));
        return vec![MonitorInfo {
            index: 0,
            name: "Primary Display".to_string(),
            x: 0,
            y: 0,
            width: logical_w,
            height: logical_h,
            is_primary: true,
        }];
    }

    list.sort_by(|a, b| {
        b.is_primary.cmp(&a.is_primary)
            .then_with(|| a.x.cmp(&b.x))
            .then_with(|| a.y.cmp(&b.y))
    });

    for (i, m) in list.iter_mut().enumerate() {
        m.index = i;
    }

    list
}

#[cfg(not(windows))]
pub fn get_system_monitors() -> Vec<MonitorInfo> {
    vec![MonitorInfo {
        index: 0,
        name: "Primary Display".to_string(),
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
        is_primary: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::OverlayAnchor;

    #[test]
    fn test_get_system_monitors_use_production_anchor_math() {
        let monitors = get_system_monitors();
        assert!(!monitors.is_empty(), "System monitors list must not be empty");
        assert!(monitors.iter().any(|m| m.is_primary), "Must include at least one primary monitor");
        assert_eq!(monitors[0].index, 0);
        assert!(monitors[0].is_primary, "First monitor must always be primary after sorting");

        let mut def = OverlayDefinition::default_new_overlay();
        def.rect.width = 200.0;
        def.rect.height = 100.0;
        def.anchor = OverlayAnchor::TopLeft;
        def.rect.x = 10.0;
        def.rect.y = 20.0;
        def.monitor_index = Some(0);

        // The production placement path is the one overlay_window uses; the old
        // cfg(test)-only copy could drift from it without failing a test.
        let monitor = &monitors[0];
        let (rel_x, rel_y) = crate::models::compute_overlay_screen_pos_with_anchor(
            def.anchor,
            &def,
            monitor.width as f32,
            monitor.height as f32,
        );
        assert_eq!(monitor.x as f32 + rel_x, monitor.x as f32 + 10.0);
        assert_eq!(monitor.y as f32 + rel_y, monitor.y as f32 + 20.0);
    }

    #[test]
    fn test_monitor_bounds_convert_physical_pixels_to_logical_points() {
        // 100% scaling is the identity.
        assert_eq!(monitor_bounds_to_logical(0, 0, 2560, 1440, 96), (0, 0, 2560, 1440));
        // 150% secondary monitor: physical (2560, 0) / 1920x1080 -> logical.
        assert_eq!(monitor_bounds_to_logical(2560, 0, 1920, 1080, 144), (1707, 0, 1280, 720));
        // 200% primary monitor.
        assert_eq!(monitor_bounds_to_logical(0, 0, 3840, 2160, 192), (0, 0, 1920, 1080));
        // Never collapse a scaled monitor to zero.
        assert_eq!(monitor_bounds_to_logical(0, 0, 1, 1, 192), (0, 0, 1, 1));
    }

    #[test]
    fn test_show_overlay_stores_fade_out_and_target_process() {
        init();

        let mut def = OverlayDefinition::default_new_overlay();
        def.id = "mgr_test_ov".to_string();
        def.target_process = Some("game.exe".to_string());
        def.close_with_target = true;
        def.fade_out_ms = Some(450);

        show_overlay(&def, false, None);

        let active_ids = get_active_overlay_ids();
        assert!(active_ids.contains("mgr_test_ov"));

        {
            let lock = active_map().lock().unwrap();
            let proc = lock.get("mgr_test_ov").unwrap();
            assert_eq!(proc.target_process, "game.exe");
            assert!(proc.close_with_target);
            assert_eq!(proc.fade_out_ms, Some(450));
        }

        hide_overlay("mgr_test_ov");
        let active_ids_after = get_active_overlay_ids();
        assert!(!active_ids_after.contains("mgr_test_ov"));
    }

    #[test]
    fn test_update_notification_and_progress_overlays() {
        init();

        show_update_notification("2.5.0");
        let active_ids = get_active_overlay_ids();
        assert!(active_ids.contains("overlay_update_notification"));

        hide_overlay("overlay_update_notification");
        assert!(!get_active_overlay_ids().contains("overlay_update_notification"));

        show_update_progress_overlay("2.5.0");
        assert!(get_active_overlay_ids().contains("overlay_update_progress"));

        hide_update_progress_overlay();
        assert!(!get_active_overlay_ids().contains("overlay_update_progress"));
    }

    #[test]
    fn test_dynamic_overlay_text_cache() {
        init();
        assert!(get_overlay_text("test_ov_cache").is_none());

        update_overlay_text("test_ov_cache", "Initial text");
        assert_eq!(get_overlay_text("test_ov_cache").as_deref(), Some("Initial text"));

        update_overlay_text("test_ov_cache", "Updated dynamic text");
        assert_eq!(get_overlay_text("test_ov_cache").as_deref(), Some("Updated dynamic text"));
    }

    #[test]
    fn test_update_overlay_text_publishes_live_text_without_restarting() {
        init();
        let id = "live_text_ov";
        let path = live_overlay_text_path(id);
        let _ = std::fs::remove_file(&path);
        assert!(live_overlay_text(id).is_none());

        update_overlay_text(id, "first");
        assert_eq!(live_overlay_text(id).as_deref(), Some("first"));
        assert_eq!(
            live_overlay_text_if_changed(id, Some("first")),
            None,
            "an unchanged label must not force a repaint"
        );
        assert_eq!(
            live_overlay_text_if_changed(id, Some("stale")).as_deref(),
            Some("first")
        );

        update_overlay_text(id, "second");
        assert_eq!(live_overlay_text(id).as_deref(), Some("second"));

        // Different overlay ids never share a live-text file.
        assert_ne!(live_overlay_text_path(id), live_overlay_text_path("live_text_ov_2"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_show_overlay_publishes_live_text_for_running_window() {
        init();
        let mut def = OverlayDefinition::default_new_overlay();
        def.id = "live_text_show_ov".to_string();
        show_overlay(&def, false, Some("shown".to_string()));
        assert_eq!(live_overlay_text(&def.id).as_deref(), Some("shown"));
        hide_overlay(&def.id);
        let _ = std::fs::remove_file(live_overlay_text_path(&def.id));
    }

    #[test]
    fn test_failed_spawn_is_reported_and_not_active() {
        init();
        // `show_overlay` short-circuits spawning under cfg(test), so exercise the
        // helper directly: a missing executable must yield None (logged) and no
        // active overlay, instead of a phantom entry.
        let command = Command::new("synector_definitely_missing_overlay_binary");
        let child = spawn_overlay_process("spawn_failure_ov", command);
        assert!(child.is_none(), "spawning a missing binary must fail");
        assert!(!get_active_overlay_ids().contains("spawn_failure_ov"));
    }
}
