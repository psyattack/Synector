use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

pub const GITHUB_REPO: &str = "psyattack/Synector";

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
pub(super) struct GhRelease {
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
pub(super) fn parse_sha256_digest(digest: &str) -> Option<String> {
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
pub(super) fn release_from_gh(release: GhRelease, current_version: &str) -> Result<Option<ReleaseInfo>, String> {
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
