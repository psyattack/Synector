//! Console reattachment for release builds, which are Windows GUI-subsystem executables.

/// Release builds use the Windows GUI subsystem, so a CLI invocation has to reattach to the
/// parent console before the first line is printed. Redirected std handles (pipes) are left
/// untouched so machine-readable output keeps working.
pub(crate) fn ensure_console() {
    #[cfg(all(windows, not(debug_assertions)))]
    attach_parent_console();
}

#[cfg(all(windows, not(debug_assertions)))]
fn attach_parent_console() {
    use windows::core::w;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, GetFileType, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_WRITE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, FILE_TYPE_UNKNOWN, OPEN_EXISTING,
    };
    use windows::Win32::System::Console::{
        AttachConsole, GetStdHandle, SetConsoleOutputCP, SetStdHandle, ATTACH_PARENT_PROCESS,
        STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
    };

    unsafe {
        let stdout_is_real = GetStdHandle(STD_OUTPUT_HANDLE)
            .map(|handle| GetFileType(handle) != FILE_TYPE_UNKNOWN)
            .unwrap_or(false);
        if stdout_is_real {
            return;
        }
        if AttachConsole(ATTACH_PARENT_PROCESS).is_err() {
            return;
        }
        let _ = SetConsoleOutputCP(65001);
        if let Ok(conout) = CreateFileW(
            w!("CONOUT$"),
            FILE_GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        ) {
            let _ = SetStdHandle(STD_OUTPUT_HANDLE, conout);
            let _ = SetStdHandle(STD_ERROR_HANDLE, conout);
        }
    }
}
