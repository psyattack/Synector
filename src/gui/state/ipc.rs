// ---------------------------------------------------------------------------
// Off-thread plumbing
//
// `GuiState::sync_with_daemon` runs inside `SynectorApp::ui`, so every named-pipe round-trip
// there used to freeze the window when the daemon was slow or hung. Requests now go through
// one worker thread (`IpcGateway`); the frame thread only enqueues and applies results, and a
// mutation keeps its draft until the daemon (or the documented direct-file fallback) confirms it.
// ---------------------------------------------------------------------------

use std::time::Instant;

use crate::ipc::{IpcClient, IpcRequest, IpcResponse};
use crate::models::Rule;

use super::library_write::{LibraryWriteOutcome, LibraryWriteRequest};
use super::GuiState;

/// Identifies a queued IPC request so its result can update GUI state on the frame thread.
/// Mutations carry just enough identity for the success/failure reporting.
#[derive(Clone, PartialEq, Eq)]
pub(super) enum IpcJobKind {
    Sync,
    /// One analytics dashboard read, polled only while its dialog is open.
    Analytics,
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
    KillProcess { pid: u32, name: String },
    /// Result of an inline step test; stored in egui temp data under the view's own id.
    TestStep { result_id: egui::Id },
    OverlayPosition { overlay_id: String },
    SaveSettings { revision: u64, restart_required: bool },
    /// A preset-library write executed through `state::library_write`; the payload lives in
    /// `GuiState::library_writes` until the result is applied.
    LibraryWrite { key: u64 },
    /// Requests whose result has no user-visible effect (acknowledgements, update fan-out).
    Background,
}

impl IpcJobKind {
    /// Short identifier used in log lines; identifiers and outcomes only, never payloads.
    fn label(&self) -> String {
        match self {
            Self::Sync => "GetState".into(),
            Self::Analytics => "GetAnalytics".into(),
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
            Self::KillProcess { pid, .. } => format!("KillProcess #{pid}"),
            Self::TestStep { .. } => "TestStep".into(),
            Self::OverlayPosition { overlay_id } => format!("GetOverlayPosition '{overlay_id}'"),
            Self::SaveSettings { .. } => "SaveSettings".into(),
            Self::LibraryWrite { key } => format!("library write #{key}"),
            Self::Background => "background request".into(),
        }
    }
}

/// One request handed to the IPC worker. `request` is `None` for a library write, which runs the
/// shared preset-library contract instead of one plain round-trip. `direct` is the section write
/// used only when the daemon is unreachable (the fallback promised by
/// `docs/ARCHITECTURE.md#data-flow`).
pub(super) struct IpcJob {
    pub(super) kind: IpcJobKind,
    pub(super) request: Option<IpcRequest>,
    pub(super) direct: Option<DirectWrite>,
    /// Pending preset-library operation executed on the worker thread.
    pub(super) library: Option<LibraryWriteRequest>,
}

/// Worker → frame result. A transport failure that the direct fallback completed arrives as
/// `Ok(IpcResponse::Ok)`, so the frame thread only has to interpret daemon-level outcomes.
pub(super) struct IpcJobResult {
    pub(super) kind: IpcJobKind,
    pub(super) result: Result<IpcResponse, String>,
}

/// Direct section writes used when IPC is unavailable. The daemon remains the only writer
/// while it is reachable, so these run only after a failed liveness probe.
pub(super) enum DirectWrite {
    Rules(DirectRuleChange),
    Library(DirectLibraryChange),
    Settings(Box<crate::config::Settings>),
    ResetRuntime { rule_id: String },
}

pub(super) enum DirectRuleChange {
    Upsert(Box<Rule>),
    Delete(String),
    SetEnabled { id: String, enabled: bool },
    Reorder(Vec<String>),
}

pub(super) enum DirectLibraryChange {
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
            // Window geometry is GUI-owned and never edited by the settings view, so the
            // offline write keeps the file's current geometry instead of overwriting it.
            Self::Settings(cfg) => {
                let current = crate::config::Settings::read_from(dir);
                let mut next = (**cfg).clone();
                next.window = current.window;
                next.save_to(dir).map_err(|e| e.to_string())
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
pub(super) enum MutationFailure {
    /// Daemon rejection or transport failure. Identifiers only — never user data.
    Detail(String),
    /// Unexpected response shape, described for log lines by its variant name.
    UnexpectedResponse(String),
}

impl MutationFailure {
    /// Detail for log lines and for toasts whose catalog line embeds one.
    pub(super) fn diagnostic(&self) -> &str {
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
        IpcResponse::Analytics(_) => "Analytics",
        IpcResponse::Processes(_) => "Processes",
        IpcResponse::TestStepResult(_) => "TestStepResult",
        IpcResponse::OverlayPosition(_) => "OverlayPosition",
        IpcResponse::ShareImportResult(_) => "ShareImportResult",
        IpcResponse::UpdateProgress { .. } => "UpdateProgress",
        IpcResponse::Error(_) => "Error",
    }
}

/// Runs one request on the IPC worker. A preset-library write runs the shared library contract
/// (which owns its own liveness-gated fallback and logging); settings keep their own fallback
/// contract; every other request uses the generic direct-section fallback.
fn run_ipc_job(job: &IpcJob) -> IpcJobResult {
    let result = if let Some(library) = &job.library {
        match crate::gui::state::library_write::execute(library) {
            LibraryWriteOutcome::Persisted => Ok(IpcResponse::Ok),
            LibraryWriteOutcome::Failed(reason) => Ok(IpcResponse::Error(reason)),
        }
    } else if matches!(job.kind, IpcJobKind::SaveSettings { .. }) {
        run_settings_job(job)
    } else {
        run_request_job(job)
    };
    IpcJobResult {
        kind: job.kind.clone(),
        result,
    }
}

/// Settings have their own fallback contract: after any non-`Ok` answer (including a daemon
/// rejection) the current path still writes `settings.json` directly, keeping the file's window
/// geometry, and reports the daemon error joined with the write error.
fn run_settings_job(job: &IpcJob) -> Result<IpcResponse, String> {
    let Some(request) = &job.request else {
        return Err("settings request is missing".to_string());
    };
    let daemon_error = match IpcClient::call(request) {
        Ok(IpcResponse::Ok) => return Ok(IpcResponse::Ok),
        Ok(IpcResponse::Error(error)) => error,
        Ok(_) => "the daemon did not confirm the settings write".to_string(),
        Err(error) => error,
    };
    match &job.direct {
        Some(direct) if direct_fallback_allowed() => {
            match direct.apply_in(&crate::config::AppConfig::config_dir()) {
                Ok(()) => Ok(IpcResponse::Ok),
                Err(error) => Err(format!("{daemon_error}; {error}")),
            }
        }
        _ => Err(daemon_error),
    }
}

fn run_request_job(job: &IpcJob) -> Result<IpcResponse, String> {
    let Some(request) = &job.request else {
        return Err("IPC job is missing its request".to_string());
    };
    match IpcClient::call(request) {
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
    }
}

/// Off-thread IPC gateway: the frame thread enqueues, the worker serializes round-trips.
pub(super) struct IpcGateway {
    job_tx: std::sync::mpsc::Sender<IpcJob>,
    pub(super) result_rx: std::sync::mpsc::Receiver<IpcJobResult>,
    pub(super) sync_in_flight: bool,
    pub(super) processes_in_flight: bool,
    /// One calibration-position poll at a time: the studio asks on every idle frame while it is
    /// calibrating, and a queued backlog would report stale coordinates.
    pub(super) overlay_position_in_flight: bool,
    /// When the last process snapshot was requested, to space retries that failed.
    pub(super) last_process_request: Option<Instant>,
    /// One analytics snapshot request at a time, and when the last one was queued, so the dialog
    /// polls on a fixed cadence instead of queueing a backlog.
    pub(super) analytics_in_flight: bool,
    pub(super) analytics_last_poll: Instant,
    /// Latest frame context, used to schedule a repaint when a request is queued so its result
    /// is applied promptly instead of waiting for the next input event.
    pub(super) frame_ctx: Option<egui::Context>,
}

impl IpcGateway {
    pub(super) fn new() -> Self {
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
            overlay_position_in_flight: false,
            last_process_request: None,
            analytics_in_flight: false,
            analytics_last_poll: Instant::now(),
            frame_ctx: None,
        }
    }

    pub(super) fn attach_frame_ctx(&mut self, ctx: &egui::Context) {
        self.frame_ctx = Some(ctx.clone());
    }

    /// Wakes the frame loop when a periodic poll is due: the window otherwise repaints on its own
    /// cadence, which would push the poll past the interval.
    pub(super) fn request_repaint_after(&self, delay: std::time::Duration) {
        if let Some(ctx) = &self.frame_ctx {
            ctx.request_repaint_after(delay);
        }
    }

    /// Queues a request and requests a repaint for its result. Returns `false` when the worker
    /// is gone, so callers can clear their in-flight markers.
    pub(super) fn send(&self, job: IpcJob) -> bool {
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

impl GuiState {
    /// Queues a request to the IPC worker. `direct` is the section write used only when the
    /// daemon is unreachable; the frame thread never sees the round-trip.
    pub(super) fn enqueue_ipc(&self, kind: IpcJobKind, request: IpcRequest, direct: Option<DirectWrite>) -> bool {
        self.ipc.send(IpcJob {
            kind,
            request: Some(request),
            direct,
            library: None,
        })
    }

    /// Queues one preset-library operation. The payload stays in `library_writes` so the result
    /// handler can update the snapshot and toast; the worker runs the shared library contract.
    pub(super) fn enqueue_library_write(&self, key: u64, library: LibraryWriteRequest) -> bool {
        self.ipc.send(IpcJob {
            kind: IpcJobKind::LibraryWrite { key },
            request: None,
            direct: None,
            library: Some(library),
        })
    }

    /// `Some(failure)` unless the daemon (or the direct fallback) confirmed the mutation.
    pub(super) fn mutation_failure(result: Result<IpcResponse, String>) -> Option<MutationFailure> {
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
    pub(super) fn mutation_failure_toast(&self, failure: &MutationFailure) -> String {
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

    pub fn toggle_autorun(&mut self, enable: bool) {
        self.enqueue_ipc(IpcJobKind::SetAutorun { enabled: enable }, IpcRequest::SetAutorun(enable), None);
    }

    pub(super) fn apply_ipc_result(&mut self, job: IpcJobResult) {
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
            IpcJobKind::Analytics => {
                self.ipc.analytics_in_flight = false;
                match result {
                    Ok(IpcResponse::Analytics(snapshot)) => self.analytics = Some(snapshot),
                    Ok(IpcResponse::Error(error)) => crate::logging::debug(
                        "GUI",
                        &format!("Analytics snapshot failed: {error}"),
                    ),
                    Ok(other) => crate::logging::warn(
                        "GUI",
                        &format!(
                            "Analytics snapshot failed: unexpected {} response",
                            ipc_response_name(&other)
                        ),
                    ),
                    // The state sync already reports a confirmed outage; a dashboard retry would
                    // only duplicate that line every second.
                    Err(error) => {
                        crate::logging::debug("GUI", &format!("Analytics snapshot failed: {error}"))
                    }
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
            IpcJobKind::KillProcess { pid, name } => self.apply_kill_process_result(*pid, name, result),
            IpcJobKind::TestStep { result_id } => self.apply_test_step_result(*result_id, result),
            IpcJobKind::OverlayPosition { overlay_id } => {
                self.apply_overlay_position_result(overlay_id, result)
            }
            IpcJobKind::SaveSettings {
                revision,
                restart_required,
            } => self.apply_settings_save_result(*revision, *restart_required, result),
            IpcJobKind::LibraryWrite { key } => self.apply_library_write_result(*key, result),
            IpcJobKind::Background => {
                if let Err(error) = result {
                    crate::logging::debug("GUI", &format!("{} failed: {error}", kind.label()));
                }
            }
        }
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

    /// Queues a force-terminate for one process. The selection is cleared when the daemon
    /// confirms, not on the click, so the drawer never closes for a refused request.
    pub fn request_kill_process(&mut self, pid: u32, name: &str) {
        self.enqueue_ipc(
            IpcJobKind::KillProcess {
                pid,
                name: name.to_string(),
            },
            IpcRequest::KillProcess { pid, force: true },
            None,
        );
    }

    fn apply_kill_process_result(&mut self, pid: u32, name: &str, result: Result<IpcResponse, String>) {
        let loc = self.loc();
        match result {
            Ok(IpcResponse::Ok) => {
                self.set_toast(format!("{} {} #{}", loc.toast_process_terminated, name, pid));
                if self.selected_process.as_ref().is_some_and(|proc| proc.pid == pid) {
                    self.selected_process = None;
                }
            }
            Ok(IpcResponse::Error(error)) => {
                crate::logging::warn("GUI", &format!("KillProcess #{pid} was refused: {error}"));
                self.set_toast(loc.toast_request_failed.replace("{}", &error));
            }
            Ok(_) => {
                crate::logging::warn(
                    "GUI",
                    &format!("KillProcess #{pid} returned an unexpected response"),
                );
                self.set_toast(loc.toast_request_failed_unexpected.to_string());
            }
            Err(error) => {
                crate::logging::warn("GUI", &format!("KillProcess #{pid} failed: {error}"));
                self.set_toast(loc.toast_request_failed.replace("{}", &error));
            }
        }
    }

    /// Queues one inline step test; the localized outcome reaches the inspector through
    /// [`Self::take_test_step_results`] on a later frame.
    pub fn test_step(
        &self,
        action: &crate::models::Action,
        context: std::collections::HashMap<String, String>,
        result_id: egui::Id,
    ) {
        self.enqueue_ipc(
            IpcJobKind::TestStep { result_id },
            IpcRequest::TestStep {
                action: action.clone(),
                context,
            },
            None,
        );
    }

    fn apply_test_step_result(&mut self, result_id: egui::Id, result: Result<IpcResponse, String>) {
        let loc = self.loc();
        let (ok, message) = match result {
            Ok(IpcResponse::TestStepResult(Ok(()))) => {
                (true, loc.inspector_test_step_passed.to_string())
            }
            Ok(IpcResponse::TestStepResult(Err(error))) => {
                (false, loc.inspector_test_step_failed.replace("{}", &error))
            }
            Ok(IpcResponse::Error(error)) => {
                (false, loc.inspector_test_step_error.replace("{}", &error))
            }
            Err(error) => (false, loc.inspector_test_step_ipc_error.replace("{}", &error)),
            Ok(_) => (false, loc.inspector_test_step_unknown.to_string()),
        };
        self.pending_test_step_results
            .push((result_id, (ok, message, Instant::now())));
    }

    /// Drains completed step tests for the inspector to place in egui temp data under the ids it
    /// computed when the user pressed Test.
    pub fn take_test_step_results(&mut self) -> Vec<(egui::Id, (bool, String, Instant))> {
        std::mem::take(&mut self.pending_test_step_results)
    }

    /// Polls the daemon's live overlay position while the studio is calibrating; at most one poll
    /// is in flight, and the latest answer is cached for the next frame.
    pub fn request_overlay_position(&mut self, overlay_id: &str) {
        if self.ipc.overlay_position_in_flight {
            return;
        }
        if self.enqueue_ipc(
            IpcJobKind::OverlayPosition {
                overlay_id: overlay_id.to_string(),
            },
            IpcRequest::GetOverlayPosition(overlay_id.to_string()),
            None,
        ) {
            self.ipc.overlay_position_in_flight = true;
        }
    }

    fn apply_overlay_position_result(&mut self, overlay_id: &str, result: Result<IpcResponse, String>) {
        self.ipc.overlay_position_in_flight = false;
        if let Ok(IpcResponse::OverlayPosition(Some((x, y)))) = result {
            self.cached_overlay_position = Some((overlay_id.to_string(), x, y));
        }
    }

    /// Latest calibration position reported for this overlay, if any.
    pub fn overlay_position_for(&self, overlay_id: &str) -> Option<(f32, f32)> {
        self.cached_overlay_position
            .as_ref()
            .filter(|(id, _, _)| id == overlay_id)
            .map(|(_, x, y)| (*x, *y))
    }

    /// Shows the live calibration preview in the overlay process (fire and forget: the studio
    /// mirrors the card locally and never waits for this round-trip).
    pub fn request_show_overlay_preview(
        &mut self,
        overlay: crate::models::OverlayDefinition,
        calibration_mode: bool,
    ) {
        self.enqueue_ipc(
            IpcJobKind::Background,
            IpcRequest::ShowOverlayPreview {
                overlay,
                calibration_mode,
            },
            None,
        );
    }

    /// Hides the live calibration preview in the overlay process.
    pub fn request_hide_overlay_preview(&mut self, overlay_id: &str) {
        self.enqueue_ipc(
            IpcJobKind::Background,
            IpcRequest::HideOverlayPreview(overlay_id.to_string()),
            None,
        );
    }

    /// Queues a settings write and returns the draft revision it belongs to. A confirmation for
    /// this revision lets the settings view drop its unsaved draft.
    pub fn request_save_settings(
        &mut self,
        cfg: crate::config::Settings,
        restart_required: bool,
    ) -> u64 {
        self.settings_revision += 1;
        let revision = self.settings_revision;
        self.enqueue_ipc(
            IpcJobKind::SaveSettings {
                revision,
                restart_required,
            },
            IpcRequest::SaveSettings(cfg.clone()),
            Some(DirectWrite::Settings(Box::new(cfg))),
        );
        revision
    }

    /// Whether the settings write of this revision (or a newer one) was confirmed.
    pub fn settings_write_confirmed(&self, revision: u64) -> bool {
        self.settings_saved_revision
            .is_some_and(|saved| saved >= revision)
    }

    fn apply_settings_save_result(
        &mut self,
        revision: u64,
        restart_required: bool,
        result: Result<IpcResponse, String>,
    ) {
        match Self::mutation_failure(result) {
            None => {
                self.settings_saved_revision = Some(revision);
                let loc = self.loc();
                self.set_toast(if restart_required {
                    loc.settings_mask_restart_required
                } else {
                    loc.toast_config_saved
                });
                self.daemon_sync.force();
            }
            Some(failure) => {
                crate::logging::error(
                    "CONFIG",
                    &format!("Failed to save settings: {}", failure.diagnostic()),
                );
                self.set_toast(
                    self.loc()
                        .toast_settings_save_failed
                        .replace("{}", failure.diagnostic()),
                );
            }
        }
    }

    fn apply_library_write_result(&mut self, key: u64, result: Result<IpcResponse, String>) {
        let Some(pending) = self.library_writes.remove(&key) else {
            return;
        };
        let loc = self.loc();
        let persisted = matches!(result, Ok(IpcResponse::Ok));
        match &pending.request {
            LibraryWriteRequest::SaveOverlay(definition) => {
                if persisted {
                    if let Some(daemon) = &mut self.daemon_state {
                        daemon.config.overlays.retain(|overlay| overlay.id != definition.id);
                        daemon.config.overlays.push((**definition).clone());
                    }
                    self.set_toast(loc.presets_saved_toast.replace("{}", &pending.name));
                } else {
                    self.set_toast(loc.presets_save_failed);
                }
            }
            LibraryWriteRequest::DeleteOverlay(overlay_id) => {
                if persisted {
                    if let Some(daemon) = &mut self.daemon_state {
                        daemon.config.overlays.retain(|overlay| &overlay.id != overlay_id);
                    }
                } else {
                    self.set_toast(loc.presets_delete_failed);
                }
            }
            LibraryWriteRequest::SaveMacroPreset(preset) => {
                if persisted {
                    if let Some(daemon) = &mut self.daemon_state {
                        if let Some(existing) = daemon.config.macro_presets.iter_mut().find(|p| p.id == preset.id) {
                            *existing = (**preset).clone();
                        } else {
                            daemon.config.macro_presets.push((**preset).clone());
                        }
                    }
                    self.set_toast(loc.presets_saved_toast.replace("{}", &pending.name));
                } else {
                    self.set_toast(loc.presets_save_failed);
                }
            }
            LibraryWriteRequest::DeleteMacroPreset(preset_id) => {
                if persisted {
                    if let Some(daemon) = &mut self.daemon_state {
                        daemon.config.macro_presets.retain(|preset| &preset.id != preset_id);
                    }
                } else {
                    self.set_toast(loc.presets_delete_failed);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn process_item(pid: u32, name: &str) -> crate::models::ProcessSnapshotItem {
        crate::models::ProcessSnapshotItem {
            pid,
            name: name.to_string(),
            window_title: String::new(),
            memory_bytes: 0,
            cpu_percent: 0.0,
        }
    }

    #[test]
    fn test_settings_save_success_requests_restart_notice_and_confirms_the_revision() {
        let mut state = GuiState::new();
        state.language = crate::i18n::Language::English;
        let en = crate::i18n::get(crate::i18n::Language::English);

        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::SaveSettings {
                revision: 3,
                restart_required: true,
            },
            result: Ok(IpcResponse::Ok),
        });
        assert_eq!(
            state.toast.as_ref().expect("success is reported").0,
            en.settings_mask_restart_required
        );
        assert!(state.settings_write_confirmed(3));
        assert!(!state.settings_write_confirmed(4), "a newer draft is still unsaved");
        assert_ne!(en.settings_mask_restart_required, en.toast_config_saved);

        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::SaveSettings {
                revision: 4,
                restart_required: false,
            },
            result: Ok(IpcResponse::Ok),
        });
        assert_eq!(
            state.toast.as_ref().expect("success is reported").0,
            en.toast_config_saved
        );
        assert!(state.settings_write_confirmed(4));
    }

    #[test]
    fn test_settings_save_failure_is_reported_and_keeps_the_draft() {
        let mut state = GuiState::new();
        state.language = crate::i18n::Language::English;
        let en = crate::i18n::get(crate::i18n::Language::English);

        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::SaveSettings {
                revision: 1,
                restart_required: false,
            },
            result: Err("disk full".into()),
        });
        assert!(
            !state.settings_write_confirmed(1),
            "a failed write leaves the draft unsaved"
        );
        assert_eq!(
            state.toast.as_ref().expect("failure is reported").0,
            en.toast_settings_save_failed.replace("{}", "disk full")
        );
    }

    #[test]
    fn test_kill_process_result_reports_and_clears_the_selection() {
        let mut state = GuiState::new();
        state.language = crate::i18n::Language::English;
        let en = crate::i18n::get(crate::i18n::Language::English);

        state.selected_process = Some(process_item(42, "game.exe"));
        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::KillProcess {
                pid: 42,
                name: "game.exe".into(),
            },
            result: Ok(IpcResponse::Ok),
        });
        assert!(state.selected_process.is_none(), "a confirmed kill clears the drawer");
        assert_eq!(
            state.toast.as_ref().expect("success is reported").0,
            format!("{} game.exe #42", en.toast_process_terminated)
        );

        state.selected_process = Some(process_item(42, "game.exe"));
        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::KillProcess {
                pid: 42,
                name: "game.exe".into(),
            },
            result: Ok(IpcResponse::Error("denied".into())),
        });
        assert!(
            state.selected_process.is_some(),
            "a refused termination keeps the selection"
        );
        assert_eq!(
            state.toast.as_ref().expect("failure is reported").0,
            en.toast_request_failed.replace("{}", "denied")
        );
    }

    #[test]
    fn test_test_step_results_are_queued_for_the_view() {
        let mut state = GuiState::new();
        state.language = crate::i18n::Language::English;
        let en = crate::i18n::get(crate::i18n::Language::English);
        let result_id = egui::Id::unique("test_step_queue");

        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::TestStep { result_id },
            result: Ok(IpcResponse::TestStepResult(Ok(()))),
        });
        let results = state.take_test_step_results();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, result_id);
        assert!(results[0].1.0);
        assert_eq!(results[0].1.1, en.inspector_test_step_passed);
        assert!(
            state.take_test_step_results().is_empty(),
            "drained results are consumed"
        );

        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::TestStep { result_id },
            result: Ok(IpcResponse::TestStepResult(Err("boom".into()))),
        });
        let results = state.take_test_step_results();
        assert!(!results[0].1.0);
        assert_eq!(
            results[0].1.1,
            en.inspector_test_step_failed.replace("{}", "boom")
        );
    }

    #[test]
    fn test_overlay_position_result_is_cached_per_overlay() {
        let mut state = GuiState::new();
        state.ipc.overlay_position_in_flight = true;
        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::OverlayPosition {
                overlay_id: "overlay_one".into(),
            },
            result: Ok(IpcResponse::OverlayPosition(Some((120.0, 80.0)))),
        });
        assert_eq!(state.overlay_position_for("overlay_one"), Some((120.0, 80.0)));
        assert_eq!(state.overlay_position_for("overlay_two"), None);
        assert!(
            !state.ipc.overlay_position_in_flight,
            "the poll flag is cleared"
        );
    }

    #[test]
    fn test_library_write_results_update_the_snapshot_and_toast() {
        use crate::gui::state::library_write::{LibraryWriteRequest, PendingLibraryWrite};

        let mut state = GuiState::new();
        state.language = crate::i18n::Language::English;
        let en = crate::i18n::get(crate::i18n::Language::English);
        let mut definition = crate::models::OverlayDefinition::default_new_overlay();
        definition.id = "preset_x".into();
        definition.name = "Preset X".into();
        state.library_writes.insert(
            7,
            PendingLibraryWrite {
                request: LibraryWriteRequest::SaveOverlay(Box::new(definition)),
                name: "Preset X".into(),
            },
        );

        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::LibraryWrite { key: 7 },
            result: Ok(IpcResponse::Ok),
        });
        assert!(state.library_writes.is_empty(), "the pending write is consumed");
        assert_eq!(
            state.toast.as_ref().expect("success is reported").0,
            en.presets_saved_toast.replace("{}", "Preset X")
        );

        state.library_writes.insert(
            8,
            PendingLibraryWrite {
                request: LibraryWriteRequest::DeleteOverlay("preset_x".into()),
                name: String::new(),
            },
        );
        state.apply_ipc_result(IpcJobResult {
            kind: IpcJobKind::LibraryWrite { key: 8 },
            result: Err("denied".into()),
        });
        assert_eq!(
            state.toast.as_ref().expect("failure is reported").0,
            en.presets_delete_failed
        );
    }
}
