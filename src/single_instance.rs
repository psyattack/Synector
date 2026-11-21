use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::{CreateMutexW, OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
use windows::Win32::UI::WindowsAndMessaging::FindWindowW;

const GUI_MUTEX_NAME: &str = "Local\\Synector_GUI_SingleInstance";
const DAEMON_MUTEX_NAME: &str = "Local\\Synector_Daemon_SingleInstance";
const DAEMON_SPAWN_MUTEX_NAME: &str = "Local\\Synector_DaemonSpawn";

pub struct SingleInstanceGuard {
    handle: HANDLE,
}

impl SingleInstanceGuard {
    fn acquire_named(name: &str) -> Option<Self> {
        unsafe {
            let hstring = HSTRING::from(name);
            let handle = CreateMutexW(None, true, PCWSTR(hstring.as_ptr())).ok()?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(handle);
                return None;
            }
            Some(Self { handle })
        }
    }

    /// Mutex probing stays private to the guard: a held single-instance mutex is not a reliable
    /// daemon-liveness signal (it exists before the pipe is served and lingers during shutdown).
    /// External callers ask the daemon over IPC instead.
    fn named_mutex_exists(name: &str) -> bool {
        unsafe {
            let hstring = HSTRING::from(name);
            match OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, PCWSTR(hstring.as_ptr())) {
                Ok(handle) => {
                    let _ = CloseHandle(handle);
                    true
                }
                Err(_) => false,
            }
        }
    }

    pub fn acquire_gui() -> Option<Self> {
        Self::acquire_named(GUI_MUTEX_NAME)
    }

    pub fn acquire_daemon() -> Option<Self> {
        Self::acquire_named(DAEMON_MUTEX_NAME)
    }

    /// Short-lived guard that serializes daemon start-up (check + prepare + spawn) across
    /// processes, so concurrent launches cannot both start a daemon.
    pub fn acquire_daemon_spawn() -> Option<Self> {
        Self::acquire_named(DAEMON_SPAWN_MUTEX_NAME)
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

/// Checks whether the Synector GUI process is currently running.
pub fn is_gui_running() -> bool {
    if SingleInstanceGuard::named_mutex_exists(GUI_MUTEX_NAME) {
        return true;
    }
    unsafe {
        if let Ok(hwnd) = FindWindowW(None, w!("Synector")) {
            if !hwnd.0.is_null() {
                return true;
            }
        }
    }
    false
}

/// Unconditionally forces the given window to the foreground, bypassing Windows focus-stealing restrictions.
pub fn force_foreground_window(hwnd: windows::Win32::Foundation::HWND) {
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{
            BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId,
            SetForegroundWindow, ShowWindow, SW_RESTORE,
        };
        use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
        use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};

        let fg_hwnd = GetForegroundWindow();
        if fg_hwnd == hwnd {
            return;
        }

        let fg_thread = GetWindowThreadProcessId(fg_hwnd, None);
        let cur_thread = GetCurrentThreadId();

        if fg_thread != 0 && fg_thread != cur_thread {
            let _ = AttachThreadInput(cur_thread, fg_thread, true);
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = BringWindowToTop(hwnd);
            let _ = SetForegroundWindow(hwnd);
            let _ = SetFocus(Some(hwnd));
            let _ = AttachThreadInput(cur_thread, fg_thread, false);
        } else {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = BringWindowToTop(hwnd);
            let _ = SetForegroundWindow(hwnd);
            let _ = SetFocus(Some(hwnd));
        }
    }
}

/// Finds the existing Synector GUI window and restores / brings it to the foreground.
/// Returns `true` if the window was found and focused, `false` otherwise.
pub fn focus_existing_gui_window() -> bool {
    for _ in 0..5 {
        unsafe {
            if let Ok(hwnd) = FindWindowW(None, w!("Synector")) {
                if !hwnd.0.is_null() {
                    force_foreground_window(hwnd);
                    return true;
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    false
}

/// Closes the existing Synector GUI window by posting WM_CLOSE.
/// Returns `true` if the window was found and signal posted, `false` otherwise.
pub fn close_existing_gui_window() -> bool {
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
        if let Ok(hwnd) = FindWindowW(None, w!("Synector")) {
            if !hwnd.0.is_null() {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, windows::Win32::Foundation::WPARAM(0), windows::Win32::Foundation::LPARAM(0));
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_instance_named_mutex() {
        let test_name = format!("Local\\Synector_Test_{}", std::process::id());
        let guard1 = SingleInstanceGuard::acquire_named(&test_name);
        assert!(guard1.is_some(), "First acquire should succeed");

        let guard2 = SingleInstanceGuard::acquire_named(&test_name);
        assert!(guard2.is_none(), "Second acquire should fail while first guard is held");

        drop(guard1);

        let guard3 = SingleInstanceGuard::acquire_named(&test_name);
        assert!(guard3.is_some(), "Acquire should succeed again after previous guard was dropped");
    }

    #[test]
    fn test_is_gui_running_with_guard() {
        if let Some(guard) = SingleInstanceGuard::acquire_gui() {
            assert!(is_gui_running(), "is_gui_running must be true while GUI single instance guard is held");
            drop(guard);
        }
    }
}
