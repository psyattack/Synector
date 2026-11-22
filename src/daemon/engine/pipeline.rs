use std::collections::HashMap;
use std::time::Instant;

use tokio::sync::{mpsc, oneshot};

use crate::daemon::actions::ActionRunner;
use crate::daemon::rule_call;
use crate::daemon::watcher::ProcessWatcher;
use crate::models::{Action, ProcessMatchMode, Rule, ScheduleType, Trigger};

use super::executions::{next_execution_id, ActionCompletionEvent, BoundOverlay, BoundProcess, ExecutionOrigin, RuleCallRuntime, RuleCallStart, RuleSkip};
use super::DaemonEngine;

/// Drop guard shared by every pipeline spawn path. It reports the execution as finished even when
/// the task is aborted, releases simulated inputs and resolves a `wait: true` rule call.
struct ExecGuard {
    id: u64,
    rule_id: String,
    is_manual: bool,
    tx: mpsc::UnboundedSender<ActionCompletionEvent>,
    /// Resolves a `wait: true` rule call when the pipeline ends or is aborted.
    completion: Option<oneshot::Sender<()>>,
}

impl Drop for ExecGuard {
    fn drop(&mut self) {
        crate::daemon::actions::release_all_simulated_inputs();
        crate::daemon::rule_activity::mark_finished(&self.rule_id);
        let _ = self.tx.send(ActionCompletionEvent::RuleExecutionFinished {
            execution_id: self.id,
            _rule_id: self.rule_id.clone(),
            is_manual: self.is_manual,
        });
        if let Some(completion) = self.completion.take() {
            let _ = completion.send(());
        }
    }
}

/// What one pipeline run needs from its caller. Triggered, exited and recurring runs share one
/// step loop; the flags below capture the few places where they legitimately differ.
pub(super) struct PipelineRun<'a> {
    pub(super) actions: &'a [Action],
    pub(super) execution_id: u64,
    pub(super) rule_id: &'a str,
    pub(super) rule_name: &'a str,
    /// Trigger context of the primary run, recorded on the companions it binds to a process.
    pub(super) context_name: &'a str,
    pub(super) pid: u32,
    pub(super) is_exit: bool,
    pub(super) stop_on_error: bool,
    /// Only the primary pipeline binds `close_on_exit` companions; exit and recurring runs do not.
    pub(super) bind_companions: bool,
    /// "Pipeline" or "Exit pipeline": the subject of the graceful-halt log line.
    pub(super) label: &'a str,
    /// "Action" or "Exit action": the subject of the failure log line.
    pub(super) error_label: &'a str,
    /// The recurring loop stops when a cycle halts; the other callers ignore the answer.
    pub(super) warn_on_error_halt: bool,
}

/// Owned input of one spawned action pipeline. [`PipelineRun`] borrows these fields, so the task
/// owns them for the duration of the run.
struct OwnedPipelineRun {
    actions: Vec<Action>,
    rule_id: String,
    rule_name: String,
    context_name: String,
    pid: u32,
    is_exit: bool,
    stop_on_error: bool,
    bind_companions: bool,
    label: &'static str,
    error_label: &'static str,
    warn_on_error_halt: bool,
}

/// Progress sink for [`run_step_loop`]. The engine turns these callbacks into
/// `ActionCompletionEvent`s; the offline `--run-rule` runner records them for the console.
pub(crate) trait PipelineObserver {
    /// Called before the action at `step_index` runs.
    fn step_started(&mut self, step_index: usize, total_steps: usize);
    /// The action at `step_index` completed with optional pipeline output.
    fn step_completed(&mut self, step_index: usize, action: &Action);
    /// The action at `step_index` failed; `stopping` tells whether `stop_on_error` ends the run.
    fn step_failed(&mut self, step_index: usize, error: &str, stopping: bool);
    /// The run halted early: `abort_reason` is set for a graceful `AbortPipeline`.
    fn pipeline_halted(&mut self, step_index: usize, abort_reason: Option<&str>);
}

/// Runs the shared action step loop used by every pipeline path: pipes `PrevOutput` and
/// `StepN.Output`, stops on an `AbortPipeline` outcome, and honours `stop_on_error`.
/// Returns `true` when the pipeline halted early.
pub(crate) async fn run_step_loop<O: PipelineObserver>(
    actions: &[Action],
    ctx: &mut HashMap<String, String>,
    stop_on_error: bool,
    observer: &mut O,
) -> bool {
    use crate::daemon::actions::ActionOutcome;

    let total_steps = actions.len();
    for (step_idx, action) in actions.iter().enumerate() {
        observer.step_started(step_idx, total_steps);

        match ActionRunner::execute_with_output(action, ctx).await {
            Ok(ActionOutcome::Continue(out_opt)) => {
                if let Some(out) = out_opt {
                    ctx.insert("PrevOutput".to_string(), out.clone());
                    ctx.insert(format!("Step{}.Output", step_idx + 1), out);
                }
                observer.step_completed(step_idx, action);
            }
            Ok(ActionOutcome::AbortPipeline { reason }) => {
                observer.pipeline_halted(step_idx, Some(&reason));
                return true;
            }
            Err(err) => {
                observer.step_failed(step_idx, &err, stop_on_error);
                if stop_on_error {
                    observer.pipeline_halted(step_idx, None);
                    return true;
                }
            }
        }
    }
    false
}

/// Turns the shared step-loop callbacks into the engine's `ActionCompletionEvent`s.
struct EnginePipelineObserver<'a> {
    tx: &'a mpsc::UnboundedSender<ActionCompletionEvent>,
    execution_id: u64,
    rule_id: &'a str,
    rule_name: &'a str,
    context_name: &'a str,
    pid: u32,
    is_exit: bool,
    bind_companions: bool,
    label: &'a str,
    error_label: &'a str,
    warn_on_error_halt: bool,
}

impl PipelineObserver for EnginePipelineObserver<'_> {
    fn step_started(&mut self, step_index: usize, total_steps: usize) {
        let _ = self.tx.send(ActionCompletionEvent::RuleExecutionStep {
            execution_id: self.execution_id,
            _rule_id: self.rule_id.to_string(),
            step_index,
            total_steps,
            is_exit: self.is_exit,
        });
    }

    fn step_completed(&mut self, _step_index: usize, action: &Action) {
        if !self.bind_companions {
            return;
        }
        match action {
            Action::StartProcess { file_path, close_on_exit: true, .. } => {
                let _ = self.tx.send(ActionCompletionEvent::AddBoundProcess(BoundProcess {
                    rule_id: self.rule_id.to_string(),
                    primary_process_name: self.context_name.to_string(),
                    primary_pid: self.pid,
                    target_executable: file_path.clone(),
                }));
            }
            Action::Overlay { overlay_id, custom_overlay, close_on_exit: true, .. } => {
                let resolved_id = custom_overlay
                    .as_ref()
                    .map(|o| o.id.clone())
                    .unwrap_or_else(|| overlay_id.clone());
                let _ = self.tx.send(ActionCompletionEvent::AddBoundOverlay(BoundOverlay {
                    rule_id: self.rule_id.to_string(),
                    primary_process_name: self.context_name.to_string(),
                    primary_pid: self.pid,
                    overlay_id: resolved_id,
                }));
            }
            _ => {}
        }
    }

    fn step_failed(&mut self, step_index: usize, error: &str, _stopping: bool) {
        let _ = self.tx.send(ActionCompletionEvent::SensitiveLog {
            level: "ERROR".to_string(),
            category: "ACTION".to_string(),
            message: format!("{} failed in '{}': {}", self.error_label, self.rule_name, error),
            file_message: format!(
                "{} failed in '{}' at step {} (details omitted from the file log)",
                self.error_label,
                self.rule_name,
                step_index + 1
            ),
        });
    }

    fn pipeline_halted(&mut self, step_index: usize, abort_reason: Option<&str>) {
        match abort_reason {
            Some(reason) => {
                let _ = self.tx.send(ActionCompletionEvent::Log {
                    level: "INFO".to_string(),
                    category: "RULE".to_string(),
                    message: format!(
                        "{} gracefully halted for rule '{}' at step {}: {}",
                        self.label,
                        self.rule_name,
                        step_index + 1,
                        reason
                    ),
                });
            }
            None => {
                if self.warn_on_error_halt {
                    let _ = self.tx.send(ActionCompletionEvent::Log {
                        level: "WARN".to_string(),
                        category: "RULE".to_string(),
                        message: format!(
                            "Pipeline stopped for rule '{}' at step {} due to error",
                            self.rule_name,
                            step_index + 1
                        ),
                    });
                }
            }
        }
    }
}

/// Runs one action pipeline. Returns `true` when it halted early (a graceful abort, or an error
/// with `stop_on_error`), which is what stops a recurring loop.
pub(super) async fn run_action_pipeline(
    run: PipelineRun<'_>,
    ctx: &mut HashMap<String, String>,
    tx: &mpsc::UnboundedSender<ActionCompletionEvent>,
) -> bool {
    let mut observer = EnginePipelineObserver {
        tx,
        execution_id: run.execution_id,
        rule_id: run.rule_id,
        rule_name: run.rule_name,
        context_name: run.context_name,
        pid: run.pid,
        is_exit: run.is_exit,
        bind_companions: run.bind_companions,
        label: run.label,
        error_label: run.error_label,
        warn_on_error_halt: run.warn_on_error_halt,
    };
    run_step_loop(run.actions, ctx, run.stop_on_error, &mut observer).await
}

impl DaemonEngine {
    /// The one rule-trigger entry point: the caller states the [`ExecutionOrigin`] explicitly
    /// instead of the engine inferring it from a context label or a process id.
    pub async fn trigger(
        &mut self,
        rule: &Rule,
        origin: ExecutionOrigin,
        context_name: &str,
        pid: u32,
        extra_ctx: HashMap<String, String>,
    ) -> Option<tokio::task::JoinHandle<()>> {
        match self.execute_rule_pipeline(rule, origin, context_name, pid, extra_ctx, None).await {
            Ok(handle) => handle,
            Err(RuleSkip::Conditions) => {
                self.log(
                    "WARN",
                    "RULE",
                    &format!("Rule '{}' triggered by {}, but conditions were not met. Skipping.", rule.name, context_name),
                );
                None
            }
            Err(_) => None,
        }
    }

    /// Seconds since the last accepted run of `rule`: the in-memory trigger stamp when it exists,
    /// otherwise the persisted `last_executed_at`. Without the fallback every daemon restart would
    /// clear all cooldowns.
    fn last_run_elapsed(&self, rule: &Rule) -> Option<f64> {
        if let Some(last) = self.rule_last_triggered.get(&rule.id) {
            return Some(last.elapsed().as_secs_f64());
        }
        let last = chrono::NaiveDateTime::parse_from_str(
            rule.last_executed_at.as_deref()?,
            "%Y-%m-%d %H:%M:%S",
        )
        .ok()?;
        let elapsed_ms = chrono::Local::now()
            .naive_local()
            .signed_duration_since(last)
            .num_milliseconds();
        (elapsed_ms >= 0).then(|| elapsed_ms as f64 / 1000.0)
    }

    /// Inserts the `WindowTitle` token from the foreground window; shared by [`Self::pipeline_context`]
    /// and the recurring loop so the probe has one copy.
    fn insert_window_title(ctx: &mut HashMap<String, String>) {
        #[cfg(windows)]
        {
            use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW};
            let hwnd = unsafe { GetForegroundWindow() };
            if !hwnd.0.is_null() {
                let mut buf = [0u16; 512];
                let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
                if len > 0 {
                    let title = String::from_utf16_lossy(&buf[..len as usize]);
                    ctx.insert("WindowTitle".into(), title);
                }
            }
        }
    }

    /// Standard pipeline context tokens shared by the daemon and the offline `--run-rule`
    /// runner: `ProcessName`, `ProcessId`, `Timestamp`, `ProcessPath`/`ProcessDir` when a live
    /// pid is known, and the current `WindowTitle`.
    pub(super) fn pipeline_context(
        context_name: &str,
        pid: u32,
        extra_ctx: HashMap<String, String>,
    ) -> HashMap<String, String> {
        let mut ctx = HashMap::new();
        ctx.insert("ProcessName".into(), context_name.into());
        ctx.insert("ProcessId".into(), pid.to_string());
        ctx.insert("Timestamp".into(), chrono::Local::now().to_rfc3339());

        if pid > 0 {
            if let Some(path) = ProcessWatcher::get_process_image_path(pid) {
                ctx.insert("ProcessPath".into(), path.clone());
                if let Some(dir) = std::path::Path::new(&path).parent().and_then(|p| p.to_str()) {
                    ctx.insert("ProcessDir".into(), dir.to_string());
                }
            }
        }

        Self::insert_window_title(&mut ctx);

        ctx.extend(extra_ctx);
        ctx
    }

    /// Shared prologue of every pipeline spawn path: reserves the process-wide execution id, marks
    /// the rule active and announces the run. Returns the id and the action-event sender the caller
    /// spawns with.
    fn begin_pipeline(
        &self,
        rule_id: &str,
        total_steps: usize,
        is_exit: bool,
        call_chain: Vec<String>,
    ) -> (u64, mpsc::UnboundedSender<ActionCompletionEvent>) {
        let tx = self.action_tx.clone();
        let execution_id = next_execution_id();
        crate::daemon::rule_activity::mark_started(rule_id);
        let _ = tx.send(ActionCompletionEvent::RuleExecutionStarted {
            execution_id,
            rule_id: rule_id.to_string(),
            step_index: 0,
            total_steps,
            is_exit,
            call_chain,
        });
        (execution_id, tx)
    }

    /// Builds the drop guard for `execution_id`. Callers construct it only once a runtime is known
    /// to exist, and before spawning, so an abort before the first poll still reports the run.
    fn exec_guard(
        execution_id: u64,
        rule_id: &str,
        is_manual: bool,
        tx: &mpsc::UnboundedSender<ActionCompletionEvent>,
        completion: Option<oneshot::Sender<()>>,
    ) -> ExecGuard {
        ExecGuard {
            id: execution_id,
            rule_id: rule_id.to_string(),
            is_manual,
            tx: tx.clone(),
            completion,
        }
    }

    /// Spawns one one-shot pipeline body on `handle` and registers its abort handle.
    fn spawn_pipeline(
        &mut self,
        handle: tokio::runtime::Handle,
        execution_id: u64,
        tx: mpsc::UnboundedSender<ActionCompletionEvent>,
        guard: ExecGuard,
        body: OwnedPipelineRun,
        ctx: HashMap<String, String>,
    ) -> tokio::task::JoinHandle<()> {
        let execution_rule_id = body.rule_id.clone();
        let join_handle = handle.spawn(async move {
            let _guard = guard;
            let mut ctx = ctx;
            let run = PipelineRun {
                actions: &body.actions,
                execution_id,
                rule_id: &body.rule_id,
                rule_name: &body.rule_name,
                context_name: &body.context_name,
                pid: body.pid,
                is_exit: body.is_exit,
                stop_on_error: body.stop_on_error,
                bind_companions: body.bind_companions,
                label: body.label,
                error_label: body.error_label,
                warn_on_error_halt: body.warn_on_error_halt,
            };
            run_action_pipeline(run, &mut ctx, &tx).await;
        });
        self.executions.register_abort_handle(execution_id, execution_rule_id, join_handle.abort_handle());
        join_handle
    }

    /// No Tokio runtime: nothing can be spawned, so complete the announced run synchronously and
    /// report it as finished exactly as the no-runtime fallback always did.
    fn finish_pipeline_without_runtime(
        tx: &mpsc::UnboundedSender<ActionCompletionEvent>,
        execution_id: u64,
        rule_id: String,
        is_manual: bool,
    ) {
        crate::daemon::rule_activity::mark_finished(&rule_id);
        let _ = tx.send(ActionCompletionEvent::RuleExecutionFinished {
            execution_id,
            _rule_id: rule_id,
            is_manual,
        });
    }

    /// Starts the pipeline of `rule` and returns the abort handle. `call` is set when another
    /// rule invoked this one: it carries the call chain and the completion notifier.
    ///
    /// `Err` means the rule was intentionally not started (quota, cooldown, conditions or an empty
    /// pipeline); `Ok(None)` means it could not be spawned because there is no Tokio runtime.
    async fn execute_rule_pipeline(
        &mut self,
        rule: &Rule,
        origin: ExecutionOrigin,
        context_name: &str,
        pid: u32,
        extra_ctx: HashMap<String, String>,
        call: Option<RuleCallRuntime>,
    ) -> Result<Option<tokio::task::JoinHandle<()>>, RuleSkip> {
        self.drain_action_events();

        // Only user-invoked runs force-terminate the satellites they spawn and skip the cooldown;
        // a webhook or schedule with pid 0 is not a manual run.
        let is_manual = call.is_none() && origin.is_user_invoked();
        if is_manual {
            // A user run replaces its rule's satellites without killing the stale ones.
            let _ = self.executions.take_rule_bound_processes(&rule.id);
            self.executions.remove_rule_bound_overlays(&rule.id);
        }

        if !self.quota_allows_execution(&rule.id, false) {
            return Err(RuleSkip::Quota);
        }

        if !origin.is_user_invoked() && rule.cooldown_seconds > 0.0 {
            if let Some(elapsed) = self.last_run_elapsed(rule) {
                if elapsed < rule.cooldown_seconds {
                    return Err(RuleSkip::Cooldown { elapsed, cooldown: rule.cooldown_seconds });
                }
            }
        }

        if !rule.conditions.is_empty() && !Self::evaluate_rule_conditions(&rule.conditions) {
            return Err(RuleSkip::Conditions);
        }

        self.rule_last_triggered.insert(rule.id.clone(), Instant::now());
        self.log("TRIGGER", "RULE", &format!("Rule '{}' triggered by {}", rule.name, context_name));

        if rule.auto_process_icon && rule.custom_icon_path.is_none() {
            let exe_path = if pid > 0 {
                ProcessWatcher::get_process_image_path(pid)
            } else {
                ProcessWatcher::find_exe_path_by_name(context_name)
            };
            if let Some(path) = exe_path {
                let _ = self.action_tx.send(ActionCompletionEvent::SetRuleCustomIcon {
                    rule_id: rule.id.clone(),
                    icon_path: path,
                });
            }
        }

        if rule.actions.is_empty() {
            return Err(RuleSkip::EmptyPipeline);
        }

        let ov_ids = Self::extract_rule_overlay_ids(rule);
        if !ov_ids.is_empty() {
            self.executions.set_rule_overlays(&rule.id, ov_ids);
        }

        let mut ctx = Self::pipeline_context(context_name, pid, extra_ctx);

        let actions = rule.actions.clone();
        let rule_name = rule.name.clone();
        let rule_id = rule.id.clone();
        let stop_on_error = rule.stop_on_error;
        let context_name = context_name.to_string();

        let call_chain = call.as_ref().map(|c| c.chain.clone()).unwrap_or_default();
        let completion = call.map(|c| c.completion);

        let increment = self.record_quota_increment(&rule.id, chrono::Local::now());
        if increment.paused {
            self.persist_rules();
        }
        if let Some(message) = increment.decision.message() {
            let level = if increment.paused { "WARN" } else { "INFO" };
            self.log(level, "RULE", message);
        }

        self.analytics.total_executions += 1;

        let (execution_id, tx) = self.begin_pipeline(&rule_id, actions.len(), false, call_chain);
        ctx.insert(rule_call::EXECUTION_CTX_KEY.to_string(), execution_id.to_string());

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            // Built before the task is spawned and moved into it, so an abort before the first
            // poll drops the future together with the guard: the caller is always notified.
            let guard = Self::exec_guard(execution_id, &rule_id, is_manual, &tx, completion);
            let body = OwnedPipelineRun {
                actions,
                rule_id,
                rule_name,
                context_name,
                pid,
                is_exit: false,
                stop_on_error,
                bind_companions: true,
                label: "Pipeline",
                error_label: "Action",
                warn_on_error_halt: true,
            };
            Ok(Some(self.spawn_pipeline(handle, execution_id, tx, guard, body, ctx)))
        } else {
            Self::finish_pipeline_without_runtime(&tx, execution_id, rule_id, is_manual);
            Ok(None)
        }
    }

    /// Starts `rule_id` on behalf of another rule. The returned [`RuleCallStart`] either carries
    /// the completion receiver of the new pipeline or reports why it was skipped.
    ///
    /// The call chain comes from the caller's active execution, so `A → B → A` is rejected before a
    /// pipeline can recurse forever.
    pub async fn start_called_rule(
        &mut self,
        rule_id: &str,
        caller_execution: u64,
    ) -> Result<RuleCallStart, String> {
        if rule_id.trim().is_empty() {
            return Err("the rule_control action has an empty rule_id".to_string());
        }

        self.drain_action_events();

        let Some(rule) = self.config.rules.iter().find(|r| r.id == rule_id).cloned() else {
            return Err(format!("Rule '{rule_id}' does not exist"));
        };
        if !rule.is_enabled {
            self.log(
                "INFO",
                "RULE",
                &format!("Call to rule '{}' skipped: the rule is disabled.", rule.name),
            );
            return Ok(RuleCallStart::Skipped);
        }

        let mut chain = self.executions.call_chain(caller_execution);

        if self.executions.is_running_rule(caller_execution, rule_id) {
            return Err(format!(
                "Rule call cycle rejected: rule '{rule_id}' calls itself"
            ));
        }
        if chain.iter().any(|id| id == rule_id) {
            return Err(format!(
                "Rule call cycle rejected: {} -> {rule_id}",
                chain.join(" -> ")
            ));
        }
        if chain.len() >= rule_call::MAX_RULE_CALL_DEPTH {
            return Err(format!(
                "Rule call depth limit ({}) reached before calling '{rule_id}'",
                rule_call::MAX_RULE_CALL_DEPTH
            ));
        }

        let caller = self
            .executions
            .running_rule_id(caller_execution)
            .map(str::to_string)
            .or_else(|| chain.last().cloned())
            .and_then(|id| {
                self.config
                    .rules
                    .iter()
                    .find(|rule| rule.id == id)
                    .map(|rule| (rule.id.clone(), rule.name.clone()))
            });
        let (context_name, extra_ctx) = match caller {
            Some((caller_id, caller_name)) => {
                let mut extra = HashMap::new();
                extra.insert("CallerRule".to_string(), caller_name.clone());
                extra.insert("CallerRuleId".to_string(), caller_id);
                (format!("Rule Call (from '{caller_name}')"), extra)
            }
            None => ("Rule Call".to_string(), HashMap::new()),
        };

        let (completion, completion_rx) = oneshot::channel();
        chain.push(rule_id.to_string());
        let result = self
            .execute_rule_pipeline(
                &rule,
                ExecutionOrigin::RuleInvocation,
                &context_name,
                0,
                extra_ctx,
                Some(RuleCallRuntime { chain, completion }),
            )
            .await;

        match result {
            Ok(Some(_)) => Ok(RuleCallStart::Started(completion_rx)),
            Ok(None) => Ok(RuleCallStart::Skipped),
            Err(skip) => {
                let reason = skip.describe();
                self.log(
                    "INFO",
                    "RULE",
                    &format!("Call to rule '{}' skipped: {reason}.", rule.name),
                );
                Ok(RuleCallStart::Skipped)
            }
        }
    }

    pub async fn try_execute_exit_actions(
        &mut self,
        rule: &Rule,
        context_name: &str,
        pid: u32,
    ) -> Option<tokio::task::JoinHandle<()>> {
        self.drain_action_events();
        self.log("TRIGGER", "RULE", &format!("Rule '{}' exit actions triggered by {}", rule.name, context_name));

        if rule.on_exit_actions.is_empty() {
            return None;
        }

        let mut ctx = HashMap::new();
        ctx.insert("ProcessName".into(), context_name.into());
        ctx.insert("ProcessId".into(), pid.to_string());
        ctx.insert("Timestamp".into(), chrono::Local::now().to_rfc3339());

        if pid > 0 {
            if let Some(path) = ProcessWatcher::get_process_image_path(pid) {
                ctx.insert("ProcessPath".into(), path.clone());
                if let Some(dir) = std::path::Path::new(&path).parent().and_then(|p| p.to_str()) {
                    ctx.insert("ProcessDir".into(), dir.to_string());
                }
            }
        }

        let actions = rule.on_exit_actions.clone();
        let rule_name = rule.name.clone();
        let rule_id = rule.id.clone();
        let stop_on_error = rule.stop_on_error;

        let (execution_id, tx) = self.begin_pipeline(&rule_id, actions.len(), true, Vec::new());
        ctx.insert(rule_call::EXECUTION_CTX_KEY.to_string(), execution_id.to_string());

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let guard = Self::exec_guard(execution_id, &rule_id, false, &tx, None);
            let body = OwnedPipelineRun {
                actions,
                rule_id,
                rule_name,
                context_name: String::new(),
                pid: 0,
                is_exit: true,
                stop_on_error,
                bind_companions: false,
                label: "Exit pipeline",
                error_label: "Exit action",
                warn_on_error_halt: false,
            };
            Some(self.spawn_pipeline(handle, execution_id, tx, guard, body, ctx))
        } else {
            Self::finish_pipeline_without_runtime(&tx, execution_id, rule_id, false);
            None
        }
    }

    pub async fn emulate_rule_trigger(
        &mut self,
        rule: &Rule,
    ) -> Option<tokio::task::JoinHandle<()>> {
        self.drain_action_events();

        if self.is_rule_executing(&rule.id) {
            // Emulation replaces whatever is running; it must never persist a pause.
            self.abort_rule_executions(&rule.id);
        }

        let _ = self.executions.take_rule_bound_processes(&rule.id);
        self.executions.remove_rule_bound_overlays(&rule.id);

        let ov_ids = Self::extract_rule_overlay_ids(rule);
        if !ov_ids.is_empty() {
            self.executions.set_rule_overlays(&rule.id, ov_ids);
        }

        if !self.quota_allows_execution(&rule.id, true) {
            return None;
        }

        if !rule.conditions.is_empty() && !Self::evaluate_rule_conditions(&rule.conditions) {
            self.log(
                "WARN",
                "RULE",
                &format!("Rule '{}' test/run skipped: conditions were not met.", rule.name),
            );
            return None;
        }

        if rule.actions.is_empty() {
            return None;
        }

        // Clear cooldown so manual/test run triggers immediately
        self.rule_last_triggered.remove(&rule.id);

        // The emulation is a user-invoked test run regardless of which trigger it represents.
        match &rule.trigger {
            Some(Trigger::Schedule { schedule_type: ScheduleType::Interval, interval_sec, .. }) if *interval_sec > 0 => {
                let interval_duration = tokio::time::Duration::from_secs(*interval_sec);
                let context_name = format!("Schedule Interval {}s", interval_sec);
                self.spawn_recurring_execution(rule, interval_duration, &context_name)
            }
            Some(Trigger::GlobalHotkey { hold_mode: true, repeat_while_held: true, .. }) => {
                let interval_duration = tokio::time::Duration::from_millis(100);
                self.spawn_recurring_execution(rule, interval_duration, "Global Hotkey (Loop)")
            }
            Some(Trigger::Process { process_name, match_mode, .. }) => {
                let pid = ProcessWatcher::snapshot_process_map()
                    .into_iter()
                    .find(|(_, name)| Self::matches_process(process_name, name, *match_mode))
                    .map(|(p, _)| p)
                    .unwrap_or(0);
                let mut extra = HashMap::new();
                extra.insert("ProcessName".to_string(), process_name.clone());
                self.test_run_with_context(rule, "Manual Run", pid, extra).await
            }
            Some(Trigger::WindowFocus { process_name, .. }) => {
                let pid = ProcessWatcher::snapshot_process_map()
                    .into_iter()
                    .find(|(_, name)| Self::matches_process(process_name, name, ProcessMatchMode::Exact))
                    .map(|(p, _)| p)
                    .unwrap_or(0);
                let mut extra = HashMap::new();
                extra.insert("ProcessName".to_string(), process_name.clone());
                self.test_run_with_context(rule, "Manual Run", pid, extra).await
            }
            Some(Trigger::ResourceThreshold { process_name, .. }) => {
                let pid = ProcessWatcher::snapshot_process_map()
                    .into_iter()
                    .find(|(_, name)| Self::matches_process(process_name, name, ProcessMatchMode::Exact))
                    .map(|(p, _)| p)
                    .unwrap_or(0);
                let mut extra = HashMap::new();
                extra.insert("ProcessName".to_string(), process_name.clone());
                self.test_run_with_context(rule, "Manual Run", pid, extra).await
            }
            Some(Trigger::SystemEvent { event_type, .. }) => {
                let ctx_name = format!("System Event ({:?})", event_type);
                self.test_run_with_context(rule, &ctx_name, 0, HashMap::new()).await
            }
            Some(Trigger::FileWatcher { folder_path, .. }) => {
                let ctx_name = format!("File Watcher ({})", folder_path);
                self.test_run_with_context(rule, &ctx_name, 0, HashMap::new()).await
            }
            Some(Trigger::Webhook { endpoint_path }) => {
                let ctx_name = format!("Webhook ({})", endpoint_path);
                self.test_run_with_context(rule, &ctx_name, 0, HashMap::new()).await
            }
            Some(Trigger::Schedule { schedule_type, .. }) => {
                let ctx_name = format!("{:?} Schedule", schedule_type);
                self.test_run_with_context(rule, &ctx_name, 0, HashMap::new()).await
            }
            _ => {
                self.test_run_with_context(rule, "Manual Run", 0, HashMap::new()).await
            }
        }
    }

    /// Runs `rule` as an explicit GUI test / run: the origin stays [`ExecutionOrigin::Test`] even
    /// when the emulated context is a webhook, schedule or system event.
    async fn test_run_with_context(
        &mut self,
        rule: &Rule,
        context_name: &str,
        pid: u32,
        extra_ctx: HashMap<String, String>,
    ) -> Option<tokio::task::JoinHandle<()>> {
        self.trigger(
            rule,
            ExecutionOrigin::Test,
            context_name,
            pid,
            extra_ctx,
        )
        .await
    }

    fn spawn_recurring_execution(
        &mut self,
        rule: &Rule,
        interval_duration: tokio::time::Duration,
        context_name: &str,
    ) -> Option<tokio::task::JoinHandle<()>> {
        self.rule_last_triggered.insert(rule.id.clone(), Instant::now());
        self.log("TRIGGER", "RULE", &format!("Rule '{}' recurring execution started ({})", rule.name, context_name));

        let rule_name = rule.name.clone();
        let rule_id = rule.id.clone();
        let context_name = context_name.to_string();
        let live_rules = std::sync::Arc::clone(&self.live_rules);
        let now = chrono::Local::now();
        let mut current_runs = self.config.rules.iter().find(|r| r.id == rule.id)
            .map(|stored| stored.quota_used(now))
            .unwrap_or_else(|| rule.quota_used(now));

        let (execution_id, tx) = self.begin_pipeline(&rule_id, rule.actions.len(), false, Vec::new());

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let task_rule_id = rule_id.clone();
            let guard = Self::exec_guard(execution_id, &rule_id, false, &tx, None);
            let join_handle = handle.spawn(async move {
                let _guard = guard;

                loop {
                    // Re-read the stored rule every cycle: a removed or disabled rule ends the
                    // loop, and an edited one runs its current pipeline instead of the captured
                    // one.
                    let current = live_rules
                        .read()
                        .ok()
                        .and_then(|rules| rules.iter().find(|r| r.id == task_rule_id).cloned());
                    let Some(current) = current else { break };
                    if !current.is_enabled || current.actions.is_empty() {
                        break;
                    }

                    if !current.conditions.is_empty()
                        && !DaemonEngine::evaluate_rule_conditions(&current.conditions)
                    {
                        let _ = tx.send(ActionCompletionEvent::Log {
                            level: "WARN".to_string(),
                            category: "RULE".to_string(),
                            message: format!("Rule '{}' triggered by {}, but conditions were not met. Skipping cycle.", rule_name, context_name),
                        });
                    } else {
                        let mut ctx = HashMap::new();
                        ctx.insert("ProcessName".into(), context_name.clone());
                        ctx.insert("ProcessId".into(), "0".into());
                        ctx.insert("Timestamp".into(), chrono::Local::now().to_rfc3339());
                        ctx.insert(rule_call::EXECUTION_CTX_KEY.to_string(), execution_id.to_string());

                        Self::insert_window_title(&mut ctx);

                        let run = PipelineRun {
                            actions: &current.actions,
                            execution_id,
                            rule_id: &task_rule_id,
                            rule_name: &rule_name,
                            context_name: "",
                            pid: 0,
                            is_exit: false,
                            stop_on_error: current.stop_on_error,
                            bind_companions: false,
                            label: "Pipeline",
                            error_label: "Action",
                            warn_on_error_halt: true,
                        };
                        if run_action_pipeline(run, &mut ctx, &tx).await {
                            break;
                        }

                        current_runs += 1;
                        let _ = tx.send(ActionCompletionEvent::IncrementExecutionCount {
                            rule_id: task_rule_id.clone(),
                        });

                        if let Some(limit) = current.max_executions {
                            if current_runs >= limit {
                                break;
                            }
                        }
                    }

                    tokio::time::sleep(interval_duration).await;
                }
            });

            self.executions.register_abort_handle(execution_id, rule_id.clone(), join_handle.abort_handle());
            self.executions.register_recurring_test(rule_id.clone(), join_handle.abort_handle(), execution_id);
            Some(join_handle)
        } else {
            Self::finish_pipeline_without_runtime(&tx, execution_id, rule_id, false);
            None
        }
    }
}
