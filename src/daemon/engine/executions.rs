use std::collections::HashMap;
use std::time::Instant;

use tokio::sync::oneshot;

use crate::models::{ResourceLifetime, RunSource};

use super::DaemonEngine;

/// Monotonic id shared by every kind of pipeline execution (triggered, exit, recurring and
/// rule-called). Active executions and abort handles are keyed by it, so it must never collide.
static EXECUTION_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub(super) fn next_execution_id() -> u64 {
    EXECUTION_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Identity of the process whose lifetime an owned resource follows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrimaryProcess {
    pub pid: u32,
    pub name: String,
}

/// One process a run owns, identified by the PID captured at spawn.
#[derive(Debug, Clone)]
pub struct OwnedProcess {
    pub execution_id: u64,
    pub rule_id: String,
    pub primary: PrimaryProcess,
    pub pid: u32,
    pub image_path: String,
    pub lifetime: ResourceLifetime,
}

/// One overlay show a run owns, identified by the generation of that show.
#[derive(Debug, Clone)]
pub struct OwnedOverlay {
    pub execution_id: u64,
    pub rule_id: String,
    pub primary: PrimaryProcess,
    pub overlay_id: String,
    pub generation: u64,
    pub lifetime: ResourceLifetime,
}

/// A runtime object a pipeline owns and must release according to its [`ResourceLifetime`].
#[derive(Debug, Clone)]
pub enum OwnedResource {
    Process(OwnedProcess),
    Overlay(OwnedOverlay),
}

impl OwnedResource {
    pub fn execution_id(&self) -> u64 {
        match self {
            Self::Process(process) => process.execution_id,
            Self::Overlay(overlay) => overlay.execution_id,
        }
    }

    pub fn rule_id(&self) -> &str {
        match self {
            Self::Process(process) => &process.rule_id,
            Self::Overlay(overlay) => &overlay.rule_id,
        }
    }

    pub fn primary(&self) -> &PrimaryProcess {
        match self {
            Self::Process(process) => &process.primary,
            Self::Overlay(overlay) => &overlay.primary,
        }
    }

    pub fn lifetime(&self) -> ResourceLifetime {
        match self {
            Self::Process(process) => process.lifetime,
            Self::Overlay(overlay) => overlay.lifetime,
        }
    }

    pub fn process_pid(&self) -> Option<u32> {
        match self {
            Self::Process(process) => Some(process.pid),
            Self::Overlay(_) => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ActiveExecutionItem {
    pub rule_id: String,
    pub step_index: usize,
    pub total_steps: usize,
    pub started_at: Instant,
    /// Rules that led to this execution, outermost first. Empty for triggered runs; `["a"]`
    /// for a rule-called run, `["a", "b"]` when `a` called `b` which called this one.
    pub call_chain: Vec<String>,
}

/// Why a triggered or called rule was not started. Skipping is not an error, but the reason is
/// reported to the caller so a rule-to-rule call can log it.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum RuleSkip {
    Quota,
    Cooldown { elapsed: f64, cooldown: f64 },
    Conditions,
    EmptyPipeline,
}

impl RuleSkip {
    pub(super) fn describe(&self) -> String {
        match self {
            Self::Quota => "its execution quota is exhausted".to_string(),
            Self::Cooldown { elapsed, cooldown } => format!(
                "its cooldown is still active ({elapsed:.1}s of {cooldown:.1}s)"
            ),
            Self::Conditions => "its conditions are not met".to_string(),
            Self::EmptyPipeline => "its action pipeline is empty".to_string(),
        }
    }
}

/// Why a pipeline is running. A process id of zero is not a manual run: webhooks, schedules,
/// system events and hotkeys all run without a pid, and only user-invoked runs may tear down the
/// rule's owned resources when they finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionOrigin {
    /// A run the user started from the GUI or through a manual trigger emulation.
    #[allow(dead_code)] // Explicit origins left no production producer; a manual run is stated by its caller.
    Manual,
    /// The GUI's test / run emulation: user-invoked, so it keeps manual semantics and bypasses
    /// the cooldown.
    Test,
    Schedule,
    SystemEvent,
    Webhook,
    Hotkey,
    /// Started by another rule's `rule_control` call.
    RuleInvocation,
    /// A process start, window focus or resource-threshold watch.
    Process,
}

impl ExecutionOrigin {
    /// True for runs the user started directly. Only those replace and release the rule's owned
    /// resources at start and after the pipeline finishes, and the cooldown never gates them.
    pub(super) fn is_user_invoked(self) -> bool {
        matches!(self, Self::Manual | Self::Test)
    }
}

/// Outcome of a rule-to-rule call request.
#[derive(Debug)]
pub enum RuleCallStart {
    /// The target pipeline runs; the receiver resolves when it finishes or is aborted.
    Started(oneshot::Receiver<()>),
    /// The target exists but was intentionally not started; the engine logged the reason.
    Skipped,
}

/// Engine-side data for a pipeline started by another rule.
pub(super) struct RuleCallRuntime {
    /// Rules that led to this execution, outermost first, ending with the target itself.
    pub(super) chain: Vec<String>,
    pub(super) completion: oneshot::Sender<()>,
}

/// How one pipeline ended. The shared step loop reports it exactly once, so the analytics
/// outcome counters do not depend on which spawn path started the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineOutcome {
    /// Every step succeeded.
    Completed,
    /// At least one step failed; a non-stopping error still counts.
    Failed,
    /// A step requested a graceful abort of the pipeline.
    Aborted,
}

#[derive(Debug)]
pub enum ActionCompletionEvent {
    Log {
        level: String,
        category: String,
        message: String,
    },
    /// Like [`Self::Log`] in the UI, but `file_message` is written to the daily file log instead of
    /// `message`: action errors can embed script stdout/stderr or HTTP response bodies, and those
    /// must not leave the in-app log.
    SensitiveLog {
        level: String,
        category: String,
        message: String,
        file_message: String,
    },
    AddOwnedResource(OwnedResource),
    RuleExecutionStarted {
        execution_id: u64,
        rule_id: String,
        step_index: usize,
        total_steps: usize,
        call_chain: Vec<String>,
    },
    RuleExecutionStep {
        execution_id: u64,
        _rule_id: String,
        step_index: usize,
        total_steps: usize,
    },
    RuleExecutionFinished {
        execution_id: u64,
        rule_id: String,
        is_manual: bool,
    },
    SetRuleCustomIcon {
        rule_id: String,
        icon_path: String,
    },
    IncrementExecutionCount {
        rule_id: String,
    },
    /// One cycle of a recurring (interval or held-hotkey) execution started. Reported at the
    /// cycle start, so a cycle that fails or is aborted mid-run still counts as a run. The source
    /// travels with the event because the cycle starts inside the loop, where no engine-side
    /// origin exists; it is a run-now emulation, so it is never attributed to the rule's
    /// configured trigger.
    RecurringCycleStarted {
        rule_id: String,
        source: RunSource,
    },
    /// One finished pipeline and how it ended.
    RunOutcome {
        outcome: PipelineOutcome,
    },
}

/// Runtime execution tracker: the pipelines that are running, their abort handles, the recurring
/// test loops and the processes/overlays those runs own. The engine mutates it only through these
/// methods, so an aborted pipeline always loses its abort handle and a finished execution always
/// leaves the recurring set.
#[derive(Default)]
pub(super) struct ExecutionTracker {
    /// Pipelines currently running, keyed by the process-wide execution id.
    pub(super) active_executions: HashMap<u64, ActiveExecutionItem>,
    /// Abort handles of spawned pipeline tasks: execution id → (owning rule id, handle).
    abort_handles: HashMap<u64, (String, tokio::task::AbortHandle)>,
    /// Recurring test loops (interval / held-hotkey emulation): rule id → (abort handle,
    /// execution id the loop was started as).
    recurring_tests: HashMap<String, (tokio::task::AbortHandle, u64)>,
    /// Processes and overlay shows owned by rule runs, released by lifetime.
    owned_resources: Vec<OwnedResource>,
}

impl ExecutionTracker {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// True when a tracked execution kind is running for `rule_id`, including resources a finished
    /// pipeline still owns.
    pub(super) fn is_rule_executing(&self, rule_id: &str) -> bool {
        self.active_executions
            .values()
            .any(|item| item.rule_id == rule_id)
            || self
                .abort_handles
                .values()
                .any(|(owning_rule, _)| owning_rule == rule_id)
            || self.recurring_tests.contains_key(rule_id)
            || self
                .owned_resources
                .iter()
                .any(|resource| {
                    resource.rule_id() == rule_id
                        && resource.lifetime() != ResourceLifetime::Detach
                })
    }

    pub(super) fn has_recurring_test(&self, rule_id: &str) -> bool {
        self.recurring_tests.contains_key(rule_id)
    }

    pub(super) fn recurring_rule_ids(&self) -> impl Iterator<Item = &String> {
        self.recurring_tests.keys()
    }

    pub(super) fn active_executions(&self) -> impl Iterator<Item = &ActiveExecutionItem> {
        self.active_executions.values()
    }

    /// The call chain recorded for a running execution, outermost first.
    pub(super) fn call_chain(&self, execution_id: u64) -> Vec<String> {
        self.active_executions
            .get(&execution_id)
            .map(|execution| execution.call_chain.clone())
            .unwrap_or_default()
    }

    /// True when `execution_id` is currently running `rule_id` (self-call detection).
    pub(super) fn is_running_rule(&self, execution_id: u64, rule_id: &str) -> bool {
        self.active_executions
            .get(&execution_id)
            .is_some_and(|execution| execution.rule_id == rule_id)
    }

    /// Rule owning a running execution, used to attribute a rule call to its caller.
    pub(super) fn running_rule_id(&self, execution_id: u64) -> Option<&str> {
        self.active_executions
            .get(&execution_id)
            .map(|execution| execution.rule_id.as_str())
    }

    pub(super) fn record_started(&mut self, execution_id: u64, item: ActiveExecutionItem) {
        self.active_executions.insert(execution_id, item);
    }

    pub(super) fn record_step(&mut self, execution_id: u64, step_index: usize, total_steps: usize) {
        if let Some(item) = self.active_executions.get_mut(&execution_id) {
            item.step_index = step_index;
            item.total_steps = total_steps;
        }
    }

    /// Forgets a finished pipeline: its entry, its abort handle and — when this execution was
    /// the recurring loop — the recurring-test entry.
    pub(super) fn record_finished(&mut self, execution_id: u64, rule_id: &str) {
        self.active_executions.remove(&execution_id);
        self.abort_handles.remove(&execution_id);
        if self
            .recurring_tests
            .get(rule_id)
            .is_some_and(|(_, recurring_execution)| *recurring_execution == execution_id)
        {
            self.recurring_tests.remove(rule_id);
        }
    }

    pub(super) fn register_abort_handle(
        &mut self,
        execution_id: u64,
        rule_id: String,
        handle: tokio::task::AbortHandle,
    ) {
        self.abort_handles.insert(execution_id, (rule_id, handle));
    }

    pub(super) fn register_recurring_test(
        &mut self,
        rule_id: String,
        handle: tokio::task::AbortHandle,
        execution_id: u64,
    ) {
        self.recurring_tests.insert(rule_id, (handle, execution_id));
    }

    /// Aborts the recurring test loop of `rule_id`, if one is running.
    pub(super) fn abort_recurring_test(&mut self, rule_id: &str) -> bool {
        let Some((abort_handle, execution_id)) = self.recurring_tests.remove(rule_id) else {
            return false;
        };
        abort_handle.abort();
        self.active_executions.remove(&execution_id);
        self.abort_handles.remove(&execution_id);
        true
    }

    /// Aborts every live pipeline of `rule_id` and returns how many had an abort handle. The
    /// engine logs one line per aborted execution, as the pre-split loop did.
    pub(super) fn abort_rule(&mut self, rule_id: &str) -> usize {
        let to_abort: Vec<u64> = self
            .abort_handles
            .iter()
            .filter_map(|(&execution_id, (owning_rule, _))| {
                (owning_rule == rule_id).then_some(execution_id)
            })
            .collect();

        let mut aborted = 0;
        for execution_id in to_abort {
            if let Some((_, abort_handle)) = self.abort_handles.remove(&execution_id) {
                abort_handle.abort();
                self.active_executions.remove(&execution_id);
                aborted += 1;
            }
        }

        self.active_executions
            .retain(|_, item| item.rule_id != rule_id);
        aborted
    }

    /// Records a new owned resource for a run. A later show of the same overlay replaces the
    /// run's previous ownership of that id, so one run owns at most one show of an overlay.
    /// Detached overlays are recorded for stop cleanup but are not "live resources": they do not
    /// count toward rule activity and do not make the rule report running.
    pub(super) fn add_owned_resource(&mut self, resource: OwnedResource) {
        if let OwnedResource::Overlay(new) = &resource {
            let mut replaced_rules = Vec::new();
            self.owned_resources.retain(|existing| {
                let OwnedResource::Overlay(old) = existing else {
                    return true;
                };
                let same =
                    old.execution_id == new.execution_id && old.overlay_id == new.overlay_id;
                if same && old.lifetime != ResourceLifetime::Detach {
                    replaced_rules.push(old.rule_id.clone());
                }
                !same
            });
            for rule_id in replaced_rules {
                crate::daemon::rule_activity::mark_owned_released(&rule_id);
            }
        }
        if resource.lifetime() != ResourceLifetime::Detach {
            crate::daemon::rule_activity::mark_owned_acquired(resource.rule_id());
        }
        self.owned_resources.push(resource);
    }

    pub(super) fn owned_resources(&self) -> &[OwnedResource] {
        &self.owned_resources
    }

    /// True when any owned process is still tracked. The daemon process loop uses this to keep
    /// polling long enough to notice the host exit that releases it.
    pub(super) fn has_owned_processes(&self) -> bool {
        self.owned_resources
            .iter()
            .any(|resource| resource.process_pid().is_some())
    }

    /// Removes and returns every owned resource matching `predicate`, keeping insertion order and
    /// releasing the per-rule activity count for each taken resource.
    pub(super) fn take_owned_where(
        &mut self,
        mut predicate: impl FnMut(&OwnedResource) -> bool,
    ) -> Vec<OwnedResource> {
        let mut taken = Vec::new();
        let mut index = 0;
        while index < self.owned_resources.len() {
            if predicate(&self.owned_resources[index]) {
                taken.push(self.owned_resources.remove(index));
            } else {
                index += 1;
            }
        }
        for resource in &taken {
            if resource.lifetime() != ResourceLifetime::Detach {
                crate::daemon::rule_activity::mark_owned_released(resource.rule_id());
            }
        }
        taken
    }
}

impl DaemonEngine {
    /// Whether any rule still owns a live process. The daemon process loop keeps polling while
    /// this is true so the host exit that releases it is noticed.
    pub fn has_active_owned_processes(&self) -> bool {
        self.executions.has_owned_processes()
    }

    pub fn is_rule_executing(&self, rule_id: &str) -> bool {
        self.executions.is_rule_executing(rule_id)
    }

    /// Aborts the recurring test loop of `rule_id`, if one is running. Recurring loops re-read
    /// their stored rule every cycle, so this is only needed when the rule goes away or its
    /// definition is replaced while the loop runs.
    pub(super) fn abort_recurring_test(&mut self, rule_id: &str) -> bool {
        if !self.executions.abort_recurring_test(rule_id) {
            return false;
        }
        self.log("INFO", "RULE", &format!("Stopped recurring test run of rule: {}", rule_id));
        true
    }

    /// Aborts every active execution of `rule_id` and releases its runtime resources without
    /// changing `is_enabled`. The user-facing stop, a rule-to-rule stop, the GUI test emulation
    /// and HoldStop share this path; only [`Self::stop_rule_execution`] persists a pause.
    ///
    /// The analytics outcome is not recorded here: every spawn path's `ExecGuard` reports an
    /// aborted run when it drops without the run having reported its own outcome, which covers a
    /// live pipeline, a cycle of a recurring loop and a loop killed between cycles (nothing to
    /// count) in one place.
    pub fn abort_rule_executions(&mut self, rule_id: &str) {
        // 1. Abort any active recurring test loop and every live pipeline of the rule.
        self.abort_recurring_test(rule_id);
        let pipeline_aborts = self.executions.abort_rule(rule_id);

        // 2. Log one line per aborted execution, as before.
        for _ in 0..pipeline_aborts {
            self.log("INFO", "RULE", &format!("Stopped execution of rule: {}", rule_id));
        }

        // 3. Release every process/overlay this rule still owns, including detached overlay
        // shows it made: a stop hides exactly the generations that rule started.
        self.release_rule_owned(
            rule_id,
            &format!("because rule '{rule_id}' was stopped"),
            true,
        );

        crate::daemon::actions::release_all_simulated_inputs();
    }

    /// Releases owned resources, killing processes by their captured PID and hiding only the
    /// overlay show that this run still owns.
    pub(super) fn release_owned_resources(
        &mut self,
        resources: Vec<OwnedResource>,
        context: &str,
    ) {
        for resource in resources {
            match resource {
                OwnedResource::Process(process) => {
                    if !crate::daemon::actions::is_process_alive(process.pid) {
                        self.log(
                            "DEBUG",
                            "RULE",
                            &format!(
                                "Owned process '{}' (pid {}) had already exited {}",
                                process.image_path, process.pid, context
                            ),
                        );
                        continue;
                    }
                    self.log(
                        "ACTION",
                        "RULE",
                        &format!(
                            "Force terminating owned process '{}' (pid {}) {}",
                            process.image_path, process.pid, context
                        ),
                    );
                    let log_tx = self.action_tx.clone();
                    let failure = format!(
                        "Failed to terminate owned process '{}' (pid {}) {}",
                        process.image_path, process.pid, context
                    );
                    let pid = process.pid;
                    match tokio::runtime::Handle::try_current() {
                        Ok(handle) => {
                            handle.spawn(async move {
                                if let Err(e) =
                                    crate::daemon::actions::kill_process_tree_by_pid(pid).await
                                {
                                    let _ = log_tx.send(ActionCompletionEvent::SensitiveLog {
                                        level: "WARN".to_string(),
                                        category: "ACTION".to_string(),
                                        message: format!("{failure}: {e}"),
                                        file_message: format!(
                                            "{failure} (details omitted from the file log)"
                                        ),
                                    });
                                }
                            });
                        }
                        Err(_) => {
                            if let Err(e) =
                                crate::daemon::actions::kill_process_tree_by_pid_blocking(pid)
                            {
                                let _ = log_tx.send(ActionCompletionEvent::SensitiveLog {
                                    level: "WARN".to_string(),
                                    category: "ACTION".to_string(),
                                    message: format!("{failure}: {e}"),
                                    file_message: format!(
                                        "{failure} (details omitted from the file log)"
                                    ),
                                });
                            }
                        }
                    }
                }
                OwnedResource::Overlay(overlay) => {
                    if crate::daemon::overlay_manager::hide_overlay_if_generation(
                        &overlay.overlay_id,
                        overlay.generation,
                    ) {
                        self.log(
                            "ACTION",
                            "RULE",
                            &format!(
                                "Hid owned overlay '{}' {}",
                                overlay.overlay_id, context
                            ),
                        );
                    }
                }
            }
        }
    }

    /// Releases the resources of one execution that live only until that run ends.
    pub(super) fn release_execution_until_run_end(&mut self, execution_id: u64, context: &str) {
        let resources = self.executions.take_owned_where(|resource| {
            resource.execution_id() == execution_id
                && resource.lifetime() == ResourceLifetime::UntilRunEnd
        });
        self.release_owned_resources(resources, context);
    }

    /// Releases the resources a rule owns. `include_detached` is true for an explicit stop (which
    /// also hides the detached overlay shows the rule made) and false for a manual run replacing
    /// its previous run, where a detached overlay must stay on screen.
    pub(super) fn release_rule_owned(
        &mut self,
        rule_id: &str,
        context: &str,
        include_detached: bool,
    ) {
        let resources = self.executions.take_owned_where(|resource| {
            resource.rule_id() == rule_id
                && (include_detached || resource.lifetime() != ResourceLifetime::Detach)
        });
        self.release_owned_resources(resources, context);
    }

    /// Forgets owned resources that disappeared on their own (a process exited, an overlay was
    /// hidden or replaced). Updates the rule-activity counters so a stale resource cannot keep a
    /// rule looking alive forever.
    pub fn prune_owned_resources(&mut self) -> usize {
        let dead = self.executions.take_owned_where(|resource| match resource {
            OwnedResource::Process(process) => {
                !crate::daemon::actions::is_process_alive(process.pid)
            }
            OwnedResource::Overlay(overlay) => {
                crate::daemon::overlay_manager::overlay_generation(&overlay.overlay_id)
                    != Some(overlay.generation)
            }
        });
        let count = dead.len();
        for resource in &dead {
            match resource {
                OwnedResource::Process(process) => self.log(
                    "DEBUG",
                    "RULE",
                    &format!(
                        "Forgot owned process '{}' (pid {}): it is no longer running",
                        process.image_path, process.pid
                    ),
                ),
                OwnedResource::Overlay(overlay) => self.log(
                    "DEBUG",
                    "RULE",
                    &format!(
                        "Forgot owned overlay '{}': that show is no longer on screen",
                        overlay.overlay_id
                    ),
                ),
            }
        }
        count
    }

    /// User-facing stop: abort the executions and disable the rule so its triggers stay quiet.
    pub fn stop_rule_execution(&mut self, rule_id: &str) {
        self.abort_rule_executions(rule_id);

        let mut disabled_in_config = false;
        if let Some(rule) = self.config.rules.iter_mut().find(|r| r.id == rule_id) {
            if rule.is_enabled {
                rule.is_enabled = false;
                disabled_in_config = true;
            }
        }
        if disabled_in_config {
            self.persist_rules();
            if let Some(h) = &self.hotkey_handle {
                h.reload(&self.config.rules);
            }
            self.log("INFO", "RULE", &format!("Paused rule '{}' upon user stop", rule_id));
        }
    }

    /// `rule_control` stop: abort what the target is running right now and keep the rule enabled,
    /// so a rule that stops another one does not silently disable it for good.
    pub fn stop_rule_by_call(&mut self, rule_id: &str) -> Result<(), String> {
        self.drain_action_events();
        let Some(rule_name) = self
            .config
            .rules
            .iter()
            .find(|r| r.id == rule_id)
            .map(|r| r.name.clone())
        else {
            return Err(format!("Rule '{rule_id}' does not exist"));
        };

        let was_running = self.is_rule_executing(rule_id);
        self.abort_rule_executions(rule_id);
        if was_running {
            self.log("INFO", "RULE", &format!("Rule '{}' stopped by a rule call.", rule_name));
        } else {
            self.log(
                "INFO",
                "RULE",
                &format!("Stop requested for rule '{}' by a rule call; nothing was running.", rule_name),
            );
        }
        Ok(())
    }

    pub fn drain_action_events(&mut self) {
        while let Ok(event) = self.action_rx.try_recv() {
            match event {
                ActionCompletionEvent::Log { level, category, message } => {
                    self.log(&level, &category, &message);
                }
                ActionCompletionEvent::SensitiveLog { level, category, message, file_message } => {
                    self.log_with_file_message(&level, &category, &message, Some(&file_message));
                }
                ActionCompletionEvent::AddOwnedResource(resource) => {
                    self.executions.add_owned_resource(resource);
                }
                ActionCompletionEvent::RuleExecutionStarted { execution_id, rule_id, step_index, total_steps, call_chain } => {
                    self.executions.record_started(execution_id, ActiveExecutionItem {
                        rule_id,
                        step_index,
                        total_steps,
                        started_at: Instant::now(),
                        call_chain,
                    });
                }
                ActionCompletionEvent::RuleExecutionStep { execution_id, step_index, total_steps, .. } => {
                    self.executions.record_step(execution_id, step_index, total_steps);
                }
                ActionCompletionEvent::RuleExecutionFinished { execution_id, rule_id, is_manual } => {
                    self.executions.record_finished(execution_id, &rule_id);

                    // Run-scoped resources die with the run on every ending (finish, abort, error).
                    self.release_execution_until_run_end(execution_id, "because its run ended");

                    if is_manual {
                        self.release_rule_owned(
                            &rule_id,
                            &format!("after manual execution of rule '{rule_id}'"),
                            false,
                        );
                    }
                }
                ActionCompletionEvent::SetRuleCustomIcon { rule_id, icon_path } => {
                    let mut updated_name = None;
                    if let Some(r) = self.config.rules.iter_mut().find(|r| r.id == rule_id) {
                        if r.custom_icon_path.is_none() {
                            r.custom_icon_path = Some(icon_path.clone());
                            updated_name = Some(r.name.clone());
                        }
                    }
                    if let Some(name) = updated_name {
                        self.persist_rules();
                        self.log("INFO", "RULE", &format!("Set custom icon for rule '{}' from process: {}", name, icon_path));
                    }
                }
                ActionCompletionEvent::RunOutcome { outcome } => {
                    let now = chrono::Local::now();
                    match outcome {
                        PipelineOutcome::Completed => self.analytics.record_success(now),
                        PipelineOutcome::Failed => self.analytics.record_failure(now),
                        PipelineOutcome::Aborted => self.analytics.record_aborts(1, now),
                    }
                }
                ActionCompletionEvent::RecurringCycleStarted { rule_id, source } => {
                    // The loop reports every cycle, so a draft rule that is only test-run is
                    // counted too: no config lookup is involved.
                    self.analytics
                        .record_run(source, &rule_id, chrono::Local::now());
                }
                ActionCompletionEvent::IncrementExecutionCount { rule_id } => {
                    let increment = self.record_quota_increment(&rule_id, chrono::Local::now());
                    if increment.paused {
                        self.persist_rules();
                        self.stop_rule_execution(&rule_id);
                        self.refresh_live_rules();
                    }
                    if let Some(message) = increment.decision.message() {
                        let level = if increment.paused { "WARN" } else { "INFO" };
                        self.log(level, "RULE", message);
                    }
                }
            }
        }
        self.persist_runtime_if_due();
    }
}
