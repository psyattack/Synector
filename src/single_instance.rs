#[cfg(windows)]
use windows::core::w;
#[cfg(windows)]
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
#[cfg(windows)]
use windows::Win32::System::Threading::CreateMutexW;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::FindWindowW;

pub struct SingleInstanceGuard {
    #[cfg(windows)]
    handle: HANDLE,
}

impl SingleInstanceGuard {
    #[cfg(windows)]
    pub fn acquire_named(name: &str) -> Option<Self> {
        use windows::core::HSTRING;
        unsafe {
            let hstring = HSTRING::from(name);
            let handle = CreateMutexW(None, true, windows::core::PCWSTR(hstring.as_ptr())).ok()?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(handle);
                return None;
            }
            Some(Self { handle })
        }
    }

    pub fn acquire_gui() -> Option<Self> {
        #[cfg(windows)]
        return Self::acquire_named("Local\\Synector_GUI_SingleInstance");
        #[cfg(not(windows))]
        Some(Self {})
    }

    pub fn acquire_daemon() -> Option<Self> {
        #[cfg(windows)]
        return Self::acquire_named("Local\\Synector_Daemon_SingleInstance");
        #[cfg(not(windows))]
        Some(Self {})
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

/// Checks whether the Synector GUI process is currently running.
pub fn is_gui_running() -> bool {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
        use windows::core::HSTRING;
        let hstring = HSTRING::from("Local\\Synector_GUI_SingleInstance");
        if let Ok(handle) = OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, windows::core::PCWSTR(hstring.as_ptr())) {
            let _ = CloseHandle(handle);
            return true;
        }
        if let Ok(hwnd) = FindWindowW(None, w!("Synector")) {
            if !hwnd.0.is_null() {
                return true;
            }
        }
        false
    }
    #[cfg(not(windows))]
    false
}

/// Checks whether the Synector Daemon process is currently running.
pub fn is_daemon_running() -> bool {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
        use windows::core::HSTRING;
        let hstring = HSTRING::from("Local\\Synector_Daemon_SingleInstance");
        if let Ok(handle) = OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, windows::core::PCWSTR(hstring.as_ptr())) {
            let _ = CloseHandle(handle);
            return true;
        }
        false
    }
    #[cfg(not(windows))]
    false
}

/// Unconditionally forces the given window to the foreground, bypassing Windows focus-stealing restrictions.
#[cfg(windows)]
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
    #[cfg(windows)]
    {
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
    #[cfg(not(windows))]
    false
}

/// Closes the existing Synector GUI window by posting WM_CLOSE.
/// Returns `true` if the window was found and signal posted, `false` otherwise.
pub fn close_existing_gui_window() -> bool {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
        if let Ok(hwnd) = FindWindowW(None, w!("Synector")) {
            if !hwnd.0.is_null() {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, windows::Win32::Foundation::WPARAM(0), windows::Win32::Foundation::LPARAM(0));
                return true;
            }
        }
        false
    }
    #[cfg(not(windows))]
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
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
    #[cfg(windows)]
    fn test_is_gui_running_with_guard() {
        if let Some(guard) = SingleInstanceGuard::acquire_gui() {
            assert!(is_gui_running(), "is_gui_running must be true while GUI single instance guard is held");
            drop(guard);
        }
    }

    #[test]
    #[cfg(windows)]
    fn test_is_daemon_running_with_guard() {
        if let Some(guard) = SingleInstanceGuard::acquire_daemon() {
            assert!(is_daemon_running(), "is_daemon_running must be true while Daemon single instance guard is held");
            drop(guard);
        }
    }
}
