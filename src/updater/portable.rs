use std::io::Read;
use std::path::{Path, PathBuf};

use super::install_mode::{is_masked_daemon_exe, masked_daemon_dir};

/// Ceiling for the executable extracted from a portable update archive.
///
/// The release executable is about 30 MiB; this limit only has to bound a corrupt or malicious
/// archive, not to track the real binary size.
const MAX_PORTABLE_EXE_BYTES: u64 = 64 * 1024 * 1024;

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

/// Extracts the main executable from a zip archive into `target_exe_path`.
///
/// Only the `Synector.exe` entry is read: the archive is downloaded content, and extracting it
/// wholesale would write names chosen by a remote file into the installation directory. The
/// executable may sit in a subfolder or use any letter case, but an ambiguous archive (two
/// candidates), an unsafe path or a suspicious size is refused instead of picked arbitrarily.
pub fn extract_exe_from_zip(zip_path: &Path, target_exe_path: &Path) -> Result<(), String> {
    extract_exe_with_limit(zip_path, target_exe_path, MAX_PORTABLE_EXE_BYTES)
}

/// The testable core of [`extract_exe_from_zip`] with an explicit size ceiling.
pub(super) fn extract_exe_with_limit(
    zip_path: &Path,
    target: &Path,
    limit: u64,
) -> Result<(), String> {
    let file = std::fs::File::open(zip_path)
        .map_err(|e| format!("Failed to open zip archive {:?}: {e}", zip_path))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("Failed to read zip archive {:?}: {e}", zip_path))?;

    let mut found = None;
    for index in 0..archive.len() {
        // Metadata only: nothing is decompressed and no archive-supplied path is used on disk.
        let entry = archive
            .by_index_data(index)
            .map_err(|e| format!("Failed to read zip entry #{index}: {e}"))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry
            .name()
            .map_err(|e| format!("Failed to read the name of zip entry #{index}: {e}"))?
            .replace('\\', "/");
        let base = name.rsplit('/').next().unwrap_or_default();
        if !base.eq_ignore_ascii_case("Synector.exe") {
            continue;
        }
        if name.starts_with('/') || name.split('/').any(|part| part == "..") {
            return Err(format!(
                "The downloaded zip archive contains Synector.exe at an unsafe path: {name:?}"
            ));
        }
        if found.is_some() {
            return Err("The downloaded zip archive contains more than one Synector.exe".to_string());
        }
        let size = entry.size();
        if size == 0 || size > limit {
            return Err(format!(
                "The Synector.exe in the downloaded zip archive has an unexpected size ({size} bytes; the limit is {limit})"
            ));
        }
        found = Some((index, size));
    }

    let Some((index, declared_size)) = found else {
        return Err("The downloaded zip archive does not contain Synector.exe".to_string());
    };

    let mut entry = archive
        .by_index(index)
        .map_err(|e| format!("Failed to open Synector.exe in the zip archive: {e}"))?;

    let tmp = target.with_extension("extracting");
    let _ = std::fs::remove_file(&tmp);
    let mut out = std::fs::File::create(&tmp)
        .map_err(|e| format!("Failed to create temporary file {:?}: {e}", tmp))?;

    // Read one byte past the limit too, so an entry whose header lies about its size is caught.
    // Reading to EOF also makes the zip reader verify the entry's CRC32.
    let copied = match std::io::copy(&mut entry.by_ref().take(limit + 1), &mut out) {
        Ok(copied) => copied,
        Err(e) => {
            drop(out);
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("Failed to extract Synector.exe to {:?}: {e}", tmp));
        }
    };
    drop(out);

    if copied > limit {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!(
            "The Synector.exe in the downloaded zip archive is larger than the {limit}-byte limit"
        ));
    }
    if copied != declared_size {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!(
            "The Synector.exe in the downloaded zip archive has an inconsistent size ({declared_size} declared, {copied} read)"
        ));
    }

    let mut header = [0u8; 2];
    let mut extracted = match std::fs::File::open(&tmp) {
        Ok(file) => file,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("Failed to verify the extracted executable {:?}: {e}", tmp));
        }
    };
    let header_result = extracted.read_exact(&mut header);
    drop(extracted);
    if header_result.is_err() || &header != b"MZ" {
        let _ = std::fs::remove_file(&tmp);
        return Err(
            "The downloaded zip archive entry Synector.exe is not a Windows executable".to_string(),
        );
    }

    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!(
            "Failed to move the extracted executable to {:?}: {e}",
            target
        ));
    }
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
