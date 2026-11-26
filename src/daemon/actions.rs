use std::collections::HashMap;
use std::sync::Arc;

use crate::command::CommandCancellation;
use crate::models::Action;

mod audio;
mod filesystem;
mod input;
#[path = "actions/macro.rs"]
mod macro_action;
mod overlay;
mod process;
mod script;
mod support {
    pub(super) mod command;
}
mod system;
mod tokens;
mod webhook;
mod window;

use self::system::set_firewall_rule;

pub use self::audio::set_master_volume_scalar;
pub use self::filesystem::{execute_archive, execute_file_system};
pub(crate) use self::input::INPUT_INJECTION_ENABLED;
pub use self::input::release_all_simulated_inputs;
pub(crate) use self::macro_action::calculate_sleep_ms;
pub use self::macro_action::{
    execute_macro_event, has_any_active_macro, is_macro_running, stop_all_active_macros,
    stop_macro_by_id_or_name,
};
pub use self::process::trim_working_set;
pub use self::system::is_firewall_rule_active;
pub use self::window::{get_foreground_window_pid, has_matching_window};

#[derive(Debug, Clone, PartialEq)]
pub enum ActionOutcome {
    Continue(Option<String>),
    AbortPipeline { reason: String },
}

pub struct ActionRunner;

impl ActionRunner {
    pub async fn execute(action: &Action, context: &HashMap<String, String>) -> Result<ActionOutcome, String> {
        Self::execute_with_output(action, context).await
    }

    pub async fn execute_with_output(
        action: &Action,
        context: &HashMap<String, String>,
    ) -> Result<ActionOutcome, String> {
        Self::execute_with_output_cancellable(action, context, None).await
    }

    /// Executes one step with an optional process-tree kill switch. A manual step test passes a
    /// [`CommandCancellation`] so `StopTestStep` can terminate a script's whole tree; pipeline
    /// runs pass `None` and keep their existing deadline behaviour.
    pub async fn execute_with_output_cancellable(
        action: &Action,
        context: &HashMap<String, String>,
        cancel: Option<Arc<CommandCancellation>>,
    ) -> Result<ActionOutcome, String> {
        match action {
            Action::ExecuteScript { interpreter, script, run_in_window } => {
                let res = Self::run_script_internal_with_cancel(
                    interpreter,
                    script,
                    *run_in_window,
                    context,
                    cancel,
                )
                .await?;
                Ok(ActionOutcome::Continue(res))
            }
            other => Self::execute_void_action_cancellable(other, context, cancel).await,
        }
    }

    async fn run_script_internal_with_cancel(
        interpreter: &crate::models::ScriptType,
        script: &str,
        run_in_window: bool,
        context: &HashMap<String, String>,
        cancel: Option<Arc<CommandCancellation>>,
    ) -> Result<Option<String>, String> {
        script::run_script_internal(interpreter, script, run_in_window, context, cancel).await
    }

    async fn execute_void_action_cancellable(
        action: &Action,
        context: &HashMap<String, String>,
        cancel: Option<Arc<CommandCancellation>>,
    ) -> Result<ActionOutcome, String> {
        match action {
            Action::Wait { mode, seconds, condition, timeout_sec, timeout_policy } => {
                match mode {
                    crate::models::WaitMode::Timer => {
                        let delay = crate::models::duration_from_secs(*seconds)
                            .map_err(|err| format!("wait: {err}"))?;
                        tokio::time::sleep(delay).await;
                        Ok(ActionOutcome::Continue(None))
                    }
                    crate::models::WaitMode::UntilCondition => {
                        // A missing condition cannot become true; refusing it here beats
                        // silently succeeding on the first poll (and burning the timeout).
                        let Some(condition) = condition else {
                            return Err(
                                "wait: until_condition mode needs a condition".to_string()
                            );
                        };
                        let timeout = crate::models::duration_from_secs(*timeout_sec)
                            .map_err(|err| format!("wait timeout: {err}"))?;
                        let has_timeout = *timeout_sec > 0.0;
                        let start = std::time::Instant::now();

                        // "Rule started" is a state check, but a short pipeline can start and
                        // finish between two polls; remembering the rule's start counter lets the
                        // wait notice that start even after the pipeline is gone.
                        let rule_start_baseline =
                            if let crate::models::WaitCondition::RuleStarted { rule_id } = condition {
                                let target = Self::interpolate_tokens(rule_id, context);
                                let target = target.trim().to_string();
                                let starts = crate::daemon::rule_activity::snapshot(&target).starts;
                                Some((target, starts))
                            } else {
                                None
                            };

                        loop {
                            let satisfied = match Some(condition) {
                                Some(crate::models::WaitCondition::ProcessStarted { process_name, match_mode }) => {
                                    let proc = Self::interpolate_tokens(process_name, context);
                                    let map = crate::daemon::watcher::ProcessWatcher::snapshot_process_map();
                                    map.values().any(|name| crate::daemon::conditions::matches_process(&proc, name, *match_mode))
                                }
                                Some(crate::models::WaitCondition::ProcessExited { process_name, match_mode }) => {
                                    let proc = Self::interpolate_tokens(process_name, context);
                                    let map = crate::daemon::watcher::ProcessWatcher::snapshot_process_map();
                                    !map.values().any(|name| crate::daemon::conditions::matches_process(&proc, name, *match_mode))
                                }
                                Some(crate::models::WaitCondition::WindowOpened { process_name, match_mode }) => {
                                    let proc = Self::interpolate_tokens(process_name, context);
                                    has_matching_window(&proc, *match_mode)
                                }
                                Some(crate::models::WaitCondition::WindowClosed { process_name, match_mode }) => {
                                    let proc = Self::interpolate_tokens(process_name, context);
                                    !has_matching_window(&proc, *match_mode)
                                }
                                Some(crate::models::WaitCondition::Hotkey { key_combination }) => {
                                    let combo = Self::interpolate_tokens(key_combination, context);
                                    crate::daemon::hotkeys::parse_hotkey_vkeys(&combo)
                                        .map(|vkeys| crate::daemon::hotkeys::is_hotkey_combination_pressed(&vkeys))
                                        .unwrap_or(false)
                                }
                                Some(crate::models::WaitCondition::File { path, condition }) => {
                                    let p = Self::interpolate_tokens(path, context);
                                    match condition {
                                        crate::models::WaitFileCondition::Exists => std::path::Path::new(&p).exists(),
                                        crate::models::WaitFileCondition::Deleted => !std::path::Path::new(&p).exists(),
                                    }
                                }
                                Some(crate::models::WaitCondition::RuleStarted { rule_id }) => {
                                    let target = Self::interpolate_tokens(rule_id, context);
                                    let target = target.trim();
                                    if target.is_empty() {
                                        false
                                    } else {
                                        let snapshot = crate::daemon::rule_activity::snapshot(target);
                                        snapshot.active > 0
                                            || rule_start_baseline.as_ref().is_some_and(
                                                |(baseline_target, starts)| {
                                                    baseline_target == target && snapshot.starts > *starts
                                                },
                                            )
                                    }
                                }
                                Some(crate::models::WaitCondition::RuleStopped { rule_id }) => {
                                    let target = Self::interpolate_tokens(rule_id, context);
                                    let target = target.trim();
                                    !target.is_empty()
                                        && !crate::daemon::rule_activity::is_running(target)
                                }
                                None => false,
                            };

                            if satisfied {
                                return Ok(ActionOutcome::Continue(None));
                            }

                            if has_timeout && start.elapsed() >= timeout {
                                match timeout_policy {
                                    crate::models::WaitTimeoutPolicy::Abort => {
                                        return Ok(ActionOutcome::AbortPipeline {
                                            reason: format!("Wait condition timed out after {:.1}s", timeout_sec),
                                        });
                                    }
                                    crate::models::WaitTimeoutPolicy::Continue => {
                                        return Ok(ActionOutcome::Continue(None));
                                    }
                                }
                            }

                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        }
                    }
                }
            }

            Action::Check { condition } => {
                let satisfied = crate::daemon::conditions::evaluate_condition(condition);
                if satisfied {
                    Ok(ActionOutcome::Continue(None))
                } else {
                    Ok(ActionOutcome::AbortPipeline {
                        reason: "Check condition not met".to_string(),
                    })
                }
            }

            Action::Firewall { process_name, action } => {
                let proc_name = Self::interpolate_tokens(process_name, context);
                set_firewall_rule(&proc_name, *action).await?;
                Ok(ActionOutcome::Continue(None))
            }

            Action::IfElse { conditions, then_actions, else_if_branches, else_actions } => {
                let is_true = if !conditions.is_empty() {
                    crate::daemon::conditions::evaluate_rule_conditions(conditions)
                } else {
                    true
                };
                let mut last_output: Option<String> = None;
                if is_true {
                    for sub_action in then_actions {
                        match Box::pin(Self::execute_with_output_cancellable(sub_action, context, cancel.clone())).await? {
                            ActionOutcome::Continue(output) => {
                                if output.is_some() {
                                    last_output = output;
                                }
                            }
                            ActionOutcome::AbortPipeline { reason } => return Ok(ActionOutcome::AbortPipeline { reason }),
                        }
                    }
                } else {
                    let mut matched = false;
                    for branch in else_if_branches {
                        if !branch.conditions.is_empty() && crate::daemon::conditions::evaluate_rule_conditions(&branch.conditions) {
                            for sub_action in &branch.actions {
                                match Box::pin(Self::execute_with_output_cancellable(sub_action, context, cancel.clone())).await? {
                                    ActionOutcome::Continue(output) => {
                                        if output.is_some() {
                                            last_output = output;
                                        }
                                    }
                                    ActionOutcome::AbortPipeline { reason } => return Ok(ActionOutcome::AbortPipeline { reason }),
                                }
                            }
                            matched = true;
                            break;
                        }
                    }
                    if !matched {
                        for sub_action in else_actions {
                            match Box::pin(Self::execute_with_output_cancellable(sub_action, context, cancel.clone())).await? {
                                ActionOutcome::Continue(output) => {
                                    if output.is_some() {
                                        last_output = output;
                                    }
                                }
                                ActionOutcome::AbortPipeline { reason } => return Ok(ActionOutcome::AbortPipeline { reason }),
                            }
                        }
                    }
                }
                // The branch output feeds `{PrevOutput}` / `{StepN.Output}` in the parent
                // pipeline; discarding it made a nested ExecuteScript unusable.
                Ok(ActionOutcome::Continue(last_output))
            }

            Action::RuleControl { rule_id, operation, wait } => {
                let target = Self::interpolate_tokens(rule_id, context);
                let caller = crate::daemon::rule_call::caller_from_context(context);
                match operation {
                    crate::models::RuleControlOperation::Call => {
                        crate::daemon::rule_call::call_rule(&target, *wait, caller).await?
                    }
                    crate::models::RuleControlOperation::Stop => {
                        crate::daemon::rule_call::stop_rule(&target, caller).await?
                    }
                }
                Ok(ActionOutcome::Continue(None))
            }

            other => {
                Self::execute_standard_void_action(other, context).await?;
                Ok(ActionOutcome::Continue(None))
            }
        }
    }

    pub(crate) async fn execute_standard_void_action(action: &Action, context: &HashMap<String, String>) -> Result<(), String> {
        match action {
            Action::StartProcess { .. } | Action::KillProcess { .. } | Action::ProcessTuning { .. }
            | Action::TrimWorkingSet { .. } => process::execute(action, context).await,
            Action::Window { .. } => window::execute(action, context),
            Action::PowerPlan { .. } | Action::SystemPower { .. } | Action::Notification { .. }
            | Action::PlaySound { .. } | Action::Display { .. } => system::execute(action, context).await,
            Action::DiscordRpcSet { .. } | Action::DiscordRpcClear => system::execute_discord(action, context).await,
            Action::HttpWebhook { .. } => webhook::execute(action, context).await,
            Action::AudioControl { .. } | Action::AudioFormat { .. } | Action::DefaultDevice { .. } => audio::execute(action, context).await,
            Action::ExecuteScript { .. } => script::execute(action, context, None).await,
            Action::Macro { .. } => macro_action::execute(action, context).await,
            Action::Archive { .. } => filesystem::execute_archive_action(action, context).await,
            Action::FileSystem { .. } => filesystem::execute_fs_action(action, context),
            Action::Overlay { .. } => overlay::execute(action, context),
            Action::Wait { .. } | Action::Check { .. } | Action::Firewall { .. } | Action::IfElse { .. }
            | Action::RuleControl { .. } => {
                unreachable!("control-flow actions are dispatched by execute_void_action_cancellable")
            }
        }
    }

    pub fn interpolate_tokens(text: &str, context: &HashMap<String, String>) -> String {
        tokens::interpolate_tokens(text, context)
    }
}

/// Upper bound for an action whose own contract has no finite ceiling. The value is the validated
/// maximum for a `Wait` timeout; it is a safety net that lets the inline step-test round-trip
/// terminate, never an expected duration.
const UNBOUNDED_ACTION_CEILING: std::time::Duration = std::time::Duration::from_secs(86_400);

/// Longest wall-clock time one daemon-side execution of `action` may legitimately occupy.
///
/// The GUI tests a step by sending the action over IPC and waiting for the daemon to run it, so
/// the client's round-trip bound is this budget plus a fixed slack window (see
/// `ipc::client`). The values are the ceilings the handlers themselves enforce on their helper
/// subprocesses; the slack covers an action that issues more than one helper. An action without
/// an internal ceiling reports [`UNBOUNDED_ACTION_CEILING`] so a legitimate long run is never cut
/// short by the client.
///
/// A new action must state its ceiling here: the match is exhaustive on purpose.
pub fn max_execution_duration(action: &Action) -> std::time::Duration {
    use std::time::Duration;

    use crate::models::{RuleControlOperation, WaitMode};

    match action {
        Action::Wait { mode, seconds, timeout_sec, .. } => match mode {
            WaitMode::Timer => crate::models::duration_from_secs(*seconds).unwrap_or_default(),
            WaitMode::UntilCondition if *timeout_sec > 0.0 => {
                crate::models::duration_from_secs(*timeout_sec).unwrap_or_default()
            }
            WaitMode::UntilCondition => UNBOUNDED_ACTION_CEILING,
        },
        Action::ExecuteScript { run_in_window, .. } => {
            // A script launched in its own window returns as soon as the window is spawned.
            if *run_in_window {
                Duration::ZERO
            } else {
                support::command::SCRIPT_TIMEOUT
            }
        }
        Action::StartProcess { run_as_admin, .. } => {
            // Elevation waits for the UAC prompt; a plain spawn returns immediately.
            if *run_as_admin {
                support::command::ELEVATION_TIMEOUT
            } else {
                Duration::ZERO
            }
        }
        Action::KillProcess { .. }
        | Action::Firewall { .. }
        | Action::Notification { .. }
        | Action::PowerPlan { .. }
        | Action::SystemPower { .. }
        | Action::DiscordRpcSet { .. } => support::command::UTILITY_TIMEOUT,
        Action::AudioControl { .. } | Action::AudioFormat { .. } | Action::DefaultDevice { .. } => {
            support::command::AUDIO_HELPER_TIMEOUT
        }
        Action::HttpWebhook { .. } => webhook::WEBHOOK_TIMEOUT,
        Action::IfElse { then_actions, else_if_branches, else_actions, .. } => {
            // Exactly one branch runs, so the longest branch bounds the action.
            let mut longest = action_sequence_duration(then_actions)
                .max(action_sequence_duration(else_actions));
            for branch in else_if_branches {
                longest = longest.max(action_sequence_duration(&branch.actions));
            }
            longest
        }
        Action::RuleControl { wait, operation, .. } => match operation {
            RuleControlOperation::Call if *wait => UNBOUNDED_ACTION_CEILING,
            _ => Duration::ZERO,
        },
        Action::Macro { .. } | Action::Archive { .. } | Action::FileSystem { .. } => {
            // An infinite macro, an archive (its zip path has no deadline) or a large file
            // copy has no guaranteed internal ceiling.
            UNBOUNDED_ACTION_CEILING
        }
        Action::Window { .. }
        | Action::ProcessTuning { .. }
        | Action::PlaySound { .. }
        | Action::Check { .. }
        | Action::Display { .. }
        | Action::TrimWorkingSet { .. }
        | Action::DiscordRpcClear
        | Action::Overlay { .. } => Duration::ZERO,
    }
}

/// Wall clock of a sequentially executed action list.
fn action_sequence_duration(actions: &[Action]) -> std::time::Duration {
    actions.iter().fold(std::time::Duration::ZERO, |total, action| {
        total.saturating_add(max_execution_duration(action))
    })
}

pub(crate) fn random_gaussian(mean: f64, std_dev: f64) -> f64 {
    macro_action::random_gaussian(mean, std_dev)
}

#[cfg(test)]
mod tests;
