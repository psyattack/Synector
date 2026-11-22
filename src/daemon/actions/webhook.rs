use std::collections::HashMap;
use std::sync::OnceLock;

use super::ActionRunner;
use crate::models::Action;

static WEBHOOK_CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();

pub(super) fn webhook_client() -> Result<&'static reqwest::Client, String> {
    WEBHOOK_CLIENT
        .get_or_init(|| {
            crate::updater::init_tls();
            reqwest::Client::builder()
                .user_agent(concat!("Synector/", env!("CARGO_PKG_VERSION")))
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .map_err(|e| format!("Failed to create HTTP client: {e}"))
        })
        .as_ref()
        .map_err(|err| err.clone())
}

/// Builds (but does not send) an HTTP webhook request. Kept separate from the send so
/// tests can prove that header values and payloads stay data — there is no shell and
/// therefore nothing to inject into.
pub(super) fn build_webhook_request(
    client: &reqwest::Client,
    url: &str,
    method: &str,
    headers: &[(String, String)],
    body: Option<&str>,
) -> Result<reqwest::Request, String> {
    use reqwest::header::{HeaderName, HeaderValue, CONTENT_TYPE};

    let method_name = method.trim().to_ascii_uppercase();
    let method = reqwest::Method::from_bytes(method_name.as_bytes())
        .map_err(|_| format!("Invalid webhook HTTP method '{method_name}'"))?;
    let mut request = client.request(method, url);
    let mut has_content_type = false;
    for (name, value) in headers {
        let name_trimmed = name.trim();
        let header_name = HeaderName::from_bytes(name_trimmed.as_bytes())
            .map_err(|_| format!("Invalid webhook header name '{name_trimmed}'"))?;
        let header_value = HeaderValue::from_str(value)
            .map_err(|_| format!("Invalid value for webhook header '{name_trimmed}'"))?;
        if header_name == CONTENT_TYPE {
            has_content_type = true;
        }
        request = request.header(header_name, header_value);
    }
    if let Some(body) = body {
        if !has_content_type {
            request = request.header(CONTENT_TYPE, "application/json");
        }
        request = request.body(body.to_string());
    }
    request
        .build()
        .map_err(|e| format!("Invalid webhook request: {e}"))
}

pub(super) async fn execute(action: &Action, context: &HashMap<String, String>) -> Result<(), String> {
    match action {
        Action::HttpWebhook {
            url,
            method,
            headers,
            payload,
        } => {
            let target_url = ActionRunner::interpolate_tokens(url, context);
            let body = payload.as_ref().map(|p| ActionRunner::interpolate_tokens(p, context));
            let header_pairs: Vec<(String, String)> = headers
                .iter()
                .flatten()
                .map(|(name, value)| {
                    (
                        ActionRunner::interpolate_tokens(name, context),
                        ActionRunner::interpolate_tokens(value, context),
                    )
                })
                .collect();

            let client = webhook_client()?;
            let request = build_webhook_request(
                client,
                &target_url,
                method,
                &header_pairs,
                body.as_deref(),
            )?;
            let response = client
                .execute(request)
                .await
                .map_err(|e| format!("Webhook request failed: {e}"))?;
            if !response.status().is_success() {
                return Err(format!(
                    "Webhook returned HTTP {}",
                    response.status().as_u16()
                ));
            }
            Ok(())
        }
        _ => unreachable!("webhook handler received a non-webhook action"),
    }
}
