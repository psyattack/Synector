use std::collections::{HashMap, HashSet};
use std::time::Instant;

use tokio::sync::oneshot;

use crate::daemon::actions::ActionRunner;
use crate::models::{Action, Rule, RunSource};

use super::DaemonEngine;

/// Monotonic id shared by every kind of pipeline execution (triggered, exit, recurring and
/// rule-called). Active executions and abort handles are keyed by it, so it must never collide.
static EXECUTION_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub(super) fn next_execution_id() -> u64 {
    EXECUTION_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[derive(Debug, Clone)]
pub struct BoundProcess {
    pub rule_id: String,
    pub primary_process_name: String,
    pub primary_pid: u32,
    pub target_executable: String,
}

#[derive(Debug, Clone)]
pub struct BoundOverlay {
    pub rule_id: String,
    pub primary_process_name: String,
    pub primary_pid: u32,
    pub overlay_id: String,
}

#[derive(Debug, Clone)]
pub struct ActiveExecutionItem {
    pub rule_id: String,
    pub step_index: usize,
    pub total_steps: usize,
    pub is_exit: bool,
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
/// rule's bound satellites when they finish.
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
    /// True for runs the user started directly. Only those force-terminate the bound satellites
    /// the run created, and the cooldown never gates them.
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
    AddBoundProcess(BoundProcess),
    AddBoundOverlay(BoundOverlay),
    RuleExecutionStarted {
        execution_id: u64,
        rule_id: String,
        step_index: usize,
        total_steps: usize,
        is_exit: bool,
        call_chain: Vec<String>,
    },
    RuleExecutionStep {
        execution_id: u64,
        _rule_id: String,
        step_index: usize,
        total_steps: usize,
        is_exit: bool,
    },
    RuleExecutionFinished {
        execution_id: u64,
        _rule_id: String,
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
    /// One finished pipeline and how it ended. Exit pipelines are never reported.
    RunOutcome {
        outcome: PipelineOutcome,
    },
}

/// Runtime execution tracker: the pipelines that are running, their abort handles, the recurring
/// test loops and the overlays bound to a host process. The engine mutates it only through these
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
    /// Overlays bound to a host process, hidden when that process exits.
    bound_overlays: Vec<BoundOverlay>,
    /// Companion ("bound satellite") processes started by a rule, force-terminated when the rule
    /// is stopped or its lap finishes.
    bound_processes: Vec<BoundProcess>,
    /// Overlay ids a rule started, so a stop can hide overlays of a rule that is not (or no longer)
    /// in the config.
    rule_overlays: HashMap<String, Vec<String>>,
}

impl ExecutionTracker {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// True when a tracked execution kind is running for `rule_id`. The bound-satellite and
    /// active-overlay scans stay on the engine façade, which combines them with this answer.
    pub(super) fn is_rule_executing(&self, rule_id: &str) -> bool {
        self.active_executions
            .values()
            .any(|item| item.rule_id == rule_id)
            || self
                .abort_handles
                .values()
                .any(|(owning_rule, _)| owning_rule == rule_id)
            || self.recurring_tests.contains_key(rule_id)
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

    pub(super) fn record_step(&mut self, execution_id: u64, step_index: usize, total_steps: usize, is_exit: bool) {
        if let Some(item) = self.active_executions.get_mut(&execution_id) {
            item.step_index = step_index;
            item.total_steps = total_steps;
            item.is_exit = is_exit;
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

    pub(super) fn add_bound_overlay(&mut self, overlay: BoundOverlay) {
        self.bound_overlays.push(overlay);
    }

    pub(super) fn bound_overlays(&self) -> &[BoundOverlay] {
        &self.bound_overlays
    }

    pub(super) fn remove_rule_bound_overlays(&mut self, rule_id: &str) {
        self.bound_overlays.retain(|overlay| overlay.rule_id != rule_id);
    }

    /// Removes and returns the bound overlays of one rule.
    pub(super) fn take_rule_bound_overlays(&mut self, rule_id: &str) -> Vec<BoundOverlay> {
        self.take_bound_overlays_where(|overlay| overlay.rule_id == rule_id)
    }

    /// Removes and returns the bound overlays matching `predicate`, keeping the rest in order.
    /// Entries sharing the `(rule, overlay)` identity of a taken one are removed as well, exactly
    /// like the pre-split filter + retain pair.
    pub(super) fn take_bound_overlays_where(
        &mut self,
        mut predicate: impl FnMut(&BoundOverlay) -> bool,
    ) -> Vec<BoundOverlay> {
        let mut taken = Vec::new();
        let mut index = 0;
        while index < self.bound_overlays.len() {
            if predicate(&self.bound_overlays[index]) {
                taken.push(self.bound_overlays.remove(index));
            } else {
                index += 1;
            }
        }
        self.bound_overlays.retain(|kept| {
            !taken
                .iter()
                .any(|gone| gone.rule_id == kept.rule_id && gone.overlay_id == kept.overlay_id)
        });
        taken
    }

    pub(super) fn add_bound_process(&mut self, process: BoundProcess) {
        self.bound_processes.push(process);
    }

    pub(super) fn bound_processes(&self) -> &[BoundProcess] {
        &self.bound_processes
    }

    pub(super) fn has_any_bound_processes(&self) -> bool {
        !self.bound_processes.is_empty()
    }

    /// Removes and returns every bound satellite of one rule, keeping the insertion order.
    pub(super) fn take_rule_bound_processes(&mut self, rule_id: &str) -> Vec<BoundProcess> {
        self.take_bound_processes_where(|process| process.rule_id == rule_id)
    }

    /// Removes and returns the bound processes matching `predicate`, keeping the rest in order.
    /// Entries sharing the `(rule, target executable)` identity of a taken one are removed as well,
    /// exactly like the pre-split filter + retain pair in the process-exit path.
    pub(super) fn take_bound_processes_where(
        &mut self,
        mut predicate: impl FnMut(&BoundProcess) -> bool,
    ) -> Vec<BoundProcess> {
        let mut taken = Vec::new();
        let mut index = 0;
        while index < self.bound_processes.len() {
            if predicate(&self.bound_processes[index]) {
                taken.push(self.bound_processes.remove(index));
            } else {
                index += 1;
            }
        }
        self.bound_processes.retain(|kept| {
            !taken.iter().any(|gone| {
                gone.rule_id == kept.rule_id && gone.target_executable == kept.target_executable
            })
        });
        taken
    }

    /// Records the overlay ids `rule_id` started, replacing any previous set.
    pub(super) fn set_rule_overlays(&mut self, rule_id: &str, overlay_ids: Vec<String>) {
        self.rule_overlays.insert(rule_id.to_string(), overlay_ids);
    }

    /// Removes and returns the overlay ids recorded for `rule_id`.
    pub(super) fn take_rule_overlays(&mut self, rule_id: &str) -> Option<Vec<String>> {
        self.rule_overlays.remove(rule_id)
    }
}

impl DaemonEngine {
    /// Whether any rule currently has a bound satellite process alive. Used by the daemon loop to
    /// decide whether a process trigger must keep polling; the registry lives in the tracker.
    pub fn has_active_bound_companions(&self) -> bool {
        self.executions.has_any_bound_processes()
    }

    pub fn is_rule_executing(&self, rule_id: &str) -> bool {
        if self.executions.is_rule_executing(rule_id)
            || self
                .executions
                .bound_processes()
                .iter()
                .any(|bound| bound.rule_id == rule_id)
        {
            return true;
        }

        let active_ovs = crate::daemon::overlay_manager::get_active_overlay_ids();
        if active_ovs.is_empty() {
            return false;
        }
        let Some(rule) = self.config.rules.iter().find(|r| r.id == rule_id) else {
            return false;
        };
        Self::rule_references_active_overlay(rule, &active_ovs)
    }

    /// True when any overlay action of `rule` targets an id in `active_overlay_ids`. The scan is
    /// the full [`Rule::visit_actions`] tree, so `IfElse` branches (and `on_exit_actions`) count.
    pub(super) fn rule_references_active_overlay(
        rule: &Rule,
        active_overlay_ids: &HashSet<String>,
    ) -> bool {
        let mut found = false;
        rule.visit_actions(&mut |action| {
            if found {
                return;
            }
            if let Action::Overlay { overlay_id, custom_overlay, .. } = action {
                let id = custom_overlay
                    .as_ref()
                    .map(|overlay| overlay.id.as_str())
                    .unwrap_or(overlay_id.as_str());
                found = active_overlay_ids.contains(id);
            }
        });
        found
    }

    pub fn extract_rule_overlay_ids(rule: &Rule) -> Vec<String> {
        let mut ids = Vec::new();
        rule.visit_actions(&mut |action| {
            if let Action::Overlay { overlay_id, custom_overlay, .. } = action {
                let id = custom_overlay.as_ref().map(|o| o.id.clone()).unwrap_or_else(|| overlay_id.clone());
                ids.push(id);
            }
        });
        ids
    }

    pub fn hide_rule_overlays(rule: &Rule) {
        for id in Self::extract_rule_overlay_ids(rule) {
            crate::daemon::overlay_manager::hide_overlay(&id);
        }
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
    /// live pipeline, a cycle of a recurring loop, a stopped exit pipeline (not counted at all) and
    /// a loop killed between cycles (nothing to count) in one place.
    pub fn abort_rule_executions(&mut self, rule_id: &str) {
        // 1. Abort any active recurring test loop and every live pipeline of the rule.
        self.abort_recurring_test(rule_id);
        let pipeline_aborts = self.executions.abort_rule(rule_id);

        // 2. Log one line per aborted execution, as before.
        for _ in 0..pipeline_aborts {
            self.log("INFO", "RULE", &format!("Stopped execution of rule: {}", rule_id));
        }

        // 3. Hide any tracked overlays associated with this rule (including draft rules)
        if let Some(ids) = self.executions.take_rule_overlays(rule_id) {
            for id in ids {
                crate::daemon::overlay_manager::hide_overlay(&id);
            }
        }

        // 4. Hide any active overlays associated with this rule from config
        if let Some(rule) = self.config.rules.iter().find(|r| r.id == rule_id) {
            Self::hide_rule_overlays(rule);
        }

        self.executions.remove_rule_bound_overlays(rule_id);

        // 5. Force stop and remove any active bound processes for this rule
        self.terminate_bound_processes(rule_id, &format!("upon stopping rule '{}'", rule_id));

        crate::daemon::actions::release_all_simulated_inputs();
    }

    /// Normalizes a bound satellite target and builds the kill action used by every teardown path.
    pub(super) fn bound_satellite_kill(target_executable: &str) -> (String, Action) {
        let mut target_name = std::path::Path::new(target_executable)
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or(target_executable)
            .to_string();
        if !target_name.to_lowercase().ends_with(".exe") && !target_name.contains('.') {
            target_name.push_str(".exe");
        }
        let action = Action::KillProcess {
            process_name: target_name.clone(),
            kill_tree: true,
            force: true,
        };
        (target_name, action)
    }

    /// Removes and force-terminates every bound satellite of `rule_id` without touching the
    /// rule's enabled state. `context` completes "Force terminating bound satellite process 'x'".
    fn terminate_bound_processes(&mut self, rule_id: &str, context: &str) {
        let to_kill = self.executions.take_rule_bound_processes(rule_id);

        for bp in to_kill {
            let (target_name, action) = Self::bound_satellite_kill(&bp.target_executable);
            let failure = format!(
                "Failed to force terminate bound satellite process '{}' {}",
                target_name, context
            );
            self.log(
                "ACTION",
                "RULE",
                &format!("Force terminating bound satellite process '{}' {}", target_name, context),
            );

            let ctx = HashMap::new();
            let log_tx = self.action_tx.clone();
            tokio::spawn(async move {
                if let Err(e) = ActionRunner::execute(&action, &ctx).await {
                    let _ = log_tx.send(ActionCompletionEvent::SensitiveLog {
                        level: "WARN".to_string(),
                        category: "ACTION".to_string(),
                        message: format!("{failure}: {e}"),
                        file_message: format!("{failure} (details omitted from the file log)"),
                    });
                }
            });
        }
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
            Self::hide_rule_overlays(rule);
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
                ActionCompletionEvent::AddBoundProcess(bp) => {
                    self.executions.add_bound_process(bp);
                }
                ActionCompletionEvent::AddBoundOverlay(bo) => {
                    self.executions.add_bound_overlay(bo);
                }
                ActionCompletionEvent::RuleExecutionStarted { execution_id, rule_id, step_index, total_steps, is_exit, call_chain } => {
                    self.executions.record_started(execution_id, ActiveExecutionItem {
                        rule_id,
                        step_index,
                        total_steps,
                        is_exit,
                        started_at: Instant::now(),
                        call_chain,
                    });
                }
                ActionCompletionEvent::RuleExecutionStep { execution_id, step_index, total_steps, is_exit, .. } => {
                    self.executions.record_step(execution_id, step_index, total_steps, is_exit);
                }
                ActionCompletionEvent::RuleExecutionFinished { execution_id, _rule_id, is_manual } => {
                    self.executions.record_finished(execution_id, &_rule_id);

                    if is_manual {
                        self.terminate_bound_processes(
                            &_rule_id,
                            &format!("after manual execution of rule '{}'", _rule_id),
                        );

                        for bo in self.executions.take_rule_bound_overlays(&_rule_id) {
                            crate::daemon::overlay_manager::hide_overlay(&bo.overlay_id);
                        }
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
