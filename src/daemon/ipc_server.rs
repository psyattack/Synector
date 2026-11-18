use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::sync::Mutex;
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
    let mut len_buf = [0u8; 4];
    while server.read_exact(&mut len_buf).await.is_ok() {
        let length = u32::from_le_bytes(len_buf) as usize;
        if length == 0 || length > 16 * 1024 * 1024 {
            crate::logging::warn(
                "IPC",
                &format!("Dropped IPC connection with invalid payload length: {length}"),
            );
            break;
        }

        let mut payload_buf = vec![0u8; length];
        if let Err(e) = server.read_exact(&mut payload_buf).await {
            crate::logging::warn("IPC", &format!("Failed to read an IPC payload: {e}"));
            break;
        }

        let response = match serde_json::from_slice::<IpcRequest>(&payload_buf) {
            Ok(req) => {
                let mut eng = engine.lock().await;
                eng.handle_request(req).await
            }
            Err(e) => {
                crate::logging::warn("IPC", &format!("Rejected a malformed IPC request: {e}"));
                IpcResponse::Error(format!("Invalid request JSON: {e}"))
            }
        };

        let resp_bytes = match serde_json::to_vec(&response) {
            Ok(b) => b,
            Err(e) => serde_json::to_vec(&IpcResponse::Error(e.to_string())).unwrap_or_default(),
        };

        let resp_len = (resp_bytes.len() as u32).to_le_bytes();
        if server.write_all(&resp_len).await.is_err() {
            break;
        }
        if server.write_all(&resp_bytes).await.is_err() {
            break;
        }
    }
}
