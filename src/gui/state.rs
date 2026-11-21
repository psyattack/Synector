use std::time::Instant;
use crate::ipc::{DaemonState, IpcClient, IpcRequest, IpcResponse};
use crate::models::{ProcessSnapshotItem, Rule};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveView {
    Rules,
    Analytics,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProcessSortColumn {
    #[default]
    Name,
    Pid,
    WindowTitle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProcessCategoryFilter {
    #[default]
    All,
    WindowedOnly,
    BackgroundOnly,
}

#[derive(Debug, Clone)]
pub struct ExpandedScriptEditorState {
    pub step_idx: usize,
    pub is_exit_pipeline: bool,
    pub language: crate::gui::components::script_highlighter::ScriptLanguage,
}

#[derive(Debug, Clone)]
pub struct MacroEditorModalState {
    pub step_idx: usize,
    pub is_exit_pipeline: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessPickerTarget {
    Trigger,
    Action(usize),
    ExitAction(usize),
    Condition(usize),
    ActionCondition { action_idx: usize, cond_idx: usize, is_exit: bool },
    ActionElseIfCondition { action_idx: usize, branch_idx: usize, cond_idx: usize, is_exit: bool },
    IfElseThen { parent_idx: usize, sub_idx: usize, is_exit: bool },
    IfElseElseIf { parent_idx: usize, branch_idx: usize, sub_idx: usize, is_exit: bool },
    IfElseElse { parent_idx: usize, sub_idx: usize, is_exit: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IfElseCatalogTarget {
    Then(usize),
    ElseIf { parent_idx: usize, branch_idx: usize },
    Else(usize),
}

#[derive(Debug, Clone)]
pub struct ActionCatalogModalState {
    pub is_exit_pipeline: bool,
    pub insert_idx: Option<usize>,
    pub selected_category: usize,
    pub search_query: String,
    pub ifelse_target: Option<IfElseCatalogTarget>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionCatalogTarget {
    RulePrecondition,
    ChangeRulePrecondition(usize),
    ActionIfElse { action_idx: usize, is_exit: bool },
    ChangeActionIfElse { action_idx: usize, cond_idx: usize, is_exit: bool },
    ActionElseIf { action_idx: usize, branch_idx: usize, is_exit: bool },
    ChangeActionElseIf { action_idx: usize, branch_idx: usize, cond_idx: usize, is_exit: bool },
}

#[derive(Debug, Clone, Default)]
pub struct TriggerCatalogState {
    pub selected_category: usize,
    pub search_query: String,
}

#[derive(Debug, Clone, Default)]
pub struct ConditionCatalogState {
    pub selected_category: usize,
    pub search_query: String,
    pub target: Option<ConditionCatalogTarget>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorPickerTarget {
    CardBg,
    CardBorder,
    ElementPrimary(usize),
    ElementSecondary(usize),
    ElementTertiary(usize),
}

#[derive(Debug, Clone)]
pub struct OverlayStudioState {
    pub overlay: crate::models::OverlayDefinition,
    pub is_calibrating: bool,
    pub selected_element_idx: Option<usize>,
    pub action_target_idx: Option<usize>,
    pub is_exit_pipeline: bool,
    pub active_color_target: Option<ColorPickerTarget>,
    pub snap_to_grid: bool,
    pub card_lock_aspect: bool,
    pub canvas_pan: egui::Vec2,
    pub canvas_zoom: Option<f32>,
    pub last_synced_pos: Option<(f32, f32)>,
    pub undo_stack: Vec<crate::models::OverlayDefinition>,
    pub redo_stack: Vec<crate::models::OverlayDefinition>,
    pub show_presets_popover: bool,
    pub preset_save_name: String,
    pub preset_search_query: String,
}

impl OverlayStudioState {
    /// Fresh studio for one overlay definition. `action_target_idx` marks an edit that is
    /// written back into the rule action instead of the library.
    pub fn new(
        overlay: crate::models::OverlayDefinition,
        action_target_idx: Option<usize>,
        is_exit_pipeline: bool,
        preset_save_name: String,
    ) -> Self {
        Self {
            overlay,
            is_calibrating: false,
            selected_element_idx: None,
            action_target_idx,
            is_exit_pipeline,
            active_color_target: None,
            snap_to_grid: false,
            card_lock_aspect: false,
            canvas_pan: egui::Vec2::ZERO,
            canvas_zoom: None,
            last_synced_pos: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            show_presets_popover: false,
            preset_save_name,
            preset_search_query: String::new(),
        }
    }

    pub fn push_undo_snapshot(&mut self) {
        if self.undo_stack.last() != Some(&self.overlay) {
            self.undo_stack.push(self.overlay.clone());
            if self.undo_stack.len() > 50 {
                self.undo_stack.remove(0);
            }
            self.redo_stack.clear();
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn undo(&mut self) {
        if let Some(prev) = self.undo_stack.pop() {
            self.redo_stack.push(self.overlay.clone());
            self.overlay = prev;
            if let Some(sel) = self.selected_element_idx {
                if sel >= self.overlay.elements.len() {
                    self.selected_element_idx = None;
                }
            }
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo_stack.pop() {
            self.undo_stack.push(self.overlay.clone());
            self.overlay = next;
            if let Some(sel) = self.selected_element_idx {
                if sel >= self.overlay.elements.len() {
                    self.selected_element_idx = None;
                }
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ElementCatalogModalState {
    pub search_query: String,
    pub selected_category: usize,
}

type ShareExportOutcome = (
    Result<String, crate::sharing::ShareError>,
    usize,
    Vec<crate::sharing::ShareWarning>,
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    General,
    Appearance,
    Updates,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RuleSortMode {
    #[default]
    Default,
    NameAsc,
    NameDesc,
    TriggerType,
    ActionCount,
    Status,
}

// ---------------------------------------------------------------------------
// Off-thread plumbing
//
// `GuiState::sync_with_daemon` runs inside `SynectorApp::ui`, so every named-pipe round-trip
// there used to freeze the window when the daemon was slow or hung. Requests now go through
// one worker thread (`IpcGateway`); the frame thread only enqueues and applies results, and a
// mutation keeps its draft until the daemon (or the documented direct-file fallback) confirms it.
// ---------------------------------------------------------------------------

/// Daemon sync cadence: fast while something executes (live progress), relaxed when idle.
const DAEMON_SYNC_FAST_MS: u128 = 80;
const DAEMON_SYNC_IDLE_MS: u128 = 400;
/// Consecutive failed polls and the quiet period that turn a single miss into a confirmed
/// outage: the client reports a lost daemon instead of treating one dropped poll as a disconnect.
const DAEMON_OUTAGE_POLLS: u32 = 3;
const DAEMON_OUTAGE_GRACE: std::time::Duration = std::time::Duration::from_secs(1);
/// A state poll that takes longer than this is treated as a failure. The IPC client has no read
/// timeout, so without it a hung daemon would leave the GUI waiting silently forever.
const DAEMON_SYNC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
/// Minimum spacing between failed process-snapshot retries: the inspector asks for a snapshot on
/// every frame while it edits, and a down daemon must not turn that into a request flood.
const PROCESS_SNAPSHOT_RETRY: std::time::Duration = std::time::Duration::from_secs(2);

/// Identifies a queued IPC request so its result can update GUI state on the frame thread.
/// Mutations carry just enough identity for the success/failure reporting.
#[derive(Debug, Clone, PartialEq, Eq)]
enum IpcJobKind {
    Sync,
    Processes,
    SaveRule { rule_id: String },
    /// A cloned rule is saved under a new id; success keeps the editor the clone opened.
    CloneRule { rule_id: String },
    DeleteRule { rule_id: String },
    ToggleRule { rule_id: String, enabled: bool },
    ReorderRules,
    ResetRule { rule_id: String },
    SaveOverlay { overlay_id: String },
    SaveMacroPreset { preset_id: String },
    TestRunRule { rule_id: String },
    StopRuleExecution { rule_id: String },
    ImportShare,
    SetAutorun { enabled: bool },
    StartUpdate,
    /// Requests whose result has no user-visible effect (acknowledgements, update fan-out).
    Background,
}

impl IpcJobKind {
    /// Short identifier used in log lines; identifiers and outcomes only, never payloads.
    fn label(&self) -> String {
        match self {
            Self::Sync => "GetState".into(),
            Self::Processes => "GetProcesses".into(),
            Self::SaveRule { rule_id } => format!("SaveRule '{rule_id}'"),
            Self::CloneRule { rule_id } => format!("CloneRule save '{rule_id}'"),
            Self::DeleteRule { rule_id } => format!("DeleteRule '{rule_id}'"),
            Self::ToggleRule { rule_id, enabled } => format!("ToggleRule '{rule_id}' ({enabled})"),
            Self::ReorderRules => "ReorderRules".into(),
            Self::ResetRule { rule_id } => format!("ResetRule '{rule_id}'"),
            Self::SaveOverlay { overlay_id } => format!("SaveOverlay '{overlay_id}'"),
            Self::SaveMacroPreset { preset_id } => format!("SaveMacroPreset '{preset_id}'"),
            Self::TestRunRule { rule_id } => format!("TestRunRule '{rule_id}'"),
            Self::StopRuleExecution { rule_id } => format!("StopRuleExecution '{rule_id}'"),
            Self::ImportShare => "ImportShare".into(),
            Self::SetAutorun { enabled } => format!("SetAutorun ({enabled})"),
            Self::StartUpdate => "StartUpdate".into(),
            Self::Background => "background request".into(),
        }
    }
}

/// One request handed to the IPC worker. `direct` is the section write used only when the
/// daemon is unreachable (the fallback promised by `docs/ARCHITECTURE.md#data-flow`).
struct IpcJob {
    kind: IpcJobKind,
    request: IpcRequest,
    direct: Option<DirectWrite>,
}

/// Worker → frame result. A transport failure that the direct fallback completed arrives as
/// `Ok(IpcResponse::Ok)`, so the frame thread only has to interpret daemon-level outcomes.
struct IpcJobResult {
    kind: IpcJobKind,
    result: Result<IpcResponse, String>,
}

/// Direct section writes used when IPC is unavailable. The daemon remains the only writer
/// while it is reachable, so these run only after a failed liveness probe.
enum DirectWrite {
    Rules(DirectRuleChange),
    Library(DirectLibraryChange),
    ResetRuntime { rule_id: String },
}

enum DirectRuleChange {
    Upsert(Box<Rule>),
    Delete(String),
    SetEnabled { id: String, enabled: bool },
    Reorder(Vec<String>),
}

enum DirectLibraryChange {
    UpsertOverlay(Box<crate::models::OverlayDefinition>),
    UpsertMacroPreset(Box<crate::models::MacroPreset>),
}

impl DirectWrite {
    /// Applies the change to the section files under `dir`. Split from the runtime path so
    /// tests can point it at an isolated directory.
    fn apply_in(&self, dir: &std::path::Path) -> Result<(), String> {
        match self {
            Self::Rules(change) => {
                let mut file = crate::config::RulesFile::read_from(dir);
                match change {
                    DirectRuleChange::Upsert(rule) => {
                        if let Some(existing) = file.rules.iter_mut().find(|r| r.id == rule.id) {
                            *existing = (**rule).clone();
                        } else {
                            file.rules.push((**rule).clone());
                        }
                    }
                    DirectRuleChange::Delete(id) => file.rules.retain(|r| &r.id != id),
                    DirectRuleChange::SetEnabled { id, enabled } => {
                        if let Some(rule) = file.rules.iter_mut().find(|r| &r.id == id) {
                            rule.is_enabled = *enabled;
                        }
                    }
                    DirectRuleChange::Reorder(order) => reorder_rules(&mut file.rules, order),
                }
                crate::config::RulesFile::save_rules_to(dir, &file.rules).map_err(|e| e.to_string())
            }
            Self::ResetRuntime { rule_id } => {
                let mut file = crate::config::RulesFile::read_from(dir);
                if let Some(rule) = file.rules.iter_mut().find(|r| &r.id == rule_id) {
                    rule.is_enabled = true;
                }
                crate::config::RulesFile::save_rules_to(dir, &file.rules)
                    .map_err(|e| e.to_string())?;
                let mut runtime = crate::config::RuntimeState::read_from(dir);
                runtime.rules.remove(rule_id);
                runtime.save_to(dir).map_err(|e| e.to_string())
            }
            Self::Library(change) => {
                let mut library = crate::config::Library::read_from(dir);
                match change {
                    DirectLibraryChange::UpsertOverlay(overlay) => {
                        if let Some(existing) = library.overlays.iter_mut().find(|o| o.id == overlay.id) {
                            *existing = (**overlay).clone();
                        } else {
                            library.overlays.push((**overlay).clone());
                        }
                    }
                    DirectLibraryChange::UpsertMacroPreset(preset) => {
                        if let Some(existing) = library.macro_presets.iter_mut().find(|p| p.id == preset.id) {
                            *existing = (**preset).clone();
                        } else {
                            library.macro_presets.push((**preset).clone());
                        }
                    }
                }
                library.save_to(dir).map_err(|e| e.to_string())
            }
        }
    }
}

/// The direct fallback never runs in tests: they share one per-process config directory, so a
/// worker-thread write could race another test. `DirectWrite::apply_in` is covered separately.
fn direct_fallback_allowed() -> bool {
    !cfg!(test)
}

/// Rebuilds the rule list in an explicit order; ids not named keep their relative order at the
/// end. Mirrors the daemon's `ReorderRules` handler for the direct-file fallback.
fn reorder_rules(rules: &mut Vec<Rule>, order: &[String]) {
    let mut remaining = std::mem::take(rules);
    let mut ordered = Vec::with_capacity(remaining.len());
    for id in order {
        if let Some(position) = remaining.iter().position(|rule| &rule.id == id) {
            ordered.push(remaining.remove(position));
        }
    }
    ordered.append(&mut remaining);
    *rules = ordered;
}

/// A mutation the daemon did not confirm. `UnexpectedResponse` means the daemon answered with a
/// shape the client cannot interpret for that request, so the toast must use the generic
/// localized line instead of embedding an English detail.
enum MutationFailure {
    /// Daemon rejection or transport failure. Identifiers only — never user data.
    Detail(String),
    /// Unexpected response shape, described for log lines by its variant name.
    UnexpectedResponse(String),
}

impl MutationFailure {
    /// Detail for log lines and for toasts whose catalog line embeds one.
    fn diagnostic(&self) -> &str {
        match self {
            Self::Detail(detail) | Self::UnexpectedResponse(detail) => detail,
        }
    }
}

/// Variant name of a response for diagnostics. Never formats the contents: `DaemonState`
/// embeds rule JSON, which must not reach the log.
fn ipc_response_name(response: &IpcResponse) -> &'static str {
    match response {
        IpcResponse::Ok => "Ok",
        IpcResponse::Pong => "Pong",
        IpcResponse::State(_) => "State",
        IpcResponse::Processes(_) => "Processes",
        IpcResponse::TestStepResult(_) => "TestStepResult",
        IpcResponse::OverlayPosition(_) => "OverlayPosition",
        IpcResponse::ShareImportResult(_) => "ShareImportResult",
        IpcResponse::UpdateProgress { .. } => "UpdateProgress",
        IpcResponse::Error(_) => "Error",
    }
}

/// Runs one request on the IPC worker. A transport failure is retried as a direct section
/// write only when the daemon is confirmed unreachable; a daemon-level rejection is returned
/// untouched for the caller to report.
fn run_ipc_job(job: &IpcJob) -> IpcJobResult {
    let result = match IpcClient::call(&job.request) {
        Ok(response) => Ok(response),
        Err(error) => match &job.direct {
            Some(direct) if direct_fallback_allowed() => {
                if IpcClient::is_daemon_running() {
                    Err(error)
                } else {
                    match direct.apply_in(&crate::config::AppConfig::config_dir()) {
                        Ok(()) => {
                            crate::logging::info(
                                "CONFIG",
                                &format!("Daemon unreachable; {} applied directly to the config section", job.kind.label()),
                            );
                            Ok(IpcResponse::Ok)
                        }
                        Err(fallback) => {
                            Err(format!("{error}; direct config write failed: {fallback}"))
                        }
                    }
                }
            }
            _ => Err(error),
        },
    };
    IpcJobResult {
        kind: job.kind.clone(),
        result,
    }
}

/// Off-thread IPC gateway: the frame thread enqueues, the worker serializes round-trips.
struct IpcGateway {
    job_tx: std::sync::mpsc::Sender<IpcJob>,
    result_rx: std::sync::mpsc::Receiver<IpcJobResult>,
    sync_in_flight: bool,
    processes_in_flight: bool,
    /// When the last process snapshot was requested, to space retries that failed.
    last_process_request: Option<Instant>,
    /// Latest frame context, used to schedule a repaint when a request is queued so its result
    /// is applied promptly instead of waiting for the next input event.
    frame_ctx: Option<egui::Context>,
}

impl IpcGateway {
    fn new() -> Self {
        let (job_tx, job_rx) = std::sync::mpsc::channel::<IpcJob>();
        let (result_tx, result_rx) = std::sync::mpsc::channel::<IpcJobResult>();
        let spawned = std::thread::Builder::new()
            .name("synector-gui-ipc".into())
            .spawn(move || {
                while let Ok(job) = job_rx.recv() {
                    if result_tx.send(run_ipc_job(&job)).is_err() {
                        break;
                    }
                }
            });
        if let Err(err) = spawned {
            crate::logging::error("GUI", &format!("Cannot start the IPC worker: {err}"));
        }
        Self {
            job_tx,
            result_rx,
            sync_in_flight: false,
            processes_in_flight: false,
            last_process_request: None,
            frame_ctx: None,
        }
    }

    fn attach_frame_ctx(&mut self, ctx: &egui::Context) {
        self.frame_ctx = Some(ctx.clone());
    }

    /// Queues a request and requests a repaint for its result. Returns `false` when the worker
    /// is gone, so callers can clear their in-flight markers.
    fn send(&mut self, job: IpcJob) -> bool {
        if self.job_tx.send(job).is_err() {
            crate::logging::error("GUI", "IPC worker is not running; request dropped");
            return false;
        }
        if let Some(ctx) = &self.frame_ctx {
            ctx.request_repaint();
        }
        true
    }
}

/// One share-code compression job waiting behind the running one.
struct ShareExportJob {
    payload: crate::sharing::SharePayload,
    warnings: Vec<crate::sharing::ShareWarning>,
}

/// Share-code export bookkeeping: one compression runs at a time and further copies queue
/// instead of overwriting the running receiver (which used to lose the earlier result).
#[derive(Default)]
struct ShareExports {
    rx: Option<std::sync::mpsc::Receiver<ShareExportOutcome>>,
    queued: std::collections::VecDeque<ShareExportJob>,
}

/// Daemon sync cadence and outage bookkeeping.
struct DaemonSync {
    last_sync: Instant,
    last_success: Instant,
    last_error: Option<String>,
    /// Set while a state request is outstanding; used to detect a daemon that never answers.
    pending_since: Option<Instant>,
}

impl DaemonSync {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            // Start due so the first frame primes the state immediately.
            last_sync: now - std::time::Duration::from_secs(10),
            last_success: now - std::time::Duration::from_secs(10),
            last_error: None,
            pending_since: None,
        }
    }

    fn due(&self, executing: bool) -> bool {
        let interval = if executing {
            DAEMON_SYNC_FAST_MS
        } else {
            DAEMON_SYNC_IDLE_MS
        };
        self.last_sync.elapsed().as_millis() >= interval
    }

    fn mark_requested(&mut self) {
        self.last_sync = Instant::now();
        self.pending_since = Some(self.last_sync);
    }

    /// Makes the next poll due immediately (after a mutation, so the fresh state is fetched).
    fn force(&mut self) {
        self.last_sync = Instant::now() - std::time::Duration::from_secs(1);
    }

    fn note_success(&mut self) {
        self.last_success = Instant::now();
        self.last_error = None;
        self.pending_since = None;
    }

    /// Records a failed poll. Returns `Some(detail)` the first time a streak starts so the
    /// caller can log it once instead of on every 80 ms retry.
    fn note_failure(&mut self, error: String) -> Option<String> {
        let first = self.last_error.is_none();
        self.last_error = Some(error.clone());
        if first {
            Some(error)
        } else {
            None
        }
    }

    fn outage_confirmed(&self, consecutive_failures: u32) -> bool {
        consecutive_failures >= DAEMON_OUTAGE_POLLS
            && self.last_success.elapsed() >= DAEMON_OUTAGE_GRACE
    }
}

/// How long a locally advertised release is protected from a daemon snapshot produced before
/// the client's `SetAvailableUpdate` notification reached the daemon.
const UPDATE_ADVERTISE_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// Updater worker channel and the client-side release advertisement bookkeeping.
struct Updates {
    tx: std::sync::mpsc::Sender<crate::updater::UpdateEvent>,
    rx: std::sync::mpsc::Receiver<crate::updater::UpdateEvent>,
    /// When the client last changed the advertisement itself (manual check, cleared release).
    advertised_at: Option<Instant>,
}

impl Updates {
    fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        Self {
            tx,
            rx,
            advertised_at: None,
        }
    }

    /// Records a local change to the advertised release.
    fn advertise(&mut self) {
        self.advertised_at = Some(Instant::now());
    }

    /// The daemon is the authority, but an in-flight snapshot may predate our notification:
    /// only let it drop the local advertisement after that notification had time to arrive.
    fn daemon_may_clear(&self) -> bool {
        self.advertised_at
            .is_none_or(|at| at.elapsed() >= UPDATE_ADVERTISE_GRACE)
    }
}

/// A `SaveMacroPreset` awaiting its result. The recorded events stay in the action — the only
/// live copy until the preset is persisted — and are cleared only after confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingMacroSave {
    step_idx: usize,
    is_exit_pipeline: bool,
    preset_id: String,
}

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
            ipc: IpcGateway::new(),
            share: ShareExports::default(),
            daemon_sync: DaemonSync::new(),
            updates: Updates::new(),
            pending_macro_saves: Vec::new(),
        }
    }

    pub fn all_distinct_tags(&self) -> Vec<crate::models::RuleTag> {
        let mut map: std::collections::BTreeMap<String, crate::models::RuleTag> = std::collections::BTreeMap::new();
        if let Some(ds) = &self.daemon_state {
            for r in &ds.config.rules {
                for t in &r.tags {
                    map.entry(t.name.clone()).or_insert_with(|| t.clone());
                }
            }
        }
        if let Some(r) = &self.editing_rule {
            for t in &r.tags {
                map.entry(t.name.clone()).or_insert_with(|| t.clone());
            }
        }
        if let Some(r) = &self.builder_draft {
            for t in &r.tags {
                map.entry(t.name.clone()).or_insert_with(|| t.clone());
            }
        }
        map.into_values().collect()
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

    pub fn open_script_editor(
        &mut self,
        step_idx: usize,
        is_exit_pipeline: bool,
        language: crate::gui::components::script_highlighter::ScriptLanguage,
    ) {
        let editor = ExpandedScriptEditorState {
            step_idx,
            is_exit_pipeline,
            language,
        };
        self.expanded_script_editor = Some(editor.clone());
        self.last_script_editor = Some(editor);
    }

    pub fn close_script_editor(&mut self) {
        self.expanded_script_editor = None;
    }

    pub fn open_macro_editor(&mut self, step_idx: usize, is_exit_pipeline: bool) {
        if let Some(rule) = &mut self.editing_rule {
            let actions = if is_exit_pipeline { &mut rule.on_exit_actions } else { &mut rule.actions };
            if let Some(crate::models::Action::Macro { preset_id: Some(ref pid), events, repeat, speed, infinite, .. }) = actions.get_mut(step_idx) {
                if events.is_empty() {
                    let library = crate::config::Library::read();
                    if let Some(preset) = library.find_macro(pid) {
                        *events = preset.events.clone();
                        *repeat = preset.repeat;
                        *speed = preset.speed;
                        *infinite = preset.infinite;
                    }
                }
            }
        }
        let editor = MacroEditorModalState {
            step_idx,
            is_exit_pipeline,
        };
        self.macro_editor = Some(editor.clone());
        self.last_macro_editor = Some(editor);
        self.macro_preset_save_name = self
            .editing_rule
            .as_ref()
            .map(|r| r.name.clone())
            .unwrap_or_else(|| self.loc().macro_studio_custom_preset_name.to_string());
        crate::gui::macro_recorder::set_studio_active(true);
    }

    pub fn close_macro_editor(&mut self) {
        let rec = crate::gui::macro_recorder::MacroRecorder::global();
        if rec.is_active() {
            rec.stop_recording();
        }
        crate::gui::macro_recorder::stop_macro_test();
        if let Some(edit_info) = self.macro_editor.take() {
            let mut pending_preset = None;
            if let Some(rule) = &mut self.editing_rule {
                let actions = if edit_info.is_exit_pipeline { &mut rule.on_exit_actions } else { &mut rule.actions };
                if let Some(crate::models::Action::Macro { preset_id: Some(ref pid), events, repeat, speed, infinite, .. }) = actions.get_mut(edit_info.step_idx) {
                    let library = crate::config::Library::read();
                    let preset_name = library.find_macro(pid).map(|p| p.name.clone()).unwrap_or_else(|| pid.clone());
                    // The recorded events stay in the action until the daemon confirms the preset:
                    // they are the only live copy and the engine hydrates empty events from the
                    // library, so clearing them on a failed save would lose the recording.
                    pending_preset = Some(crate::models::MacroPreset {
                        id: pid.clone(),
                        name: preset_name,
                        events: events.clone(),
                        repeat: *repeat,
                        speed: *speed,
                        infinite: *infinite,
                    });
                }
            }
            if let Some(preset) = pending_preset {
                self.pending_macro_saves.push(PendingMacroSave {
                    step_idx: edit_info.step_idx,
                    is_exit_pipeline: edit_info.is_exit_pipeline,
                    preset_id: preset.id.clone(),
                });
                let direct = DirectWrite::Library(DirectLibraryChange::UpsertMacroPreset(Box::new(preset.clone())));
                self.enqueue_ipc(
                    IpcJobKind::SaveMacroPreset { preset_id: preset.id.clone() },
                    IpcRequest::SaveMacroPreset(preset),
                    Some(direct),
                );
            }
        }
        crate::gui::macro_recorder::set_studio_active(false);
    }

    /// Clears the recorded events only once the preset is persisted. On failure the recording
    /// stays in the action — it is the only live copy, and the engine hydrates empty events from
    /// the library — and the error is surfaced instead of a silent loss.
    fn apply_macro_save_result(&mut self, preset_id: &str, result: Result<IpcResponse, String>) {
        let failure = Self::mutation_failure(result);
        let pending = self
            .pending_macro_saves
            .iter()
            .position(|pending| pending.preset_id == preset_id)
            .map(|position| self.pending_macro_saves.remove(position));
        match (failure, pending) {
            (None, Some(pending)) => {
                if let Some(rule) = &mut self.editing_rule {
                    let actions = if pending.is_exit_pipeline {
                        &mut rule.on_exit_actions
                    } else {
                        &mut rule.actions
                    };
                    if let Some(crate::models::Action::Macro { preset_id: Some(ref pid), events, .. }) =
                        actions.get_mut(pending.step_idx)
                    {
                        if pid.as_str() == preset_id {
                            events.clear();
                        }
                    }
                }
            }
            (None, None) => {}
            (Some(failure), _) => {
                crate::logging::warn(
                    "GUI",
                    &format!(
                        "SaveMacroPreset '{preset_id}' was not applied: {}",
                        failure.diagnostic()
                    ),
                );
                self.set_toast(self.mutation_failure_toast(&failure));
            }
        }
    }

    pub fn open_overlay_studio(&mut self, overlay_id: &str) {
        let library = crate::config::Library::read();
        let overlay = library
            .overlays
            .iter()
            .find(|o| o.id == overlay_id)
            .cloned()
            .unwrap_or_else(|| default_overlay_for(overlay_id));
        self.show_overlay_studio(OverlayStudioState::new(overlay, None, false, String::new()));
    }

    pub fn open_overlay_studio_for_action(&mut self, action_idx: usize, is_exit: bool) {
        let studio = {
            let Some(rule) = &self.editing_rule else { return };
            let actions = if is_exit { &rule.on_exit_actions } else { &rule.actions };
            let Some(crate::models::Action::Overlay { overlay_id, custom_overlay, .. }) = actions.get(action_idx) else {
                return;
            };
            let overlay = if let Some(cust) = custom_overlay {
                (**cust).clone()
            } else {
                let library = crate::config::Library::read();
                library
                    .overlays
                    .iter()
                    .find(|o| o.id == *overlay_id)
                    .cloned()
                    .unwrap_or_else(|| default_overlay_for(overlay_id))
            };
            let preset_save_name = overlay.name.clone();
            OverlayStudioState::new(overlay, Some(action_idx), is_exit, preset_save_name)
        };
        self.show_overlay_studio(studio);
    }

    fn show_overlay_studio(&mut self, studio: OverlayStudioState) {
        self.overlay_studio = Some(studio.clone());
        self.last_overlay_studio = Some(studio);
    }

    pub fn close_overlay_studio(&mut self) {
        let Some(studio) = self.overlay_studio.as_ref() else { return };
        let overlay = studio.overlay.clone();
        let action_target = studio.action_target_idx;
        let is_exit_pipeline = studio.is_exit_pipeline;
        let _ = IpcClient::call(&IpcRequest::HideOverlayPreview(overlay.id.clone()));
        let mut save_to_library = action_target.is_none();
        if let Some(action_idx) = action_target {
            if let Some(rule) = &mut self.editing_rule {
                let actions = if is_exit_pipeline {
                    &mut rule.on_exit_actions
                } else {
                    &mut rule.actions
                };
                if let Some(crate::models::Action::Overlay { custom_overlay, overlay_id, .. }) = actions.get_mut(action_idx) {
                    if custom_overlay.is_some() {
                        // Inline definitions live in the rule action, not in the library.
                        *custom_overlay = Some(Box::new(overlay.clone()));
                        *overlay_id = overlay.id.clone();
                    } else {
                        save_to_library = true;
                    }
                }
            }
        }
        if save_to_library {
            let direct = DirectWrite::Library(DirectLibraryChange::UpsertOverlay(Box::new(overlay.clone())));
            self.enqueue_ipc(
                IpcJobKind::SaveOverlay { overlay_id: overlay.id.clone() },
                IpcRequest::SaveOverlay(overlay),
                Some(direct),
            );
        }
        self.overlay_studio = None;
    }

    pub fn open_action_catalog(&mut self, is_exit_pipeline: bool, insert_idx: Option<usize>) {
        let catalog = ActionCatalogModalState {
            is_exit_pipeline,
            insert_idx,
            selected_category: 0,
            search_query: String::new(),
            ifelse_target: None,
        };
        self.action_catalog = Some(catalog.clone());
        self.last_action_catalog = Some(catalog);
    }

    pub fn open_action_catalog_nested(&mut self, is_exit_pipeline: bool, target: IfElseCatalogTarget) {
        let catalog = ActionCatalogModalState {
            is_exit_pipeline,
            insert_idx: None,
            selected_category: 0,
            search_query: String::new(),
            ifelse_target: Some(target),
        };
        self.action_catalog = Some(catalog.clone());
        self.last_action_catalog = Some(catalog);
    }

    pub fn close_action_catalog(&mut self) {
        self.action_catalog = None;
    }

    pub fn open_trigger_catalog(&mut self) {
        let catalog = TriggerCatalogState::default();
        self.trigger_catalog = Some(catalog.clone());
        self.last_trigger_catalog = Some(catalog);
    }

    pub fn close_trigger_catalog(&mut self) {
        self.trigger_catalog = None;
    }

    pub fn set_trigger_from_catalog(&mut self, trigger: Option<crate::models::Trigger>) {
        self.push_undo_snapshot();
        if let Some(rule) = &mut self.editing_rule {
            rule.trigger = trigger;
        }
        self.close_trigger_catalog();
    }

    pub fn open_condition_catalog_with_target(&mut self, target: ConditionCatalogTarget) {
        let catalog = ConditionCatalogState {
            target: Some(target),
            ..Default::default()
        };
        self.condition_catalog = Some(catalog.clone());
        self.last_condition_catalog = Some(catalog);
    }

    #[cfg(test)]
    pub fn open_condition_catalog(&mut self) {
        self.open_condition_catalog_with_target(ConditionCatalogTarget::RulePrecondition);
    }

    pub fn close_condition_catalog(&mut self) {
        self.condition_catalog = None;
    }

    pub fn open_element_catalog(&mut self) {
        let catalog = ElementCatalogModalState::default();
        self.element_catalog = Some(catalog.clone());
        self.last_element_catalog = Some(catalog);
    }

    pub fn close_element_catalog(&mut self) {
        self.element_catalog = None;
    }

    pub fn apply_condition_from_catalog(&mut self, condition: crate::models::RuleCondition) {
        self.push_undo_snapshot();
        let target = self
            .condition_catalog
            .as_ref()
            .and_then(|c| c.target)
            .unwrap_or(ConditionCatalogTarget::RulePrecondition);

        if let Some(rule) = &mut self.editing_rule {
            match target {
                ConditionCatalogTarget::RulePrecondition => {
                    rule.conditions.push(crate::models::RuleConditionItem::new(condition));
                }
                ConditionCatalogTarget::ChangeRulePrecondition(idx) => {
                    if let Some(item) = rule.conditions.get_mut(idx) {
                        item.condition = condition;
                    }
                }
                ConditionCatalogTarget::ActionIfElse { action_idx, is_exit } => {
                    let actions = if is_exit { &mut rule.on_exit_actions } else { &mut rule.actions };
                    if let Some(crate::models::Action::IfElse { conditions, .. }) = actions.get_mut(action_idx) {
                        conditions.push(crate::models::RuleConditionItem::new(condition));
                    }
                }
                ConditionCatalogTarget::ChangeActionIfElse { action_idx, cond_idx, is_exit } => {
                    let actions = if is_exit { &mut rule.on_exit_actions } else { &mut rule.actions };
                    if let Some(crate::models::Action::IfElse { conditions, .. }) = actions.get_mut(action_idx) {
                        if let Some(item) = conditions.get_mut(cond_idx) {
                            item.condition = condition;
                        }
                    }
                }
                ConditionCatalogTarget::ActionElseIf { action_idx, branch_idx, is_exit } => {
                    let actions = if is_exit { &mut rule.on_exit_actions } else { &mut rule.actions };
                    if let Some(crate::models::Action::IfElse { else_if_branches, .. }) = actions.get_mut(action_idx) {
                        if let Some(branch) = else_if_branches.get_mut(branch_idx) {
                            branch.conditions.push(crate::models::RuleConditionItem::new(condition));
                        }
                    }
                }
                ConditionCatalogTarget::ChangeActionElseIf { action_idx, branch_idx, cond_idx, is_exit } => {
                    let actions = if is_exit { &mut rule.on_exit_actions } else { &mut rule.actions };
                    if let Some(crate::models::Action::IfElse { else_if_branches, .. }) = actions.get_mut(action_idx) {
                        if let Some(branch) = else_if_branches.get_mut(branch_idx) {
                            if let Some(item) = branch.conditions.get_mut(cond_idx) {
                                item.condition = condition;
                            }
                        }
                    }
                }
            }
        }
        self.close_condition_catalog();
    }

    pub fn add_action_from_catalog(
        &mut self,
        action: crate::models::Action,
        is_exit_pipeline: bool,
        insert_idx: Option<usize>,
        ifelse_target: Option<IfElseCatalogTarget>,
    ) {
        self.push_undo_snapshot();
        if let Some(rule) = &mut self.editing_rule {
            if let Some(target) = ifelse_target {
                let list = if is_exit_pipeline {
                    &mut rule.on_exit_actions
                } else {
                    &mut rule.actions
                };
                match target {
                    IfElseCatalogTarget::Then(parent_idx) => {
                        if let Some(crate::models::Action::IfElse { then_actions, .. }) = list.get_mut(parent_idx) {
                            then_actions.push(action);
                        }
                    }
                    IfElseCatalogTarget::ElseIf { parent_idx, branch_idx } => {
                        if let Some(crate::models::Action::IfElse { else_if_branches, .. }) = list.get_mut(parent_idx) {
                            if let Some(branch) = else_if_branches.get_mut(branch_idx) {
                                branch.actions.push(action);
                            }
                        }
                    }
                    IfElseCatalogTarget::Else(parent_idx) => {
                        if let Some(crate::models::Action::IfElse { else_actions, .. }) = list.get_mut(parent_idx) {
                            else_actions.push(action);
                        }
                    }
                }
            } else {
                let list = if is_exit_pipeline {
                    &mut rule.on_exit_actions
                } else {
                    &mut rule.actions
                };
                if let Some(idx) = insert_idx {
                    let clamped = idx.min(list.len());
                    list.insert(clamped, action);
                } else {
                    list.push(action);
                }
            }
        }
        self.close_action_catalog();
    }

    pub fn set_toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    /// Queues a request to the IPC worker. `direct` is the section write used only when the
    /// daemon is unreachable; the frame thread never sees the round-trip.
    fn enqueue_ipc(&mut self, kind: IpcJobKind, request: IpcRequest, direct: Option<DirectWrite>) -> bool {
        self.ipc.send(IpcJob { kind, request, direct })
    }

    /// `Some(failure)` unless the daemon (or the direct fallback) confirmed the mutation.
    fn mutation_failure(result: Result<IpcResponse, String>) -> Option<MutationFailure> {
        match result {
            Ok(IpcResponse::Ok) => None,
            Ok(IpcResponse::Error(error)) => Some(MutationFailure::Detail(error)),
            Ok(other) => Some(MutationFailure::UnexpectedResponse(format!(
                "unexpected {} response",
                ipc_response_name(&other)
            ))),
            Err(error) => Some(MutationFailure::Detail(error)),
        }
    }

    /// Localized toast line for a mutation the daemon did not confirm.
    fn mutation_failure_toast(&self, failure: &MutationFailure) -> String {
        match failure {
            MutationFailure::Detail(detail) => self.loc().toast_request_failed.replace("{}", detail),
            MutationFailure::UnexpectedResponse(_) => {
                self.loc().toast_request_failed_unexpected.to_string()
            }
        }
    }

    /// Reports a mutation that was not applied and keeps whatever draft was in flight.
    fn report_mutation_failure(&mut self, kind: &IpcJobKind, failure: &MutationFailure) {
        crate::logging::warn(
            "GUI",
            &format!("{} was not applied: {}", kind.label(), failure.diagnostic()),
        );
        self.set_toast(self.mutation_failure_toast(failure));
    }

    /// Drops a release the daemon no longer advertises (nothing found by a manual check, the
    /// daemon restarted, the update was applied) so the footer and changelog stop offering it.
    fn clear_available_update(&mut self) {
        self.updates.advertise();
        if self.available_update.take().is_some() {
            self.show_changelog_modal = false;
        }
    }

    pub fn toggle_autorun(&mut self, enable: bool) {
        self.enqueue_ipc(IpcJobKind::SetAutorun { enabled: enable }, IpcRequest::SetAutorun(enable), None);
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

    /// Drains finished IPC work and queues a daemon poll when one is due. Called once per frame
    /// from `SynectorApp::ui`; it never blocks on the named pipe.
    pub fn sync_with_daemon(&mut self) {
        self.drain_ipc_results();
        self.check_sync_timeout();
        if self.ipc.sync_in_flight || !self.daemon_sync.due(self.is_any_rule_executing()) {
            return;
        }
        self.daemon_sync.mark_requested();
        if self.enqueue_ipc(IpcJobKind::Sync, IpcRequest::GetState, None) {
            self.ipc.sync_in_flight = true;
        }
    }

    /// An outstanding poll that never returns must not stall the client silently: treat it as a
    /// failure and let the cadence retry (the hung request stays queued in the worker).
    fn check_sync_timeout(&mut self) {
        let Some(since) = self.daemon_sync.pending_since else { return };
        if since.elapsed() < DAEMON_SYNC_TIMEOUT {
            return;
        }
        self.ipc.sync_in_flight = false;
        self.daemon_sync.pending_since = None;
        self.note_sync_failure("the daemon did not answer the state request within 2 s".to_string());
    }

    /// Applies every completed job on the frame thread.
    fn drain_ipc_results(&mut self) {
        while let Ok(result) = self.ipc.result_rx.try_recv() {
            self.apply_ipc_result(result);
        }
    }

    fn apply_ipc_result(&mut self, job: IpcJobResult) {
        let IpcJobResult { kind, result } = job;
        match &kind {
            IpcJobKind::Sync => {
                self.ipc.sync_in_flight = false;
                match result {
                    Ok(IpcResponse::State(state)) => self.apply_daemon_state(state),
                    Ok(IpcResponse::Error(error)) => self.note_sync_failure(error),
                    Ok(other) => self.note_sync_failure(format!("unexpected {} response", ipc_response_name(&other))),
                    Err(error) => self.note_sync_failure(error),
                }
            }
            IpcJobKind::Processes => {
                self.ipc.processes_in_flight = false;
                match result {
                    Ok(IpcResponse::Processes(processes)) => self.cached_processes = processes,
                    Ok(other) => crate::logging::warn(
                        "GUI",
                        &format!("Process snapshot failed: unexpected {} response", ipc_response_name(&other)),
                    ),
                    // Background refresh: a daemon outage is already reported by the state sync,
                    // so the transport detail is logged at debug to avoid per-retry noise.
                    Err(error) => crate::logging::debug("GUI", &format!("Process snapshot failed: {error}")),
                }
            }
            IpcJobKind::SaveRule { rule_id } => self.apply_save_rule_result(rule_id, result),
            IpcJobKind::CloneRule { .. } => {
                if let Some(failure) = Self::mutation_failure(result) {
                    self.report_mutation_failure(&kind, &failure);
                }
                self.daemon_sync.force();
            }
            IpcJobKind::DeleteRule { rule_id } => {
                match Self::mutation_failure(result) {
                    None => {
                        if self.selected_rule_id.as_deref() == Some(rule_id.as_str()) {
                            self.selected_rule_id = None;
                            self.editing_rule = None;
                            self.is_creating_new_rule = false;
                        }
                        self.set_toast(self.loc().toast_rule_deleted);
                    }
                    Some(failure) => self.report_mutation_failure(&kind, &failure),
                }
                self.daemon_sync.force();
            }
            IpcJobKind::ToggleRule { .. }
            | IpcJobKind::ReorderRules
            | IpcJobKind::ResetRule { .. }
            | IpcJobKind::SaveOverlay { .. }
            | IpcJobKind::StopRuleExecution { .. } => {
                if let Some(failure) = Self::mutation_failure(result) {
                    self.report_mutation_failure(&kind, &failure);
                }
                self.daemon_sync.force();
            }
            IpcJobKind::SaveMacroPreset { preset_id } => self.apply_macro_save_result(preset_id, result),
            IpcJobKind::TestRunRule { rule_id } => self.apply_test_run_result(rule_id, result),
            IpcJobKind::ImportShare => self.apply_import_result(result),
            IpcJobKind::SetAutorun { enabled } => self.apply_autorun_result(*enabled, result),
            IpcJobKind::StartUpdate => self.apply_start_update_result(result),
            IpcJobKind::Background => {
                if let Err(error) = result {
                    crate::logging::debug("GUI", &format!("{} failed: {error}", kind.label()));
                }
            }
        }
    }

    fn apply_daemon_state(&mut self, state: DaemonState) {
        self.has_connected_daemon = true;
        self.failed_daemon_syncs = 0;
        self.is_daemon_connected = true;
        self.daemon_sync.note_success();
        let rules_action_counts: std::collections::HashMap<String, usize> = state
            .config
            .rules
            .iter()
            .map(|r| (r.id.clone(), r.actions.len()))
            .collect();

        self.active_test_runs.retain(|id, started_at| {
            if state.active_test_rule_ids.contains(id) {
                return true;
            }
            let num_actions = rules_action_counts.get(id).copied().unwrap_or(1);
            let min_dur = std::time::Duration::from_millis(((num_actions * 400).max(900) as u64).min(4000));
            started_at.elapsed() < min_dur || state.active_rule_executions.contains_key(id)
        });
        for id in &state.active_test_rule_ids {
            self.active_test_runs.entry(id.clone()).or_insert_with(Instant::now);
        }

        match state.available_update {
            Some(ref update) => {
                if self.available_update.as_ref().map(|u| &u.version) != Some(&update.version) {
                    self.available_update = Some(update.clone());
                }
            }
            // The daemon is the authority: a manual check that found nothing and a fresh daemon
            // must not leave the client offering a stale release. A snapshot that was in flight
            // while the client changed the advertisement cannot be distinguished from a fresh
            // one, so it is ignored for the grace period.
            None => {
                if self.updates.daemon_may_clear() {
                    self.clear_available_update();
                }
            }
        }
        if let Some(pct) = state.update_progress {
            self.is_downloading_update = true;
            self.update_download_progress = pct;
        } else if self.is_downloading_update {
            self.is_downloading_update = false;
        }
        if state.pending_open_update_modal {
            self.show_changelog_modal = true;
            self.enqueue_ipc(
                IpcJobKind::Background,
                IpcRequest::AcknowledgeOpenUpdateModal,
                None,
            );
        }
        if state.pending_auto_start_update && !self.is_downloading_update {
            if let Some(ref update) = state.available_update {
                self.enqueue_ipc(
                    IpcJobKind::Background,
                    IpcRequest::AcknowledgeAutoStartUpdate,
                    None,
                );
                self.start_update(update.clone());
            }
        }

        self.daemon_state = Some(state);
    }

    fn note_sync_failure(&mut self, error: String) {
        self.failed_daemon_syncs = self.failed_daemon_syncs.saturating_add(1);
        if let Some(first) = self.daemon_sync.note_failure(error) {
            crate::logging::warn("GUI", &format!("Daemon state sync failed: {first}"));
        }
        // A single missed poll is not an outage: report the disconnected state only after a
        // sustained stretch of failures, with the collected error detail.
        if self.daemon_sync.outage_confirmed(self.failed_daemon_syncs) && self.is_daemon_connected {
            let last_error = self.daemon_sync.last_error.clone();
            crate::logging::warn(
                "GUI",
                &format!(
                    "Daemon unreachable after {} failed polls: {}",
                    self.failed_daemon_syncs,
                    last_error.as_deref().unwrap_or("no response")
                ),
            );
            self.is_daemon_connected = false;
            let detail = last_error.as_deref().unwrap_or(self.loc().toast_daemon_no_response);
            self.set_toast(format!("{}: {detail}", self.loc().settings_daemon_offline));
        }
    }

    pub fn copy_single_rule_to_clipboard(&mut self, _ctx: &egui::Context, rule: &Rule) {
        self.start_share_export(vec![rule.clone()]);
    }

    /// Copies every automation currently visible in the rules view: the same tag filter and
    /// search that drive the list, so sharing a subset no longer requires deleting rules.
    pub fn copy_all_rules_to_clipboard(&mut self, _ctx: &egui::Context) {
        let rules = self.visible_rules();
        if rules.is_empty() {
            self.set_toast(self.loc().toast_copy_all_empty);
            return;
        }
        self.start_share_export(rules);
    }

    /// Rules matching the active tag filter and search query.
    pub fn visible_rules(&self) -> Vec<Rule> {
        let rules = self
            .daemon_state
            .as_ref()
            .map(|s| s.config.rules.clone())
            .unwrap_or_default();
        let needle = self.search_query.trim().to_lowercase();
        rules
            .into_iter()
            .filter(|rule| {
                if let Some(tag) = &self.selected_tag_filter {
                    if !rule.tags.iter().any(|t| t.name.eq_ignore_ascii_case(tag)) {
                        return false;
                    }
                }
                if needle.is_empty() {
                    return true;
                }
                rule.name.to_lowercase().contains(&needle)
                    || rule
                        .tags
                        .iter()
                        .any(|t| t.name.to_lowercase().contains(&needle))
            })
            .collect()
    }

    fn share_library(&self) -> (Vec<crate::models::OverlayDefinition>, Vec<crate::models::MacroPreset>) {
        match &self.daemon_state {
            Some(ds) => (ds.config.overlays.clone(), ds.config.macro_presets.clone()),
            None => {
                let library = crate::config::Library::read();
                (library.overlays, library.macro_presets)
            }
        }
    }

    fn start_share_export(&mut self, rules: Vec<Rule>) {
        let (overlays, presets) = self.share_library();
        let (payload, warnings) = crate::sharing::build_payload(rules, &overlays, &presets);
        for warning in &warnings {
            // Notices are advisory: they go to the log and are summarized in the toast, but
            // copying stays a single click.
            crate::logging::warn("SHARE", &self.share_warning_text(warning));
        }
        if self.share.rx.is_some() {
            // A second Copy must not drop the running export's receiver: the earlier code would
            // have swallowed its result and only the second code reached the clipboard.
            let rule_count = payload.rules.len();
            self.share.queued.push_back(ShareExportJob { payload, warnings });
            crate::logging::info(
                "SHARE",
                &format!("Queued share code for {rule_count} automation(s) behind the running export"),
            );
            return;
        }
        self.spawn_share_export(payload, warnings);
    }

    fn spawn_share_export(
        &mut self,
        payload: crate::sharing::SharePayload,
        warnings: Vec<crate::sharing::ShareWarning>,
    ) {
        let rule_count = payload.rules.len();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = crate::sharing::compress_and_encode(&payload);
            let _ = tx.send((result, rule_count, warnings));
        });
        self.share.rx = Some(rx);
        // Keep a frame coming so the result reaches the clipboard without waiting for input.
        if let Some(ctx) = &self.ipc.frame_ctx {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        crate::logging::info(
            "SHARE",
            &format!("Building share code for {rule_count} automation(s) in the background"),
        );
    }

    fn start_next_queued_export(&mut self) {
        if self.share.rx.is_none() {
            if let Some(next) = self.share.queued.pop_front() {
                self.spawn_share_export(next.payload, next.warnings);
            }
        }
    }

    /// Drains the background export channel; called once per frame from the app loop.
    pub fn poll_share_events(&mut self, ctx: &egui::Context) {
        self.ipc.attach_frame_ctx(ctx);
        enum Polled {
            Outcome(ShareExportOutcome),
            Empty,
            Closed,
        }
        let polled = match self.share.rx.as_ref().map(|rx| rx.try_recv()) {
            None => {
                self.start_next_queued_export();
                return;
            }
            Some(Ok(outcome)) => Polled::Outcome(outcome),
            Some(Err(std::sync::mpsc::TryRecvError::Empty)) => Polled::Empty,
            Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => Polled::Closed,
        };
        match polled {
            Polled::Outcome((Ok(code), count, warnings)) => {
                self.share.rx = None;
                crate::sharing::copy_text_to_clipboard(ctx, &code);
                let mut message = self.loc().toast_share_copied.replace("{}", &count.to_string());
                if !warnings.is_empty() {
                    let notices = self
                        .loc()
                        .toast_share_warnings
                        .replace("{}", &warnings.len().to_string());
                    message = format!("{message} · {notices}");
                }
                self.set_toast(message);
                crate::logging::info(
                    "SHARE",
                    &format!(
                        "Share code copied: {count} automation(s), {} byte(s), {} notice(s)",
                        code.len(),
                        warnings.len()
                    ),
                );
                self.start_next_queued_export();
            }
            Polled::Outcome((Err(error), _, _)) => {
                self.share.rx = None;
                let detail = self.share_error_text(&error);
                self.set_toast(self.loc().toast_share_failed.replace("{}", &detail));
                crate::logging::warn("SHARE", &format!("Share code export failed: {error}"));
                self.start_next_queued_export();
            }
            Polled::Empty => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
            Polled::Closed => {
                self.share.rx = None;
                self.start_next_queued_export();
            }
        }
    }

    pub fn create_shortcut_dialog(&mut self, rule: &crate::models::Rule) {
        let desktop = dirs_desktop().or_else(|| std::env::var_os("USERPROFILE").map(|u| std::path::PathBuf::from(u).join("Desktop")));
        let mut dialog = rfd::FileDialog::new().set_title(self.loc().dialog_select_shortcut_folder);
        if let Some(d) = desktop {
            dialog = dialog.set_directory(d);
        }
        if let Some(folder) = dialog.pick_folder() {
            match crate::shortcut::create_rule_shortcut(rule, &folder) {
                Ok(path) => {
                    let path_str = path.to_string_lossy();
                    self.set_toast(self.loc().toast_shortcut_created.replace("{}", &path_str));
                }
                Err(err) => {
                    self.set_toast(self.loc().toast_shortcut_failed.replace("{}", &err));
                }
            }
        }
    }

    /// Imports the share code currently in the clipboard in one step.
    ///
    /// Rules are created disabled; the daemon applies the code atomically and reports how many
    /// rules, library entries and duplicates it handled, which the toast summarizes.
    pub fn import_automations_from_clipboard(&mut self, _ctx: &egui::Context) {
        let text = match crate::sharing::read_text_from_clipboard() {
            Ok(text) => text,
            Err(error) => {
                self.set_toast(format!("{}: {}", self.loc().toast_import_clipboard_err, error));
                return;
            }
        };

        let payload = match crate::sharing::decode_and_decompress(&text) {
            Ok(payload) => payload,
            Err(error) => {
                let detail = self.share_error_text(&error);
                crate::logging::warn("SHARE", &format!("Share code rejected: {error}"));
                self.set_toast(format!("{}: {}", self.loc().toast_import_invalid, detail));
                return;
            }
        };

        let request = IpcRequest::ImportShare {
            payload: Box::new(payload),
            enable: false,
        };
        self.enqueue_ipc(IpcJobKind::ImportShare, request, None);
    }

    /// Applies the daemon's answer to a decoded share code. Imported rules are disabled and the
    /// toast summarizes what the daemon actually applied.
    fn apply_import_result(&mut self, result: Result<IpcResponse, String>) {
        match result {
            Ok(IpcResponse::ShareImportResult(summary)) => {
                let mut message = if summary.imported == 0 {
                    self.loc().share_import_nothing_new.to_string()
                } else {
                    self.loc()
                        .toast_import_success
                        .replace("{}", &summary.imported.to_string())
                };
                if summary.skipped_duplicates > 0 {
                    message.push_str(" · ");
                    message.push_str(
                        &self
                            .loc()
                            .toast_import_skipped
                            .replace("{}", &summary.skipped_duplicates.to_string()),
                    );
                }
                if summary.imported > 0 {
                    message.push_str(" · ");
                    message.push_str(self.loc().toast_import_disabled);
                }
                self.set_toast(message);
                crate::logging::info(
                    "SHARE",
                    &format!(
                        "Import applied: {} rule(s), {} overlay(s), {} preset(s), {} skipped",
                        summary.imported,
                        summary.overlays,
                        summary.macro_presets,
                        summary.skipped_duplicates
                    ),
                );
                self.daemon_sync.force();
            }
            Ok(IpcResponse::Error(error)) => {
                crate::logging::warn("SHARE", &format!("Import rejected by daemon: {error}"));
                self.set_toast(self.loc().toast_import_failed.replace("{}", &error));
            }
            Ok(_) => {
                self.set_toast(self.loc().toast_request_failed_unexpected);
            }
            Err(error) => {
                crate::logging::warn("SHARE", &format!("Import IPC failed: {error}"));
                self.set_toast(self.loc().toast_import_failed.replace("{}", &error));
            }
        }
    }

    /// Localized explanation of a decoding failure.
    pub fn share_error_text(&self, error: &crate::sharing::ShareError) -> String {
        use crate::sharing::ShareError as E;
        let loc = self.loc();
        match error {
            E::EmptyInput => loc.share_err_empty.to_string(),
            E::LegacyFormat => loc.share_err_legacy.to_string(),
            E::MissingPrefix => loc.share_err_missing_prefix.to_string(),
            E::CodeTooLong => loc.share_err_code_too_long.to_string(),
            E::InvalidBase64 => loc.share_err_invalid_base64.to_string(),
            E::CorruptedPayload => loc.share_err_corrupted.to_string(),
            E::ChecksumMismatch => loc.share_err_checksum.to_string(),
            E::PayloadTooLarge => loc.share_err_payload_too_large.to_string(),
            E::TooManyRules => loc.share_err_too_many_rules.to_string(),
            E::TooManyLibraryItems => loc.share_err_too_many_library_items.to_string(),
            E::EmptyPayload => loc.share_err_empty_payload.to_string(),
            E::InvalidPayload => loc.share_err_invalid_payload.to_string(),
            E::UnsupportedVersion(version) => {
                format!("{} (v{version})", loc.share_err_unsupported_version)
            }
        }
    }

    /// Localized line for a share-code notice.
    pub fn share_warning_text(&self, warning: &crate::sharing::ShareWarning) -> String {
        use crate::sharing::ShareWarning as W;
        let loc = self.loc();
        match warning {
            W::MissingMacroPreset(id) => loc.share_warn_missing_macro.replace("{}", id),
            W::MissingOverlay(id) => loc.share_warn_missing_overlay.replace("{}", id),
            W::SensitiveScript => loc.share_warn_script.to_string(),
            W::SensitiveAdminProcess => loc.share_warn_admin.to_string(),
            W::SensitiveWebhook => loc.share_warn_webhook.to_string(),
            W::CustomIconDropped => loc.share_warn_icon.to_string(),
            W::OverlayAssetDropped => loc.share_warn_asset.to_string(),
            W::MissingRule(id) => loc.share_warn_missing_rule.replace("{}", id),
        }
    }


    /// Fetches a process snapshot when the cache is still empty. Non-blocking: the result is
    /// applied by the next frame.
    pub fn ensure_processes(&mut self) {
        self.request_process_snapshot(false);
    }

    /// Queues a `GetProcesses` round-trip unless one is already in flight. `force` is used when
    /// the picker opens, so the list is fresh exactly when the user asks for it.
    fn request_process_snapshot(&mut self, force: bool) {
        if self.ipc.processes_in_flight {
            return;
        }
        if !force {
            if !self.cached_processes.is_empty() {
                return;
            }
            if self
                .ipc
                .last_process_request
                .is_some_and(|at| at.elapsed() < PROCESS_SNAPSHOT_RETRY)
            {
                return;
            }
        }
        if self.enqueue_ipc(IpcJobKind::Processes, IpcRequest::GetProcesses, None) {
            self.ipc.processes_in_flight = true;
            self.ipc.last_process_request = Some(Instant::now());
        }
    }


    pub fn initial_new_rule(&self) -> Rule {
        Rule {
            id: format!("rule_{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()),
            name: String::new(),
            description: String::new(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            max_executions: None,
            quota_period: None,
            execution_count: 0,
            stop_on_error: true,
            trigger: Some(crate::models::Trigger::Manual),
            actions: Vec::new(),
            on_exit_actions: Vec::new(),
            custom_icon_path: None,
            auto_process_icon: false,
            conditions: Vec::new(),
            last_executed_at: None,
            quota_window_start: None,
            tags: Vec::new(),
        }
    }

    pub fn push_undo_snapshot(&mut self) {
        if let Some(r) = &self.editing_rule {
            if self.undo_stack.last() != Some(r) {
                self.undo_stack.push(r.clone());
                if self.undo_stack.len() > 50 {
                    self.undo_stack.remove(0);
                }
                self.redo_stack.clear();
            }
        }
    }

    pub fn push_undo_snapshot_of(&mut self, r: &Rule) {
        if self.undo_stack.last() != Some(r) {
            self.undo_stack.push(r.clone());
            if self.undo_stack.len() > 50 {
                self.undo_stack.remove(0);
            }
            self.redo_stack.clear();
        }
    }

    pub fn undo(&mut self) {
        if let Some(prev) = self.undo_stack.pop() {
            if let Some(curr) = self.editing_rule.take() {
                self.redo_stack.push(curr);
            }
            self.editing_rule = Some(prev);
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo_stack.pop() {
            if let Some(curr) = self.editing_rule.take() {
                self.undo_stack.push(curr);
            }
            self.editing_rule = Some(next);
        }
    }

    pub fn start_new_rule(&mut self) {
        self.selected_rule_id = None;
        self.selected_process = None;
        self.is_creating_new_rule = true;
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.tag_picker_open = false;
        self.new_tag_name.clear();
        self.new_tag_color = Some("#3B82F6".into());
        self.new_tag_icon = None;
        let rule = self.builder_draft.take().unwrap_or_else(|| self.initial_new_rule());
        self.editing_rule = Some(rule);
    }

    pub fn reset_builder_state(&mut self) {
        self.builder_draft = None;
        self.close_action_catalog();
        if self.is_creating_new_rule {
            self.editing_rule = Some(self.initial_new_rule());
        }
    }

    pub fn extract_action_overlay_ids(action: &crate::models::Action, out: &mut Vec<String>) {
        match action {
            crate::models::Action::Overlay { overlay_id, custom_overlay, .. } => {
                out.push(overlay_id.clone());
                if let Some(cust) = custom_overlay {
                    out.push(cust.id.clone());
                }
            }
            crate::models::Action::IfElse { then_actions, else_if_branches, else_actions, .. } => {
                for a in then_actions {
                    Self::extract_action_overlay_ids(a, out);
                }
                for b in else_if_branches {
                    for a in &b.actions {
                        Self::extract_action_overlay_ids(a, out);
                    }
                }
                for a in else_actions {
                    Self::extract_action_overlay_ids(a, out);
                }
            }
            _ => {}
        }
    }

    pub fn extract_rule_overlay_ids(rule: &Rule) -> Vec<String> {
        let mut ids = Vec::new();
        for a in &rule.actions {
            Self::extract_action_overlay_ids(a, &mut ids);
        }
        for a in &rule.on_exit_actions {
            Self::extract_action_overlay_ids(a, &mut ids);
        }
        ids
    }

    pub fn get_rule_execution_state(&self, rule: &Rule) -> Option<crate::ipc::ActiveRuleExecutionState> {
        if let Some(daemon_exec) = self.daemon_state.as_ref().and_then(|s| s.active_rule_executions.get(&rule.id)).copied() {
            return Some(daemon_exec);
        }

        if let Some(started_at) = self.active_test_runs.get(&rule.id) {
            let total = rule.actions.len().max(1);
            let elapsed_ms = started_at.elapsed().as_millis() as u64;
            let step_dur = 400;
            let current_step = if rule.actions.is_empty() {
                0
            } else {
                ((elapsed_ms / step_dur) as usize).min(rule.actions.len() - 1)
            };
            return Some(crate::ipc::ActiveRuleExecutionState {
                active_count: 1,
                current_step,
                total_steps: total,
                is_exit: false,
            });
        }

        let has_active_overlay = rule.actions.iter().enumerate().find_map(|(idx, a)| match a {
                crate::models::Action::Overlay { overlay_id, custom_overlay, .. } => {
                    let ov_id = custom_overlay.as_ref().map(|o| o.id.as_str()).unwrap_or(overlay_id.as_str());
                    if self.daemon_state.as_ref().is_some_and(|d| d.active_overlay_ids.contains(ov_id))
                        || self.active_test_overlays.contains(ov_id)
                    {
                        Some(idx)
                    } else {
                        None
                    }
                }
                _ => None,
            });
        has_active_overlay.map(|step_idx| crate::ipc::ActiveRuleExecutionState {
            active_count: 1,
            current_step: step_idx,
            total_steps: rule.actions.len(),
            is_exit: false,
        })
    }

    pub fn hide_rule_overlays(rule: &Rule) {
        for id in Self::extract_rule_overlay_ids(rule) {
            let _ = IpcClient::call(&IpcRequest::HideOverlayPreview(id));
        }
    }

    pub fn cleanup_on_exit(&mut self) {
        if let Some(studio) = &self.overlay_studio {
            let _ = IpcClient::call(&IpcRequest::HideOverlayPreview(studio.overlay.id.clone()));
        }
        if let Some(rule) = &self.editing_rule {
            Self::hide_rule_overlays(rule);
        }
        if let Some(rule) = &self.builder_draft {
            Self::hide_rule_overlays(rule);
        }
        for id in self.active_test_overlays.drain() {
            let _ = IpcClient::call(&IpcRequest::HideOverlayPreview(id));
        }
    }

    pub fn close_editor(&mut self) {
        self.close_action_catalog();
        if let Some(studio) = &self.overlay_studio {
            let _ = IpcClient::call(&IpcRequest::HideOverlayPreview(studio.overlay.id.clone()));
        }
        if let Some(rule) = &self.editing_rule {
            Self::hide_rule_overlays(rule);
        }
        for id in self.active_test_overlays.drain() {
            let _ = IpcClient::call(&IpcRequest::HideOverlayPreview(id));
        }
        if self.is_creating_new_rule {
            self.builder_draft = self.editing_rule.take();
        } else {
            self.editing_rule = None;
        }
        self.is_creating_new_rule = false;
        self.selected_rule_id = None;
        self.picker_target = None;
    }

    pub fn open_process_picker(&mut self, target: ProcessPickerTarget) {
        // Opening the picker is the explicit refresh point: the list is fetched for exactly the
        // moment the user needs it, instead of on every daemon-sync tick while editing.
        self.request_process_snapshot(true);
        self.process_search_query.clear();
        self.picker_target = Some(target);
        self.last_picker_target = Some(target);
    }

    pub fn close_process_picker(&mut self) {
        self.picker_target = None;
    }

    /// Whether any modal popup (script editor, action catalog, process picker, or changelog) is currently open.
    pub fn is_modal_open(&self) -> bool {
        self.expanded_script_editor.is_some()
            || self.action_catalog.is_some()
            || self.trigger_catalog.is_some()
            || self.condition_catalog.is_some()
            || self.element_catalog.is_some()
            || self.picker_target.is_some()
            || (self.show_changelog_modal && self.available_update.is_some())
            || self.show_guide_modal
            || self.macro_editor.is_some()
            || self.overlay_studio.is_some()
    }

    pub fn apply_selected_process(&mut self, proc_name: &str) {
        let loc = self.loc();
        let target = self.picker_target.take().or(self.last_picker_target);
        self.picker_target = None;

        self.push_undo_snapshot();
        if let Some(rule) = &mut self.editing_rule {
            match target {
                Some(ProcessPickerTarget::Action(idx)) => {
                    if let Some(action) = rule.actions.get_mut(idx) {
                        apply_process_to_action(action, proc_name);
                    }
                }
                Some(ProcessPickerTarget::ExitAction(idx)) => {
                    if let Some(action) = rule.on_exit_actions.get_mut(idx) {
                        apply_process_to_action(action, proc_name);
                    }
                }
                Some(ProcessPickerTarget::Condition(idx)) => {
                    if let Some(item) = rule.conditions.get_mut(idx) {
                        apply_process_to_condition(&mut item.condition, proc_name);
                    }
                }
                Some(ProcessPickerTarget::ActionCondition { action_idx, cond_idx, is_exit }) => {
                    let list = if is_exit { &mut rule.on_exit_actions } else { &mut rule.actions };
                    if let Some(crate::models::Action::IfElse { conditions, .. }) = list.get_mut(action_idx) {
                        if let Some(item) = conditions.get_mut(cond_idx) {
                            apply_process_to_condition(&mut item.condition, proc_name);
                        }
                    }
                }
                Some(ProcessPickerTarget::ActionElseIfCondition { action_idx, branch_idx, cond_idx, is_exit }) => {
                    let list = if is_exit { &mut rule.on_exit_actions } else { &mut rule.actions };
                    if let Some(crate::models::Action::IfElse { else_if_branches, .. }) = list.get_mut(action_idx) {
                        if let Some(branch) = else_if_branches.get_mut(branch_idx) {
                            if let Some(item) = branch.conditions.get_mut(cond_idx) {
                                apply_process_to_condition(&mut item.condition, proc_name);
                            }
                        }
                    }
                }
                Some(ProcessPickerTarget::IfElseThen { parent_idx, sub_idx, is_exit }) => {
                    let list = if is_exit { &mut rule.on_exit_actions } else { &mut rule.actions };
                    if let Some(crate::models::Action::IfElse { then_actions, .. }) = list.get_mut(parent_idx) {
                        if let Some(sub_action) = then_actions.get_mut(sub_idx) {
                            apply_process_to_action(sub_action, proc_name);
                        }
                    }
                }
                Some(ProcessPickerTarget::IfElseElseIf { parent_idx, branch_idx, sub_idx, is_exit }) => {
                    let list = if is_exit { &mut rule.on_exit_actions } else { &mut rule.actions };
                    if let Some(crate::models::Action::IfElse { else_if_branches, .. }) = list.get_mut(parent_idx) {
                        if let Some(branch) = else_if_branches.get_mut(branch_idx) {
                            if let Some(sub_action) = branch.actions.get_mut(sub_idx) {
                                apply_process_to_action(sub_action, proc_name);
                            }
                        }
                    }
                }
                Some(ProcessPickerTarget::IfElseElse { parent_idx, sub_idx, is_exit }) => {
                    let list = if is_exit { &mut rule.on_exit_actions } else { &mut rule.actions };
                    if let Some(crate::models::Action::IfElse { else_actions, .. }) = list.get_mut(parent_idx) {
                        if let Some(sub_action) = else_actions.get_mut(sub_idx) {
                            apply_process_to_action(sub_action, proc_name);
                        }
                    }
                }
                Some(ProcessPickerTarget::Trigger) | None => {
                    match &mut rule.trigger {
                        Some(crate::models::Trigger::Process { process_name, .. }) => {
                            *process_name = proc_name.to_string();
                        }
                        Some(crate::models::Trigger::WindowFocus { process_name, .. }) => {
                            *process_name = proc_name.to_string();
                        }
                        Some(crate::models::Trigger::ResourceThreshold { process_name, .. }) => {
                            *process_name = proc_name.to_string();
                        }
                        _ => {
                            rule.trigger = Some(crate::models::Trigger::Process {
                                process_name: proc_name.to_string(),
                                match_mode: crate::models::ProcessMatchMode::Exact,
                                single_instance: false,
                            });
                        }
                    }

                    if rule.name.is_empty()
                        || rule.name == loc.rules_default_new_name
                        || rule.name.starts_with(loc.inspector_rule_track_prefix)
                    {
                        rule.name = format!("{} {}", loc.inspector_rule_track_prefix, proc_name);
                    }
                }
            }
            rule.auto_process_icon = true;
        }
    }

    pub fn apply_process_to_draft_or_new_rule(&mut self, proc_name: &str) {
        let loc = self.loc();
        let mut rule = self.initial_new_rule();
        rule.trigger = Some(crate::models::Trigger::Process {
            process_name: proc_name.to_string(),
            match_mode: crate::models::ProcessMatchMode::Exact,
            single_instance: false,
        });
        rule.name = format!("{} {}", loc.inspector_rule_track_prefix, proc_name);
        rule.auto_process_icon = true;
        self.active_view = ActiveView::Rules;
        self.is_creating_new_rule = true;
        self.selected_rule_id = None;
        self.editing_rule = Some(rule);
        self.selected_process = None;
    }

    pub fn edit_rule(&mut self, rule: Rule) {
        if self.is_creating_new_rule {
            self.builder_draft = self.editing_rule.take();
        }
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.tag_picker_open = false;
        self.new_tag_name.clear();
        self.new_tag_color = Some("#3B82F6".into());
        self.new_tag_icon = None;
        self.selected_rule_id = Some(rule.id.clone());
        self.editing_rule = Some(rule);
        self.is_creating_new_rule = false;
        self.selected_process = None;
    }

    /// Clears the rule's quota progress. The daemon owns the counters, so a rule that
    /// already exists is reset through `ResetRule`; an unsaved draft is cleared locally.
    pub fn reset_rule_quota(&mut self, rule: &mut Rule) {
        rule.clear_quota_runtime();
        rule.is_enabled = true;
        let saved = self
            .daemon_state
            .as_ref()
            .is_some_and(|state| state.config.rules.iter().any(|saved| saved.id == rule.id));
        if saved {
            let id = rule.id.clone();
            let direct = DirectWrite::ResetRuntime { rule_id: id.clone() };
            self.enqueue_ipc(IpcJobKind::ResetRule { rule_id: id.clone() }, IpcRequest::ResetRule(id), Some(direct));
        }
    }

    pub fn save_rule(&mut self, mut rule: Rule) {
        if rule.name.trim().is_empty() {
            rule.name = self.loc().rules_default_new_name.to_string();
        }
        let mut seen_ids = std::collections::HashSet::new();
        fn deduplicate_overlay_ids(actions: &mut [crate::models::Action], seen: &mut std::collections::HashSet<String>) {
            for action in actions.iter_mut() {
                match action {
                    crate::models::Action::Overlay { overlay_id, custom_overlay, .. } => {
                        if !seen.insert(overlay_id.clone()) {
                            static DEDUP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
                            let count = DEDUP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            let fresh_id = format!(
                                "overlay_{:x}_{:x}",
                                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos(),
                                count
                            );
                            *overlay_id = fresh_id.clone();
                            if let Some(ref mut cust) = custom_overlay {
                                cust.id = fresh_id;
                            }
                        } else if let Some(ref mut cust) = custom_overlay {
                            cust.id = overlay_id.clone();
                        }
                    }
                    crate::models::Action::IfElse { then_actions, else_if_branches, else_actions, .. } => {
                        deduplicate_overlay_ids(then_actions, seen);
                        for b in else_if_branches {
                            deduplicate_overlay_ids(&mut b.actions, seen);
                        }
                        deduplicate_overlay_ids(else_actions, seen);
                    }
                    _ => {}
                }
            }
        }
        deduplicate_overlay_ids(&mut rule.actions, &mut seen_ids);
        deduplicate_overlay_ids(&mut rule.on_exit_actions, &mut seen_ids);

        // The draft stays in `editing_rule` until the daemon confirms the save; a transport or
        // validation failure must not discard the only copy of the user's rule.
        let rule_id = rule.id.clone();
        let direct = DirectWrite::Rules(DirectRuleChange::Upsert(Box::new(rule.clone())));
        self.enqueue_ipc(IpcJobKind::SaveRule { rule_id }, IpcRequest::SaveRule(rule), Some(direct));
    }

    /// Applies the daemon's answer to `SaveRule`: clears the editor only on confirmation and
    /// reports the reason otherwise.
    fn apply_save_rule_result(&mut self, rule_id: &str, result: Result<IpcResponse, String>) {
        match Self::mutation_failure(result) {
            None => {
                self.editing_rule = None;
                self.is_creating_new_rule = false;
                self.selected_rule_id = None;
                self.builder_draft = None;
                self.set_toast(self.loc().toast_rule_saved);
            }
            Some(failure) => {
                crate::logging::warn(
                    "GUI",
                    &format!("SaveRule '{rule_id}' was not applied: {}", failure.diagnostic()),
                );
                self.set_toast(self.mutation_failure_toast(&failure));
            }
        }
        self.daemon_sync.force();
    }

    pub fn delete_rule(&mut self, id: &str) {
        // Previews are hidden right away; the rule itself is removed only once the daemon
        // confirms, so a failed delete leaves the editor and selection intact.
        if let Some(rule) = self.daemon_state.as_ref().and_then(|s| s.config.rules.iter().find(|r| r.id == id)) {
            Self::hide_rule_overlays(rule);
        }
        if let Some(rule) = &self.editing_rule {
            if rule.id == id {
                Self::hide_rule_overlays(rule);
            }
        }
        let direct = DirectWrite::Rules(DirectRuleChange::Delete(id.to_string()));
        self.enqueue_ipc(
            IpcJobKind::DeleteRule { rule_id: id.to_string() },
            IpcRequest::DeleteRule(id.to_string()),
            Some(direct),
        );
    }

    pub fn clone_rule(&mut self, rule: &Rule) {
        let cloned = rule.clone_with_new_id();
        let new_name = cloned.name.clone();

        // The clone opens in the editor immediately; the daemon save is confirmed separately.
        self.selected_rule_id = None;
        if self.editing_rule.is_some() {
            self.editing_rule = Some(cloned.clone());
            self.is_creating_new_rule = false;
        }
        self.set_toast(format!("{}: {}", self.loc().toast_rule_cloned, new_name));
        let direct = DirectWrite::Rules(DirectRuleChange::Upsert(Box::new(cloned.clone())));
        self.enqueue_ipc(
            IpcJobKind::CloneRule { rule_id: cloned.id.clone() },
            IpcRequest::SaveRule(cloned),
            Some(direct),
        );
    }

    pub fn reorder_rule_by_id(&mut self, src_id: &str, target_id: Option<&str>, insert_after: bool) {
        let mut order = None;
        if let Some(ds) = &mut self.daemon_state {
            if let Some(src_pos) = ds.config.rules.iter().position(|r| r.id == src_id) {
                let rule = ds.config.rules.remove(src_pos);
                let target_pos = if let Some(tid) = target_id {
                    ds.config.rules.iter().position(|r| r.id == tid)
                } else {
                    None
                };
                let insert_idx = match target_pos {
                    Some(pos) => {
                        if insert_after {
                            pos + 1
                        } else {
                            pos
                        }
                    }
                    None => ds.config.rules.len(),
                };
                let insert_idx = insert_idx.min(ds.config.rules.len());
                ds.config.rules.insert(insert_idx, rule);
                order = Some(ds.config.rules.iter().map(|rule| rule.id.clone()).collect::<Vec<String>>());
            }
        }
        if let Some(order) = order {
            let direct = DirectWrite::Rules(DirectRuleChange::Reorder(order.clone()));
            self.enqueue_ipc(IpcJobKind::ReorderRules, IpcRequest::ReorderRules(order), Some(direct));
        }
    }

    pub fn toggle_rule(&mut self, id: &str, enabled: bool) {
        let direct = DirectWrite::Rules(DirectRuleChange::SetEnabled { id: id.to_string(), enabled });
        self.enqueue_ipc(
            IpcJobKind::ToggleRule { rule_id: id.to_string(), enabled },
            IpcRequest::ToggleRule { id: id.to_string(), enabled },
            Some(direct),
        );
    }

    pub fn test_run_rule(&mut self, id: &str) {
        self.active_test_runs.insert(id.to_string(), Instant::now());
        let mut overlay_ids = Vec::new();
        if let Some(rule) = self.daemon_state.as_ref().and_then(|s| s.config.rules.iter().find(|r| r.id == id)) {
            overlay_ids.extend(Self::extract_rule_overlay_ids(rule));
        }
        let mut rule_override = None;
        if let Some(rule) = &self.editing_rule {
            if rule.id == id {
                overlay_ids.extend(Self::extract_rule_overlay_ids(rule));
                rule_override = Some(Box::new(rule.clone()));
            }
        }
        for overlay_id in overlay_ids {
            self.active_test_overlays.insert(overlay_id);
        }
        self.enqueue_ipc(
            IpcJobKind::TestRunRule { rule_id: id.to_string() },
            IpcRequest::TestRunRule { id: id.to_string(), rule: rule_override },
            None,
        );
        self.daemon_sync.force();
    }

    /// Overlay ids referenced by the saved rule or by the open draft with the same id.
    fn rule_overlay_ids(&self, rule_id: &str) -> Option<Vec<String>> {
        self.daemon_state
            .as_ref()
            .and_then(|s| s.config.rules.iter().find(|r| r.id == rule_id))
            .map(Self::extract_rule_overlay_ids)
            .or_else(|| {
                self.editing_rule
                    .as_ref()
                    .filter(|r| r.id == rule_id)
                    .map(Self::extract_rule_overlay_ids)
            })
    }

    /// A test run starts only when the daemon accepts it; otherwise the optimistic running
    /// markers are rolled back.
    fn apply_test_run_result(&mut self, rule_id: &str, result: Result<IpcResponse, String>) {
        match Self::mutation_failure(result) {
            None => self.set_toast(self.loc().toast_test_execution),
            Some(failure) => {
                self.active_test_runs.remove(rule_id);
                if let Some(ids) = self.rule_overlay_ids(rule_id) {
                    for overlay_id in ids {
                        self.active_test_overlays.remove(&overlay_id);
                    }
                }
                crate::logging::warn(
                    "GUI",
                    &format!("TestRunRule '{rule_id}' was not applied: {}", failure.diagnostic()),
                );
                self.set_toast(self.mutation_failure_toast(&failure));
            }
        }
        self.daemon_sync.force();
    }

    pub fn stop_rule_execution(&mut self, id: &str) {
        self.active_test_runs.remove(id);
        if let Some(ids) = self.rule_overlay_ids(id) {
            for ov_id in ids {
                self.active_test_overlays.remove(&ov_id);
            }
        }
        self.enqueue_ipc(
            IpcJobKind::StopRuleExecution { rule_id: id.to_string() },
            IpcRequest::StopRuleExecution(id.to_string()),
            None,
        );
        self.daemon_sync.force();
    }

    pub fn check_for_updates(&mut self, is_manual: bool) {
        if self.is_checking_update || self.is_downloading_update {
            return;
        }
        self.is_checking_update = true;
        let tx = self.updates.tx.clone();
        let current_ver = env!("CARGO_PKG_VERSION").to_string();

        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = tx.send(crate::updater::UpdateEvent::CheckComplete {
                        result: Box::new(Err(format!("Runtime error: {e}"))),
                        is_manual,
                    });
                    return;
                }
            };

            let result = rt.block_on(async {
                crate::updater::check_github_release(crate::updater::GITHUB_REPO, &current_ver).await
            });

            let _ = tx.send(crate::updater::UpdateEvent::CheckComplete {
                result: Box::new(result),
                is_manual,
            });
        });
    }

    pub fn start_update(&mut self, info: crate::updater::ReleaseInfo) {
        if self.is_downloading_update {
            return;
        }
        self.is_downloading_update = true;
        self.gui_closed_during_update = false;
        crate::updater::set_gui_closed_during_update(false);
        self.update_download_progress = 0.0;
        self.set_toast(self.loc().toast_update_downloading.replace("{}", &info.version));

        self.enqueue_ipc(IpcJobKind::StartUpdate, IpcRequest::StartUpdate, None);
    }

    /// The optimistic "downloading" flag is rolled back when the daemon never accepted the
    /// request, so the footer stops claiming progress that will never happen.
    fn apply_start_update_result(&mut self, result: Result<IpcResponse, String>) {
        if let Some(failure) = Self::mutation_failure(result) {
            self.is_downloading_update = false;
            self.update_download_progress = 0.0;
            crate::logging::warn(
                "GUI",
                &format!("StartUpdate was not applied: {}", failure.diagnostic()),
            );
            self.set_toast(match &failure {
                MutationFailure::UnexpectedResponse(_) => {
                    self.loc().toast_update_failed_unexpected.to_string()
                }
                MutationFailure::Detail(_) => {
                    self.loc().toast_update_failed.replace("{}", failure.diagnostic())
                }
            });
        }
        self.daemon_sync.force();
    }

    fn apply_autorun_result(&mut self, enabled: bool, result: Result<IpcResponse, String>) {
        match Self::mutation_failure(result) {
            None => {
                let msg = if enabled {
                    self.loc().toast_autorun_enabled
                } else {
                    self.loc().toast_autorun_disabled
                };
                self.set_toast(msg);
            }
            Some(failure) => {
                crate::logging::warn(
                    "GUI",
                    &format!("SetAutorun was not applied: {}", failure.diagnostic()),
                );
                self.set_toast(self.mutation_failure_toast(&failure));
            }
        }
        self.daemon_sync.force();
    }

    pub fn poll_update_events(&mut self) {
        while let Ok(event) = self.updates.rx.try_recv() {
            match event {
                crate::updater::UpdateEvent::CheckStarted => {
                    self.is_checking_update = true;
                }
                crate::updater::UpdateEvent::CheckComplete { result, is_manual } => {
                    self.is_checking_update = false;
                    match *result {
                        Ok(Some(info)) => {
                            let ver = info.version.clone();
                            self.available_update = Some(info.clone());
                            self.updates.advertise();
                            self.enqueue_ipc(
                                IpcJobKind::Background,
                                IpcRequest::SetAvailableUpdate(Some(info)),
                                None,
                            );
                            if is_manual {
                                self.set_toast(self.loc().toast_update_available.replace("{}", &ver));
                            }
                        }
                        Ok(None) => {
                            self.enqueue_ipc(
                                IpcJobKind::Background,
                                IpcRequest::SetAvailableUpdate(None),
                                None,
                            );
                            // The manual check found nothing: stop offering the release the
                            // client remembered from an older daemon session.
                            self.clear_available_update();
                            if is_manual {
                                self.set_toast(self.loc().toast_latest_version.replace("{}", env!("CARGO_PKG_VERSION")));
                            }
                        }
                        Err(err) => {
                            if is_manual {
                                self.set_toast(self.loc().toast_update_check_failed.replace("{}", &err));
                            }
                        }
                    }
                }
                crate::updater::UpdateEvent::DownloadProgress(pct) => {
                    self.update_download_progress = pct;
                }
                crate::updater::UpdateEvent::DownloadComplete(result) => {
                    self.is_downloading_update = false;
                    match result {
                        Ok(()) => {
                            // The binary was swapped and PowerShell restart helper spawned.
                            // Exit GUI cleanly so PowerShell continues immediately.
                            std::process::exit(0);
                        }
                        Err(err) => {
                            self.set_toast(self.loc().toast_update_failed.replace("{}", &err));
                            if self.gui_closed_during_update {
                                std::process::exit(1);
                            }
                        }
                    }
                }
            }
        }
    }
}

fn apply_process_to_action(action: &mut crate::models::Action, proc_name: &str) {
    match action {
        crate::models::Action::KillProcess { process_name, .. } => {
            *process_name = proc_name.to_string();
        }
        crate::models::Action::AudioControl { target, process_name, .. } => {
            if *target == crate::models::AudioTarget::Process {
                *process_name = Some(proc_name.to_string());
            }
        }
        crate::models::Action::Firewall { process_name, .. } => {
            *process_name = proc_name.to_string();
        }
        crate::models::Action::Window { target_process, .. } => {
            *target_process = Some(proc_name.to_string());
        }
        crate::models::Action::ProcessTuning { target_process, .. } => {
            *target_process = Some(proc_name.to_string());
        }
        crate::models::Action::Wait { condition: Some(crate::models::WaitCondition::ProcessStarted { process_name, .. } | crate::models::WaitCondition::ProcessExited { process_name, .. } | crate::models::WaitCondition::WindowOpened { process_name, .. } | crate::models::WaitCondition::WindowClosed { process_name, .. }), .. } => {
            *process_name = proc_name.to_string();
        }
        crate::models::Action::Check { condition } => {
            apply_process_to_condition(condition, proc_name);
        }
        _ => {}
    }
}

/// Built-in overlay definitions by id; unknown ids fall back to the blank template. Shared by
/// both studio openers so a new built-in overlay is declared once.
fn default_overlay_for(id: &str) -> crate::models::OverlayDefinition {
    match id {
        "overlay_crosshair" => crate::models::OverlayDefinition::default_crosshair(),
        "overlay_telemetry" => crate::models::OverlayDefinition::default_telemetry(),
        "overlay_macro_badge" => crate::models::OverlayDefinition::default_macro_badge(),
        _ => crate::models::OverlayDefinition::default_new_overlay(),
    }
}

fn dirs_desktop() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE").map(|u| std::path::PathBuf::from(u).join("Desktop"))
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join("Desktop"))
    }
}

fn apply_process_to_condition(cond: &mut crate::models::RuleCondition, proc_name: &str) {
    match cond {
        crate::models::RuleCondition::ProcessRunning { process_name, .. } => {
            *process_name = proc_name.to_string();
        }
        crate::models::RuleCondition::WindowFocused { process_name, .. } => {
            *process_name = proc_name.to_string();
        }
        crate::models::RuleCondition::ResourceThreshold { process_name, .. } => {
            *process_name = proc_name.to_string();
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Action, ProcessMatchMode, Rule, RuleCondition, RuleConditionItem, SystemEventType};

    #[test]
    fn test_is_modal_open() {
        let mut state = GuiState::new();
        assert!(!state.is_modal_open());

        // Process picker
        state.open_process_picker(ProcessPickerTarget::Trigger);
        assert!(state.is_modal_open());
        state.close_process_picker();
        assert!(!state.is_modal_open());

        // Action catalog
        state.action_catalog = Some(ActionCatalogModalState {
            is_exit_pipeline: false,
            insert_idx: None,
            search_query: String::new(),
            selected_category: 0,
            ifelse_target: None,
        });
        assert!(state.is_modal_open());
        state.action_catalog = None;
        assert!(!state.is_modal_open());

        // Script editor
        state.expanded_script_editor = Some(ExpandedScriptEditorState {
            step_idx: 0,
            is_exit_pipeline: false,
            language: crate::gui::components::script_highlighter::ScriptLanguage::PowerShell,
        });
        assert!(state.is_modal_open());
        state.expanded_script_editor = None;
        assert!(!state.is_modal_open());

        // Changelog modal (requires show_changelog_modal && available_update.is_some())
        state.show_changelog_modal = true;
        assert!(!state.is_modal_open());
        state.available_update = Some(crate::updater::ReleaseInfo {
            version: "1.4.0".into(),
            tag_name: "v1.4.0".into(),
            name: "v1.4.0".into(),
            body: "Notes".into(),
            html_url: "http://example.com".into(),
            portable: Some(crate::updater::UpdateAsset {
                name: "Synector-v1.4.0-Windows-x64.zip".into(),
                url: "http://example.com/asset.zip".into(),
                size: 1024,
                sha256: None,
            }),
            installer: None,
        });
        assert!(state.is_modal_open());
        state.show_changelog_modal = false;
        assert!(!state.is_modal_open());

        // Guide modal
        state.show_guide_modal = true;
        assert!(state.is_modal_open());
        state.show_guide_modal = false;
        assert!(!state.is_modal_open());

        // Macro editor
        state.open_macro_editor(0, false);
        assert!(state.is_modal_open());
        state.close_macro_editor();
        assert!(!state.is_modal_open());

        // Overlay studio
        state.open_overlay_studio("overlay_crosshair");
        assert!(state.is_modal_open());
        state.close_overlay_studio();
        assert!(!state.is_modal_open());
    }

    #[test]
    fn test_undo_redo_multi_step() {
        let mut state = GuiState::new();
        let mut r0 = state.initial_new_rule();
        r0.name = "Step 0".to_string();
        state.editing_rule = Some(r0.clone());

        let mut r1 = r0.clone();
        r1.name = "Step 1".to_string();
        state.push_undo_snapshot_of(&r0);
        state.editing_rule = Some(r1.clone());

        let mut r2 = r1.clone();
        r2.name = "Step 2".to_string();
        state.push_undo_snapshot_of(&r1);
        state.editing_rule = Some(r2.clone());

        assert_eq!(state.editing_rule.as_ref().unwrap().name, "Step 2");
        assert_eq!(state.undo_stack.len(), 2);
        assert_eq!(state.redo_stack.len(), 0);

        // Undo 1 -> Step 1
        state.undo();
        assert_eq!(state.editing_rule.as_ref().unwrap().name, "Step 1");
        assert_eq!(state.undo_stack.len(), 1);
        assert_eq!(state.redo_stack.len(), 1);

        // Undo 2 -> Step 0
        state.undo();
        assert_eq!(state.editing_rule.as_ref().unwrap().name, "Step 0");
        assert_eq!(state.undo_stack.len(), 0);
        assert_eq!(state.redo_stack.len(), 2);

        // Redo 1 -> Step 1
        state.redo();
        assert_eq!(state.editing_rule.as_ref().unwrap().name, "Step 1");
        assert_eq!(state.undo_stack.len(), 1);
        assert_eq!(state.redo_stack.len(), 1);

        // Redo 2 -> Step 2
        state.redo();
        assert_eq!(state.editing_rule.as_ref().unwrap().name, "Step 2");
        assert_eq!(state.undo_stack.len(), 2);
        assert_eq!(state.redo_stack.len(), 0);
    }

    #[test]
    fn test_condition_catalog_targets_application() {
        let mut state = GuiState::new();
        let mut rule = Rule::default();
        rule.actions.push(Action::IfElse {
            conditions: vec![RuleConditionItem::new(RuleCondition::default_process())],
            then_actions: vec![],
            else_if_branches: vec![crate::models::ElseIfBranch {
                conditions: vec![RuleConditionItem::new(RuleCondition::default_process())],
                actions: vec![],
            }],
            else_actions: vec![],
        });
        state.editing_rule = Some(rule);

        // 1. Add rule precondition
        state.open_condition_catalog_with_target(ConditionCatalogTarget::RulePrecondition);
        state.apply_condition_from_catalog(RuleCondition::TimeRange {
            start_time: "08:00".into(),
            end_time: "17:00".into(),
            days_of_week: vec![],
        });
        assert_eq!(state.editing_rule.as_ref().unwrap().conditions.len(), 1);
        assert!(matches!(state.editing_rule.as_ref().unwrap().conditions[0].condition, RuleCondition::TimeRange { .. }));

        // 2. Change rule precondition
        state.open_condition_catalog_with_target(ConditionCatalogTarget::ChangeRulePrecondition(0));
        state.apply_condition_from_catalog(RuleCondition::FileWatcher {
            path: "C:\\test.txt".into(),
            filter_extension: None,
            must_exist: true,
        });
        assert_eq!(state.editing_rule.as_ref().unwrap().conditions.len(), 1);
        assert!(matches!(state.editing_rule.as_ref().unwrap().conditions[0].condition, RuleCondition::FileWatcher { .. }));

        // 3. Change primary IfElse condition (card 0)
        state.open_condition_catalog_with_target(ConditionCatalogTarget::ChangeActionIfElse {
            action_idx: 0,
            cond_idx: 0,
            is_exit: false,
        });
        state.apply_condition_from_catalog(RuleCondition::WindowFocused {
            process_name: "code.exe".into(),
            match_mode: ProcessMatchMode::Exact,
            must_be_focused: true,
        });
        if let Some(Action::IfElse { conditions, .. }) = state.editing_rule.as_ref().unwrap().actions.first() {
            assert!(matches!(conditions[0].condition, RuleCondition::WindowFocused { .. }));
        } else {
            panic!("Expected IfElse action");
        }

        // 4. Add condition to primary IfElse
        state.open_condition_catalog_with_target(ConditionCatalogTarget::ActionIfElse {
            action_idx: 0,
            is_exit: false,
        });
        state.apply_condition_from_catalog(RuleCondition::TimeRange {
            start_time: "10:00".into(),
            end_time: "12:00".into(),
            days_of_week: vec![],
        });
        if let Some(Action::IfElse { conditions, .. }) = state.editing_rule.as_ref().unwrap().actions.first() {
            assert_eq!(conditions.len(), 2);
            assert!(matches!(conditions[1].condition, RuleCondition::TimeRange { .. }));
        } else {
            panic!("Expected IfElse action");
        }

        // 5. Change condition in else_if_branches
        state.open_condition_catalog_with_target(ConditionCatalogTarget::ChangeActionElseIf {
            action_idx: 0,
            branch_idx: 0,
            cond_idx: 0,
            is_exit: false,
        });
        state.apply_condition_from_catalog(RuleCondition::Event {
            event_type: SystemEventType::PowerSourceBattery,
            idle_duration_sec: 0,
        });
        if let Some(Action::IfElse { else_if_branches, .. }) = state.editing_rule.as_ref().unwrap().actions.first() {
            assert!(matches!(else_if_branches[0].conditions[0].condition, RuleCondition::Event { .. }));
        } else {
            panic!("Expected IfElse action");
        }

        // 6. Add condition to else_if_branches
        state.open_condition_catalog_with_target(ConditionCatalogTarget::ActionElseIf {
            action_idx: 0,
            branch_idx: 0,
            is_exit: false,
        });
        state.apply_condition_from_catalog(RuleCondition::TimeRange {
            start_time: "13:00".into(),
            end_time: "14:00".into(),
            days_of_week: vec![],
        });
        if let Some(Action::IfElse { else_if_branches, .. }) = state.editing_rule.as_ref().unwrap().actions.first() {
            assert_eq!(else_if_branches[0].conditions.len(), 2);
            assert!(matches!(else_if_branches[0].conditions[1].condition, RuleCondition::TimeRange { .. }));
        } else {
            panic!("Expected IfElse action");
        }
    }

    #[test]
    fn test_reorder_rule_by_id() {
        let mut state = GuiState::new();
        let r1 = Rule { id: "rule-1".into(), ..Rule::default() };
        let r2 = Rule { id: "rule-2".into(), ..Rule::default() };
        let r3 = Rule { id: "rule-3".into(), ..Rule::default() };
        let r4 = Rule { id: "rule-4".into(), ..Rule::default() };
        let config = crate::config::AppConfig {
            rules: vec![r1, r2, r3, r4],
            ..Default::default()
        };
        state.daemon_state = Some(DaemonState {
            autorun_enabled: false,
            uptime_secs: 0,
            daemon_memory_bytes: 0,
            active_rules_count: 0,
            total_rules_count: 0,
            config,
            logs: vec![],
            active_rule_executions: std::collections::HashMap::new(),
            active_overlay_ids: std::collections::HashSet::new(),
            active_test_rule_ids: std::collections::HashSet::new(),
            analytics: Default::default(),
            available_update: None,
            update_progress: None,
            update_status: None,
            pending_open_update_modal: false,
            pending_auto_start_update: false,
        });

        // Move rule-1 after rule-3
        state.reorder_rule_by_id("rule-1", Some("rule-3"), true);
        let ids: Vec<_> = state.daemon_state.as_ref().unwrap().config.rules.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["rule-2", "rule-3", "rule-1", "rule-4"]);

        // Move rule-4 before rule-2
        state.reorder_rule_by_id("rule-4", Some("rule-2"), false);
        let ids: Vec<_> = state.daemon_state.as_ref().unwrap().config.rules.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["rule-4", "rule-2", "rule-3", "rule-1"]);

        // Move rule-2 to bottom (target None)
        state.reorder_rule_by_id("rule-2", None, false);
        let ids: Vec<_> = state.daemon_state.as_ref().unwrap().config.rules.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["rule-4", "rule-3", "rule-1", "rule-2"]);
    }

    fn state_with_shared_rules() -> GuiState {
        let mut state = GuiState::new();
        let json = serde_json::json!({
            "autorun_enabled": false,
            "uptime_secs": 0,
            "daemon_memory_bytes": 0,
            "daemon_cpu_usage": 0.0,
            "active_rules_count": 3,
            "total_rules_count": 3,
            "config": {
                "rules": [
                    {"id": "work_rule", "name": "Work Profile", "tags": [{"name": "Work"}]},
                    {"id": "game_rule", "name": "Game Boost", "tags": [{"name": "Gaming"}]},
                    {"id": "misc_rule", "name": "Misc Workflow"}
                ]
            },
            "logs": []
        });
        state.daemon_state =
            Some(serde_json::from_value(json).expect("daemon state deserializes"));
        state
    }

    #[test]
    fn test_visible_rules_respect_tag_filter_and_search() {
        let mut state = state_with_shared_rules();
        assert_eq!(state.visible_rules().len(), 3);

        state.selected_tag_filter = Some("Work".into());
        let filtered = state.visible_rules();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "work_rule");

        state.selected_tag_filter = None;
        state.search_query = "work".into();
        let searched = state.visible_rules();
        assert_eq!(searched.len(), 2, "name and tag matches are both kept");

        state.search_query = "nothing-matches".into();
        assert!(state.visible_rules().is_empty());
    }

    #[test]
    fn test_share_error_text_is_localized() {
        let mut state = GuiState::new();
        state.language = crate::i18n::Language::English;
        assert_eq!(
            state.share_error_text(&crate::sharing::ShareError::LegacyFormat),
            crate::i18n::get(crate::i18n::Language::English).share_err_legacy
        );
        state.language = crate::i18n::Language::Russian;
        assert_eq!(
            state.share_error_text(&crate::sharing::ShareError::ChecksumMismatch),
            crate::i18n::get(crate::i18n::Language::Russian).share_err_checksum
        );
        let version = state.share_error_text(&crate::sharing::ShareError::UnsupportedVersion(9));
        assert!(version.contains('9'));
    }

    // -----------------------------------------------------------------------
    // Mutation outcomes (U05-001, U05-002)
    // -----------------------------------------------------------------------

    fn macro_rule(state: &GuiState, preset_id: &str) -> Rule {
        let mut rule = state.initial_new_rule();
        rule.name = "Macro rule".into();
        rule.actions.push(Action::Macro {
            preset_id: Some(preset_id.into()),
            operation: crate::models::MacroOperation::Run,
            events: vec![crate::models::MacroEvent::KeyPress { key: "A".into() }],
            repeat: 1,
            speed: 1.0,
            infinite: false,
        });
        rule
    }

    fn macro_events(state: &GuiState, step_idx: usize) -> &[crate::models::MacroEvent] {
        match &state.editing_rule.as_ref().expect("draft").actions[step_idx] {
            Action::Macro { events, .. } => events,
            other => panic!("expected a macro action, got {other:?}"),
        }
    }

    #[test]
    fn test_save_rule_failure_keeps_the_draft() {
        let mut state = GuiState::new();
        state.language = crate::i18n::Language::English;
        let mut rule = state.initial_new_rule();
        rule.name = "Unsaved draft".into();
        state.editing_rule = Some(rule.clone());
        state.is_creating_new_rule = true;

        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::SaveRule { rule_id: rule.id.clone() },
            result: Err("Cannot connect to daemon pipe".into()),
        });

        let draft = state.editing_rule.as_ref().expect("a failed save must keep the only draft");
        assert_eq!(draft.id, rule.id);
        assert!(state.is_creating_new_rule, "the editor stays in create mode");
        let toast = state.toast.as_ref().expect("failure is reported").0.clone();
        assert_eq!(
            toast,
            crate::i18n::get(state.language).toast_request_failed.replace("{}", "Cannot connect to daemon pipe")
        );

        // A confirmed save is the only path that clears the editor.
        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::SaveRule { rule_id: rule.id.clone() },
            result: Ok(IpcResponse::Ok),
        });
        assert!(state.editing_rule.is_none());
        assert!(!state.is_creating_new_rule);
        assert_eq!(state.toast.as_ref().unwrap().0, crate::i18n::get(state.language).toast_rule_saved);
    }

    #[test]
    fn test_macro_preset_failure_keeps_the_recorded_events() {
        let mut state = GuiState::new();
        state.language = crate::i18n::Language::English;
        state.editing_rule = Some(macro_rule(&state, "macro_demo"));
        let preset_id = "macro_demo".to_string();
        state.pending_macro_saves.push(PendingMacroSave {
            step_idx: 0,
            is_exit_pipeline: false,
            preset_id: preset_id.clone(),
        });

        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::SaveMacroPreset { preset_id: preset_id.clone() },
            result: Err("daemon unavailable".into()),
        });

        assert!(!macro_events(&state, 0).is_empty(), "a failed preset save must keep the recording");
        assert!(state.pending_macro_saves.is_empty(), "no retry is queued");
        let toast = state.toast.as_ref().expect("failure is reported").0.clone();
        assert_eq!(
            toast,
            crate::i18n::get(state.language).toast_request_failed.replace("{}", "daemon unavailable")
        );

        // Confirmation is the only path that drops the events from the rule.
        state.pending_macro_saves.push(PendingMacroSave {
            step_idx: 0,
            is_exit_pipeline: false,
            preset_id: preset_id.clone(),
        });
        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::SaveMacroPreset { preset_id },
            result: Ok(IpcResponse::Ok),
        });
        assert!(macro_events(&state, 0).is_empty(), "confirmed presets are referenced by id");
    }

    // -----------------------------------------------------------------------
    // User-visible failure copy (U05-010, U16-012)
    // -----------------------------------------------------------------------

    #[test]
    fn test_macro_preset_default_name_is_localized() {
        let mut state = GuiState::new();
        state.language = crate::i18n::Language::English;
        state.open_macro_editor(0, false);
        assert_eq!(
            state.macro_preset_save_name,
            crate::i18n::get(crate::i18n::Language::English).macro_studio_custom_preset_name
        );

        state.language = crate::i18n::Language::Russian;
        state.open_macro_editor(0, false);
        assert_eq!(
            state.macro_preset_save_name,
            crate::i18n::get(crate::i18n::Language::Russian).macro_studio_custom_preset_name
        );
        assert_ne!(
            crate::i18n::get(crate::i18n::Language::English).macro_studio_custom_preset_name,
            crate::i18n::get(crate::i18n::Language::Russian).macro_studio_custom_preset_name,
            "the default preset name is translated, not an English literal"
        );
    }

    #[test]
    fn test_unexpected_response_toast_is_localized() {
        let mut state = GuiState::new();
        state.language = crate::i18n::Language::English;
        let toggle = IpcJobResult {
            kind: IpcJobKind::ToggleRule { rule_id: "rule-unexpected".into(), enabled: true },
            result: Ok(IpcResponse::Pong),
        };
        state.apply_ipc_result(toggle);
        let toast = state.toast.as_ref().expect("failure is reported").0.clone();
        assert_eq!(
            toast,
            crate::i18n::get(crate::i18n::Language::English).toast_request_failed_unexpected
        );
        assert!(!toast.contains("Pong"), "the response name stays in the log only: {toast}");

        state.language = crate::i18n::Language::Russian;
        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::ToggleRule { rule_id: "rule-unexpected".into(), enabled: true },
            result: Ok(IpcResponse::Pong),
        });
        assert_eq!(
            state.toast.as_ref().expect("failure is reported").0.as_str(),
            crate::i18n::get(crate::i18n::Language::Russian).toast_request_failed_unexpected
        );
    }

    #[test]
    fn test_import_unexpected_response_toast_is_localized() {
        let mut state = GuiState::new();
        state.language = crate::i18n::Language::Russian;
        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::ImportShare,
            result: Ok(IpcResponse::Pong),
        });
        assert_eq!(
            state.toast.as_ref().expect("failure is reported").0.as_str(),
            crate::i18n::get(crate::i18n::Language::Russian).toast_request_failed_unexpected
        );
    }

    /// I18N-GAP: the update path reports an unexpected response with its own localized line
    /// instead of embedding the English protocol detail in `toast_update_failed`.
    #[test]
    fn test_update_unexpected_response_toast_is_localized() {
        let en = crate::i18n::get(crate::i18n::Language::English);
        let ru = crate::i18n::get(crate::i18n::Language::Russian);

        let mut state = GuiState::new();
        state.language = crate::i18n::Language::English;
        state.apply_start_update_result(Ok(IpcResponse::Pong));
        let toast = state.toast.as_ref().expect("failure is reported").0.clone();
        assert_eq!(toast, en.toast_update_failed_unexpected);
        assert!(!toast.contains("Pong"), "the response name stays in the log only: {toast}");
        assert!(!toast.contains("{}"), "the generic update toast must not keep a placeholder: {toast}");

        state.language = crate::i18n::Language::Russian;
        state.apply_start_update_result(Ok(IpcResponse::Pong));
        assert_eq!(
            state.toast.as_ref().expect("failure is reported").0.as_str(),
            ru.toast_update_failed_unexpected
        );

        // A daemon rejection still carries its (already safe) detail through the generic line.
        state.language = crate::i18n::Language::English;
        state.apply_start_update_result(Ok(IpcResponse::Error("startup refused".into())));
        assert_eq!(
            state.toast.as_ref().expect("failure is reported").0.as_str(),
            en.toast_update_failed.replace("{}", "startup refused")
        );
    }

    // -----------------------------------------------------------------------
    // Daemon sync (U05-003)
    // -----------------------------------------------------------------------

    #[test]
    fn test_sync_failure_needs_a_sustained_outage_and_keeps_the_error() {
        let mut state = GuiState::new();
        state.language = crate::i18n::Language::English;
        state.has_connected_daemon = true;
        state.is_daemon_connected = true;
        state.daemon_sync.note_success();

        for failures in 1..=2 {
            state.apply_ipc_result(IpcJobResult {
                kind: IpcJobKind::Sync,
                result: Err("pipe closed".into()),
            });
            assert_eq!(state.failed_daemon_syncs, failures);
            assert!(state.is_daemon_connected, "one or two missed polls are not an outage yet");
        }
        assert_eq!(state.daemon_sync.last_error.as_deref(), Some("pipe closed"));

        // The quiet period has elapsed: the third failure confirms the outage and is reported.
        state.daemon_sync.last_success = Instant::now() - std::time::Duration::from_secs(2);
        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::Sync,
            result: Err("pipe closed".into()),
        });
        assert!(!state.is_daemon_connected);
        let toast = state.toast.as_ref().expect("the outage is reported").0.clone();
        assert_eq!(
            toast,
            format!("{}: pipe closed", crate::i18n::get(state.language).settings_daemon_offline)
        );

        // A successful poll resets the streak and the stored error.
        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::Sync,
            result: Ok(IpcResponse::State(daemon_state_json(serde_json::json!({
                "autorun_enabled": false,
                "uptime_secs": 0,
                "daemon_memory_bytes": 0,
                "daemon_cpu_usage": 0.0,
                "active_rules_count": 0,
                "total_rules_count": 0,
                "config": { "rules": [] },
                "logs": []
            })))),
        });
        assert!(state.is_daemon_connected);
        assert_eq!(state.failed_daemon_syncs, 0);
        assert!(state.daemon_sync.last_error.is_none());
    }

    #[test]
    fn test_process_snapshot_requests_are_rate_limited() {
        let mut state = GuiState::new();
        assert!(state.cached_processes.is_empty());

        state.ensure_processes();
        assert!(state.ipc.processes_in_flight, "an empty cache requests a snapshot");

        // The request fails (no daemon). The inspector asks again on the next frame: the retry
        // must not be issued immediately.
        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::Processes,
            result: Err("Cannot connect to daemon pipe".into()),
        });
        assert!(!state.ipc.processes_in_flight);
        state.ensure_processes();
        assert!(!state.ipc.processes_in_flight, "failed retries are spaced out");

        // Opening the picker is an explicit request and bypasses the spacing.
        state.open_process_picker(ProcessPickerTarget::Trigger);
        assert!(state.ipc.processes_in_flight);
    }

    #[test]
    fn test_overdue_sync_is_reported_instead_of_waiting_forever() {
        let mut state = GuiState::new();
        state.has_connected_daemon = true;
        state.is_daemon_connected = true;
        state.daemon_sync.note_success();
        state.ipc.sync_in_flight = true;
        state.daemon_sync.pending_since = Some(Instant::now() - DAEMON_SYNC_TIMEOUT);

        state.check_sync_timeout();

        assert!(!state.ipc.sync_in_flight, "an overdue poll stops blocking the next one");
        assert_eq!(state.failed_daemon_syncs, 1);
        let error = state.daemon_sync.last_error.as_deref().expect("timeout is recorded");
        assert!(error.contains("did not answer"), "error: {error}");
        assert!(state.is_daemon_connected, "one overdue poll is not an outage");
    }

    // -----------------------------------------------------------------------
    // Direct-file fallback (U05-001)
    // -----------------------------------------------------------------------

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "synector_gui_state_{label}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn test_direct_rule_writes_when_ipc_is_unavailable() {
        let dir = temp_dir("direct_rules");
        let rule = Rule {
            id: "rule_direct".into(),
            name: "Direct".into(),
            is_enabled: false,
            ..Rule::default()
        };

        DirectWrite::Rules(DirectRuleChange::Upsert(Box::new(rule)))
            .apply_in(&dir)
            .expect("upsert");
        let saved = crate::config::RulesFile::read_from(&dir);
        assert_eq!(saved.rules.len(), 1);
        assert_eq!(saved.rules[0].id, "rule_direct");
        assert!(!saved.rules[0].is_enabled);

        DirectWrite::Rules(DirectRuleChange::SetEnabled { id: "rule_direct".into(), enabled: true })
            .apply_in(&dir)
            .expect("toggle");
        assert!(crate::config::RulesFile::read_from(&dir).rules[0].is_enabled);

        DirectWrite::ResetRuntime { rule_id: "rule_direct".into() }
            .apply_in(&dir)
            .expect("reset runtime");
        assert!(crate::config::RulesFile::read_from(&dir).rules[0].is_enabled);

        DirectWrite::Rules(DirectRuleChange::Delete("rule_direct".into()))
            .apply_in(&dir)
            .expect("delete");
        assert!(crate::config::RulesFile::read_from(&dir).rules.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_direct_reorder_keeps_unnamed_rules_at_the_end() {
        let mut rules = vec![
            Rule { id: "a".into(), ..Rule::default() },
            Rule { id: "b".into(), ..Rule::default() },
            Rule { id: "c".into(), ..Rule::default() },
        ];
        reorder_rules(&mut rules, &["c".to_string(), "a".to_string()]);
        let ids: Vec<_> = rules.iter().map(|rule| rule.id.as_str()).collect();
        assert_eq!(ids, vec!["c", "a", "b"]);
    }

    // -----------------------------------------------------------------------
    // Built-in overlay studio (U05-009)
    // -----------------------------------------------------------------------

    #[test]
    fn test_default_overlay_for_builtin_ids() {
        assert_eq!(
            default_overlay_for("overlay_crosshair").id,
            crate::models::OverlayDefinition::default_crosshair().id
        );
        assert_eq!(
            default_overlay_for("overlay_telemetry").id,
            crate::models::OverlayDefinition::default_telemetry().id
        );
        assert_eq!(
            default_overlay_for("overlay_macro_badge").id,
            crate::models::OverlayDefinition::default_macro_badge().id
        );
        assert_eq!(
            default_overlay_for("something_custom").id,
            crate::models::OverlayDefinition::default_new_overlay().id
        );

        let studio = OverlayStudioState::new(
            crate::models::OverlayDefinition::default_crosshair(),
            Some(3),
            true,
            "Saved name".into(),
        );
        assert_eq!(studio.action_target_idx, Some(3));
        assert!(studio.is_exit_pipeline);
        assert_eq!(studio.preset_save_name, "Saved name");
        assert!(studio.undo_stack.is_empty() && studio.redo_stack.is_empty());
        assert!(!studio.is_calibrating);
    }

    // -----------------------------------------------------------------------
    // Share export queue (U05-011)
    // -----------------------------------------------------------------------

    #[test]
    fn test_second_share_copy_queues_instead_of_dropping_the_running_export() {
        let mut state = GuiState::new();
        let (tx, rx) = std::sync::mpsc::channel();
        state.share.rx = Some(rx);

        state.start_share_export(vec![]);
        assert_eq!(state.share.queued.len(), 1, "the second copy is queued");
        assert!(state.share.rx.is_some(), "the running receiver is kept");

        // The first export ends without a result; the queued one takes over.
        drop(tx);
        state.poll_share_events(&egui::Context::default());
        assert!(state.share.queued.is_empty());
        assert!(state.share.rx.is_some(), "the queued export starts after the running one");
    }

    // -----------------------------------------------------------------------
    // Stale update advertisement (U03-013)
    // -----------------------------------------------------------------------

    fn release_info() -> crate::updater::ReleaseInfo {
        crate::updater::ReleaseInfo {
            version: "9.9.9".into(),
            tag_name: "v9.9.9".into(),
            name: "v9.9.9".into(),
            body: "Notes".into(),
            html_url: "http://example.com".into(),
            portable: None,
            installer: None,
        }
    }

    fn daemon_state_json(value: serde_json::Value) -> DaemonState {
        serde_json::from_value(value).expect("daemon state deserializes")
    }

    #[test]
    fn test_manual_check_without_update_clears_the_stale_release() {
        let mut state = GuiState::new();
        state.available_update = Some(release_info());
        state.show_changelog_modal = true;

        state
            .updates
            .tx
            .send(crate::updater::UpdateEvent::CheckComplete {
                result: Box::new(Ok(None)),
                is_manual: true,
            })
            .expect("send check result");
        state.poll_update_events();

        assert!(state.available_update.is_none(), "the manual check found nothing");
        assert!(!state.show_changelog_modal, "the changelog must stop offering the release");
        assert!(state.toast.is_some());
    }

    #[test]
    fn test_daemon_state_without_update_clears_the_stale_release() {
        let mut state = GuiState::new();
        state.available_update = Some(release_info());
        state.show_changelog_modal = true;

        state.apply_daemon_state(daemon_state_json(serde_json::json!({
            "autorun_enabled": false,
            "uptime_secs": 0,
            "daemon_memory_bytes": 0,
            "daemon_cpu_usage": 0.0,
            "active_rules_count": 0,
            "total_rules_count": 0,
            "config": { "rules": [] },
            "logs": []
        })));

        assert!(state.available_update.is_none(), "the daemon is the authority on updates");
        assert!(!state.show_changelog_modal);
    }

    #[test]
    fn test_fresh_manual_result_survives_a_snapshot_taken_before_the_notification() {
        let mut state = GuiState::new();
        // The manual check just advertised a release; its notification is still in flight.
        state.available_update = Some(release_info());
        state.updates.advertise();

        state.apply_daemon_state(daemon_state_json(serde_json::json!({
            "autorun_enabled": false,
            "uptime_secs": 0,
            "daemon_memory_bytes": 0,
            "daemon_cpu_usage": 0.0,
            "active_rules_count": 0,
            "total_rules_count": 0,
            "config": { "rules": [] },
            "logs": []
        })));

        assert!(
            state.available_update.is_some(),
            "a stale snapshot must not erase the release the client just found"
        );
    }
}

