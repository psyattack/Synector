//! Canonical spellings of the process flags that cross module boundaries.
//!
//! The vocabulary table in [`crate::app::args`] decides which surface claims a spelling and
//! whether it takes a value; this module is the one definition of the spelling itself. Parsers
//! (`app::args`, `daemon`, `cli::args`) and writers (`app::spawn`, `daemon::overlay_manager`,
//! `shortcut`) all reference these constants, so a flag cannot be written with one spelling in
//! one process and read with another in the other process.
//!
//! Usage strings stay literal because they are frozen user-visible text; the tests pin that every
//! constant is accepted by the surface that advertises it.

// Runtime dispatch.
pub(crate) const GUI: &str = "--gui";
pub(crate) const DAEMON: &str = "--daemon";
pub(crate) const DAEMON_SHORT: &str = "-d";
pub(crate) const SERVICE: &str = "--service";
pub(crate) const SERVICE_SHORT: &str = "-s";
pub(crate) const STATUS: &str = "--status";
pub(crate) const NO_ELEVATE: &str = "--no-elevate";
pub(crate) const SPAWNED_BY_GUI: &str = "--spawned-by-gui";
pub(crate) const RUN_RULE: &str = "--run-rule";
pub(crate) const RUN_RULE_SHORT: &str = "-r";
pub(crate) const OVERLAY_WINDOW: &str = "--overlay-window";
pub(crate) const OVERLAY_WINDOW_SHORT: &str = "-o";
pub(crate) const GUI_EXE: &str = "--gui-exe";
pub(crate) const HELP: &str = "--help";
pub(crate) const HELP_SHORT: &str = "-h";

/// Every spelling that selects the daemon tier. Checked by the daemon and by the runtime table.
pub(crate) const DAEMON_MODE: &[&str] = &[DAEMON, DAEMON_SHORT, SERVICE, SERVICE_SHORT];

// Standalone overlay companions.
pub(crate) const CALIBRATION: &str = "--calibration";
pub(crate) const DURATION: &str = "--duration";
pub(crate) const OVERRIDE_TEXT: &str = "--override-text";
pub(crate) const OVERLAY_DATA: &str = "--overlay-data";

// Rule-authoring commands.
pub(crate) const RULES_EXAMPLE: &str = "--rules-example";
pub(crate) const RULES_SCHEMA: &str = "--rules-schema";
pub(crate) const RULES_REFERENCE: &str = "--rules-reference";
pub(crate) const RULES_TEMPLATE: &str = "--rules-template";
pub(crate) const RULES_VALIDATE: &str = "--rules-validate";
pub(crate) const RULES_APPLY: &str = "--rules-apply";
pub(crate) const RULES_SYNC: &str = "--rules-sync";
pub(crate) const RULES_DELETE: &str = "--rules-delete";
pub(crate) const RULES_ENABLE: &str = "--rules-enable";
pub(crate) const RULES_DISABLE: &str = "--rules-disable";
pub(crate) const RULES_RESET: &str = "--rules-reset";
pub(crate) const RULES_LIST: &str = "--rules-list";
pub(crate) const RULES_EXPORT: &str = "--rules-export";
pub(crate) const RULES_SHARE: &str = "--rules-share";
pub(crate) const RULES_IMPORT_SHARE: &str = "--rules-import-share";

// Rule-authoring modifiers.
pub(crate) const JSON: &str = "--json";
pub(crate) const DRY_RUN: &str = "--dry-run";
pub(crate) const ENABLE_IMPORTED: &str = "--enable-imported";

/// Every spelling this module declares, in one place, for the uniqueness gate.
#[cfg(test)]
const ALL: &[&str] = &[
    GUI, DAEMON, DAEMON_SHORT, SERVICE, SERVICE_SHORT, STATUS, NO_ELEVATE, SPAWNED_BY_GUI,
    RUN_RULE, RUN_RULE_SHORT, OVERLAY_WINDOW, OVERLAY_WINDOW_SHORT, GUI_EXE, HELP, HELP_SHORT,
    CALIBRATION, DURATION, OVERRIDE_TEXT, OVERLAY_DATA, RULES_EXAMPLE, RULES_SCHEMA,
    RULES_REFERENCE, RULES_TEMPLATE, RULES_VALIDATE, RULES_APPLY, RULES_SYNC, RULES_DELETE,
    RULES_ENABLE, RULES_DISABLE, RULES_RESET, RULES_LIST, RULES_EXPORT, RULES_SHARE,
    RULES_IMPORT_SHARE, JSON, DRY_RUN, ENABLE_IMPORTED,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flag_spellings_are_unique_and_prefixed() {
        let mut seen = std::collections::BTreeSet::new();
        for spelling in ALL {
            assert!(spelling.starts_with('-'), "{spelling} is not a flag");
            assert!(seen.insert(*spelling), "{spelling} is declared twice");
        }
        assert_eq!(ALL.len(), seen.len());
    }

    #[test]
    fn test_daemon_mode_lists_every_daemon_spelling() {
        assert_eq!(
            DAEMON_MODE,
            &[DAEMON, DAEMON_SHORT, SERVICE, SERVICE_SHORT]
        );
        assert_eq!(DAEMON_MODE.len(), 4);
    }
}
