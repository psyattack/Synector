use std::collections::HashMap;

use crate::daemon::actions::SpawnedResource;
use crate::models::{Rule, Trigger};

use super::{run_step_loop, PipelineObserver, PipelineOutcome};
use super::quota::QuotaDecision;
use super::DaemonEngine;

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
    /// Processes and overlays the run owns; an offline run is a manual run, so it releases all of
    /// them when the pipeline ends instead of tracking them across daemon lifetime.
    owned: Vec<SpawnedResource>,
}

impl OfflineRunObserver {
    async fn release_owned_resources(&self) {
        for resource in &self.owned {
            match resource {
                SpawnedResource::Process { pid, .. } => {
                    let _ = crate::daemon::actions::kill_process_tree_by_pid(*pid).await;
                }
                SpawnedResource::Overlay {
                    lifetime: crate::models::ResourceLifetime::Detach,
                    ..
                } => {
                    // A detached overlay is left on screen, exactly like a manual run's Detach.
                }
                SpawnedResource::Overlay {
                    overlay_id,
                    generation,
                    ..
                } => {
                    crate::daemon::overlay_manager::hide_overlay_if_generation(
                        overlay_id,
                        *generation,
                    );
                }
            }
        }
    }
}

impl PipelineObserver for OfflineRunObserver {
    fn step_started(&mut self, _step_index: usize, _total_steps: usize) {}

    fn step_completed(&mut self, _step_index: usize, owned: &[SpawnedResource]) {
        self.owned.extend_from_slice(owned);
    }

    fn step_failed(&mut self, step_index: usize, error: &str, _stopping: bool) {
        self.failures.push((step_index + 1, error.to_string()));
    }

    fn pipeline_halted(&mut self, step_index: usize, abort_reason: Option<&str>) {
        if let Some(reason) = abort_reason {
            self.abort = Some((step_index + 1, reason.to_string()));
        }
    }

    fn pipeline_finished(&mut self, _outcome: PipelineOutcome) {}
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
/// disk), the rule-to-rule channel has no dispatcher, and only the tokens available in this
/// process are provided (`ProcessId` stays `0`; no live pid or process path).
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
    observer.release_owned_resources().await;

    OfflineRunResult {
        skipped: None,
        context,
        halted,
        failures: observer.failures,
        abort: observer.abort,
    }
}
