use std::path::{Path, PathBuf};

pub(super) const UPDATE_TEMP_PREFIX: &str = "Synector-update-";

fn update_temp_dir() -> PathBuf {
    std::env::temp_dir().join(format!("{UPDATE_TEMP_PREFIX}{}", std::process::id()))
}

pub(super) fn prepare_update_temp_dir() -> Result<PathBuf, String> {
    let dir = update_temp_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create the update directory {:?}: {e}", dir))?;
    Ok(dir)
}

pub(super) fn remove_dir_quietly(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

/// How long a leftover update entry may stay in `%TEMP%` before the cleanup removes it. Update
/// directories are named after the owning process id, so a young entry can belong to another
/// Synector instance whose download is still in progress.
pub(super) const STALE_UPDATE_TEMP_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// True when an update temp entry is old enough to be cleaned up by an unrelated process.
pub(super) fn update_temp_entry_is_stale(path: &Path, now: std::time::SystemTime) -> bool {
    let Ok(modified) = std::fs::metadata(path).and_then(|meta| meta.modified()) else {
        return false;
    };
    now.duration_since(modified)
        .is_ok_and(|age| age > STALE_UPDATE_TEMP_AGE)
}

/// True when an update temp entry belongs to this process (`Synector-update-<pid>` or its log).
pub(super) fn update_temp_entry_is_ours(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let Some(suffix) = name.strip_prefix(UPDATE_TEMP_PREFIX) else {
        return false;
    };
    suffix
        .split('.')
        .next()
        .and_then(|pid| pid.parse::<u32>().ok())
        .is_some_and(|pid| pid == std::process::id())
}

/// Cleans up leftover update files: staged packages next to the executable and the temporary
/// update directories a previous, interrupted download may have left behind.
pub fn clean_temp_update_files() {
    if let Ok(exe_path) = std::env::current_exe() {
        let download = exe_path.with_extension("download");
        if download.exists() {
            let _ = std::fs::remove_file(&download);
            crate::logging::debug(
                "UPDATER",
                &format!("Removed leftover update file {}", download.display()),
            );
        }
        let pending = exe_path.with_extension("pending");
        if pending.exists() {
            let _ = std::fs::remove_file(&pending);
            crate::logging::debug(
                "UPDATER",
                &format!("Removed leftover update file {}", pending.display()),
            );
        }
        let extracting = exe_path.with_extension("extracting");
        if extracting.exists() {
            let _ = std::fs::remove_file(&extracting);
            crate::logging::debug(
                "UPDATER",
                &format!("Removed leftover update file {}", extracting.display()),
            );
        }
        // A portable update stages `<exe>.pending` and its in-progress `<exe>.extracting` sibling
        // next to the executable; both are removed above. The archive is streamed straight into
        // that file, so there is no extraction directory to clean up.
    }

    let now = std::time::SystemTime::now();
    if let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if !name.to_string_lossy().starts_with(UPDATE_TEMP_PREFIX) {
                continue;
            }
            let path = entry.path();
            // Another portable instance may be staging its own package or installer log here:
            // only this process's entries and entries old enough that no live process can own
            // them are removed.
            if !update_temp_entry_is_ours(&path) && !update_temp_entry_is_stale(&path, now) {
                continue;
            }
            if path.is_dir() {
                let _ = std::fs::remove_dir_all(&path);
            } else {
                let _ = std::fs::remove_file(&path);
            }
            crate::logging::debug(
                "UPDATER",
                &format!("Removed leftover update file {}", path.display()),
            );
        }
    }
}

/// Removes the previous executable left behind by a portable update (`<exe>.old`).
///
/// This gate is unconditional and independent of daemon liveness: a stale `.old` must not survive
/// a launch just because a daemon happens to be running. It is deliberately separate from
/// [`clean_temp_update_files`], which may only run while the daemon is offline.
pub fn remove_old_executable() {
    if let Ok(exe_path) = std::env::current_exe() {
        remove_old_executable_beside(&exe_path);
    }
}

/// The testable half of [`remove_old_executable`]: the stale file is derived from the given
/// executable path.
pub(super) fn remove_old_executable_beside(exe_path: &Path) {
    let old = exe_path.with_extension("old");
    if old.exists() {
        let _ = std::fs::remove_file(old);
    }
}
