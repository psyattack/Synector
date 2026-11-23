//! `settings.json`: rarely written host preferences.
//!
//! Window geometry, tray, language, theme and update behaviour. Reads are served through the
//! process cache; every successful save invalidates it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::cache::{cached_section, invalidate_section, SETTINGS_CACHE};
use crate::config::storage::{SectionSpec, SETTINGS_FILE};
use crate::config::{default_true, default_version, AppConfig, CONFIG_VERSION};

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

/// The two shipped palettes. A palette is only colours; the geometry and typography of the
/// interface belong to the GUI skin, so a skin switch never changes the theme and vice versa.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AppTheme {
    #[default]
    Dark,
    Light,
}

/// The visual generation of the interface: its geometry, density, strokes and type voice.
///
/// Independent of [`AppTheme`]: switching the skin never changes the palette and switching the
/// palette never changes the skin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UiSkin {
    /// Rounded, layered, sans-serif. The default generation.
    #[default]
    Modern,
    /// The original studio identity: hard edges, hairlines, mono labels.
    Classic,
}

// ---------------------------------------------------------------------------
// settings.json
// ---------------------------------------------------------------------------

/// Rarely written host preferences: window geometry, tray, language, theme, skin and
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
    #[serde(default)]
    pub ui_skin: UiSkin,
    /// Custom accent colour as #RRGGBB; absent means the palette's default accent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
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
            ui_skin: UiSkin::default(),
            accent: None,
        }
    }
}

fn version_mut(settings: &mut Settings) -> &mut u32 {
    &mut settings.version
}

fn invalidate_cache() {
    invalidate_section(&SETTINGS_CACHE);
}

/// Path, version stamp and cache invalidation for `settings.json`.
static SECTION: SectionSpec<Settings> = SectionSpec {
    file: SETTINGS_FILE,
    version: version_mut,
    invalidate: Some(invalidate_cache),
};

impl Settings {
    pub fn path_in(dir: &Path) -> PathBuf {
        SECTION.path_in(dir)
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
        let mut settings = SECTION.load(path)?;
        if let Some(ws) = &mut settings.window {
            ws.sanitize();
        }
        Some(settings)
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&AppConfig::config_dir())
    }

    pub fn save_to(&self, dir: &Path) -> std::io::Result<()> {
        SECTION.save(dir, self.clone())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::RwLock;
    use std::time::SystemTime;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "synector-config-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
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
            theme: AppTheme::Light,
            hide_tray_icon: true,
            check_updates_on_startup: false,
            ..Default::default()
        };
        settings.version = 0;
        settings.save_to(&dir).expect("save settings");

        let decoded = Settings::read_from(&dir);
        assert_eq!(decoded.version, CONFIG_VERSION);
        assert_eq!(decoded.language, crate::i18n::Language::Russian);
        assert_eq!(decoded.theme, AppTheme::Light);
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
    fn accent_round_trips_and_stays_optional() {
        let mut settings = Settings::default();
        assert_eq!(settings.accent, None, "no accent means the palette default");
        let decoded: Settings = serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(decoded.accent, None);

        settings.accent = Some("#8b7bd8".to_string());
        let decoded: Settings = serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(decoded.accent.as_deref(), Some("#8b7bd8"));
    }

    #[test]
    fn app_theme_serde_and_default() {
        assert_eq!(AppTheme::default(), AppTheme::Dark);
        assert_eq!(serde_json::to_string(&AppTheme::Dark).unwrap(), "\"dark\"");
        assert_eq!(serde_json::to_string(&AppTheme::Light).unwrap(), "\"light\"");
        assert_eq!(
            serde_json::from_str::<AppTheme>("\"dark\"").unwrap(),
            AppTheme::Dark
        );
        assert_eq!(
            serde_json::from_str::<AppTheme>("\"light\"").unwrap(),
            AppTheme::Light
        );
        assert!(
            serde_json::from_str::<AppTheme>("\"chocolate\"").is_err(),
            "a removed palette name is rejected like any other unknown value"
        );
    }
}
