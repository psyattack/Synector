//! Rule-to-rule control: `Action::RuleControl` in its `call` and `stop` modes.
//!
//! Actions run inside a spawned pipeline task and cannot reach the engine: the daemon owns
//! `Arc<Mutex<DaemonEngine>>` and applies every rule mutation through it. The daemon therefore
//! installs a dispatcher here. An action submits a request and awaits the reply, and a waiting
//! call is resolved outside the engine lock so a long pipeline never freezes the daemon.

use std::collections::HashMap;
use std::sync::OnceLock;
use tokio::sync::{mpsc, oneshot};

/// Deepest allowed chain of nested rule calls (`A → B → C …`). Cycle detection catches a rule
/// that is already in the chain; this bound stops runaway nesting that is not a literal cycle.
pub const MAX_RULE_CALL_DEPTH: usize = 8;

/// Context key carrying the id of the pipeline that is running. The engine inserts it into every
/// execution context and `Action::RuleControl` forwards it, which is how the daemon resolves the
/// caller's call chain. Keys starting with `__` are internal and stay out of script environments.
pub const EXECUTION_CTX_KEY: &str = "__RuleExecutionId";

/// Caller execution id from a pipeline context; `0` for a standalone step test.
pub fn caller_from_context(context: &HashMap<String, String>) -> u64 {
    context
        .get(EXECUTION_CTX_KEY)
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

#[derive(Debug)]
pub enum RuleControlCommand {
    /// Run another rule's action pipeline.
    Call { rule_id: String, wait: bool },
    /// Abort the active executions of another rule without disabling it.
    Stop { rule_id: String },
}

pub struct RuleControlRequest {
    pub command: RuleControlCommand,
    /// Pipeline that sent the request; `0` when there is none.
    pub caller_execution: u64,
    pub reply: oneshot::Sender<Result<(), String>>,
}

static CONTROL_TX: OnceLock<mpsc::UnboundedSender<RuleControlRequest>> = OnceLock::new();

/// Arms the dispatcher. Called once by the daemon before any rule can run.
pub fn install(tx: mpsc::UnboundedSender<RuleControlRequest>) {
    let _ = CONTROL_TX.set(tx);
}

pub async fn call_rule(rule_id: &str, wait: bool, caller_execution: u64) -> Result<(), String> {
    submit(
        RuleControlCommand::Call {
            rule_id: rule_id.to_string(),
            wait,
        },
        caller_execution,
    )
    .await
}

pub async fn stop_rule(rule_id: &str, caller_execution: u64) -> Result<(), String> {
    submit(
        RuleControlCommand::Stop {
            rule_id: rule_id.to_string(),
        },
        caller_execution,
    )
    .await
}

async fn submit(command: RuleControlCommand, caller_execution: u64) -> Result<(), String> {
    let Some(tx) = CONTROL_TX.get() else {
        return Err("rule control is unavailable: the daemon dispatcher is not running".to_string());
    };
    let (reply, response) = oneshot::channel();
    tx.send(RuleControlRequest {
        command,
        caller_execution,
        reply,
    })
    .map_err(|_| "rule control is unavailable: the daemon dispatcher stopped".to_string())?;
    response
        .await
        .map_err(|_| "rule control was cancelled".to_string())?
}

/// Executes one request against the engine. Spawned per request by [`crate::daemon::run_daemon`],
/// so a waiting call never blocks the request loop or holds the engine lock while it waits.
pub async fn dispatch(
    engine: std::sync::Arc<tokio::sync::Mutex<crate::daemon::engine::DaemonEngine>>,
    request: RuleControlRequest,
) {
    match request.command {
        RuleControlCommand::Call { rule_id, wait } => {
            let start = {
                let mut eng = engine.lock().await;
                eng.start_called_rule(&rule_id, request.caller_execution).await
            };
            let reply = match start {
                Ok(crate::daemon::engine::RuleCallStart::Started(completion)) => {
                    if wait {
                        // A receiver error means the called pipeline was torn down before its
                        // completion guard could report (stopped or shutting down); either way
                        // the caller has nothing left to wait for.
                        let _ = completion.await;
                    }
                    Ok(())
                }
                // A skipped call is not a failure: the engine logged the reason.
                Ok(crate::daemon::engine::RuleCallStart::Skipped) => Ok(()),
                Err(err) => Err(err),
            };
            let _ = request.reply.send(reply);
        }
        RuleControlCommand::Stop { rule_id } => {
            let reply = {
                let mut eng = engine.lock().await;
                eng.stop_rule_by_call(&rule_id)
            };
            let _ = request.reply.send(reply);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[test]
    fn test_caller_from_context() {
        let mut context = HashMap::new();
        assert_eq!(caller_from_context(&context), 0);
        context.insert(EXECUTION_CTX_KEY.to_string(), "42".to_string());
        assert_eq!(caller_from_context(&context), 42);
        context.insert(EXECUTION_CTX_KEY.to_string(), "not-a-number".to_string());
        assert_eq!(caller_from_context(&context), 0);
    }

    #[tokio::test]
    async fn test_submit_without_dispatcher_is_an_error() {
        let err = call_rule("anything", false, 0).await.unwrap_err();
        assert!(err.contains("rule control is unavailable"), "{err}");
    }

    #[tokio::test]
    async fn test_dispatch_waits_for_completion_and_reports_errors() {
        let target = crate::models::Rule {
            id: "target".into(),
            name: "Target".into(),
            actions: vec![crate::models::Action::Wait {
                mode: crate::models::WaitMode::Timer,
                seconds: 0.05,
                condition: None,
                timeout_sec: 30.0,
                timeout_policy: crate::models::WaitTimeoutPolicy::Abort,
            }],
            ..Default::default()
        };
        let mut config = crate::config::AppConfig::default();
        config.rules.push(target);
        let engine = Arc::new(Mutex::new(crate::daemon::engine::DaemonEngine::new(config)));

        let started = std::time::Instant::now();
        let (reply, response) = oneshot::channel();
        dispatch(
            Arc::clone(&engine),
            RuleControlRequest {
                command: RuleControlCommand::Call {
                    rule_id: "target".into(),
                    wait: true,
                },
                caller_execution: 0,
                reply,
            },
        )
        .await;
        assert!(response.await.unwrap().is_ok());
        assert!(
            started.elapsed() >= std::time::Duration::from_millis(40),
            "wait: true must not reply before the pipeline ends"
        );

        let (reply, response) = oneshot::channel();
        dispatch(
            Arc::clone(&engine),
            RuleControlRequest {
                command: RuleControlCommand::Call {
                    rule_id: "missing".into(),
                    wait: true,
                },
                caller_execution: 0,
                reply,
            },
        )
        .await;
        assert!(response.await.unwrap().is_err());
    }
}
