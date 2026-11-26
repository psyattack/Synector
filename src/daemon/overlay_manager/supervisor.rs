use std::collections::{HashMap, HashSet};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::models::OverlayDefinition;

#[cfg(windows)]
use super::job;
use super::text::{get_overlay_text, texts_map, write_live_overlay_text};

pub struct ActiveOverlayProcess {
    pub id: String,
    pub child: Option<Child>,
    pub close_with_target: bool,
    pub target_process: String,
    pub fade_out_ms: Option<u32>,
    /// Monotonic show id. A run binds the generation it showed, so a later show by another run is
    /// never hidden by the first run's teardown.
    pub generation: u64,
}

/// Generations distinguish individual shows of the same overlay id. Zero is never a generation.
static NEXT_OVERLAY_GENERATION: AtomicU64 = AtomicU64::new(1);

fn next_overlay_generation() -> u64 {
    NEXT_OVERLAY_GENERATION.fetch_add(1, Ordering::Relaxed).max(1)
}

static ACTIVE_PROCESSES: OnceLock<Arc<Mutex<HashMap<String, ActiveOverlayProcess>>>> = OnceLock::new();
type CalibratedPositionsMap = Arc<Mutex<HashMap<String, (f32, f32)>>>;
static CALIBRATED_POSITIONS: OnceLock<CalibratedPositionsMap> = OnceLock::new();

pub(super) fn active_map() -> &'static Arc<Mutex<HashMap<String, ActiveOverlayProcess>>> {
    ACTIVE_PROCESSES.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

fn calibrated_map() -> &'static Arc<Mutex<HashMap<String, (f32, f32)>>> {
    CALIBRATED_POSITIONS.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

pub fn init() {
    let _ = active_map();
    let _ = calibrated_map();
    let _ = texts_map();
}

pub fn show_overlay(overlay: &OverlayDefinition, calibration: bool, override_text: Option<String>) -> Option<u64> {
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
        return None;
    }

    let generation = next_overlay_generation();
    lock.insert(
        overlay.id.clone(),
        ActiveOverlayProcess {
            id: overlay.id.clone(),
            child,
            close_with_target: overlay.close_with_target,
            target_process: overlay.target_process.clone().unwrap_or_default(),
            fade_out_ms: overlay.fade_out_ms,
            generation,
        },
    );
    Some(generation)
}

/// Spawns an overlay window and logs a failure with the overlay id. Spawn errors
/// used to be swallowed to `None`, leaving a permanently "active" overlay that
/// nothing could see.
pub(super) fn spawn_overlay_process(overlay_id: &str, command: Command) -> Option<Child> {
    let mut command = command;
    #[cfg(windows)]
    {
        crate::win_args::apply_no_window(&mut command);
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

fn take_active_overlay(id: &str, expected_generation: Option<u64>) -> Option<ActiveOverlayProcess> {
    let mut lock = active_map().lock().unwrap();
    if let Some(expected) = expected_generation {
        if lock.get(id).is_none_or(|proc| proc.generation != expected) {
            return None;
        }
    }
    lock.remove(id)
}

fn stop_active_overlay(mut proc: ActiveOverlayProcess) {
    let id = proc.id.as_str();
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

pub fn hide_overlay(id: &str) {
    if let Some(proc) = take_active_overlay(id, None) {
        stop_active_overlay(proc);
    }
}

/// Hides the overlay only while it is still the show that `generation` identified.
pub fn hide_overlay_if_generation(id: &str, generation: u64) -> bool {
    match take_active_overlay(id, Some(generation)) {
        Some(proc) => {
            stop_active_overlay(proc);
            true
        }
        None => false,
    }
}

/// Current show generation of an active overlay; `None` when it is not on screen.
pub fn overlay_generation(id: &str) -> Option<u64> {
    active_map().lock().ok()?.get(id).map(|proc| proc.generation)
}

pub fn toggle_overlay(overlay: &OverlayDefinition, override_text: Option<String>) -> Option<u64> {
    if get_active_overlay_ids().contains(&overlay.id) {
        hide_overlay(&overlay.id);
        None
    } else {
        show_overlay(overlay, false, override_text)
    }
}

pub fn flash_overlay(overlay: &OverlayDefinition, duration_ms: u32, override_text: Option<String>) -> Option<u64> {
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
        return None;
    }

    let generation = next_overlay_generation();
    lock.insert(
        overlay.id.clone(),
        ActiveOverlayProcess {
            id: overlay.id.clone(),
            child,
            close_with_target: overlay.close_with_target,
            target_process: overlay.target_process.clone().unwrap_or_default(),
            fade_out_ms: overlay.fade_out_ms,
            generation,
        },
    );
    Some(generation)
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
