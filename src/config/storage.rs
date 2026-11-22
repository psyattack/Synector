//! Section file layout and the shared atomic write path.
//!
//! Each config section is a versioned JSON file in the config directory. Reads quarantine
//! an unparsable file and fall back to defaults; writes refuse to downgrade a newer layout
//! or overwrite a corrupt file that could not be moved aside, and land atomically through a
//! sibling temp file.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime};

use serde::de::DeserializeOwned;
use serde::Serialize;

use super::CONFIG_VERSION;

pub(super) const SETTINGS_FILE: &str = "settings.json";
pub(super) const RULES_FILE: &str = "rules.json";
pub(super) const LIBRARY_FILE: &str = "library.json";
pub(super) const RUNTIME_FILE: &str = "runtime.json";
const SECTION_FILES: [&str; 4] = [SETTINGS_FILE, RULES_FILE, LIBRARY_FILE, RUNTIME_FILE];

/// A cached file stamp is trusted only after the filesystem timestamp has settled. Coarse
/// timestamp volumes (FAT/exFAT, portable mode) can otherwise hide an equal-length rewrite
/// that happened within the same timestamp tick.
pub(super) const STAMP_SETTLE: Duration = Duration::from_secs(2);

/// `<section>.<pid>.tmp` files older than this were left behind by a writer that died between
/// the temp-file write and the rename; younger files may belong to a live writer.
const STALE_TEMP_AGE: Duration = Duration::from_secs(5 * 60);

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
pub(super) fn clean_stale_temp_files(dir: &Path) {
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

pub(super) fn log_section_create_failure(section: &str, result: std::io::Result<()>) {
    if let Err(err) = result {
        crate::logging::error("CONFIG", &format!("Failed to create {section}: {err}"));
    }
}

/// Reads the raw text of one section. A missing file is not an error: the caller falls back
/// to defaults so a partially deleted config directory still starts up.
fn read_raw(path: &Path) -> Option<String> {
    match fs::read_to_string(path) {
        Ok(raw) => Some(raw),
        Err(err) => {
            if err.kind() != std::io::ErrorKind::NotFound {
                crate::logging::error(
                    "CONFIG",
                    &format!("Failed to read {}: {err}", path.display()),
                );
            }
            None
        }
    }
}

/// Moves an unparsable section aside (see [`quarantine_corrupt`]) and reports the reason,
/// naming the quarantine copy when one could be made.
fn log_parse_failure(path: &Path, err: &impl std::fmt::Display) {
    let quarantined = quarantine_corrupt(path);
    let detail = match &quarantined {
        Some(target) => format!("; moved aside to {}", target.display()),
        None => String::new(),
    };
    crate::logging::error(
        "CONFIG",
        &format!("Failed to parse {}: {err}{detail}", path.display()),
    );
}

/// Reads one JSON section. A missing file is not an error: the caller falls back to
/// defaults so a partially deleted config directory still starts up. An unparsable file
/// is moved aside as `<name>.corrupt-<unix time>` before defaults take over, so the next
/// save cannot silently destroy recoverable user data.
fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let raw = read_raw(path)?;
    match serde_json::from_str(&raw) {
        Ok(value) => Some(value),
        Err(err) => {
            log_parse_failure(path, &err);
            None
        }
    }
}

/// Reads one JSON section through a `serde_json::Value`, so `filter` can drop individual
/// entries (a hand-edited rule with unknown keys) before the section is materialised.
/// Read failures and quarantine behaviour are the same as [`read_json`].
pub(super) fn read_json_filtered<T: DeserializeOwned>(
    path: &Path,
    filter: impl FnOnce(&mut serde_json::Value),
) -> Option<T> {
    let raw = read_raw(path)?;
    let mut value: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(err) => {
            log_parse_failure(path, &err);
            return None;
        }
    };
    filter(&mut value);
    match serde_json::from_value(value) {
        Ok(value) => Some(value),
        Err(err) => {
            log_parse_failure(path, &err);
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
// Section template
// ---------------------------------------------------------------------------

/// One versioned section file: its file name, the accessor for its `version` field and, for
/// the hot sections, the cache to invalidate after a successful save. The four sections share
/// this read/save skeleton instead of re-implementing it; genuine differences (cached vs
/// uncached, a per-rule filter, a zero-counter projection) stay in the section modules.
pub(super) struct SectionSpec<T> {
    /// File name inside the config directory; also names the section in version log lines.
    pub(super) file: &'static str,
    /// Accessor for the section's `version` field.
    pub(super) version: fn(&mut T) -> &mut u32,
    /// Cache invalidation hook for cached sections.
    pub(super) invalidate: Option<fn()>,
}

impl<T> SectionSpec<T> {
    pub(super) fn path_in(&self, dir: &Path) -> PathBuf {
        section_path(dir, self.file)
    }

    /// Applies the read-time version policy to a parsed section.
    pub(super) fn normalize(&self, value: &mut T) {
        normalize_version(self.file, (self.version)(value));
    }

    fn invalidate_cache(&self) {
        if let Some(invalidate) = self.invalidate {
            invalidate();
        }
    }
}

impl<T: DeserializeOwned> SectionSpec<T> {
    /// Shared read path: raw read, quarantine on a parse error, version normalization.
    pub(super) fn load(&self, path: &Path) -> Option<T> {
        let mut value = read_json::<T>(path)?;
        self.normalize(&mut value);
        Some(value)
    }
}

impl<T: Serialize> SectionSpec<T> {
    /// Shared save path: stamp the current version, write atomically, then invalidate the
    /// section cache (cached sections only).
    pub(super) fn save(&self, dir: &Path, mut value: T) -> std::io::Result<()> {
        *(self.version)(&mut value) = CONFIG_VERSION;
        let result = write_json_atomic(&self.path_in(dir), &value);
        if result.is_ok() {
            self.invalidate_cache();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::{AppTheme, RulesFile, RuntimeState, Settings};
    use crate::models::Rule;

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
}
