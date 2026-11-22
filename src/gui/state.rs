//! GUI-side state hub: the [`GuiState`] aggregate plus its owned submodules for modals,
//! drafts, the overlay studio, IPC, daemon sync, updates, share codes and process snapshots.

use std::time::Instant;

use crate::ipc::DaemonState;
use crate::models::{ProcessSnapshotItem, Rule};

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

pub use self::modals::{
    ActionCatalogModalState, ActiveView, ConditionCatalogState, ConditionCatalogTarget,
    ElementCatalogModalState, ExpandedScriptEditorState, IfElseCatalogTarget,
    MacroEditorModalState, ProcessCategoryFilter, ProcessPickerTarget, ProcessSortColumn,
    RuleSortMode, SettingsTab, TriggerCatalogState,
};
pub use self::overlay_studio::{ColorPickerTarget, OverlayStudioState};

pub(crate) use self::undo::push_if_new;

pub struct GuiState {
    pub active_view: ActiveView,
    pub search_query: String,
    pub daemon_state: Option<DaemonState>,
    pub cached_processes: Vec<ProcessSnapshotItem>,
    pub process_sort_column: ProcessSortColumn,
    pub process_sort_ascending: bool,
    pub process_category_filter: ProcessCategoryFilter,
    pub selected_process: Option<ProcessSnapshotItem>,
    pub selected_rule_id: Option<String>,
    pub is_creating_new_rule: bool,
    pub editing_rule: Option<Rule>,
    pub builder_draft: Option<Rule>,
    pub picker_target: Option<ProcessPickerTarget>,
    pub last_picker_target: Option<ProcessPickerTarget>,
    pub process_search_query: String,
    pub expanded_script_editor: Option<ExpandedScriptEditorState>,
    pub last_script_editor: Option<ExpandedScriptEditorState>,
    pub macro_editor: Option<MacroEditorModalState>,
    pub last_macro_editor: Option<MacroEditorModalState>,
    pub show_macro_presets_modal: bool,
    pub macro_preset_save_name: String,
    pub macro_preset_search_query: String,
    pub overlay_studio: Option<OverlayStudioState>,
    pub last_overlay_studio: Option<OverlayStudioState>,
    pub action_catalog: Option<ActionCatalogModalState>,
    pub last_action_catalog: Option<ActionCatalogModalState>,
    pub trigger_catalog: Option<TriggerCatalogState>,
    pub last_trigger_catalog: Option<TriggerCatalogState>,
    pub condition_catalog: Option<ConditionCatalogState>,
    pub last_condition_catalog: Option<ConditionCatalogState>,
    pub element_catalog: Option<ElementCatalogModalState>,
    pub last_element_catalog: Option<ElementCatalogModalState>,
    pub language: crate::i18n::Language,
    pub theme: crate::config::AppTheme,
    pub toast: Option<(String, Instant)>,
    pub is_daemon_connected: bool,
    pub has_connected_daemon: bool,
    pub failed_daemon_syncs: u32,
    pub icon_cache: crate::gui::components::icons::ProcessIconCache,
    pub available_update: Option<crate::updater::ReleaseInfo>,
    pub is_checking_update: bool,
    pub is_downloading_update: bool,
    pub gui_closed_during_update: bool,
    pub update_download_progress: f32,
    pub show_changelog_modal: bool,
    pub show_guide_modal: bool,
    pub guide_selected_topic: usize,
    pub analytics_log_filter: String,
    pub group_full_presses: bool,
    pub undo_stack: Vec<Rule>,
    pub redo_stack: Vec<Rule>,
    pub settings_tab: SettingsTab,
    pub active_test_overlays: std::collections::HashSet<String>,
    pub active_test_runs: std::collections::HashMap<String, Instant>,
    pub rule_sort_mode: RuleSortMode,
    pub selected_tag_filter: Option<String>,
    pub tag_picker_open: bool,
    pub new_tag_name: String,
    pub new_tag_color: Option<String>,
    pub new_tag_icon: Option<String>,
    /// Completed inline step tests waiting for the inspector to place them in egui temp data.
    pub(crate) pending_test_step_results: Vec<(egui::Id, (bool, String, Instant))>,
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
            active_view: ActiveView::Rules,
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
            guide_selected_topic: 0,
            analytics_log_filter: String::new(),
            group_full_presses: false,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            settings_tab: SettingsTab::General,
            active_test_overlays: std::collections::HashSet::new(),
            active_test_runs: std::collections::HashMap::new(),
            rule_sort_mode: RuleSortMode::Default,
            selected_tag_filter: None,
            tag_picker_open: false,
            new_tag_name: String::new(),
            new_tag_color: Some("#3B82F6".into()),
            new_tag_icon: None,
            pending_test_step_results: Vec::new(),
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
