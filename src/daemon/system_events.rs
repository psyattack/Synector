#![allow(clippy::missing_transmute_annotations)]
use crate::models::SystemEventType;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

#[cfg(windows)]
use windows::core::{w, GUID};
#[cfg(windows)]
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
#[cfg(windows)]
use windows::Win32::System::Power::{
    GetSystemPowerStatus, RegisterPowerSettingNotification, POWERBROADCAST_SETTING,
    SYSTEM_POWER_STATUS,
};
#[cfg(windows)]
use windows::Win32::System::SystemInformation::GetTickCount;
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostQuitMessage,
    RegisterClassW, RegisterDeviceNotificationW, TranslateMessage, DBT_DEVICEARRIVAL,
    DBT_DEVICEREMOVECOMPLETE, DBT_DEVTYP_DEVICEINTERFACE, DEVICE_NOTIFY_ALL_INTERFACE_CLASSES,
    DEVICE_NOTIFY_WINDOW_HANDLE, DEV_BROADCAST_DEVICEINTERFACE_W, DEV_BROADCAST_HDR, MSG,
    PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMECRITICAL, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND,
    PBT_POWERSETTINGCHANGE, WINDOW_EX_STYLE, WINDOW_STYLE, WM_DESTROY, WM_DEVICECHANGE,
    WM_POWERBROADCAST, WNDCLASSW,
};

/// One event produced by the notification window or by the polling watcher.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemEventNotification {
    pub event_type: SystemEventType,
    /// Device interface path for plug-and-play events, when Windows provides one.
    pub device_id: Option<String>,
}

impl SystemEventNotification {
    fn new(event_type: SystemEventType) -> Self {
        Self {
            event_type,
            device_id: None,
        }
    }
}

/// Queue filled by the notification window thread and drained by the daemon loop.
static NOTIFICATION_QUEUE: Mutex<Vec<SystemEventNotification>> = Mutex::new(Vec::new());
static NOTIFICATION_WAKE: OnceLock<Arc<tokio::sync::Notify>> = OnceLock::new();
static NOTIFICATIONS_STARTED: OnceLock<()> = OnceLock::new();

/// True between PBT_APMSUSPEND and the matching resume notification.
static SYSTEM_SLEEPING: AtomicBool = AtomicBool::new(false);
/// True while Windows reports the console display as off (not merely dimmed).
static DISPLAY_OFF: AtomicBool = AtomicBool::new(false);
/// Set after the first plug-and-play notification so conditions can distinguish
/// "no device is connected" from "we have not observed any device yet".
static DEVICE_STATE_KNOWN: AtomicBool = AtomicBool::new(false);
/// Device interface paths that are currently connected.
static CONNECTED_DEVICES: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn notification_wake() -> &'static Arc<tokio::sync::Notify> {
    NOTIFICATION_WAKE.get_or_init(|| Arc::new(tokio::sync::Notify::new()))
}

/// Handle the daemon loop can wait on for immediate notification delivery.
pub fn notification_handle() -> Arc<tokio::sync::Notify> {
    ensure_notifications_started();
    Arc::clone(notification_wake())
}

fn ensure_notifications_started() {
    NOTIFICATIONS_STARTED.get_or_init(|| {
        #[cfg(windows)]
        {
            let _ = notification_wake();
            if let Err(err) = std::thread::Builder::new()
                .name("system-event-notifications".into())
                .spawn(run_notification_window)
            {
                crate::logging::warn(
                    "SYSTEM",
                    &format!("Failed to start the system event notification thread: {err}"),
                );
            }
        }
    });
}

fn push_notification(event_type: SystemEventType, device_id: Option<String>) {
    if let Ok(mut queue) = NOTIFICATION_QUEUE.lock() {
        if queue.len() < 256 {
            queue.push(SystemEventNotification {
                event_type,
                device_id,
            });
        }
    }
    notification_wake().notify_one();
}

pub struct SystemEventsWatcher {
    last_idle_state: bool,
    idle_threshold_ms: u32,
    last_power_is_ac: Option<bool>,
    last_network_is_connected: Option<bool>,
    last_session_locked: Option<bool>,
}

impl SystemEventsWatcher {
    pub fn new() -> Self {
        ensure_notifications_started();
        Self {
            last_idle_state: false,
            idle_threshold_ms: 300_000, // default 5 minutes
            last_power_is_ac: None,
            last_network_is_connected: None,
            last_session_locked: None,
        }
    }
}

impl Default for SystemEventsWatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemEventsWatcher {
    pub fn check_events(&mut self) -> Vec<SystemEventNotification> {
        ensure_notifications_started();
        let mut events = Vec::new();

        // Events delivered by the notification window since the previous drain.
        if let Ok(mut queue) = NOTIFICATION_QUEUE.lock() {
            events.append(&mut queue);
        }

        // 1. Check Idle / Active
        #[cfg(windows)]
        unsafe {
            let mut lii = LASTINPUTINFO {
                cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
                dwTime: 0,
            };
            if GetLastInputInfo(&mut lii).as_bool() {
                let current_tick = GetTickCount();
                let idle_ms = current_tick.saturating_sub(lii.dwTime);

                let is_idle_now = idle_ms >= self.idle_threshold_ms;
                if is_idle_now != self.last_idle_state {
                    self.last_idle_state = is_idle_now;
                    if is_idle_now {
                        events.push(SystemEventNotification::new(SystemEventType::UserIdle));
                    } else {
                        events.push(SystemEventNotification::new(SystemEventType::UserActive));
                    }
                }
            }

            // 2. Check AC / Battery power source
            let mut sps: SYSTEM_POWER_STATUS = std::mem::zeroed();
            if GetSystemPowerStatus(&mut sps).is_ok() {
                let is_ac = sps.ACLineStatus == 1;
                match self.last_power_is_ac {
                    Some(prev) if prev != is_ac => {
                        self.last_power_is_ac = Some(is_ac);
                        if is_ac {
                            events
                                .push(SystemEventNotification::new(SystemEventType::PowerSourceAc));
                        } else {
                            events.push(SystemEventNotification::new(
                                SystemEventType::PowerSourceBattery,
                            ));
                        }
                    }
                    None => {
                        self.last_power_is_ac = Some(is_ac);
                    }
                    _ => {}
                }
            }

            // 3. Check Network connection state
            let is_net = is_network_connected();
            match self.last_network_is_connected {
                Some(prev) if prev != is_net => {
                    self.last_network_is_connected = Some(is_net);
                    if is_net {
                        events.push(SystemEventNotification::new(
                            SystemEventType::NetworkConnected,
                        ));
                    } else {
                        events.push(SystemEventNotification::new(
                            SystemEventType::NetworkDisconnected,
                        ));
                    }
                }
                None => {
                    self.last_network_is_connected = Some(is_net);
                }
                _ => {}
            }

            // 4. Check Session Lock / Unlock
            let is_locked = is_session_locked();
            match self.last_session_locked {
                Some(prev) if prev != is_locked => {
                    self.last_session_locked = Some(is_locked);
                    if is_locked {
                        events.push(SystemEventNotification::new(SystemEventType::SessionLock));
                    } else {
                        events.push(SystemEventNotification::new(SystemEventType::SessionUnlock));
                    }
                }
                None => {
                    self.last_session_locked = Some(is_locked);
                }
                _ => {}
            }
        }

        events
    }
}

// ---------------------------------------------------------------------------
// Notification window
// ---------------------------------------------------------------------------

#[cfg(windows)]
const CONSOLE_DISPLAY_STATE_GUID: GUID = GUID::from_u128(0x6fe69556_704a_47a0_8f24_c28d936fda47);

#[cfg(windows)]
fn run_notification_window() {
    unsafe {
        let class_name = w!("SynectorSystemEventNotifications");
        let window_class = WNDCLASSW {
            lpfnWndProc: Some(system_event_wndproc),
            lpszClassName: class_name,
            ..Default::default()
        };
        if RegisterClassW(&window_class) == 0 {
            crate::logging::warn(
                "SYSTEM",
                "Failed to register the system event notification window class",
            );
            return;
        }

        let hwnd = match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            w!("Synector System Events"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            None,
            None,
        ) {
            Ok(hwnd) => hwnd,
            Err(err) => {
                crate::logging::warn(
                    "SYSTEM",
                    &format!("Failed to create the system event notification window: {err}"),
                );
                return;
            }
        };

        let handle = HANDLE(hwnd.0);
        if let Err(err) = RegisterPowerSettingNotification(
            handle,
            &CONSOLE_DISPLAY_STATE_GUID,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        ) {
            crate::logging::warn(
                "SYSTEM",
                &format!("Failed to subscribe to display power notifications: {err}"),
            );
        }

        let device_filter = DEV_BROADCAST_DEVICEINTERFACE_W {
            dbcc_size: std::mem::size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32,
            dbcc_devicetype: DBT_DEVTYP_DEVICEINTERFACE.0,
            dbcc_reserved: 0,
            dbcc_classguid: GUID::from_u128(0),
            dbcc_name: [0],
        };
        if let Err(err) = RegisterDeviceNotificationW(
            handle,
            &device_filter as *const _ as *const std::ffi::c_void,
            DEVICE_NOTIFY_WINDOW_HANDLE | DEVICE_NOTIFY_ALL_INTERFACE_CLASSES,
        ) {
            crate::logging::warn(
                "SYSTEM",
                &format!("Failed to subscribe to device notifications: {err}"),
            );
        }

        crate::logging::debug(
            "SYSTEM",
            "System event notification window is active (sleep, devices, display state)",
        );

        let mut msg = MSG::default();
        loop {
            let result = GetMessageW(&mut msg, None, 0, 0);
            if result.0 <= 0 {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

#[cfg(windows)]
unsafe extern "system" fn system_event_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_POWERBROADCAST => {
            match wparam.0 as u32 {
                PBT_APMSUSPEND => update_system_sleeping(true),
                PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND | PBT_APMRESUMECRITICAL => {
                    update_system_sleeping(false)
                }
                PBT_POWERSETTINGCHANGE => update_display_state(lparam),
                _ => {}
            }
            return LRESULT(1);
        }
        WM_DEVICECHANGE => {
            update_device_state(wparam.0 as u32, lparam);
            return LRESULT(1);
        }
        WM_DESTROY => PostQuitMessage(0),
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

#[cfg(windows)]
fn update_system_sleeping(sleeping: bool) {
    let was_sleeping = SYSTEM_SLEEPING.swap(sleeping, Ordering::SeqCst);
    if was_sleeping != sleeping {
        push_notification(
            if sleeping {
                SystemEventType::SystemSleep
            } else {
                SystemEventType::SystemResume
            },
            None,
        );
    }
}

#[cfg(windows)]
unsafe fn update_display_state(lparam: LPARAM) {
    if lparam.0 == 0 {
        return;
    }
    let setting = &*(lparam.0 as *const POWERBROADCAST_SETTING);
    if setting.PowerSetting != CONSOLE_DISPLAY_STATE_GUID || setting.DataLength == 0 {
        return;
    }
    let state = if setting.DataLength >= 4 {
        std::ptr::read_unaligned(setting.Data.as_ptr() as *const u32)
    } else {
        setting.Data[0] as u32
    };
    // 0 = off, 1 = on, 2 = dimmed. Dimmed still means the display is powered.
    let off = state == 0;
    let was_off = DISPLAY_OFF.swap(off, Ordering::SeqCst);
    if was_off != off {
        push_notification(
            if off {
                SystemEventType::MonitorOff
            } else {
                SystemEventType::MonitorOn
            },
            None,
        );
    }
}

#[cfg(windows)]
unsafe fn update_device_state(wparam: u32, lparam: LPARAM) {
    let connected = match wparam {
        DBT_DEVICEARRIVAL => true,
        DBT_DEVICEREMOVECOMPLETE => false,
        _ => return,
    };

    let device_id = device_interface_path(lparam);
    DEVICE_STATE_KNOWN.store(true, Ordering::SeqCst);

    if let Some(device_id) = &device_id {
        if let Ok(mut devices) = CONNECTED_DEVICES.lock() {
            if connected {
                // Composite devices can report the same interface twice.
                if devices
                    .iter()
                    .any(|known| known.eq_ignore_ascii_case(device_id))
                {
                    return;
                }
                if devices.len() < 256 {
                    devices.push(device_id.clone());
                }
            } else {
                devices.retain(|known| !known.eq_ignore_ascii_case(device_id));
            }
        }
    }

    push_notification(
        if connected {
            SystemEventType::DeviceConnected
        } else {
            SystemEventType::DeviceDisconnected
        },
        device_id,
    );
}

#[cfg(windows)]
unsafe fn device_interface_path(lparam: LPARAM) -> Option<String> {
    if lparam.0 == 0 {
        return None;
    }
    let header = &*(lparam.0 as *const DEV_BROADCAST_HDR);
    if header.dbch_devicetype != DBT_DEVTYP_DEVICEINTERFACE {
        return None;
    }
    let interface = &*(lparam.0 as *const DEV_BROADCAST_DEVICEINTERFACE_W);
    let name_offset = std::mem::offset_of!(DEV_BROADCAST_DEVICEINTERFACE_W, dbcc_name);
    let total_bytes = interface.dbcc_size as usize;
    if total_bytes <= name_offset {
        return None;
    }
    let units = (total_bytes - name_offset) / std::mem::size_of::<u16>();
    let mut length = 0;
    while length < units && *interface.dbcc_name.as_ptr().add(length) != 0 {
        length += 1;
    }
    if length == 0 {
        return None;
    }
    let name = String::from_utf16_lossy(std::slice::from_raw_parts(
        interface.dbcc_name.as_ptr(),
        length,
    ));
    Some(name)
}

// ---------------------------------------------------------------------------
// State queries used by rule conditions
// ---------------------------------------------------------------------------

pub fn is_system_sleeping() -> bool {
    SYSTEM_SLEEPING.load(Ordering::SeqCst)
}

/// Windows reports the display as on until it explicitly says it is off.
pub fn is_display_on() -> bool {
    !DISPLAY_OFF.load(Ordering::SeqCst)
}

pub fn device_state_known() -> bool {
    DEVICE_STATE_KNOWN.load(Ordering::SeqCst)
}

pub fn has_connected_device() -> bool {
    CONNECTED_DEVICES
        .lock()
        .map(|devices| !devices.is_empty())
        .unwrap_or(false)
}

pub fn is_network_connected() -> bool {
    #[cfg(windows)]
    {
        type FnInternetGetConnectedState = unsafe extern "system" fn(*mut u32, u32) -> i32;
        static FUNC: std::sync::LazyLock<Option<FnInternetGetConnectedState>> =
            std::sync::LazyLock::new(|| {
                use windows::core::s;
                use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};
                unsafe {
                    if let Ok(h) = LoadLibraryA(s!("wininet.dll")) {
                        if let Some(proc) = GetProcAddress(h, s!("InternetGetConnectedState")) {
                            return Some(std::mem::transmute::<_, FnInternetGetConnectedState>(
                                proc,
                            ));
                        }
                    }
                    None
                }
            });

        if let Some(func) = *FUNC {
            let mut flags = 0u32;
            return unsafe { func(&mut flags, 0) != 0 };
        }
    }
    true
}

pub fn get_user_idle_seconds() -> Option<u64> {
    #[cfg(windows)]
    unsafe {
        let mut lii = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        if GetLastInputInfo(&mut lii).as_bool() {
            let current_tick = GetTickCount();
            let idle_ms = current_tick.saturating_sub(lii.dwTime);
            return Some((idle_ms / 1000) as u64);
        }
    }
    None
}

pub fn is_ac_power() -> Option<bool> {
    #[cfg(windows)]
    unsafe {
        let mut sps: SYSTEM_POWER_STATUS = std::mem::zeroed();
        if GetSystemPowerStatus(&mut sps).is_ok() {
            return Some(sps.ACLineStatus == 1);
        }
    }
    None
}

pub fn is_session_locked() -> bool {
    #[cfg(windows)]
    {
        type FnOpenInputDesktop =
            unsafe extern "system" fn(u32, i32, u32) -> windows::Win32::Foundation::HANDLE;
        type FnCloseDesktop = unsafe extern "system" fn(windows::Win32::Foundation::HANDLE) -> i32;

        static DESKTOP_FNS: std::sync::LazyLock<Option<(FnOpenInputDesktop, FnCloseDesktop)>> =
            std::sync::LazyLock::new(|| {
                use windows::core::s;
                use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};
                unsafe {
                    if let Ok(h) = LoadLibraryA(s!("user32.dll")) {
                        if let (Some(open_p), Some(close_p)) = (
                            GetProcAddress(h, s!("OpenInputDesktop")),
                            GetProcAddress(h, s!("CloseDesktop")),
                        ) {
                            return Some((
                                std::mem::transmute::<_, FnOpenInputDesktop>(open_p),
                                std::mem::transmute::<_, FnCloseDesktop>(close_p),
                            ));
                        }
                    }
                    None
                }
            });

        if let Some((open_desk, close_desk)) = *DESKTOP_FNS {
            unsafe {
                let hdesk = open_desk(0, 0, 0x0001); // 0x0001 = DESKTOP_READOBJECTS
                if !hdesk.is_invalid() && !hdesk.0.is_null() {
                    let _ = close_desk(hdesk);
                    false
                } else {
                    true
                }
            }
        } else {
            use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
            let hwnd = unsafe { GetForegroundWindow() };
            hwnd.0.is_null()
        }
    }
    #[cfg(not(windows))]
    {
        false
    }
}
