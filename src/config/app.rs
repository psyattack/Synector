//! The in-memory [`AppConfig`] aggregate assembled from the four section files.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::sections::{
    default_overlays, AppTheme, Library, RulesFile, RuntimeState, Settings, UiSkin, WindowState,
};
use super::storage::{
    clean_stale_temp_files, log_section_create_failure, LIBRARY_FILE, RULES_FILE, RUNTIME_FILE,
    SETTINGS_FILE,
};
use super::{default_true, CONFIG_VERSION};
use crate::models::{MacroPreset, OverlayDefinition, Rule};

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
    #[serde(default = "default_true")]
    pub log_to_disk: bool,
    #[serde(default)]
    pub language: crate::i18n::Language,
    #[serde(default = "default_true")]
    pub check_updates_on_startup: bool,
    #[serde(default)]
    pub auto_install_updates: bool,
    #[serde(default)]
    pub theme: AppTheme,
    #[serde(default)]
    pub follow_system_theme: bool,
    #[serde(default)]
    pub ui_skin: UiSkin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,

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
            log_to_disk: true,
            language: crate::i18n::Language::English,
            check_updates_on_startup: true,
            auto_install_updates: false,
            theme: AppTheme::Dark,
            follow_system_theme: false,
            ui_skin: UiSkin::default(),
            accent: None,
            rules: Vec::new(),
            overlays: default_overlays(),
            macro_presets: crate::models::default_macro_presets(),
        }
    }
}

impl AppConfig {
    /// Config directory without filesystem side effects. Hot readers go through this so a
    /// cache hit costs one `stat` rather than a `create_dir_all` + read + parse.
    pub(super) fn config_home() -> PathBuf {
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

    /// Daemon-side load: materialises missing section files and performs the one-shot first-run
    /// initialisation (the Windows UI language and the autostart registration).
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
        Self::load_from_with(dir, crate::i18n::Language::system)
    }

    /// [`Self::load_from`] with the language a first run adopts as a parameter: production passes
    /// [`crate::i18n::Language::system`], while tests pass a fixed language so the one-shot
    /// adoption is verifiable without depending on the host's UI language.
    fn load_from_with(
        dir: &Path,
        first_run_language: impl FnOnce() -> crate::i18n::Language,
    ) -> Self {
        clean_stale_temp_files(dir);

        let settings_path = Settings::path_in(dir);
        let first_run = !settings_path.exists();
        let mut config = Self::read_from(dir);

        // First launch: no section records a language choice yet, so the Windows UI language is
        // adopted once, like the autostart registration below. It is persisted together with the
        // settings section materialized further down; every later launch keeps the user's choice.
        if first_run {
            config.language = first_run_language();
            crate::logging::info(
                "CONFIG",
                "First run: adopting the Windows interface language",
            );
        }

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
            log_to_disk: settings.log_to_disk,
            language: settings.language,
            check_updates_on_startup: settings.check_updates_on_startup,
            auto_install_updates: settings.auto_install_updates,
            theme: settings.theme,
            follow_system_theme: settings.follow_system_theme,
            ui_skin: settings.ui_skin,
            accent: settings.accent,
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
            log_to_disk: self.log_to_disk,
            language: self.language,
            check_updates_on_startup: self.check_updates_on_startup,
            auto_install_updates: self.auto_install_updates,
            theme: self.theme,
            follow_system_theme: self.follow_system_theme,
            ui_skin: self.ui_skin,
            accent: self.accent.clone(),
        }
    }

    /// Replaces the settings half of the aggregate with `settings`, keeping rules and
    /// presets untouched.
    pub fn apply_settings(&mut self, settings: Settings) {
        self.tray_icon_path = settings.tray_icon_path;
        self.window = settings.window;
        self.hide_tray_icon = settings.hide_tray_icon;
        self.mask_daemon_process = settings.mask_daemon_process;
        self.log_to_disk = settings.log_to_disk;
        self.language = settings.language;
        self.check_updates_on_startup = settings.check_updates_on_startup;
        self.auto_install_updates = settings.auto_install_updates;
        self.theme = settings.theme;
        self.follow_system_theme = settings.follow_system_theme;
        self.ui_skin = settings.ui_skin;
        self.accent = settings.accent;
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
    use crate::config::RuleRuntime;
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

    fn rule(id: &str, name: &str) -> Rule {
        Rule {
            id: id.to_string(),
            name: name.to_string(),
            ..Default::default()
        }
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
            log_to_disk: false,
            language: crate::i18n::Language::Russian,
            check_updates_on_startup: false,
            auto_install_updates: true,
            theme: AppTheme::Light,
            follow_system_theme: true,
            ui_skin: UiSkin::Modern,
            accent: Some("#2d81ff".to_string()),
        };
        let mut config = AppConfig::default();
        config.apply_settings(settings.clone());
        assert_eq!(config.settings(), settings, "every setting must survive the projection");

        let library = config.library();
        assert_eq!(library.overlays, config.overlays);
        assert_eq!(library.macro_presets, config.macro_presets);
        assert_eq!(library.version, CONFIG_VERSION);
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

        let config = AppConfig::load_from_with(&dir, || crate::i18n::Language::English);
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

        AppConfig::load_from_with(&dir, || crate::i18n::Language::English);

        assert!(!stale.exists(), "a stale section temp file must be removed");
        assert!(fresh.exists(), "a fresh temp file may belong to a live writer");
        assert!(foreign.exists(), "unrelated files must not be touched");
    }

    /// A first run adopts the supplied system language and persists it with the section it
    /// materializes. Once `settings.json` exists the persisted choice wins, because the detection
    /// is a one-shot initialization like the autostart registration — never a re-detection.
    #[test]
    fn first_run_adopts_the_system_language_once() {
        let dir = temp_dir("first-run-language");

        let config = AppConfig::load_from_with(&dir, || crate::i18n::Language::Russian);
        assert_eq!(config.language, crate::i18n::Language::Russian);
        assert_eq!(
            Settings::read_from(&dir).language,
            crate::i18n::Language::Russian,
            "the adopted language must be persisted with the created settings section"
        );

        let config = AppConfig::load_from_with(&dir, || crate::i18n::Language::English);
        assert_eq!(
            config.language,
            crate::i18n::Language::Russian,
            "an existing settings section must keep the recorded choice"
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
