//! `NtfyChannel` — push-notification channel that POSTs to
//! [ntfy.sh](https://ntfy.sh/) (or a self-hosted ntfy server).
//!
//! Phase 2 notify-only — ntfy has no inbound or artifact-delivery
//! semantics, so this channel implements only [`NotifySink`]. The
//! [`crate::notify::NotifyWorker`] (story 2.12) routes
//! `NotifyRequest` envelopes from the `notify_request` Redis stream
//! into per-channel [`NotifySink::notify`] calls — ntfy is one of the
//! sinks; webhook + email cover the others.
//!
//! Wire shape: `POST {host}/{topic}` with body = the
//! [`NotifyEvent::payload`] JSON serialised, and the channel-side
//! `Title:` / `Priority:` / `Tags:` headers derived from
//! [`NotifyEvent::metadata`] (see [`build_headers`]). ntfy's response
//! body is a small JSON envelope including `"id": "<msg-id>"`; we
//! surface that as the [`NotifyReceipt::external_id`] for audit.
//!
//! refs: /specs/phase-2/architecture.md §2.7 (channel matrix), §2.9
//! refs: /specs/phase-2/stories/story-2.12.md

use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use reqwest::Client;
use serde_json::{Value, json};

use super::{ChannelError, NotifyEvent, NotifyReceipt, NotifySink, NotifyTarget};

/// Channel name registered into the [`crate::channel::ChannelRegistry`].
pub const CHANNEL_NAME: &str = "ntfy";

/// Default ntfy host. Operators override per-deployment via the
/// `NTFY_HOST` env (see `register_ntfy_channel` in the server crate).
pub const DEFAULT_HOST: &str = "https://ntfy.sh";

/// Per-request HTTP timeout for ntfy POSTs. Architecture §7 budgets
/// notify deliveries at < 500 ms p95 — generous-but-not-unbounded gives
/// flaky network paths a fair chance without stalling the worker.
const HTTP_TIMEOUT_SECS: u64 = 15;

/// Build with a default reqwest client (rustls-tls, 15 s timeout).
pub struct NtfyChannel {
    host: String,
    http: Client,
}

impl NtfyChannel {
    pub fn new(host: impl Into<String>, http: Client) -> Self {
        Self {
            host: trim_trailing_slash(host.into()),
            http,
        }
    }

    /// Convenience for production / tests: build with a sensible
    /// default reqwest client. Falls back to a basic builder if the
    /// custom config fails.
    pub fn with_default_client(host: impl Into<String>) -> Self {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT_SECS))
            .use_rustls_tls()
            .build()
            .unwrap_or_else(|_| Client::new());
        Self::new(host, http)
    }

    pub fn host(&self) -> &str {
        &self.host
    }
}

#[async_trait]
impl NotifySink for NtfyChannel {
    fn name(&self) -> &'static str {
        CHANNEL_NAME
    }

    async fn notify(
        &self,
        target: &NotifyTarget,
        event: &NotifyEvent,
    ) -> Result<NotifyReceipt, ChannelError> {
        let topic = target.target_ref.trim();
        if topic.is_empty() {
            return Err(ChannelError::RemoteRejected {
                status: 400,
                message: "empty_topic".into(),
            });
        }
        let url = format!("{}/{}", self.host, topic);

        let body = serde_json::to_vec(&event.payload)
            .map_err(|e| ChannelError::Decode(format!("payload serialize: {e}")))?;

        let mut req = self
            .http
            .post(&url)
            .header("Content-Type", "application/json")
            .body(body);
        for (k, v) in build_headers(&target.metadata, &event.metadata_or_payload()) {
            req = req.header(k, v);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| ChannelError::Http(e.to_string()))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| ChannelError::Http(e.to_string()))?;
        if !status.is_success() {
            return Err(ChannelError::RemoteRejected {
                status: status.as_u16(),
                message: truncate(&text, 200),
            });
        }
        let raw: Value = serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text }));
        let external_id = raw
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| Some(format!("ntfy:{}", status.as_u16())));
        Ok(NotifyReceipt {
            channel: CHANNEL_NAME.into(),
            external_id,
            sent_at: now_micros(),
            raw_response: raw,
        })
    }
}

/// Collect `Title:`, `Priority:`, `Tags:` headers from a metadata blob.
///
/// Per architecture §2.7 the operator-facing notify config supplies
/// these via `NotifyTarget.metadata` (channel-side defaults) and / or
/// `NotifyEvent.metadata` (per-trigger overrides). We accept either —
/// the per-event side wins.
fn build_headers(target_meta: &Value, event_meta: &Value) -> Vec<(&'static str, String)> {
    let mut out = Vec::with_capacity(3);
    if let Some(title) = pick_string(event_meta, target_meta, "title") {
        out.push(("Title", title));
    }
    if let Some(priority) = pick_string(event_meta, target_meta, "priority") {
        out.push(("Priority", priority));
    }
    if let Some(tags) = pick_tags(event_meta, target_meta) {
        out.push(("Tags", tags));
    }
    out
}

fn pick_string(event: &Value, target: &Value, key: &str) -> Option<String> {
    event
        .get(key)
        .and_then(Value::as_str)
        .or_else(|| target.get(key).and_then(Value::as_str))
        .map(str::to_string)
}

/// Tags can arrive either as `"warning,robot"` or as a JSON array
/// `["warning","robot"]`. ntfy expects a comma-separated string in the
/// `Tags:` header.
fn pick_tags(event: &Value, target: &Value) -> Option<String> {
    fn from(value: &Value) -> Option<String> {
        match value {
            Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
            Value::Array(items) => {
                let parts: Vec<String> = items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                if parts.is_empty() {
                    None
                } else {
                    Some(parts.join(","))
                }
            }
            _ => None,
        }
    }
    event
        .get("tags")
        .and_then(from)
        .or_else(|| target.get("tags").and_then(from))
}

fn trim_trailing_slash(mut s: String) -> String {
    while s.ends_with('/') {
        s.pop();
    }
    s
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect()
    }
}

fn now_micros() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_micros()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Internal helper trait — the `NotifyEvent` shape doesn't expose a
/// dedicated `metadata` field today (see `channel::notify::NotifyEvent`).
/// Treat the event's `payload` as the metadata source so callers can
/// pass `{"title": "...", "priority": "high"}` through without a
/// schema change. If a separate `metadata` field is added later, the
/// trait impl just gets updated.
trait NotifyEventMetaExt {
    fn metadata_or_payload(&self) -> Value;
}

impl NotifyEventMetaExt for NotifyEvent {
    fn metadata_or_payload(&self) -> Value {
        self.payload.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{NotifyEvent, NotifyTarget};
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn target(topic: &str) -> NotifyTarget {
        NotifyTarget {
            channel: CHANNEL_NAME.into(),
            target_ref: topic.into(),
            metadata: json!({}),
        }
    }

    fn event_with(payload: Value) -> NotifyEvent {
        NotifyEvent {
            trigger_kind: "task_finished".into(),
            task_id: Some("t-1".into()),
            payload,
        }
    }

    #[tokio::test]
    async fn ntfy_channel_posts_to_topic() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/alerts"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "ntfy-msg-42",
                "topic": "alerts",
            })))
            .mount(&server)
            .await;

        let channel = NtfyChannel::new(server.uri(), Client::new());
        let receipt = channel
            .notify(&target("alerts"), &event_with(json!({"text": "hi"})))
            .await
            .expect("notify ok");
        assert_eq!(receipt.channel, CHANNEL_NAME);
        assert_eq!(receipt.external_id.as_deref(), Some("ntfy-msg-42"));
    }

    /// wiremock matches headers case-insensitively but values are
    /// matched exactly — if the channel fails to set any of the three
    /// headers (or sets a wrong value), the matcher misses and the
    /// fallback 200 mock catches the request. We then inspect the
    /// captured request directly for stronger assertions.
    #[tokio::test]
    async fn ntfy_channel_sets_title_priority_tags() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/alerts"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "ntfy-msg-43" })))
            .mount(&server)
            .await;

        let channel = NtfyChannel::new(server.uri(), Client::new());
        let event = event_with(json!({
            "title": "Task done",
            "priority": "high",
            "tags": ["white_check_mark", "robot"],
            "text": "ok"
        }));
        let receipt = channel.notify(&target("alerts"), &event).await.expect("ok");
        assert_eq!(receipt.external_id.as_deref(), Some("ntfy-msg-43"));

        // Inspect the captured request directly — wiremock's matchers
        // run before the response template, so a missing header would
        // still 200 here. The explicit-inspect form is more diagnostic.
        let requests = server.received_requests().await.expect("requests captured");
        assert_eq!(requests.len(), 1, "exactly one notify POST");
        let req = &requests[0];
        let header_eq = |name: &str, expected: &str| -> bool {
            req.headers.get(name).and_then(|v| v.to_str().ok()) == Some(expected)
        };
        assert!(header_eq("Title", "Task done"), "Title header missing");
        assert!(header_eq("Priority", "high"), "Priority header missing");
        assert!(
            header_eq("Tags", "white_check_mark,robot"),
            "Tags header CSV-joined: got {:?}",
            req.headers.get("Tags")
        );
    }

    #[tokio::test]
    async fn ntfy_channel_rejects_empty_topic() {
        let channel = NtfyChannel::new("https://ntfy.sh", Client::new());
        let err = channel
            .notify(&target(""), &event_with(json!({})))
            .await
            .expect_err("empty topic rejected");
        match err {
            ChannelError::RemoteRejected { status, message } => {
                assert_eq!(status, 400);
                assert_eq!(message, "empty_topic");
            }
            other => panic!("expected RemoteRejected, got {other:?}"),
        }
    }
}
