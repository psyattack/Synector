use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::apply::{apply_update_and_restart, shutdown_for_update};
use super::install_mode::current_install_mode;
use super::portable::{extract_exe_from_zip, is_zip_archive, resolve_portable_update_target};
use super::release::{init_tls, InstallMode, ReleaseInfo, UpdateAsset};
use super::temp::{prepare_update_temp_dir, remove_dir_quietly, UPDATE_TEMP_PREFIX};

/// Downloads one update asset into `dest_dir`, streaming progress and verifying the size and
/// the SHA-256 digest published by the GitHub API.
async fn download_asset<F>(asset: &UpdateAsset, dest_dir: &Path, progress_cb: &F) -> Result<PathBuf, String>
where
    F: Fn(f32),
{
    init_tls();
    let client = reqwest::Client::builder()
        .user_agent(concat!("Synector-Updater/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let mut response = client
        .get(&asset.url)
        .send()
        .await
        .map_err(|e| format!("Failed to start download: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("Download failed with HTTP {}", response.status()));
    }

    let file_name = Path::new(&asset.name)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "Synector-update.bin".to_string());
    let download_path = dest_dir.join(file_name);
    let total_size = response.content_length().unwrap_or(asset.size);

    let mut file = std::fs::File::create(&download_path)
        .map_err(|e| format!("Failed to create download file {:?}: {e}", download_path))?;
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;

    while let Some(chunk) = response.chunk().await.map_err(|e| format!("Error during download: {e}"))? {
        file.write_all(&chunk).map_err(|e| format!("Failed to write chunk: {e}"))?;
        hasher.update(&chunk);
        downloaded += chunk.len() as u64;
        if total_size > 0 {
            progress_cb((downloaded as f32 / total_size as f32).clamp(0.0, 1.0));
        }
    }

    file.flush().map_err(|e| format!("Failed to flush download file: {e}"))?;
    drop(file);

    if downloaded == 0 {
        let _ = std::fs::remove_file(&download_path);
        return Err("Downloaded file is empty".to_string());
    }
    if asset.size > 0 && downloaded != asset.size {
        let _ = std::fs::remove_file(&download_path);
        return Err(format!(
            "Update package size mismatch: expected {} bytes, received {downloaded}",
            asset.size
        ));
    }

    let digest = hex_encode(&hasher.finalize());
    if let Err(err) = verify_asset_digest(&asset.name, asset.sha256.as_deref(), &digest) {
        let _ = std::fs::remove_file(&download_path);
        return Err(err);
    }

    Ok(download_path)
}

/// Checks a finished download against the SHA-256 digest the GitHub API publishes for the asset.
///
/// A missing digest is a verification failure, not a warning: the package is extracted and its
/// executable is started, so an unverifiable download must never reach the apply stage.
pub(super) fn verify_asset_digest(asset_name: &str, expected: Option<&str>, actual: &str) -> Result<(), String> {
    let Some(expected) = expected else {
        return Err(format!(
            "Update asset {asset_name} has no published SHA-256 digest; refusing to apply an unverified download"
        ));
    };
    if !expected.eq_ignore_ascii_case(actual) {
        return Err(format!(
            "Update package checksum mismatch: expected {expected}, received {actual}"
        ));
    }
    crate::logging::debug("UPDATER", "Update package checksum verified (SHA-256)");
    Ok(())
}

pub(super) fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(windows)]
fn launch_installer_process(setup_path: &Path, parameters: &str) -> Result<(), String> {
    use windows::core::{PCWSTR, HSTRING};
    use windows::Win32::UI::Shell::{ShellExecuteExW, SHELLEXECUTEINFOW, SEE_MASK_NOASYNC};
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

    let file = HSTRING::from(setup_path.to_string_lossy().to_string());
    let params = HSTRING::from(parameters.to_string());
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC,
        // No explicit verb: the installer manifest requests administrator privileges, so Windows
        // elevates it when needed and starts it directly when Synector is already elevated.
        lpVerb: PCWSTR::null(),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe {
        ShellExecuteExW(&mut info).map_err(|err| {
            format!(
                "Failed to start the update installer (administrator elevation may have been declined): {err}"
            )
        })?;
    }
    Ok(())
}

/// Runs the downloaded setup silently and exits: the installer replaces the files, refreshes the
/// uninstaller metadata and restarts Synector in the mode requested by `UpdateMode`.
fn install_with_installer(setup_path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    if !crate::daemon::watcher::is_elevated() {
        return Err("Installing an update requires administrator rights".to_string());
    }

    let restart_gui = shutdown_for_update();
    let mode = if restart_gui { "gui" } else { "daemon" };
    let log_path = std::env::temp_dir().join(format!("{UPDATE_TEMP_PREFIX}{}.log", std::process::id()));
    let parameters = format!(
        "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /NOCANCEL /UpdateMode={mode} /LOG=\"{}\"",
        log_path.display()
    );
    crate::logging::info(
        "UPDATER",
        &format!("Starting the silent update installer ({mode} restart)"),
    );
    launch_installer_process(setup_path, &parameters)?;
    std::process::exit(0);
}

async fn download_and_install_impl<F>(info: &ReleaseInfo, progress_cb: F) -> Result<(), String>
where
    F: Fn(f32) + Send + 'static,
{
    let mode = current_install_mode();
    let asset = info.asset_for_mode(mode).cloned().ok_or_else(|| match mode {
        InstallMode::Portable => "This release does not publish a portable archive".to_string(),
        InstallMode::Installed => {
            "This release does not publish an installer; install the update manually".to_string()
        }
    })?;

    crate::logging::info(
        "UPDATER",
        &format!(
            "Downloading v{} update ({}, {}, {})",
            info.version,
            asset.name,
            asset.size,
            mode.label()
        ),
    );

    let temp_dir = prepare_update_temp_dir()?;
    let downloaded = match download_asset(&asset, &temp_dir, &progress_cb).await {
        Ok(path) => path,
        Err(err) => {
            remove_dir_quietly(&temp_dir);
            return Err(err);
        }
    };

    match mode {
        InstallMode::Installed => {
            let result = install_with_installer(&downloaded);
            if result.is_err() {
                remove_dir_quietly(&temp_dir);
            }
            result
        }
        InstallMode::Portable => {
            let exe_path = resolve_portable_update_target()?;
            let pending_path = exe_path.with_extension("pending");

            if !is_zip_archive(&downloaded) {
                remove_dir_quietly(&temp_dir);
                return Err("The portable update package is not a zip archive".to_string());
            }
            let extract_result = extract_exe_from_zip(&downloaded, &pending_path);
            remove_dir_quietly(&temp_dir);
            extract_result?;

            let pending_meta = std::fs::metadata(&pending_path)
                .map_err(|e| format!("Failed to verify pending executable: {e}"))?;
            if pending_meta.len() == 0 {
                let _ = std::fs::remove_file(&pending_path);
                return Err("Pending executable file is empty".to_string());
            }

            crate::logging::debug(
                "UPDATER",
                &format!(
                    "Portable update verified ({} bytes); preparing installation",
                    pending_meta.len()
                ),
            );
            shutdown_for_update();
            apply_update_and_restart(&exe_path, &pending_path)
        }
    }
}

async fn download_and_install_internal<F>(info: &ReleaseInfo, progress_cb: F) -> Result<(), String>
where
    F: Fn(f32) + Send + 'static,
{
    let result = download_and_install_impl(info, progress_cb).await;
    match &result {
        Ok(()) => crate::logging::info("UPDATER", "Update package downloaded and applied"),
        Err(err) => crate::logging::error("UPDATER", &format!("Update failed: {err}")),
    }
    result
}

/// Downloads and applies an update triggered from the background daemon.
pub async fn download_and_install_daemon<F>(info: &ReleaseInfo, progress_cb: F) -> Result<(), String>
where
    F: Fn(f32) + Send + 'static,
{
    download_and_install_internal(info, progress_cb).await
}
