//! Persisted user configuration.
//!
//! The configuration is split across four independent, versioned JSON files in the
//! config directory instead of one aggregate document:
//!
//! | File          | Contents                                      | Write frequency  |
//! |---------------|-----------------------------------------------|------------------|
//! | `settings.json` | window, tray, language, theme, update options | rare           |
//! | `rules.json`    | automation definitions, no runtime counters  | on rule edits    |
//! | `library.json`  | overlay and macro presets                    | on preset edits  |
//! | `runtime.json`  | per-rule execution counters                  | coalesced        |
//!
//! [`AppConfig`] is the in-memory aggregate exchanged by the daemon, the GUI and IPC;
//! it is never serialized to disk as a whole. The daemon owns the files, the GUI only
//! edits a draft and sends mutations over IPC, and the CLI writes `rules.json` when the
//! daemon is offline. Every write is atomic (temp file + rename) and stamped with
//! [`CONFIG_VERSION`], so a crash mid-write cannot truncate user data.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, RwLock};
use std::time::{Duration, SystemTime};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::models::{MacroPreset, OverlayDefinition, Rule};

/// Schema version stamped into every config file. Bump it when a section layout
/// changes and handle the older value in [`normalize_version`].
pub const CONFIG_VERSION: u32 = 1;

const SETTINGS_FILE: &str = "settings.json";
const RULES_FILE: &str = "rules.json";
const LIBRARY_FILE: &str = "library.json";
const RUNTIME_FILE: &str = "runtime.json";
const SECTION_FILES: [&str; 4] = [SETTINGS_FILE, RULES_FILE, LIBRARY_FILE, RUNTIME_FILE];

/// A cached file stamp is trusted only after the filesystem timestamp has settled. Coarse
/// timestamp volumes (FAT/exFAT, portable mode) can otherwise hide an equal-length rewrite
/// that happened within the same timestamp tick.
const STAMP_SETTLE: Duration = Duration::from_secs(2);

/// `<section>.<pid>.tmp` files older than this were left behind by a writer that died between
/// the temp-file write and the rename; younger files may belong to a live writer.
const STALE_TEMP_AGE: Duration = Duration::from_secs(5 * 60);

fn default_version() -> u32 {
    CONFIG_VERSION
}

fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------------------
// Atomic section I/O
// ---------------------------------------------------------------------------

fn section_path(dir: &Path, file: &str) -> PathBuf {
    dir.join(file)
}

fn tmp_path(path: &Path) -> PathBuf {
    // The process id keeps concurrent writers (daemon, GUI, CLI) from clobbering each
    // other's temporary file; the rename itself is atomic and last-write-wins.
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".{}.tmp", std::process::id()));
    PathBuf::from(name)
}

/// Removes `<section>.<pid>.tmp` files left behind by a writer that died between the temp-file
/// write and the rename. Only names produced by [`tmp_path`] and older than [`STALE_TEMP_AGE`]
/// are touched, so a live writer's fresh temporary file survives.
fn clean_stale_temp_files(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    let mut removed = 0u32;
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        let is_section_tmp = SECTION_FILES.iter().any(|section| {
            file_name
                .strip_prefix(section)
                .and_then(|rest| rest.strip_prefix('.'))
                .and_then(|rest| rest.strip_suffix(".tmp"))
                .is_some_and(|pid| {
                    !pid.is_empty() && pid.bytes().all(|byte| byte.is_ascii_digit())
                })
        });
        if !is_section_tmp {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= STALE_TEMP_AGE);
        if stale && fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    if removed > 0 {
        crate::logging::debug(
            "CONFIG",
            &format!("Removed {removed} stale config temp file(s)"),
        );
    }
}

fn log_section_create_failure(section: &str, result: std::io::Result<()>) {
    if let Err(err) = result {
        crate::logging::error("CONFIG", &format!("Failed to create {section}: {err}"));
    }
}

/// Reads one JSON section. A missing file is not an error: the caller falls back to
/// defaults so a partially deleted config directory still starts up. An unparsable file
/// is moved aside as `<name>.corrupt-<unix time>` before defaults take over, so the next
/// save cannot silently destroy recoverable user data.
fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) => {
            if err.kind() != std::io::ErrorKind::NotFound {
                crate::logging::error(
                    "CONFIG",
                    &format!("Failed to read {}: {err}", path.display()),
                );
            }
            return None;
        }
    };
    match serde_json::from_str(&raw) {
        Ok(value) => Some(value),
        Err(err) => {
            let quarantined = quarantine_corrupt(path);
            let detail = match &quarantined {
                Some(target) => format!("; moved aside to {}", target.display()),
                None => String::new(),
            };
            crate::logging::error(
                "CONFIG",
                &format!("Failed to parse {}: {err}{detail}", path.display()),
            );
            None
        }
    }
}

/// Sections that failed to parse and could not be moved aside. While such a file is still on
/// disk, saving defaults over it would destroy the only recoverable copy.
static UNQUARANTINED: LazyLock<Mutex<HashSet<PathBuf>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// Moves a file that failed to parse next to itself so it can be recovered by hand.
///
/// The target name carries the wall-clock second; the name is reserved with `create_new`
/// first so an existing `.corrupt-*` copy from an earlier corruption is never replaced
/// (Windows `rename` overwrites an existing destination). When the move itself fails the
/// section is registered as unquarantined, and [`write_json_atomic`] refuses to overwrite
/// it until the user moves or fixes the file.
fn quarantine_corrupt(path: &Path) -> Option<PathBuf> {
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let mut base = path.as_os_str().to_os_string();
    base.push(format!(".corrupt-{timestamp}"));

    for attempt in 0..100u32 {
        let mut name = base.clone();
        if attempt > 0 {
            name.push(format!("-{attempt}"));
        }
        let target = PathBuf::from(name);
        let reservation = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
        {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                crate::logging::error(
                    "CONFIG",
                    &format!(
                        "Failed to reserve a quarantine name for {}: {err}",
                        path.display()
                    ),
                );
                break;
            }
        };
        drop(reservation);
        match fs::rename(path, &target) {
            Ok(()) => return Some(target),
            Err(err) => {
                let _ = fs::remove_file(&target);
                crate::logging::error(
                    "CONFIG",
                    &format!(
                        "Failed to quarantine {}: {err}; the file is kept and will not be overwritten",
                        path.display()
                    ),
                );
                break;
            }
        }
    }

    if let Ok(mut unquarantined) = UNQUARANTINED.lock() {
        unquarantined.insert(path.to_path_buf());
    }
    None
}

/// Schema version stored in an existing section file, read without materialising the rest of
/// the section. `None` means the file is missing or its version cannot be determined, in
/// which case the normal read path decides (quarantine or defaults).
fn disk_version(path: &Path) -> Option<u32> {
    let raw = fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
}

/// Refuses a write that would downgrade a section written by a newer build.
///
/// [`normalize_version`] deliberately keeps a newer on-disk version instead of re-stamping it:
/// the unknown fields of that layout are not understood here, so persisting the older shape
/// would silently drop them. The file stays untouched and the condition is reported through
/// the returned error (and one log line) so callers can surface it.
fn refuse_newer_section(path: &Path) -> std::io::Result<()> {
    let Some(version) = disk_version(path) else {
        return Ok(());
    };
    if version <= CONFIG_VERSION {
        return Ok(());
    }
    crate::logging::error(
        "CONFIG",
        &format!(
            "Refusing to overwrite {} (schema version {version} is newer than {CONFIG_VERSION})",
            path.display()
        ),
    );
    Err(std::io::Error::other(format!(
        "{} uses schema version {version}, newer than this build's {CONFIG_VERSION}; refusing to overwrite it",
        path.display()
    )))
}

/// Refuses a write while a corrupt section could not be moved aside.
fn refuse_unquarantined_section(path: &Path) -> std::io::Result<()> {
    let blocked = UNQUARANTINED
        .lock()
        .is_ok_and(|unquarantined| unquarantined.contains(path));
    if !blocked {
        return Ok(());
    }
    Err(std::io::Error::other(format!(
        "{} could not be quarantined; refusing to overwrite the only recoverable copy",
        path.display()
    )))
}

/// Serializes to a sibling temp file, flushes it to disk and renames it over the
/// target, so readers never observe a half-written section.
///
/// Two invariants are enforced here, before anything on disk is touched: a section written by
/// a newer build is never overwritten, and a corrupt section that could not be quarantined
/// keeps its only recoverable copy.
fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    refuse_newer_section(path)?;
    refuse_unquarantined_section(path)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    let tmp = tmp_path(path);
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    // Antivirus and indexers can hold a fresh file open for a moment on Windows; a
    // short retry keeps a transient share violation from losing the write.
    let mut last_error = None;
    for attempt in 0..3 {
        match fs::rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(err) => last_error = Some(err),
        }
        if attempt < 2 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    let _ = fs::remove_file(&tmp);
    Err(last_error.unwrap_or_else(|| {
        std::io::Error::other("atomic rename failed")
    }))
}

/// Applies the read-time version policy. An older section is upgraded in memory and stamped
/// with [`CONFIG_VERSION`] on the next save; a section written by a newer build is readable
/// (unknown fields are not understood, so they stay out of memory) but its version is kept
/// as-is and [`write_json_atomic`] refuses to overwrite it. This is the one place the
/// no-downgrade invariant lives.
fn normalize_version(section: &str, version: &mut u32) {
    if *version > CONFIG_VERSION {
        crate::logging::warn(
            "CONFIG",
            &format!(
                "{section} has schema version {} (newer than {CONFIG_VERSION}); it is read-only for this build",
                *version
            ),
        );
        return;
    }
    if *version < CONFIG_VERSION {
        crate::logging::debug(
            "CONFIG",
            &format!("{section} upgraded from schema version {} to {CONFIG_VERSION}", *version),
        );
    }
    *version = CONFIG_VERSION;
}

// ---------------------------------------------------------------------------
// Read cache for hot sections
// ---------------------------------------------------------------------------

/// Identity of a file on disk. A cached section is reused while the file keeps the same
/// size and modification time; any write (our own or another process's) invalidates it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileStamp {
    modified: Option<std::time::SystemTime>,
    len: u64,
}

fn file_stamp(path: &Path) -> Option<FileStamp> {
    fs::metadata(path).ok().map(|metadata| FileStamp {
        modified: metadata.modified().ok(),
        len: metadata.len(),
    })
}

impl FileStamp {
    /// A stamp is trustworthy only once the timestamp behind it is old enough to have been
    /// recorded in full. A fresh file is re-read on every access until then, so a coarse
    /// timestamp cannot hide a rewrite and a cached default cannot pin a transient failure.
    fn is_settled(self, now: SystemTime) -> bool {
        self.modified.is_some_and(|modified| {
            now.duration_since(modified)
                .is_ok_and(|age| age >= STAMP_SETTLE)
        })
    }
}

struct CachedSection<T> {
    path: PathBuf,
    stamp: Option<FileStamp>,
    value: Arc<T>,
}

/// `settings.json` and `library.json` are read from hot paths (every action step, every
/// Lua `ph.overlay_*` call, overlay windows). The cache turns those reads into one
/// `stat` per call; a miss re-reads and re-parses the file.
static SETTINGS_CACHE: LazyLock<RwLock<Option<CachedSection<Settings>>>> =
    LazyLock::new(|| RwLock::new(None));
static LIBRARY_CACHE: LazyLock<RwLock<Option<CachedSection<Library>>>> =
    LazyLock::new(|| RwLock::new(None));

fn cached_section<T: Default>(
    cache: &RwLock<Option<CachedSection<T>>>,
    path: PathBuf,
    load: impl FnOnce(&Path) -> Option<T>,
) -> Arc<T> {
    let stamp = file_stamp(&path);
    let now = SystemTime::now();
    if let Ok(guard) = cache.read() {
        if let Some(entry) = guard.as_ref() {
            if entry.path == path
                && entry.stamp == stamp
                && stamp.is_some_and(|stamp| stamp.is_settled(now))
            {
                return Arc::clone(&entry.value);
            }
        }
    }
    let Some(loaded) = load(&path) else {
        // A failed read is never cached: one transient sharing or permission error would
        // otherwise pin defaults for the rest of the process lifetime.
        return Arc::new(T::default());
    };
    let value = Arc::new(loaded);
    if let Ok(mut guard) = cache.write() {
        *guard = Some(CachedSection {
            path,
            stamp,
            value: Arc::clone(&value),
        });
    }
    value
}

fn invalidate_section<T>(cache: &RwLock<Option<CachedSection<T>>>) {
    if let Ok(mut guard) = cache.write() {
        *guard = None;
    }
}

// ---------------------------------------------------------------------------
// Window geometry and theme
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WindowState {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub is_maximized: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            x: 100.0,
            y: 100.0,
            width: 1180.0,
            height: 760.0,
            is_maximized: false,
        }
    }
}

impl WindowState {
    pub fn sanitize(&mut self) {
        self.width = self.width.clamp(880.0, 3840.0);
        self.height = self.height.clamp(580.0, 2160.0);
        if self.x < -4000.0 || self.x > 8000.0 || self.y < -1000.0 || self.y > 5000.0 {
            self.x = 100.0;
            self.y = 100.0;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AppTheme {
    #[default]
    Dark,
    Light,
    Chocolate,
}

// ---------------------------------------------------------------------------
// settings.json
// ---------------------------------------------------------------------------

/// Rarely written host preferences: window geometry, tray, language, theme and
/// update behaviour.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tray_icon_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowState>,
    #[serde(default)]
    pub hide_tray_icon: bool,
    #[serde(default)]
    pub mask_daemon_process: bool,
    #[serde(default)]
    pub language: crate::i18n::Language,
    #[serde(default = "default_true")]
    pub check_updates_on_startup: bool,
    #[serde(default)]
    pub auto_install_updates: bool,
    #[serde(default)]
    pub theme: AppTheme,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            tray_icon_path: None,
            window: None,
            hide_tray_icon: false,
            mask_daemon_process: false,
            language: crate::i18n::Language::English,
            check_updates_on_startup: true,
            auto_install_updates: false,
            theme: AppTheme::Dark,
        }
    }
}

impl Settings {
    pub fn path_in(dir: &Path) -> PathBuf {
        section_path(dir, SETTINGS_FILE)
    }

    /// Reads `settings.json` from the live config directory without writing anything.
    /// Served from the process cache unless the file changed since the last read.
    pub fn read() -> Self {
        let path = Self::path_in(&AppConfig::config_home());
        (*cached_section(&SETTINGS_CACHE, path, Self::load_file)).clone()
    }

    /// Uncached read from an explicit directory; used by tests, `AppConfig::read_from`
    /// and one-shot tools where a stale cache would be surprising.
    pub fn read_from(dir: &Path) -> Self {
        Self::read_file(&Self::path_in(dir))
    }

    fn read_file(path: &Path) -> Self {
        Self::load_file(path).unwrap_or_default()
    }

    /// Parses one settings section. `None` means the read failed (missing, unreadable or
    /// unparsable); the cache must not turn that into a pinned default.
    fn load_file(path: &Path) -> Option<Self> {
        let mut settings = read_json::<Settings>(path)?;
        normalize_version(SETTINGS_FILE, &mut settings.version);
        if let Some(ws) = &mut settings.window {
            ws.sanitize();
        }
        Some(settings)
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&AppConfig::config_dir())
    }

    pub fn save_to(&self, dir: &Path) -> std::io::Result<()> {
        let mut snapshot = self.clone();
        snapshot.version = CONFIG_VERSION;
        let result = write_json_atomic(&Self::path_in(dir), &snapshot);
        if result.is_ok() {
            invalidate_section(&SETTINGS_CACHE);
        }
        result
    }

    /// Persists only the window geometry. Used by the GUI as a fallback when the
    /// daemon is not reachable over IPC.
    pub fn save_window_state(ws: &WindowState) {
        let mut settings = Self::read();
        settings.window = Some(ws.clone());
        if let Err(err) = settings.save() {
            crate::logging::error("CONFIG", &format!("Failed to save window state: {err}"));
        }
    }
}

// ---------------------------------------------------------------------------
// library.json
// ---------------------------------------------------------------------------

fn default_overlays() -> Vec<OverlayDefinition> {
    vec![
        OverlayDefinition::default_crosshair(),
        OverlayDefinition::default_macro_badge(),
        OverlayDefinition::default_telemetry(),
    ]
}

/// Overlay and macro presets: reusable templates referenced by rules.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Library {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default = "default_overlays")]
    pub overlays: Vec<OverlayDefinition>,
    #[serde(default = "crate::models::default_macro_presets")]
    pub macro_presets: Vec<MacroPreset>,
}

impl Default for Library {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            overlays: default_overlays(),
            macro_presets: crate::models::default_macro_presets(),
        }
    }
}

impl Library {
    pub fn path_in(dir: &Path) -> PathBuf {
        section_path(dir, LIBRARY_FILE)
    }

    /// Presets for hot paths: one cached `Arc` shared by action steps, Lua helpers and
    /// tray/overlay reads instead of re-reading and re-parsing the file every time.
    pub fn shared() -> Arc<Library> {
        let path = Self::path_in(&AppConfig::config_home());
        cached_section(&LIBRARY_CACHE, path, Self::load_file)
    }

    pub fn read() -> Self {
        (*Self::shared()).clone()
    }

    /// Uncached read from an explicit directory; used by tests and one-shot tools.
    pub fn read_from(dir: &Path) -> Self {
        Self::read_file(&Self::path_in(dir))
    }

    fn read_file(path: &Path) -> Self {
        Self::load_file(path).unwrap_or_default()
    }

    /// Parses one library section. `None` means the read failed and must not be cached.
    fn load_file(path: &Path) -> Option<Self> {
        let mut library = read_json::<Library>(path)?;
        normalize_version(LIBRARY_FILE, &mut library.version);
        Some(library)
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&AppConfig::config_dir())
    }

    pub fn save_to(&self, dir: &Path) -> std::io::Result<()> {
        let mut snapshot = self.clone();
        snapshot.version = CONFIG_VERSION;
        let result = write_json_atomic(&Self::path_in(dir), &snapshot);
        if result.is_ok() {
            invalidate_section(&LIBRARY_CACHE);
        }
        result
    }

    pub fn find_overlay(&self, id_or_name: &str) -> Option<&OverlayDefinition> {
        let needle = id_or_name.trim();
        if needle.is_empty() {
            return None;
        }
        self.overlays
            .iter()
            .find(|o| o.id == needle || o.name.eq_ignore_ascii_case(needle))
    }

    pub fn find_macro(&self, id_or_name: &str) -> Option<&MacroPreset> {
        let needle = id_or_name.trim();
        if needle.is_empty() {
            return None;
        }
        self.macro_presets
            .iter()
            .find(|p| p.id == needle || p.name.eq_ignore_ascii_case(needle))
    }
}

// ---------------------------------------------------------------------------
// rules.json
// ---------------------------------------------------------------------------

/// Rule definitions. Runtime counters are written as zero and ignored on read: the
/// only authoritative home for `execution_count`/`last_executed_at` is `runtime.json`,
/// so a rule edit never rewrites counters and a counter bump never rewrites definitions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RulesFile {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

impl Default for RulesFile {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            rules: Vec::new(),
        }
    }
}

impl RulesFile {
    pub fn path() -> PathBuf {
        Self::path_in(&AppConfig::config_dir())
    }

    pub fn path_in(dir: &Path) -> PathBuf {
        section_path(dir, RULES_FILE)
    }

    pub fn read_from(dir: &Path) -> Self {
        let path = Self::path_in(dir);
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(err) => {
                if err.kind() != std::io::ErrorKind::NotFound {
                    crate::logging::error(
                        "CONFIG",
                        &format!("Failed to read {}: {err}", path.display()),
                    );
                }
                return Self::default();
            }
        };
        let mut value: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(err) => {
                let quarantined = quarantine_corrupt(&path);
                let detail = match &quarantined {
                    Some(target) => format!("; moved aside to {}", target.display()),
                    None => String::new(),
                };
                crate::logging::error(
                    "CONFIG",
                    &format!("Failed to parse {}: {err}{detail}", path.display()),
                );
                return Self::default();
            }
        };
        // A typo in a hand-edited rule is not a corrupt section: drop only that rule so the
        // rest of the file keeps loading, and leave a visible error naming the rule and keys.
        if let Some(rules) = value.get_mut("rules").and_then(serde_json::Value::as_array_mut) {
            rules.retain(|rule| match crate::models::validate_rule_keys(rule) {
                Ok(()) => true,
                Err(unknown) => {
                    let id = rule
                        .get("id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("<no id>");
                    let details = unknown
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; ");
                    crate::logging::error(
                        "CONFIG",
                        &format!("rules.json: rule '{id}': {details}; skipping the rule"),
                    );
                    false
                }
            });
        }
        match serde_json::from_value::<RulesFile>(value) {
            Ok(mut rules) => {
                for rule in &mut rules.rules {
                    rule.normalize();
                }
                // Value checks are not a corrupt section either: a hand-edited rule the CLI and
                // the GUI both reject (an out-of-range custom quota window, an impossible wait,
                // a malformed schedule) is skipped on its own, named in the error log. The same
                // per-rule validator the GUI's Save path runs is used, so nothing that path
                // accepts is dropped here.
                rules.rules.retain(|rule| {
                    let errors = crate::cli::validate_rules(std::slice::from_ref(rule));
                    if errors.is_empty() {
                        return true;
                    }
                    crate::logging::error(
                        "CONFIG",
                        &format!(
                            "rules.json: rule '{}': {}; skipping the rule",
                            rule.id,
                            errors.join("; ")
                        ),
                    );
                    false
                });
                normalize_version(RULES_FILE, &mut rules.version);
                rules
            }
            Err(err) => {
                let quarantined = quarantine_corrupt(&path);
                let detail = match &quarantined {
                    Some(target) => format!("; moved aside to {}", target.display()),
                    None => String::new(),
                };
                crate::logging::error(
                    "CONFIG",
                    &format!("Failed to parse {}: {err}{detail}", path.display()),
                );
                Self::default()
            }
        }
    }

    pub fn save_rules(rules: &[Rule]) -> std::io::Result<()> {
        Self::save_rules_to(&AppConfig::config_dir(), rules)
    }

    pub fn save_rules_to(dir: &Path, rules: &[Rule]) -> std::io::Result<()> {
        let snapshot = RulesFile {
            version: CONFIG_VERSION,
            rules: rules.iter().map(rule_definition).collect(),
        };
        write_json_atomic(&Self::path_in(dir), &snapshot)
    }
}

/// Strips daemon-owned runtime counters from a rule before it is written to disk.
fn rule_definition(rule: &Rule) -> Rule {
    let mut definition = rule.clone();
    definition.clear_quota_runtime();
    definition
}

// ---------------------------------------------------------------------------
// runtime.json
// ---------------------------------------------------------------------------

/// One rule's daemon-owned execution counters.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleRuntime {
    #[serde(default)]
    pub execution_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_executed_at: Option<String>,
    /// Anchor of a `QuotaPeriod::Custom` window, local `%Y-%m-%d %H:%M:%S`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_window_start: Option<String>,
}

/// Hot runtime state. The daemon flushes it coalesced, so a fast-firing rule cannot
/// turn every execution into a disk write.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RuntimeState {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rules: BTreeMap<String, RuleRuntime>,
}

impl RuntimeState {
    pub fn path() -> PathBuf {
        Self::path_in(&AppConfig::config_dir())
    }

    pub fn path_in(dir: &Path) -> PathBuf {
        section_path(dir, RUNTIME_FILE)
    }

    pub fn read() -> Self {
        Self::read_from(&AppConfig::config_dir())
    }

    pub fn read_from(dir: &Path) -> Self {
        match read_json::<RuntimeState>(&Self::path_in(dir)) {
            Some(mut runtime) => {
                normalize_version(RUNTIME_FILE, &mut runtime.version);
                runtime
            }
            None => Self::default(),
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&AppConfig::config_dir())
    }

    pub fn save_to(&self, dir: &Path) -> std::io::Result<()> {
        let mut snapshot = self.clone();
        snapshot.version = CONFIG_VERSION;
        write_json_atomic(&Self::path_in(dir), &snapshot)
    }

    /// Snapshots the counters of every rule that has run at least once (including the
    /// anchor of a custom quota window, which only survives a restart through this file).
    pub fn capture(rules: &[Rule]) -> Self {
        let mut runtime = Self {
            version: CONFIG_VERSION,
            rules: BTreeMap::new(),
        };
        for rule in rules {
            if rule.execution_count == 0
                && rule.last_executed_at.is_none()
                && rule.quota_window_start.is_none()
            {
                continue;
            }
            runtime.rules.insert(
                rule.id.clone(),
                RuleRuntime {
                    execution_count: rule.execution_count,
                    last_executed_at: rule.last_executed_at.clone(),
                    quota_window_start: rule.quota_window_start.clone(),
                },
            );
        }
        runtime
    }

    /// Restores the counters captured by [`Self::capture`] onto freshly loaded rules.
    pub fn apply_to(&self, rules: &mut [Rule]) {
        for rule in rules {
            if let Some(entry) = self.rules.get(&rule.id) {
                rule.execution_count = entry.execution_count;
                rule.last_executed_at = entry.last_executed_at.clone();
                rule.quota_window_start = entry.quota_window_start.clone();
            }
        }
    }

    /// Drops a deleted rule's counters, and implements the CLI `--rules-reset`.
    pub fn forget(&mut self, rule_id: &str) {
        self.rules.remove(rule_id);
    }
}

// ---------------------------------------------------------------------------
// In-memory aggregate
// ---------------------------------------------------------------------------

/// In-memory view of the whole configuration used by the daemon, the GUI draft and
/// IPC. It is assembled from the four section files on load and never written as a
/// single file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    // settings.json
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tray_icon_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowState>,
    #[serde(default)]
    pub hide_tray_icon: bool,
    #[serde(default)]
    pub mask_daemon_process: bool,
    #[serde(default)]
    pub language: crate::i18n::Language,
    #[serde(default = "default_true")]
    pub check_updates_on_startup: bool,
    #[serde(default)]
    pub auto_install_updates: bool,
    #[serde(default)]
    pub theme: AppTheme,

    // rules.json
    #[serde(default)]
    pub rules: Vec<Rule>,

    // library.json
    #[serde(default = "default_overlays")]
    pub overlays: Vec<OverlayDefinition>,
    #[serde(default = "crate::models::default_macro_presets")]
    pub macro_presets: Vec<MacroPreset>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            tray_icon_path: None,
            window: None,
            hide_tray_icon: false,
            mask_daemon_process: false,
            language: crate::i18n::Language::English,
            check_updates_on_startup: true,
            auto_install_updates: false,
            theme: AppTheme::Dark,
            rules: Vec::new(),
            overlays: default_overlays(),
            macro_presets: crate::models::default_macro_presets(),
        }
    }
}

impl AppConfig {
    /// Config directory without filesystem side effects. Hot readers go through this so a
    /// cache hit costs one `stat` rather than a `create_dir_all` + read + parse.
    fn config_home() -> PathBuf {
        // Unit tests must never touch the real user configuration: engine tests exercise
        // persistence paths, so they get an isolated per-process directory.
        if cfg!(test) {
            return std::env::temp_dir().join(format!("Synector-tests-{}", std::process::id()));
        }
        // Portable mode, QA and scripted end-to-end tests can relocate the whole config
        // directory; nothing else in the process caches the path.
        if let Some(custom) = std::env::var_os("SYNECTOR_CONFIG_DIR") {
            return PathBuf::from(custom);
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(local).join("Synector");
        }
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            return PathBuf::from(profile).join("AppData").join("Local").join("Synector");
        }
        PathBuf::from("Synector")
    }

    pub fn config_dir() -> PathBuf {
        let dir = Self::config_home();
        let _ = fs::create_dir_all(&dir);
        dir
    }

    pub fn logs_dir() -> PathBuf {
        Self::config_dir().join("logs")
    }

    /// Daemon-side load: materialises missing section files and performs the
    /// first-run autostart registration.
    pub fn load() -> Self {
        Self::load_from(&Self::config_dir())
    }

    /// Daemon-side load from an explicit directory.
    ///
    /// Only section files that are actually missing are created, and each one is written from
    /// the state loaded out of the other sections. Rewriting an existing `runtime.json` with
    /// defaults on a `settings.json`-only first run would zero counters that are already
    /// applied in memory, so a missing or quarantined settings file never touches it.
    fn load_from(dir: &Path) -> Self {
        clean_stale_temp_files(dir);

        let settings_path = Settings::path_in(dir);
        let first_run = !settings_path.exists();
        let config = Self::read_from(dir);

        // Re-check after the read: a corrupt section was moved aside by then, and the fresh
        // default file still has to be materialised.
        if !settings_path.exists() {
            log_section_create_failure(SETTINGS_FILE, config.settings().save_to(dir));
        }
        if !RulesFile::path_in(dir).exists() {
            log_section_create_failure(RULES_FILE, RulesFile::save_rules_to(dir, &config.rules));
        }
        if !Library::path_in(dir).exists() {
            log_section_create_failure(LIBRARY_FILE, config.library().save_to(dir));
        }
        if !RuntimeState::path_in(dir).exists() {
            log_section_create_failure(RUNTIME_FILE, config.runtime_state().save_to(dir));
        }

        #[cfg(windows)]
        if first_run
            && !cfg!(test)
            && !crate::autorun::is_autorun_enabled(crate::autorun::APP_NAME)
        {
            let _ = crate::autorun::set_autorun_with_mask(
                crate::autorun::APP_NAME,
                true,
                config.mask_daemon_process,
            );
        }

        config
    }

    /// Read-only load used by the CLI and by read-only tools: the config directory and the
    /// section files are not created, and no autostart entry is registered.
    pub fn read() -> Self {
        Self::read_from(&Self::config_home())
    }

    pub fn read_from(dir: &Path) -> Self {
        let settings = Settings::read_from(dir);
        let library = Library::read_from(dir);
        let mut rules = RulesFile::read_from(dir).rules;
        RuntimeState::read_from(dir).apply_to(&mut rules);
        let window = settings.window.clone();
        Self {
            tray_icon_path: settings.tray_icon_path,
            window,
            hide_tray_icon: settings.hide_tray_icon,
            mask_daemon_process: settings.mask_daemon_process,
            language: settings.language,
            check_updates_on_startup: settings.check_updates_on_startup,
            auto_install_updates: settings.auto_install_updates,
            theme: settings.theme,
            rules,
            overlays: library.overlays,
            macro_presets: library.macro_presets,
        }
    }

    /// Projects the settings half of the aggregate into its own section.
    pub fn settings(&self) -> Settings {
        Settings {
            version: CONFIG_VERSION,
            tray_icon_path: self.tray_icon_path.clone(),
            window: self.window.clone(),
            hide_tray_icon: self.hide_tray_icon,
            mask_daemon_process: self.mask_daemon_process,
            language: self.language,
            check_updates_on_startup: self.check_updates_on_startup,
            auto_install_updates: self.auto_install_updates,
            theme: self.theme,
        }
    }

    /// Replaces the settings half of the aggregate with `settings`, keeping rules and
    /// presets untouched.
    pub fn apply_settings(&mut self, settings: Settings) {
        self.tray_icon_path = settings.tray_icon_path;
        self.window = settings.window;
        self.hide_tray_icon = settings.hide_tray_icon;
        self.mask_daemon_process = settings.mask_daemon_process;
        self.language = settings.language;
        self.check_updates_on_startup = settings.check_updates_on_startup;
        self.auto_install_updates = settings.auto_install_updates;
        self.theme = settings.theme;
    }

    /// Projects the presets half of the aggregate into its own section.
    pub fn library(&self) -> Library {
        Library {
            version: CONFIG_VERSION,
            overlays: self.overlays.clone(),
            macro_presets: self.macro_presets.clone(),
        }
    }

    pub fn save_settings(&self) -> std::io::Result<()> {
        self.settings().save()
    }

    pub fn save_rules(&self) -> std::io::Result<()> {
        RulesFile::save_rules(&self.rules)
    }

    pub fn save_library(&self) -> std::io::Result<()> {
        self.library().save()
    }

    pub fn runtime_state(&self) -> RuntimeState {
        RuntimeState::capture(&self.rules)
    }

    pub fn find_overlay(&self, id_or_name: &str) -> Option<&OverlayDefinition> {
        let needle = id_or_name.trim();
        if needle.is_empty() {
            return None;
        }
        self.overlays
            .iter()
            .find(|o| o.id == needle || o.name.eq_ignore_ascii_case(needle))
    }

    pub fn find_macro(&self, id_or_name: &str) -> Option<&MacroPreset> {
        let needle = id_or_name.trim();
        if needle.is_empty() {
            return None;
        }
        self.macro_presets
            .iter()
            .find(|p| p.id == needle || p.name.eq_ignore_ascii_case(needle))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "synector-config-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn rule(id: &str, name: &str) -> Rule {
        Rule {
            id: id.to_string(),
            name: name.to_string(),
            ..Default::default()
        }
    }

    fn stray_temp_files(dir: &Path) -> usize {
        fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok())
                    .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "tmp"))
                    .count()
            })
            .unwrap_or(0)
    }

    #[test]
    fn settings_round_trip_stamps_current_version() {
        let dir = temp_dir("settings");
        let mut settings = Settings {
            window: Some(WindowState {
                x: 10.0,
                y: 20.0,
                width: 1280.0,
                height: 720.0,
                is_maximized: true,
            }),
            language: crate::i18n::Language::Russian,
            theme: AppTheme::Chocolate,
            hide_tray_icon: true,
            check_updates_on_startup: false,
            ..Default::default()
        };
        settings.version = 0;
        settings.save_to(&dir).expect("save settings");

        let decoded = Settings::read_from(&dir);
        assert_eq!(decoded.version, CONFIG_VERSION);
        assert_eq!(decoded.language, crate::i18n::Language::Russian);
        assert_eq!(decoded.theme, AppTheme::Chocolate);
        assert!(decoded.hide_tray_icon);
        assert!(!decoded.check_updates_on_startup);
        assert_eq!(decoded.window.as_ref().map(|w| w.is_maximized), Some(true));
        assert_eq!(stray_temp_files(&dir), 0);
    }

    #[test]
    fn window_state_is_sanitized_on_read() {
        let dir = temp_dir("window");
        let settings = Settings {
            window: Some(WindowState {
                x: -9000.0,
                y: 20.0,
                width: 100.0,
                height: 90.0,
                is_maximized: false,
            }),
            ..Default::default()
        };
        settings.save_to(&dir).expect("save settings");

        let window = Settings::read_from(&dir).window.expect("window");
        assert_eq!(window.x, 100.0);
        assert_eq!(window.width, 880.0);
        assert_eq!(window.height, 580.0);
    }

    #[test]
    fn missing_sections_fall_back_to_defaults() {
        let dir = temp_dir("empty");
        let config = AppConfig::read_from(&dir);
        assert!(config.rules.is_empty());
        assert_eq!(config.overlays.len(), 3);
        assert_eq!(config.macro_presets.len(), 2);
        assert!(config.find_overlay("overlay_crosshair").is_some());
        assert!(config.find_overlay("Gaming Crosshair").is_some());
        assert!(config.find_macro("macro_spam_click").is_some());
        assert!(config.find_macro("Rapid Clicker").is_some());
        assert_eq!(config.language, crate::i18n::Language::English);
        assert_eq!(config.theme, AppTheme::Dark);
        assert!(config.check_updates_on_startup);
        assert!(!Settings::path_in(&dir).exists());
    }

    #[test]
    fn library_round_trip_and_lookup() {
        let dir = temp_dir("library");
        let mut library = Library::default();
        library.overlays.clear();
        library.overlays.push(OverlayDefinition::default_crosshair());
        library.macro_presets.clear();
        library.save_to(&dir).expect("save library");

        let decoded = Library::read_from(&dir);
        assert_eq!(decoded.version, CONFIG_VERSION);
        assert_eq!(decoded.overlays.len(), 1);
        assert!(decoded.macro_presets.is_empty());
        assert!(decoded.find_overlay("overlay_crosshair").is_some());
        assert!(decoded.find_macro("anything").is_none());
    }

    #[test]
    fn rules_file_strips_runtime_counters() {
        let dir = temp_dir("rules");
        let mut stored = rule("rule_a", "A");
        stored.execution_count = 42;
        stored.last_executed_at = Some("2026-01-01 00:00:00".to_string());
        stored.quota_window_start = Some("2026-01-01 00:00:00".to_string());
        stored.is_enabled = false;

        RulesFile::save_rules_to(&dir, &[stored]).expect("save rules");

        let decoded = RulesFile::read_from(&dir);
        assert_eq!(decoded.version, CONFIG_VERSION);
        assert_eq!(decoded.rules.len(), 1);
        assert_eq!(decoded.rules[0].id, "rule_a");
        assert_eq!(decoded.rules[0].execution_count, 0);
        assert!(decoded.rules[0].last_executed_at.is_none());
        assert!(decoded.rules[0].quota_window_start.is_none());
        assert!(!decoded.rules[0].is_enabled);

        let raw = fs::read_to_string(RulesFile::path_in(&dir)).expect("rules json");
        assert!(!raw.contains("last_executed_at"), "runtime counter leaked: {raw}");
        assert!(!raw.contains("quota_window_start"), "quota anchor leaked: {raw}");
        assert!(raw.contains("\"version\": 1"));
    }

    #[test]
    fn rules_file_skips_rules_with_unknown_keys() {
        let dir = temp_dir("rules-unknown");
        let path = RulesFile::path_in(&dir);
        let raw = r#"{
            "version": 1,
            "rules": [
                {"id": "rule_good", "name": "Good"},
                {"id": "rule_typo", "name": "Typo", "max_execution": 5}
            ]
        }"#;
        fs::write(&path, raw).expect("write rules");

        let decoded = RulesFile::read_from(&dir);
        assert_eq!(decoded.rules.len(), 1);
        assert_eq!(decoded.rules[0].id, "rule_good");
    }

    /// U12-002 residual: a hand-edited rule with a custom quota window the CLI/GUI reject is
    /// skipped on its own, like an unknown key, instead of loading and being silently clamped.
    #[test]
    fn rules_file_skips_rules_with_invalid_values() {
        let dir = temp_dir("rules-invalid-values");
        let path = RulesFile::path_in(&dir);
        let raw = r#"{
            "version": 1,
            "rules": [
                {"id": "rule_good", "name": "Good"},
                {
                    "id": "rule_bad_window",
                    "name": "Bad Window",
                    "max_executions": 5,
                    "quota_period": {"custom": {"value": 100001, "unit": "hours"}}
                }
            ]
        }"#;
        fs::write(&path, raw).expect("write rules");

        let decoded = RulesFile::read_from(&dir);
        assert_eq!(decoded.rules.len(), 1);
        assert_eq!(decoded.rules[0].id, "rule_good");
    }

    /// The load-time check uses the same per-rule validator as the GUI's Save path, so every
    /// rule that path accepts survives a write/read round trip unchanged (boundary values
    /// included).
    #[test]
    fn rules_file_keeps_valid_rules_after_load_validation() {
        let dir = temp_dir("rules-valid-roundtrip");
        let mut stored = rule("rule_window", "Window Rule");
        stored.max_executions = Some(2);
        stored.quota_period = Some(crate::models::QuotaPeriod::Custom {
            value: crate::models::QuotaPeriod::MAX_CUSTOM_WINDOW_UNITS,
            unit: crate::models::QuotaUnit::Days,
        });
        stored.actions = vec![crate::models::Action::Wait {
            mode: crate::models::WaitMode::Timer,
            seconds: 86_400.0,
            condition: None,
            timeout_sec: 86_400.0,
            timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
        }];
        RulesFile::save_rules_to(&dir, std::slice::from_ref(&stored)).expect("save rules");

        let decoded = RulesFile::read_from(&dir);
        assert_eq!(
            decoded.rules.len(),
            1,
            "a rule the CLI/GUI accept must not be dropped on load"
        );
        assert_eq!(decoded.rules[0], stored, "the loaded rule must be unchanged");
    }

    #[test]
    fn rules_file_normalizes_overlay_ids_on_load() {
        let dir = temp_dir("rules-overlay-ids");
        let path = RulesFile::path_in(&dir);
        let raw = r#"{"version":1,"rules":[{"id":"r","name":"R","actions":[{"$action_type":"overlay","overlay_id":"outside","custom_overlay":{"id":"inside","name":"HUD"},"operation":"show"}]}]}"#;
        fs::write(&path, raw).expect("write rules");

        let decoded = RulesFile::read_from(&dir);
        match &decoded.rules[0].actions[0] {
            crate::models::Action::Overlay {
                overlay_id,
                custom_overlay,
                ..
            } => {
                assert_eq!(overlay_id, "outside");
                assert_eq!(
                    custom_overlay.as_ref().map(|definition| definition.id.as_str()),
                    Some("outside")
                );
            }
            other => panic!("expected an overlay action, got {other:?}"),
        }
    }

    #[test]
    fn runtime_state_capture_and_apply() {
        let mut first = rule("rule_a", "A");
        first.execution_count = 7;
        first.last_executed_at = Some("2026-02-03 04:05:06".to_string());
        first.quota_window_start = Some("2026-02-03 01:00:00".to_string());
        let untouched = rule("rule_b", "B");

        let runtime = RuntimeState::capture(&[first, untouched]);
        assert_eq!(runtime.rules.len(), 1);
        assert_eq!(runtime.rules["rule_a"].execution_count, 7);
        assert_eq!(
            runtime.rules["rule_a"].quota_window_start.as_deref(),
            Some("2026-02-03 01:00:00")
        );

        let mut loaded = vec![rule("rule_a", "A"), rule("rule_b", "B")];
        runtime.apply_to(&mut loaded);
        assert_eq!(loaded[0].execution_count, 7);
        assert_eq!(
            loaded[0].last_executed_at.as_deref(),
            Some("2026-02-03 04:05:06")
        );
        assert_eq!(
            loaded[0].quota_window_start.as_deref(),
            Some("2026-02-03 01:00:00")
        );
        assert_eq!(loaded[1].execution_count, 0);
    }

    #[test]
    fn runtime_state_round_trip_and_forget() {
        let dir = temp_dir("runtime");
        let mut runtime = RuntimeState::default();
        runtime.rules.insert(
            "rule_a".to_string(),
            RuleRuntime {
                execution_count: 3,
                last_executed_at: None,
                quota_window_start: Some("2026-02-03 01:00:00".to_string()),
            },
        );
        runtime.save_to(&dir).expect("save runtime");

        let mut decoded = RuntimeState::read_from(&dir);
        assert_eq!(decoded.version, CONFIG_VERSION);
        assert_eq!(
            decoded.rules["rule_a"].quota_window_start.as_deref(),
            Some("2026-02-03 01:00:00")
        );
        assert_eq!(decoded.rules["rule_a"].execution_count, 3);
        decoded.forget("rule_a");
        assert!(decoded.rules.is_empty());
        decoded.forget("missing");
    }

    #[test]
    fn app_config_read_merges_every_section() {
        let dir = temp_dir("merge");
        Settings {
            language: crate::i18n::Language::Russian,
            ..Default::default()
        }
        .save_to(&dir)
        .expect("save settings");
        RulesFile::save_rules_to(&dir, &[rule("rule_a", "A")]).expect("save rules");

        let mut runtime = RuntimeState::default();
        runtime.rules.insert(
            "rule_a".to_string(),
            RuleRuntime {
                execution_count: 11,
                last_executed_at: Some("2026-01-01 00:00:00".to_string()),
                quota_window_start: None,
            },
        );
        runtime.save_to(&dir).expect("save runtime");

        let mut library = Library::default();
        library.overlays.clear();
        library
            .overlays
            .push(OverlayDefinition::default_telemetry());
        library.macro_presets.clear();
        library.save_to(&dir).expect("save library");

        let config = AppConfig::read_from(&dir);
        assert_eq!(config.language, crate::i18n::Language::Russian);
        assert_eq!(config.rules.len(), 1);
        assert_eq!(config.rules[0].execution_count, 11);
        assert_eq!(
            config.rules[0].last_executed_at.as_deref(),
            Some("2026-01-01 00:00:00")
        );
        assert_eq!(config.overlays.len(), 1);
        assert!(config.macro_presets.is_empty());
        assert_eq!(config.settings().language, crate::i18n::Language::Russian);
        assert_eq!(config.library().overlays.len(), 1);
    }

    #[test]
    fn app_config_settings_projection_round_trips_every_field() {
        let settings = Settings {
            version: CONFIG_VERSION,
            tray_icon_path: Some("C:\\icon.ico".to_string()),
            window: Some(WindowState {
                x: 11.0,
                y: 22.0,
                width: 1280.0,
                height: 720.0,
                is_maximized: true,
            }),
            hide_tray_icon: true,
            mask_daemon_process: true,
            language: crate::i18n::Language::Russian,
            check_updates_on_startup: false,
            auto_install_updates: true,
            theme: AppTheme::Chocolate,
        };
        let mut config = AppConfig::default();
        config.apply_settings(settings.clone());
        assert_eq!(config.settings(), settings, "every setting must survive the projection");

        let library = config.library();
        assert_eq!(library.overlays, config.overlays);
        assert_eq!(library.macro_presets, config.macro_presets);
        assert_eq!(library.version, CONFIG_VERSION);
    }

    fn backdate_file(path: &Path, age: Duration) {
        let modified = SystemTime::now() - age;
        fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("open file to backdate")
            .set_times(fs::FileTimes::new().set_modified(modified))
            .expect("set file modification time");
    }

    #[test]
    fn library_cache_shared_until_saved() {
        let dir = temp_dir("library-cache");
        let path = Library::path_in(&dir);
        let cache = RwLock::new(None);
        let empty = Library {
            overlays: Vec::new(),
            macro_presets: Vec::new(),
            ..Default::default()
        };
        fs::write(&path, serde_json::to_vec(&empty).expect("encode")).expect("write library");
        // A fresh timestamp makes the cache re-read on purpose; backdate it so this test
        // observes the steady-state reuse instead.
        backdate_file(&path, Duration::from_secs(5));

        let first = cached_section(&cache, path.clone(), Library::load_file);
        let second = cached_section(&cache, path.clone(), Library::load_file);
        assert!(Arc::ptr_eq(&first, &second), "unchanged file must reuse the cached value");
        assert!(first.macro_presets.is_empty());

        let preset = Library {
            overlays: Vec::new(),
            macro_presets: crate::models::default_macro_presets(),
            ..Default::default()
        };
        fs::write(&path, serde_json::to_vec(&preset).expect("encode")).expect("rewrite library");
        invalidate_section(&cache);
        let third = cached_section(&cache, path.clone(), Library::load_file);
        assert!(!Arc::ptr_eq(&second, &third));
        assert_eq!(third.macro_presets.len(), 2);

        let missing = dir.join("missing.json");
        let fallback = cached_section(&cache, missing, Library::load_file);
        assert_eq!(fallback.version, CONFIG_VERSION);
    }

    #[test]
    fn cache_does_not_pin_defaults_after_a_read_error() {
        let dir = temp_dir("cache-read-error");
        let path = Settings::path_in(&dir);
        // A directory in place of the file makes the read fail deterministically.
        fs::create_dir_all(&path).expect("create obstacle directory");
        let cache = RwLock::new(None);

        let failed = cached_section(&cache, path.clone(), Settings::load_file);
        assert_eq!(failed.theme, AppTheme::Dark);
        assert!(
            cache.read().expect("cache lock").is_none(),
            "a failed read must not be cached"
        );

        fs::remove_dir_all(&path).expect("remove obstacle directory");
        Settings {
            theme: AppTheme::Light,
            ..Settings::default()
        }
        .save_to(&dir)
        .expect("save recovered settings");

        let loaded = cached_section(&cache, path, Settings::load_file);
        assert_eq!(
            loaded.theme,
            AppTheme::Light,
            "the next call must observe the recovered file, not the transient default"
        );
    }

    #[test]
    fn cache_rechecks_a_recent_timestamp() {
        let dir = temp_dir("cache-recent");
        let path = Settings::path_in(&dir);
        let cache = RwLock::new(None);
        let stamp = SystemTime::now();

        Settings {
            theme: AppTheme::Light,
            ..Settings::default()
        }
        .save_to(&dir)
        .expect("save first settings");
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open settings")
            .set_times(fs::FileTimes::new().set_modified(stamp))
            .expect("stamp settings");
        let file_len = fs::metadata(&path).expect("metadata").len() as usize;
        let cached = cached_section(&cache, path.clone(), Settings::load_file);
        assert_eq!(cached.theme, AppTheme::Light);

        // Same size (pad the shorter `dark` value with JSON whitespace) and exactly the same
        // timestamp: only the settle rule prevents a stale hit, which is the coarse-timestamp
        // case the cache has to tolerate.
        let second = Settings {
            theme: AppTheme::Dark,
            ..Settings::default()
        };
        let mut same_size = serde_json::to_vec(&second).expect("encode settings");
        same_size.resize(file_len, b' ');
        fs::write(&path, &same_size).expect("rewrite settings");
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open rewritten settings")
            .set_times(fs::FileTimes::new().set_modified(stamp))
            .expect("restamp settings");

        let reloaded = cached_section(&cache, path, Settings::load_file);
        assert_eq!(
            reloaded.theme,
            AppTheme::Dark,
            "a rewrite with an unchanged stamp must still be observed"
        );
    }

    #[test]
    fn corrupt_section_is_quarantined_and_replaced() {
        let dir = temp_dir("corrupt");
        let path = RulesFile::path_in(&dir);
        let invalid = "{ this is not json";
        fs::write(&path, invalid).expect("write corrupt rules");

        let rules = RulesFile::read_from(&dir);
        assert!(rules.rules.is_empty(), "corrupt file falls back to defaults");
        assert!(!path.exists(), "corrupt file must be moved aside, not left in place");

        let quarantined: Vec<PathBuf> = fs::read_dir(&dir)
            .expect("read dir")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|entry| {
                entry
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("rules.json.corrupt-"))
            })
            .collect();
        assert_eq!(quarantined.len(), 1, "exactly one quarantined copy");
        assert_eq!(
            fs::read_to_string(&quarantined[0]).expect("read quarantined"),
            invalid
        );

        RulesFile::save_rules_to(&dir, &[rule("rule_after_corruption", "Recovered")])
            .expect("save rules");
        let reloaded = RulesFile::read_from(&dir);
        assert_eq!(reloaded.rules.len(), 1);
        assert_eq!(reloaded.rules[0].id, "rule_after_corruption");
    }

    #[test]
    fn atomic_write_replaces_existing_file() {
        let dir = temp_dir("atomic");
        let first = Settings::default();
        first.save_to(&dir).expect("first save");
        let mut second = first.clone();
        second.theme = AppTheme::Light;
        second.save_to(&dir).expect("second save");

        assert_eq!(Settings::read_from(&dir).theme, AppTheme::Light);
        assert_eq!(stray_temp_files(&dir), 0, "temp file must be renamed away");
    }

    #[test]
    fn newer_schema_version_is_readable_but_never_saved() {
        let dir = temp_dir("version");
        let stamp = CONFIG_VERSION + 7;
        let raw = format!("{{\"version\": {stamp}, \"language\": \"russian\"}}");
        let path = Settings::path_in(&dir);
        fs::write(&path, &raw).expect("write settings");

        let mut settings = Settings::read_from(&dir);
        assert_eq!(settings.version, stamp, "a newer version must not be re-stamped");
        assert_eq!(settings.language, crate::i18n::Language::Russian);

        settings.theme = AppTheme::Light;
        let error = settings
            .save_to(&dir)
            .expect_err("a section from a newer build must not be overwritten");
        assert!(error.to_string().contains("newer"), "{error}");
        assert_eq!(
            fs::read_to_string(&path).expect("read settings"),
            raw,
            "the newer file must stay byte-for-byte untouched"
        );

        let runtime_path = RuntimeState::path_in(&dir);
        let runtime_raw = format!(
            "{{\"version\": {stamp}, \"rules\": {{\"rule_a\": {{\"execution_count\": 42}}}}}}"
        );
        fs::write(&runtime_path, &runtime_raw).expect("write runtime");
        assert!(
            RuntimeState::default().save_to(&dir).is_err(),
            "a newer runtime section must not be overwritten either"
        );
        assert_eq!(
            fs::read_to_string(&runtime_path).expect("read runtime"),
            runtime_raw
        );
    }

    #[test]
    fn missing_settings_does_not_reset_runtime_counters() {
        let dir = temp_dir("first-run-counters");
        RulesFile::save_rules_to(&dir, &[rule("rule_a", "A")]).expect("save rules");
        let rules_before =
            fs::read_to_string(RulesFile::path_in(&dir)).expect("read rules before load");

        let mut runtime = RuntimeState::default();
        runtime.rules.insert(
            "rule_a".to_string(),
            RuleRuntime {
                execution_count: 9,
                last_executed_at: Some("2026-02-03 04:05:06".to_string()),
                quota_window_start: Some("2026-02-03 01:00:00".to_string()),
            },
        );
        runtime.save_to(&dir).expect("save runtime");

        let config = AppConfig::load_from(&dir);
        assert_eq!(config.rules.len(), 1);
        assert_eq!(config.rules[0].execution_count, 9);
        assert!(
            Settings::path_in(&dir).exists(),
            "the missing settings section must be created"
        );
        assert_eq!(
            fs::read_to_string(RulesFile::path_in(&dir)).expect("read rules after load"),
            rules_before,
            "an existing rules section must not be rewritten"
        );

        let stored = RuntimeState::read_from(&dir);
        assert_eq!(
            stored.rules["rule_a"].execution_count, 9,
            "runtime counters must survive a settings-only first run"
        );
        assert_eq!(
            stored.rules["rule_a"].last_executed_at.as_deref(),
            Some("2026-02-03 04:05:06")
        );
        assert_eq!(
            stored.rules["rule_a"].quota_window_start.as_deref(),
            Some("2026-02-03 01:00:00")
        );
    }

    #[test]
    fn stale_temp_files_are_swept_on_load() {
        let dir = temp_dir("stale-temp");
        let stale = dir.join(format!("settings.json.{}.tmp", std::process::id()));
        fs::write(&stale, "half written").expect("write stale temp");
        backdate_file(&stale, Duration::from_secs(10 * 60));
        let fresh = dir.join("rules.json.999999.tmp");
        fs::write(&fresh, "a live writer").expect("write fresh temp");
        let foreign = dir.join("notes.tmp");
        fs::write(&foreign, "keep me").expect("write foreign file");

        AppConfig::load_from(&dir);

        assert!(!stale.exists(), "a stale section temp file must be removed");
        assert!(fresh.exists(), "a fresh temp file may belong to a live writer");
        assert!(foreign.exists(), "unrelated files must not be touched");
    }

    #[test]
    fn quarantine_never_replaces_an_existing_copy() {
        let dir = temp_dir("quarantine-collision");
        let path = RulesFile::path_in(&dir);
        let invalid = "{ this is not json";
        fs::write(&path, invalid).expect("write corrupt rules");

        let second = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock after the epoch")
            .as_secs();
        let mut sentinels = Vec::new();
        for timestamp in [second.saturating_sub(1), second, second + 1] {
            let sentinel = dir.join(format!("rules.json.corrupt-{timestamp}"));
            let content = format!("sentinel {timestamp}");
            fs::write(&sentinel, &content).expect("write existing quarantine copy");
            sentinels.push((sentinel, content));
        }

        let rules = RulesFile::read_from(&dir);
        assert!(rules.rules.is_empty());
        for (sentinel, content) in &sentinels {
            assert!(
                sentinel.exists(),
                "an existing quarantine copy must never be replaced: {}",
                sentinel.display()
            );
            assert_eq!(
                &fs::read_to_string(sentinel).expect("read existing quarantine copy"),
                content
            );
        }

        let copies: Vec<PathBuf> = fs::read_dir(&dir)
            .expect("read dir")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|entry| {
                entry
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("rules.json.corrupt-"))
            })
            .collect();
        assert_eq!(
            copies.len(),
            sentinels.len() + 1,
            "the corrupt file must be quarantined under a free name"
        );
        let new_copy = copies
            .iter()
            .find(|copy| !sentinels.iter().any(|(sentinel, _)| sentinel == *copy))
            .expect("the new quarantine copy");
        assert_eq!(
            fs::read_to_string(new_copy).expect("read new quarantine copy"),
            invalid
        );
    }

    #[test]
    fn app_theme_serde_and_default() {
        assert_eq!(AppTheme::default(), AppTheme::Dark);
        assert_eq!(serde_json::to_string(&AppTheme::Dark).unwrap(), "\"dark\"");
        assert_eq!(serde_json::to_string(&AppTheme::Light).unwrap(), "\"light\"");
        assert_eq!(
            serde_json::to_string(&AppTheme::Chocolate).unwrap(),
            "\"chocolate\""
        );
        assert_eq!(
            serde_json::from_str::<AppTheme>("\"chocolate\"").unwrap(),
            AppTheme::Chocolate
        );
    }

    #[test]
    fn app_config_default_presets() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.overlays.len(), 3);
        assert_eq!(cfg.macro_presets.len(), 2);
        assert!(cfg.find_overlay("Gaming Crosshair").is_some());
        assert!(cfg.find_macro("Rapid Clicker").is_some());
        assert!(cfg.find_overlay("  ").is_none());
    }
}
