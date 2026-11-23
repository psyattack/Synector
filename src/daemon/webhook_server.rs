use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use crate::daemon::engine::{DaemonEngine, ExecutionOrigin};
use crate::models::{Rule, Trigger};

pub const DEFAULT_WEBHOOK_PORT: u16 = 48123;
/// Upper bound for the bind retry backoff, so a port that stays busy is retried
/// forever without flooding the log.
const MAX_BIND_RETRY_DELAY_SECS: u64 = 30;

fn bind_retry_delay(attempt: u32) -> Duration {
    let exp = attempt.saturating_sub(1).min(5) as u64;
    Duration::from_secs((1u64 << exp).min(MAX_BIND_RETRY_DELAY_SECS))
}

/// Whether the local webhook server has any reason to listen. The server is idle work for a
/// daemon whose rules never use the trigger, so it only binds while an enabled `webhook` rule
/// exists and is shut down again when the last one is removed or disabled.
pub fn has_enabled_webhook_rules(rules: &[Rule]) -> bool {
    rules
        .iter()
        .any(|rule| rule.is_enabled && matches!(rule.trigger, Some(Trigger::Webhook { .. })))
}

pub struct WebhookServer {
    engine: Arc<Mutex<DaemonEngine>>,
    port: u16,
}

impl WebhookServer {
    pub fn new(engine: Arc<Mutex<DaemonEngine>>, port: u16) -> Self {
        Self { engine, port }
    }

    /// Serves requests until `stop` completes — the supervisor signals it when the last enabled
    /// webhook rule disappears — or the sender is dropped (daemon shutdown). Dropping the
    /// listener on return releases the port.
    pub async fn run(&self, mut stop: tokio::sync::oneshot::Receiver<()>) {
        let addr = format!("127.0.0.1:{}", self.port);
        let mut attempt = 0u32;
        let mut retry_at: Option<tokio::time::Instant> = None;
        let listener = loop {
            if let Some(at) = retry_at {
                tokio::select! {
                    _ = &mut stop => return,
                    _ = tokio::time::sleep_until(at) => {}
                }
            } else if stop.try_recv().is_ok() {
                return;
            }
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
                    retry_at = Some(tokio::time::Instant::now() + delay);
                }
            }
        };

        self.serve(listener, stop).await;
    }

    /// Accepts connections on an already bound listener until `stop` fires. Split from [`run`] so
    /// the request/matching contract can be exercised on an ephemeral port in tests.
    async fn serve(&self, listener: TcpListener, mut stop: tokio::sync::oneshot::Receiver<()>) {
        loop {
            let (mut socket, _) = tokio::select! {
                _ = &mut stop => {
                    crate::logging::info(
                        "WEBHOOK",
                        "Webhook server stopped: no enabled webhook rules",
                    );
                    return;
                }
                accepted = listener.accept() => match accepted {
                    Ok(conn) => conn,
                    Err(_) => continue,
                },
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
                            let _ = eng.trigger(&rule, ExecutionOrigin::Webhook, &format!("Webhook ({})", target_clean), 0, HashMap::new()).await;
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

    #[test]
    fn test_server_is_only_required_by_enabled_webhook_rules() {
        let webhook = Rule {
            trigger: Some(Trigger::Webhook {
                endpoint_path: "backup".to_string(),
            }),
            ..Default::default()
        };
        assert!(has_enabled_webhook_rules(std::slice::from_ref(&webhook)));

        let mut disabled = webhook.clone();
        disabled.is_enabled = false;
        assert!(
            !has_enabled_webhook_rules(&[disabled]),
            "a disabled webhook rule must not keep the server listening"
        );

        assert!(!has_enabled_webhook_rules(&[Rule::default()]));
    }

    #[tokio::test]
    async fn test_serve_triggers_a_webhook_rule_and_releases_the_listener_on_stop() {
        let rule = Rule {
            id: "rule_webhook_test".to_string(),
            name: "Test webhook".to_string(),
            trigger: Some(Trigger::Webhook {
                endpoint_path: "backup".to_string(),
            }),
            ..Default::default()
        };
        let mut engine = DaemonEngine::new(crate::config::AppConfig::default());
        engine.config.rules.push(rule);
        let engine = Arc::new(Mutex::new(engine));

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind test listener");
        let addr = listener.local_addr().expect("listener address");
        let server = WebhookServer::new(Arc::clone(&engine), addr.port());
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            server.serve(listener, stop_rx).await;
        });

        let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
        stream
            .write_all(b"GET /api/v1/trigger/backup HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .expect("write request");
        let mut response = String::new();
        stream.read_to_string(&mut response).await.expect("read response");
        assert!(
            response.starts_with("HTTP/1.1 200"),
            "expected a match for the enabled webhook rule, got: {response}"
        );

        stop_tx.send(()).expect("signal stop");
        task.await.expect("server task");
        assert!(
            tokio::net::TcpStream::connect(addr).await.is_err(),
            "the listener must be closed after the last webhook rule disappears"
        );
    }
}
