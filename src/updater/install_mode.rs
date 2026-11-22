use std::path::{Path, PathBuf};

use super::release::InstallMode;

/// Inno Setup `AppId` of the installer, braces included. The installer registers the uninstaller
/// under `...\Uninstall\{AppId}_is1`; `installer/synector.iss` and this constant must stay in sync.
pub const INSTALLER_APP_ID: &str = "{4A0D9C42-7B31-4E5A-A6F2-3F8D1C2B7E90}";

const UNINSTALL_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall";

/// Returns the installation directory recorded by the Inno Setup installer.
#[cfg(windows)]
fn installer_install_location() -> Option<PathBuf> {
    use windows::core::{PCWSTR, HSTRING};
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};

    let subkey = HSTRING::from(format!("{UNINSTALL_KEY}\\{INSTALLER_APP_ID}_is1"));
    let value = HSTRING::from("InstallLocation");
    let mut buffer = [0u16; 512];
    let mut size = (buffer.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr() as *mut std::ffi::c_void),
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let len = ((size as usize) / 2).saturating_sub(1).min(buffer.len());
    let location = String::from_utf16_lossy(&buffer[..len]);
    (!location.trim().is_empty()).then(|| PathBuf::from(location))
}

/// Directory holding the masked daemon copy (`WindowsServiceHost.exe`).
pub(super) fn masked_daemon_dir() -> PathBuf {
    crate::config::AppConfig::config_dir().join("bin")
}

fn normalized_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

fn same_path(a: &Path, b: &Path) -> bool {
    !a.as_os_str().is_empty() && !b.as_os_str().is_empty() && normalized_path(a) == normalized_path(b)
}

/// True when the running executable is the masked daemon copy rather than the user's binary.
pub(super) fn is_masked_daemon_exe(exe: &Path, masked_dir: &Path) -> bool {
    exe.parent().is_some_and(|parent| same_path(parent, masked_dir))
}

/// Classifies an executable as a portable copy or as part of an installed application.
pub(super) fn resolve_install_mode(exe: &Path, install_location: Option<&Path>, masked_dir: &Path) -> InstallMode {
    let (Some(exe_dir), Some(location)) = (exe.parent(), install_location) else {
        return InstallMode::Portable;
    };
    if same_path(exe_dir, location) {
        return InstallMode::Installed;
    }
    // The masked daemon runs from a user-config copy; it still belongs to the installed
    // application when the installer's directory holds the main executable.
    if same_path(exe_dir, masked_dir) && location.join("Synector.exe").is_file() {
        return InstallMode::Installed;
    }
    InstallMode::Portable
}

/// Detects whether the running executable belongs to an Inno Setup installation.
pub fn current_install_mode() -> InstallMode {
    let Ok(exe) = std::env::current_exe() else {
        return InstallMode::Portable;
    };
    let install_location = installer_install_location();
    if install_location.is_none() {
        // Defensive fallback: an Inno Setup uninstaller next to the executable is a reliable
        // marker that this copy was installed rather than unzipped.
        if exe
            .parent()
            .is_some_and(|dir| dir.join("unins000.exe").is_file())
        {
            return InstallMode::Installed;
        }
    }
    resolve_install_mode(&exe, install_location.as_deref(), &masked_daemon_dir())
}
