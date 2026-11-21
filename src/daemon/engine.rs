use std::collections::{HashMap, HashSet};
use std::time::Instant;
use tokio::sync::{mpsc, oneshot};
use crate::config::AppConfig;
use crate::ipc::{DaemonState, IpcRequest, IpcResponse};
use crate::models::{
    Action, AnalyticsSummary, ConditionOperator, LogItem, ProcessMatchMode,
    Rule, RuleCondition, RuleConditionItem, ScheduleType, SystemEventType, Trigger,
};
use crate::daemon::actions::ActionRunner;
use crate::daemon::rule_call;
use crate::daemon::watcher::{ProcessEvent, ProcessWatcher};

/// Monotonic id shared by every kind of pipeline execution (triggered, exit, recurring and
/// rule-called). Active executions and abort handles are keyed by it, so it must never collide.
static EXECUTION_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_execution_id() -> u64 {
    EXECUTION_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Serializes config-section writes inside this process. The daemon is the only writer, but the
/// test harness runs many engines in one process against one config directory, and two atomic
/// writes to the same section would otherwise race each other and report a spurious failure.
static CONFIG_WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn config_write_guard() -> std::sync::MutexGuard<'static, ()> {
    CONFIG_WRITE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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
enum RuleSkip {
    Quota,
    Cooldown { elapsed: f64, cooldown: f64 },
    Conditions,
    EmptyPipeline,
}

impl RuleSkip {
    fn describe(&self) -> String {
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
    fn is_user_invoked(self) -> bool {
        matches!(self, Self::Manual | Self::Test)
    }

    /// Fallback mapping for trigger modules that only pass a context label. Engine-owned call
    /// sites pass the origin explicitly; this mapping exists so a pid of zero is never read as
    /// "manual" and the default for an unknown trigger is the non-manual system-event origin.
    fn from_context(context_name: &str, pid: u32) -> Self {
        if context_name == "MANUAL_TEST" {
            return Self::Test;
        }
        if context_name == "Manual Run" {
            return Self::Manual;
        }
        if context_name.starts_with("Global Hotkey") {
            return Self::Hotkey;
        }
        if context_name.starts_with("Webhook (") {
            return Self::Webhook;
        }
        if context_name.contains("Schedule")
            || context_name == "App Startup"
            || context_name == "App Shutdown"
        {
            return Self::Schedule;
        }
        if pid > 0 {
            return Self::Process;
        }
        Self::SystemEvent
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
struct RuleCallRuntime {
    /// Rules that led to this execution, outermost first, ending with the target itself.
    chain: Vec<String>,
    completion: oneshot::Sender<()>,
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
}

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
struct PipelineRun<'a> {
    actions: &'a [Action],
    execution_id: u64,
    rule_id: &'a str,
    rule_name: &'a str,
    /// Trigger context of the primary run, recorded on the companions it binds to a process.
    context_name: &'a str,
    pid: u32,
    is_exit: bool,
    stop_on_error: bool,
    /// Only the primary pipeline binds `close_on_exit` companions; exit and recurring runs do not.
    bind_companions: bool,
    /// "Pipeline" or "Exit pipeline": the subject of the graceful-halt log line.
    label: &'a str,
    /// "Action" or "Exit action": the subject of the failure log line.
    error_label: &'a str,
    /// The recurring loop stops when a cycle halts; the other callers ignore the answer.
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
async fn run_action_pipeline(
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

/// What a quota check means for one counted execution.
enum QuotaDecision {
    /// The run fits in the current window, or the rule has no limit.
    Allowed,
    /// A periodic quota is spent: the rule stays enabled and resumes with the next window.
    PeriodSpent { message: String },
    /// A lifetime quota is spent: the rule must be auto-paused.
    LimitReached { message: String },
}

impl QuotaDecision {
    /// Classifies `counted` executions against the rule's limit. `counted` includes the
    /// execution being decided: the pre-run gate passes the stored period count and the
    /// post-run path passes the already incremented counter, so the pause/rollover
    /// semantics and their message live here once.
    fn classify(rule: &Rule, counted: u64, now: chrono::DateTime<chrono::Local>) -> Self {
        let Some(limit) = rule.max_executions else {
            return Self::Allowed;
        };
        if counted < limit {
            return Self::Allowed;
        }
        if rule.quota_period.is_some() {
            Self::PeriodSpent {
                message: format!(
                    "Rule '{}' reached its quota ({}/{}); the counter resets {}.",
                    rule.name,
                    counted,
                    limit,
                    QuotaLedger::reset_text(rule.quota_next_reset(now))
                ),
            }
        } else {
            Self::LimitReached {
                message: format!(
                    "Rule '{}' completed quota ({}/{}) and has been auto-paused.",
                    rule.name, counted, limit
                ),
            }
        }
    }

    fn message(&self) -> Option<&str> {
        match self {
            Self::Allowed => None,
            Self::PeriodSpent { message } | Self::LimitReached { message } => Some(message),
        }
    }
}

/// Daemon-owned quota counters: the coalesced `runtime.json` flush clock and the single
/// place that turns a spent quota into its message and pause decision.
struct QuotaLedger {
    /// Set when rule counters changed; `runtime.json` is flushed coalesced so a fast-firing
    /// rule cannot rewrite the file on every execution.
    dirty: bool,
    last_save: Instant,
}

impl QuotaLedger {
    fn new() -> Self {
        Self {
            dirty: false,
            last_save: Instant::now(),
        }
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// True when counters changed and the coalescing window has elapsed.
    fn flush_due(&self, interval: std::time::Duration) -> bool {
        self.dirty && self.last_save.elapsed() >= interval
    }

    fn mark_flushed(&mut self) {
        self.dirty = false;
        self.last_save = Instant::now();
    }

    /// Human-readable "when" part of a periodic-quota reset message.
    fn reset_text(reset: Option<chrono::DateTime<chrono::Local>>) -> String {
        match reset {
            Some(at) => format!("at {}", at.format("%Y-%m-%d %H:%M")),
            None => "when the period ends".to_string(),
        }
    }
}

/// One counted execution, as reported by [`DaemonEngine::record_quota_increment`].
struct QuotaIncrement {
    decision: QuotaDecision,
    /// A lifetime limit is spent; the caller persists the pause (and may stop the rule).
    paused: bool,
}

/// Scheduler admission clocks. `rule_last_triggered` stays on [`DaemonEngine`] because the
/// daemon's hotkey-toggle path stamps it; the tick-level retry throttle and the
/// resource-threshold dwell tracker live here.
#[derive(Default)]
struct Scheduler {
    /// Instant of the last scheduler *attempt* (accepted or rejected). It only throttles
    /// tick-level retries so a rejected run is not retried every 250 ms; the cooldown gate
    /// reads `rule_last_triggered`, which is stamped when a run is accepted.
    last_attempt: HashMap<String, Instant>,
    /// Since when each `(rule, pid)` pair has been above its resource thresholds.
    resource_since: HashMap<(String, u32), Instant>,
}

impl Scheduler {
    fn new() -> Self {
        Self::default()
    }

    /// Drops every admitted-run clock of a rule that no longer exists.
    fn forget_rule(&mut self, rule_id: &str) {
        self.last_attempt.remove(rule_id);
        self.resource_since
            .retain(|(id, _), _| id.as_str() != rule_id);
    }

    /// Interval schedules. A persisted `last_executed_at` seeds the accepted-run clock so the
    /// next run happens one interval later, and a rejected attempt is retried once per
    /// interval instead of every tick. Returns whether a run is due now.
    fn interval_due(
        &mut self,
        rule: &Rule,
        last_triggered: &mut HashMap<String, Instant>,
        now_instant: Instant,
        now_local: chrono::DateTime<chrono::Local>,
        interval: u64,
        target_datetime: Option<&str>,
    ) -> bool {
        if interval == 0 {
            return false;
        }

        // If a specific first execution date & time is set, ensure current time >= target.
        if let Some(target) = target_datetime {
            let cur_datetime = now_local.format("%Y-%m-%d %H:%M:%S").to_string();
            let target_cmp = if target.len() == 16 {
                format!("{target}:00")
            } else {
                target.to_string()
            };
            if cur_datetime < target_cmp && !last_triggered.contains_key(&rule.id) {
                return false;
            }
        }

        let interval_due = match last_triggered.get(&rule.id) {
            Some(last) => last.elapsed().as_secs() >= interval,
            None => {
                if let Some(last_str) = &rule.last_executed_at {
                    match chrono::NaiveDateTime::parse_from_str(last_str, "%Y-%m-%d %H:%M:%S") {
                        Ok(last_dt) => {
                            let elapsed_secs = now_local
                                .naive_local()
                                .signed_duration_since(last_dt)
                                .num_seconds();
                            if elapsed_secs >= 0 && (elapsed_secs as u64) < interval {
                                // Seed the in-memory clock from the persisted run so the next
                                // one happens one interval later. This is not a trigger: the
                                // cooldown keeps measuring the last accepted run.
                                let virtual_start = now_instant
                                    - std::time::Duration::from_secs(elapsed_secs as u64);
                                last_triggered.insert(rule.id.clone(), virtual_start);
                                false
                            } else {
                                true
                            }
                        }
                        Err(_) => target_datetime.is_some(),
                    }
                } else if target_datetime.is_some() {
                    true
                } else {
                    // No run yet: the first tick starts the interval clock and later ticks
                    // measure the interval from that attempt.
                    match self.last_attempt.get(&rule.id).copied() {
                        Some(attempt) => attempt.elapsed().as_secs() >= interval,
                        None => {
                            self.last_attempt.insert(rule.id.clone(), now_instant);
                            false
                        }
                    }
                }
            }
        };

        // A rejected attempt is retried once per interval, not every 250 ms tick.
        let retry_throttled = last_triggered.contains_key(&rule.id)
            && self
                .last_attempt
                .get(&rule.id)
                .is_some_and(|attempt| attempt.elapsed().as_secs() < interval);

        if interval_due && !retry_throttled {
            self.last_attempt.insert(rule.id.clone(), now_instant);
            true
        } else {
            false
        }
    }

    /// Daily schedules: due after `daily_time` on a configured weekday, once per day, with
    /// rejected attempts retried after a short suppression window.
    fn daily_due(
        &mut self,
        rule: &Rule,
        now_instant: Instant,
        now_local: chrono::DateTime<chrono::Local>,
        daily_time: &str,
        days_of_week: &[u8],
    ) -> bool {
        use chrono::Datelike;
        let current_dow = now_local.weekday().number_from_monday() as u8;
        if !days_of_week.is_empty() && !days_of_week.contains(&current_dow) {
            return false;
        }

        let today = now_local.format("%Y-%m-%d").to_string();
        let already_executed_today = rule
            .last_executed_at
            .as_deref()
            .is_some_and(|ts| ts.starts_with(&today));
        let retry_suppressed = self
            .last_attempt
            .get(&rule.id)
            .is_some_and(|attempt| attempt.elapsed().as_secs() < 3);
        if already_executed_today || retry_suppressed {
            return false;
        }

        let cur_time_hms = now_local.format("%H:%M:%S").to_string();
        let cur_time_hm = now_local.format("%H:%M").to_string();
        let target_time_str = if daily_time.len() == 5 {
            format!("{daily_time}:00")
        } else {
            daily_time.to_string()
        };
        let target_passed =
            cur_time_hms >= target_time_str || cur_time_hm.as_str() >= daily_time;

        if target_passed {
            // The accepted run stamps `rule_last_triggered`; stamping it here would make the
            // cooldown reject that very run.
            self.last_attempt.insert(rule.id.clone(), now_instant);
            true
        } else {
            false
        }
    }

    /// Monthly schedules: due from `day_of_month` at `daily_time` onward, once per month.
    fn monthly_due(
        rule: &Rule,
        now_local: chrono::DateTime<chrono::Local>,
        day_of_month: Option<u8>,
        daily_time: Option<&str>,
    ) -> bool {
        use chrono::Datelike;
        let target_day = day_of_month.unwrap_or(1).clamp(1, 31);
        let target_time_str = daily_time.unwrap_or("12:00:00");
        let cur_year_month = now_local.format("%Y-%m").to_string();
        let cur_day = now_local.day() as u8;
        let cur_time_hms = now_local.format("%H:%M:%S").to_string();
        let cur_time_hm = now_local.format("%H:%M").to_string();

        let already_executed_this_month = rule
            .last_executed_at
            .as_deref()
            .is_some_and(|ts| ts.starts_with(&cur_year_month));
        if already_executed_this_month {
            return false;
        }

        if cur_day > target_day {
            true
        } else if cur_day == target_day {
            cur_time_hms.as_str() >= target_time_str
                || cur_time_hm.as_str() >= target_time_str
                || format!("{cur_time_hm}:00").as_str() >= target_time_str
        } else {
            false
        }
    }

    /// One-shot schedules: due from `target` onward until the run is accepted. A rejected run
    /// keeps the one-shot armed.
    fn oneshot_due(
        rule: &Rule,
        last_triggered: &HashMap<String, Instant>,
        now_local: chrono::DateTime<chrono::Local>,
        target: &str,
    ) -> bool {
        let cur_datetime = now_local.format("%Y-%m-%d %H:%M:%S").to_string();
        let target_cmp = if target.len() == 16 {
            format!("{target}:00")
        } else {
            target.to_string()
        };
        cur_datetime >= target_cmp && !last_triggered.contains_key(&rule.id)
    }

    /// Drops resource-threshold dwell entries for rules that were disabled or processes that
    /// disappeared, so the trigger restarts its dwell from a fresh sighting.
    fn retain_resource_tracking(
        &mut self,
        active_rule_ids: &HashSet<&str>,
        active_pids: &HashSet<u32>,
    ) {
        self.resource_since.retain(|(rule_id, pid), _| {
            active_rule_ids.contains(rule_id.as_str()) && active_pids.contains(pid)
        });
    }

    /// Since when `(rule, pid)` has been above its thresholds; inserts `now` on first sight.
    fn resource_since(&mut self, rule_id: &str, pid: u32, now: Instant) -> Instant {
        *self
            .resource_since
            .entry((rule_id.to_string(), pid))
            .or_insert(now)
    }

    fn reset_resource_dwell(&mut self, rule_id: &str, pid: u32, now: Instant) {
        self.resource_since
            .insert((rule_id.to_string(), pid), now);
    }

    fn clear_resource_dwell(&mut self, rule_id: &str, pid: u32) {
        self.resource_since.remove(&(rule_id.to_string(), pid));
    }

    fn clear_resource_tracking(&mut self) {
        self.resource_since.clear();
    }

    fn resource_tracking_is_empty(&self) -> bool {
        self.resource_since.is_empty()
    }
}

/// Runtime execution tracker: the pipelines that are running, their abort handles, the recurring
/// test loops and the overlays bound to a host process. The engine mutates it only through these
/// methods, so an aborted pipeline always loses its abort handle and a finished execution always
/// leaves the recurring set.
#[derive(Default)]
struct ExecutionTracker {
    /// Pipelines currently running, keyed by the process-wide execution id.
    active_executions: HashMap<u64, ActiveExecutionItem>,
    /// Abort handles of spawned pipeline tasks: execution id → (owning rule id, handle).
    abort_handles: HashMap<u64, (String, tokio::task::AbortHandle)>,
    /// Recurring test loops (interval / held-hotkey emulation): rule id → (abort handle,
    /// execution id the loop was started as).
    recurring_tests: HashMap<String, (tokio::task::AbortHandle, u64)>,
    /// Overlays bound to a host process, hidden when that process exits.
    bound_overlays: Vec<BoundOverlay>,
}

impl ExecutionTracker {
    fn new() -> Self {
        Self::default()
    }

    /// True when a tracked execution kind is running for `rule_id`. The bound-process registry
    /// stays on the engine because `src/daemon.rs` reads the public field while polling.
    fn is_rule_executing(&self, rule_id: &str) -> bool {
        self.active_executions
            .values()
            .any(|item| item.rule_id == rule_id)
            || self
                .abort_handles
                .values()
                .any(|(owning_rule, _)| owning_rule == rule_id)
            || self.recurring_tests.contains_key(rule_id)
    }

    fn has_recurring_test(&self, rule_id: &str) -> bool {
        self.recurring_tests.contains_key(rule_id)
    }

    fn recurring_rule_ids(&self) -> impl Iterator<Item = &String> {
        self.recurring_tests.keys()
    }

    fn active_executions(&self) -> impl Iterator<Item = &ActiveExecutionItem> {
        self.active_executions.values()
    }

    /// The call chain recorded for a running execution, outermost first.
    fn call_chain(&self, execution_id: u64) -> Vec<String> {
        self.active_executions
            .get(&execution_id)
            .map(|execution| execution.call_chain.clone())
            .unwrap_or_default()
    }

    /// True when `execution_id` is currently running `rule_id` (self-call detection).
    fn is_running_rule(&self, execution_id: u64, rule_id: &str) -> bool {
        self.active_executions
            .get(&execution_id)
            .is_some_and(|execution| execution.rule_id == rule_id)
    }

    /// Rule owning a running execution, used to attribute a rule call to its caller.
    fn running_rule_id(&self, execution_id: u64) -> Option<&str> {
        self.active_executions
            .get(&execution_id)
            .map(|execution| execution.rule_id.as_str())
    }

    fn record_started(&mut self, execution_id: u64, item: ActiveExecutionItem) {
        self.active_executions.insert(execution_id, item);
    }

    fn record_step(&mut self, execution_id: u64, step_index: usize, total_steps: usize, is_exit: bool) {
        if let Some(item) = self.active_executions.get_mut(&execution_id) {
            item.step_index = step_index;
            item.total_steps = total_steps;
            item.is_exit = is_exit;
        }
    }

    /// Forgets a finished pipeline: its entry, its abort handle and — when this execution was
    /// the recurring loop — the recurring-test entry.
    fn record_finished(&mut self, execution_id: u64, rule_id: &str) {
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

    fn register_abort_handle(
        &mut self,
        execution_id: u64,
        rule_id: String,
        handle: tokio::task::AbortHandle,
    ) {
        self.abort_handles.insert(execution_id, (rule_id, handle));
    }

    fn register_recurring_test(
        &mut self,
        rule_id: String,
        handle: tokio::task::AbortHandle,
        execution_id: u64,
    ) {
        self.recurring_tests.insert(rule_id, (handle, execution_id));
    }

    /// Aborts the recurring test loop of `rule_id`, if one is running.
    fn abort_recurring_test(&mut self, rule_id: &str) -> bool {
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
    fn abort_rule(&mut self, rule_id: &str) -> usize {
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

    fn add_bound_overlay(&mut self, overlay: BoundOverlay) {
        self.bound_overlays.push(overlay);
    }

    fn bound_overlays(&self) -> &[BoundOverlay] {
        &self.bound_overlays
    }

    fn remove_rule_bound_overlays(&mut self, rule_id: &str) {
        self.bound_overlays.retain(|overlay| overlay.rule_id != rule_id);
    }

    /// Removes and returns the bound overlays of one rule.
    fn take_rule_bound_overlays(&mut self, rule_id: &str) -> Vec<BoundOverlay> {
        self.take_bound_overlays_where(|overlay| overlay.rule_id == rule_id)
    }

    /// Removes and returns the bound overlays matching `predicate`, keeping the rest in order.
    /// Entries sharing the `(rule, overlay)` identity of a taken one are removed as well, exactly
    /// like the pre-split filter + retain pair.
    fn take_bound_overlays_where(
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
}

/// Builds the [`DaemonState`] snapshot the GUI renders. It is a read-only view over the engine:
/// [`DaemonEngine::get_current_state`] keeps the two state mutations (event drain and quota
/// rollover) and delegates the aggregation to this projector.
struct StateProjector<'a> {
    engine: &'a DaemonEngine,
}

impl<'a> StateProjector<'a> {
    fn new(engine: &'a DaemonEngine) -> Self {
        Self { engine }
    }

    fn project(&self) -> DaemonState {
        let active_rules = self
            .engine
            .config
            .rules
            .iter()
            .filter(|rule| rule.is_enabled)
            .count();
        let total_rules = self.engine.config.rules.len();

        let active_overlay_ids = crate::daemon::overlay_manager::get_active_overlay_ids();
        let active_rule_executions = self.active_rule_executions(&active_overlay_ids);
        let active_test_rule_ids = self.engine.executions.recurring_rule_ids().cloned().collect();

        crate::daemon::telemetry::update_engine_stats(
            active_rules,
            active_rule_executions.len(),
            None,
            None,
        );

        let (update_progress, update_status) = self.engine.update_progress_snapshot();

        DaemonState {
            autorun_enabled: crate::autorun::is_autorun_enabled(crate::autorun::APP_NAME),
            uptime_secs: self.engine.start_time.elapsed().as_secs(),
            daemon_memory_bytes: Self::daemon_memory(),
            active_rules_count: active_rules,
            total_rules_count: total_rules,
            config: self.engine.config.clone(),
            logs: self.engine.logs.clone(),
            active_rule_executions,
            active_overlay_ids,
            active_test_rule_ids,
            analytics: self.engine.analytics.clone(),
            available_update: self.engine.available_update.clone(),
            update_progress,
            update_status,
            pending_open_update_modal: self.engine.pending_open_update_modal,
            pending_auto_start_update: self.engine.pending_auto_start_update,
        }
    }

    /// Active executions grouped per rule, plus the runs implied by a bound companion or a
    /// showing overlay so the GUI can render a rule as running while its satellite lives.
    fn active_rule_executions(
        &self,
        active_overlay_ids: &HashSet<String>,
    ) -> HashMap<String, crate::ipc::ActiveRuleExecutionState> {
        let mut active_rule_executions = HashMap::new();
        let mut grouped: HashMap<String, Vec<&ActiveExecutionItem>> = HashMap::new();
        for item in self.engine.executions.active_executions() {
            grouped.entry(item.rule_id.clone()).or_default().push(item);
        }
        for (rule_id, items) in grouped {
            let active_count = items.len();
            if let Some(latest) = items.into_iter().max_by_key(|item| item.started_at) {
                active_rule_executions.insert(rule_id, crate::ipc::ActiveRuleExecutionState {
                    active_count,
                    current_step: latest.step_index,
                    total_steps: latest.total_steps,
                    is_exit: latest.is_exit,
                });
            }
        }

        // Track active bound satellites waiting for host process exit
        for bp in &self.engine.active_bound_processes {
            if !active_rule_executions.contains_key(&bp.rule_id) {
                if let Some(rule) = self.engine.config.rules.iter().find(|r| r.id == bp.rule_id) {
                    let step_idx = rule
                        .actions
                        .iter()
                        .position(|a| matches!(a, Action::StartProcess { close_on_exit: true, .. }))
                        .unwrap_or(0);
                    active_rule_executions.insert(
                        bp.rule_id.clone(),
                        crate::ipc::ActiveRuleExecutionState {
                            active_count: 1,
                            current_step: step_idx,
                            total_steps: rule.actions.len(),
                            is_exit: false,
                        },
                    );
                }
            }
        }

        // Track active bound overlays waiting for host process exit
        for bo in self.engine.executions.bound_overlays() {
            if !active_rule_executions.contains_key(&bo.rule_id) {
                if let Some(rule) = self.engine.config.rules.iter().find(|r| r.id == bo.rule_id) {
                    let step_idx = rule
                        .actions
                        .iter()
                        .position(|a| matches!(a, Action::Overlay { close_on_exit: true, .. }))
                        .unwrap_or(0);
                    active_rule_executions.insert(
                        bo.rule_id.clone(),
                        crate::ipc::ActiveRuleExecutionState {
                            active_count: 1,
                            current_step: step_idx,
                            total_steps: rule.actions.len(),
                            is_exit: false,
                        },
                    );
                }
            }
        }

        // Track rules with active showing overlays
        if !active_overlay_ids.is_empty() {
            for rule in &self.engine.config.rules {
                if !active_rule_executions.contains_key(&rule.id) {
                    let overlay_step = rule.actions.iter().enumerate().find_map(|(idx, a)| {
                        match a {
                            Action::Overlay { overlay_id, custom_overlay, .. } => {
                                let id = custom_overlay.as_ref().map(|o| o.id.as_str()).unwrap_or(overlay_id.as_str());
                                if active_overlay_ids.contains(id) {
                                    Some(idx)
                                } else {
                                    None
                                }
                            }
                            Action::IfElse { then_actions, else_if_branches, else_actions, .. } => {
                                let in_then = then_actions.iter().any(|sub| match sub {
                                    Action::Overlay { overlay_id, custom_overlay, .. } => {
                                        let id = custom_overlay.as_ref().map(|o| o.id.as_str()).unwrap_or(overlay_id.as_str());
                                        active_overlay_ids.contains(id)
                                    }
                                    _ => false,
                                });
                                let in_elseif = else_if_branches.iter().any(|b| b.actions.iter().any(|sub| match sub {
                                    Action::Overlay { overlay_id, custom_overlay, .. } => {
                                        let id = custom_overlay.as_ref().map(|o| o.id.as_str()).unwrap_or(overlay_id.as_str());
                                        active_overlay_ids.contains(id)
                                    }
                                    _ => false,
                                }));
                                let in_else = else_actions.iter().any(|sub| match sub {
                                    Action::Overlay { overlay_id, custom_overlay, .. } => {
                                        let id = custom_overlay.as_ref().map(|o| o.id.as_str()).unwrap_or(overlay_id.as_str());
                                        active_overlay_ids.contains(id)
                                    }
                                    _ => false,
                                });
                                if in_then || in_elseif || in_else {
                                    Some(idx)
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        }
                    });

                    if let Some(step_idx) = overlay_step {
                        active_rule_executions.insert(
                            rule.id.clone(),
                            crate::ipc::ActiveRuleExecutionState {
                                active_count: 1,
                                current_step: step_idx,
                                total_steps: rule.actions.len(),
                                is_exit: false,
                            },
                        );
                    }
                }
            }
        }

        // Track rules in active recurring test execution (e.g. interval triggers)
        for rule_id in self.engine.executions.recurring_rule_ids() {
            if !active_rule_executions.contains_key(rule_id) {
                let total_steps = self.engine.config.rules.iter().find(|r| &r.id == rule_id)
                    .map(|r| r.actions.len().max(1))
                    .unwrap_or(1);
                active_rule_executions.insert(rule_id.clone(), crate::ipc::ActiveRuleExecutionState {
                    active_count: 1,
                    current_step: 0,
                    total_steps,
                    is_exit: false,
                });
            }
        }

        active_rule_executions
    }

    fn daemon_memory() -> u64 {
        #[cfg(windows)]
        unsafe {
            use windows::Win32::System::Threading::GetCurrentProcess;
            use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
            let mut pmc: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
            pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
            if K32GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb).as_bool() {
                return pmc.WorkingSetSize as u64;
            }
        }
        2 * 1024 * 1024 // ~2MB default
    }
}

pub struct DaemonEngine {
    pub config: AppConfig,
    start_time: Instant,
    logs: Vec<LogItem>,
    pub watcher: ProcessWatcher,
    active_bound_processes: Vec<BoundProcess>,
    /// Active pipelines, abort handles, recurring test loops and bound overlays.
    executions: ExecutionTracker,
    pub rule_last_triggered: HashMap<String, Instant>,
    /// Latest rule definitions plus their runtime counters, shared with recurring executions so a
    /// running loop re-reads its rule instead of executing a stale captured pipeline.
    live_rules: std::sync::Arc<std::sync::RwLock<Vec<Rule>>>,
    active_rule_pids: HashMap<String, HashSet<u32>>,
    rule_overlay_map: HashMap<String, Vec<String>>,
    analytics: AnalyticsSummary,
    last_foreground_pid: Option<u32>,
    pub hotkey_handle: Option<crate::daemon::hotkeys::HotkeyHandle>,
    pub available_update: Option<crate::updater::ReleaseInfo>,
    pub is_downloading_update: bool,
    pub update_download_progress: f32,
    pub update_status: Option<String>,
    pub pending_open_update_modal: bool,
    pub pending_auto_start_update: bool,
    /// Scheduler admission clocks (attempt throttle and resource-threshold dwell).
    scheduler: Scheduler,
    /// Quota counters and their coalesced `runtime.json` flush clock.
    quota: QuotaLedger,
    /// Overlay drags stream position updates; persisting presets on every frame would
    /// hammer `library.json`, so those saves are coalesced separately.
    overlay_position_library_dirty: bool,
    overlay_position_rules_dirty: bool,
    last_overlay_position_save: Instant,
    action_tx: mpsc::UnboundedSender<ActionCompletionEvent>,
    action_rx: mpsc::UnboundedReceiver<ActionCompletionEvent>,
    /// Test-only switch that makes rule writes fail so the IPC error contract can be exercised
    /// without corrupting the shared test config directory.
    #[cfg(test)]
    forced_persist_failure: bool,
}

#[cfg(windows)]
pub fn get_current_foreground_process() -> Option<String> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    let hwnd = unsafe { GetForegroundWindow() };
    if !hwnd.0.is_null() {
        let mut fg_pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut fg_pid)) };
        if fg_pid != 0 {
            let snap = ProcessWatcher::snapshot_process_map();
            return snap.get(&fg_pid).cloned();
        }
    }
    None
}
#[cfg(not(windows))]
pub fn get_current_foreground_process() -> Option<String> {
    None
}

static REGEX_CACHE: std::sync::LazyLock<std::sync::RwLock<std::collections::HashMap<String, Option<regex::Regex>>>> =
    std::sync::LazyLock::new(|| std::sync::RwLock::new(std::collections::HashMap::new()));

fn match_cached_regex(pattern: &str, actual: &str) -> bool {
    if let Ok(cache) = REGEX_CACHE.read() {
        if let Some(opt_re) = cache.get(pattern) {
            return match opt_re {
                Some(re) => re.is_match(actual),
                None => actual.to_lowercase().contains(&pattern.to_lowercase()),
            };
        }
    }

    let compiled = regex::RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .ok();

    let matched = match &compiled {
        Some(re) => re.is_match(actual),
        None => actual.to_lowercase().contains(&pattern.to_lowercase()),
    };

    if let Ok(mut cache) = REGEX_CACHE.write() {
        if cache.len() >= 256 {
            cache.clear();
        }
        cache.insert(pattern.to_string(), compiled);
    }

    matched
}

/// Merges the daemon-owned runtime counters into a freshly saved rule definition.
/// Redefining the quota (changing its period, or enabling one) starts a fresh counting
/// window, because the old counter measured a different thing; every other edit keeps
/// the counters so a rule continues where it left off. Returns `true` when the counters
/// were reset and `runtime.json` therefore has to be flushed. `last_executed_at` is
/// daemon state too and is always preserved, since schedule triggers use it for
/// deduplication.
fn adopt_rule_runtime(rule: &mut Rule, existing: &Rule) -> bool {
    let quota_redefined = rule.quota_redefined_from(existing);
    if quota_redefined {
        rule.reset_quota_progress();
    } else {
        rule.execution_count = existing.execution_count;
        rule.quota_window_start = existing.quota_window_start.clone();
    }
    rule.last_executed_at = existing.last_executed_at.clone();
    quota_redefined
}

impl DaemonEngine {
    pub const MAX_LOGS: usize = 500;
    /// Coalescing window for `runtime.json`; counters may lag the daemon by this much
    /// after a hard kill, which is acceptable for execution quotas.
    const RUNTIME_SAVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);
    /// Coalescing window for overlay positions streamed while dragging an overlay.
    const OVERLAY_POSITION_SAVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

    pub fn new(config: AppConfig) -> Self {
        let (action_tx, action_rx) = mpsc::unbounded_channel();
        let live_rules = std::sync::Arc::new(std::sync::RwLock::new(config.rules.clone()));
        Self {
            config,
            start_time: Instant::now(),
            logs: Vec::new(),
            watcher: ProcessWatcher::new(),
            active_bound_processes: Vec::new(),
            executions: ExecutionTracker::new(),
            rule_last_triggered: HashMap::new(),
            live_rules,
            active_rule_pids: HashMap::new(),
            rule_overlay_map: HashMap::new(),
            analytics: AnalyticsSummary::default(),
            last_foreground_pid: None,
            hotkey_handle: None,
            available_update: None,
            is_downloading_update: false,
            update_download_progress: 0.0,
            update_status: None,
            pending_open_update_modal: false,
            pending_auto_start_update: false,
            scheduler: Scheduler::new(),
            quota: QuotaLedger::new(),
            overlay_position_library_dirty: false,
            overlay_position_rules_dirty: false,
            last_overlay_position_save: Instant::now(),
            action_tx,
            action_rx,
            #[cfg(test)]
            forced_persist_failure: false,
        }
    }

    /// Persists the hot runtime counters at most once per window instead of on every
    /// execution. Quota pauses and shutdown force an immediate flush.
    pub fn persist_runtime_if_due(&mut self) {
        if !self.quota.flush_due(Self::RUNTIME_SAVE_INTERVAL) {
            return;
        }
        self.persist_runtime_now();
    }

    /// Forces `runtime.json` to disk now.
    pub fn persist_runtime_now(&mut self) {
        if !self.quota.is_dirty() {
            return;
        }
        let _write = config_write_guard();
        match self.config.runtime_state().save() {
            Ok(()) => {
                self.quota.mark_flushed();
            }
            Err(err) => {
                self.log(
                    "ERROR",
                    "CONFIG",
                    &format!("Failed to save runtime counters: {err}"),
                );
            }
        }
    }

    fn mark_runtime_dirty(&mut self) {
        self.quota.mark_dirty();
    }

    /// Publishes the current rule definitions to recurring executions. A recurring loop re-reads
    /// its rule every cycle instead of executing the pipeline it captured when it started.
    fn refresh_live_rules(&self) {
        if let Ok(mut live) = self.live_rules.write() {
            *live = self.config.rules.clone();
        }
    }

    /// A rule edit that redefined its quota resets the daemon-owned counter, so the
    /// stale `runtime.json` entry must be replaced immediately: the regular flush is
    /// coalesced, and a restart before it would resurrect the old counter.
    fn flush_quota_reset(&mut self, reset: bool) {
        if !reset {
            return;
        }
        self.mark_runtime_dirty();
        self.persist_runtime_now();
    }

    /// Persists positions streamed by an overlay drag at most once per window; the
    /// in-memory position is always current, only the disk write is deferred.
    pub fn persist_overlay_positions_if_due(&mut self) {
        if !self.overlay_position_library_dirty && !self.overlay_position_rules_dirty {
            return;
        }
        if self.last_overlay_position_save.elapsed() < Self::OVERLAY_POSITION_SAVE_INTERVAL {
            return;
        }
        self.persist_overlay_positions_now();
    }

    /// Forces deferred overlay positions to disk (daemon shutdown, calibration end).
    pub fn persist_overlay_positions_now(&mut self) {
        if self.overlay_position_library_dirty && self.persist_library() {
            self.overlay_position_library_dirty = false;
        }
        if self.overlay_position_rules_dirty && self.persist_rules() {
            self.overlay_position_rules_dirty = false;
        }
        self.last_overlay_position_save = Instant::now();
    }

    fn mark_overlay_position_dirty(&mut self, library_changed: bool, rules_changed: bool) {
        self.overlay_position_library_dirty |= library_changed;
        self.overlay_position_rules_dirty |= rules_changed;
    }

    fn persist_rules(&mut self) -> bool {
        #[cfg(test)]
        if self.forced_persist_failure {
            self.log("ERROR", "CONFIG", "Failed to save rules: injected test failure");
            return false;
        }
        let _write = config_write_guard();
        match self.config.save_rules() {
            Ok(()) => true,
            Err(err) => {
                self.log("ERROR", "CONFIG", &format!("Failed to save rules: {err}"));
                false
            }
        }
    }

    fn persist_library(&mut self) -> bool {
        let _write = config_write_guard();
        match self.config.save_library() {
            Ok(()) => true,
            Err(err) => {
                self.log("ERROR", "CONFIG", &format!("Failed to save presets: {err}"));
                false
            }
        }
    }

    fn persist_settings(&mut self) {
        let _write = config_write_guard();
        if let Err(err) = self.config.save_settings() {
            self.log("ERROR", "CONFIG", &format!("Failed to save settings: {err}"));
        }
    }

    /// IPC answer for a failed config write. A handler that answers for a write must never report
    /// success when the daemon could not store it.
    fn persist_failure(section: &str) -> IpcResponse {
        IpcResponse::Error(format!("failed to save {section}"))
    }

    /// Whether any rule currently has a bound satellite process alive. Used by the daemon loop to
    /// decide whether a process trigger must keep polling; the registry itself stays private.
    pub fn has_active_bound_companions(&self) -> bool {
        !self.active_bound_processes.is_empty()
    }

    pub fn is_rule_executing(&self, rule_id: &str) -> bool {
        if self.executions.is_rule_executing(rule_id)
            || self.active_bound_processes.iter().any(|bp| bp.rule_id == rule_id)
        {
            return true;
        }

        let active_ovs = crate::daemon::overlay_manager::get_active_overlay_ids();
        if !active_ovs.is_empty() {
            if let Some(rule) = self.config.rules.iter().find(|r| r.id == rule_id) {
                return rule.actions.iter().any(|a| match a {
                    Action::Overlay { overlay_id, custom_overlay, .. } => {
                        let id = custom_overlay.as_ref().map(|o| o.id.as_str()).unwrap_or(overlay_id.as_str());
                        active_ovs.contains(id)
                    }
                    _ => false,
                });
            }
        }

        false
    }

    pub fn extract_rule_overlay_ids(rule: &Rule) -> Vec<String> {
        let mut ids = Vec::new();
        fn collect_action_overlays(action: &crate::models::Action, ids: &mut Vec<String>) {
            match action {
                crate::models::Action::Overlay { overlay_id, custom_overlay, .. } => {
                    let id = custom_overlay.as_ref().map(|o| o.id.clone()).unwrap_or_else(|| overlay_id.clone());
                    ids.push(id);
                }
                crate::models::Action::IfElse { then_actions, else_if_branches, else_actions, .. } => {
                    for a in then_actions {
                        collect_action_overlays(a, ids);
                    }
                    for b in else_if_branches {
                        for a in &b.actions {
                            collect_action_overlays(a, ids);
                        }
                    }
                    for a in else_actions {
                        collect_action_overlays(a, ids);
                    }
                }
                _ => {}
            }
        }
        for a in &rule.actions {
            collect_action_overlays(a, &mut ids);
        }
        for a in &rule.on_exit_actions {
            collect_action_overlays(a, &mut ids);
        }
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
    fn abort_recurring_test(&mut self, rule_id: &str) -> bool {
        if !self.executions.abort_recurring_test(rule_id) {
            return false;
        }
        self.log("INFO", "RULE", &format!("Stopped recurring test run of rule: {}", rule_id));
        true
    }

    /// Aborts every active execution of `rule_id` and releases its runtime resources without
    /// changing `is_enabled`. The user-facing stop, a rule-to-rule stop, the GUI test emulation
    /// and HoldStop share this path; only [`Self::stop_rule_execution`] persists a pause.
    pub fn abort_rule_executions(&mut self, rule_id: &str) {
        // 1. Abort any active recurring test loop
        self.abort_recurring_test(rule_id);

        // 2. Abort all execution handles matching rule_id
        for _ in 0..self.executions.abort_rule(rule_id) {
            self.log("INFO", "RULE", &format!("Stopped execution of rule: {}", rule_id));
        }

        // 3. Hide any tracked overlays associated with this rule (including draft rules)
        if let Some(ids) = self.rule_overlay_map.remove(rule_id) {
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
    fn bound_satellite_kill(target_executable: &str) -> (String, Action) {
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
        let to_kill: Vec<BoundProcess> = self
            .active_bound_processes
            .iter()
            .filter(|bp| bp.rule_id == rule_id)
            .cloned()
            .collect();
        self.active_bound_processes.retain(|bp| bp.rule_id != rule_id);

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
                    self.active_bound_processes.push(bp);
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
                ActionCompletionEvent::IncrementExecutionCount { rule_id } => {
                    self.analytics.total_executions += 1;
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

    pub fn log(&mut self, level: &str, category: &str, message: &str) {
        self.log_with_file_message(level, category, message, None);
    }

    /// Shared implementation of [`Self::log`]. `file_message` overrides the text mirrored into the
    /// daily file log when the in-app message may carry user data (script output, HTTP bodies).
    fn log_with_file_message(
        &mut self,
        level: &str,
        category: &str,
        message: &str,
        file_message: Option<&str>,
    ) {
        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();

        if let Some(last) = self.logs.last_mut() {
            if last.level == level && last.category == category {
                let (is_match, count) = if last.message == message {
                    (true, 1)
                } else if let Some(stripped) = last.message.strip_prefix(message) {
                    if let Some(inner) = stripped.strip_prefix(" (x").and_then(|s| s.strip_suffix(')')) {
                        if let Ok(n) = inner.parse::<usize>() {
                            (true, n)
                        } else {
                            (false, 1)
                        }
                    } else {
                        (false, 1)
                    }
                } else {
                    (false, 1)
                };

                if is_match {
                    let next_count = count + 1;
                    last.message = format!("{} (x{})", message, next_count);
                    last.timestamp = timestamp;
                    return;
                }
            }
        }

        while self.logs.len() >= Self::MAX_LOGS {
            // Prioritize preserving RULE and ERROR logs: evict oldest entry that is neither RULE nor ERROR
            if let Some(pos) = self.logs.iter().position(|l| !l.category.eq_ignore_ascii_case("RULE") && !l.level.eq_ignore_ascii_case("ERROR")) {
                self.logs.remove(pos);
            } else {
                self.logs.remove(0);
            }
        }
        self.logs.push(LogItem {
            timestamp,
            level: level.to_string(),
            category: category.to_string(),
            message: message.to_string(),
        });
        // Mirror the engine event stream into the daily file log. Consecutive duplicates are
        // already collapsed above, so a chatty rule cannot flood the file.
        crate::logging::engine_log(level, category, file_message.unwrap_or(message));
    }

    pub async fn tick_process_events(&mut self) -> bool {
        self.drain_action_events();
        let events = self.watcher.poll_changes();
        let had_events = !events.is_empty();
        for event in events {
            self.handle_process_event(event).await;
        }
        had_events
    }

    #[inline]
    pub fn strip_exe_suffix(name: &str) -> &str {
        if name.len() >= 4 && name[name.len() - 4..].eq_ignore_ascii_case(".exe") {
            &name[..name.len() - 4]
        } else {
            name
        }
    }

    /// Matches an optional device filter against the interface path reported by Windows.
    /// An empty filter accepts any device; otherwise the filter is a case-insensitive
    /// substring, which works for IDs (`VID_1234`), interface paths and drive letters.
    pub fn device_filter_matches(filter: Option<&str>, device_id: Option<&str>) -> bool {
        let Some(filter) = filter.map(str::trim).filter(|value| !value.is_empty()) else {
            return true;
        };
        if filter == "*" {
            return true;
        }
        let Some(device_id) = device_id else {
            return false;
        };
        device_id
            .to_lowercase()
            .contains(&filter.to_lowercase())
    }

    pub fn matches_process(pattern: &str, actual: &str, mode: ProcessMatchMode) -> bool {
        if pattern == "*" {
            return true;
        }
        match mode {
            ProcessMatchMode::Exact => {
                pattern.eq_ignore_ascii_case(actual)
                    || Self::strip_exe_suffix(pattern).eq_ignore_ascii_case(Self::strip_exe_suffix(actual))
            }
            ProcessMatchMode::Wildcard => {
                let match_wildcard = |p: &str, a: &str| -> bool {
                    let pat = p.to_lowercase();
                    let act = a.to_lowercase();
                    if pat.starts_with('*') && pat.ends_with('*') && pat.len() >= 2 {
                        act.contains(&pat[1..pat.len() - 1])
                    } else if let Some(stripped) = pat.strip_prefix('*') {
                        act.ends_with(stripped)
                    } else if let Some(stripped) = pat.strip_suffix('*') {
                        act.starts_with(stripped)
                    } else {
                        act == pat || Self::strip_exe_suffix(&act) == Self::strip_exe_suffix(&pat)
                    }
                };
                match_wildcard(pattern, actual) || match_wildcard(pattern, Self::strip_exe_suffix(actual))
            }
            ProcessMatchMode::Regex => {
                match_cached_regex(pattern, actual) || match_cached_regex(pattern, Self::strip_exe_suffix(actual))
            }
        }
    }

    pub fn evaluate_condition(condition: &RuleCondition) -> bool {
        match condition {
            RuleCondition::ProcessRunning { process_name, match_mode, must_be_running } => {
                let snap = ProcessWatcher::snapshot_process_map();
                let is_running = snap.values().any(|name| Self::matches_process(process_name, name, *match_mode));
                is_running == *must_be_running
            }
            RuleCondition::WindowFocused { process_name, match_mode, must_be_focused } => {
                let fg_name = get_current_foreground_process();
                let is_focused = fg_name.as_deref().is_some_and(|name| Self::matches_process(process_name, name, *match_mode));
                is_focused == *must_be_focused
            }
            RuleCondition::TimeRange { start_time, end_time, days_of_week } => {
                let now = chrono::Local::now();
                if !days_of_week.is_empty() {
                    use chrono::Datelike;
                    let dow = now.weekday().number_from_monday() as u8;
                    if !days_of_week.contains(&dow) {
                        return false;
                    }
                }
                if start_time.is_empty() && end_time.is_empty() {
                    return true;
                }
                let cur_hm = now.format("%H:%M").to_string();
                let st = if start_time.is_empty() { "00:00" } else { start_time.as_str() };
                let et = if end_time.is_empty() { "23:59" } else { end_time.as_str() };
                if st <= et {
                    cur_hm.as_str() >= st && cur_hm.as_str() <= et
                } else {
                    cur_hm.as_str() >= st || cur_hm.as_str() <= et
                }
            }
            RuleCondition::Event { event_type, idle_duration_sec } => {
                match event_type {
                    crate::models::SystemEventType::PowerSourceAc => {
                        crate::daemon::system_events::is_ac_power().unwrap_or(true)
                    }
                    crate::models::SystemEventType::PowerSourceBattery => {
                        !crate::daemon::system_events::is_ac_power().unwrap_or(true)
                    }
                    crate::models::SystemEventType::UserIdle => {
                        let idle_s = crate::daemon::system_events::get_user_idle_seconds().unwrap_or(0);
                        idle_s >= (*idle_duration_sec as u64)
                    }
                    crate::models::SystemEventType::UserActive => {
                        let idle_s = crate::daemon::system_events::get_user_idle_seconds().unwrap_or(0);
                        idle_s < (*idle_duration_sec as u64)
                    }
                    crate::models::SystemEventType::SessionLock => {
                        crate::daemon::system_events::is_session_locked()
                    }
                    crate::models::SystemEventType::SessionUnlock => {
                        !crate::daemon::system_events::is_session_locked()
                    }
                    crate::models::SystemEventType::NetworkConnected => {
                        crate::daemon::system_events::is_network_connected()
                    }
                    crate::models::SystemEventType::NetworkDisconnected => {
                        !crate::daemon::system_events::is_network_connected()
                    }
                    crate::models::SystemEventType::SystemSleep => {
                        crate::daemon::system_events::is_system_sleeping()
                    }
                    crate::models::SystemEventType::SystemResume => {
                        !crate::daemon::system_events::is_system_sleeping()
                    }
                    crate::models::SystemEventType::DeviceConnected => {
                        crate::daemon::system_events::device_state_known()
                            && crate::daemon::system_events::has_connected_device()
                    }
                    crate::models::SystemEventType::DeviceDisconnected => {
                        crate::daemon::system_events::device_state_known()
                            && !crate::daemon::system_events::has_connected_device()
                    }
                    crate::models::SystemEventType::MonitorOn => {
                        crate::daemon::system_events::is_display_on()
                    }
                    crate::models::SystemEventType::MonitorOff => {
                        !crate::daemon::system_events::is_display_on()
                    }
                }
            }
            RuleCondition::RuleRunning { rule_id, must_be_running } => {
                let target = rule_id.trim();
                !target.is_empty()
                    && crate::daemon::rule_activity::is_running(target) == *must_be_running
            }
            RuleCondition::FileWatcher { path, filter_extension, must_exist } => {
                let p = std::path::Path::new(path);
                let exists = if p.is_dir() {
                    if let Some(ext) = filter_extension {
                        let ext_clean = ext.trim().trim_start_matches('*').trim_start_matches('.').to_lowercase();
                        if ext_clean.is_empty() {
                            p.exists()
                        } else if let Ok(entries) = std::fs::read_dir(p) {
                            entries.filter_map(|e| e.ok()).any(|e| {
                                let ep = e.path();
                                if let Some(file_ext) = ep.extension().and_then(|s| s.to_str()) {
                                    file_ext.eq_ignore_ascii_case(&ext_clean)
                                } else if let Some(file_name) = ep.file_name().and_then(|s| s.to_str()) {
                                    file_name.eq_ignore_ascii_case(&ext_clean)
                                } else {
                                    false
                                }
                            })
                        } else {
                            false
                        }
                    } else {
                        p.exists()
                    }
                } else if let Some(ext) = filter_extension {
                    let ext_clean = ext.trim().trim_start_matches('*').trim_start_matches('.').to_lowercase();
                    if ext_clean.is_empty() {
                        p.exists()
                    } else {
                        let matches_ext = p.extension()
                            .and_then(|s| s.to_str())
                            .map(|e| e.eq_ignore_ascii_case(&ext_clean))
                            .unwrap_or(false);
                        p.exists() && matches_ext
                    }
                } else {
                    p.exists()
                };
                exists == *must_exist
            }
            RuleCondition::ResourceThreshold { process_name, cpu_percent, ram_mb } => {
                let snap = ProcessWatcher::get_detailed_snapshot();
                for proc in &snap {
                    if Self::matches_process(process_name, &proc.name, ProcessMatchMode::Exact) {
                        let mem_mb = proc.memory_bytes / (1024 * 1024);
                        let over_mem = ram_mb.is_none_or(|limit| mem_mb >= limit);
                        let over_cpu = cpu_percent.is_none_or(|limit| proc.cpu_percent >= limit);
                        if over_mem && over_cpu && (ram_mb.is_some() || cpu_percent.is_some()) {
                            return true;
                        }
                    }
                }
                false
            }
        }
    }

    pub fn evaluate_rule_conditions(conditions: &[RuleConditionItem]) -> bool {
        if conditions.is_empty() {
            return true;
        }

        let mut current_group_satisfied = true;
        let mut any_group_satisfied = false;

        for (i, item) in conditions.iter().enumerate() {
            let cond_val = Self::evaluate_condition(&item.condition);
            if i == 0 {
                current_group_satisfied = cond_val;
            } else {
                match item.operator {
                    ConditionOperator::And => {
                        current_group_satisfied = current_group_satisfied && cond_val;
                    }
                    ConditionOperator::Or => {
                        if current_group_satisfied {
                            return true;
                        }
                        any_group_satisfied = any_group_satisfied || current_group_satisfied;
                        current_group_satisfied = cond_val;
                    }
                }
            }
        }

        any_group_satisfied || current_group_satisfied
    }

    pub async fn handle_process_event(&mut self, event: ProcessEvent) {
        let action_str = if event.is_started { "started" } else { "stopped" };
        self.log(
            "INFO",
            "PROCESS",
            &format!("Process {} (PID: {}) {}", event.process_name, event.pid, action_str),
        );

        if event.is_started {
            // Check rules on Process Started
            let rules = self.config.rules.clone();
            for rule in &rules {
                if !rule.is_enabled {
                    continue;
                }

                if let Some(Trigger::Process { process_name, match_mode, single_instance }) = &rule.trigger {
                    if Self::matches_process(process_name, &event.process_name, *match_mode) {
                        if *single_instance {
                            let pids = self.active_rule_pids.entry(rule.id.clone()).or_default();
                            let is_first = pids.is_empty();
                            pids.insert(event.pid);

                            if is_first {
                                if !rule.actions.is_empty() {
                                    let _ = self.try_execute_rule_from(rule, ExecutionOrigin::Process, &event.process_name, event.pid).await;
                                }
                            } else if !rule.actions.is_empty() {
                                self.log(
                                    "INFO",
                                    "RULE",
                                    &format!(
                                        "Rule '{}' skipped (Single Instance): another instance of {} is already running (PID: {})",
                                        rule.name, event.process_name, event.pid
                                    ),
                                );
                            }
                        } else if !rule.actions.is_empty() {
                            let _ = self.try_execute_rule_from(rule, ExecutionOrigin::Process, &event.process_name, event.pid).await;
                        }
                    }
                }
            }
        } else {
            // Process STOPPED:
            let rules = self.config.rules.clone();
            let rules_map: HashMap<String, Rule> = rules.iter().map(|r| (r.id.clone(), r.clone())).collect();

            // Track which single_instance rules have completely terminated (all instances closed)
            let mut single_instance_rules_exited: HashSet<String> = HashSet::new();
            for rule in &rules {
                if let Some(Trigger::Process { process_name, match_mode, single_instance }) = &rule.trigger {
                    if *single_instance && Self::matches_process(process_name, &event.process_name, *match_mode) {
                        if let Some(pids) = self.active_rule_pids.get_mut(&rule.id) {
                            pids.remove(&event.pid);
                            if pids.is_empty() {
                                single_instance_rules_exited.insert(rule.id.clone());
                            }
                        }
                    }
                }
            }
            for rule_id in &single_instance_rules_exited {
                self.active_rule_pids.remove(rule_id);
            }

            // 1. Auto-close any bound processes that were launched for this process (Companion Mode in Rules)
            let to_kill: Vec<BoundProcess> = self
                .active_bound_processes
                .iter()
                .filter(|bp| {
                    let is_match = bp.primary_pid == event.pid
                        || bp.primary_process_name.eq_ignore_ascii_case(&event.process_name);
                    if !is_match {
                        return false;
                    }
                    // If the rule has single_instance enabled, only kill if all instances have exited
                    if let Some(rule) = rules_map.get(&bp.rule_id) {
                        if let Some(Trigger::Process { single_instance: true, .. }) = &rule.trigger {
                            return single_instance_rules_exited.contains(&bp.rule_id);
                        }
                    }
                    true
                })
                .cloned()
                .collect();

            self.active_bound_processes.retain(|bp| {
                !to_kill.iter().any(|k| k.rule_id == bp.rule_id && k.target_executable == bp.target_executable)
            });

            for bp in to_kill {
                let (target_name, action) = Self::bound_satellite_kill(&bp.target_executable);

                self.log(
                    "ACTION",
                    "RULE",
                    &format!(
                        "Auto-closing bound process '{}' (rule: {}) because '{}' terminated",
                        target_name, bp.rule_id, event.process_name
                    ),
                );

                let ctx = HashMap::new();
                let _ = ActionRunner::execute(&action, &ctx).await;
            }

            // 1.1. Auto-close any bound overlays that were launched for this process (Companion Mode in Rules)
            let to_close_overlays = self.executions.take_bound_overlays_where(|bo| {
                let is_match = bo.primary_pid == event.pid
                    || bo.primary_process_name.eq_ignore_ascii_case(&event.process_name);
                if !is_match {
                    return false;
                }
                if let Some(rule) = rules_map.get(&bo.rule_id) {
                    if let Some(Trigger::Process { single_instance: true, .. }) = &rule.trigger {
                        return single_instance_rules_exited.contains(&bo.rule_id);
                    }
                }
                true
            });

            for bo in to_close_overlays {
                self.log(
                    "ACTION",
                    "RULE",
                    &format!(
                        "Auto-closing bound overlay '{}' (rule: {}) because '{}' terminated",
                        bo.overlay_id, bo.rule_id, event.process_name
                    ),
                );
                crate::daemon::overlay_manager::hide_overlay(&bo.overlay_id);
            }

            // 1.2. Also signal overlay_manager to auto-close any standalone overlays with close_with_target == true
            crate::daemon::overlay_manager::check_process_terminated(&event.process_name);

            // 2. Check rules with on_exit_actions
            for rule in &rules {
                if !rule.is_enabled || rule.on_exit_actions.is_empty() {
                    continue;
                }

                if let Some(Trigger::Process { process_name, match_mode, single_instance }) = &rule.trigger {
                    if Self::matches_process(process_name, &event.process_name, *match_mode) {
                        if *single_instance {
                            if single_instance_rules_exited.contains(&rule.id) {
                                let _ = self.try_execute_exit_actions(rule, &event.process_name, event.pid).await;
                            }
                        } else {
                            let _ = self.try_execute_exit_actions(rule, &event.process_name, event.pid).await;
                        }
                    }
                }
            }

            // 3. Auto-clear Discord RPC if configured on process rules
            for rule in &rules {
                if let Some(Trigger::Process { process_name, match_mode, .. }) = &rule.trigger {
                    if Self::matches_process(process_name, &event.process_name, *match_mode) {
                        let has_auto_clear = rule.actions.iter().any(|a| matches!(a, Action::DiscordRpcSet { auto_clear_on_process_exit: true, .. }));
                        if has_auto_clear {
                            let _ = crate::daemon::discord_rpc::clear_presence().await;
                        }
                    }
                }
            }
        }
    }

    pub async fn handle_system_event(&mut self, event: SystemEventType, device_id: Option<&str>) {
        self.drain_action_events();
        let event_name = format!("{:?}", event);
        self.log("INFO", "SYSTEM", &format!("System Event: {}", event_name));

        let rules = self.config.rules.clone();
        for rule in &rules {
            if !rule.is_enabled {
                continue;
            }
            if self.executions.has_recurring_test(&rule.id) {
                continue;
            }

            if let Some(Trigger::SystemEvent {
                event_type,
                device_id_filter,
                ..
            }) = &rule.trigger
            {
                if *event_type == event
                    && Self::device_filter_matches(device_id_filter.as_deref(), device_id)
                {
                    let _ = self.try_execute_rule_from(rule, ExecutionOrigin::SystemEvent, &event_name, 0).await;
                }
            }
        }
    }

    pub async fn tick_schedule(&mut self) {
        self.drain_action_events();
        let now_instant = Instant::now();
        let now_local = chrono::Local::now();
        let rules = self.config.rules.clone();

        for rule in &rules {
            if !rule.is_enabled {
                continue;
            }
            if self.executions.has_recurring_test(&rule.id) {
                continue;
            }

            if let Some(Trigger::Schedule {
                schedule_type,
                interval_sec,
                daily_time,
                days_of_week,
                day_of_month,
                target_datetime,
                ..
            }) = &rule.trigger
            {
                match schedule_type {
                    ScheduleType::Interval => {
                        let interval = *interval_sec;
                        if self.scheduler.interval_due(
                            rule,
                            &mut self.rule_last_triggered,
                            now_instant,
                            now_local,
                            interval,
                            target_datetime.as_deref(),
                        ) {
                            let _ = self.try_execute_rule_from(
                                rule,
                                ExecutionOrigin::Schedule,
                                &format!("Schedule Interval {}s", interval),
                                0,
                            ).await;
                        }
                    }
                    ScheduleType::Daily => {
                        if let Some(t_str) = daily_time {
                            if self.scheduler.daily_due(
                                rule,
                                now_instant,
                                now_local,
                                t_str,
                                days_of_week,
                            ) {
                                let _ = self.try_execute_rule_from(
                                    rule,
                                    ExecutionOrigin::Schedule,
                                    &format!("Daily Schedule {}", t_str),
                                    0,
                                ).await;
                            }
                        }
                    }
                    ScheduleType::Monthly => {
                        if Scheduler::monthly_due(
                            rule,
                            now_local,
                            *day_of_month,
                            daily_time.as_deref(),
                        ) {
                            let target_day = day_of_month.unwrap_or(1).clamp(1, 31);
                            let target_time_str = daily_time.as_deref().unwrap_or("12:00:00");
                            let _ = self.try_execute_rule_from(
                                rule,
                                ExecutionOrigin::Schedule,
                                &format!("Monthly Schedule (Day {}, {})", target_day, target_time_str),
                                0,
                            ).await;
                        }
                    }
                    ScheduleType::OneShot => {
                        if let Some(target) = target_datetime {
                            if Scheduler::oneshot_due(
                                rule,
                                &self.rule_last_triggered,
                                now_local,
                                target,
                            ) {
                                let started = self.try_execute_rule_from(
                                    rule,
                                    ExecutionOrigin::Schedule,
                                    &format!("One-Shot Schedule {}", target),
                                    0,
                                ).await;
                                // Only a run that actually started consumes the one-shot: a rule
                                // that is waiting for its conditions must stay armed.
                                if started.is_some() {
                                    let mut disabled = false;
                                    if let Some(stored) = self.config.rules.iter_mut().find(|r| r.id == rule.id) {
                                        if stored.is_enabled {
                                            stored.is_enabled = false;
                                            disabled = true;
                                        }
                                    }
                                    if disabled {
                                        self.persist_rules();
                                        self.refresh_live_rules();
                                    }
                                }
                            }
                        }
                    }
                    ScheduleType::AppStartup => {
                        // Handled on initial startup
                    }
                    _ => {}
                }
            }
        }

    }

    #[cfg(windows)]
    pub async fn tick_window_focus(&mut self) {
        let has_focus_rules = self.config.rules.iter().any(|r| {
            r.is_enabled && matches!(r.trigger, Some(Trigger::WindowFocus { .. }))
        });
        if !has_focus_rules {
            self.last_foreground_pid = None;
            return;
        }

        use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
        let hwnd = unsafe { GetForegroundWindow() };
        let (fg_pid, is_null) = if hwnd.0.is_null() {
            (0, true)
        } else {
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
            (pid, pid == 0)
        };

        if is_null {
            if let Some(old_id) = self.last_foreground_pid.take() {
                let snap = ProcessWatcher::snapshot_process_map();
                if let Some(old_name) = snap.get(&old_id).cloned() {
                    let rules = self.config.rules.clone();
                    for rule in &rules {
                        if !rule.is_enabled {
                            continue;
                        }
                        if let Some(Trigger::WindowFocus {
                            process_name,
                            on_focus_lost,
                            ..
                        }) = &rule.trigger
                        {
                            if *on_focus_lost
                                && Self::matches_process(
                                    process_name,
                                    &old_name,
                                    ProcessMatchMode::Exact,
                                )
                            {
                                let _ = self
                                    .try_execute_rule(
                                        rule,
                                        &format!("WindowFocus Lost {}", old_name),
                                        old_id,
                                    )
                                    .await;
                            }
                        }
                    }
                }
            }
            return;
        }

        if fg_pid != 0 && self.last_foreground_pid != Some(fg_pid) {
            let old_pid = self.last_foreground_pid;
            self.last_foreground_pid = Some(fg_pid);

            let snap = ProcessWatcher::snapshot_process_map();
            let rules = self.config.rules.clone();
            if let Some(fg_name) = snap.get(&fg_pid).cloned() {
                for rule in &rules {
                    if !rule.is_enabled {
                        continue;
                    }
                    if let Some(Trigger::WindowFocus {
                        process_name,
                        on_focus_gained,
                        ..
                    }) = &rule.trigger
                    {
                        if *on_focus_gained
                            && Self::matches_process(
                                process_name,
                                &fg_name,
                                ProcessMatchMode::Exact,
                            )
                        {
                            let _ = self
                                .try_execute_rule(
                                    rule,
                                    &format!("WindowFocus Gained {}", fg_name),
                                    fg_pid,
                                )
                                .await;
                        }
                    }
                }
            }

            if let Some(old_id) = old_pid {
                if let Some(old_name) = snap.get(&old_id).cloned() {
                    for rule in &rules {
                        if !rule.is_enabled {
                            continue;
                        }
                        if let Some(Trigger::WindowFocus {
                            process_name,
                            on_focus_lost,
                            ..
                        }) = &rule.trigger
                        {
                            if *on_focus_lost
                                && Self::matches_process(
                                    process_name,
                                    &old_name,
                                    ProcessMatchMode::Exact,
                                )
                            {
                                let _ = self
                                    .try_execute_rule(
                                        rule,
                                        &format!("WindowFocus Lost {}", old_name),
                                        old_id,
                                    )
                                    .await;
                            }
                        }
                    }
                }
            }
        }
    }

    #[cfg(not(windows))]
    pub async fn tick_window_focus(&mut self) {}

    pub async fn tick_resource_threshold(&mut self) {
        let rules = self.config.rules.clone();
        let has_resource_rules = rules.iter().any(|r| r.is_enabled && matches!(r.trigger, Some(Trigger::ResourceThreshold { .. })));
        if !has_resource_rules {
            if !self.scheduler.resource_tracking_is_empty() {
                self.scheduler.clear_resource_tracking();
            }
            return;
        }

        let snap = ProcessWatcher::get_detailed_snapshot();
        let active_pids: HashSet<u32> = snap.iter().map(|p| p.pid).collect();
        let active_rule_ids: HashSet<&str> = rules.iter().filter(|r| r.is_enabled).map(|r| r.id.as_str()).collect();
        self.scheduler.retain_resource_tracking(&active_rule_ids, &active_pids);

        for rule in &rules {
            if !rule.is_enabled {
                continue;
            }
            if let Some(Trigger::ResourceThreshold { process_name, ram_mb, cpu_percent, duration_sec }) = &rule.trigger {
                for proc in &snap {
                    if Self::matches_process(process_name, &proc.name, ProcessMatchMode::Exact) {
                        let mem_mb = proc.memory_bytes / (1024 * 1024);
                        // Mirror `evaluate_condition`: every configured limit must hold, and a
                        // CPU-only rule fires on CPU alone.
                        let over_mem = ram_mb.is_none_or(|limit| mem_mb >= limit);
                        let over_cpu = cpu_percent.is_none_or(|limit| proc.cpu_percent >= limit);
                        let over_threshold =
                            over_mem && over_cpu && (ram_mb.is_some() || cpu_percent.is_some());

                        if over_threshold {
                            let now = Instant::now();
                            let start = self.scheduler.resource_since(&rule.id, proc.pid, now);
                            if start.elapsed().as_secs() >= *duration_sec {
                                self.scheduler.reset_resource_dwell(&rule.id, proc.pid, now);
                                let detail = match (ram_mb, cpu_percent) {
                                    (Some(_), Some(_)) => format!("{} MB, {:.0}% CPU", mem_mb, proc.cpu_percent),
                                    (Some(_), None) => format!("{} MB", mem_mb),
                                    (None, Some(_)) => format!("{:.0}% CPU", proc.cpu_percent),
                                    (None, None) => String::new(),
                                };
                                let _ = self.try_execute_rule_from(
                                    rule,
                                    ExecutionOrigin::SystemEvent,
                                    &format!("Resource Threshold: {} ({})", proc.name, detail),
                                    proc.pid,
                                ).await;
                            }
                        } else {
                            self.scheduler.clear_resource_dwell(&rule.id, proc.pid);
                        }
                    }
                }
            }
        }
    }

    /// Counts one started execution against the quota of `rule_id`: rolls a lapsed window
    /// over, anchors a custom window, advances the counter and stamps `last_executed_at`.
    /// Returns the shared quota decision; a spent lifetime limit is applied to `is_enabled`
    /// here, while persisting and stopping the rule stay with the caller.
    fn record_quota_increment(
        &mut self,
        rule_id: &str,
        now: chrono::DateTime<chrono::Local>,
    ) -> QuotaIncrement {
        let mut paused = false;
        let mut decision = QuotaDecision::Allowed;
        if let Some(rule) = self.config.rules.iter_mut().find(|r| r.id == rule_id) {
            let _ = rule.normalize_quota(now);
            rule.begin_quota_window(now);
            rule.execution_count += 1;
            rule.last_executed_at = Some(now.format("%Y-%m-%d %H:%M:%S").to_string());
            decision = QuotaDecision::classify(rule, rule.execution_count, now);
            if matches!(decision, QuotaDecision::LimitReached { .. }) {
                rule.is_enabled = false;
                paused = true;
            }
        }
        self.mark_runtime_dirty();
        QuotaIncrement { decision, paused }
    }

    /// Rolls a lapsed quota period over and reports whether the rule may execute now.
    /// An exhausted lifetime quota pauses the rule (historical behavior); a periodic
    /// quota only waits for the next period, so the rule stays enabled. `announce`
    /// logs the exhausted state for user-invoked runs, which should not fail silently.
    fn quota_allows_execution(&mut self, rule_id: &str, announce: bool) -> bool {
        let now = chrono::Local::now();
        let rolled;
        let decision;
        {
            let Some(rule) = self.config.rules.iter_mut().find(|r| r.id == rule_id) else {
                return true;
            };
            rolled = rule.normalize_quota(now);
            decision = QuotaDecision::classify(rule, rule.quota_used(now), now);
            if matches!(decision, QuotaDecision::LimitReached { .. }) {
                rule.is_enabled = false;
            }
        }
        if rolled {
            self.mark_runtime_dirty();
        }
        if matches!(decision, QuotaDecision::LimitReached { .. }) {
            self.persist_rules();
            self.refresh_live_rules();
        }
        match &decision {
            QuotaDecision::Allowed => {}
            QuotaDecision::PeriodSpent { message } => {
                if announce {
                    self.log("INFO", "RULE", message);
                }
            }
            QuotaDecision::LimitReached { message } => self.log("WARN", "RULE", message),
        }
        matches!(decision, QuotaDecision::Allowed)
    }

    /// Trigger entry point for the modules that only know a context label (hotkeys, webhook,
    /// file watcher, startup). The engine's own dispatch passes [`ExecutionOrigin`] explicitly.
    pub async fn try_execute_rule(
        &mut self,
        rule: &Rule,
        context_name: &str,
        pid: u32,
    ) -> Option<tokio::task::JoinHandle<()>> {
        self.try_execute_rule_with_context(rule, context_name, pid, HashMap::new()).await
    }

    /// Starts `rule` with an explicit origin; this is what the scheduler, process watcher and
    /// user-invoked paths use so manual-mode behaviour is never inferred from a pid.
    pub async fn try_execute_rule_from(
        &mut self,
        rule: &Rule,
        origin: ExecutionOrigin,
        context_name: &str,
        pid: u32,
    ) -> Option<tokio::task::JoinHandle<()>> {
        self.try_execute_rule_with_context_from(rule, origin, context_name, pid, HashMap::new()).await
    }

    pub async fn try_execute_rule_with_context(
        &mut self,
        rule: &Rule,
        context_name: &str,
        pid: u32,
        extra_ctx: HashMap<String, String>,
    ) -> Option<tokio::task::JoinHandle<()>> {
        let origin = ExecutionOrigin::from_context(context_name, pid);
        self.try_execute_rule_with_context_from(rule, origin, context_name, pid, extra_ctx).await
    }

    pub async fn try_execute_rule_with_context_from(
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

    /// Standard pipeline context tokens shared by the daemon and the offline `--run-rule`
    /// runner: `ProcessName`, `ProcessId`, `Timestamp`, `ProcessPath`/`ProcessDir` when a live
    /// pid is known, and the current `WindowTitle`.
    fn pipeline_context(
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

        ctx.extend(extra_ctx);
        ctx
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
            self.active_bound_processes.retain(|bp| bp.rule_id != rule.id);
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
            self.rule_overlay_map.insert(rule.id.clone(), ov_ids);
        }

        let mut ctx = Self::pipeline_context(context_name, pid, extra_ctx);

        let actions = rule.actions.clone();
        let rule_name = rule.name.clone();
        let rule_id = rule.id.clone();
        let stop_on_error = rule.stop_on_error;
        let context_name = context_name.to_string();
        let tx = self.action_tx.clone();

        let execution_id = next_execution_id();
        let total_steps = actions.len();
        let call_chain = call.as_ref().map(|c| c.chain.clone()).unwrap_or_default();
        let completion = call.map(|c| c.completion);
        ctx.insert(rule_call::EXECUTION_CTX_KEY.to_string(), execution_id.to_string());

        let increment = self.record_quota_increment(&rule.id, chrono::Local::now());
        if increment.paused {
            self.persist_rules();
        }
        if let Some(message) = increment.decision.message() {
            let level = if increment.paused { "WARN" } else { "INFO" };
            self.log(level, "RULE", message);
        }

        self.analytics.total_executions += 1;

        crate::daemon::rule_activity::mark_started(&rule_id);
        let _ = tx.send(ActionCompletionEvent::RuleExecutionStarted {
            execution_id,
            rule_id: rule_id.clone(),
            step_index: 0,
            total_steps,
            is_exit: false,
            call_chain,
        });

        let execution_rule_id = rule_id.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            // Built before the task is spawned and moved into it, so an abort before the first
            // poll drops the future together with the guard: the caller is always notified.
            let guard = ExecGuard {
                id: execution_id,
                rule_id: rule_id.clone(),
                is_manual,
                tx: tx.clone(),
                completion,
            };

            let join_handle = handle.spawn(async move {
                let _guard = guard;
                let mut ctx = ctx;
                let run = PipelineRun {
                    actions: &actions,
                    execution_id,
                    rule_id: &rule_id,
                    rule_name: &rule_name,
                    context_name: &context_name,
                    pid,
                    is_exit: false,
                    stop_on_error,
                    bind_companions: true,
                    label: "Pipeline",
                    error_label: "Action",
                    warn_on_error_halt: true,
                };
                run_action_pipeline(run, &mut ctx, &tx).await;
            });
            self.executions.register_abort_handle(execution_id, execution_rule_id, join_handle.abort_handle());
            Ok(Some(join_handle))
        } else {
            crate::daemon::rule_activity::mark_finished(&rule_id);
            let _ = tx.send(ActionCompletionEvent::RuleExecutionFinished {
                execution_id,
                _rule_id: rule_id,
                is_manual,
            });
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
        let tx = self.action_tx.clone();

        let execution_id = next_execution_id();
        let total_steps = actions.len();
        ctx.insert(rule_call::EXECUTION_CTX_KEY.to_string(), execution_id.to_string());

        crate::daemon::rule_activity::mark_started(&rule_id);
        let _ = tx.send(ActionCompletionEvent::RuleExecutionStarted {
            execution_id,
            rule_id: rule_id.clone(),
            step_index: 0,
            total_steps,
            is_exit: true,
            call_chain: Vec::new(),
        });

        let execution_rule_id = rule_id.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let guard = ExecGuard {
                id: execution_id,
                rule_id: rule_id.clone(),
                is_manual: false,
                tx: tx.clone(),
                completion: None,
            };
            let join_handle = handle.spawn(async move {
                let _guard = guard;
                let mut ctx = ctx;
                let run = PipelineRun {
                    actions: &actions,
                    execution_id,
                    rule_id: &rule_id,
                    rule_name: &rule_name,
                    context_name: "",
                    pid: 0,
                    is_exit: true,
                    stop_on_error,
                    bind_companions: false,
                    label: "Exit pipeline",
                    error_label: "Exit action",
                    warn_on_error_halt: false,
                };
                run_action_pipeline(run, &mut ctx, &tx).await;
            });
            self.executions.register_abort_handle(execution_id, execution_rule_id, join_handle.abort_handle());
            Some(join_handle)
        } else {
            crate::daemon::rule_activity::mark_finished(&rule_id);
            let _ = tx.send(ActionCompletionEvent::RuleExecutionFinished {
                execution_id,
                _rule_id: rule_id,
                is_manual: false,
            });
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

        self.active_bound_processes.retain(|bp| bp.rule_id != rule.id);
        self.executions.remove_rule_bound_overlays(&rule.id);

        let ov_ids = Self::extract_rule_overlay_ids(rule);
        if !ov_ids.is_empty() {
            self.rule_overlay_map.insert(rule.id.clone(), ov_ids);
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
        self.try_execute_rule_with_context_from(
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
        let tx = self.action_tx.clone();
        let live_rules = std::sync::Arc::clone(&self.live_rules);
        let now = chrono::Local::now();
        let mut current_runs = self.config.rules.iter().find(|r| r.id == rule.id)
            .map(|stored| stored.quota_used(now))
            .unwrap_or_else(|| rule.quota_used(now));

        let execution_id = next_execution_id();
        let total_steps = rule.actions.len();

        crate::daemon::rule_activity::mark_started(&rule_id);
        let _ = tx.send(ActionCompletionEvent::RuleExecutionStarted {
            execution_id,
            rule_id: rule_id.clone(),
            step_index: 0,
            total_steps,
            is_exit: false,
            call_chain: Vec::new(),
        });

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let task_rule_id = rule_id.clone();
            let guard = ExecGuard {
                id: execution_id,
                rule_id: rule_id.clone(),
                is_manual: false,
                tx: tx.clone(),
                completion: None,
            };
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
            crate::daemon::rule_activity::mark_finished(&rule_id);
            let _ = tx.send(ActionCompletionEvent::RuleExecutionFinished {
                execution_id,
                _rule_id: rule_id,
                is_manual: false,
            });
            None
        }
    }

    pub async fn handle_request(&mut self, req: IpcRequest) -> IpcResponse {
        self.drain_action_events();
        match req {
            IpcRequest::Ping => IpcResponse::Pong,

            IpcRequest::GetState => {
                let state = self.get_current_state();
                IpcResponse::State(state)
            }

            // Narrow answer for the standalone update-progress overlay: two updater fields, no
            // `DaemonState` projection (config and logs stay where they are).
            IpcRequest::GetUpdateProgress => {
                let (progress, status) = self.update_progress_snapshot();
                IpcResponse::UpdateProgress { progress, status }
            }

            IpcRequest::SaveSettings(settings) => {
                let tray_changed = self.config.tray_icon_path != settings.tray_icon_path;
                let new_tray_path = settings.tray_icon_path.clone();
                // Window geometry is owned by the daemon through UpdateWindowState; a
                // settings snapshot from the GUI may be older, so it never overrides it.
                let mut settings = settings;
                settings.window = self.config.window.clone();
                self.config.apply_settings(settings);
                self.persist_settings();
                if tray_changed {
                    crate::daemon::tray::update_tray_icon(new_tray_path);
                }
                crate::daemon::tray::set_tray_visible(!self.config.hide_tray_icon);
                self.log("INFO", "CONFIG", "Settings updated and saved");
                IpcResponse::Ok
            }

            IpcRequest::SaveRule(rule) => self.ipc_save_rule(rule),

            IpcRequest::ReorderRules(order) => {
                let mut remaining = std::mem::take(&mut self.config.rules);
                let mut ordered = Vec::with_capacity(remaining.len());
                for id in order {
                    if let Some(position) = remaining.iter().position(|rule| rule.id == id) {
                        ordered.push(remaining.remove(position));
                    }
                }
                ordered.append(&mut remaining);
                self.config.rules = ordered;
                let saved = self.persist_rules();
                self.refresh_live_rules();
                if !saved {
                    return Self::persist_failure("rules");
                }
                IpcResponse::Ok
            }

            IpcRequest::ResetRule(id) => {
                let Some(rule) = self.config.rules.iter_mut().find(|r| r.id == id) else {
                    return IpcResponse::Error(format!("rule \"{id}\" not found"));
                };
                rule.clear_quota_runtime();
                rule.is_enabled = true;
                let saved = self.persist_rules();
                self.refresh_live_rules();
                self.mark_runtime_dirty();
                self.persist_runtime_now();
                if let Some(h) = &self.hotkey_handle {
                    h.reload(&self.config.rules);
                }
                if !saved {
                    return Self::persist_failure("rules");
                }
                self.log("INFO", "RULE", &format!("Reset rule '{id}': counter cleared, rule enabled"));
                IpcResponse::Ok
            }

            IpcRequest::DeleteRule(id) => {
                // The rule is going away: aborting (not pausing) is the only thing left to do.
                self.abort_rule_executions(&id);
                if let Some(rule) = self.config.rules.iter().find(|r| r.id == id) {
                    Self::hide_rule_overlays(rule);
                }
                self.config.rules.retain(|r| r.id != id);
                self.rule_last_triggered.remove(&id);
                self.scheduler.forget_rule(&id);
                let saved = self.persist_rules();
                self.refresh_live_rules();
                self.mark_runtime_dirty();
                self.persist_runtime_now();
                if let Some(h) = &self.hotkey_handle {
                    h.reload(&self.config.rules);
                }
                if !saved {
                    return Self::persist_failure("rules");
                }
                IpcResponse::Ok
            }

            IpcRequest::ToggleRule { id, enabled } => {
                if !enabled {
                    self.abort_rule_executions(&id);
                }
                if let Some(rule) = self.config.rules.iter_mut().find(|r| r.id == id) {
                    rule.is_enabled = enabled;
                }
                let saved = self.persist_rules();
                self.refresh_live_rules();
                if let Some(h) = &self.hotkey_handle {
                    h.reload(&self.config.rules);
                }
                if !saved {
                    return Self::persist_failure("rules");
                }
                IpcResponse::Ok
            }

            IpcRequest::TestRunRule { id, rule } => {
                let target_rule = if let Some(r) = rule {
                    *r
                } else if let Some(r) = self.config.rules.iter().find(|r| r.id == id).cloned() {
                    r
                } else {
                    return IpcResponse::Error("Rule not found".into());
                };
                let _ = self.emulate_rule_trigger(&target_rule).await;
                self.drain_action_events();
                IpcResponse::Ok
            }

            IpcRequest::StopRuleExecution(id) => {
                self.stop_rule_execution(&id);
                IpcResponse::Ok
            }

            IpcRequest::GetProcesses => {
                let list = ProcessWatcher::get_detailed_snapshot();
                IpcResponse::Processes(list)
            }

            IpcRequest::KillProcess { pid, force } => {
                let mut cmd = std::process::Command::new("taskkill");
                if force {
                    cmd.arg("/F");
                }
                cmd.args(["/PID", &pid.to_string()]);
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    cmd.creation_flags(0x08000000);
                }
                let _ = cmd.spawn();
                IpcResponse::Ok
            }

            IpcRequest::SetAutorun(enable) => {
                match crate::autorun::set_autorun(crate::autorun::APP_NAME, enable) {
                    Ok(()) => {
                        self.log("INFO", "CONFIG", &format!("Autorun set to: {enable}"));
                        IpcResponse::Ok
                    }
                    Err(err) => {
                        self.log("ERROR", "CONFIG", &format!("Failed to set autorun: {err}"));
                        IpcResponse::Error(err)
                    }
                }
            }

            IpcRequest::UpdateWindowState(ws) => {
                self.config.window = Some(ws);
                self.persist_settings();
                IpcResponse::Ok
            }

            IpcRequest::ClearLogs => {
                self.logs.clear();
                IpcResponse::Ok
            }

            IpcRequest::ImportRules(imported) => self.ipc_import_rules(imported),

            IpcRequest::ImportShare { payload, enable } => self.ipc_import_share(*payload, enable),

            IpcRequest::SyncRules(incoming) => self.ipc_sync_rules(incoming),

            // The only production path for `TestStep` is `daemon::ipc_server::dispatch`, which
            // runs the action *before* taking the engine lock: a wait or a script executed here
            // would stall the scheduler and every other IPC request for the action's duration.
            // The arm stays only to keep the match exhaustive; reaching it means a caller
            // bypassed the dispatcher, so it must never run the action.
            IpcRequest::TestStep { .. } => IpcResponse::Error(
                "TestStep must be dispatched before the engine lock is taken".to_string(),
            ),

            IpcRequest::SaveOverlay(overlay) => {
                if let Some(existing) = self.config.overlays.iter_mut().find(|o| o.id == overlay.id) {
                    *existing = overlay;
                } else {
                    self.config.overlays.push(overlay);
                }
                self.persist_library();
                IpcResponse::Ok
            }

            IpcRequest::DeleteOverlay(id) => {
                crate::daemon::overlay_manager::hide_overlay(&id);
                self.config.overlays.retain(|o| o.id != id && !o.name.eq_ignore_ascii_case(&id));
                self.persist_library();
                IpcResponse::Ok
            }

            IpcRequest::SaveMacroPreset(preset) => {
                if let Some(existing) = self.config.macro_presets.iter_mut().find(|p| p.id == preset.id) {
                    *existing = preset;
                } else {
                    self.config.macro_presets.push(preset);
                }
                self.persist_library();
                IpcResponse::Ok
            }

            IpcRequest::DeleteMacroPreset(id) => {
                self.config.macro_presets.retain(|p| p.id != id && !p.name.eq_ignore_ascii_case(&id));
                self.persist_library();
                IpcResponse::Ok
            }

            IpcRequest::ShowOverlayPreview { overlay, calibration_mode } => {
                crate::daemon::overlay_manager::show_overlay(&overlay, calibration_mode, None);
                IpcResponse::Ok
            }

            IpcRequest::HideOverlayPreview(id) => {
                crate::daemon::overlay_manager::hide_overlay(&id);
                IpcResponse::Ok
            }

            IpcRequest::GetOverlayPosition(id) => {
                let pos = crate::daemon::overlay_manager::get_calibrated_position(&id);
                IpcResponse::OverlayPosition(pos)
            }

            IpcRequest::UpdateOverlayPosition { id, x, y } => {
                crate::daemon::overlay_manager::save_calibrated_position(&id, x, y);
                let monitors = crate::daemon::overlay_manager::get_system_monitors();
                let mut library_changed = false;
                let mut rules_changed = false;
                if let Some(ov) = self.config.overlays.iter_mut().find(|o| o.id == id) {
                    let mon = ov.monitor_index
                        .and_then(|idx| monitors.iter().find(|m| m.index == idx))
                        .or_else(|| monitors.iter().find(|m| m.is_primary));
                    let (mon_x, mon_y) = mon.map(|m| (m.x as f32, m.y as f32)).unwrap_or((0.0, 0.0));
                    ov.rect.x = x - mon_x;
                    ov.rect.y = y - mon_y;
                    library_changed = true;
                }
                for rule in &mut self.config.rules {
                    for action in &mut rule.actions {
                        if let Action::Overlay { overlay_id, custom_overlay: Some(ov), .. } = action {
                            if overlay_id == &id || ov.id == id {
                                let mon = ov.monitor_index
                                    .and_then(|idx| monitors.iter().find(|m| m.index == idx))
                                    .or_else(|| monitors.iter().find(|m| m.is_primary));
                                let (mon_x, mon_y) = mon.map(|m| (m.x as f32, m.y as f32)).unwrap_or((0.0, 0.0));
                                ov.rect.x = x - mon_x;
                                ov.rect.y = y - mon_y;
                                rules_changed = true;
                            }
                        }
                    }
                }
                self.mark_overlay_position_dirty(library_changed, rules_changed);
                IpcResponse::Ok
            }

            IpcRequest::SetAvailableUpdate(info) => {
                self.available_update = info.clone();
                crate::daemon::tray::set_available_update(info);
                IpcResponse::Ok
            }

            IpcRequest::AcknowledgeOpenUpdateModal => {
                self.pending_open_update_modal = false;
                IpcResponse::Ok
            }

            IpcRequest::TriggerUpdate => {
                crate::daemon::tray::trigger_update_flow(false);
                IpcResponse::Ok
            }

            IpcRequest::StartUpdate => {
                crate::daemon::tray::start_update_download(false);
                IpcResponse::Ok
            }

            IpcRequest::AcknowledgeAutoStartUpdate => {
                self.pending_auto_start_update = false;
                IpcResponse::Ok
            }

            IpcRequest::ShutdownDaemon => {
                // Hand the exit to the daemon main thread instead of ending the process here: the
                // coordinated shutdown runs the On App Close rules and flushes the coalesced
                // runtime counters and overlay positions before the process ends.
                crate::daemon::actions::release_all_simulated_inputs();
                crate::daemon::overlay_manager::hide_all();
                crate::daemon::request_shutdown();
                IpcResponse::Ok
            }
        }
    }

    /// Executes one `TestStep` action for the GUI. It deliberately takes no `&self`: the IPC
    /// server runs it before taking the engine lock, so a wait or a script cannot stall the
    /// scheduler and every other IPC request.
    pub async fn execute_test_step(
        action: &Action,
        context: &HashMap<String, String>,
    ) -> Result<crate::daemon::actions::ActionOutcome, String> {
        ActionRunner::execute(action, context).await
    }

    /// Records a step-test outcome. Script errors embed stdout/stderr, so they stay in the in-app
    /// log while the daily file log gets a redacted line.
    pub fn record_test_step(
        &mut self,
        outcome: Result<crate::daemon::actions::ActionOutcome, String>,
    ) -> IpcResponse {
        let result = match outcome {
            Ok(crate::daemon::actions::ActionOutcome::Continue(_)) => {
                self.log("INFO", "TEST", "Step test executed successfully");
                Ok(())
            }
            Ok(crate::daemon::actions::ActionOutcome::AbortPipeline { reason }) => {
                self.log("INFO", "TEST", &format!("Pipeline check halted: {reason}"));
                Ok(())
            }
            Err(err) => {
                self.log_with_file_message(
                    "ERROR",
                    "TEST",
                    &format!("Step test failed: {err}"),
                    Some("Step test failed (details omitted from the file log)"),
                );
                Err(err)
            }
        };
        IpcResponse::TestStepResult(result)
    }

    /// Applies a GUI draft of one rule. Runtime counters stay with the daemon, and a replaced
    /// definition may not keep an old recurring test loop running. The shared validator runs
    /// before anything is applied, so a rejected draft leaves the stored set untouched.
    fn ipc_save_rule(&mut self, mut rule: Rule) -> IpcResponse {
        let errors = crate::cli::validate_rules(std::slice::from_ref(&rule));
        if !errors.is_empty() {
            return IpcResponse::Error(format!(
                "rejected invalid rule: {}",
                errors.join("; ")
            ));
        }
        let rule_id = rule.id.clone();
        let mut runtime_reset = false;
        if let Some(existing) = self.config.rules.iter_mut().find(|r| r.id == rule.id) {
            // Counters belong to the daemon: the GUI edits drafts, it does not
            // own execution history.
            runtime_reset = adopt_rule_runtime(&mut rule, existing);
            *existing = rule;
        } else {
            // The GUI draft may carry counters copied from another rule; a newly
            // created rule starts with a clean runtime window.
            rule.clear_quota_runtime();
            self.config.rules.push(rule);
        }
        self.abort_recurring_test(&rule_id);
        let saved = self.persist_rules();
        self.refresh_live_rules();
        self.flush_quota_reset(runtime_reset);
        if let Some(h) = &self.hotkey_handle {
            h.reload(&self.config.rules);
        }
        if !saved {
            return Self::persist_failure("rules");
        }
        IpcResponse::Ok
    }

    /// Merges a validated import file into the rule set.
    fn ipc_import_rules(&mut self, imported: Vec<Rule>) -> IpcResponse {
        let errors = crate::cli::validate_rules(&imported);
        if !errors.is_empty() {
            return IpcResponse::Error(format!(
                "rejected invalid rules: {}",
                errors.join("; ")
            ));
        }
        let mut count = 0;
        let mut runtime_reset = false;
        for mut rule in imported {
            if let Some(existing) = self.config.rules.iter_mut().find(|r| r.id == rule.id) {
                // Imported definitions never carry execution history: counters stay
                // with the daemon and the rule keeps running where it left off.
                runtime_reset |= adopt_rule_runtime(&mut rule, existing);
                *existing = rule;
            } else {
                // A new rule starts with a clean quota window even if the file
                // carried counter fields; runtime.json owns them.
                rule.clear_quota_runtime();
                self.config.rules.push(rule);
            }
            count += 1;
        }
        let saved = self.persist_rules();
        self.refresh_live_rules();
        self.flush_quota_reset(runtime_reset);
        if let Some(h) = &self.hotkey_handle {
            h.reload(&self.config.rules);
        }
        if !saved {
            return Self::persist_failure("rules");
        }
        self.log("INFO", "CONFIG", &format!("Imported {count} automations"));
        IpcResponse::Ok
    }

    /// Adds a decoded share code together with the library presets and overlays it references.
    fn ipc_import_share(
        &mut self,
        payload: crate::sharing::SharePayload,
        enable: bool,
    ) -> IpcResponse {
        let prepared = match crate::sharing::prepare_import(
            payload,
            &self.config.rules,
            &self.config.overlays,
            &self.config.macro_presets,
            enable,
        ) {
            Ok(prepared) => prepared,
            Err(err) => return IpcResponse::Error(crate::sharing::error_text(&err).to_string()),
        };
        let errors = crate::cli::validate_rules(&prepared.rules);
        if !errors.is_empty() {
            return IpcResponse::Error(format!(
                "share import rejected: {}",
                errors.join("; ")
            ));
        }
        let summary = crate::sharing::ShareImportSummary {
            imported: prepared.rules.len(),
            skipped_duplicates: prepared.skipped_duplicates,
            overlays: prepared.overlays.len(),
            macro_presets: prepared.macro_presets.len(),
            enabled: enable,
        };
        if !prepared.has_new_content {
            return IpcResponse::ShareImportResult(summary);
        }
        for overlay in prepared.overlays {
            self.config.overlays.push(overlay);
        }
        for preset in prepared.macro_presets {
            self.config.macro_presets.push(preset);
        }
        for rule in prepared.rules {
            self.config.rules.push(rule);
        }
        let library_saved = if summary.overlays > 0 || summary.macro_presets > 0 {
            self.persist_library()
        } else {
            true
        };
        let rules_saved = self.persist_rules();
        self.refresh_live_rules();
        self.mark_runtime_dirty();
        if let Some(h) = &self.hotkey_handle {
            h.reload(&self.config.rules);
        }
        if !rules_saved || !library_saved {
            return Self::persist_failure("the imported rules and presets");
        }
        self.log(
            "INFO",
            "CONFIG",
            &format!(
                "Imported share code: {} automation(s), {} duplicate(s) skipped, {} overlay(s), {} preset(s)",
                summary.imported,
                summary.skipped_duplicates,
                summary.overlays,
                summary.macro_presets
            ),
        );
        IpcResponse::ShareImportResult(summary)
    }

    /// Declarative replace: the file is the full desired rule set. Counters of surviving rules
    /// stay with the daemon; removed rules lose theirs on the runtime flush below. One persist +
    /// one hotkey reload for the whole sync.
    fn ipc_sync_rules(&mut self, incoming: Vec<Rule>) -> IpcResponse {
        // The file is the whole desired set, so every rule and the duplicate-id check are
        // validated together; a rejected sync leaves the stored rules untouched.
        let errors = crate::cli::validate_rules(&incoming);
        if !errors.is_empty() {
            return IpcResponse::Error(format!(
                "rejected invalid rule set: {}",
                errors.join("; ")
            ));
        }
        let mut synced = Vec::with_capacity(incoming.len());
        let mut runtime_reset = false;
        for mut rule in incoming {
            if let Some(existing) = self.config.rules.iter().find(|r| r.id == rule.id) {
                runtime_reset |= adopt_rule_runtime(&mut rule, existing);
            } else {
                // Sync files define rules, not execution history.
                rule.clear_quota_runtime();
            }
            synced.push(rule);
        }
        // Stop what the replaced set still has running. A surviving rule may get a new pipeline
        // under its running executions, and a vanished rule has nothing left to own its pipelines
        // or bound companions.
        let incoming_ids: HashSet<&str> = synced.iter().map(|r| r.id.as_str()).collect();
        let removed: Vec<String> = self.config.rules.iter()
            .filter(|r| !incoming_ids.contains(r.id.as_str()))
            .map(|r| r.id.clone())
            .collect();
        let replaced: Vec<String> = self.config.rules.iter()
            .filter(|r| incoming_ids.contains(r.id.as_str()))
            .map(|r| r.id.clone())
            .collect();
        for id in replaced.iter().chain(removed.iter()) {
            self.abort_recurring_test(id);
        }
        for id in &removed {
            self.abort_rule_executions(id);
            self.rule_last_triggered.remove(id);
            self.scheduler.forget_rule(id);
        }

        let count = synced.len();
        self.config.rules = synced;
        let saved = self.persist_rules();
        self.refresh_live_rules();
        self.flush_quota_reset(runtime_reset);
        self.mark_runtime_dirty();
        self.persist_runtime_now();
        if let Some(h) = &self.hotkey_handle {
            h.reload(&self.config.rules);
        }
        if !saved {
            return Self::persist_failure("rules");
        }
        self.log("INFO", "CONFIG", &format!("Synchronized {count} rules"));
        IpcResponse::Ok
    }

    pub async fn execute_shutdown_rules(&mut self) {
        let rules = self.config.rules.clone();
        let mut handles = Vec::new();
        for rule in &rules {
            if rule.is_enabled {
                if let Some(Trigger::Schedule {
                    schedule_type: ScheduleType::AppShutdown,
                    ..
                }) = &rule.trigger
                {
                    if let Some(handle) = self.try_execute_rule_from(rule, ExecutionOrigin::Schedule, "App Shutdown", 0).await {
                        handles.push(handle);
                    }
                }
            }
        }
        for handle in handles {
            let _ = handle.await;
        }
        self.drain_action_events();
    }

    /// Builds the IPC snapshot the GUI renders. The two state mutations — draining completion
    /// events and rolling lapsed quota periods over — stay here; the aggregation itself lives
    /// in [`StateProjector`].
    pub fn get_current_state(&mut self) -> DaemonState {
        self.drain_action_events();

        // Roll lapsed quota periods over before the snapshot is built, so the GUI never
        // shows a spent counter that has already reset with the calendar.
        let now = chrono::Local::now();
        let mut quota_rolled = false;
        for rule in &mut self.config.rules {
            quota_rolled |= rule.normalize_quota(now);
        }
        if quota_rolled {
            self.mark_runtime_dirty();
        }

        StateProjector::new(self).project()
    }

    /// The updater fields the update-progress overlay needs. `GetUpdateProgress` answers from
    /// this pair without projecting a [`DaemonState`]; the full snapshot projects the same pair,
    /// so the two answers cannot disagree.
    fn update_progress_snapshot(&self) -> (Option<f32>, Option<String>) {
        (
            self.is_downloading_update.then_some(self.update_download_progress),
            self.update_status.clone(),
        )
    }
}

/// Which gates an offline rule run applies. The daemon's manual run (`emulate_rule_trigger`,
/// behind the GUI Run/Test button and `--run-rule` over IPC) enforces both; making them an
/// explicit parameter keeps the shared pipeline honest about what each tier checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleRunPolicy {
    /// Honour `max_executions`/`quota_period` against the counters carried by the rule.
    pub enforce_quota: bool,
    /// Honour the rule's preconditions.
    pub enforce_conditions: bool,
}

impl RuleRunPolicy {
    /// The daemon's manual-run gate: quota and preconditions apply; the cooldown and
    /// `is_enabled` do not, so a disabled rule can still be tested from the GUI or the CLI.
    pub const MANUAL: Self = Self {
        enforce_quota: true,
        enforce_conditions: true,
    };
}

impl Default for RuleRunPolicy {
    fn default() -> Self {
        Self::MANUAL
    }
}

/// Result of one offline `--run-rule` execution.
pub struct OfflineRunResult {
    /// Why the rule did not start (quota, conditions, empty pipeline); `None` when it ran.
    pub skipped: Option<String>,
    /// Final pipeline context: standard tokens plus `PrevOutput`/`StepN.Output`.
    pub context: HashMap<String, String>,
    /// The pipeline halted early (a graceful abort, or an error with `stop_on_error`).
    pub halted: bool,
    /// Action failures, in order: 1-based step number and message.
    pub failures: Vec<(usize, String)>,
    /// Graceful halt: 1-based step number and abort reason.
    pub abort: Option<(usize, String)>,
}

/// Records the shared step-loop callbacks for the offline runner.
#[derive(Default)]
struct OfflineRunObserver {
    failures: Vec<(usize, String)>,
    abort: Option<(usize, String)>,
}

impl PipelineObserver for OfflineRunObserver {
    fn step_started(&mut self, _step_index: usize, _total_steps: usize) {}

    fn step_completed(&mut self, _step_index: usize, _action: &Action) {}

    fn step_failed(&mut self, step_index: usize, error: &str, _stopping: bool) {
        self.failures.push((step_index + 1, error.to_string()));
    }

    fn pipeline_halted(&mut self, step_index: usize, abort_reason: Option<&str>) {
        if let Some(reason) = abort_reason {
            self.abort = Some((step_index + 1, reason.to_string()));
        }
    }
}

/// Runs one rule without a daemon, the way `--run-rule` does when no daemon is reachable.
///
/// It mirrors the daemon's manual run: the quota and preconditions are enforced, the cooldown
/// and `is_enabled` are not (a manual run may test a disabled rule, exactly like the GUI's
/// Run/Test button), and the pipeline goes through the very same [`run_step_loop`], so
/// `PrevOutput`, `StepN.Output`, `AbortPipeline` and `stop_on_error` behave identically.
///
/// Deliberate offline differences, all consequences of there being no daemon: nothing is
/// persisted (analytics, runtime counters and a lifetime-quota auto-pause are not written to
/// disk), `on_exit_actions` do not run (the daemon runs them when the bound process exits, not
/// as part of a manual run), the rule-to-rule channel has no dispatcher, and only the tokens
/// available in this process are provided (`ProcessId` stays `0`; no live pid or process path).
pub async fn run_rule_offline(rule: &Rule, policy: RuleRunPolicy) -> OfflineRunResult {
    let skipped = |reason: String| OfflineRunResult {
        skipped: Some(reason),
        context: HashMap::new(),
        halted: false,
        failures: Vec::new(),
        abort: None,
    };

    if policy.enforce_quota {
        if let Some(limit) = rule.max_executions {
            let now = chrono::Local::now();
            let mut counted = rule.clone();
            let _ = counted.normalize_quota(now);
            let used = counted.quota_used(now);
            if used >= limit {
                let decision = QuotaDecision::classify(&counted, used, now);
                return skipped(
                    decision
                        .message()
                        .unwrap_or("the execution quota is exhausted")
                        .to_string(),
                );
            }
        }
    }

    if policy.enforce_conditions
        && !rule.conditions.is_empty()
        && !DaemonEngine::evaluate_rule_conditions(&rule.conditions)
    {
        return skipped(format!(
            "Rule '{}' was not started: conditions were not met.",
            rule.name
        ));
    }

    if rule.actions.is_empty() {
        return skipped(format!(
            "Rule '{}' has an empty action pipeline.",
            rule.name
        ));
    }

    // The daemon's manual run adds the configured process name for process-like triggers; its
    // live pid lookup stays daemon-side, so the offline run keeps `ProcessId` at 0.
    let mut extra_ctx = HashMap::new();
    if let Some(Trigger::Process { process_name, .. })
    | Some(Trigger::WindowFocus { process_name, .. })
    | Some(Trigger::ResourceThreshold { process_name, .. }) = &rule.trigger
    {
        extra_ctx.insert("ProcessName".to_string(), process_name.clone());
    }
    let mut context = DaemonEngine::pipeline_context("Manual Run", 0, extra_ctx);

    let mut observer = OfflineRunObserver::default();
    let halted = run_step_loop(&rule.actions, &mut context, rule.stop_on_error, &mut observer).await;

    OfflineRunResult {
        skipped: None,
        context,
        halted,
        failures: observer.failures,
        abort: observer.abort,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};

    #[tokio::test]
    async fn test_bound_overlay_companion_lifecycle() {
        let mut config = AppConfig::default();
        let overlay_id = "companion_overlay_hud".to_string();
        let rule = Rule {
            id: "game_with_overlay".into(),
            name: "Game with Overlay HUD".into(),
            description: "".into(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            trigger: Some(Trigger::Process {
                process_name: "mygame.exe".into(),
                match_mode: ProcessMatchMode::Exact,
                single_instance: false,
            }),
            actions: vec![
                Action::Overlay {
                    overlay_id: overlay_id.clone(),
                    custom_overlay: None,
                    operation: crate::models::OverlayOperation::Show,
                    override_text: None,
                    duration_ms: None,
                    close_on_exit: true,
                },
            ],
            ..Default::default()
        };
        config.rules.push(rule.clone());

        let mut engine = DaemonEngine::new(config);

        // 1. Process starts -> triggers rule execution
        engine.handle_process_event(ProcessEvent {
            pid: 4321,
            process_name: "mygame.exe".into(),
            is_started: true,
        }).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        engine.drain_action_events();

        // Overlay is registered as bound companion
        assert_eq!(engine.executions.bound_overlays().len(), 1);
        assert_eq!(engine.executions.bound_overlays()[0].overlay_id, overlay_id);
        assert_eq!(engine.executions.bound_overlays()[0].primary_pid, 4321);

        // State reflects companion supervisor execution
        let state = engine.get_current_state();
        assert!(state.active_rule_executions.contains_key("game_with_overlay"));

        // 2. Process exits -> auto-closes bound overlay
        engine.handle_process_event(ProcessEvent {
            pid: 4321,
            process_name: "mygame.exe".into(),
            is_started: false,
        }).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        engine.drain_action_events();

        assert_eq!(engine.executions.bound_overlays().len(), 0, "Bound overlay must be removed after primary process exit");
        assert!(engine.logs.iter().any(|l| l.message.contains("Auto-closing bound overlay")));
    }

    #[test]
    fn test_log_limit_500() {
        let mut engine = DaemonEngine::new(AppConfig::default());
        assert_eq!(engine.logs.len(), 0);

        for i in 1..=600 {
            engine.log("INFO", "TEST", &format!("Log message {}", i));
        }

        assert_eq!(engine.logs.len(), 500);
        assert_eq!(engine.logs[0].message, "Log message 101");
        assert_eq!(engine.logs[499].message, "Log message 600");
    }

    #[test]
    fn test_sensitive_action_log_keeps_details_in_app_log() {
        let mut engine = DaemonEngine::new(AppConfig::default());
        engine
            .action_tx
            .send(ActionCompletionEvent::SensitiveLog {
                level: "ERROR".to_string(),
                category: "ACTION".to_string(),
                message: "Action failed in 'Rule': script output with SECRET".to_string(),
                file_message: "Action failed in 'Rule' at step 2 (details omitted from the file log)"
                    .to_string(),
            })
            .expect("engine event channel is open");
        engine.drain_action_events();

        let entry = engine.logs.last().expect("action log entry");
        assert_eq!(entry.level, "ERROR");
        assert_eq!(entry.category, "ACTION");
        assert!(
            entry.message.contains("SECRET"),
            "the in-app log must keep the full action error"
        );
    }

    #[test]
    fn test_log_deduplication_aggregation() {
        let mut engine = DaemonEngine::new(AppConfig::default());
        assert_eq!(engine.logs.len(), 0);

        engine.log("WARN", "RULE", "Rule 'Test' conditions were not met");
        assert_eq!(engine.logs.len(), 1);
        assert_eq!(engine.logs[0].message, "Rule 'Test' conditions were not met");

        // Consecutive repeat increments to (x2)
        engine.log("WARN", "RULE", "Rule 'Test' conditions were not met");
        assert_eq!(engine.logs.len(), 1);
        assert_eq!(engine.logs[0].message, "Rule 'Test' conditions were not met (x2)");

        // Consecutive repeat increments to (x3)
        engine.log("WARN", "RULE", "Rule 'Test' conditions were not met");
        assert_eq!(engine.logs.len(), 1);
        assert_eq!(engine.logs[0].message, "Rule 'Test' conditions were not met (x3)");

        // Different message creates a new log line
        engine.log("INFO", "RULE", "Different message");
        assert_eq!(engine.logs.len(), 2);
        assert_eq!(engine.logs[1].message, "Different message");

        // Repeat of original message now creates a new entry
        engine.log("WARN", "RULE", "Rule 'Test' conditions were not met");
        assert_eq!(engine.logs.len(), 3);
        assert_eq!(engine.logs[2].message, "Rule 'Test' conditions were not met");
    }

    #[test]
    fn test_matches_process() {
        assert!(DaemonEngine::matches_process("notepad.exe", "notepad.exe", ProcessMatchMode::Exact));
        assert!(DaemonEngine::matches_process("NOTEPAD.EXE", "notepad.exe", ProcessMatchMode::Exact));
        assert!(!DaemonEngine::matches_process("notepad.exe", "calc.exe", ProcessMatchMode::Exact));

        assert!(DaemonEngine::matches_process("*game*", "my_game_client.exe", ProcessMatchMode::Wildcard));
        assert!(DaemonEngine::matches_process("*.exe", "anything.exe", ProcessMatchMode::Wildcard));
        assert!(DaemonEngine::matches_process("steam*", "steamwebhelper.exe", ProcessMatchMode::Wildcard));
        assert!(!DaemonEngine::matches_process("*.exe", "anything.dll", ProcessMatchMode::Wildcard));

        assert!(DaemonEngine::matches_process("chrome", "google-chrome-stable", ProcessMatchMode::Regex));
        assert!(!DaemonEngine::matches_process("firefox", "google-chrome-stable", ProcessMatchMode::Regex));
        assert!(DaemonEngine::matches_process(r"^notepad\.exe$", "notepad.exe", ProcessMatchMode::Regex));
        assert!(DaemonEngine::matches_process(r"^NOTEPAD\.EXE$", "notepad.exe", ProcessMatchMode::Regex));
        assert!(!DaemonEngine::matches_process(r"^notepad\.exe$", "my_notepad.exe", ProcessMatchMode::Regex));
        assert!(DaemonEngine::matches_process(r"(chrome|edge)\.exe", "edge.exe", ProcessMatchMode::Regex));
        assert!(DaemonEngine::matches_process(r"game_\d+\.exe", "game_42.exe", ProcessMatchMode::Regex));
        assert!(!DaemonEngine::matches_process(r"game_\d+\.exe", "game_abc.exe", ProcessMatchMode::Regex));
        // Graceful handling of invalid regex pattern without panicking (falls back to contains)
        assert!(DaemonEngine::matches_process("[invalid", "test_[invalid_proc", ProcessMatchMode::Regex));
    }

    #[test]
    fn test_device_filter_matches() {
        let device = r"\\?\USB#VID_046D&PID_C52B#12345678#{guid}";
        assert!(DaemonEngine::device_filter_matches(None, Some(device)));
        assert!(DaemonEngine::device_filter_matches(Some(""), Some(device)));
        assert!(DaemonEngine::device_filter_matches(Some("vid_046d"), Some(device)));
        assert!(DaemonEngine::device_filter_matches(Some("USB"), Some(device)));
        assert!(!DaemonEngine::device_filter_matches(Some("PCI"), Some(device)));
        assert!(!DaemonEngine::device_filter_matches(Some("USB"), None));
        assert!(DaemonEngine::device_filter_matches(Some("*"), None));
    }

    #[test]
    fn test_rule_with_companion_mode_and_exit_actions_serde() {
        let json = r#"{
            "id": "rule_1",
            "name": "Game with Companion",
            "is_enabled": true,
            "trigger": {
                "$trigger_type": "process",
                "process_name": "game.exe",
                "is_started": true,
                "match_mode": "exact"
            },
            "actions": [
                {
                    "$action_type": "start_process",
                    "file_path": "companion.exe",
                    "close_on_exit": true,
                    "run_hidden": true
                }
            ],
            "on_exit_actions": [
                {
                    "$action_type": "notification",
                    "title": "Game Over",
                    "message": "Game exited"
                }
            ]
        }"#;

        let rule: Rule = serde_json::from_str(json).expect("deserialize rule");
        assert_eq!(rule.name, "Game with Companion");
        assert_eq!(rule.actions.len(), 1);
        if let Action::StartProcess { close_on_exit, run_hidden, .. } = &rule.actions[0] {
            assert!(*close_on_exit);
            assert!(*run_hidden);
        } else {
            panic!("Expected StartProcess");
        }

        assert_eq!(rule.on_exit_actions.len(), 1);
        if let Action::Notification { title, .. } = &rule.on_exit_actions[0] {
            assert_eq!(title, "Game Over");
        } else {
            panic!("Expected Notification");
        }
    }

    #[test]
    fn test_process_audio_rule_serde() {
        use crate::models::{AudioTarget, AudioVolumeAction};

        let json = r#"{
            "id": "audio_rule_1",
            "name": "Game Audio Intro Muter",
            "is_enabled": true,
            "actions": [
                {
                    "$action_type": "audio_control",
                    "target": "process",
                    "process_name": "game.exe",
                    "action_type": "mute",
                    "volume_level": 100
                },
                {
                    "$action_type": "wait",
                    "mode": "timer",
                    "seconds": 5.0
                },
                {
                    "$action_type": "audio_control",
                    "target": "process",
                    "process_name": "{ProcessName}",
                    "action_type": "unmute",
                    "volume_level": 100
                }
            ]
        }"#;

        let rule: Rule = serde_json::from_str(json).expect("deserialize process audio rule");
        assert_eq!(rule.name, "Game Audio Intro Muter");
        assert_eq!(rule.actions.len(), 3);

        if let Action::AudioControl { target, process_name, action_type, .. } = &rule.actions[0] {
            assert_eq!(*target, AudioTarget::Process);
            assert_eq!(process_name.as_deref(), Some("game.exe"));
            assert_eq!(*action_type, AudioVolumeAction::Mute);
        } else {
            panic!("Expected AudioControl Process Mute");
        }

        if let Action::Wait { seconds, .. } = &rule.actions[1] {
            assert_eq!(*seconds, 5.0);
        } else {
            panic!("Expected Wait");
        }

        if let Action::AudioControl { target, process_name, action_type, .. } = &rule.actions[2] {
            assert_eq!(*target, AudioTarget::Process);
            assert_eq!(process_name.as_deref(), Some("{ProcessName}"));
            assert_eq!(*action_type, AudioVolumeAction::Unmute);
        } else {
            panic!("Expected AudioControl Process Unmute");
        }

        let serialized = serde_json::to_string(&rule).expect("serialize rule");
        assert!(serialized.contains("audio_control"));
        assert!(serialized.contains("game.exe"));
    }

    #[test]
    fn test_microphone_rule_serde() {
        use crate::models::{AudioTarget, AudioVolumeAction};

        let json = r#"{
            "id": "mic_rule_1",
            "name": "Mic Muter",
            "is_enabled": true,
            "actions": [
                {
                    "$action_type": "audio_control",
                    "target": "microphone",
                    "action_type": "toggle_mute",
                    "volume_level": 80
                }
            ]
        }"#;

        let rule: Rule = serde_json::from_str(json).expect("deserialize mic rule");
        assert_eq!(rule.name, "Mic Muter");
        assert_eq!(rule.actions.len(), 1);

        if let Action::AudioControl { target, action_type, volume_level, .. } = &rule.actions[0] {
            assert_eq!(*target, AudioTarget::Microphone);
            assert_eq!(*action_type, AudioVolumeAction::ToggleMute);
            assert_eq!(*volume_level, 80);
        } else {
            panic!("Expected Action::AudioControl Microphone");
        }

        let serialized = serde_json::to_string(&rule).expect("serialize mic rule");
        assert!(serialized.contains("audio_control"));
        assert!(serialized.contains("toggle_mute"));
    }







    #[tokio::test]
    async fn test_schedule_trigger_interval() {
        let mut config = AppConfig::default();
        config.rules.push(Rule {
            id: "sched_rule".into(),
            name: "Interval Rule".into(),
            description: String::new(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            trigger: Some(Trigger::Schedule {
                schedule_type: ScheduleType::Interval,
                interval_sec: 2,
                daily_time: None,
                days_of_week: Vec::new(),
                day_of_month: None,
                target_datetime: None,
            }),
            actions: vec![Action::Notification {
                title: "Timer".into(),
                message: "Tick".into(),
            }],
            on_exit_actions: Vec::new(),
            execution_count: 0,
            custom_icon_path: None,
            auto_process_icon: false,
            ..Default::default()
        });

        let mut engine = DaemonEngine::new(config);
        // First tick sets initial timestamp
        engine.tick_schedule().await;
        assert_eq!(engine.logs.len(), 0);

        // Artificially advance timer past the 2 second interval. The scheduler keeps its own
        // attempt stamp next to the accepted-trigger stamp, so both have to move back.
        let past = Instant::now() - std::time::Duration::from_secs(3);
        engine.rule_last_triggered.insert("sched_rule".into(), past);
        engine.scheduler.last_attempt.insert("sched_rule".into(), past);

        // Next tick triggers rule execution
        engine.tick_schedule().await;
        assert!(engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("triggered")));
    }

    #[tokio::test]
    async fn test_schedule_trigger_daily_with_seconds() {
        let mut config = AppConfig::default();
        let now = chrono::Local::now();
        let time_str = format!("{:02}:{:02}:{:02}", now.hour(), now.minute(), now.second());

        config.rules.push(Rule {
            id: "daily_rule".into(),
            name: "Daily Rule".into(),
            description: String::new(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            trigger: Some(Trigger::Schedule {
                schedule_type: ScheduleType::Daily,
                interval_sec: 0,
                daily_time: Some(time_str),
                days_of_week: Vec::new(),
                day_of_month: None,
                target_datetime: None,
            }),
            actions: vec![Action::Notification {
                title: "Daily".into(),
                message: "Alarm".into(),
            }],
            on_exit_actions: Vec::new(),
            execution_count: 0,
            custom_icon_path: None,
            auto_process_icon: false,
            ..Default::default()
        });

        let mut engine = DaemonEngine::new(config);
        let tick_now = chrono::Local::now();
        let tick_time_str = format!("{:02}:{:02}:{:02}", tick_now.hour(), tick_now.minute(), tick_now.second());
        if let Some(r) = engine.config.rules.first_mut() {
            if let Some(Trigger::Schedule { daily_time, .. }) = &mut r.trigger {
                *daily_time = Some(tick_time_str);
            }
        }
        engine.tick_schedule().await;
        assert!(engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("Daily Rule")));
    }

    #[tokio::test]
    async fn test_schedule_trigger_oneshot_with_seconds() {
        let mut config = AppConfig::default();
        let past = chrono::Local::now() - chrono::Duration::seconds(5);
        let dt_str = format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            past.year(), past.month(), past.day(), past.hour(), past.minute(), past.second()
        );

        config.rules.push(Rule {
            id: "oneshot_rule".into(),
            name: "OneShot Rule".into(),
            description: String::new(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            trigger: Some(Trigger::Schedule {
                schedule_type: ScheduleType::OneShot,
                interval_sec: 0,
                daily_time: None,
                days_of_week: Vec::new(),
                day_of_month: None,
                target_datetime: Some(dt_str),
            }),
            actions: vec![Action::Notification {
                title: "OneShot".into(),
                message: "Fired".into(),
            }],
            on_exit_actions: Vec::new(),
            execution_count: 0,
            custom_icon_path: None,
            auto_process_icon: false,
            ..Default::default()
        });

        let mut engine = DaemonEngine::new(config);
        engine.tick_schedule().await;
        assert!(engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("OneShot Rule")));
        assert!(!engine.config.rules[0].is_enabled); // OneShot automatically deactivates after execution
    }

    #[tokio::test]
    async fn test_schedule_trigger_interval_with_start_datetime() {
        let mut config = AppConfig::default();
        // Set future start datetime (1 hour in future)
        let future = chrono::Local::now() + chrono::Duration::hours(1);
        let future_str = format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            future.year(), future.month(), future.day(), future.hour(), future.minute(), future.second()
        );

        config.rules.push(Rule {
            id: "future_interval_rule".into(),
            name: "Future Interval Rule".into(),
            description: String::new(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            trigger: Some(Trigger::Schedule {
                schedule_type: ScheduleType::Interval,
                interval_sec: 10,
                daily_time: None,
                days_of_week: Vec::new(),
                day_of_month: None,
                target_datetime: Some(future_str),
            }),
            actions: vec![Action::Notification {
                title: "Timer".into(),
                message: "Tick".into(),
            }],
            on_exit_actions: Vec::new(),
            execution_count: 0,
            custom_icon_path: None,
            auto_process_icon: false,
            ..Default::default()
        });

        let mut engine = DaemonEngine::new(config);
        engine.tick_schedule().await;
        // Should not have run because future time has not arrived
        assert!(!engine.logs.iter().any(|l| l.message.contains("Future Interval Rule")));

        // Now set past start datetime (5 seconds ago)
        let past = chrono::Local::now() - chrono::Duration::seconds(5);
        let past_str = format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            past.year(), past.month(), past.day(), past.hour(), past.minute(), past.second()
        );
        if let Some(Trigger::Schedule { target_datetime, .. }) = &mut engine.config.rules[0].trigger {
            *target_datetime = Some(past_str);
        }

        engine.tick_schedule().await;
        // Should have run immediately on first execution after start time
        assert!(engine.logs.iter().any(|l| l.message.contains("Future Interval Rule")));
    }

    #[tokio::test]
    async fn test_execute_shutdown_rules() {
        let mut config = AppConfig::default();
        config.rules.push(Rule {
            id: "shutdown_rule".into(),
            name: "Shutdown Cleanup".into(),
            description: String::new(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            trigger: Some(Trigger::Schedule {
                schedule_type: ScheduleType::AppShutdown,
                interval_sec: 0,
                daily_time: None,
                days_of_week: Vec::new(),
                day_of_month: None,
                target_datetime: None,
            }),
            actions: vec![Action::Notification {
                title: "Bye".into(),
                message: "Closing".into(),
            }],
            on_exit_actions: Vec::new(),
            execution_count: 0,
            custom_icon_path: None,
            auto_process_icon: false,
            ..Default::default()
        });

        let mut engine = DaemonEngine::new(config);
        engine.execute_shutdown_rules().await;
        assert!(engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("Shutdown Cleanup")));
    }

    #[tokio::test]
    async fn test_single_instance_process_lifecycle() {
        let mut config = AppConfig::default();
        config.rules.push(Rule {
            id: "si_rule".into(),
            name: "Single Instance App".into(),
            description: String::new(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            trigger: Some(Trigger::Process {
                process_name: "testapp.exe".into(),
                match_mode: ProcessMatchMode::Exact,
                single_instance: true,
            }),
            actions: vec![Action::Notification {
                title: "Started".into(),
                message: "App started".into(),
            }],
            on_exit_actions: vec![Action::Notification {
                title: "Exited".into(),
                message: "All instances exited".into(),
            }],
            execution_count: 0,
            custom_icon_path: None,
            auto_process_icon: false,
            ..Default::default()
        });

        let mut engine = DaemonEngine::new(config);

        // 1. First process starts (PID 1000) -> triggers rule
        engine.handle_process_event(ProcessEvent {
            pid: 1000,
            process_name: "testapp.exe".into(),
            is_started: true,
        }).await;

        let start_logs: Vec<_> = engine.logs.iter()
            .filter(|l| l.category == "RULE" && l.message.contains("triggered by testapp.exe") && !l.message.contains("exit actions"))
            .collect();
        assert_eq!(start_logs.len(), 1, "First instance must trigger the rule");

        // 2. Second process of same exe starts (PID 1001) -> must be SKIPPED
        engine.handle_process_event(ProcessEvent {
            pid: 1001,
            process_name: "testapp.exe".into(),
            is_started: true,
        }).await;

        let start_logs_after_second: Vec<_> = engine.logs.iter()
            .filter(|l| l.category == "RULE" && l.message.contains("triggered by testapp.exe") && !l.message.contains("exit actions"))
            .collect();
        assert_eq!(start_logs_after_second.len(), 1, "Second instance must NOT trigger the rule");
        assert!(engine.logs.iter().any(|l| l.message.contains("skipped (Single Instance)")));

        // 3. First process exits (PID 1000), but second process (PID 1001) is still active
        // -> on_exit_actions must NOT run yet
        engine.handle_process_event(ProcessEvent {
            pid: 1000,
            process_name: "testapp.exe".into(),
            is_started: false,
        }).await;

        let exit_logs: Vec<_> = engine.logs.iter()
            .filter(|l| l.category == "RULE" && l.message.contains("exit actions triggered"))
            .collect();
        assert_eq!(exit_logs.len(), 0, "Exit actions must NOT trigger while another instance is running");

        // 4. Second process exits (PID 1001) -> all instances closed -> on_exit_actions MUST trigger!
        engine.handle_process_event(ProcessEvent {
            pid: 1001,
            process_name: "testapp.exe".into(),
            is_started: false,
        }).await;

        let exit_logs_after_all: Vec<_> = engine.logs.iter()
            .filter(|l| l.category == "RULE" && l.message.contains("exit actions triggered"))
            .collect();
        assert_eq!(exit_logs_after_all.len(), 1, "Exit actions MUST trigger when last instance closes");

        // 5. Subsequent new launch (PID 2000) -> should trigger again as new first instance!
        engine.handle_process_event(ProcessEvent {
            pid: 2000,
            process_name: "testapp.exe".into(),
            is_started: true,
        }).await;

        let final_start_logs: Vec<_> = engine.logs.iter()
            .filter(|l| l.category == "RULE" && l.message.contains("triggered by testapp.exe") && !l.message.contains("exit actions"))
            .collect();
        assert_eq!(final_start_logs.len(), 2, "Subsequent launch after all instances closed must trigger again");
    }

    #[tokio::test]
    async fn test_process_termination_only_rule() {
        let mut config = AppConfig::default();
        config.rules.push(Rule {
            id: "exit_only_rule".into(),
            name: "Exit Only".into(),
            description: String::new(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            trigger: Some(Trigger::Process {
                process_name: "exitapp.exe".into(),
                match_mode: ProcessMatchMode::Exact,
                single_instance: true,
            }),
            actions: Vec::new(),
            on_exit_actions: vec![Action::Notification {
                title: "Exited".into(),
                message: "Closed".into(),
            }],
            execution_count: 0,
            custom_icon_path: None,
            auto_process_icon: false,
            ..Default::default()
        });

        let mut engine = DaemonEngine::new(config);

        // Process starts: actions is empty, so no rule trigger log
        engine.handle_process_event(ProcessEvent {
            pid: 500,
            process_name: "exitapp.exe".into(),
            is_started: true,
        }).await;

        assert!(!engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("triggered by exitapp.exe")));

        // Process terminates: exit actions trigger!
        engine.handle_process_event(ProcessEvent {
            pid: 500,
            process_name: "exitapp.exe".into(),
            is_started: false,
        }).await;

        assert!(engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("exit actions triggered by exitapp.exe")));
    }

    #[tokio::test]
    async fn test_rule_delay_does_not_block_engine() {
        let mut config = AppConfig::default();
        config.rules.push(Rule {
            id: "delayed_rule".into(),
            name: "Delayed Automation".into(),
            description: String::new(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            trigger: None,
            actions: vec![
                Action::Wait {
                    mode: crate::models::WaitMode::Timer,
                    seconds: 0.5,
                    condition: None,
                    timeout_sec: 30.0,
                    timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
                },
                Action::Notification {
                    title: "Test".into(),
                    message: "Completed".into(),
                },
            ],
            on_exit_actions: Vec::new(),
            execution_count: 0,
            custom_icon_path: None,
            auto_process_icon: false,
            ..Default::default()
        });

        let mut engine = DaemonEngine::new(config);
        let start = Instant::now();

        // 1. try_execute_rule MUST return immediately without blocking on the 0.5s delay
        let handle = engine.try_execute_rule(&engine.config.rules[0].clone(), "MANUAL_TEST", 0).await;
        let elapsed_on_return = start.elapsed();
        assert!(
            elapsed_on_return < std::time::Duration::from_millis(150),
            "try_execute_rule took too long to return: {:?}",
            elapsed_on_return
        );

        // 2. The trigger log was generated immediately
        assert!(engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("Delayed Automation")));

        // 3. Engine is free to perform other operations immediately (e.g. handle requests, state query)
        let resp = engine.handle_request(IpcRequest::Ping).await;
        assert!(matches!(resp, IpcResponse::Pong));

        // 4. Awaiting the task handle waits for the actual background delay to complete
        assert!(handle.is_some());
        let _ = handle.unwrap().await;
        assert!(start.elapsed() >= std::time::Duration::from_millis(450));
    }

    #[tokio::test]
    async fn test_concurrent_delays_do_not_block_each_other() {
        let config = AppConfig::default();
        let rule_long = Rule {
            id: "long_rule".into(),
            name: "Long Rule".into(),
            description: String::new(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            trigger: None,
            actions: vec![Action::Wait {
                mode: crate::models::WaitMode::Timer,
                seconds: 0.4,
                condition: None,
                timeout_sec: 30.0,
                timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
            }],
            on_exit_actions: Vec::new(),
            execution_count: 0,
            custom_icon_path: None,
            auto_process_icon: false,
            ..Default::default()
        };
        let rule_short = Rule {
            id: "short_rule".into(),
            name: "Short Rule".into(),
            description: String::new(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            trigger: None,
            actions: vec![Action::Wait {
                mode: crate::models::WaitMode::Timer,
                seconds: 0.1,
                condition: None,
                timeout_sec: 30.0,
                timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
            }],
            on_exit_actions: Vec::new(),
            execution_count: 0,
            custom_icon_path: None,
            auto_process_icon: false,
            ..Default::default()
        };

        let mut engine = DaemonEngine::new(config);
        let start = Instant::now();

        // Launch long rule first, then short rule immediately after
        let handle_long = engine.try_execute_rule(&rule_long, "Test Concurrent", 0).await.unwrap();
        let handle_short = engine.try_execute_rule(&rule_short, "Test Concurrent", 0).await.unwrap();

        // The short rule must complete before the long rule, without being blocked by it
        handle_short.await.unwrap();
        let short_duration = start.elapsed();
        assert!(short_duration < std::time::Duration::from_millis(300), "Short rule was blocked: {:?}", short_duration);

        handle_long.await.unwrap();
        let total_duration = start.elapsed();
        assert!(total_duration >= std::time::Duration::from_millis(380), "Long rule finished too early: {:?}", total_duration);
    }

    #[tokio::test]
    async fn test_rule_execution_tracking_concurrent_and_step() {
        let config = AppConfig::default();
        let rule = Rule {
            id: "tracked_rule".into(),
            name: "Tracked Multi-Step Rule".into(),
            description: "Testing execution tracking".into(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            trigger: None,
            actions: vec![
                Action::Wait {
                    mode: crate::models::WaitMode::Timer,
                    seconds: 0.2,
                    condition: None,
                    timeout_sec: 30.0,
                    timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
                },
                Action::Wait {
                    mode: crate::models::WaitMode::Timer,
                    seconds: 0.2,
                    condition: None,
                    timeout_sec: 30.0,
                    timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
                },
            ],
            on_exit_actions: Vec::new(),
            execution_count: 0,
            custom_icon_path: None,
            auto_process_icon: false,
            ..Default::default()
        };

        let mut engine = DaemonEngine::new(config);

        // Initially no active executions
        let state0 = engine.get_current_state();
        assert!(!state0.active_rule_executions.contains_key("tracked_rule"));

        // Launch instance 1
        let h1 = engine.try_execute_rule(&rule, "MANUAL_TEST", 0).await.unwrap();
        // Give tokio runtime a brief tick to process channel event
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let state1 = engine.get_current_state();
        assert!(state1.active_rule_executions.contains_key("tracked_rule"));
        let exec1 = &state1.active_rule_executions["tracked_rule"];
        assert_eq!(exec1.active_count, 1);
        assert_eq!(exec1.current_step, 0);
        assert_eq!(exec1.total_steps, 2);

        // Launch instance 2 concurrently
        let h2 = engine.try_execute_rule(&rule, "MANUAL_TEST", 0).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let state2 = engine.get_current_state();
        let exec2 = &state2.active_rule_executions["tracked_rule"];
        assert_eq!(exec2.active_count, 2, "Must report 2 concurrent active executions");

        // Wait for both to finish
        let _ = h1.await;
        let _ = h2.await;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let state_done = engine.get_current_state();
        assert!(!state_done.active_rule_executions.contains_key("tracked_rule"));
    }

    #[tokio::test]
    async fn test_stop_rule_execution_aborts_immediately() {
        let config = AppConfig::default();
        let rule = Rule {
            id: "long_running_rule".into(),
            name: "Long Running Rule".into(),
            description: "".into(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            trigger: None,
            actions: vec![
                Action::Wait {
                    mode: crate::models::WaitMode::Timer,
                    seconds: 5.0,
                    condition: None,
                    timeout_sec: 30.0,
                    timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
                },
            ],
            ..Default::default()
        };

        let mut engine = DaemonEngine::new(config);
        let h = engine.try_execute_rule(&rule, "MANUAL_TEST", 0).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        assert!(engine.is_rule_executing("long_running_rule"));

        // Stop execution
        engine.stop_rule_execution("long_running_rule");
        assert!(!engine.is_rule_executing("long_running_rule"));

        // Join handle should return error (cancelled)
        let res = h.await;
        assert!(res.is_err(), "Aborted task must return join error");
    }

    #[tokio::test]
    async fn test_rule_activity_registry_tracks_executions() {
        use crate::daemon::rule_activity;

        let rule_id = "rule_activity_registry_rule";
        let rule = Rule {
            id: rule_id.into(),
            name: "Activity Rule".into(),
            description: String::new(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            trigger: None,
            actions: vec![
                Action::Wait {
                    mode: crate::models::WaitMode::Timer,
                    seconds: 5.0,
                    condition: None,
                    timeout_sec: 30.0,
                    timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
                },
            ],
            ..Default::default()
        };

        let mut engine = DaemonEngine::new(AppConfig::default());
        assert!(!rule_activity::is_running(rule_id));

        let handle = engine.try_execute_rule(&rule, "MANUAL_TEST", 0).await.unwrap();
        assert!(
            rule_activity::is_running(rule_id),
            "a started pipeline must be observable as running"
        );
        assert!(DaemonEngine::evaluate_condition(&RuleCondition::RuleRunning {
            rule_id: rule_id.into(),
            must_be_running: true,
        }));

        engine.stop_rule_execution(rule_id);
        let _ = handle.await;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(
            !rule_activity::is_running(rule_id),
            "an aborted pipeline must drop out of the registry"
        );
    }

    #[test]
    fn test_rule_running_condition_evaluation() {
        use crate::daemon::rule_activity;

        let target = "rule_activity_test_condition_target";
        let running = RuleCondition::RuleRunning {
            rule_id: target.into(),
            must_be_running: true,
        };
        let stopped = RuleCondition::RuleRunning {
            rule_id: target.into(),
            must_be_running: false,
        };

        assert!(!DaemonEngine::evaluate_condition(&running));
        assert!(DaemonEngine::evaluate_condition(&stopped));

        rule_activity::mark_started(target);
        assert!(DaemonEngine::evaluate_condition(&running));
        assert!(!DaemonEngine::evaluate_condition(&stopped));

        rule_activity::mark_finished(target);
        assert!(!DaemonEngine::evaluate_condition(&running));
        assert!(DaemonEngine::evaluate_condition(&stopped));

        // An empty target is invalid and never satisfies the condition, in either direction.
        for must_be_running in [true, false] {
            assert!(!DaemonEngine::evaluate_condition(&RuleCondition::RuleRunning {
                rule_id: String::new(),
                must_be_running,
            }));
        }
    }

    #[tokio::test]
    async fn test_overlay_rule_execution_tracking_and_stop() {
        let _overlay_guard = lock_overlay_state().await;
        crate::daemon::overlay_manager::init();
        let overlay_id = "test_overlay_tracking_id".to_string();
        let rule = Rule {
            id: "overlay_rule".into(),
            name: "Overlay Rule".into(),
            description: String::new(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            trigger: None,
            actions: vec![
                Action::PlaySound { file_path: "".into(), play_beep: false, beep_frequency: None, beep_duration_ms: None },
                Action::Overlay {
                    overlay_id: overlay_id.clone(),
                    custom_overlay: None,
                    operation: crate::models::OverlayOperation::Toggle,
                    override_text: None,
                    duration_ms: None,
                    close_on_exit: false,
                },
            ],
            on_exit_actions: Vec::new(),
            execution_count: 0,
            custom_icon_path: None,
            auto_process_icon: false,
            ..Default::default()
        };

        let config = AppConfig {
            rules: vec![rule],
            ..Default::default()
        };

        let mut engine = DaemonEngine::new(config);

        // Initially overlay is not active
        let state0 = engine.get_current_state();
        assert!(!state0.active_rule_executions.contains_key("overlay_rule"));
        assert!(!engine.is_rule_executing("overlay_rule"));

        // Simulate overlay becoming active in overlay_manager
        crate::daemon::overlay_manager::show_overlay(
            &crate::models::OverlayDefinition {
                id: overlay_id.clone(),
                ..Default::default()
            },
            false,
            None,
        );

        // Allow background thread to process Show command
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Overlay is active in overlay_manager
        let active_ovs = crate::daemon::overlay_manager::get_active_overlay_ids();
        assert!(active_ovs.contains(&overlay_id));

        // Rule MUST report as executing, pointing to step 1 (the Action::Overlay step)
        assert!(engine.is_rule_executing("overlay_rule"));
        let state1 = engine.get_current_state();
        assert!(state1.active_rule_executions.contains_key("overlay_rule"));
        let exec = &state1.active_rule_executions["overlay_rule"];
        assert_eq!(exec.current_step, 1);
        assert_eq!(exec.total_steps, 2);
        assert!(state1.active_overlay_ids.contains(&overlay_id));

        // Stopping the rule MUST hide the overlay
        engine.stop_rule_execution("overlay_rule");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let active_ovs_after = crate::daemon::overlay_manager::get_active_overlay_ids();
        assert!(!active_ovs_after.contains(&overlay_id));
        assert!(!engine.is_rule_executing("overlay_rule"));

        let state_after = engine.get_current_state();
        assert!(!state_after.active_rule_executions.contains_key("overlay_rule"));
    }

    #[test]
    fn test_condition_evaluation_logic() {
        let temp_dir = std::env::temp_dir();
        let existing_path = temp_dir.join("synector_test_cond.tmp");
        let _ = std::fs::write(&existing_path, "test");
        let non_existing_path = temp_dir.join("synector_non_existing_file_12345.tmp");

        let cond_file_exists = RuleCondition::FileWatcher {
            path: existing_path.to_string_lossy().to_string(),
            filter_extension: None,
            must_exist: true,
        };
        let cond_file_not_exists = RuleCondition::FileWatcher {
            path: non_existing_path.to_string_lossy().to_string(),
            filter_extension: None,
            must_exist: true,
        };

        assert!(DaemonEngine::evaluate_condition(&cond_file_exists));
        assert!(!DaemonEngine::evaluate_condition(&cond_file_not_exists));

        let cond_file_must_not_exist = RuleCondition::FileWatcher {
            path: non_existing_path.to_string_lossy().to_string(),
            filter_extension: None,
            must_exist: false,
        };
        assert!(DaemonEngine::evaluate_condition(&cond_file_must_not_exist));

        // Test operators: (1 AND 2) OR (3 AND 4)
        // Case 1: C0 (true) AND C1 (true) -> true
        let all_ok = vec![
            RuleConditionItem::new(cond_file_exists.clone()),
            RuleConditionItem::with_operator(ConditionOperator::And, cond_file_must_not_exist.clone()),
        ];
        assert!(DaemonEngine::evaluate_rule_conditions(&all_ok));

        // Case 2: C0 (true) AND C1 (false) -> false
        let one_fail = vec![
            RuleConditionItem::new(cond_file_exists.clone()),
            RuleConditionItem::with_operator(ConditionOperator::And, cond_file_not_exists.clone()),
        ];
        assert!(!DaemonEngine::evaluate_rule_conditions(&one_fail));

        // Case 3: C0 (true) AND C1 (false) OR C2 (true) -> true (first group fails, second group succeeds)
        let or_branch = vec![
            RuleConditionItem::new(cond_file_exists.clone()),
            RuleConditionItem::with_operator(ConditionOperator::And, cond_file_not_exists.clone()),
            RuleConditionItem::with_operator(ConditionOperator::Or, cond_file_must_not_exist.clone()),
        ];
        assert!(DaemonEngine::evaluate_rule_conditions(&or_branch));

        // Case 4: (1 and 2) or (3 and 4)
        let two_by_two = vec![
            RuleConditionItem::new(cond_file_exists.clone()),
            RuleConditionItem::with_operator(ConditionOperator::And, cond_file_not_exists.clone()), // Group 1 fails
            RuleConditionItem::with_operator(ConditionOperator::Or, cond_file_exists.clone()),
            RuleConditionItem::with_operator(ConditionOperator::And, cond_file_must_not_exist.clone()), // Group 2 passes
        ];
        assert!(DaemonEngine::evaluate_rule_conditions(&two_by_two));

        let _ = std::fs::remove_file(&existing_path);
    }

    #[tokio::test]
    async fn test_preconditions_gate_rule_execution() {
        let temp_dir = std::env::temp_dir();
        let non_existing_path = temp_dir.join("synector_never_exists_abc.tmp");

        let config = AppConfig::default();
        let rule = Rule {
            id: "gated_rule".into(),
            name: "Gated Rule".into(),
            description: "".into(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            trigger: None,
            conditions: vec![
                RuleConditionItem::new(RuleCondition::FileWatcher {
                    path: non_existing_path.to_string_lossy().to_string(),
                    filter_extension: None,
                    must_exist: true,
                }),
            ],
            actions: vec![
                Action::Wait {
                    mode: crate::models::WaitMode::Timer,
                    seconds: 0.05,
                    condition: None,
                    timeout_sec: 30.0,
                    timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
                },
            ],
            ..Default::default()
        };

        let mut engine = DaemonEngine::new(config);

        // When triggered by a real trigger (e.g. process or schedule), conditions fail -> returns None
        let handle = engine.try_execute_rule(&rule, "AUTO_TRIGGER", 0).await;
        assert!(handle.is_none(), "Rule execution must be skipped when preconditions are not met");

        // Preconditions gate rule execution consistently, including in manual/test runs
        let manual_handle = engine.try_execute_rule(&rule, "MANUAL_TEST", 0).await;
        assert!(manual_handle.is_none(), "Manual test must also respect preconditions");

        let test_run_handle = engine.emulate_rule_trigger(&rule).await;
        assert!(test_run_handle.is_none(), "Test run emulation must skip when conditions fail");

        // When conditions are met (e.g. creating the file):
        let _ = std::fs::write(&non_existing_path, b"ok");
        let satisfied_handle = engine.emulate_rule_trigger(&rule).await;
        assert!(satisfied_handle.is_some(), "Test run emulation must execute when conditions are met");
        let _ = satisfied_handle.unwrap().await;
        let _ = std::fs::remove_file(&non_existing_path);
    }

    #[tokio::test]
    async fn test_emulate_rule_trigger_interval() {
        let config = AppConfig::default();
        let rule = Rule {
            id: "interval_test_rule".into(),
            name: "Interval Emulation Rule".into(),
            description: "".into(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            trigger: Some(Trigger::Schedule {
                schedule_type: ScheduleType::Interval,
                interval_sec: 1,
                daily_time: None,
                days_of_week: Vec::new(),
                day_of_month: None,
                target_datetime: None,
            }),
            actions: vec![
                Action::Wait {
                    mode: crate::models::WaitMode::Timer,
                    seconds: 0.05,
                    condition: None,
                    timeout_sec: 30.0,
                    timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
                },
            ],
            ..Default::default()
        };

        let mut engine = DaemonEngine::new(config);
        let handle = engine.emulate_rule_trigger(&rule).await;
        assert!(handle.is_some(), "Interval rule emulation must spawn a task");
        assert!(engine.is_rule_executing(&rule.id), "Rule must be tracked as executing");
        assert!(engine.executions.has_recurring_test(&rule.id));

        let state = engine.get_current_state();
        assert!(state.active_test_rule_ids.contains(&rule.id));
        assert!(state.active_rule_executions.contains_key(&rule.id));

        // Stopping execution cleanly terminates the loop
        engine.stop_rule_execution(&rule.id);
        assert!(!engine.is_rule_executing(&rule.id), "Rule must not be executing after stop");
        assert!(!engine.executions.has_recurring_test(&rule.id));

        let state_after = engine.get_current_state();
        assert!(!state_after.active_test_rule_ids.contains(&rule.id));
        assert!(!state_after.active_rule_executions.contains_key(&rule.id));
    }

    #[tokio::test]
    async fn test_schedule_trigger_monthly_catch_up() {
        let mut config = AppConfig::default();
        config.rules.push(Rule {
            id: "monthly_rule".into(),
            name: "Monthly Catchup Rule".into(),
            description: String::new(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            trigger: Some(Trigger::Schedule {
                schedule_type: ScheduleType::Monthly,
                interval_sec: 0,
                daily_time: Some("00:00:00".into()),
                days_of_week: Vec::new(),
                day_of_month: Some(1),
                target_datetime: None,
            }),
            actions: vec![Action::Notification {
                title: "Month".into(),
                message: "Run".into(),
            }],
            last_executed_at: None,
            ..Default::default()
        });

        let mut engine = DaemonEngine::new(config);
        engine.tick_schedule().await;
        assert!(engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("Monthly Catchup Rule")));

        let cur_ym = chrono::Local::now().format("%Y-%m-01 12:00:00").to_string();
        engine.config.rules[0].last_executed_at = Some(cur_ym);
        engine.logs.clear();

        engine.tick_schedule().await;
        assert!(!engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("Monthly Catchup Rule")));
    }

    #[tokio::test]
    async fn test_schedule_interval_persisted_timestamp() {
        let mut config = AppConfig::default();
        let last_ts = (chrono::Local::now() - chrono::Duration::seconds(20)).format("%Y-%m-%d %H:%M:%S").to_string();
        config.rules.push(Rule {
            id: "persisted_interval_rule".into(),
            name: "Persisted Interval Rule".into(),
            description: String::new(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            trigger: Some(Trigger::Schedule {
                schedule_type: ScheduleType::Interval,
                interval_sec: 100,
                daily_time: None,
                days_of_week: Vec::new(),
                day_of_month: None,
                target_datetime: None,
            }),
            actions: vec![Action::Notification {
                title: "Timer".into(),
                message: "Tick".into(),
            }],
            last_executed_at: Some(last_ts),
            ..Default::default()
        });

        let mut engine = DaemonEngine::new(config);
        engine.tick_schedule().await;
        assert!(!engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("Persisted Interval Rule")));
        assert!(engine.rule_last_triggered.contains_key("persisted_interval_rule"));
    }

    #[test]
    fn test_matches_process_extension_normalization() {
        // Exact mode: automatically normalizes with/without .exe
        assert!(DaemonEngine::matches_process("notepad", "notepad.exe", ProcessMatchMode::Exact));
        assert!(DaemonEngine::matches_process("notepad.exe", "notepad", ProcessMatchMode::Exact));
        assert!(DaemonEngine::matches_process("notepad", "notepad", ProcessMatchMode::Exact));
        assert!(DaemonEngine::matches_process("NOTEPAD", "notepad.exe", ProcessMatchMode::Exact));
        assert!(DaemonEngine::matches_process("notepad.exe", "NOTEPAD.EXE", ProcessMatchMode::Exact));
        assert!(!DaemonEngine::matches_process("notepad", "calc.exe", ProcessMatchMode::Exact));
        assert!(!DaemonEngine::matches_process("notepad.exe", "calc", ProcessMatchMode::Exact));

        // Wildcard mode: supports with/without .exe
        assert!(DaemonEngine::matches_process("note*", "notepad.exe", ProcessMatchMode::Wildcard));
        assert!(DaemonEngine::matches_process("note*", "notepad", ProcessMatchMode::Wildcard));
        assert!(DaemonEngine::matches_process("*pad", "notepad.exe", ProcessMatchMode::Wildcard));
        assert!(DaemonEngine::matches_process("notepad", "notepad.exe", ProcessMatchMode::Wildcard));

        // Regex mode: supports matching process name without .exe
        assert!(DaemonEngine::matches_process(r"^notepad$", "notepad.exe", ProcessMatchMode::Regex));
        assert!(DaemonEngine::matches_process(r"^notepad\.exe$", "notepad.exe", ProcessMatchMode::Regex));
        assert!(!DaemonEngine::matches_process(r"^calc$", "notepad.exe", ProcessMatchMode::Regex));
    }

    #[tokio::test]
    async fn test_stop_rule_execution_terminates_and_clears_bound_processes() {
        let mut config = AppConfig::default();
        let rule = Rule {
            id: "companion_rule".into(),
            name: "Companion Rule".into(),
            description: String::new(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            trigger: None,
            actions: vec![
                Action::StartProcess {
                    file_path: "satellite_tool.exe".into(),
                    arguments: String::new(),
                    working_directory: None,
                    run_as_admin: false,
                    only_if_not_running: false,
                    run_hidden: false,
                    close_on_exit: true,
                },
            ],
            ..Default::default()
        };
        config.rules.push(rule);

        let mut engine = DaemonEngine::new(config);
        // Simulate active bound companion process registered for this rule
        engine.active_bound_processes.push(BoundProcess {
            rule_id: "companion_rule".into(),
            primary_process_name: "game.exe".into(),
            primary_pid: 1234,
            target_executable: "satellite_tool.exe".into(),
        });

        assert!(engine.is_rule_executing("companion_rule"), "Rule must report executing while bound processes are active");

        // Stop rule execution
        engine.stop_rule_execution("companion_rule");

        assert_eq!(engine.active_bound_processes.len(), 0, "Bound processes must be cleared upon stopping rule");
        assert!(!engine.is_rule_executing("companion_rule"), "is_rule_executing must be false after stopping rule");
    }

    #[tokio::test]
    async fn test_manual_run_clears_and_ignores_bound_processes() {
        let mut config = AppConfig::default();
        let rule = Rule {
            id: "manual_companion_rule".into(),
            name: "Manual Companion Rule".into(),
            description: String::new(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            trigger: None,
            actions: vec![
                Action::StartProcess {
                    file_path: "cmd.exe".into(),
                    arguments: "/c exit 0".into(),
                    working_directory: None,
                    run_as_admin: false,
                    only_if_not_running: false,
                    run_hidden: false,
                    close_on_exit: true,
                },
            ],
            ..Default::default()
        };
        config.rules.push(rule.clone());

        let mut engine = DaemonEngine::new(config);
        // Pre-populate stale bound process from earlier
        engine.active_bound_processes.push(BoundProcess {
            rule_id: "manual_companion_rule".into(),
            primary_process_name: "old_game.exe".into(),
            primary_pid: 9999,
            target_executable: "satellite_tool.exe".into(),
        });

        // Trigger manual run (pid == 0 / "Manual Run")
        let h = engine.try_execute_rule(&rule, "Manual Run", 0).await;
        if let Some(handle) = h {
            let _ = handle.await;
        }
        engine.drain_action_events();

        assert_eq!(engine.active_bound_processes.len(), 0, "Manual run must clear leftover bound processes and avoid registering new ones without host PID");
        assert!(!engine.is_rule_executing("manual_companion_rule"), "Manual run must not hang in executing state");
        assert!(engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("after manual execution of rule 'manual_companion_rule'")), "Must log force terminating bound satellite after manual run");
    }

    #[tokio::test]
    async fn test_schedule_trigger_daily_catch_up() {
        let mut config = AppConfig::default();
        config.rules.push(Rule {
            id: "daily_catchup_rule".into(),
            name: "Daily Catchup Rule".into(),
            description: String::new(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            trigger: Some(Trigger::Schedule {
                schedule_type: ScheduleType::Daily,
                interval_sec: 0,
                daily_time: Some("00:00:00".into()),
                days_of_week: Vec::new(),
                day_of_month: None,
                target_datetime: None,
            }),
            actions: vec![Action::Notification {
                title: "Daily Catchup".into(),
                message: "Run".into(),
            }],
            last_executed_at: None,
            ..Default::default()
        });

        let mut engine = DaemonEngine::new(config);
        engine.tick_schedule().await;
        assert!(engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("Daily Catchup Rule")), "Rule should catch up when started after scheduled daily time");

        // Set last_executed_at to today so it won't re-run
        let today = chrono::Local::now().format("%Y-%m-%d 10:00:00").to_string();
        engine.config.rules[0].last_executed_at = Some(today);
        engine.logs.clear();

        engine.tick_schedule().await;
        assert!(!engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("Daily Catchup Rule")), "Rule should not re-run if already executed today");
    }

    #[tokio::test]
    async fn test_pipeline_check_step_halts_subsequent_actions() {
        let mut config = AppConfig::default();
        let rule = Rule {
            id: "check_halt_rule".into(),
            name: "Check Halt Rule".into(),
            description: "".into(),
            is_enabled: true,
            cooldown_seconds: 0.0,
            trigger: None,
            actions: vec![
                Action::Check {
                    condition: RuleCondition::FileWatcher {
                        path: "C:\\definitely_non_existent_file_xyz_123.tmp".into(),
                        filter_extension: None,
                        must_exist: true,
                    },
                },
                Action::Notification {
                    title: "ShouldNotRun".into(),
                    message: "Never".into(),
                },
            ],
            ..Default::default()
        };
        config.rules.push(rule.clone());

        let mut engine = DaemonEngine::new(config);
        let h = engine.try_execute_rule(&rule, "MANUAL_TEST", 0).await;
        if let Some(handle) = h {
            let _ = handle.await;
        }
        engine.drain_action_events();

        assert!(engine.logs.iter().any(|l| l.category == "RULE" && l.message.contains("gracefully halted")), "Check failure must gracefully halt pipeline");
        assert!(!engine.logs.iter().any(|l| l.message.contains("ShouldNotRun")), "Subsequent actions must be skipped");
    }

    #[test]
    fn test_runtime_counter_flush_is_coalesced() {
        let mut config = AppConfig::default();
        config.rules.push(Rule {
            id: "runtime_rule".into(),
            name: "Runtime Rule".into(),
            ..Default::default()
        });
        let mut engine = DaemonEngine::new(config);
        engine.config.rules[0].execution_count = 3;

        engine.mark_runtime_dirty();
        engine.persist_runtime_if_due();
        assert!(
            engine.quota.dirty,
            "the first flush must wait for the coalescing window"
        );

        engine.quota.last_save = Instant::now() - DaemonEngine::RUNTIME_SAVE_INTERVAL - std::time::Duration::from_millis(1);
        engine.persist_runtime_if_due();
        assert!(!engine.quota.dirty, "a flushed runtime must be clean again");
        assert_eq!(engine.config.runtime_state().rules["runtime_rule"].execution_count, 3);

        // A forced flush (quota pause, reset, shutdown) ignores the window.
        engine.mark_runtime_dirty();
        engine.persist_runtime_now();
        assert!(!engine.quota.dirty);
    }

    #[tokio::test]
    async fn test_sync_rules_replaces_set_preserves_counters_and_reloads_once() {
        let mut config = AppConfig::default();
        config.rules.push(Rule {
            id: "keep".into(),
            name: "Keep".into(),
            execution_count: 4,
            last_executed_at: Some("2026-01-01 00:00:00".into()),
            ..Default::default()
        });
        config.rules.push(Rule {
            id: "drop".into(),
            name: "Drop".into(),
            ..Default::default()
        });
        let mut engine = DaemonEngine::new(config);

        let synced = vec![
            Rule {
                id: "keep".into(),
                name: "Keep renamed".into(),
                ..Default::default()
            },
            Rule {
                id: "new".into(),
                name: "New".into(),
                // Sync files define rules, not execution history: these must be dropped.
                execution_count: 9,
                last_executed_at: Some("2026-02-02 00:00:00".into()),
                quota_window_start: Some("2026-02-02 00:00:00".into()),
                ..Default::default()
            },
        ];
        let response = engine.handle_request(IpcRequest::SyncRules(synced)).await;
        assert!(matches!(response, IpcResponse::Ok));

        let ids: Vec<&str> = engine.config.rules.iter().map(|rule| rule.id.as_str()).collect();
        assert_eq!(ids, vec!["keep", "new"]);
        assert_eq!(engine.config.rules[0].name, "Keep renamed");
        assert_eq!(engine.config.rules[0].execution_count, 4);
        assert_eq!(
            engine.config.rules[0].last_executed_at.as_deref(),
            Some("2026-01-01 00:00:00")
        );
        assert_eq!(engine.config.rules[1].execution_count, 0);
        assert!(engine.config.rules[1].last_executed_at.is_none());
        assert!(engine.config.rules[1].quota_window_start.is_none());
    }

    #[tokio::test]
    async fn test_import_rules_starts_new_rules_with_a_clean_runtime() {
        let mut engine = DaemonEngine::new(AppConfig::default());
        let imported = vec![Rule {
            id: "imported".into(),
            name: "Imported".into(),
            trigger: Some(Trigger::Manual),
            // Counters in a rule file are not execution history.
            execution_count: 7,
            last_executed_at: Some("2026-03-03 00:00:00".into()),
            quota_window_start: Some("2026-03-03 00:00:00".into()),
            ..Default::default()
        }];

        let response = engine.handle_request(IpcRequest::ImportRules(imported)).await;
        assert!(matches!(response, IpcResponse::Ok));
        let rule = &engine.config.rules[0];
        assert_eq!(rule.id, "imported");
        assert_eq!(rule.execution_count, 0);
        assert!(rule.last_executed_at.is_none());
        assert!(rule.quota_window_start.is_none());
    }

    #[tokio::test]
    async fn test_import_share_bundles_deps_disables_rules_and_skips_duplicates() {
        let mut config = AppConfig::default();
        let existing = Rule {
            id: "keep".into(),
            name: "Keep".into(),
            trigger: Some(Trigger::Manual),
            ..Default::default()
        };
        config.rules.push(existing.clone());
        let mut engine = DaemonEngine::new(config);

        let mut overlay = crate::models::OverlayDefinition::default_new_overlay();
        overlay.id = "share_ov".into();
        overlay.name = "Shared overlay".into();
        let preset = crate::models::MacroPreset {
            id: "share_preset".into(),
            name: "Shared preset".into(),
            events: vec![crate::models::MacroEvent::KeyDown { key: "A".into() }],
            repeat: 1,
            speed: 1.0,
            infinite: false,
        };
        let mut duplicate = existing.clone();
        duplicate.id = "renamed_duplicate".into();
        let fresh = Rule {
            id: "fresh".into(),
            name: "Fresh".into(),
            trigger: Some(Trigger::Manual),
            actions: vec![
                Action::Macro {
                    preset_id: Some("share_preset".into()),
                    operation: crate::models::MacroOperation::Run,
                    events: vec![],
                    repeat: 1,
                    speed: 1.0,
                    infinite: false,
                },
                Action::Overlay {
                    overlay_id: "share_ov".into(),
                    custom_overlay: None,
                    operation: crate::models::OverlayOperation::Show,
                    override_text: None,
                    duration_ms: None,
                    close_on_exit: false,
                },
            ],
            ..Default::default()
        };
        let payload = crate::sharing::SharePayload {
            version: crate::sharing::SHARE_VERSION,
            app_version: "test".into(),
            created_at: String::new(),
            rules: vec![duplicate, fresh],
            overlays: vec![overlay],
            macro_presets: vec![preset],
        };

        let response = engine
            .handle_request(IpcRequest::ImportShare {
                payload: Box::new(payload.clone()),
                enable: false,
            })
            .await;
        match response {
            IpcResponse::ShareImportResult(summary) => {
                assert_eq!(summary.imported, 1);
                assert_eq!(summary.skipped_duplicates, 1);
                assert_eq!(summary.overlays, 1);
                assert_eq!(summary.macro_presets, 1);
                assert!(!summary.enabled);
            }
            other => panic!("unexpected response: {other:?}"),
        }

        let imported = engine
            .config
            .rules
            .iter()
            .find(|rule| rule.id == "fresh")
            .expect("fresh rule must be imported");
        assert!(!imported.is_enabled, "share imports are disabled by default");
        assert_eq!(imported.execution_count, 0);
        assert!(engine.config.overlays.iter().any(|ov| ov.id == "share_ov"));
        assert!(engine
            .config
            .macro_presets
            .iter()
            .any(|preset| preset.id == "share_preset"));

        // Importing the same code again imports nothing and skips both rules as duplicates.
        let response = engine
            .handle_request(IpcRequest::ImportShare {
                payload: Box::new(payload),
                enable: false,
            })
            .await;
        match response {
            IpcResponse::ShareImportResult(summary) => {
                assert_eq!(summary.imported, 0);
                assert_eq!(summary.skipped_duplicates, 2);
                assert_eq!(summary.overlays, 0);
                assert_eq!(summary.macro_presets, 0);
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_periodic_quota_resets_without_pausing_the_rule() {
        let yesterday = (chrono::Local::now() - chrono::Duration::days(1))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let mut config = AppConfig::default();
        config.rules.push(Rule {
            id: "periodic_quota".into(),
            name: "Periodic quota".into(),
            max_executions: Some(2),
            quota_period: Some(crate::models::QuotaPeriod::Day),
            execution_count: 2,
            last_executed_at: Some(yesterday),
            trigger: Some(Trigger::Manual),
            actions: vec![Action::Notification {
                title: "Tick".into(),
                message: "Tick".into(),
            }],
            ..Default::default()
        });
        let mut engine = DaemonEngine::new(config);

        // Yesterday's spent daily counter does not block today's first run.
        let first = engine.try_execute_rule(&engine.config.rules[0].clone(), "MANUAL_TEST", 0).await;
        assert!(first.is_some());
        assert_eq!(engine.config.rules[0].execution_count, 1);
        engine.drain_action_events();

        let second = engine.try_execute_rule(&engine.config.rules[0].clone(), "MANUAL_TEST", 0).await;
        assert!(second.is_some());
        assert_eq!(engine.config.rules[0].execution_count, 2);
        assert!(engine.config.rules[0].is_enabled, "a periodic quota must not pause the rule");
        assert!(engine
            .logs
            .iter()
            .any(|log| log.message.contains("reached its quota (2/2)")));
        engine.drain_action_events();

        // Once the period is spent the rule stays enabled but is skipped.
        let blocked = engine.try_execute_rule(&engine.config.rules[0].clone(), "MANUAL_TEST", 0).await;
        assert!(blocked.is_none());
        assert!(engine.config.rules[0].is_enabled);
        assert_eq!(engine.config.rules[0].execution_count, 2);

        // A user-invoked run reports the exhausted quota instead of failing silently.
        let emulated = engine.emulate_rule_trigger(&engine.config.rules[0].clone()).await;
        assert!(emulated.is_none());
    }

    #[tokio::test]
    async fn test_lifetime_quota_still_pauses_the_rule() {
        let mut config = AppConfig::default();
        config.rules.push(Rule {
            id: "lifetime_quota".into(),
            name: "Lifetime quota".into(),
            max_executions: Some(1),
            execution_count: 1,
            last_executed_at: Some("2020-01-01 00:00:00".into()),
            trigger: Some(Trigger::Manual),
            actions: vec![Action::Notification {
                title: "Tick".into(),
                message: "Tick".into(),
            }],
            ..Default::default()
        });
        let mut engine = DaemonEngine::new(config);

        let blocked = engine.try_execute_rule(&engine.config.rules[0].clone(), "MANUAL_TEST", 0).await;
        assert!(blocked.is_none());
        assert!(!engine.config.rules[0].is_enabled);
        assert_eq!(engine.config.rules[0].execution_count, 1);
        assert!(engine
            .logs
            .iter()
            .any(|log| log.message.contains("completed quota (1/1)")));
    }

    #[tokio::test]
    async fn test_save_rule_resets_counters_only_when_quota_is_redefined() {
        let mut config = AppConfig::default();
        config.rules.push(Rule {
            id: "quota_edit".into(),
            name: "Quota edit".into(),
            max_executions: Some(5),
            execution_count: 4,
            last_executed_at: Some("2026-11-19 10:00:00".into()),
            quota_window_start: Some("2026-11-19 09:00:00".into()),
            ..Default::default()
        });
        let mut engine = DaemonEngine::new(config);

        // Unrelated edits keep the daemon-owned counters.
        let mut draft = engine.config.rules[0].clone();
        draft.name = "Renamed".into();
        engine.handle_request(IpcRequest::SaveRule(draft)).await;
        assert_eq!(engine.config.rules[0].execution_count, 4);
        assert!(engine.config.rules[0].quota_window_start.is_some());

        // Changing the quota period starts a fresh counting window.
        let mut draft = engine.config.rules[0].clone();
        draft.quota_period = Some(crate::models::QuotaPeriod::Day);
        engine.handle_request(IpcRequest::SaveRule(draft)).await;
        assert_eq!(engine.config.rules[0].execution_count, 0);
        assert!(engine.config.rules[0].quota_window_start.is_none());
        assert!(
            engine.config.rules[0].last_executed_at.is_some(),
            "schedule deduplication relies on the last-run stamp"
        );
    }

    // -----------------------------------------------------------------------
    // Rule-to-rule control (`Action::RuleControl`)
    // -----------------------------------------------------------------------

    fn callable_rule(id: &str, name: &str, actions: Vec<Action>) -> Rule {
        Rule {
            id: id.into(),
            name: name.into(),
            trigger: None,
            actions,
            ..Default::default()
        }
    }

    fn timer_wait(seconds: f64) -> Action {
        Action::Wait {
            mode: crate::models::WaitMode::Timer,
            seconds,
            condition: None,
            timeout_sec: 30.0,
            timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
        }
    }

    fn started_receiver(start: RuleCallStart) -> oneshot::Receiver<()> {
        match start {
            RuleCallStart::Started(completion) => completion,
            RuleCallStart::Skipped => panic!("the rule should have started"),
        }
    }

    fn execution_of(engine: &DaemonEngine, rule_id: &str) -> u64 {
        engine
            .executions
            .active_executions
            .iter()
            .find(|(_, execution)| execution.rule_id == rule_id)
            .map(|(id, _)| *id)
            .expect("the rule should have an active execution")
    }

    #[tokio::test]
    async fn test_rule_call_starts_pipeline_and_signals_completion() {
        let mut config = AppConfig::default();
        config.rules.push(callable_rule("target", "Target", vec![timer_wait(0.1)]));
        let mut engine = DaemonEngine::new(config);

        let completion = started_receiver(
            engine.start_called_rule("target", 0).await.expect("call accepted"),
        );
        engine.drain_action_events();

        assert!(engine.is_rule_executing("target"), "the called pipeline is tracked as active");
        assert_eq!(
            engine.config.rules[0].execution_count, 1,
            "a rule call consumes the target's execution quota"
        );
        assert!(
            engine.logs.iter().any(|log| log.message.contains("triggered by Rule Call")),
            "the call is logged as a trigger"
        );

        completion.await.expect("completion signal");
        engine.drain_action_events();
        assert!(!engine.is_rule_executing("target"));
    }

    #[tokio::test]
    async fn test_rule_call_exposes_the_direct_caller() {
        let mut config = AppConfig::default();
        config.rules.push(callable_rule("caller", "Caller", Vec::new()));
        config.rules.push(callable_rule("target", "Target", vec![timer_wait(0.05)]));
        let mut engine = DaemonEngine::new(config);

        // The caller's chain is empty (it was triggered, not called): the direct caller must
        // still reach the callee's context.
        engine.executions.record_started(
            7,
            ActiveExecutionItem {
                rule_id: "caller".into(),
                step_index: 0,
                total_steps: 1,
                is_exit: false,
                started_at: Instant::now(),
                call_chain: Vec::new(),
            },
        );
        let _completion = started_receiver(engine.start_called_rule("target", 7).await.unwrap());

        assert!(
            engine
                .logs
                .iter()
                .any(|log| log.message.contains("triggered by Rule Call (from 'Caller')")),
            "the callee context names the calling rule: {:?}",
            engine.logs.iter().map(|log| log.message.as_str()).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn test_rule_call_rejects_cycles_and_self_calls() {
        let mut config = AppConfig::default();
        config.rules.push(callable_rule("a", "A", vec![timer_wait(5.0)]));
        config.rules.push(callable_rule("b", "B", vec![timer_wait(5.0)]));
        let mut engine = DaemonEngine::new(config);

        let _completion = started_receiver(engine.start_called_rule("a", 0).await.unwrap());
        engine.drain_action_events();
        let a_execution = execution_of(&engine, "a");

        let err = engine.start_called_rule("a", a_execution).await.unwrap_err();
        assert!(err.contains("calls itself"), "{err}");

        // A pipeline whose chain already contains the target is rejected before spawning.
        engine.executions.record_started(
            999,
            ActiveExecutionItem {
                rule_id: "a".into(),
                step_index: 0,
                total_steps: 1,
                is_exit: false,
                started_at: Instant::now(),
                call_chain: vec!["b".into(), "a".into()],
            },
        );
        let err = engine.start_called_rule("b", 999).await.unwrap_err();
        assert!(err.contains("cycle"), "{err}");

        engine.stop_rule_by_call("a").unwrap();
    }

    #[tokio::test]
    async fn test_rule_call_skips_disabled_targets_and_depth_overflow() {
        let mut config = AppConfig::default();
        config.rules.push(callable_rule("deep", "Deep", vec![timer_wait(5.0)]));
        let mut disabled = callable_rule("off", "Off", vec![timer_wait(5.0)]);
        disabled.is_enabled = false;
        config.rules.push(disabled);
        let mut engine = DaemonEngine::new(config);

        assert!(matches!(
            engine.start_called_rule("off", 0).await.unwrap(),
            RuleCallStart::Skipped
        ));

        engine.executions.record_started(
            1000,
            ActiveExecutionItem {
                rule_id: "chain".into(),
                step_index: 0,
                total_steps: 1,
                is_exit: false,
                started_at: Instant::now(),
                call_chain: (0..rule_call::MAX_RULE_CALL_DEPTH)
                    .map(|index| format!("r{index}"))
                    .collect(),
            },
        );
        let err = engine.start_called_rule("deep", 1000).await.unwrap_err();
        assert!(err.contains("depth limit"), "{err}");
    }

    #[tokio::test]
    async fn test_rule_call_respects_cooldown_and_conditions() {
        let mut config = AppConfig::default();
        let mut cooled = callable_rule("cool", "Cool", vec![timer_wait(0.05)]);
        cooled.cooldown_seconds = 60.0;
        config.rules.push(cooled);
        let mut conditional = callable_rule("cond", "Cond", vec![timer_wait(0.05)]);
        conditional.conditions = vec![RuleConditionItem::new(RuleCondition::ProcessRunning {
            process_name: "synector-not-a-real-process.exe".into(),
            match_mode: ProcessMatchMode::Exact,
            must_be_running: true,
        })];
        config.rules.push(conditional);
        let mut engine = DaemonEngine::new(config);

        let _first = started_receiver(engine.start_called_rule("cool", 0).await.unwrap());
        assert!(
            matches!(
                engine.start_called_rule("cool", 0).await.unwrap(),
                RuleCallStart::Skipped
            ),
            "the second call inside the cooldown window is skipped"
        );
        assert!(engine.logs.iter().any(|log| log.message.contains("cooldown is still active")));

        assert!(matches!(
            engine.start_called_rule("cond", 0).await.unwrap(),
            RuleCallStart::Skipped
        ));
        assert!(engine.logs.iter().any(|log| log.message.contains("conditions are not met")));
    }

    #[tokio::test]
    async fn test_rule_stop_aborts_without_disabling() {
        let mut config = AppConfig::default();
        config.rules.push(callable_rule("victim", "Victim", vec![timer_wait(30.0)]));
        let mut engine = DaemonEngine::new(config);

        let completion = started_receiver(engine.start_called_rule("victim", 0).await.unwrap());
        engine.drain_action_events();
        assert!(engine.is_rule_executing("victim"));

        engine.stop_rule_by_call("victim").expect("stop accepted");
        completion.await.expect("a stopped pipeline still reports completion");
        assert!(!engine.is_rule_executing("victim"));
        assert!(
            engine.config.rules[0].is_enabled,
            "a rule-to-rule stop must not disable the target"
        );
        assert!(engine.logs.iter().any(|log| log.message.contains("stopped by a rule call")));

        // Stopping an unknown rule fails loudly instead of pretending success.
        assert!(engine.stop_rule_by_call("missing").is_err());
    }

    #[tokio::test]
    async fn test_rule_call_reports_missing_target() {
        let mut engine = DaemonEngine::new(AppConfig::default());
        let err = engine.start_called_rule("missing", 0).await.unwrap_err();
        assert!(err.contains("does not exist"), "{err}");
        let err = engine.start_called_rule("   ", 0).await.unwrap_err();
        assert!(err.contains("empty rule_id"), "{err}");
    }

    // -----------------------------------------------------------------------
    // Regression tests for the engine audit findings (U08 / U11 / U14)
    // -----------------------------------------------------------------------

    fn notify_rule(id: &str, name: &str) -> Rule {
        Rule {
            id: id.into(),
            name: name.into(),
            is_enabled: true,
            actions: vec![Action::Notification {
                title: name.into(),
                message: "tick".into(),
            }],
            ..Default::default()
        }
    }

    /// Serializes tests that inspect the global overlay-manager map: the shutdown handler hides
    /// every overlay, which would otherwise race another test's `show_overlay`.
    static OVERLAY_STATE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn lock_overlay_state() -> tokio::sync::MutexGuard<'static, ()> {
        OVERLAY_STATE_LOCK.lock().await
    }

    #[tokio::test]
    async fn test_daily_schedule_with_cooldown_still_fires() {
        let mut config = AppConfig::default();
        config.rules.push(Rule {
            cooldown_seconds: 3600.0,
            trigger: Some(Trigger::Schedule {
                schedule_type: ScheduleType::Daily,
                interval_sec: 0,
                daily_time: Some("00:00:00".into()),
                days_of_week: Vec::new(),
                day_of_month: None,
                target_datetime: None,
            }),
            ..notify_rule("daily_cooldown_rule", "Daily Cooldown Rule")
        });
        let mut engine = DaemonEngine::new(config);

        engine.tick_schedule().await;
        assert!(
            engine.logs.iter().any(|log| log.message.contains("triggered by Daily Schedule")),
            "a daily rule with a cooldown must fire, not pre-record its trigger: {:?}",
            engine.logs.iter().map(|log| log.message.as_str()).collect::<Vec<_>>()
        );
        assert!(engine.rule_last_triggered.contains_key("daily_cooldown_rule"));

        // The accepted run recorded `last_executed_at`, so the same day does not fire twice.
        engine.logs.clear();
        engine.tick_schedule().await;
        assert!(!engine.logs.iter().any(|log| log.message.contains("triggered by Daily Schedule")));
    }

    #[tokio::test]
    async fn test_interval_schedule_with_cooldown_fires_after_the_first_interval() {
        let mut config = AppConfig::default();
        config.rules.push(Rule {
            cooldown_seconds: 60.0,
            trigger: Some(Trigger::Schedule {
                schedule_type: ScheduleType::Interval,
                interval_sec: 1,
                daily_time: None,
                days_of_week: Vec::new(),
                day_of_month: None,
                target_datetime: None,
            }),
            ..notify_rule("interval_cooldown_rule", "Interval Cooldown Rule")
        });
        let mut engine = DaemonEngine::new(config);

        // The first tick starts the interval clock without running.
        engine.tick_schedule().await;
        assert!(!engine.logs.iter().any(|log| log.message.contains("triggered by Schedule Interval")));

        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        engine.tick_schedule().await;
        assert!(
            engine.logs.iter().any(|log| log.message.contains("triggered by Schedule Interval 1s")),
            "the interval run must not be rejected by the trigger stamp it just wrote: {:?}",
            engine.logs.iter().map(|log| log.message.as_str()).collect::<Vec<_>>()
        );
    }

    fn test_process_name() -> String {
        std::env::current_exe()
            .ok()
            .and_then(|path| path.file_name().map(|name| name.to_string_lossy().into_owned()))
            .expect("the test executable has a file name")
    }

    #[tokio::test]
    async fn test_resource_threshold_cpu_only_fires() {
        let mut config = AppConfig::default();
        config.rules.push(Rule {
            trigger: Some(Trigger::ResourceThreshold {
                process_name: test_process_name(),
                cpu_percent: Some(0.0),
                ram_mb: None,
                duration_sec: 0,
            }),
            ..notify_rule("cpu_only_rule", "CPU Only Rule")
        });
        let mut engine = DaemonEngine::new(config);

        engine.tick_resource_threshold().await;
        assert!(
            engine.rule_last_triggered.contains_key("cpu_only_rule"),
            "a CPU-only resource rule must fire"
        );
    }

    #[tokio::test]
    async fn test_resource_threshold_requires_every_configured_metric() {
        let mut config = AppConfig::default();
        config.rules.push(Rule {
            trigger: Some(Trigger::ResourceThreshold {
                process_name: test_process_name(),
                cpu_percent: Some(0.0),
                ram_mb: Some(u64::MAX),
                duration_sec: 0,
            }),
            ..notify_rule("both_metrics_rule", "Both Metrics Rule")
        });
        let mut engine = DaemonEngine::new(config);

        engine.tick_resource_threshold().await;
        assert!(
            !engine.rule_last_triggered.contains_key("both_metrics_rule"),
            "a rule with both limits must not fire on CPU alone"
        );
    }

    #[tokio::test]
    async fn test_emulate_rule_trigger_aborts_without_disabling() {
        let mut config = AppConfig::default();
        config.rules.push(Rule {
            actions: vec![timer_wait(5.0)],
            ..notify_rule("emulated_rule", "Emulated Rule")
        });
        let mut engine = DaemonEngine::new(config);
        let rule = engine.config.rules[0].clone();

        let handle = engine.try_execute_rule(&rule, "MANUAL_TEST", 0).await;
        assert!(handle.is_some(), "the rule should be running before the emulation");
        let _ = engine.emulate_rule_trigger(&rule).await;
        engine.drain_action_events();

        assert!(
            engine.config.rules[0].is_enabled,
            "a GUI test/run must abort the previous execution, not pause the rule"
        );
        engine.abort_rule_executions("emulated_rule");
    }

    /// U08-002: all three spawn paths draw execution ids from one process-wide counter, so
    /// concurrent executions can never share an id. Separate counters would collide (each
    /// starts low) and a later allocation would not be greater than every issued id.
    #[tokio::test]
    async fn test_execution_ids_are_unique_across_the_three_spawn_paths() {
        let mut config = AppConfig::default();
        config.rules.push(Rule {
            actions: vec![timer_wait(30.0)],
            ..notify_rule("triggered_id_rule", "Triggered ID Rule")
        });
        config.rules.push(Rule {
            actions: vec![Action::Notification {
                title: "exit".into(),
                message: "x".into(),
            }],
            on_exit_actions: vec![timer_wait(30.0)],
            ..notify_rule("exit_id_rule", "Exit ID Rule")
        });
        config.rules.push(Rule {
            trigger: Some(Trigger::Schedule {
                schedule_type: ScheduleType::Interval,
                interval_sec: 60,
                daily_time: None,
                days_of_week: Vec::new(),
                day_of_month: None,
                target_datetime: None,
            }),
            actions: vec![timer_wait(30.0)],
            ..notify_rule("recurring_id_rule", "Recurring ID Rule")
        });
        let mut engine = DaemonEngine::new(config);
        let triggered_rule = engine.config.rules[0].clone();
        let exit_rule = engine.config.rules[1].clone();
        let recurring_rule = engine.config.rules[2].clone();

        let triggered_handle = engine
            .try_execute_rule(&triggered_rule, "MANUAL_TEST", 0)
            .await
            .expect("the triggered spawn path must start");
        let exit_handle = engine
            .try_execute_exit_actions(&exit_rule, "MANUAL_TEST", 0)
            .await
            .expect("the exit spawn path must start");
        let recurring_handle = engine
            .emulate_rule_trigger(&recurring_rule)
            .await
            .expect("the recurring spawn path must start");
        engine.drain_action_events();

        let ids = [
            execution_of(&engine, "triggered_id_rule"),
            execution_of(&engine, "exit_id_rule"),
            execution_of(&engine, "recurring_id_rule"),
        ];
        let mut unique = ids.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            3,
            "the three spawn paths must not collide on an execution id: {ids:?}"
        );
        let observed_max = *ids.iter().max().expect("three ids were collected");
        assert!(
            next_execution_id() > observed_max,
            "a later allocation must be greater than every issued id; separate counters would \
             hand out low numbers again ({ids:?})"
        );

        engine.abort_rule_executions("triggered_id_rule");
        engine.abort_rule_executions("exit_id_rule");
        engine.abort_rule_executions("recurring_id_rule");
        let _ = triggered_handle.await;
        let _ = exit_handle.await;
        let _ = recurring_handle.await;
    }

    /// U08-003: the daemon's hold-mode hotkey loop routes `HotkeyEvent::HoldStop` through
    /// `abort_rule_executions` (`src/daemon.rs`), so releasing the hotkey ends the run without
    /// pausing the rule. The explicit user stop is the path that persists a pause; the
    /// contrast pins the difference.
    #[tokio::test]
    async fn test_holdstop_abort_keeps_the_rule_enabled() {
        let mut config = AppConfig::default();
        config.rules.push(Rule {
            actions: vec![timer_wait(30.0)],
            ..notify_rule("holdstop_rule", "HoldStop Rule")
        });
        let mut engine = DaemonEngine::new(config);
        let rule = engine.config.rules[0].clone();

        let handle = engine
            .try_execute_rule(&rule, "Global Hotkey (Hold)", 0)
            .await
            .expect("the held rule must start");
        engine.drain_action_events();
        assert!(engine.is_rule_executing("holdstop_rule"));

        engine.abort_rule_executions("holdstop_rule");
        engine.drain_action_events();
        assert!(
            engine.config.rules[0].is_enabled,
            "releasing a hold hotkey must abort the run, not pause the rule"
        );
        assert!(
            !engine.is_rule_executing("holdstop_rule"),
            "the held execution must be aborted"
        );
        assert!(handle.await.is_err(), "the aborted task must be cancelled");

        // The user-facing Stop remains the pause: it disables and persists the rule.
        let handle = engine
            .try_execute_rule(&rule, "MANUAL_TEST", 0)
            .await
            .expect("the rule is still enabled after the HoldStop abort");
        engine.stop_rule_execution("holdstop_rule");
        assert!(
            !engine.config.rules[0].is_enabled,
            "only the user-facing stop persists a pause"
        );
        assert!(handle.await.is_err(), "the stopped task must be cancelled");
    }

    #[tokio::test]
    async fn test_shutdown_daemon_defers_to_the_coordinated_shutdown() {
        let _overlay_guard = lock_overlay_state().await;
        let mut engine = DaemonEngine::new(AppConfig::default());
        let response = engine.handle_request(IpcRequest::ShutdownDaemon).await;
        assert!(
            matches!(response, IpcResponse::Ok),
            "the handler must answer before the daemon exits"
        );
        // The engine is still alive: the exit and the final flush belong to the daemon main
        // thread, which the handler only asks to shut down.
        assert!(matches!(engine.handle_request(IpcRequest::Ping).await, IpcResponse::Pong));
    }

    #[test]
    fn test_test_step_error_is_redacted_in_the_file_log() {
        crate::logging::init();
        let mut engine = DaemonEngine::new(AppConfig::default());
        let secret = "SYNECTOR_TEST_STEP_SECRET_OUTPUT";
        let response = engine.record_test_step(Err(format!("script failed: {secret}")));

        assert!(matches!(response, IpcResponse::TestStepResult(Err(_))));
        assert!(
            engine.logs.iter().any(|log| log.category == "TEST" && log.message.contains(secret)),
            "the in-app log keeps the actionable error"
        );
        let contents = std::fs::read_to_string(crate::logging::current_log_path()).unwrap_or_default();
        assert!(
            !contents.contains(secret),
            "script output must not reach the daily file log"
        );
        assert!(
            contents.contains("Step test failed (details omitted from the file log)"),
            "the file log keeps a redacted record of the failure"
        );
    }

    /// U08-011: the engine must never run a `TestStep` action itself. Only
    /// `daemon::ipc_server::dispatch` does, before it takes the engine lock, so a wait or a
    /// script cannot stall the scheduler. The inline arm that executed the action under the
    /// lock is gone; this pins the refusal.
    #[tokio::test]
    async fn test_handle_request_does_not_execute_test_step_under_the_lock() {
        let mut engine = DaemonEngine::new(AppConfig::default());
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            engine.handle_request(IpcRequest::TestStep {
                action: timer_wait(30.0),
                context: HashMap::new(),
            }),
        )
        .await
        .expect("handle_request must not run the step test under the engine lock");
        match response {
            IpcResponse::Error(message) => assert!(
                message.contains("dispatched"),
                "the refusal must point at the pre-lock dispatcher: {message}"
            ),
            other => panic!("the engine must refuse to execute a TestStep directly: {other:?}"),
        }
        assert!(
            !engine.logs.iter().any(|log| log.category == "TEST"),
            "no step-test outcome may be recorded: the action never ran"
        );
    }

    #[tokio::test]
    async fn test_rule_writes_report_a_failed_persist() {
        let mut engine = DaemonEngine::new(AppConfig::default());
        // The daemon's write path is switched to fail; the response is what the GUI and CLI read,
        // so it must not claim success.
        engine.forced_persist_failure = true;
        let rule = notify_rule("unpersistable_rule", "Unpersistable Rule");

        let response = engine.handle_request(IpcRequest::SaveRule(rule.clone())).await;
        assert!(matches!(response, IpcResponse::Error(_)), "SaveRule must not answer Ok when the write failed");

        let response = engine
            .handle_request(IpcRequest::ToggleRule { id: rule.id.clone(), enabled: false })
            .await;
        assert!(matches!(response, IpcResponse::Error(_)), "ToggleRule must not answer Ok when the write failed");

        let response = engine.handle_request(IpcRequest::SyncRules(vec![rule.clone()])).await;
        assert!(matches!(response, IpcResponse::Error(_)), "SyncRules must not answer Ok when the write failed");

        let response = engine.handle_request(IpcRequest::DeleteRule(rule.id)).await;
        assert!(matches!(response, IpcResponse::Error(_)), "DeleteRule must not answer Ok when the write failed");
    }

    #[tokio::test]
    async fn test_webhook_run_is_not_treated_as_manual() {
        let mut config = AppConfig::default();
        config.rules.push(Rule {
            trigger: Some(Trigger::Webhook { endpoint_path: "/hook".into() }),
            ..notify_rule("webhook_run_rule", "Webhook Run Rule")
        });
        let mut engine = DaemonEngine::new(config);
        engine.active_bound_processes.push(BoundProcess {
            rule_id: "webhook_run_rule".into(),
            primary_process_name: "game.exe".into(),
            primary_pid: 4242,
            target_executable: "synector-test-nonexistent-satellite.exe".into(),
        });

        let rule = engine.config.rules[0].clone();
        let handle = engine.try_execute_rule(&rule, "Webhook (/hook)", 0).await;
        assert!(handle.is_some());
        assert_eq!(
            engine.active_bound_processes.len(),
            1,
            "a webhook run has no pid but is not a manual run: it must keep companion bindings"
        );
        engine.abort_rule_executions("webhook_run_rule");
    }

    #[tokio::test]
    async fn test_cooldown_survives_a_restart_from_last_executed_at() {
        let mut config = AppConfig::default();
        config.rules.push(Rule {
            cooldown_seconds: 3600.0,
            last_executed_at: Some(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()),
            ..notify_rule("persisted_cooldown_rule", "Persisted Cooldown Rule")
        });
        let mut engine = DaemonEngine::new(config);
        let mut rule = engine.config.rules[0].clone();
        assert!(!engine.rule_last_triggered.contains_key(&rule.id), "the memory map starts empty");

        let blocked = engine.try_execute_rule(&rule, "Schedule Interval 60s", 0).await;
        assert!(blocked.is_none(), "the persisted last run must still gate the cooldown after a restart");

        // User-invoked runs bypass the cooldown.
        let manual = engine.try_execute_rule(&rule, "MANUAL_TEST", 0).await;
        assert!(manual.is_some());
        if let Some(handle) = manual {
            let _ = handle.await;
        }
        engine.drain_action_events();

        // An old persisted timestamp no longer blocks.
        rule.last_executed_at = Some("2020-01-01 00:00:00".into());
        engine.rule_last_triggered.remove(&rule.id);
        let started = engine.try_execute_rule(&rule, "Schedule Interval 60s", 0).await;
        assert!(started.is_some(), "a stale last run must not block forever");
        engine.abort_rule_executions(&rule.id);
    }

    #[tokio::test]
    async fn test_sync_rules_aborts_executions_of_removed_rules() {
        let mut config = AppConfig::default();
        config.rules.push(Rule {
            actions: vec![timer_wait(5.0)],
            ..notify_rule("vanishing_rule", "Vanishing Rule")
        });
        let mut engine = DaemonEngine::new(config);
        let rule = engine.config.rules[0].clone();
        let handle = engine.try_execute_rule(&rule, "MANUAL_TEST", 0).await.expect("rule started");
        engine.drain_action_events();
        assert!(engine.is_rule_executing("vanishing_rule"));
        engine.active_bound_processes.push(BoundProcess {
            rule_id: "vanishing_rule".into(),
            primary_process_name: "game.exe".into(),
            primary_pid: 1,
            target_executable: "synector-test-nonexistent-satellite.exe".into(),
        });

        let response = engine.handle_request(IpcRequest::SyncRules(Vec::new())).await;
        assert!(matches!(response, IpcResponse::Ok));

        assert!(
            engine.executions.active_executions.is_empty(),
            "the removed rule's pipeline must be aborted by the declarative sync"
        );
        assert!(
            engine.active_bound_processes.is_empty(),
            "its bound satellites must be torn down with the rule"
        );
        assert!(handle.await.is_err(), "the aborted task must be cancelled");
    }

    #[tokio::test]
    async fn test_recurring_execution_stops_when_its_rule_disappears() {
        let mut config = AppConfig::default();
        config.rules.push(Rule {
            trigger: Some(Trigger::Schedule {
                schedule_type: ScheduleType::Interval,
                interval_sec: 1,
                daily_time: None,
                days_of_week: Vec::new(),
                day_of_month: None,
                target_datetime: None,
            }),
            ..notify_rule("recurring_live_rule", "Recurring Live Rule")
        });
        let mut engine = DaemonEngine::new(config);
        let rule = engine.config.rules[0].clone();
        let handle = engine.emulate_rule_trigger(&rule).await.expect("recurring run");
        engine.drain_action_events();
        assert!(engine.executions.has_recurring_test("recurring_live_rule"));

        // Remove the rule from the live set the recurring loop re-reads every cycle.
        engine.config.rules.clear();
        engine.refresh_live_rules();
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        engine.drain_action_events();

        assert!(
            !engine.executions.has_recurring_test("recurring_live_rule"),
            "a recurring loop must stop once its rule no longer exists"
        );
        let _ = handle.await;
    }

    #[tokio::test]
    async fn test_oneshot_schedule_stays_armed_when_the_run_is_rejected() {
        let past = chrono::Local::now() - chrono::Duration::seconds(5);
        let mut config = AppConfig::default();
        config.rules.push(Rule {
            trigger: Some(Trigger::Schedule {
                schedule_type: ScheduleType::OneShot,
                interval_sec: 0,
                daily_time: None,
                days_of_week: Vec::new(),
                day_of_month: None,
                target_datetime: Some(past.format("%Y-%m-%d %H:%M:%S").to_string()),
            }),
            conditions: vec![RuleConditionItem::new(RuleCondition::ProcessRunning {
                process_name: "synector-test-absent-process.exe".into(),
                match_mode: ProcessMatchMode::Exact,
                must_be_running: true,
            })],
            ..notify_rule("conditional_oneshot_rule", "Conditional OneShot Rule")
        });
        let mut engine = DaemonEngine::new(config);

        engine.tick_schedule().await;
        assert!(
            engine.config.rules[0].is_enabled,
            "a one-shot whose conditions were not met must stay armed"
        );

        engine.config.rules[0].conditions.clear();
        engine.tick_schedule().await;
        assert!(
            !engine.config.rules[0].is_enabled,
            "a one-shot disables itself only after a run actually started"
        );
    }

    // -----------------------------------------------------------------------
    // Offline `--run-rule` parity (U17-008)
    // -----------------------------------------------------------------------

    fn lua_action(script: &str) -> Action {
        Action::ExecuteScript {
            interpreter: crate::models::ScriptType::Lua,
            script: script.to_string(),
            run_in_window: false,
        }
    }

    #[tokio::test]
    async fn test_offline_runner_produces_the_same_step_tokens_as_the_engine_pipeline() {
        let rule = Rule {
            id: "token_parity".into(),
            name: "Token parity".into(),
            trigger: Some(Trigger::Manual),
            actions: vec![
                lua_action("return 'alpha'"),
                lua_action("return '{PrevOutput}-beta'"),
                lua_action("return '{Step1.Output}/{Step2.Output}'"),
            ],
            stop_on_error: true,
            ..Default::default()
        };

        let offline = run_rule_offline(&rule, RuleRunPolicy::MANUAL).await;
        assert!(offline.skipped.is_none(), "unexpected skip: {:?}", offline.skipped);
        assert!(offline.failures.is_empty(), "no step may fail: {:?}", offline.failures);
        assert!(!offline.halted);
        assert_eq!(offline.context.get("Step1.Output").map(String::as_str), Some("alpha"));
        assert_eq!(offline.context.get("Step2.Output").map(String::as_str), Some("alpha-beta"));
        assert_eq!(
            offline.context.get("Step3.Output").map(String::as_str),
            Some("alpha/alpha-beta")
        );
        assert_eq!(
            offline.context.get("PrevOutput").map(String::as_str),
            Some("alpha/alpha-beta")
        );

        // The daemon pipeline runs the same loop over the same standard context.
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut engine_ctx = DaemonEngine::pipeline_context("Manual Run", 0, HashMap::new());
        let run = PipelineRun {
            actions: &rule.actions,
            execution_id: 42,
            rule_id: &rule.id,
            rule_name: &rule.name,
            context_name: "Manual Run",
            pid: 0,
            is_exit: false,
            stop_on_error: true,
            bind_companions: true,
            label: "Pipeline",
            error_label: "Action",
            warn_on_error_halt: true,
        };
        let halted = run_action_pipeline(run, &mut engine_ctx, &tx).await;
        assert!(!halted);
        for key in [
            "PrevOutput",
            "Step1.Output",
            "Step2.Output",
            "Step3.Output",
            "ProcessName",
            "ProcessId",
        ] {
            assert_eq!(
                offline.context.get(key),
                engine_ctx.get(key),
                "token {key} diverged between the offline runner and the engine"
            );
        }
    }

    #[tokio::test]
    async fn test_offline_runner_matches_daemon_quota_and_disabled_behaviour() {
        let mut config = AppConfig::default();
        config.rules.push(Rule {
            id: "offline_quota".into(),
            name: "Offline quota".into(),
            // A spent lifetime quota and a disabled flag: both tiers must treat the manual run
            // the same way (quota refuses, `is_enabled` does not gate a manual run).
            max_executions: Some(1),
            execution_count: 1,
            is_enabled: false,
            trigger: Some(Trigger::Manual),
            actions: vec![lua_action("return 'ran'")],
            ..Default::default()
        });
        let rule = config.rules[0].clone();

        let offline = run_rule_offline(&rule, RuleRunPolicy::MANUAL).await;
        assert!(
            offline
                .skipped
                .as_deref()
                .is_some_and(|reason| reason.contains("completed quota (1/1)")),
            "the offline run must refuse the spent quota: {:?}",
            offline.skipped
        );
        assert!(offline.context.is_empty());

        let mut engine = DaemonEngine::new(config);
        assert!(
            engine.emulate_rule_trigger(&rule).await.is_none(),
            "the daemon manual run must refuse the spent quota too"
        );
        assert!(engine
            .logs
            .iter()
            .any(|log| log.message.contains("completed quota (1/1)")));

        // With the quota gate disabled the pipeline still runs through the same loop.
        let ungated = run_rule_offline(
            &rule,
            RuleRunPolicy {
                enforce_quota: false,
                enforce_conditions: true,
            },
        )
        .await;
        assert!(ungated.skipped.is_none());
        assert_eq!(
            ungated.context.get("Step1.Output").map(String::as_str),
            Some("ran")
        );

        // A disabled rule without limits runs on both manual paths.
        let disabled = Rule {
            max_executions: None,
            execution_count: 0,
            last_executed_at: None,
            ..rule.clone()
        };
        let offline = run_rule_offline(&disabled, RuleRunPolicy::MANUAL).await;
        assert!(offline.skipped.is_none(), "a manual run may test a disabled rule");
        let mut engine = DaemonEngine::new(AppConfig::default());
        let handle = engine
            .emulate_rule_trigger(&disabled)
            .await
            .expect("the daemon manual run must start a disabled rule");
        let _ = handle.await;

        // Preconditions are enforced identically on both sides.
        let conditional = Rule {
            conditions: vec![RuleConditionItem::new(RuleCondition::ProcessRunning {
                process_name: "synector-test-absent-process.exe".into(),
                match_mode: ProcessMatchMode::Exact,
                must_be_running: true,
            })],
            ..disabled
        };
        let offline = run_rule_offline(&conditional, RuleRunPolicy::MANUAL).await;
        assert!(
            offline
                .skipped
                .as_deref()
                .is_some_and(|reason| reason.contains("conditions were not met")),
            "the offline run must honour preconditions: {:?}",
            offline.skipped
        );
        let mut engine = DaemonEngine::new(AppConfig::default());
        assert!(engine.emulate_rule_trigger(&conditional).await.is_none());
    }

    #[tokio::test]
    async fn test_offline_runner_honours_abort_pipeline() {
        let rule = Rule {
            id: "offline_abort".into(),
            name: "Offline abort".into(),
            trigger: Some(Trigger::Manual),
            actions: vec![
                lua_action("return 'before'"),
                Action::Check {
                    condition: RuleCondition::ProcessRunning {
                        process_name: "synector-test-absent-process.exe".into(),
                        match_mode: ProcessMatchMode::Exact,
                        must_be_running: true,
                    },
                },
                lua_action("return 'after'"),
            ],
            ..Default::default()
        };

        let result = run_rule_offline(&rule, RuleRunPolicy::MANUAL).await;
        assert!(result.halted, "a failed inline check must halt the pipeline");
        assert_eq!(
            result.abort.as_ref().map(|(step, _)| *step),
            Some(2),
            "the abort must be reported at the check step"
        );
        assert_eq!(
            result.context.get("Step1.Output").map(String::as_str),
            Some("before")
        );
        assert!(
            !result.context.contains_key("Step3.Output"),
            "no step after the abort may run"
        );
    }

    #[tokio::test]
    async fn test_offline_runner_stops_on_error_only_when_requested() {
        let actions = vec![lua_action("error('offline boom')"), lua_action("return 'after'")];
        let stop = Rule {
            id: "offline_stop".into(),
            name: "Offline stop".into(),
            actions: actions.clone(),
            stop_on_error: true,
            ..Default::default()
        };

        let result = run_rule_offline(&stop, RuleRunPolicy::MANUAL).await;
        assert!(result.halted);
        assert_eq!(result.failures.len(), 1);
        assert!(!result.context.contains_key("Step2.Output"));

        let continue_on_error = Rule {
            stop_on_error: false,
            ..stop
        };
        let result = run_rule_offline(&continue_on_error, RuleRunPolicy::MANUAL).await;
        assert!(!result.halted, "without stop_on_error the pipeline continues");
        assert_eq!(result.failures.len(), 1);
        assert_eq!(
            result.context.get("Step2.Output").map(String::as_str),
            Some("after")
        );
    }

    // -----------------------------------------------------------------------
    // IPC rule validation (U13-006)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_save_rule_rejects_invalid_rule_and_keeps_the_stored_set() {
        let mut config = AppConfig::default();
        config.rules.push(Rule {
            id: "stored".into(),
            name: "Stored".into(),
            actions: vec![Action::Notification {
                title: "keep".into(),
                message: "me".into(),
            }],
            ..Default::default()
        });
        let mut engine = DaemonEngine::new(config);

        // The documented invalid combination: an infinite macro without a global-hotkey trigger.
        let invalid = Rule {
            id: "stored".into(),
            name: "Replaced".into(),
            actions: vec![Action::Macro {
                preset_id: None,
                operation: crate::models::MacroOperation::Run,
                events: Vec::new(),
                repeat: 1,
                speed: 1.0,
                infinite: true,
            }],
            ..Default::default()
        };
        let response = engine
            .handle_request(IpcRequest::SaveRule(invalid))
            .await;
        match response {
            IpcResponse::Error(message) => assert!(
                message.contains("infinite macro"),
                "the validation message must explain the rejection: {message}"
            ),
            other => panic!("an invalid rule must be rejected: {other:?}"),
        }
        assert_eq!(engine.config.rules.len(), 1);
        assert_eq!(engine.config.rules[0].name, "Stored");
        assert!(matches!(
            engine.config.rules[0].actions.as_slice(),
            [Action::Notification { .. }]
        ));
    }

    #[tokio::test]
    async fn test_sync_rules_rejects_invalid_set_and_keeps_the_stored_set() {
        let mut config = AppConfig::default();
        config.rules.push(notify_rule("stored", "Stored"));
        let mut engine = DaemonEngine::new(config);

        let synced = vec![
            Rule {
                id: "duplicate".into(),
                name: "First".into(),
                trigger: Some(Trigger::Manual),
                ..Default::default()
            },
            Rule {
                id: "duplicate".into(),
                name: "Second".into(),
                trigger: Some(Trigger::Manual),
                ..Default::default()
            },
        ];
        let response = engine
            .handle_request(IpcRequest::SyncRules(synced))
            .await;
        match response {
            IpcResponse::Error(message) => assert!(
                message.contains("duplicate rule id"),
                "the validation message must explain the rejection: {message}"
            ),
            other => panic!("an invalid rule set must be rejected: {other:?}"),
        }
        let ids: Vec<&str> = engine.config.rules.iter().map(|rule| rule.id.as_str()).collect();
        assert_eq!(ids, vec!["stored"], "the stored rules must stay untouched");
        assert_eq!(engine.config.rules[0].name, "Stored");
    }

    fn tracked_execution(rule_id: &str, call_chain: Vec<String>) -> ActiveExecutionItem {
        ActiveExecutionItem {
            rule_id: rule_id.to_string(),
            step_index: 0,
            total_steps: 2,
            is_exit: false,
            started_at: Instant::now(),
            call_chain,
        }
    }

    #[tokio::test]
    async fn test_execution_tracker_keeps_pipelines_and_abort_handles_in_step() {
        let mut tracker = ExecutionTracker::new();
        let first = tokio::spawn(std::future::pending::<()>());
        tracker.record_started(1, tracked_execution("rule_a", vec!["caller".into()]));
        tracker.register_abort_handle(1, "rule_a".into(), first.abort_handle());

        assert!(tracker.is_rule_executing("rule_a"));
        assert_eq!(tracker.call_chain(1), vec!["caller".to_string()]);
        assert!(tracker.is_running_rule(1, "rule_a"));
        assert_eq!(tracker.running_rule_id(1), Some("rule_a"));
        tracker.record_step(1, 1, 2, true);
        let item = tracker.active_executions().next().expect("one tracked execution");
        assert_eq!((item.step_index, item.total_steps, item.is_exit), (1, 2, true));

        // A finished pipeline leaves neither its entry nor its abort handle behind.
        tracker.record_finished(1, "rule_a");
        assert_eq!(tracker.active_executions().count(), 0);
        assert!(!tracker.is_rule_executing("rule_a"));

        // A recurring loop is removed together with the execution it was started as.
        let recurring = tokio::spawn(std::future::pending::<()>());
        tracker.record_started(2, tracked_execution("rule_b", Vec::new()));
        tracker.register_abort_handle(2, "rule_b".into(), recurring.abort_handle());
        tracker.register_recurring_test("rule_b".into(), recurring.abort_handle(), 2);
        assert!(tracker.has_recurring_test("rule_b"));
        assert_eq!(
            tracker.recurring_rule_ids().map(String::as_str).collect::<Vec<_>>(),
            vec!["rule_b"]
        );
        assert!(tracker.abort_recurring_test("rule_b"));
        assert!(!tracker.has_recurring_test("rule_b"));
        assert_eq!(tracker.active_executions().count(), 0);
        assert!(!tracker.abort_recurring_test("rule_b"), "aborting twice must be a no-op");

        // `abort_rule` aborts the live pipelines and drops the rule's stale entries, even when
        // the completion event has not been drained yet.
        let pipeline = tokio::spawn(std::future::pending::<()>());
        tracker.record_started(3, tracked_execution("rule_c", Vec::new()));
        tracker.register_abort_handle(3, "rule_c".into(), pipeline.abort_handle());
        tracker.record_started(4, tracked_execution("rule_c", Vec::new()));
        assert_eq!(tracker.abort_rule("rule_c"), 1);
        assert_eq!(tracker.active_executions().count(), 0);
        assert_eq!(tracker.abort_rule("rule_c"), 0);
    }

    #[test]
    fn test_execution_tracker_releases_bound_companions() {
        let mut tracker = ExecutionTracker::new();
        tracker.add_bound_overlay(BoundOverlay {
            rule_id: "rule_a".into(),
            primary_process_name: "game.exe".into(),
            primary_pid: 1,
            overlay_id: "overlay_one".into(),
        });
        tracker.add_bound_overlay(BoundOverlay {
            rule_id: "rule_a".into(),
            primary_process_name: "game.exe".into(),
            primary_pid: 2,
            overlay_id: "overlay_two".into(),
        });
        tracker.add_bound_overlay(BoundOverlay {
            rule_id: "rule_b".into(),
            primary_process_name: "other.exe".into(),
            primary_pid: 1,
            overlay_id: "overlay_three".into(),
        });
        assert_eq!(tracker.bound_overlays().len(), 3);

        // Both overlays of PID 1 go, the other host's overlay stays.
        let closed = tracker.take_bound_overlays_where(|overlay| overlay.primary_pid == 1);
        assert_eq!(closed.len(), 2);
        assert_eq!(tracker.bound_overlays().len(), 1);
        assert_eq!(tracker.bound_overlays()[0].overlay_id, "overlay_two");

        // Rule teardown removes every remaining overlay of that rule.
        assert_eq!(tracker.take_rule_bound_overlays("rule_a").len(), 1);
        assert!(tracker.bound_overlays().is_empty());

        // A taken identity also drops a same-(rule, overlay) entry that did not match the
        // predicate itself, matching the pre-split filter + retain pair.
        tracker.add_bound_overlay(BoundOverlay {
            rule_id: "rule_c".into(),
            primary_process_name: "game.exe".into(),
            primary_pid: 9,
            overlay_id: "shared".into(),
        });
        tracker.add_bound_overlay(BoundOverlay {
            rule_id: "rule_c".into(),
            primary_process_name: "game.exe".into(),
            primary_pid: 1,
            overlay_id: "shared".into(),
        });
        assert_eq!(tracker.take_bound_overlays_where(|overlay| overlay.primary_pid == 1).len(), 1);
        assert!(tracker.bound_overlays().is_empty());

        tracker.add_bound_overlay(BoundOverlay {
            rule_id: "rule_d".into(),
            primary_process_name: "game.exe".into(),
            primary_pid: 1,
            overlay_id: "kill_me".into(),
        });
        tracker.remove_rule_bound_overlays("rule_d");
        assert!(tracker.bound_overlays().is_empty());
    }

    #[tokio::test]
    async fn test_state_projector_aggregates_status_telemetry_and_logs() {
        let mut config = AppConfig::default();
        config.rules.push(notify_rule("running", "Running"));
        config.rules.push(notify_rule("companion", "Companion"));
        config.rules.push(notify_rule("overlay_rule", "Overlay Rule"));
        let mut engine = DaemonEngine::new(config);
        engine.analytics.total_executions = 4;
        engine.log("INFO", "TEST", "projector probe");
        engine.executions.record_started(1, tracked_execution("running", Vec::new()));
        engine.active_bound_processes.push(BoundProcess {
            rule_id: "companion".into(),
            primary_process_name: "game.exe".into(),
            primary_pid: 7,
            target_executable: "satellite.exe".into(),
        });
        engine.executions.add_bound_overlay(BoundOverlay {
            rule_id: "overlay_rule".into(),
            primary_process_name: "game.exe".into(),
            primary_pid: 8,
            overlay_id: "overlay_one".into(),
        });
        engine.is_downloading_update = true;
        engine.update_download_progress = 0.25;
        engine.update_status = Some("Downloading".to_string());
        engine.pending_open_update_modal = true;

        let state = StateProjector::new(&engine).project();

        assert_eq!(state.total_rules_count, 3);
        assert_eq!(state.active_rules_count, 3);
        assert_eq!(state.analytics.total_executions, 4);
        assert!(state.logs.iter().any(|log| log.message == "projector probe"));
        assert_eq!(state.active_rule_executions["running"].active_count, 1);
        assert_eq!(state.active_rule_executions["running"].total_steps, 2);
        assert!(
            state.active_rule_executions.contains_key("companion"),
            "a bound companion keeps its rule visible as running"
        );
        assert!(
            state.active_rule_executions.contains_key("overlay_rule"),
            "a bound overlay keeps its rule visible as running"
        );
        assert_eq!(state.update_progress, Some(0.25));
        assert_eq!(state.update_status.as_deref(), Some("Downloading"));
        assert!(state.pending_open_update_modal);
        assert_eq!(state.config.rules.len(), 3);
    }

    #[tokio::test]
    async fn test_get_update_progress_answers_narrowly_and_matches_the_state_snapshot() {
        let mut engine = DaemonEngine::new(AppConfig::default());

        let idle = engine.handle_request(IpcRequest::GetUpdateProgress).await;
        assert!(
            matches!(idle, IpcResponse::UpdateProgress { progress: None, status: None }),
            "an idle daemon reports no progress: {idle:?}"
        );

        // A terminal updater status without an active download is still reported.
        engine.update_status = Some("Update failed".to_string());
        match engine.handle_request(IpcRequest::GetUpdateProgress).await {
            IpcResponse::UpdateProgress { progress, status } => {
                assert_eq!(progress, None);
                assert_eq!(status.as_deref(), Some("Update failed"));
            }
            other => panic!("wrong response variant: {other:?}"),
        }

        engine.is_downloading_update = true;
        engine.update_download_progress = 0.5;
        engine.update_status = Some("50%".to_string());
        match engine.handle_request(IpcRequest::GetUpdateProgress).await {
            IpcResponse::UpdateProgress { progress, status } => {
                assert_eq!(progress, Some(0.5));
                assert_eq!(status.as_deref(), Some("50%"));
            }
            other => panic!("wrong response variant: {other:?}"),
        }

        // The full snapshot projects the same pair, so the overlay and the GUI cannot disagree.
        let state = engine.get_current_state();
        assert_eq!(state.update_progress, Some(0.5));
        assert_eq!(state.update_status.as_deref(), Some("50%"));
    }
}

