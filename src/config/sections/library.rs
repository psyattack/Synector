//! `library.json`: overlay and macro presets.
//!
//! Hot paths share one cached `Arc<Library>`; every successful save invalidates it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::config::cache::{cached_section, invalidate_section, LIBRARY_CACHE};
use crate::config::storage::{SectionSpec, LIBRARY_FILE};
use crate::config::{default_version, AppConfig, CONFIG_VERSION};
use crate::models::{MacroPreset, OverlayDefinition};

pub(crate) fn default_overlays() -> Vec<OverlayDefinition> {
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

fn version_mut(library: &mut Library) -> &mut u32 {
    &mut library.version
}

fn invalidate_cache() {
    invalidate_section(&LIBRARY_CACHE);
}

/// Path, version stamp and cache invalidation for `library.json`.
static SECTION: SectionSpec<Library> = SectionSpec {
    file: LIBRARY_FILE,
    version: version_mut,
    invalidate: Some(invalidate_cache),
};

impl Library {
    pub fn path_in(dir: &Path) -> PathBuf {
        SECTION.path_in(dir)
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
        SECTION.load(path)
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&AppConfig::config_dir())
    }

    pub fn save_to(&self, dir: &Path) -> std::io::Result<()> {
        SECTION.save(dir, self.clone())
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
    use std::fs;
    use std::sync::RwLock;
    use std::time::{Duration, SystemTime};

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "synector-config-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
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
}
