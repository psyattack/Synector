#[cfg(windows)]
use std::collections::HashMap;
#[cfg(windows)]
use std::sync::{Mutex, OnceLock};

use crate::models::MonitorInfo;

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
pub(super) fn monitor_bounds_to_logical(
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
