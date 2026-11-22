//! Self-elevation through the `runas` shell verb.

/// Relaunches the process elevated, with exactly the arguments of this launch.
///
/// Returns `true` when an elevated process was started; the caller then returns without
/// continuing the unelevated dispatch. `--no-elevate` is handled by the caller.
#[cfg(windows)]
pub(crate) fn try_self_elevate(launch: &super::args::LaunchContext) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::SW_NORMAL;
    use windows::Win32::UI::Shell::{ShellExecuteExW, SHELLEXECUTEINFOW, SEE_MASK_NOASYNC};
    use windows::core::{HSTRING, PCWSTR};

    use super::args::build_elevation_arguments;

    if crate::daemon::watcher::is_elevated() {
        return false;
    }

    let exe_path = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return false,
    };

    let args_str = build_elevation_arguments(launch.argv());

    let verb = HSTRING::from("runas");
    let file = HSTRING::from(exe_path.to_string_lossy().to_string());
    let params = HSTRING::from(args_str);

    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_NORMAL.0,
        ..Default::default()
    };

    unsafe {
        ShellExecuteExW(&mut info).is_ok()
    }
}
