use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use crate::daemon::engine::DaemonEngine;
use crate::models::Trigger;

pub const DEFAULT_WEBHOOK_PORT: u16 = 48123;
/// Upper bound for the bind retry backoff, so a port that stays busy is retried
/// forever without flooding the log.
const MAX_BIND_RETRY_DELAY_SECS: u64 = 30;

fn bind_retry_delay(attempt: u32) -> Duration {
    let exp = attempt.saturating_sub(1).min(5) as u64;
    Duration::from_secs((1u64 << exp).min(MAX_BIND_RETRY_DELAY_SECS))
}

pub struct WebhookServer {
    engine: Arc<Mutex<DaemonEngine>>,
    port: u16,
}

impl WebhookServer {
    pub fn new(engine: Arc<Mutex<DaemonEngine>>, port: u16) -> Self {
        Self { engine, port }
    }

    pub async fn run(&self) {
        let addr = format!("127.0.0.1:{}", self.port);
        let mut attempt = 0u32;
        let listener = loop {
            match TcpListener::bind(&addr).await {
                Ok(listener) => {
                    crate::logging::info(
                        "WEBHOOK",
                        &format!("Webhook server listening on http://{addr}"),
                    );
                    break listener;
                }
                Err(err) => {
                    attempt = attempt.saturating_add(1);
                    let delay = bind_retry_delay(attempt);
                    // The daemon is a GUI-subsystem process with null stderr, so a
                    // silent `eprintln!` used to hide a busy port completely.
                    crate::logging::error(
                        "WEBHOOK",
                        &format!(
                            "Failed to bind webhook server on {addr}: {err}; retrying in {}s",
                            delay.as_secs()
                        ),
                    );
                    tokio::time::sleep(delay).await;
                }
            }
        };

        loop {
            let (mut socket, _) = match listener.accept().await {
                Ok(conn) => conn,
                Err(_) => continue,
            };

            let engine_clone = Arc::clone(&self.engine);
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let n = match socket.read(&mut buf).await {
                    Ok(n) if n > 0 => n,
                    _ => return,
                };

                let req = String::from_utf8_lossy(&buf[..n]);
                let first_line = req.lines().next().unwrap_or_default();
                let parts: Vec<&str> = first_line.split_whitespace().collect();

                if parts.len() >= 2 {
                    let path = parts[1];
                    let prefix = "/api/v1/trigger/";
                    if let Some(target) = path.strip_prefix(prefix) {
                        let target_clean = target.trim_end_matches('/').to_string();
                        let mut eng = engine_clone.lock().await;

                        let matching_rule = eng.config.rules.iter().find(|r| {
                            if !r.is_enabled {
                                return false;
                            }
                            if r.id == target_clean {
                                return true;
                            }
                            if let Some(Trigger::Webhook { endpoint_path }) = &r.trigger {
                                if endpoint_path == &target_clean || endpoint_path.trim_start_matches('/') == target_clean {
                                    return true;
                                }
                            }
                            false
                        }).cloned();

                        if let Some(rule) = matching_rule {
                            let _ = eng.try_execute_rule(&rule, &format!("Webhook ({})", target_clean), 0).await;
                            let body = format!("{{\"status\":\"triggered\",\"rule\":\"{}\"}}\n", rule.name);
                            let resp = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                body.len(),
                                body
                            );
                            let _ = socket.write_all(resp.as_bytes()).await;
                            return;
                        } else {
                            let body = format!("{{\"status\":\"error\",\"message\":\"Rule '{}' not found or disabled\"}}\n", target_clean);
                            let resp = format!(
                                "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                body.len(),
                                body
                            );
                            let _ = socket.write_all(resp.as_bytes()).await;
                            return;
                        }
                    }
                }

                let body = "{\"status\":\"error\",\"message\":\"Invalid endpoint. Use /api/v1/trigger/<endpoint_or_rule_id>\"}\n";
                let resp = format!(
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(resp.as_bytes()).await;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bind_retry_backoff_starts_short_and_is_capped() {
        assert_eq!(bind_retry_delay(1), Duration::from_secs(1));
        assert_eq!(bind_retry_delay(2), Duration::from_secs(2));
        assert_eq!(bind_retry_delay(3), Duration::from_secs(4));
        assert_eq!(bind_retry_delay(5), Duration::from_secs(16));
        assert_eq!(bind_retry_delay(6), Duration::from_secs(30));
        assert_eq!(bind_retry_delay(200), Duration::from_secs(30));
    }
}
