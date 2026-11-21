use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const GITHUB_REPO: &str = "psyattack/Synector";

/// Inno Setup `AppId` of the installer, braces included. The installer registers the uninstaller
/// under `...\Uninstall\{AppId}_is1`; `installer/synector.iss` and this constant must stay in sync.
pub const INSTALLER_APP_ID: &str = "{4A0D9C42-7B31-4E5A-A6F2-3F8D1C2B7E90}";

const UNINSTALL_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall";
const UPDATE_TEMP_PREFIX: &str = "Synector-update-";

static TLS_PROVIDER: OnceLock<()> = OnceLock::new();

/// Installs the rustls `ring` crypto provider that the update client needs.
///
/// `reqwest` is built with `rustls-no-provider` so the aws-lc-rs backend is not linked: on
/// Windows its vendored jitterentropy marks internal symbols `__declspec(dllexport)`, which
/// makes the MSVC linker export them and emit an import library for `Synector.exe`. `ring` is
/// already part of the dependency tree (the image loaders use it), so installing it here keeps
/// rustls as the TLS implementation without that linker side effect.
pub fn init_tls() {
    TLS_PROVIDER.get_or_init(|| {
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::ring::default_provider().install_default();
        }
    });
}

/// One file published with a GitHub release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// Lowercase SHA-256 hex digest reported by the GitHub API for the asset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

/// How this copy of Synector was put on disk, which decides how an update is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMode {
    /// A stand-alone executable: the update replaces the file in place.
    Portable,
    /// Files placed by the Inno Setup installer: the update runs the new setup silently.
    Installed,
}

impl InstallMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Portable => "portable",
            Self::Installed => "installed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub version: String,
    pub tag_name: String,
    pub name: String,
    pub body: String,
    pub html_url: String,
    /// `Synector-v<version>-Windows-x64.zip` — used by portable copies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portable: Option<UpdateAsset>,
    /// `Synector-Setup-<version>-Windows-x64.exe` — used by installed copies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installer: Option<UpdateAsset>,
}

impl ReleaseInfo {
    /// Picks the asset that matches how the application is installed. An installed copy never
    /// falls back to the portable archive: replacing a single file there would leave the
    /// uninstaller metadata and shortcuts out of date.
    pub fn asset_for_mode(&self, mode: InstallMode) -> Option<&UpdateAsset> {
        match mode {
            InstallMode::Portable => self.portable.as_ref(),
            InstallMode::Installed => self.installer.as_ref(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum UpdateEvent {
    CheckStarted,
    CheckComplete {
        result: Box<Result<Option<ReleaseInfo>, String>>,
        is_manual: bool,
    },
    DownloadProgress(f32),
    DownloadComplete(Result<(), String>),
}

/// Parses a semver-like version string into (major, minor, patch).
/// Strips leading 'v' / 'V' and ignores trailing metadata/pre-release tags.
pub fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    let clean = v.trim().trim_start_matches(['v', 'V']);
    // Take only the numeric part before any '-' or '+' (e.g. 1.0.1-beta -> 1.0.1)
    let num_part = clean.split(&['-', '+'][..]).next().unwrap_or(clean);
    let mut parts = num_part.split('.');
    let major = parts.next()?.parse::<u32>().ok()?;
    let minor = parts.next().unwrap_or("0").parse::<u32>().ok()?;
    let patch = parts.next().unwrap_or("0").parse::<u32>().ok()?;
    Some((major, minor, patch))
}

/// Returns true if `remote` is strictly newer than `current`.
pub fn is_newer_version(current: &str, remote: &str) -> bool {
    match (parse_version(current), parse_version(remote)) {
        (Some((c_maj, c_min, c_pat)), Some((r_maj, r_min, r_pat))) => {
            (r_maj, r_min, r_pat) > (c_maj, c_min, c_pat)
        }
        _ => {
            // Fallback string comparison if unable to parse standard digits
            let c_clean = current.trim().trim_start_matches(['v', 'V']);
            let r_clean = remote.trim().trim_start_matches(['v', 'V']);
            r_clean != c_clean
        }
    }
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    html_url: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    /// GitHub publishes `sha256:<hex>` for uploaded release assets.
    #[serde(default)]
    digest: Option<String>,
}

impl UpdateAsset {
    fn from_github(asset: &GhAsset) -> Self {
        Self {
            name: asset.name.clone(),
            url: asset.browser_download_url.clone(),
            size: asset.size,
            sha256: asset.digest.as_deref().and_then(parse_sha256_digest),
        }
    }
}

/// Extracts a lowercase SHA-256 hex digest from the GitHub `digest` field (`sha256:<hex>`).
fn parse_sha256_digest(digest: &str) -> Option<String> {
    let lower = digest.trim().to_ascii_lowercase();
    let hex = lower.strip_prefix("sha256:").unwrap_or(&lower);
    (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())).then(|| hex.to_string())
}

fn is_portable_asset(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".zip") && lower.contains("synector")
}

fn is_installer_asset(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".exe") && lower.contains("synector-setup")
}

fn describe_assets(info: &ReleaseInfo) -> String {
    let mut parts = Vec::new();
    if let Some(asset) = &info.portable {
        parts.push(format!("portable {} ({} bytes)", asset.name, asset.size));
    }
    if let Some(asset) = &info.installer {
        parts.push(format!("installer {} ({} bytes)", asset.name, asset.size));
    }
    parts.join(", ")
}

/// Turns a parsed GitHub release into update info, or `None` when it is not newer.
fn release_from_gh(release: GhRelease, current_version: &str) -> Result<Option<ReleaseInfo>, String> {
    if !is_newer_version(current_version, &release.tag_name) {
        return Ok(None);
    }

    let remote_ver = release.tag_name.trim().trim_start_matches(['v', 'V']).to_string();
    let portable = release
        .assets
        .iter()
        .find(|a| is_portable_asset(&a.name))
        .map(UpdateAsset::from_github);
    let installer = release
        .assets
        .iter()
        .find(|a| is_installer_asset(&a.name))
        .map(UpdateAsset::from_github);

    if portable.is_none() && installer.is_none() {
        return Err(format!(
            "Release v{remote_ver} contains neither a portable archive nor an installer"
        ));
    }

    let tag_name = release.tag_name.clone();
    let name = release.name.unwrap_or_else(|| format!("v{tag_name}"));
    Ok(Some(ReleaseInfo {
        version: remote_ver,
        tag_name,
        name,
        body: release.body.unwrap_or_default(),
        html_url: release.html_url,
        portable,
        installer,
    }))
}

/// Queries GitHub releases API for the latest release and checks if a newer version is available.
pub async fn check_github_release(repo: &str, current_version: &str) -> Result<Option<ReleaseInfo>, String> {
    init_tls();
    crate::logging::debug(
        "UPDATER",
        &format!("Checking GitHub releases for {repo} (current v{current_version})"),
    );
    let result = check_github_release_inner(repo, current_version).await;
    match &result {
        Ok(Some(info)) => crate::logging::info(
            "UPDATER",
            &format!("Update available: v{} ({})", info.version, describe_assets(info)),
        ),
        Ok(None) => crate::logging::debug(
            "UPDATER",
            &format!("No update available (current v{current_version})"),
        ),
        Err(err) => crate::logging::warn("UPDATER", &format!("Update check failed: {err}")),
    }
    result
}

async fn check_github_release_inner(repo: &str, current_version: &str) -> Result<Option<ReleaseInfo>, String> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let client = reqwest::Client::builder()
        .user_agent(concat!("Synector-Updater/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(12))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Network request failed: {e}"))?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }

    if !response.status().is_success() {
        return Err(format!("GitHub API returned HTTP {}", response.status()));
    }

    let release: GhRelease = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse release metadata: {e}"))?;

    release_from_gh(release, current_version)
}

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

#[cfg(not(windows))]
fn installer_install_location() -> Option<PathBuf> {
    None
}

/// Directory holding the masked daemon copy (`WindowsServiceHost.exe`).
fn masked_daemon_dir() -> PathBuf {
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
fn is_masked_daemon_exe(exe: &Path, masked_dir: &Path) -> bool {
    exe.parent().is_some_and(|parent| same_path(parent, masked_dir))
}

/// Absolute path of a Windows system tool, resolved under `%SystemRoot%\System32`.
///
/// Update helpers must never be spawned by bare name: CreateProcess searches the executable's
/// directory and the inherited working directory before System32, so a planted binary could run
/// with the (possibly elevated) daemon's rights. A missing tool is an error, not a silent fallback
/// to the search order.
#[cfg(windows)]
fn system32_tool(name: &str) -> Result<PathBuf, String> {
    let system_root = std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("WINDIR"))
        .map(PathBuf::from)
        .ok_or_else(|| "Cannot locate the Windows directory (SystemRoot is not set)".to_string())?;
    let tool = system_root.join("System32").join(name);
    if !tool.is_file() {
        return Err(format!(
            "Required Windows tool {} is missing; the update cannot continue",
            tool.display()
        ));
    }
    Ok(tool)
}

/// Escapes a value for a PowerShell single-quoted literal by doubling every apostrophe.
///
/// Every path interpolated into the extraction and restart scripts goes through this helper: an
/// apostrophe inside a single-quoted PowerShell string would otherwise end the literal, breaking
/// the script or injecting commands into an elevated process.
fn escape_ps_literal(value: &str) -> String {
    value.replace('\'', "''")
}

/// Classifies an executable as a portable copy or as part of an installed application.
fn resolve_install_mode(exe: &Path, install_location: Option<&Path>, masked_dir: &Path) -> InstallMode {
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
fn resolve_portable_update_target() -> Result<PathBuf, String> {
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
fn resolve_portable_target_from(
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
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        match system32_tool("tar.exe") {
            Ok(tar) => {
                let mut cmd = std::process::Command::new(tar);
                cmd.args(["-xf", &zip_path.to_string_lossy(), "-C", &extract_dir.to_string_lossy()]);
                cmd.creation_flags(CREATE_NO_WINDOW);
                if let Ok(status) = cmd.status() {
                    if status.success() {
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
        if let Ok(status) = cmd.status() {
            if status.success() {
                extracted_successfully = true;
            }
        }
    }

    // 2. Fallback to PowerShell Expand-Archive if tar is unavailable
    #[cfg(windows)]
    if !extracted_successfully {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        match system32_tool("powershell.exe") {
            Ok(powershell) => {
                let zip_esc = escape_ps_literal(&zip_path.to_string_lossy());
                let dir_esc = escape_ps_literal(&extract_dir.to_string_lossy());
                let ps_script = format!(
                    "Expand-Archive -LiteralPath '{zip_esc}' -DestinationPath '{dir_esc}' -Force"
                );
                let mut cmd = std::process::Command::new(powershell);
                cmd.args(["-NoProfile", "-NonInteractive", "-Command", &ps_script]);
                cmd.creation_flags(CREATE_NO_WINDOW);
                if let Ok(status) = cmd.status() {
                    if status.success() {
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
fn verify_asset_digest(asset_name: &str, expected: Option<&str>, actual: &str) -> Result<(), String> {
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

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn update_temp_dir() -> PathBuf {
    std::env::temp_dir().join(format!("{UPDATE_TEMP_PREFIX}{}", std::process::id()))
}

fn prepare_update_temp_dir() -> Result<PathBuf, String> {
    let dir = update_temp_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create the update directory {:?}: {e}", dir))?;
    Ok(dir)
}

fn remove_dir_quietly(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

fn is_zip_archive(path: &Path) -> bool {
    let mut header = [0u8; 2];
    std::fs::File::open(path)
        .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut header))
        .is_ok()
        && &header == b"PK"
}

/// Stops Synector processes before an update replaces files. Returns true when the GUI was running
/// and should be restored, false for a daemon-only restart. The daemon is the only update
/// initiator: the GUI triggers updates over IPC, so the caller always runs in the daemon.
fn shutdown_for_update() -> bool {
    let gui_running = crate::single_instance::is_gui_running();
    set_gui_closed_during_update(!gui_running);
    if gui_running {
        crate::single_instance::close_existing_gui_window();
    }
    // Cleanly and synchronously dismiss all overlays before replacing binaries.
    crate::daemon::overlay_manager::hide_all_sync();
    std::thread::sleep(std::time::Duration::from_millis(200));
    gui_running
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

#[cfg(not(windows))]
fn launch_installer_process(setup_path: &Path, parameters: &str) -> Result<(), String> {
    std::process::Command::new(setup_path)
        .args(parameters.split_whitespace())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Failed to start the update installer: {e}"))
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

/// How long a leftover update entry may stay in `%TEMP%` before the cleanup removes it. Update
/// directories are named after the owning process id, so a young entry can belong to another
/// Synector instance whose download is still in progress.
const STALE_UPDATE_TEMP_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// True when an update temp entry is old enough to be cleaned up by an unrelated process.
fn update_temp_entry_is_stale(path: &Path, now: std::time::SystemTime) -> bool {
    let Ok(modified) = std::fs::metadata(path).and_then(|meta| meta.modified()) else {
        return false;
    };
    now.duration_since(modified)
        .is_ok_and(|age| age > STALE_UPDATE_TEMP_AGE)
}

/// True when an update temp entry belongs to this process (`Synector-update-<pid>` or its log).
fn update_temp_entry_is_ours(path: &Path) -> bool {
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
        // Extraction always happens under the update temp directory (`<zip>.extracted`), never
        // next to the executable, so there is no exe-adjacent extraction directory to remove.
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

static GUI_CLOSED_DURING_UPDATE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_gui_closed_during_update(closed: bool) {
    GUI_CLOSED_DURING_UPDATE.store(closed, std::sync::atomic::Ordering::SeqCst);
}

pub fn is_gui_closed_during_update() -> bool {
    GUI_CLOSED_DURING_UPDATE.load(std::sync::atomic::Ordering::SeqCst)
}

/// Moves the staged executable into place and keeps the previous one at `old_path`.
///
/// Returns `Ok(true)` when the swap happened in place and `Ok(false)` when the running executable
/// could not be renamed aside (the detached restart script retries after the process exits).
/// Returns `Err` after rolling back: when the swap fails halfway, the previous executable is put
/// back so the installation keeps a runnable binary.
fn swap_pending_executable(exe_path: &Path, pending_path: &Path, old_path: &Path) -> Result<bool, String> {
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

/// Performs executable swap and restarts both daemon and GUI.
///
/// The in-place path renames the running executable aside and moves the staged one into its place,
/// then starts the new process from a detached PowerShell script and exits.
pub fn apply_update_and_restart(exe_path: &Path, pending_path: &Path) -> Result<(), String> {
    crate::logging::info("UPDATER", "Applying update and restarting Synector");
    let mut old_path = exe_path.with_extension("old");
    if old_path.exists() && std::fs::remove_file(&old_path).is_err() {
        old_path = exe_path.with_extension(format!("old.{}", std::process::id()));
    }

    let gui_pid = std::process::id();
    let old_esc = escape_ps_literal(&old_path.to_string_lossy());
    let exe_esc = escape_ps_literal(&exe_path.to_string_lossy());
    let pending_esc = escape_ps_literal(&pending_path.to_string_lossy());

    let restart_args = if is_gui_closed_during_update() {
        " -ArgumentList '--daemon'"
    } else {
        ""
    };

    let focus_post_start = if is_gui_closed_during_update() {
        "".to_string()
    } else {
        "; Start-Sleep -Milliseconds 700; (New-Object -ComObject WScript.Shell).AppActivate('Synector')".to_string()
    };

    let in_place_renamed = swap_pending_executable(exe_path, pending_path, &old_path)?;

    let ps_command = if in_place_renamed {
        // Start the new executable before deleting the old one so a startup failure still leaves
        // the previous binary on disk.
        format!(
            "Wait-Process -Id {gui_pid} -Timeout 10 -ErrorAction SilentlyContinue; \
             Start-Sleep -Milliseconds 600; \
             $started = Start-Process -FilePath '{exe_esc}'{restart_args} -PassThru -ErrorAction SilentlyContinue; \
             if ($started) {{ Remove-Item -LiteralPath '{old_esc}' -Force -ErrorAction SilentlyContinue{focus_post_start} }} \
             else {{ Move-Item -LiteralPath '{old_esc}' -Destination '{exe_esc}' -Force -ErrorAction SilentlyContinue; Start-Process -FilePath '{exe_esc}'{restart_args} }}",
        )
    } else {
        // The executable could not be renamed while running: retry from the detached script once
        // this process has exited. The previous binary is kept until the new one is in place and
        // started, and a failed move restores it instead of deleting the only copy.
        format!(
            "Wait-Process -Id {gui_pid} -Timeout 10 -ErrorAction SilentlyContinue; \
             Get-Process -Name 'Synector' -ErrorAction SilentlyContinue | Where-Object {{ $_.Id -ne $PID }} | Stop-Process -Force -ErrorAction SilentlyContinue; \
             Start-Sleep -Milliseconds 400; \
             $sw = [System.Diagnostics.Stopwatch]::StartNew(); \
             while ($sw.ElapsedMilliseconds -lt 6000) {{ \
                 if (Test-Path -LiteralPath '{pending_esc}') {{ \
                     if (Test-Path -LiteralPath '{exe_esc}') {{ \
                         Remove-Item -LiteralPath '{old_esc}' -Force -ErrorAction SilentlyContinue; \
                         Move-Item -LiteralPath '{exe_esc}' -Destination '{old_esc}' -Force -ErrorAction SilentlyContinue; \
                     }} \
                     Move-Item -LiteralPath '{pending_esc}' -Destination '{exe_esc}' -Force -ErrorAction SilentlyContinue; \
                 }} \
                 if (Test-Path -LiteralPath '{exe_esc}') {{ break; }} \
                 if ((Test-Path -LiteralPath '{old_esc}') -and -not (Test-Path -LiteralPath '{exe_esc}')) {{ \
                     Move-Item -LiteralPath '{old_esc}' -Destination '{exe_esc}' -Force -ErrorAction SilentlyContinue; \
                 }} \
                 Start-Sleep -Milliseconds 250; \
             }} \
             $started = Start-Process -FilePath '{exe_esc}'{restart_args} -PassThru -ErrorAction SilentlyContinue; \
             if ($started) {{ Remove-Item -LiteralPath '{old_esc}' -Force -ErrorAction SilentlyContinue{focus_post_start} }}",
        )
    };

    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};
        let _ = AllowSetForegroundWindow(ASFW_ANY);
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let powershell = system32_tool("powershell.exe")?;
        let mut cmd = std::process::Command::new(powershell);
        cmd.args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &ps_command]);
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd.spawn().map_err(|e| format!("Failed to spawn restart script: {e}"))?;
    }

    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new(exe_path).spawn();
    }

    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZIP_SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const SETUP_SHA: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

    fn release_json() -> String {
        format!(
            r###"{{
            "tag_name": "v2.9.0",
            "name": "Synector v2.9.0",
            "body": "## What's Changed\n* Added installer",
            "html_url": "https://github.com/psyattack/Synector/releases/tag/v2.9.0",
            "assets": [
                {{
                    "name": "checksums.txt",
                    "browser_download_url": "https://github.com/psyattack/Synector/releases/download/v2.9.0/checksums.txt",
                    "size": 128
                }},
                {{
                    "name": "Synector-v2.9.0-Windows-x64.zip",
                    "browser_download_url": "https://github.com/psyattack/Synector/releases/download/v2.9.0/Synector-v2.9.0-Windows-x64.zip",
                    "size": 15728640,
                    "digest": "sha256:{ZIP_SHA}"
                }},
                {{
                    "name": "Synector-Setup-2.9.0-Windows-x64.exe",
                    "browser_download_url": "https://github.com/psyattack/Synector/releases/download/v2.9.0/Synector-Setup-2.9.0-Windows-x64.exe",
                    "size": 16777216,
                    "digest": "sha256:{SETUP_SHA}"
                }}
            ]
        }}"###
        )
    }

    #[test]
    fn test_clean_temp_update_files_no_panic() {
        clean_temp_update_files();
    }

    #[test]
    fn test_version_parsing() {
        assert_eq!(parse_version("1.0.0"), Some((1, 0, 0)));
        assert_eq!(parse_version("v1.0.0"), Some((1, 0, 0)));
        assert_eq!(parse_version("V2.4.9"), Some((2, 4, 9)));
        assert_eq!(parse_version("1.2"), Some((1, 2, 0)));
        assert_eq!(parse_version("v1.1.0-beta.1"), Some((1, 1, 0)));
        assert_eq!(parse_version("v1.3.0+build.42"), Some((1, 3, 0)));
    }

    #[test]
    fn test_is_newer_version() {
        assert!(is_newer_version("1.0.0", "1.0.1"));
        assert!(is_newer_version("1.0.0", "v1.1.0"));
        assert!(is_newer_version("1.0.0", "v2.0.0"));
        assert!(is_newer_version("1.2.3", "v1.3.0"));
        assert!(is_newer_version("1.2.3", "v1.2.4"));

        assert!(!is_newer_version("1.0.0", "1.0.0"));
        assert!(!is_newer_version("1.0.0", "v1.0.0"));
        assert!(!is_newer_version("1.1.0", "v1.0.9"));
        assert!(!is_newer_version("2.0.0", "v1.9.9"));
        assert!(!is_newer_version("1.0.5", "0.9.9"));
    }

    #[test]
    fn test_github_release_deserialization_and_asset_selection() {
        let release: GhRelease = serde_json::from_str(&release_json()).expect("failed to deserialize release");
        let info = release_from_gh(release, "2.8.0")
            .expect("newer release should parse")
            .expect("release should be newer than 2.8.0");

        assert_eq!(info.version, "2.9.0");
        assert_eq!(info.tag_name, "v2.9.0");

        let portable = info.portable.as_ref().expect("portable asset");
        assert_eq!(portable.name, "Synector-v2.9.0-Windows-x64.zip");
        assert_eq!(portable.size, 15_728_640);
        assert_eq!(portable.sha256.as_deref(), Some(ZIP_SHA));

        let installer = info.installer.as_ref().expect("installer asset");
        assert_eq!(installer.name, "Synector-Setup-2.9.0-Windows-x64.exe");
        assert_eq!(installer.size, 16_777_216);
        assert_eq!(installer.sha256.as_deref(), Some(SETUP_SHA));

        assert_eq!(
            info.asset_for_mode(InstallMode::Portable).map(|a| a.name.as_str()),
            Some("Synector-v2.9.0-Windows-x64.zip")
        );
        assert_eq!(
            info.asset_for_mode(InstallMode::Installed).map(|a| a.name.as_str()),
            Some("Synector-Setup-2.9.0-Windows-x64.exe")
        );

        let same_release: GhRelease = serde_json::from_str(&release_json()).unwrap();
        assert!(
            release_from_gh(same_release, "2.9.0").unwrap().is_none(),
            "an equal version must not be offered as an update"
        );
    }

    #[test]
    fn test_github_release_without_update_assets_is_rejected() {
        let json = r#"{
            "tag_name": "v3.0.0",
            "name": "Synector v3.0.0",
            "body": "",
            "html_url": "https://github.com/psyattack/Synector/releases/tag/v3.0.0",
            "assets": [
                {"name": "checksums.txt", "browser_download_url": "https://example.com/checksums.txt", "size": 10}
            ]
        }"#;
        let release: GhRelease = serde_json::from_str(json).unwrap();
        assert!(release_from_gh(release, "2.9.0").is_err());
    }

    #[test]
    fn test_parse_sha256_digest() {
        assert_eq!(
            parse_sha256_digest(&format!("sha256:{}", ZIP_SHA.to_uppercase())),
            Some(ZIP_SHA.to_string())
        );
        assert_eq!(parse_sha256_digest(&format!("  SHA256:{ZIP_SHA}  ")), Some(ZIP_SHA.to_string()));
        assert_eq!(parse_sha256_digest("sha256:abc"), None);
        assert_eq!(parse_sha256_digest("md5:0123"), None);
        assert_eq!(parse_sha256_digest(""), None);
    }

    #[test]
    fn test_hex_encode() {
        assert_eq!(hex_encode(&[0x00, 0x0f, 0xa5, 0xff]), "000fa5ff");
        assert_eq!(hex_encode(&[]), "");
    }

    #[test]
    fn test_resolve_install_mode() {
        let masked_dir = Path::new("C:\\Users\\u\\AppData\\Local\\Synector\\bin");
        let program_files = Path::new("C:\\Program Files\\Synector");

        assert_eq!(
            resolve_install_mode(
                &program_files.join("Synector.exe"),
                Some(program_files),
                masked_dir
            ),
            InstallMode::Installed
        );
        assert_eq!(
            resolve_install_mode(
                Path::new("d:\\apps\\synector\\SYNECTOR.EXE"),
                Some(Path::new("D:\\Apps\\Synector\\")),
                masked_dir
            ),
            InstallMode::Installed,
            "paths compare case-insensitively and ignore a trailing separator"
        );
        assert_eq!(
            resolve_install_mode(
                Path::new("D:\\Tools\\Portable\\Synector.exe"),
                Some(program_files),
                masked_dir
            ),
            InstallMode::Portable
        );
        assert_eq!(
            resolve_install_mode(Path::new("D:\\Tools\\Synector.exe"), None, masked_dir),
            InstallMode::Portable,
            "no installer registration means a portable copy"
        );

        // The masked daemon of an installed application is recognized by the copied executable
        // it runs next to the user configuration.
        let temp = std::env::temp_dir().join(format!("synector_mode_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();
        std::fs::write(temp.join("Synector.exe"), b"MZ").unwrap();
        assert_eq!(
            resolve_install_mode(
                &masked_dir.join("WindowsServiceHost.exe"),
                Some(&temp),
                masked_dir
            ),
            InstallMode::Installed
        );
        assert_eq!(
            resolve_install_mode(
                &masked_dir.join("WindowsServiceHost.exe"),
                Some(&temp.join("missing")),
                masked_dir
            ),
            InstallMode::Portable
        );
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_is_zip_archive() {
        let temp = std::env::temp_dir().join(format!("synector_zip_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();

        let zip = temp.join("archive.zip");
        std::fs::write(&zip, b"PK\x03\x04rest").unwrap();
        assert!(is_zip_archive(&zip));

        let plain = temp.join("plain.bin");
        std::fs::write(&plain, b"MZbinary").unwrap();
        assert!(!is_zip_archive(&plain));
        assert!(!is_zip_archive(&temp.join("missing.zip")));

        let _ = std::fs::remove_dir_all(&temp);
    }

    /// Writes a zip archive with the given `(file name, contents)` entries.
    fn write_zip(zip_path: &Path, files: &[(&str, &[u8])]) {
        use std::io::Write as _;
        let mut writer = zip::ZipWriter::new(std::fs::File::create(zip_path).unwrap());
        for (name, contents) in files {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(contents).unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn test_extract_exe_from_zip() {
        let temp_dir = std::env::temp_dir().join(format!("synector_test_zip_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let zip_path = temp_dir.join("test_archive.zip");
        write_zip(&zip_path, &[("Synector.exe", b"MZ_MOCK_EXECUTABLE_BINARY_DATA")]);

        let extracted_target = temp_dir.join("Extracted_Synector.exe");
        let result = extract_exe_from_zip(&zip_path, &extracted_target);
        assert!(result.is_ok(), "Extraction failed: {:?}", result.err());
        assert!(extracted_target.exists());
        let extracted_data = std::fs::read(&extracted_target).unwrap();
        assert_eq!(extracted_data, b"MZ_MOCK_EXECUTABLE_BINARY_DATA");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_extract_rejects_archive_without_synector_exe() {
        let temp_dir = std::env::temp_dir().join(format!("synector_test_no_exe_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let zip_path = temp_dir.join("no_synector.zip");
        write_zip(&zip_path, &[("helper.exe", b"MZ_NOT_SYNECTOR"), ("readme.txt", b"notes")]);

        let extracted_target = temp_dir.join("Extracted.exe");
        let err = extract_exe_from_zip(&zip_path, &extracted_target)
            .expect_err("an archive without Synector.exe must be rejected");
        assert!(err.contains("Synector.exe"), "unexpected error: {err}");
        assert!(!extracted_target.exists(), "an arbitrary .exe must never be staged");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_missing_asset_digest_is_rejected() {
        let err = verify_asset_digest("Synector-v2.9.0-Windows-x64.zip", None, ZIP_SHA)
            .expect_err("an asset without a digest must not be applied");
        assert!(
            err.contains("no published SHA-256 digest"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_asset_digest_is_verified_case_insensitively() {
        assert!(verify_asset_digest("asset", Some(ZIP_SHA), &ZIP_SHA.to_ascii_uppercase()).is_ok());
        assert!(verify_asset_digest("asset", Some(ZIP_SHA), SETUP_SHA).is_err());
    }

    #[test]
    fn test_escape_ps_literal() {
        assert_eq!(
            escape_ps_literal(r"C:\Users\O'Brien\Synector.exe"),
            r"C:\Users\O''Brien\Synector.exe"
        );
        assert_eq!(
            escape_ps_literal(r"C:\Synector\Synector.exe"),
            r"C:\Synector\Synector.exe"
        );
        assert_eq!(escape_ps_literal("'"), "''");
        assert_eq!(escape_ps_literal("it's 'quoted'"), "it''s ''quoted''");
    }

    #[test]
    fn test_is_masked_daemon_exe() {
        let masked_dir = Path::new("C:\\Users\\u\\AppData\\Local\\Synector\\bin");
        assert!(is_masked_daemon_exe(
            &masked_dir.join("WindowsServiceHost.exe"),
            masked_dir
        ));
        assert!(is_masked_daemon_exe(
            Path::new("C:\\Users\\u\\AppData\\Local\\Synector\\Bin\\WindowsServiceHost.exe"),
            masked_dir
        ));
        assert!(!is_masked_daemon_exe(
            Path::new("D:\\Tools\\Portable\\Synector.exe"),
            masked_dir
        ));
    }

    #[test]
    fn test_portable_update_target_prefers_the_recorded_gui_executable() {
        let dir = std::env::temp_dir().join(format!("synector_update_target_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let recorded = dir.join("Synector.exe");
        let running_gui = dir.join("RunningSynector.exe");
        std::fs::write(&recorded, b"MZ").unwrap();

        // The path the GUI handed to the daemon wins, and the window lookup is not even consulted:
        // it is the only source that still works after the GUI window has closed.
        assert_eq!(
            resolve_portable_target_from(Some(recorded.clone()), || {
                panic!("a valid recorded executable must not need the GUI-window lookup")
            }),
            Some(recorded.clone())
        );

        // A recorded path that no longer exists must fall back instead of staging onto a stale file.
        let stale = dir.join("gone.exe");
        assert_eq!(
            resolve_portable_target_from(Some(stale), || Some(running_gui.clone())),
            Some(running_gui.clone())
        );
        assert_eq!(
            resolve_portable_target_from(None, || Some(running_gui.clone())),
            Some(running_gui.clone())
        );
        assert_eq!(resolve_portable_target_from(None, || None), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_update_temp_entry_staleness() {
        let temp = std::env::temp_dir().join(format!("synector_stale_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();
        let entry = temp.join("entry.zip");
        std::fs::write(&entry, b"x").unwrap();

        let now = std::time::SystemTime::now();
        assert!(
            !update_temp_entry_is_stale(&entry, now),
            "a fresh entry may belong to a live instance"
        );

        let file = std::fs::OpenOptions::new().write(true).open(&entry).unwrap();
        file.set_modified(now - STALE_UPDATE_TEMP_AGE - std::time::Duration::from_secs(60))
            .unwrap();
        drop(file);
        assert!(
            update_temp_entry_is_stale(&entry, now),
            "an old entry is a leftover"
        );
        assert!(!update_temp_entry_is_stale(&temp.join("missing"), now));

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_update_temp_entry_ownership() {
        let temp = std::env::temp_dir();
        let own_dir = temp.join(format!("{UPDATE_TEMP_PREFIX}{}", std::process::id()));
        let own_log = temp.join(format!("{UPDATE_TEMP_PREFIX}{}.log", std::process::id()));
        let other_pid = std::process::id().wrapping_add(1);
        let other_dir = temp.join(format!("{UPDATE_TEMP_PREFIX}{other_pid}"));

        assert!(update_temp_entry_is_ours(&own_dir));
        assert!(update_temp_entry_is_ours(&own_log));
        assert!(!update_temp_entry_is_ours(&other_dir));
        assert!(!update_temp_entry_is_ours(&temp.join("unrelated.txt")));
    }

    #[test]
    fn test_swap_failure_restores_previous_executable() {
        let temp = std::env::temp_dir().join(format!("synector_swap_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();
        let exe = temp.join("Synector.exe");
        let pending = temp.join("Synector.pending");
        let old = temp.join("Synector.old");
        std::fs::write(&exe, b"OLD_BINARY").unwrap();

        // No staged file: the second rename fails after the running executable was moved aside.
        // The previous executable must be restored and the failure reported.
        let err = swap_pending_executable(&exe, &pending, &old)
            .expect_err("a half-completed swap must fail");
        assert!(err.contains("restored"), "unexpected error: {err}");
        assert_eq!(
            std::fs::read(&exe).unwrap(),
            b"OLD_BINARY",
            "the previous executable must be back at its original path"
        );
        assert!(!old.exists());

        // With a staged file the swap succeeds and the previous executable is kept as .old.
        std::fs::write(&pending, b"NEW_BINARY").unwrap();
        assert!(swap_pending_executable(&exe, &pending, &old).unwrap());
        assert_eq!(std::fs::read(&exe).unwrap(), b"NEW_BINARY");
        assert_eq!(std::fs::read(&old).unwrap(), b"OLD_BINARY");
        assert!(!pending.exists());

        // When the running executable cannot be renamed aside, the caller falls back to the
        // detached restart script instead of reporting success.
        std::fs::remove_file(&old).unwrap();
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("keep.txt"), b"keep").unwrap();
        std::fs::write(&pending, b"NEW_BINARY").unwrap();
        assert!(!swap_pending_executable(&exe, &pending, &old).unwrap());
        assert_eq!(std::fs::read(&exe).unwrap(), b"NEW_BINARY");

        let _ = std::fs::remove_dir_all(&temp);
    }
}
