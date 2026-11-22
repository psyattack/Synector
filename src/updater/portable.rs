use std::path::{Path, PathBuf};

use crate::command::{run_command_bounded, TOOL_TIMEOUT};
use super::install_mode::{is_masked_daemon_exe, masked_daemon_dir};
use super::util::{escape_ps_literal, system32_tool};

/// Executable of the running Synector GUI process, if one exists.
#[cfg(windows)]
fn gui_process_exe_path() -> Option<PathBuf> {
    use windows::core::{w, PWSTR};
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowThreadProcessId};

    unsafe {
        let hwnd = FindWindowW(None, w!("Synector")).ok()?;
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = [0u16; 1024];
        let mut size = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_FORMAT(0),
            PWSTR(buffer.as_mut_ptr()),
            &mut size,
        );
        let _ = CloseHandle(process);
        result.ok()?;
        let path = PathBuf::from(String::from_utf16_lossy(&buffer[..size as usize]));
        (!path.as_os_str().is_empty()).then_some(path)
    }
}

#[cfg(not(windows))]
fn gui_process_exe_path() -> Option<PathBuf> {
    None
}

/// The executable a portable update has to replace.
///
/// With process masking enabled the portable daemon runs from `...\Synector\bin\WindowsServiceHost.exe`.
/// Staging the update there would leave the user's real executable untouched, and
/// `prepare_masked_daemon` would copy the old executable back over it on the next start. The real
/// target is therefore the executable that launched the GUI: the daemon records it from the
/// `--gui-exe` argument, which stays valid after the GUI window closes (tray-only updates); a
/// running GUI window is only the fallback for a daemon started outside that handshake. Without
/// either the target cannot be established, so the update fails instead of being silently
/// reverted later.
pub(super) fn resolve_portable_update_target() -> Result<PathBuf, String> {
    let current = std::env::current_exe()
        .map_err(|e| format!("Failed to resolve current exe path: {e}"))?;
    if !is_masked_daemon_exe(&current, &masked_daemon_dir()) {
        return Ok(current);
    }
    match resolve_portable_target_from(crate::daemon::launching_gui_exe(), gui_process_exe_path) {
        Some(path) => Ok(path),
        None => Err(
            "The running daemon is a masked copy and the Synector GUI executable could not be \
             located; start Synector normally and retry the update"
                .to_string(),
        ),
    }
}

/// Chooses the real executable from the two available sources: the path the GUI handed to the
/// daemon at spawn time (preferred, it survives the window closing) and the executable of a
/// currently open GUI window. A recorded path that no longer exists falls back to the window
/// lookup instead of staging the update onto a stale file.
pub(super) fn resolve_portable_target_from(
    recorded: Option<PathBuf>,
    running_gui: impl FnOnce() -> Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(path) = recorded.filter(|path| path.is_file()) {
        return Some(path);
    }
    running_gui()
}

/// Recursively locates the exact `Synector.exe` entry in an extracted archive. Any other
/// executable is ignored: the archive is downloaded content, and picking an arbitrary .exe would
/// install and start it as Synector.
fn find_synector_exe(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.eq_ignore_ascii_case("Synector.exe") {
                return Some(path);
            }
        } else if path.is_dir() {
            if let Some(found) = find_synector_exe(&path) {
                return Some(found);
            }
        }
    }
    None
}

/// Unpacks a zip archive into a temporary folder and extracts the main executable into `target_exe_path`.
pub fn extract_exe_from_zip(zip_path: &Path, target_exe_path: &Path) -> Result<(), String> {
    let extract_dir = zip_path.with_extension("extracted");
    if extract_dir.exists() {
        let _ = std::fs::remove_dir_all(&extract_dir);
    }
    std::fs::create_dir_all(&extract_dir)
        .map_err(|e| format!("Failed to create temporary extraction directory {:?}: {e}", extract_dir))?;

    let mut extracted_successfully = false;

    // 1. Try Windows built-in bsdtar (`tar.exe`). System tools are launched by absolute path:
    // bare names resolve through the executable's directory and the inherited working directory
    // before System32, which would let a planted binary run with the daemon's rights.
    #[cfg(windows)]
    {
        match system32_tool("tar.exe") {
            Ok(tar) => {
                let mut cmd = std::process::Command::new(tar);
                cmd.args(["-xf", &zip_path.to_string_lossy(), "-C", &extract_dir.to_string_lossy()]);
                crate::win_args::apply_no_window(&mut cmd);
                if let Ok(outcome) = run_command_bounded(cmd, TOOL_TIMEOUT) {
                    if outcome.succeeded() {
                        extracted_successfully = true;
                    }
                }
            }
            Err(err) => crate::logging::warn("UPDATER", &err),
        }
    }

    #[cfg(not(windows))]
    {
        let mut cmd = std::process::Command::new("tar");
        cmd.args(["-xf", &zip_path.to_string_lossy(), "-C", &extract_dir.to_string_lossy()]);
        if let Ok(outcome) = run_command_bounded(cmd, TOOL_TIMEOUT) {
            if outcome.succeeded() {
                extracted_successfully = true;
            }
        }
    }

    // 2. Fallback to PowerShell Expand-Archive if tar is unavailable
    #[cfg(windows)]
    if !extracted_successfully {
        match system32_tool("powershell.exe") {
            Ok(powershell) => {
                let zip_esc = escape_ps_literal(&zip_path.to_string_lossy());
                let dir_esc = escape_ps_literal(&extract_dir.to_string_lossy());
                let ps_script = format!(
                    "Expand-Archive -LiteralPath '{zip_esc}' -DestinationPath '{dir_esc}' -Force"
                );
                let mut cmd = std::process::Command::new(powershell);
                cmd.args(["-NoProfile", "-NonInteractive", "-Command", &ps_script]);
                crate::win_args::apply_no_window(&mut cmd);
                if let Ok(outcome) = run_command_bounded(cmd, TOOL_TIMEOUT) {
                    if outcome.succeeded() {
                        extracted_successfully = true;
                    }
                }
            }
            Err(err) => crate::logging::warn("UPDATER", &err),
        }
    }

    if !extracted_successfully {
        let _ = std::fs::remove_dir_all(&extract_dir);
        return Err(format!(
            "Failed to unpack zip archive {:?} (tar.exe and powershell.exe extraction both failed)",
            zip_path
        ));
    }

    // Recursively locate the exact `Synector.exe` entry inside extract_dir.
    let found_exe = match find_synector_exe(&extract_dir) {
        Some(p) => p,
        None => {
            let _ = std::fs::remove_dir_all(&extract_dir);
            return Err("The downloaded zip archive does not contain Synector.exe".to_string());
        }
    };

    if target_exe_path.exists() {
        let _ = std::fs::remove_file(target_exe_path);
    }

    if let Err(e) = std::fs::copy(&found_exe, target_exe_path) {
        let _ = std::fs::remove_dir_all(&extract_dir);
        return Err(format!("Failed to copy extracted executable to {:?}: {e}", target_exe_path));
    }

    let _ = std::fs::remove_dir_all(&extract_dir);
    Ok(())
}

pub(super) fn is_zip_archive(path: &Path) -> bool {
    let mut header = [0u8; 2];
    std::fs::File::open(path)
        .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut header))
        .is_ok()
        && &header == b"PK"
}

/// Moves the staged executable into place and keeps the previous one at `old_path`.
///
/// Returns `Ok(true)` when the swap happened in place and `Ok(false)` when the running executable
/// could not be renamed aside (the detached restart script retries after the process exits).
/// Returns `Err` after rolling back: when the swap fails halfway, the previous executable is put
/// back so the installation keeps a runnable binary.
pub(super) fn swap_pending_executable(exe_path: &Path, pending_path: &Path, old_path: &Path) -> Result<bool, String> {
    // Windows NTFS allows renaming a running executable file to .old and placing the new file at
    // the original path. The two renames are separate steps: if moving the staged file into place
    // fails, the old executable goes back instead of leaving the original path empty.
    match std::fs::rename(exe_path, old_path) {
        Ok(()) => match std::fs::rename(pending_path, exe_path) {
            Ok(()) => Ok(true),
            Err(err) => Err(match std::fs::rename(old_path, exe_path) {
                Ok(()) => format!(
                    "Failed to move the downloaded executable into place: {err}; the previous executable was restored"
                ),
                Err(restore_err) => format!(
                    "Failed to move the downloaded executable into place: {err}; the previous executable is kept at {} but could not be restored: {restore_err}",
                    old_path.display()
                ),
            }),
        },
        Err(err) => {
            crate::logging::debug(
                "UPDATER",
                &format!("In-place executable swap failed ({err}); using the detached restart script"),
            );
            Ok(false)
        }
    }
}
