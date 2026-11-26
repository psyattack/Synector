use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::sync::Mutex;
use crate::ipc::framing;
use crate::ipc::{IpcRequest, IpcResponse, PIPE_NAME};
use crate::daemon::engine::DaemonEngine;

/// Consecutive failures to create a pipe instance. A broken pipe setup would otherwise log on
/// every 200 ms retry, so only the first failure and every hundredth repeat are recorded.
static PIPE_CREATE_FAILURES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub struct IpcServer {
    engine: Arc<Mutex<DaemonEngine>>,
}

impl IpcServer {
    pub fn new(engine: Arc<Mutex<DaemonEngine>>) -> Self {
        Self { engine }
    }

    pub async fn run(&self) {
        loop {
            match ServerOptions::new()
                .first_pipe_instance(false)
                .max_instances(16)
                .create(PIPE_NAME)
            {
                Ok(server) => {
                    PIPE_CREATE_FAILURES.store(0, Ordering::Relaxed);
                    if let Err(e) = server.connect().await {
                        crate::logging::debug(
                            "IPC",
                            &format!("Named pipe client connect failed: {e}"),
                        );
                        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                        continue;
                    }

                    let engine = Arc::clone(&self.engine);
                    tokio::spawn(async move {
                        handle_client(server, engine).await;
                    });
                }
                Err(err) => {
                    let failures = PIPE_CREATE_FAILURES.fetch_add(1, Ordering::Relaxed) + 1;
                    if failures == 1 || failures.is_multiple_of(100) {
                        crate::logging::warn(
                            "IPC",
                            &format!(
                                "Failed to create a named pipe instance (attempt {failures}): {err}"
                            ),
                        );
                    }
                    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                }
            }
        }
    }
}

async fn handle_client(mut server: NamedPipeServer, engine: Arc<Mutex<DaemonEngine>>) {
    // Read 4-byte length prefix, then JSON payload
    while let Ok(length) = framing::read_length_async(&mut server).await {
        if length == 0 || framing::exceeds_payload_cap(length) {
            crate::logging::warn(
                "IPC",
                &format!("Dropped IPC connection with invalid payload length: {length}"),
            );
            break;
        }

        let mut payload_buf = vec![0u8; length];
        if let Err(e) = framing::read_payload_async(&mut server, &mut payload_buf).await {
            crate::logging::warn("IPC", &format!("Failed to read an IPC payload: {e}"));
            break;
        }

        let response = match serde_json::from_slice::<IpcRequest>(&payload_buf) {
            Ok(req) => dispatch(Arc::clone(&engine), req).await,
            Err(e) => {
                crate::logging::warn("IPC", &format!("Rejected a malformed IPC request: {e}"));
                IpcResponse::Error(format!("Invalid request JSON: {e}"))
            }
        };

        let resp_bytes = serialized_response(&response);

        if framing::write_length_async(&mut server, resp_bytes.len())
            .await
            .is_err()
        {
            break;
        }
        if framing::write_payload_async(&mut server, &resp_bytes)
            .await
            .is_err()
        {
            break;
        }
    }
}

/// Serializes one response frame.
///
/// A response larger than [`framing::MAX_PAYLOAD_BYTES`] cannot be written as a frame the client
/// would accept: the client rejects an over-cap length prefix with `TooLarge` and the caller loses
/// the daemon's actual answer. The daemon therefore applies the same cap it enforces on requests
/// and turns an oversized state snapshot into a small `Error` response, which is what the framing
/// contract promises.
fn serialized_response(response: &IpcResponse) -> Vec<u8> {
    let bytes = match serde_json::to_vec(response) {
        Ok(bytes) => bytes,
        Err(error) => serde_json::to_vec(&IpcResponse::Error(error.to_string())).unwrap_or_default(),
    };
    cap_response_bytes(bytes, framing::MAX_PAYLOAD_BYTES)
}

/// Replaces an over-cap serialized response with an `Error` frame that itself fits the cap.
fn cap_response_bytes(bytes: Vec<u8>, limit: usize) -> Vec<u8> {
    if bytes.len() <= limit {
        return bytes;
    }
    let error = IpcResponse::Error(format!(
        "IPC response is {} bytes, over the {limit} byte limit",
        bytes.len()
    ));
    serde_json::to_vec(&error).unwrap_or_default()
}

/// Routes one request to the engine.
///
/// A `TestStep` never runs under the engine lock. The lock serializes every daemon task and every
/// other IPC request, so executing a wait or a script while holding it would stall process
/// polling, schedules and the whole GUI for the action's duration. The dispatcher reserves the
/// test id, spawns the action task, registers its abort handle and awaits it without the lock;
/// the lock is taken only for the registration and the final outcome. `StopTestStep` aborts that
/// handle from another connection, exactly like `StopRuleExecution` aborts a Test Run.
pub async fn dispatch(engine: Arc<Mutex<DaemonEngine>>, req: IpcRequest) -> IpcResponse {
    match req {
        IpcRequest::TestStep {
            test_id,
            action,
            context,
        } => dispatch_test_step(engine, test_id, action, context).await,
        req => {
            let mut eng = engine.lock().await;
            eng.handle_request(req).await
        }
    }
}

/// Runs one manual step test outside the engine lock and records its outcome. A stopped test is
/// not recorded at all: the stop is already logged, and a success or failure line would
/// misdescribe the run.
async fn dispatch_test_step(
    engine: Arc<Mutex<DaemonEngine>>,
    test_id: u64,
    action: crate::models::Action,
    context: std::collections::HashMap<String, String>,
) -> IpcResponse {
    {
        let mut eng = engine.lock().await;
        if !eng.begin_step_test(test_id) {
            return IpcResponse::TestStepResult(Err(
                "the step test was stopped before it started".to_string(),
            ));
        }
    }

    let cancel = Arc::new(crate::command::CommandCancellation::new());
    let task_cancel = Arc::clone(&cancel);
    let task = tokio::spawn(async move {
        DaemonEngine::execute_test_step(&action, &context, Some(task_cancel)).await
    });
    let abort_handle = task.abort_handle();
    {
        let mut eng = engine.lock().await;
        // A stop that arrived before the handles existed aborts the task and terminates the
        // script's job here; the await below then resolves as cancelled.
        eng.attach_step_test(test_id, abort_handle, cancel);
    }

    let outcome = task.await;
    let mut eng = engine.lock().await;
    if !eng.finish_step_test(test_id) {
        return IpcResponse::TestStepResult(Err("the step test was stopped".to_string()));
    }
    match outcome {
        Ok(outcome) => eng.record_test_step(outcome),
        Err(error) if error.is_cancelled() => IpcResponse::TestStepResult(Err(
            "the step test was cancelled".to_string(),
        )),
        Err(error) => {
            IpcResponse::TestStepResult(Err(format!("the step test task failed: {error}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::models::{Action, WaitMode, WaitTimeoutPolicy};
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_test_step_runs_before_taking_the_engine_lock() {
        let engine = Arc::new(Mutex::new(DaemonEngine::new(AppConfig::default())));
        let step = tokio::spawn(dispatch(
            Arc::clone(&engine),
            IpcRequest::TestStep {
                test_id: 1,
                action: Action::Wait {
                    mode: WaitMode::Timer,
                    seconds: 0.4,
                    condition: None,
                    timeout_sec: 30.0,
                    timeout_policy: WaitTimeoutPolicy::Abort,
                },
                context: HashMap::new(),
            },
        ));

        // While the wait runs, the engine mutex must stay free for the scheduler and other
        // clients; holding it across the action is exactly what the audit found.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(!step.is_finished(), "the step test must still be running");
        let guard = tokio::time::timeout(std::time::Duration::from_millis(200), engine.lock()).await;
        assert!(
            guard.is_ok(),
            "the engine lock must not be held while a step test executes"
        );
        drop(guard);

        let response = step.await.expect("dispatch finished");
        assert!(
            matches!(response, IpcResponse::TestStepResult(Ok(()))),
            "a successful wait must report a successful step"
        );
    }

    /// `StopTestStep` aborts the spawned action task, like `StopRuleExecution` aborts a Test Run:
    /// the wait ends immediately and the stopped run is not recorded as a success.
    #[tokio::test]
    async fn test_stop_test_step_aborts_the_running_action() {
        let engine = Arc::new(Mutex::new(DaemonEngine::new(AppConfig::default())));
        let test_id = 41;
        let step = tokio::spawn(dispatch(
            Arc::clone(&engine),
            IpcRequest::TestStep {
                test_id,
                action: timer_wait(30.0),
                context: HashMap::new(),
            },
        ));

        let register_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        while !engine.lock().await.is_step_test_running(test_id) {
            assert!(
                tokio::time::Instant::now() < register_deadline,
                "the step test was never registered"
            );
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        let response = dispatch(Arc::clone(&engine), IpcRequest::StopTestStep(test_id)).await;
        assert!(matches!(response, IpcResponse::Ok));
        let response = tokio::time::timeout(std::time::Duration::from_secs(2), step)
            .await
            .expect("a stopped step test must finish immediately")
            .expect("the dispatch task");
        match response {
            IpcResponse::TestStepResult(Err(error)) => {
                assert!(error.contains("stopped"), "the answer must name the stop: {error}");
            }
            other => panic!("a stopped step test must answer TestStepResult(Err(..)): {other:?}"),
        }
        assert!(
            !engine.lock().await.is_step_test_running(test_id),
            "the stopped test must be unregistered"
        );
    }

    /// A stop can overtake the start on the wire; the dispatcher must still refuse to run the
    /// action instead of silently executing a test the user already cancelled.
    #[tokio::test]
    async fn test_a_stop_that_overtakes_the_start_refuses_the_step_test() {
        let engine = Arc::new(Mutex::new(DaemonEngine::new(AppConfig::default())));
        let test_id = 42;

        let response = dispatch(Arc::clone(&engine), IpcRequest::StopTestStep(test_id)).await;
        assert!(matches!(response, IpcResponse::Ok));

        let response = dispatch(
            Arc::clone(&engine),
            IpcRequest::TestStep {
                test_id,
                action: timer_wait(30.0),
                context: HashMap::new(),
            },
        )
        .await;
        match response {
            IpcResponse::TestStepResult(Err(error)) => {
                assert!(error.contains("stopped"), "the answer must name the stop: {error}");
            }
            other => panic!("a pre-stopped test must not run: {other:?}"),
        }
        assert!(!engine.lock().await.is_step_test_running(test_id));
    }

    fn timer_wait(seconds: f64) -> Action {
        Action::Wait {
            mode: WaitMode::Timer,
            seconds,
            condition: None,
            timeout_sec: 30.0,
            timeout_policy: WaitTimeoutPolicy::Abort,
        }
    }

    #[tokio::test]
    async fn test_dispatch_answers_update_progress_without_a_state_snapshot() {
        let engine = Arc::new(Mutex::new(DaemonEngine::new(AppConfig::default())));
        {
            let mut eng = engine.lock().await;
            eng.begin_update_download();
            eng.set_update_progress(0.3);
            eng.set_update_status("30%".to_string());
        }

        let response = dispatch(Arc::clone(&engine), IpcRequest::GetUpdateProgress).await;
        match response {
            IpcResponse::UpdateProgress { progress, status } => {
                assert_eq!(progress, Some(0.3));
                assert_eq!(status.as_deref(), Some("30%"));
            }
            other => panic!("the narrow update query must answer with UpdateProgress: {other:?}"),
        }
    }

    #[test]
    fn test_response_within_the_cap_is_written_unchanged() {
        let payload = serde_json::to_vec(&IpcResponse::Pong).expect("serialize the response");
        assert_eq!(cap_response_bytes(payload.clone(), 1024), payload);
    }

    #[test]
    fn test_oversized_response_becomes_a_small_error() {
        let oversized = vec![b'x'; 4096];
        let capped = cap_response_bytes(oversized, 1024);

        assert!(capped.len() <= 1024, "the replacement must fit the cap");
        match serde_json::from_slice::<IpcResponse>(&capped).expect("the replacement is valid IPC JSON")
        {
            IpcResponse::Error(message) => {
                assert!(message.contains("4096 bytes"), "{message}");
                assert!(message.contains("1024 byte limit"), "{message}");
            }
            other => panic!("an over-cap response must become Error, got {other:?}"),
        }
    }
}
