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

mod app;
mod cache;
mod sections;
mod storage;

pub use app::AppConfig;
pub use sections::{AppTheme, Library, RuleRuntime, RulesFile, RuntimeState, Settings, WindowState};

/// Schema version stamped into every config file. Bump it when a section layout
/// changes and handle the older value in `storage::normalize_version`.
pub const CONFIG_VERSION: u32 = 1;

fn default_version() -> u32 {
    CONFIG_VERSION
}

fn default_true() -> bool {
    true
}
