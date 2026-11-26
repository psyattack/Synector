//! GUI-side state hub: the [`GuiState`] aggregate plus its owned submodules for modals,
//! drafts, the overlay studio, IPC, daemon sync, updates, share codes and process snapshots.

use std::time::Instant;

use crate::ipc::DaemonState;
use crate::models::{ProcessSnapshotItem, Rule};

mod analytics;
mod drafts;
mod filter;
mod ipc;
pub(crate) mod library_write;
mod modals;
mod overlay_studio;
mod processes;
mod share;
mod shortcuts;
mod sync;
mod undo;
mod updates;

use self::drafts::PendingMacroSave;
use self::ipc::IpcGateway;
use self::library_write::PendingLibraryWrite;
use self::share::ShareExports;
use self::sync::DaemonSync;
use self::updates::Updates;

pub use self::analytics::AnalyticsLogFilter;
pub use self::modals::{
    ActionCatalogModalState, ConditionCatalogState, ConditionCatalogTarget,
    ElementCatalogModalState, ExpandedScriptEditorState, IfElseCatalogTarget,
    MacroEditorModalState, ProcessCategoryFilter, ProcessPickerTarget, ProcessSortColumn,
    RuleSortMode, TriggerCatalogState,
};
pub use self::overlay_studio::{ColorPickerTarget, OverlayStudioState};

pub(crate) use self::filter::rule_matches_visible_filters;
pub(crate) use self::undo::push_if_new;

pub struct GuiState {
    /// Query of the header search field: it filters the rules list.
    pub(crate) search_query: String,
    pub(crate) daemon_state: Option<DaemonState>,
    pub(crate) cached_processes: Vec<ProcessSnapshotItem>,
    pub(crate) process_sort_column: ProcessSortColumn,
    pub(crate) process_sort_ascending: bool,
    pub(crate) process_category_filter: ProcessCategoryFilter,
    pub(crate) selected_process: Option<ProcessSnapshotItem>,
    pub(crate) selected_rule_id: Option<String>,
    pub(crate) is_creating_new_rule: bool,
    pub(crate) editing_rule: Option<Rule>,
    pub(crate) builder_draft: Option<Rule>,
    pub(crate) picker_target: Option<ProcessPickerTarget>,
    pub(crate) last_picker_target: Option<ProcessPickerTarget>,
    pub(crate) process_search_query: String,
    pub(crate) expanded_script_editor: Option<ExpandedScriptEditorState>,
    pub(crate) last_script_editor: Option<ExpandedScriptEditorState>,
    pub(crate) macro_editor: Option<MacroEditorModalState>,
    pub(crate) last_macro_editor: Option<MacroEditorModalState>,
    pub(crate) show_macro_presets_modal: bool,
    pub(crate) macro_preset_save_name: String,
    pub(crate) macro_preset_search_query: String,
    pub(crate) overlay_studio: Option<OverlayStudioState>,
    pub(crate) last_overlay_studio: Option<OverlayStudioState>,
    pub(crate) action_catalog: Option<ActionCatalogModalState>,
    pub(crate) last_action_catalog: Option<ActionCatalogModalState>,
    pub(crate) trigger_catalog: Option<TriggerCatalogState>,
    pub(crate) last_trigger_catalog: Option<TriggerCatalogState>,
    pub(crate) condition_catalog: Option<ConditionCatalogState>,
    pub(crate) last_condition_catalog: Option<ConditionCatalogState>,
    pub(crate) element_catalog: Option<ElementCatalogModalState>,
    pub(crate) last_element_catalog: Option<ElementCatalogModalState>,
    pub(crate) language: crate::i18n::Language,
    pub(crate) theme: crate::config::AppTheme,
    /// Whether the palette follows the Windows colour scheme; `theme` is then the effective
    /// palette and the persisted fallback at once.
    pub(crate) follow_system_theme: bool,
    pub(crate) ui_skin: crate::config::UiSkin,
    pub(crate) accent: Option<String>,
    pub(crate) toast: Option<(String, Instant)>,
    pub(crate) is_daemon_connected: bool,
    pub(crate) has_connected_daemon: bool,
    pub(crate) failed_daemon_syncs: u32,
    pub(crate) icon_cache: crate::gui::components::icons::ProcessIconCache,
    pub(crate) available_update: Option<crate::updater::ReleaseInfo>,
    pub(crate) is_checking_update: bool,
    pub(crate) is_downloading_update: bool,
    pub(crate) gui_closed_during_update: bool,
    pub(crate) update_download_progress: f32,
    pub(crate) show_changelog_modal: bool,
    pub(crate) show_guide_modal: bool,
    /// The analytics dashboard dialog, opened from the header glyph.
    pub(crate) show_analytics_modal: bool,
    /// The settings dialog, opened from the header glyph.
    pub(crate) show_settings_modal: bool,
    /// The header global search results panel. It is shown while the header field holds a query
    /// and is open; applying a result closes it or opens the target surface.
    pub(crate) search_overlay_open: bool,
    /// Latest analytics snapshot from the daemon, refreshed while the dialog is open.
    pub(crate) analytics: Option<crate::models::AnalyticsSnapshot>,
    /// Level filter of the analytics activity log; a global-search handoff resets it to `All`.
    pub(crate) analytics_log_level: self::analytics::AnalyticsLogFilter,
    pub(crate) guide_selected_topic: usize,
    /// Set when the guide switches chapter, so its content scroll area returns to the top.
    pub(crate) guide_scroll_to_top: bool,
    pub(crate) analytics_log_filter: String,
    pub(crate) group_full_presses: bool,
    pub(crate) undo_stack: Vec<Rule>,
    pub(crate) redo_stack: Vec<Rule>,
    /// Query of the settings dialog search field: it filters the setting rows by their own text.
    pub(crate) settings_search_query: String,
    pub(crate) active_test_overlays: std::collections::HashSet<String>,
    pub(crate) active_test_runs: std::collections::HashMap<String, Instant>,
    pub(crate) rule_sort_mode: RuleSortMode,
    pub(crate) selected_tag_filter: Option<String>,
    pub(crate) tag_picker_open: bool,
    pub(crate) new_tag_name: String,
    pub(crate) new_tag_color: Option<String>,
    pub(crate) new_tag_icon: Option<String>,
    /// Completed inline step tests whose running focus the inspector must clear, paired with the
    /// daemon-side test id so a stale completion cannot clear a newer run of the same step.
    pub(crate) finished_step_tests: Vec<(egui::Id, u64)>,
    /// Latest calibration position reported by the daemon for one overlay.
    pub(crate) cached_overlay_position: Option<(String, f32, f32)>,
    /// Revision of the newest settings write and the newest revision the daemon confirmed.
    pub(crate) settings_revision: u64,
    pub(crate) settings_saved_revision: Option<u64>,
    /// Preset-library writes queued for the IPC worker, keyed by their job key.
    pub(crate) library_writes: std::collections::BTreeMap<u64, PendingLibraryWrite>,
    pub(crate) next_library_write_key: u64,
    /// Off-thread IPC (daemon sync, process snapshots, mutations).
    ipc: IpcGateway,
    /// Share-code compression worker and its queue.
    share: ShareExports,
    /// Poll cadence and last outage detail.
    daemon_sync: DaemonSync,
    /// Updater channel and release advertisement.
    updates: Updates,
    /// Macro preset saves awaiting confirmation.
    pending_macro_saves: Vec<PendingMacroSave>,
}

impl GuiState {
    pub fn new() -> Self {
        let settings = crate::config::Settings::read();
        Self {
            search_query: String::new(),
            daemon_state: None,
            cached_processes: Vec::new(),
            process_sort_column: ProcessSortColumn::Name,
            process_sort_ascending: true,
            process_category_filter: ProcessCategoryFilter::All,
            selected_process: None,
            selected_rule_id: None,
            is_creating_new_rule: false,
            editing_rule: None,
            builder_draft: None,
            picker_target: None,
            last_picker_target: None,
            process_search_query: String::new(),
            expanded_script_editor: None,
            last_script_editor: None,
            macro_editor: None,
            last_macro_editor: None,
            show_macro_presets_modal: false,
            macro_preset_save_name: String::new(),
            macro_preset_search_query: String::new(),
            overlay_studio: None,
            last_overlay_studio: None,
            action_catalog: None,
            last_action_catalog: None,
            trigger_catalog: None,
            last_trigger_catalog: None,
            condition_catalog: None,
            last_condition_catalog: None,
            element_catalog: None,
            last_element_catalog: None,
            language: settings.language,
            theme: settings.theme,
            follow_system_theme: settings.follow_system_theme,
            ui_skin: settings.ui_skin,
            accent: settings.accent,
            toast: None,
            is_daemon_connected: false,
            has_connected_daemon: false,
            failed_daemon_syncs: 0,
            icon_cache: crate::gui::components::icons::ProcessIconCache::new(),
            available_update: None,
            is_checking_update: false,
            is_downloading_update: false,
            gui_closed_during_update: false,
            update_download_progress: 0.0,
            show_changelog_modal: false,
            show_guide_modal: false,
            show_analytics_modal: false,
            show_settings_modal: false,
            search_overlay_open: false,
            analytics: None,
            analytics_log_level: Default::default(),
            guide_selected_topic: 0,
            guide_scroll_to_top: false,
            analytics_log_filter: String::new(),
            group_full_presses: false,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            settings_search_query: String::new(),
            active_test_overlays: std::collections::HashSet::new(),
            active_test_runs: std::collections::HashMap::new(),
            rule_sort_mode: RuleSortMode::Default,
            selected_tag_filter: None,
            tag_picker_open: false,
            new_tag_name: String::new(),
            new_tag_color: Some("#3B82F6".into()),
            new_tag_icon: None,
            finished_step_tests: Vec::new(),
            cached_overlay_position: None,
            settings_revision: 0,
            settings_saved_revision: None,
            library_writes: std::collections::BTreeMap::new(),
            next_library_write_key: 0,
            ipc: IpcGateway::new(),
            share: ShareExports::default(),
            daemon_sync: DaemonSync::new(),
            updates: Updates::new(),
            pending_macro_saves: Vec::new(),
        }
    }

    pub fn loc(&self) -> &'static crate::i18n::Strings {
        crate::i18n::get(self.language)
    }

    /// Updates the in-memory language only. Persistence is the daemon's job: the
    /// settings view sends the change over IPC, which keeps the GUI from racing the
    /// daemon on `settings.json` (the daemon also owns window geometry there).
    pub fn set_language(&mut self, lang: crate::i18n::Language) {
        if self.language != lang {
            self.language = lang;
            self.set_toast(self.loc().toast_lang_changed);
        }
    }

    /// Updates the in-memory theme only; persisted through `IpcRequest::SaveSettings`.
    pub fn set_theme(&mut self, theme: crate::config::AppTheme) {
        if self.theme != theme {
            self.theme = theme;
            self.set_toast(self.loc().toast_theme_changed);
        }
    }

    /// Turns the "follow the Windows colour scheme" preference on or off. Turning it on adopts the
    /// palette Windows uses right now; turning it off keeps the palette on screen, which the
    /// settings row then stores as the explicit choice.
    pub fn set_follow_system_theme(&mut self, follow: bool) {
        self.follow_system_theme = follow;
        if follow {
            self.adopt_system_theme();
        }
    }

    /// Renders with the palette Windows uses for its own apps. An unreadable preference keeps the
    /// current palette, and the switch stays silent: nobody clicked a palette, so there is no
    /// toast and nothing is persisted.
    pub(crate) fn adopt_system_theme(&mut self) {
        if let Some(system) = crate::config::AppTheme::system() {
            self.theme = system;
        }
    }

    /// Switches the visual generation. The palette is untouched: the two axes are independent.
    pub fn set_ui_skin(&mut self, skin: crate::config::UiSkin) {
        self.ui_skin = skin;
    }

    /// Sets the custom accent (`#RRGGBB`), or clears it back to the palette default.
    pub fn set_accent(&mut self, accent: Option<String>) {
        self.accent = accent;
    }

    pub fn set_toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    pub fn is_any_rule_executing(&self) -> bool {
        if !self.active_test_runs.is_empty() {
            return true;
        }
        self.daemon_state
            .as_ref()
            .map(|s| !s.active_rule_executions.is_empty())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Turning the Windows sync on adopts the palette Windows reports and never raises the
    /// user-facing theme toast; turning it off leaves the palette on screen untouched, because
    /// the settings row stores that palette as the explicit choice.
    #[test]
    fn follow_system_theme_adopts_and_keeps_the_windows_palette() {
        let mut state = GuiState::new();
        state.theme = crate::config::AppTheme::Dark;
        state.toast = None;

        state.set_follow_system_theme(true);
        assert!(state.follow_system_theme);
        match crate::config::AppTheme::system() {
            Some(system) => assert_eq!(state.theme, system),
            None => assert_eq!(
                state.theme,
                crate::config::AppTheme::Dark,
                "an unreadable Windows preference keeps the current palette"
            ),
        }
        assert!(
            state.toast.is_none(),
            "an automatic palette switch is not a user edit"
        );

        let on_screen = state.theme;
        state.set_follow_system_theme(false);
        assert!(!state.follow_system_theme);
        assert_eq!(
            state.theme, on_screen,
            "stopping the sync must not change the palette the user is looking at"
        );
    }
}
