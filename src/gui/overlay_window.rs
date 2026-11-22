use std::sync::Mutex;
use std::time::{Duration, Instant};
use egui::Color32;
use crate::config::{Library, Settings};
use crate::gui::components::overlay_renderer;
use crate::gui::theme;
use crate::models::{OverlayClickAction, OverlayDefinition, OverlayElement, TimerMode};

#[derive(Clone)]
struct OverlayRuntimeState {
    id: String,
    calibration: bool,
    draggable: bool,
    click_through: bool,
    has_fade_out: bool,
}

static OVERLAY_RUNTIME_STATE: Mutex<Option<OverlayRuntimeState>> = Mutex::new(None);
static LAST_MOVE_IPC: Mutex<Option<Instant>> = Mutex::new(None);
static FADEOUT_TRIGGERED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// DPI scale of the monitor under `hwnd`; `GetWindowRect` reports physical
/// pixels while `OverlayRect` and `MonitorInfo` are in logical points.
#[cfg(windows)]
fn window_dpi_scale(hwnd: windows::Win32::Foundation::HWND) -> f32 {
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
    };

    unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(monitor, &mut info as *mut MONITORINFOEXW as *mut _).as_bool() {
            let device = String::from_utf16_lossy(&info.szDevice).trim_matches('\0').to_string();
            crate::daemon::overlay_manager::monitor_scale(&device)
        } else {
            1.0
        }
    }
}

/// Latest logical position reported by `WM_MOVE` / `WM_EXITSIZEMOVE`. The window
/// procedure only records it; a background worker performs the named-pipe call,
/// because `IpcClient::call` has no timeout and a stalled daemon would otherwise
/// freeze the overlay mid-drag and block its message loop (U06-004).
static PENDING_OVERLAY_POSITION: Mutex<Option<(String, f32, f32)>> = Mutex::new(None);
static OVERLAY_POSITION_WAKE: std::sync::Condvar = std::sync::Condvar::new();

fn ensure_overlay_position_worker() {
    static STARTED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    STARTED.get_or_init(|| {
        if let Err(err) = std::thread::Builder::new()
            .name("overlay-position".into())
            .spawn(|| loop {
                let update = {
                    let mut slot = PENDING_OVERLAY_POSITION.lock().unwrap_or_else(|e| e.into_inner());
                    while slot.is_none() {
                        slot = OVERLAY_POSITION_WAKE.wait(slot).unwrap_or_else(|e| e.into_inner());
                    }
                    slot.take()
                };
                if let Some((id, x, y)) = update {
                    let _ = crate::ipc::IpcClient::call(&crate::ipc::IpcRequest::UpdateOverlayPosition { id, x, y });
                }
            })
        {
            crate::logging::error("OVERLAY", &format!("Failed to spawn overlay position worker: {err}"));
        }
    });
}

/// Queues the newest window position; intermediate `WM_MOVE` reports are coalesced.
fn queue_overlay_position(id: String, x: f32, y: f32) {
    ensure_overlay_position_worker();
    if let Ok(mut slot) = PENDING_OVERLAY_POSITION.lock() {
        *slot = Some((id, x, y));
    }
    OVERLAY_POSITION_WAKE.notify_one();
}

/// Progress values fed by the update-progress poller. The overlay only needs the two updater
/// fields, so it polls the narrow `GetUpdateProgress` query on a worker instead of pulling and
/// deserializing the whole `DaemonState` (config plus logs) on the render thread; the frame loop
/// reads these cached values (U06-013).
static UPDATE_PROGRESS_BITS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static UPDATE_STATUS_TEXT: Mutex<Option<String>> = Mutex::new(None);

fn ensure_update_progress_poller() {
    static STARTED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    STARTED.get_or_init(|| {
        if let Err(err) = std::thread::Builder::new()
            .name("overlay-update-progress".into())
            .spawn(|| loop {
                std::thread::sleep(Duration::from_millis(100));
                if let Ok(response) =
                    crate::ipc::IpcClient::call(&crate::ipc::IpcRequest::GetUpdateProgress)
                {
                    cache_update_progress(&response);
                }
            })
        {
            crate::logging::error("OVERLAY", &format!("Failed to spawn update-progress poller: {err}"));
        }
    });
}

/// Stores one poll answer in the atomics the frame loop reads. Any other response is ignored,
/// and a missing progress keeps the last value (the daemon reports progress only while a
/// download is running).
fn cache_update_progress(response: &crate::ipc::IpcResponse) {
    let crate::ipc::IpcResponse::UpdateProgress { progress, status } = response else {
        return;
    };
    if let Some(progress) = progress {
        UPDATE_PROGRESS_BITS.store(progress.to_bits(), std::sync::atomic::Ordering::Relaxed);
    }
    if let Ok(mut slot) = UPDATE_STATUS_TEXT.lock() {
        *slot = status.clone();
    }
}

fn cached_update_progress() -> f32 {
    f32::from_bits(UPDATE_PROGRESS_BITS.load(std::sync::atomic::Ordering::Relaxed))
}

fn cached_update_status() -> Option<String> {
    UPDATE_STATUS_TEXT.lock().ok().and_then(|slot| slot.clone())
}

#[cfg(windows)]
unsafe extern "system" fn overlay_subclass_proc(
    hwnd: windows::Win32::Foundation::HWND,
    umsg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
    _uid: usize,
    _ref_data: usize,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::{LRESULT, RECT};
    use windows::Win32::UI::Shell::DefSubclassProc;
    use windows::Win32::UI::WindowsAndMessaging::{
        DefWindowProcW, GetWindowRect, HTCLIENT, HTTRANSPARENT, MA_NOACTIVATE, STYLESTRUCT, WM_CLOSE,
        WM_ERASEBKGND, WM_EXITSIZEMOVE, WM_LBUTTONDBLCLK, WM_MOUSEACTIVATE, WM_MOVE, WM_NCACTIVATE,
        WM_NCCALCSIZE, WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCPAINT, WM_STYLECHANGED, WM_STYLECHANGING,
        WM_SYSCOMMAND, WS_BORDER, WS_CAPTION, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
        WS_EX_TRANSPARENT, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP, WS_THICKFRAME,
    };

    let runtime = OVERLAY_RUNTIME_STATE.lock().ok().and_then(|g| g.clone());

    match umsg {
        WM_STYLECHANGING => {
            let style_struct = &mut *(lparam.0 as *mut STYLESTRUCT);
            if wparam.0 as i32 == windows::Win32::UI::WindowsAndMessaging::GWL_STYLE.0 {
                style_struct.styleNew = (style_struct.styleNew | WS_POPUP.0)
                    & !(WS_BORDER.0 | WS_CAPTION.0 | WS_THICKFRAME.0 | WS_MAXIMIZEBOX.0 | WS_MINIMIZEBOX.0);
                return LRESULT(0);
            } else if wparam.0 as i32 == windows::Win32::UI::WindowsAndMessaging::GWL_EXSTYLE.0 {
                let mut ex = style_struct.styleNew
                    | WS_EX_LAYERED.0
                    | WS_EX_TOOLWINDOW.0
                    | WS_EX_NOACTIVATE.0;
                if let Some(ref rt) = runtime {
                    if rt.click_through && !rt.calibration {
                        ex |= WS_EX_TRANSPARENT.0;
                    } else {
                        ex &= !WS_EX_TRANSPARENT.0;
                    }
                }
                style_struct.styleNew = ex;
                return LRESULT(0);
            }
        }
        WM_STYLECHANGED => {
            return LRESULT(0);
        }
        WM_MOUSEACTIVATE => {
            // Allow mouse click to be processed without activating/focusing the window,
            // preventing the overlay from stealing focus from games or active apps!
            return LRESULT(MA_NOACTIVATE as isize);
        }
        WM_NCACTIVATE => {
            // Prevent Windows from drawing active/inactive title bar
            return LRESULT(1);
        }
        WM_ERASEBKGND => {
            // Tell Windows the background has been erased; prevents GDI from painting the default white COLOR_WINDOW brush
            return LRESULT(1);
        }
        WM_NCPAINT => {
            // Suppress standard Windows non-client frame/border painting
            return LRESULT(0);
        }
        WM_NCCALCSIZE => {
            if wparam.0 != 0 {
                // Return 0: The entire window rectangle is the client area.
                // This completely bypasses and eliminates the 1px non-client top offset and black strip!
                return LRESULT(0);
            }
        }
        WM_NCHITTEST => {
            if let Some(ref rt) = runtime {
                if rt.click_through && !rt.calibration {
                    return LRESULT(HTTRANSPARENT as isize);
                }
            }
            // Treat entire window as client area.
            // This prevents Windows DWM from intercepting clicks as non-client caption events,
            // allowing egui to receive all mouse clicks, hovers, and focus,
            // while preventing double-click from maximizing the window!
            return LRESULT(HTCLIENT as isize);
        }
        WM_NCLBUTTONDBLCLK | WM_LBUTTONDBLCLK => {
            // Completely disable double-click maximizing behavior on overlay
            return LRESULT(0);
        }
        WM_SYSCOMMAND => {
            // Prevent SC_MAXIMIZE (0xF030)
            if (wparam.0 & 0xFFF0) == 0xF030 {
                return LRESULT(0);
            }
            // Directly forward SC_MOVE / SC_DRAGMOVE (0xF010 / 0xF012) to DefWindowProcW,
            // completely bypassing winit's event loop so winit cannot cancel the modal move!
            if (wparam.0 & 0xFFF0) == 0xF010 {
                return DefWindowProcW(hwnd, umsg, wparam, lparam);
            }
        }
        WM_CLOSE => {
            let has_fade_out = runtime.as_ref().is_some_and(|rt| rt.has_fade_out);
            if has_fade_out && !FADEOUT_TRIGGERED.swap(true, std::sync::atomic::Ordering::SeqCst) {
                // Intercept close so that egui::App::ui can execute the smooth fade-out animation
                unsafe {
                    use windows::Win32::Graphics::Gdi::InvalidateRect;
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                return LRESULT(0);
            }
        }
        WM_MOVE => {
            if let Some(ref rt) = runtime {
                if (rt.calibration || rt.draggable) && !rt.id.is_empty() {
                    let should_send = {
                        let mut last = LAST_MOVE_IPC.lock().unwrap_or_else(|e| e.into_inner());
                        let now = Instant::now();
                        match *last {
                            Some(t) if now.duration_since(t) < Duration::from_millis(100) => false,
                            _ => {
                                *last = Some(now);
                                true
                            }
                        }
                    };
                    if should_send {
                        let mut rect = RECT::default();
                        if GetWindowRect(hwnd, &mut rect).is_ok() {
                            let scale = window_dpi_scale(hwnd);
                            queue_overlay_position(rt.id.clone(), rect.left as f32 / scale, rect.top as f32 / scale);
                        }
                    }
                }
            }
        }
        WM_EXITSIZEMOVE => {
            if let Some(ref rt) = runtime {
                if (rt.calibration || rt.draggable) && !rt.id.is_empty() {
                    let mut rect = RECT::default();
                    if GetWindowRect(hwnd, &mut rect).is_ok() {
                        let scale = window_dpi_scale(hwnd);
                        queue_overlay_position(rt.id.clone(), rect.left as f32 / scale, rect.top as f32 / scale);
                    }
                }
            }
        }
        _ => {}
    }

    DefSubclassProc(hwnd, umsg, wparam, lparam)
}

/// Per-window timer bookkeeping. The renderer only flips `is_running` on a click;
/// advancing the stored milliseconds from real elapsed time belongs to the frame
/// loop, and a countdown's `on_finish` action fires exactly once per completed
/// run (U06-016).
#[derive(Default)]
struct TimerRuntime {
    last_tick: Option<Instant>,
    /// Indices of countdowns whose `on_finish` already fired for the current run.
    finished: std::collections::HashSet<usize>,
}

/// Advances every running `ClockTimer` by the real frame delta and returns the
/// `on_finish` actions of countdowns that reached zero on this tick. Subtraction
/// and addition saturate, so a clock jump or a paused process can never wrap the
/// stored milliseconds.
fn advance_clock_timers(
    elements: &mut [OverlayElement],
    runtime: &mut TimerRuntime,
    now: Instant,
) -> Vec<OverlayClickAction> {
    let delta_ms = runtime.last_tick.map_or(0, |last| {
        now.saturating_duration_since(last).as_millis().min(u64::MAX as u128) as u64
    });
    runtime.last_tick = Some(now);

    let mut finished_actions = Vec::new();
    for (index, elem) in elements.iter_mut().enumerate() {
        let OverlayElement::ClockTimer {
            mode,
            is_running,
            countdown_remaining_ms,
            stopwatch_elapsed_ms,
            on_finish,
            ..
        } = elem
        else {
            continue;
        };

        // A countdown that was reset (or restarted) above zero can finish again.
        if *mode == TimerMode::Countdown && *countdown_remaining_ms > 0 {
            runtime.finished.remove(&index);
        }

        if !*is_running || delta_ms == 0 {
            continue;
        }

        match mode {
            TimerMode::Stopwatch => {
                *stopwatch_elapsed_ms = stopwatch_elapsed_ms.saturating_add(delta_ms);
            }
            TimerMode::Countdown => {
                *countdown_remaining_ms = countdown_remaining_ms.saturating_sub(delta_ms);
                if *countdown_remaining_ms == 0 {
                    *is_running = false;
                    if runtime.finished.insert(index) {
                        if let Some(action) = on_finish {
                            finished_actions.push(action.clone());
                        }
                    }
                }
            }
            TimerMode::Clock | TimerMode::Uptime => {}
        }
    }
    finished_actions
}

/// `OverlayClickAction::ResetTimer`: countdowns return to their configured
/// duration, stopwatches to zero (U06-002).
fn reset_clock_timers(elements: &mut [OverlayElement]) {
    for elem in elements.iter_mut() {
        if let OverlayElement::ClockTimer {
            mode,
            countdown_total_ms,
            countdown_remaining_ms,
            stopwatch_elapsed_ms,
            ..
        } = elem
        {
            match mode {
                TimerMode::Countdown => *countdown_remaining_ms = *countdown_total_ms,
                TimerMode::Stopwatch => *stopwatch_elapsed_ms = 0,
                TimerMode::Clock | TimerMode::Uptime => {}
            }
        }
    }
}

/// `OverlayClickAction::ToggleTimer`: flips `is_running` exactly like a direct
/// click on the timer, including re-arming a countdown that sits at zero
/// (U06-002).
fn toggle_clock_timers(elements: &mut [OverlayElement]) {
    for elem in elements.iter_mut() {
        if let OverlayElement::ClockTimer {
            mode,
            is_running,
            countdown_total_ms,
            countdown_remaining_ms,
            ..
        } = elem
        {
            match mode {
                TimerMode::Countdown => {
                    if *countdown_remaining_ms == 0 {
                        *countdown_remaining_ms = *countdown_total_ms;
                    }
                    *is_running = !*is_running;
                }
                TimerMode::Stopwatch => *is_running = !*is_running,
                TimerMode::Clock | TimerMode::Uptime => {}
            }
        }
    }
}

pub struct OverlayWindowApp {
    pub overlay: OverlayDefinition,
    pub calibration: bool,
    pub override_text: Option<String>,
    pub start_time: Instant,
    pub duration: Option<Duration>,
    pub hw_configured: bool,
    pub current_opacity: f32,
    pub fade_in_duration: Option<Duration>,
    pub fade_out_duration: Option<Duration>,
    pub fade_out_start: Option<(Instant, f32)>,
    pub is_closing: bool,
    pub hwnd: Option<isize>,
    pub last_window_track: Instant,
    /// Last time the `WindowRelative` target handle was re-resolved. The process
    /// table is scanned on a seconds-scale timer instead of on every 50 ms tick
    /// (U06-006).
    pub last_target_window_lookup: Instant,
    /// Cached `HWND` of the tracked target window (raw pointer value).
    pub target_window: Option<isize>,
    pub frame_count: u32,
    /// Throttle for the live-text channel (`ph.overlay_set_text` / `SetText`).
    pub last_text_poll: Instant,
    /// UI language resolved once at window startup: overlay windows are short-lived and
    /// must not re-read settings every frame.
    pub language: crate::i18n::Language,
    timer_runtime: TimerRuntime,
}

impl OverlayWindowApp {
    pub fn new(
        overlay: OverlayDefinition,
        calibration: bool,
        override_text: Option<String>,
        duration_ms: Option<u32>,
        ctx: &egui::Context,
    ) -> Self {
        theme::install_fonts(ctx);
        egui_extras::install_image_loaders(ctx);

        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::TRANSPARENT;
        visuals.window_fill = Color32::TRANSPARENT;
        visuals.extreme_bg_color = Color32::TRANSPARENT;
        ctx.set_visuals(visuals);

        let fade_in_duration = if !calibration {
            overlay.fade_in_ms.filter(|&ms| ms > 0).map(|ms| Duration::from_millis(ms as u64))
        } else {
            None
        };

        let fade_out_duration = if !calibration {
            overlay.fade_out_ms.filter(|&ms| ms > 0).map(|ms| Duration::from_millis(ms as u64))
        } else {
            None
        };

        let initial_opacity = if fade_in_duration.is_some() {
            0.0
        } else {
            overlay.opacity
        };

        FADEOUT_TRIGGERED.store(false, std::sync::atomic::Ordering::SeqCst);

        if let Ok(mut lock) = OVERLAY_RUNTIME_STATE.lock() {
            *lock = Some(OverlayRuntimeState {
                id: overlay.id.clone(),
                calibration,
                draggable: overlay.draggable,
                click_through: overlay.click_through,
                has_fade_out: fade_out_duration.is_some(),
            });
        }

        let duration = duration_ms
            .map(|ms| Duration::from_millis(ms as u64))
            .or_else(|| {
                overlay
                    .auto_hide_seconds
                    .filter(|&s| s > 0.0)
                    .map(Duration::from_secs_f32)
            });

        let mut app = Self {
            overlay,
            calibration,
            override_text,
            start_time: Instant::now(),
            duration,
            hw_configured: false,
            current_opacity: initial_opacity,
            fade_in_duration,
            fade_out_duration,
            fade_out_start: None,
            is_closing: false,
            hwnd: None,
            last_window_track: Instant::now(),
            last_target_window_lookup: Instant::now() - Duration::from_secs(10),
            target_window: None,
            frame_count: 0,
            last_text_poll: Instant::now() - Duration::from_secs(1),
            language: Settings::read().language,
            timer_runtime: TimerRuntime::default(),
        };

        // Start the workers up front so the window procedure never has to spawn a
        // thread from inside a message handler.
        if !calibration && app.overlay.id == "overlay_update_progress" {
            ensure_update_progress_poller();
        }
        if app.calibration || app.overlay.draggable {
            ensure_overlay_position_worker();
        }

        app.apply_win32_window_styles();

        app
    }

    /// Applies a click action returned by the renderer. Timer actions need the
    /// element list, which only the window owns; everything else is the renderer
    /// dispatcher (blocking work goes to its worker thread).
    fn apply_click_action(&mut self, action: &OverlayClickAction) {
        match action {
            OverlayClickAction::ResetTimer => reset_clock_timers(&mut self.overlay.elements),
            OverlayClickAction::ToggleTimer => toggle_clock_timers(&mut self.overlay.elements),
            other => overlay_renderer::execute_click_action(other),
        }
    }

    fn apply_win32_window_styles(&mut self) {
        #[cfg(windows)]
        unsafe {
            use windows::core::BOOL;
            use windows::Win32::Foundation::{HWND, LPARAM};
            use windows::Win32::Graphics::Dwm::{
                DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMNCRP_DISABLED, DWMWA_BORDER_COLOR,
                DWMWA_NCRENDERING_POLICY, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE,
                DWMWCP_DONOTROUND,
            };
            use windows::Win32::System::Threading::GetCurrentThreadId;
            use windows::Win32::UI::Controls::MARGINS;
            use windows::Win32::UI::Shell::SetWindowSubclass;
            use windows::Win32::UI::WindowsAndMessaging::{
                EnumThreadWindows, GetWindowLongPtrW, SetClassLongPtrW, SetLayeredWindowAttributes,
                SetWindowDisplayAffinity, SetWindowLongPtrW, SetWindowPos, GCLP_HBRBACKGROUND, GWL_EXSTYLE,
                GWL_STYLE, LWA_ALPHA, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
                WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WS_BORDER, WS_CAPTION, WS_EX_LAYERED, WS_EX_NOACTIVATE,
                WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP, WS_THICKFRAME,
            };

            struct ThreadHwndContext {
                target_id: String,
                hwnd: Option<HWND>,
            }

            unsafe extern "system" fn enum_hwnd_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
                let ctx = &mut *(lparam.0 as *mut ThreadHwndContext);
                let mut title_buf = [0u16; 128];
                let len = windows::Win32::UI::WindowsAndMessaging::GetWindowTextW(hwnd, &mut title_buf);
                if len > 0 {
                    let title = String::from_utf16_lossy(&title_buf[..len as usize]);
                    let expected = format!("[{}]", ctx.target_id);
                    if title.contains(&expected) || title.contains("Synector Overlay") {
                        ctx.hwnd = Some(hwnd);
                        return BOOL(0);
                    }
                }
                if ctx.hwnd.is_none() {
                    ctx.hwnd = Some(hwnd);
                }
                BOOL(1)
            }

            let mut context = ThreadHwndContext {
                target_id: self.overlay.id.clone(),
                hwnd: None,
            };
            let tid = GetCurrentThreadId();
            let _ = EnumThreadWindows(tid, Some(enum_hwnd_proc), LPARAM(&mut context as *mut _ as isize));

            if let Some(hwnd) = context.hwnd {
                self.hwnd = Some(hwnd.0 as isize);

                // 1. Clear window class background brush to prevent GDI from painting the default white COLOR_WINDOW brush
                let _ = SetClassLongPtrW(hwnd, GCLP_HBRBACKGROUND, 0);

                // 2. Extend DWM frame into client area to enable seamless desktop composition transparency
                let margins = MARGINS {
                    cxLeftWidth: -1,
                    cxRightWidth: -1,
                    cyTopHeight: -1,
                    cyBottomHeight: -1,
                };
                let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);

                // 3. Hide from Alt+Tab and configure Layered Window for flicker-free alpha composition
                let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                let mut clean_ex = ex | WS_EX_TOOLWINDOW.0 as isize | WS_EX_LAYERED.0 as isize | WS_EX_NOACTIVATE.0 as isize;
                if self.overlay.click_through && !self.calibration {
                    clean_ex |= WS_EX_TRANSPARENT.0 as isize;
                } else {
                    clean_ex &= !(WS_EX_TRANSPARENT.0 as isize);
                }
                let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, clean_ex);

                // Start with window alpha = 0 (100% transparent) so no white uninitialized frame is ever visible
                let _ = SetLayeredWindowAttributes(hwnd, windows::Win32::Foundation::COLORREF(0), 0, LWA_ALPHA);

                // 4. Remove standard window borders, captions, and sizing borders
                let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
                let clean_style = (style | WS_POPUP.0 as isize)
                    & !(WS_BORDER.0 as isize | WS_CAPTION.0 as isize | WS_THICKFRAME.0 as isize | WS_MAXIMIZEBOX.0 as isize | WS_MINIMIZEBOX.0 as isize);
                let _ = SetWindowLongPtrW(hwnd, GWL_STYLE, clean_style);

                // 5. Disable DWM active accent borders and corner rounding (Windows 11 / 10)
                let color_none: u32 = 0xFFFFFFFE;
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_BORDER_COLOR,
                    &color_none as *const _ as *const _,
                    std::mem::size_of::<u32>() as u32,
                );

                let corner_pref = DWMWCP_DONOTROUND.0 as u32;
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_WINDOW_CORNER_PREFERENCE,
                    &corner_pref as *const _ as *const _,
                    std::mem::size_of::<u32>() as u32,
                );

                // 6. Disable DWM non-client rendering (prevents unwanted DWM non-client shadows and margins)
                let policy = DWMNCRP_DISABLED.0 as u32;
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_NCRENDERING_POLICY,
                    &policy as *const _ as *const _,
                    std::mem::size_of::<u32>() as u32,
                );

                // 7. Enable immersive dark mode so fallback composition is black rather than white
                let dark_mode = BOOL(1);
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_USE_IMMERSIVE_DARK_MODE,
                    &dark_mode as *const _ as *const _,
                    std::mem::size_of::<BOOL>() as u32,
                );

                // 8. Install window subclass to intercept WM_NCCALCSIZE (eliminating 1px top line) and handle dragging
                let _ = SetWindowSubclass(hwnd, Some(overlay_subclass_proc), 1001, 0);

                // 9. Notify Windows that the window frame has changed (keep hidden, do NOT pass SWP_SHOWWINDOW)
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED | SWP_NOACTIVATE,
                );

                // 7. Privacy screen capture affinity
                let affinity = if self.overlay.hide_from_capture {
                    WDA_EXCLUDEFROMCAPTURE
                } else {
                    WDA_NONE
                };
                let _ = SetWindowDisplayAffinity(hwnd, affinity);
                self.hw_configured = true;
            }
        }
    }
}

/// True while a cached window handle still designates a window the overlay can
/// follow: it exists, is visible and is not minimized.
#[cfg(windows)]
fn is_trackable_window(hwnd: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindow, IsWindowVisible};

    unsafe {
        IsWindow(Some(hwnd)).as_bool()
            && IsWindowVisible(hwnd).as_bool()
            && !IsIconic(hwnd).as_bool()
    }
}

/// Physical screen rectangle `(x, y, width, height)` of a live window.
#[cfg(windows)]
fn physical_window_rect(hwnd: windows::Win32::Foundation::HWND) -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }.ok()?;
    let w = rect.right - rect.left;
    let h = rect.bottom - rect.top;
    (w > 50 && h > 50).then_some((rect.left, rect.top, w, h))
}

/// Resolves the window that the `WindowRelative` anchor follows. This is the only
/// path that enumerates the process table; callers cache the handle because the
/// full scan must not run every 50 ms over a game (U06-006).
#[cfg(windows)]
pub fn resolve_target_window(target_process: Option<&str>) -> Option<windows::Win32::Foundation::HWND> {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    };

    unsafe {
        let fg_hwnd = GetForegroundWindow();
        if !fg_hwnd.is_invalid() && IsWindowVisible(fg_hwnd).as_bool() && !IsIconic(fg_hwnd).as_bool() {
            if let Some(target) = target_process {
                let mut pid = 0u32;
                GetWindowThreadProcessId(fg_hwnd, Some(&mut pid));
                let snap = crate::daemon::watcher::ProcessWatcher::snapshot_process_map();
                let clean_target = std::path::Path::new(target)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(target);
                if snap.get(&pid).is_some_and(|name| name.eq_ignore_ascii_case(clean_target)) {
                    let mut rect = RECT::default();
                    if GetWindowRect(fg_hwnd, &mut rect).is_ok() {
                        let w = rect.right - rect.left;
                        let h = rect.bottom - rect.top;
                        if w > 50 && h > 50 {
                            return Some(fg_hwnd);
                        }
                    }
                }
            } else {
                let mut rect = RECT::default();
                if GetWindowRect(fg_hwnd, &mut rect).is_ok() {
                    let w = rect.right - rect.left;
                    let h = rect.bottom - rect.top;
                    if w > 50 && h > 50 {
                        return Some(fg_hwnd);
                    }
                }
            }
        }

        if let Some(target) = target_process {
            let snap = crate::daemon::watcher::ProcessWatcher::snapshot_process_map();
            let clean_target = std::path::Path::new(target)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(target);

            let matching_pids: Vec<u32> = snap
                .iter()
                .filter(|(_, name)| name.eq_ignore_ascii_case(clean_target))
                .map(|(&pid, _)| pid)
                .collect();

            if matching_pids.is_empty() {
                return None;
            }

            struct SearchCtx<'a> {
                pids: &'a [u32],
                hwnd: Option<HWND>,
            }

            unsafe extern "system" fn enum_wnd(hwnd: HWND, lparam: LPARAM) -> BOOL {
                let ctx = &mut *(lparam.0 as *mut SearchCtx);
                if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
                    return BOOL(1);
                }
                let mut pid = 0u32;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                if ctx.pids.contains(&pid) {
                    let mut r = RECT::default();
                    if GetWindowRect(hwnd, &mut r).is_ok() {
                        let w = r.right - r.left;
                        let h = r.bottom - r.top;
                        if w > 100 && h > 100 {
                            ctx.hwnd = Some(hwnd);
                            return BOOL(0);
                        }
                    }
                }
                BOOL(1)
            }

            let mut search = SearchCtx {
                pids: &matching_pids,
                hwnd: None,
            };
            let _ = EnumWindows(Some(enum_wnd), LPARAM(&mut search as *mut _ as isize));
            return search.hwnd;
        }

        None
    }
}

#[inline]
fn smootherstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

impl eframe::App for OverlayWindowApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Advance running timers from the real frame delta before drawing; the renderer
        // only flips `is_running` / resets values on a click (U06-016).
        let elapsed_actions = advance_clock_timers(&mut self.overlay.elements, &mut self.timer_runtime, Instant::now());
        for action in elapsed_actions {
            self.apply_click_action(&action);
        }

        // Live text updates are published to a per-overlay file by the daemon, so
        // a `SetText` action swaps the label instead of restarting this process.
        if !self.calibration && self.last_text_poll.elapsed() >= Duration::from_millis(150) {
            self.last_text_poll = Instant::now();
            if let Some(text) = crate::daemon::overlay_manager::live_overlay_text_if_changed(
                &self.overlay.id,
                self.override_text.as_deref(),
            ) {
                self.override_text = Some(text);
                ctx.request_repaint();
            }
        }

        if !self.hw_configured {
            self.apply_win32_window_styles();
        }

        // Ensure WS_EX_LAYERED is never stripped by winit when visibility changes or frames render
        #[cfg(windows)]
        if let Some(hwnd_val) = self.hwnd {
            unsafe {
                use windows::Win32::Foundation::HWND;
                use windows::Win32::UI::WindowsAndMessaging::{
                    GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_LAYERED, WS_EX_NOACTIVATE,
                    WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
                };
                let hwnd = HWND(hwnd_val as _);
                let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                if (ex & WS_EX_LAYERED.0 as isize) == 0 {
                    let mut clean_ex = ex
                        | WS_EX_LAYERED.0 as isize
                        | WS_EX_TOOLWINDOW.0 as isize
                        | WS_EX_NOACTIVATE.0 as isize;
                    if self.overlay.click_through && !self.calibration {
                        clean_ex |= WS_EX_TRANSPARENT.0 as isize;
                    } else {
                        clean_ex &= !(WS_EX_TRANSPARENT.0 as isize);
                    }
                    let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, clean_ex);
                }
            }
        }

        self.frame_count += 1;
        if self.frame_count == 1 {
            // First frame: let GPU pipeline render into backbuffer while window is still 100% hidden.
            ctx.request_repaint();
        } else if self.frame_count == 2 {
            // Second frame: backbuffer is ready and presented to DWM.
            // Reveal the window flicker-free without any uninitialized white background or caption artifacts!
            #[cfg(windows)]
            if let Some(hwnd_val) = self.hwnd {
                unsafe {
                    use windows::Win32::Foundation::{COLORREF, HWND};
                    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SetLayeredWindowAttributes, SW_SHOWNOACTIVATE, LWA_ALPHA};
                    if self.fade_in_duration.is_some() {
                        self.start_time = Instant::now();
                    }
                    let initial_alpha = if self.fade_in_duration.is_some() {
                        0
                    } else {
                        (self.current_opacity * 255.0).round().clamp(0.0, 255.0) as u8
                    };
                    let _ = SetLayeredWindowAttributes(HWND(hwnd_val as _), COLORREF(0), initial_alpha, LWA_ALPHA);
                    let _ = ShowWindow(HWND(hwnd_val as _), SW_SHOWNOACTIVATE);
                }
            }
        }

        // 1. External close/fadeout triggered (via WM_CLOSE to hwnd)
        if !self.is_closing && FADEOUT_TRIGGERED.load(std::sync::atomic::Ordering::SeqCst) {
            self.is_closing = true;
            if self.fade_out_duration.is_some() {
                self.fade_out_start = Some((Instant::now(), self.current_opacity));
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
        }

        // 2. Check auto-hide / flash duration timer
        if !self.is_closing {
            if let Some(dur) = self.duration {
                if self.start_time.elapsed() >= dur {
                    self.is_closing = true;
                    if self.fade_out_duration.is_some() {
                        self.fade_out_start = Some((Instant::now(), self.current_opacity));
                        FADEOUT_TRIGGERED.store(true, std::sync::atomic::Ordering::SeqCst);
                    } else {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        return;
                    }
                }
            }
        }

        // 3. Compute opacity with smooth Ken Perlin smootherstep easing
        if let Some((fo_time, start_op)) = self.fade_out_start {
            if let Some(fo_dur) = self.fade_out_duration {
                let elapsed = fo_time.elapsed();
                if elapsed < fo_dur {
                    let t = (elapsed.as_secs_f32() / fo_dur.as_secs_f32().max(0.001)).clamp(0.0, 1.0);
                    let smooth = smootherstep(t);
                    let ease = 1.0 - smooth;
                    self.current_opacity = (start_op * ease).clamp(0.0, 1.0);
                    ctx.request_repaint();
                } else {
                    self.current_opacity = 0.0;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    std::process::exit(0);
                }
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                std::process::exit(0);
            }
        } else if let Some(fi_dur) = self.fade_in_duration {
            if self.frame_count >= 2 {
                let elapsed = self.start_time.elapsed();
                if elapsed < fi_dur {
                    let t = (elapsed.as_secs_f32() / fi_dur.as_secs_f32().max(0.001)).clamp(0.0, 1.0);
                    let smooth = smootherstep(t);
                    self.current_opacity = (self.overlay.opacity * smooth).clamp(0.0, 1.0);
                    ctx.request_repaint();
                } else {
                    self.current_opacity = self.overlay.opacity;
                }
            }
        } else {
            self.current_opacity = self.overlay.opacity;
        }

        // Synchronize hardware OS compositor alpha on every frame
        #[cfg(windows)]
        if self.frame_count >= 2 {
            if let Some(hwnd_val) = self.hwnd {
                unsafe {
                    use windows::Win32::Foundation::{COLORREF, HWND};
                    use windows::Win32::UI::WindowsAndMessaging::{SetLayeredWindowAttributes, LWA_ALPHA};
                    let win_alpha = (self.current_opacity * 255.0).round().clamp(0.0, 255.0) as u8;
                    let _ = SetLayeredWindowAttributes(HWND(hwnd_val as _), COLORREF(0), win_alpha, LWA_ALPHA);
                }
            }
        }

        // 4. Track WindowRelative target window. The process table is scanned only when
        // the cached handle goes stale; the per-tick work is a GetWindowRect on a live
        // handle instead of two full Toolhelp snapshots every 50 ms (U06-006). The
        // target rect is physical pixels, while element geometry is logical points, so
        // the target monitor's DPI converts between the two (U11-005).
        #[cfg(windows)]
        if self.overlay.anchor == crate::models::OverlayAnchor::WindowRelative
            && !self.calibration
            && !self.overlay.draggable
            && self.last_window_track.elapsed() >= Duration::from_millis(50)
        {
            use windows::Win32::Foundation::HWND;
            self.last_window_track = Instant::now();
            let cached_handle = self.target_window.map(|value| HWND(value as _));
            let needs_lookup = cached_handle.is_none_or(|hwnd| !is_trackable_window(hwnd))
                || self.last_target_window_lookup.elapsed() >= Duration::from_secs(2);
            if needs_lookup {
                self.target_window = resolve_target_window(self.overlay.target_process.as_deref())
                    .map(|hwnd| hwnd.0 as isize);
                self.last_target_window_lookup = Instant::now();
            }
            if let Some(target_hwnd) = self.target_window.map(|value| HWND(value as _)) {
                if let Some((wx, wy, ww, _)) = physical_window_rect(target_hwnd) {
                    let scale = window_dpi_scale(target_hwnd);
                    let logical_w = self.overlay.rect.width + self.overlay.rect.x;
                    let target_x = (wx as f32 + ww as f32 - logical_w * scale).round() as i32;
                    let target_y = (wy as f32 + self.overlay.rect.y * scale).round() as i32;
                    if let Some(hwnd_val) = self.hwnd {
                        unsafe {
                            use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER};
                            let _ = SetWindowPos(
                                HWND(hwnd_val as _),
                                None,
                                target_x,
                                target_y,
                                0,
                                0,
                                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                            );
                        }
                    }
                }
            }
        }

        let card_rect = ui.max_rect();
        let painter = ui.painter();

        #[cfg(windows)]
        let egui_render_opacity = 1.0;
        #[cfg(not(windows))]
        let egui_render_opacity = self.current_opacity;

        if self.overlay.id == "overlay_update_notification" {
            let version = self.override_text.as_deref().unwrap_or("latest");
            let loc = crate::i18n::get(self.language);

            let is_hovered = ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| card_rect.contains(p));
            if is_hovered {
                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if is_hovered && ctx.input(|i| i.pointer.primary_clicked()) {
                let _ = crate::ipc::IpcClient::call(&crate::ipc::IpcRequest::TriggerUpdate);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }

            let bg_color = Color32::from_rgba_premultiplied(16, 17, 23, 245);
            let border_color = if is_hovered {
                Color32::from_rgba_premultiplied(0, 215, 120, 160)
            } else {
                Color32::from_white_alpha(35)
            };
            painter.rect(
                card_rect,
                8.0,
                bg_color,
                egui::Stroke::new(if is_hovered { 1.5 } else { 1.0 }, border_color),
                egui::StrokeKind::Inside,
            );

            // Left accent badge with download icon on high-contrast white card
            let badge_rect = egui::Rect::from_min_size(
                card_rect.min + egui::vec2(16.0, 18.0),
                egui::vec2(48.0, 48.0),
            );
            let badge_bg = Color32::from_rgb(245, 248, 252);
            let badge_border = Color32::from_rgb(220, 228, 238);
            painter.rect(badge_rect, 10.0, badge_bg, egui::Stroke::new(1.0, badge_border), egui::StrokeKind::Inside);

            let icon_text = egui_phosphor::regular::ARROW_FAT_LINES_DOWN;
            let icon_color = Color32::from_rgb(0, 150, 75);
            painter.text(
                badge_rect.center(),
                egui::Align2::CENTER_CENTER,
                icon_text,
                egui::FontId::new(26.0, crate::gui::theme::phosphor()),
                icon_color,
            );

            let text_x = card_rect.min.x + 78.0;

            // Header row: "Synector Update"
            painter.text(
                egui::pos2(text_x, card_rect.min.y + 16.0),
                egui::Align2::LEFT_TOP,
                loc.update_notification_title,
                egui::FontId::proportional(13.5),
                Color32::WHITE,
            );

            // Version text: "Version v{} is available"
            let ver_text = loc.update_notification_available.replace("{}", version);
            painter.text(
                egui::pos2(text_x, card_rect.min.y + 36.0),
                egui::Align2::LEFT_TOP,
                &ver_text,
                egui::FontId::proportional(12.0),
                Color32::from_rgb(0, 215, 120),
            );

            // Hint text: "Click here or use tray to install"
            painter.text(
                egui::pos2(text_x, card_rect.min.y + 55.0),
                egui::Align2::LEFT_TOP,
                loc.update_notification_hint,
                egui::FontId::proportional(10.5),
                Color32::from_white_alpha(150),
            );

            ctx.request_repaint_after(Duration::from_millis(30));
            return;
        }

        if self.overlay.id == "overlay_update_progress" {
            // The standalone overlay fast path never probes single-instance
            // mutexes; lifetime is owned by the daemon that spawned it (child
            // process plus the kill-on-close job object).
            let version = self.override_text.as_deref().unwrap_or("latest");
            let loc = crate::i18n::get(self.language);

            // The worker polls the daemon and publishes the two fields this card
            // needs; the frame loop never deserializes the full DaemonState (U06-013).
            let progress = cached_update_progress();
            let cached_status = cached_update_status();

            let bg_color = Color32::from_rgba_premultiplied(16, 17, 23, 245);
            let border_color = Color32::from_white_alpha(40);
            painter.rect(
                card_rect,
                8.0,
                bg_color,
                egui::Stroke::new(1.0, border_color),
                egui::StrokeKind::Inside,
            );

            // Left accent badge on high-contrast white card
            let badge_rect = egui::Rect::from_min_size(
                card_rect.min + egui::vec2(16.0, 20.0),
                egui::vec2(48.0, 48.0),
            );
            let badge_bg = Color32::from_rgb(245, 248, 252);
            let badge_border = Color32::from_rgb(220, 228, 238);
            painter.rect(badge_rect, 10.0, badge_bg, egui::Stroke::new(1.0, badge_border), egui::StrokeKind::Inside);

            let icon_text = egui_phosphor::regular::CLOUD_ARROW_DOWN;
            let icon_color = Color32::from_rgb(20, 115, 225);
            painter.text(
                badge_rect.center(),
                egui::Align2::CENTER_CENTER,
                icon_text,
                egui::FontId::new(26.0, crate::gui::theme::phosphor()),
                icon_color,
            );

            let text_x = card_rect.min.x + 78.0;

            // Header row: "Synector Update" and Percentage
            painter.text(
                egui::pos2(text_x, card_rect.min.y + 17.0),
                egui::Align2::LEFT_TOP,
                loc.update_notification_title,
                egui::FontId::proportional(13.5),
                Color32::WHITE,
            );

            let pct_str = format!("{:.0}%", (progress * 100.0).clamp(0.0, 100.0));
            painter.text(
                egui::pos2(card_rect.max.x - 16.0, card_rect.min.y + 17.0),
                egui::Align2::RIGHT_TOP,
                &pct_str,
                egui::FontId::monospace(12.5),
                Color32::from_rgb(70, 160, 255),
            );

            // Status line
            let display_status = if progress >= 1.0 {
                loc.update_progress_applying.to_string()
            } else if let Some(ref s) = cached_status {
                s.clone()
            } else {
                loc.update_progress_downloading.replace("{}", version)
            };

            painter.text(
                egui::pos2(text_x, card_rect.min.y + 38.0),
                egui::Align2::LEFT_TOP,
                &display_status,
                egui::FontId::proportional(11.5),
                Color32::from_white_alpha(180),
            );

            // Progress bar track at bottom
            let bar_rect = egui::Rect::from_min_max(
                egui::pos2(text_x, card_rect.min.y + 61.0),
                egui::pos2(card_rect.max.x - 16.0, card_rect.min.y + 67.0),
            );
            painter.rect_filled(bar_rect, 3.0, Color32::from_white_alpha(20));

            let fill_w = bar_rect.width() * progress.clamp(0.0, 1.0);
            if fill_w > 0.0 {
                let fill_rect = egui::Rect::from_min_max(
                    bar_rect.min,
                    egui::pos2(bar_rect.min.x + fill_w, bar_rect.max.y),
                );
                let fill_color = if progress >= 1.0 {
                    Color32::from_rgb(0, 210, 120)
                } else {
                    Color32::from_rgb(60, 150, 255)
                };
                painter.rect_filled(fill_rect, 3.0, fill_color);
            }

            ctx.request_repaint_after(Duration::from_millis(30));
            return;
        }

        // Card background
        overlay_renderer::render_overlay_card_bg(
            painter,
            card_rect,
            &self.overlay,
            1.0,
            egui_render_opacity,
        );

        let pointer_pos = ctx.input(|i| i.pointer.hover_pos());
        let pointer_clicked = ctx.input(|i| i.pointer.primary_clicked());
        let pointer_down = ctx.input(|i| i.pointer.primary_down());

        let is_over_interactive = if let Some(pos) = pointer_pos {
            self.overlay.elements.iter().any(|elem| {
                let is_interactive = match elem {
                    crate::models::OverlayElement::InteractiveControl { .. } => true,
                    crate::models::OverlayElement::MediaWidget { .. } => true,
                    crate::models::OverlayElement::ClockTimer { mode, .. } => {
                        *mode == crate::models::TimerMode::Stopwatch || *mode == crate::models::TimerMode::Countdown
                    }
                    _ => false,
                };
                let has_click_action = matches!(
                    elem,
                    crate::models::OverlayElement::Text { on_click: Some(_), .. }
                        | crate::models::OverlayElement::Image { on_click: Some(_), .. }
                        | crate::models::OverlayElement::Shape { on_click: Some(_), .. }
                        | crate::models::OverlayElement::SystemTelemetry { on_click: Some(_), .. }
                );
                if !is_interactive && !has_click_action {
                    return false;
                }
                let (raw_w, raw_h) = overlay_renderer::get_element_size(elem);
                let center = card_rect.min + egui::vec2(elem.x() + raw_w * 0.5, elem.y() + raw_h * 0.5);
                let angle_rad = elem.rotation().to_radians();
                let local_pos = if angle_rad.abs() > 0.001 {
                    center + overlay_renderer::rotate_vec(pos - center, -angle_rad)
                } else {
                    pos
                };
                let elem_rect = egui::Rect::from_center_size(center, egui::vec2(raw_w, raw_h));
                elem_rect.expand(4.0).contains(local_pos)
            })
        } else {
            false
        };

        if is_over_interactive {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        let is_draggable_mode = (self.calibration || self.overlay.draggable) && !self.is_closing;
        if is_draggable_mode && !is_over_interactive {
            if pointer_down {
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            } else {
                ctx.set_cursor_icon(egui::CursorIcon::Grab);
            }

            if pointer_down {
                #[cfg(windows)]
                if let Some(hwnd_val) = self.hwnd {
                    unsafe {
                        use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
                        use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
                        use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_SYSCOMMAND};
                        let _ = ReleaseCapture();
                        let _ = SendMessageW(HWND(hwnd_val as _), WM_SYSCOMMAND, Some(WPARAM(0xF012)), Some(LPARAM(0)));
                    }
                }
            }
        }

        let mut pending_actions = Vec::new();
        {
            let mut render_ctx = overlay_renderer::RenderContext {
                painter,
                card_canvas_rect: card_rect,
                zoom: 1.0,
                global_opacity: egui_render_opacity,
                is_interactive: (!self.overlay.click_through || self.calibration) && !self.is_closing,
                pointer_pos,
                pointer_clicked,
                pointer_down,
                override_text: self.override_text.as_deref(),
                target_process: self.overlay.target_process.as_deref(),
            };

            for elem in &mut self.overlay.elements {
                if let Some(action) = overlay_renderer::render_element(&mut render_ctx, elem) {
                    pending_actions.push(action);
                }
            }
        }

        // Click actions run after the element pass so timer actions can reach the real
        // `ClockTimer` state and blocking work is queued instead of stalling the frame.
        for action in pending_actions {
            self.apply_click_action(&action);
        }

        // Request repaint every 30ms for smooth 60fps animations (equalizer, key visualizer glow, graphs)
        ctx.request_repaint_after(Duration::from_millis(30));
    }
}

pub fn run_overlay_window(
    overlay_id: String,
    calibration: bool,
    override_text: Option<String>,
    duration_ms: Option<u32>,
    overlay_data_b64: Option<String>,
) -> eframe::Result<()> {
    // 1. Resolve overlay definition
    let overlay: OverlayDefinition = if let Some(b64) = overlay_data_b64 {
        crate::ipc::overlay_wire::decode_overlay_b64(&b64)
            .unwrap_or_else(OverlayDefinition::default_new_overlay)
    } else {
        let library = Library::shared();
        library
            .overlays
            .iter()
            .find(|o| o.id == overlay_id)
            .cloned()
            .unwrap_or_else(OverlayDefinition::default_new_overlay)
    };

    // 2. Compute screen position
    let monitors = crate::daemon::overlay_manager::get_system_monitors();
    let target_mon = if let Some(idx) = overlay.monitor_index {
        monitors.iter().find(|m| m.index == idx).or_else(|| monitors.iter().find(|m| m.is_primary))
    } else {
        monitors.iter().find(|m| m.is_primary)
    };

    let (screen_w, screen_h, mon_x, mon_y) = if let Some(m) = target_mon {
        (m.width as f32, m.height as f32, m.x as f32, m.y as f32)
    } else {
        (1920.0, 1080.0, 0.0, 0.0)
    };

    let (rel_x, rel_y) = crate::models::compute_overlay_screen_pos_with_anchor(
        overlay.anchor,
        &overlay,
        screen_w,
        screen_h,
    );

    let (abs_x, abs_y) = if overlay.anchor == crate::models::OverlayAnchor::WindowRelative && !calibration {
        #[cfg(windows)]
        {
            // `GetWindowRect` reports physical pixels while `with_position` expects
            // logical points, so the target monitor's DPI scale converts them (U11-005).
            resolve_target_window(overlay.target_process.as_deref())
                .and_then(|hwnd| {
                    physical_window_rect(hwnd).map(|(wx, wy, ww, _)| {
                        let scale = window_dpi_scale(hwnd);
                        (
                            (wx as f32 + ww as f32) / scale - overlay.rect.width - overlay.rect.x,
                            wy as f32 / scale + overlay.rect.y,
                        )
                    })
                })
                .unwrap_or((mon_x + rel_x, mon_y + rel_y))
        }
        #[cfg(not(windows))]
        (mon_x + rel_x, mon_y + rel_y)
    } else {
        (mon_x + rel_x, mon_y + rel_y)
    };

    let title_str = format!("Synector Overlay [{}]", overlay.id);

    let mut viewport = egui::ViewportBuilder::default()
        .with_title(title_str)
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_has_shadow(false)
        .with_taskbar(false)
        .with_resizable(false)
        .with_maximize_button(false)
        .with_visible(false)
        .with_inner_size([overlay.rect.width, overlay.rect.height])
        .with_position([abs_x, abs_y]);

    if overlay.click_through && !calibration {
        viewport = viewport.with_mouse_passthrough(true);
    }

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Synector_Overlay",
        native_options,
        Box::new(move |cc| {
            Ok(Box::new(OverlayWindowApp::new(
                overlay,
                calibration,
                override_text,
                duration_ms,
                &cc.egui_ctx,
            )))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static TEST_OVERLAY_LOCK: Mutex<()> = Mutex::new(());

    /// U06-013: the frame loop reads cached values fed by the narrow `GetUpdateProgress` answer.
    #[test]
    fn test_update_progress_poller_caches_the_narrow_answer() {
        let _lock = TEST_OVERLAY_LOCK.lock().unwrap();

        cache_update_progress(&crate::ipc::IpcResponse::UpdateProgress {
            progress: Some(0.37),
            status: Some("37%".to_string()),
        });
        assert!((cached_update_progress() - 0.37).abs() < f32::EPSILON);
        assert_eq!(cached_update_status().as_deref(), Some("37%"));

        // A missing progress keeps the last value; the status still updates.
        cache_update_progress(&crate::ipc::IpcResponse::UpdateProgress {
            progress: None,
            status: Some("done".to_string()),
        });
        assert!((cached_update_progress() - 0.37).abs() < f32::EPSILON);
        assert_eq!(cached_update_status().as_deref(), Some("done"));

        // A response that is not the narrow answer is ignored.
        cache_update_progress(&crate::ipc::IpcResponse::Pong);
        assert!((cached_update_progress() - 0.37).abs() < f32::EPSILON);
        assert_eq!(cached_update_status().as_deref(), Some("done"));
    }

    /// The poller itself must use the narrow query; `GetState` would pull the whole snapshot.
    #[test]
    fn test_update_progress_poller_uses_the_narrow_request() {
        let source = include_str!("overlay_window.rs");
        let production = source
            .find(concat!("#[cfg(", "test)]"))
            .map(|tests_start| &source[..tests_start])
            .unwrap_or(source);
        assert!(
            production.contains("GetUpdateProgress"),
            "the update-progress poller must use the narrow query"
        );
        assert!(
            !production.contains("GetState"),
            "the overlay fast path must not pull the full daemon state"
        );
    }

    #[test]
    fn test_overlay_window_fade_in_and_fade_out_initialization() {
        let _lock = TEST_OVERLAY_LOCK.lock().unwrap();
        let ctx = egui::Context::default();

        // 1. Overlay with fade in
        let mut def_fade = OverlayDefinition::default_new_overlay();
        def_fade.opacity = 0.85;
        def_fade.fade_in_ms = Some(300);
        def_fade.fade_out_ms = Some(400);

        let app_fade = OverlayWindowApp::new(def_fade, false, None, None, &ctx);
        assert_eq!(app_fade.current_opacity, 0.0, "Overlay with fade_in must start at 0.0 opacity");
        assert_eq!(app_fade.fade_in_duration, Some(Duration::from_millis(300)));
        assert_eq!(app_fade.fade_out_duration, Some(Duration::from_millis(400)));

        // 2. Overlay without fade in
        let mut def_instant = OverlayDefinition::default_new_overlay();
        def_instant.opacity = 0.75;
        def_instant.fade_in_ms = None;
        def_instant.fade_out_ms = None;

        let app_instant = OverlayWindowApp::new(def_instant, false, None, None, &ctx);
        assert_eq!(app_instant.current_opacity, 0.75, "Overlay without fade_in must start at defined opacity");
        assert_eq!(app_instant.fade_in_duration, None);
        assert_eq!(app_instant.fade_out_duration, None);

        // 3. Calibration mode must bypass fade in
        let mut def_calib = OverlayDefinition::default_new_overlay();
        def_calib.opacity = 0.9;
        def_calib.fade_in_ms = Some(500);

        let app_calib = OverlayWindowApp::new(def_calib, true, None, None, &ctx);
        assert_eq!(app_calib.current_opacity, 0.9, "Calibration mode must bypass fade in");
        assert_eq!(app_calib.fade_in_duration, None);
    }

    #[test]
    fn test_smootherstep_bounds_and_symmetry() {
        assert_eq!(smootherstep(0.0), 0.0);
        assert_eq!(smootherstep(0.5), 0.5);
        assert_eq!(smootherstep(1.0), 1.0);
        assert_eq!(smootherstep(-0.5), 0.0);
        assert_eq!(smootherstep(1.5), 1.0);
        // Test symmetry: f(x) + f(1 - x) == 1.0
        let s1 = smootherstep(0.25);
        let s2 = smootherstep(0.75);
        assert!((s1 + s2 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_overlay_window_draggable_runtime_state() {
        let _lock = TEST_OVERLAY_LOCK.lock().unwrap();
        let ctx = egui::Context::default();
        let mut def = OverlayDefinition::default_new_overlay();
        def.draggable = true;
        def.click_through = false;

        let _app = OverlayWindowApp::new(def, false, None, None, &ctx);
        let rt = OVERLAY_RUNTIME_STATE.lock().unwrap().clone().expect("Runtime state set");
        assert!(rt.draggable, "Runtime state must reflect overlay.draggable");
        assert!(!rt.click_through, "Click-through must be false when draggable is enabled");
    }

    #[test]
    fn test_overlay_window_auto_hide_resolution() {
        let _lock = TEST_OVERLAY_LOCK.lock().unwrap();
        let ctx = egui::Context::default();

        // Priority 1: CLI duration_ms
        let mut def1 = OverlayDefinition::default_new_overlay();
        def1.auto_hide_seconds = Some(10.0);
        let app1 = OverlayWindowApp::new(def1, false, None, Some(2500), &ctx);
        assert_eq!(app1.duration, Some(Duration::from_millis(2500)));

        // Priority 2: overlay.auto_hide_seconds fallback
        let mut def2 = OverlayDefinition::default_new_overlay();
        def2.auto_hide_seconds = Some(4.5);
        let app2 = OverlayWindowApp::new(def2, false, None, None, &ctx);
        assert_eq!(app2.duration, Some(Duration::from_millis(4500)));

        // Neither: indefinite
        let mut def3 = OverlayDefinition::default_new_overlay();
        def3.auto_hide_seconds = None;
        let app3 = OverlayWindowApp::new(def3, false, None, None, &ctx);
        assert_eq!(app3.duration, None);
    }

    fn timer_element(
        mode: TimerMode,
        is_running: bool,
        total_ms: u64,
        remaining_ms: u64,
        elapsed_ms: u64,
        on_finish: Option<OverlayClickAction>,
    ) -> OverlayElement {
        OverlayElement::ClockTimer {
            id: "timer_test".to_string(),
            mode,
            format_str: "%M:%S".to_string(),
            countdown_total_ms: total_ms,
            countdown_remaining_ms: remaining_ms,
            stopwatch_elapsed_ms: elapsed_ms,
            is_running,
            font_size: 14.0,
            color: [255, 255, 255, 255],
            bg_color: [0, 0, 0, 0],
            bold: false,
            font_family: "Inter".to_string(),
            x: 0.0,
            y: 0.0,
            width: 120.0,
            height: 30.0,
            rotation: 0.0,
            on_finish,
        }
    }

    fn countdown_ms(elem: &OverlayElement) -> u64 {
        match elem {
            OverlayElement::ClockTimer { countdown_remaining_ms, .. } => *countdown_remaining_ms,
            _ => panic!("expected ClockTimer"),
        }
    }

    fn stopwatch_ms(elem: &OverlayElement) -> u64 {
        match elem {
            OverlayElement::ClockTimer { stopwatch_elapsed_ms, .. } => *stopwatch_elapsed_ms,
            _ => panic!("expected ClockTimer"),
        }
    }

    fn timer_running(elem: &OverlayElement) -> bool {
        match elem {
            OverlayElement::ClockTimer { is_running, .. } => *is_running,
            _ => panic!("expected ClockTimer"),
        }
    }

    #[test]
    fn test_clock_timer_stopwatch_advances_from_frame_delta_and_pauses() {
        let start = Instant::now();
        let mut elements = vec![timer_element(TimerMode::Stopwatch, true, 0, 0, 1_000, None)];
        let mut runtime = TimerRuntime::default();

        // The first tick only establishes the anchor.
        assert!(advance_clock_timers(&mut elements, &mut runtime, start).is_empty());
        assert_eq!(stopwatch_ms(&elements[0]), 1_000);

        assert!(advance_clock_timers(&mut elements, &mut runtime, start + Duration::from_millis(250)).is_empty());
        assert_eq!(stopwatch_ms(&elements[0]), 1_250);

        if let OverlayElement::ClockTimer { is_running, .. } = &mut elements[0] {
            *is_running = false;
        }
        let paused_at = start + Duration::from_millis(700);
        assert!(advance_clock_timers(&mut elements, &mut runtime, paused_at).is_empty());
        assert_eq!(stopwatch_ms(&elements[0]), 1_250);
    }

    #[test]
    fn test_clock_timer_stopwatch_saturates_instead_of_overflowing() {
        let start = Instant::now();
        let mut elements = vec![timer_element(TimerMode::Stopwatch, true, 0, 0, u64::MAX, None)];
        let mut runtime = TimerRuntime::default();
        advance_clock_timers(&mut elements, &mut runtime, start);
        advance_clock_timers(&mut elements, &mut runtime, start + Duration::from_secs(86_400));
        assert_eq!(stopwatch_ms(&elements[0]), u64::MAX);
    }

    #[test]
    fn test_countdown_finish_action_fires_exactly_once_and_rearms_on_reset() {
        let start = Instant::now();
        let mut elements = vec![timer_element(
            TimerMode::Countdown,
            true,
            2_000,
            100,
            0,
            Some(OverlayClickAction::CloseOverlay),
        )];
        let mut runtime = TimerRuntime::default();

        advance_clock_timers(&mut elements, &mut runtime, start);
        let actions = advance_clock_timers(&mut elements, &mut runtime, start + Duration::from_millis(150));
        assert_eq!(actions.len(), 1, "countdown reaching zero must execute on_finish once");
        assert!(matches!(actions[0], OverlayClickAction::CloseOverlay));
        assert_eq!(countdown_ms(&elements[0]), 0);
        assert!(!timer_running(&elements[0]), "a finished countdown stops");

        // Further ticks must not repeat the finish action.
        assert!(advance_clock_timers(&mut elements, &mut runtime, start + Duration::from_millis(600)).is_empty());

        // Reset re-arms it; starting again finishes a second time.
        reset_clock_timers(&mut elements);
        assert_eq!(countdown_ms(&elements[0]), 2_000);
        toggle_clock_timers(&mut elements);
        assert!(timer_running(&elements[0]));
        advance_clock_timers(&mut elements, &mut runtime, start + Duration::from_millis(700));
        let second = advance_clock_timers(&mut elements, &mut runtime, start + Duration::from_millis(2_800));
        assert_eq!(second.len(), 1, "a reset countdown must be able to finish again");
        assert_eq!(countdown_ms(&elements[0]), 0);
    }

    #[test]
    fn test_timer_click_actions_reset_and_toggle_elements() {
        let mut elements = vec![
            timer_element(TimerMode::Countdown, false, 5_000, 4_200, 0, None),
            timer_element(TimerMode::Stopwatch, false, 0, 0, 900, None),
        ];

        reset_clock_timers(&mut elements);
        assert_eq!(countdown_ms(&elements[0]), 5_000);
        assert_eq!(stopwatch_ms(&elements[1]), 0);

        toggle_clock_timers(&mut elements);
        assert!(timer_running(&elements[0]));
        assert!(timer_running(&elements[1]));

        // Toggling a countdown that sits at zero re-arms it, exactly like a direct click.
        if let OverlayElement::ClockTimer { countdown_remaining_ms, .. } = &mut elements[0] {
            *countdown_remaining_ms = 0;
        }
        toggle_clock_timers(&mut elements);
        assert!(!timer_running(&elements[0]));
        assert_eq!(countdown_ms(&elements[0]), 5_000);
    }

    #[test]
    fn test_overlay_window_applies_timer_click_actions() {
        let _lock = TEST_OVERLAY_LOCK.lock().unwrap();
        let ctx = egui::Context::default();
        let mut def = OverlayDefinition::default_new_overlay();
        def.elements = vec![timer_element(TimerMode::Countdown, false, 3_000, 1_000, 0, None)];

        let mut app = OverlayWindowApp::new(def, false, None, None, &ctx);
        app.apply_click_action(&OverlayClickAction::ResetTimer);
        assert_eq!(countdown_ms(&app.overlay.elements[0]), 3_000);

        app.apply_click_action(&OverlayClickAction::ToggleTimer);
        assert!(timer_running(&app.overlay.elements[0]));
    }

    /// The standalone `--overlay-window` fast path must never probe single-instance mutexes
    /// (U06-001). The needles are built with `concat!` so this test cannot match itself.
    #[test]
    fn test_fast_path_never_probes_single_instance_state() {
        let source = include_str!("overlay_window.rs");
        let mut production = source;
        if let Some(tests_start) = source.find(concat!("#[cfg(", "test)]")) {
            production = &source[..tests_start];
        }
        for needle in [
            concat!("single_", "instance::"),
            concat!("is_daemon_", "running("),
            concat!("is_gui_", "running("),
        ] {
            assert!(
                !production.contains(needle),
                "overlay window fast path must not reference {needle}"
            );
        }
    }
}
