//! `NotifyEventListener` — translates Misc events of interest into
//! `notify_request` Redis-stream XADDs. The dispatch logic is pure
//! (testable without Redis) and exposed as
//! [`NotifyEventListener::dispatch_event`]; the live PSUBSCRIBE loop
//! lives in [`NotifyEventListener::run`] and forwards every event it
//! sees into `dispatch_event`.
//!
//! Trigger map (architecture §2.7):
//! - `Misc{kind:"task_state", to:"completed"}` → `trigger_kind:"task_finished"`
//! - `Misc{kind:"task_state", to:"failed"}`    → `trigger_kind:"task_failed"`
//! - `Misc{kind:"briefing_pending"}`           → `trigger_kind:"briefing_pending"` (opt-in)
//! - `Misc{kind:"verifier_verdict", verdict:"fail"}` → `trigger_kind:"verifier_fail"` (opt-in)
//!
//! Channels routing is read from [`super::config::NotifyConfig`] —
//! triggers whose channels list is empty are silently skipped, which is
//! how "opt-in" triggers stay quiet until the operator wires them up.
//!
//! refs: /specs/phase-2/architecture.md §2.7, §2.9
//! refs: /specs/phase-2/stories/story-2.12.md

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::config::NotifyConfig;
use super::worker::NotifyRequest;
use crate::pubsub::RedisPool;

/// Pluggable XADD seam — production wraps [`RedisPool::xadd_json`],
/// tests use a recording impl so the listener stays unit-testable
/// without live Redis.
#[async_trait]
pub trait NotifyDispatch: Send + Sync {
    async fn dispatch(&self, req: NotifyRequest) -> Result<(), String>;
}

/// Production impl: XADD onto the `notify_request` stream.
pub struct RedisNotifyDispatch {
    redis: Arc<RedisPool>,
    stream: String,
}

impl RedisNotifyDispatch {
    pub fn new(redis: Arc<RedisPool>) -> Self {
        Self {
            redis,
            stream: super::worker::NOTIFY_STREAM.into(),
        }
    }
}

#[async_trait]
impl NotifyDispatch for RedisNotifyDispatch {
    async fn dispatch(&self, req: NotifyRequest) -> Result<(), String> {
        self.redis
            .xadd_json(&self.stream, &req)
            .await
            .map(|_id| ())
            .map_err(|e| e.to_string())
    }
}

/// Pure event-stream listener. Production wires
/// `Arc<RedisNotifyDispatch>`; tests pass a recording dispatch.
pub struct NotifyEventListener {
    config: Arc<NotifyConfig>,
    dispatch: Arc<dyn NotifyDispatch>,
}

impl NotifyEventListener {
    pub fn new(config: Arc<NotifyConfig>, dispatch: Arc<dyn NotifyDispatch>) -> Self {
        Self { config, dispatch }
    }

    /// Examine one event payload and, if it matches a configured
    /// trigger with at least one channel registered, fire a
    /// [`NotifyRequest`] XADD.
    ///
    /// `event_data` is the JSON payload of the `Misc` event (what
    /// `events.append`'s `data` field stored). The event_type filter
    /// upstream — only Misc events arrive here — so this function
    /// trusts that and only inspects `kind` / `to` / `verdict`.
    pub async fn dispatch_event(&self, session_id: &str, event_data: &Value) {
        let Some((trigger_kind, task_id)) = match_trigger(event_data) else {
            return;
        };
        let channels = self.config.channels_for(trigger_kind);
        if channels.is_empty() {
            return; // Trigger configured off; no-op.
        }
        let payload = render_payload(trigger_kind, session_id, event_data);
        let req = NotifyRequest {
            trigger_kind: trigger_kind.to_string(),
            task_id,
            payload,
            target_channels: channels.to_vec(),
            target_override: None,
            tenant_id: None,
        };
        if let Err(error) = self.dispatch.dispatch(req).await {
            tracing::warn!(
                trigger_kind = %trigger_kind,
                session_id = %session_id,
                %error,
                "notify_listener: dispatch (XADD) failed",
            );
        }
    }

    /// Long-running entry point — PSUBSCRIBEs to every per-session
    /// event channel (`sh:events:*`) and forwards each `Misc` event
    /// through [`Self::dispatch_event`]. Other event types are
    /// silently dropped here so the worker only sees notify-relevant
    /// traffic.
    ///
    /// Returns when `shutdown` cancels or when the subscription
    /// terminates (e.g., Redis went away — we reconnect after
    /// [`RECONNECT_BACKOFF`]).
    pub async fn run(&self, redis: Arc<RedisPool>, shutdown: CancellationToken) {
        let pattern = RedisPool::events_pattern();
        loop {
            if shutdown.is_cancelled() {
                return;
            }
            let subscription = match redis.psubscribe(pattern).await {
                Ok(sub) => sub,
                Err(error) => {
                    tracing::warn!(
                        %error,
                        pattern,
                        "notify_listener: psubscribe failed; backing off"
                    );
                    if backoff_or_cancel(&shutdown, RECONNECT_BACKOFF).await {
                        return;
                    }
                    continue;
                }
            };
            let mut stream = Box::pin(subscription.into_stream());
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => return,
                    maybe = stream.next() => match maybe {
                        Some(payload) => self.handle_pubsub_payload(&payload).await,
                        None => {
                            tracing::info!("notify_listener: PSUBSCRIBE stream closed; reconnecting");
                            break;
                        }
                    }
                }
            }
            if backoff_or_cancel(&shutdown, RECONNECT_BACKOFF).await {
                return;
            }
        }
    }

    async fn handle_pubsub_payload(&self, raw: &str) {
        // The events module publishes a full `Event` JSON per message
        // (see `events::sqlite::SqliteEventStore::append`'s
        // `publish_event`). Pull `type` and `data` out manually.
        let value: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(error) => {
                tracing::warn!(
                    %error,
                    "notify_listener: failed to parse event payload"
                );
                return;
            }
        };
        if value.get("type").and_then(Value::as_str) != Some("Misc") {
            return;
        }
        let session_id = value
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        let data = value.get("data").cloned().unwrap_or_else(|| json!({}));
        self.dispatch_event(session_id, &data).await;
    }
}

const RECONNECT_BACKOFF: Duration = Duration::from_secs(2);

async fn backoff_or_cancel(shutdown: &CancellationToken, dur: Duration) -> bool {
    tokio::select! {
        _ = shutdown.cancelled() => true,
        _ = tokio::time::sleep(dur) => false,
    }
}

/// Classify one event payload. Returns `Some((trigger_kind, task_id))`
/// when it matches one of the four Phase 2 triggers; `None` otherwise.
fn match_trigger(data: &Value) -> Option<(&'static str, Option<String>)> {
    let kind = data.get("kind").and_then(Value::as_str)?;
    match kind {
        "task_state" => {
            let to = data.get("to").and_then(Value::as_str)?;
            let task_id = data
                .get("task_id")
                .and_then(Value::as_str)
                .map(str::to_string);
            match to {
                "completed" => Some(("task_finished", task_id)),
                "failed" => Some(("task_failed", task_id)),
                _ => None,
            }
        }
        "briefing_pending" => {
            let task_id = data
                .get("task_id")
                .and_then(Value::as_str)
                .map(str::to_string);
            Some(("briefing_pending", task_id))
        }
        "verifier_verdict" => {
            let verdict = data.get("verdict").and_then(Value::as_str)?;
            if verdict != "fail" {
                return None;
            }
            let task_id = data
                .get("task_id")
                .and_then(Value::as_str)
                .map(str::to_string);
            Some(("verifier_fail", task_id))
        }
        _ => None,
    }
}

/// Build the channel-side render of the notify payload. Channels are
/// free to pluck whatever subset of fields applies to their protocol
/// (ntfy reads `title`/`priority`/`tags`; webhook sends the whole
/// blob). Phase 2 keeps the render minimal — the listener doesn't
/// know about per-channel templates yet (DEBT-tracked for a future
/// story when richer templates ship).
fn render_payload(trigger_kind: &str, session_id: &str, source: &Value) -> Value {
    let (title, priority) = match trigger_kind {
        "task_finished" => ("Task finished", "default"),
        "task_failed" => ("Task failed", "high"),
        "briefing_pending" => ("Briefing awaiting confirmation", "default"),
        "verifier_fail" => ("Verifier flagged a failure", "high"),
        _ => ("Notification", "default"),
    };
    json!({
        "title": title,
        "priority": priority,
        "trigger_kind": trigger_kind,
        "session_id": session_id,
        "source": source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingDispatch {
        seen: Mutex<Vec<NotifyRequest>>,
    }

    #[async_trait]
    impl NotifyDispatch for RecordingDispatch {
        async fn dispatch(&self, req: NotifyRequest) -> Result<(), String> {
            self.seen.lock().unwrap().push(req);
            Ok(())
        }
    }

    fn config_with(trigger: &str, channels: &[&str]) -> Arc<NotifyConfig> {
        let mut cfg = NotifyConfig::empty();
        cfg.insert_trigger(trigger, channels.iter().map(|s| s.to_string()).collect());
        Arc::new(cfg)
    }

    #[tokio::test]
    async fn event_listener_emits_notify_request_on_task_completed() {
        let dispatch = Arc::new(RecordingDispatch::default());
        let listener = NotifyEventListener::new(
            config_with("task_finished", &["ntfy", "email"]),
            dispatch.clone(),
        );

        let event = json!({
            "kind": "task_state",
            "to": "completed",
            "task_id": "task-42",
        });
        listener.dispatch_event("sess-1", &event).await;

        let seen = dispatch.seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "exactly one XADD");
        let req = &seen[0];
        assert_eq!(req.trigger_kind, "task_finished");
        assert_eq!(req.task_id.as_deref(), Some("task-42"));
        assert_eq!(req.target_channels, vec!["ntfy", "email"]);
        assert_eq!(req.payload["title"], "Task finished");
        assert_eq!(req.payload["priority"], "default");
    }

    #[tokio::test]
    async fn event_listener_skips_notify_for_unconfigured_trigger() {
        let dispatch = Arc::new(RecordingDispatch::default());
        let listener = NotifyEventListener::new(
            // task_finished IS in config but with an empty channels list.
            config_with("task_finished", &[]),
            dispatch.clone(),
        );

        // (1) Empty channel list — skip
        listener
            .dispatch_event(
                "s",
                &json!({"kind":"task_state","to":"completed","task_id":"t-1"}),
            )
            .await;
        // (2) Trigger not in config at all — skip
        listener
            .dispatch_event(
                "s",
                &json!({"kind":"task_state","to":"failed","task_id":"t-2"}),
            )
            .await;
        // (3) Misc event of an irrelevant kind — skip
        listener
            .dispatch_event("s", &json!({"kind":"some_other_misc"}))
            .await;
        // (4) verifier_verdict with verdict=pass — skip
        listener
            .dispatch_event("s", &json!({"kind":"verifier_verdict","verdict":"pass"}))
            .await;

        let seen = dispatch.seen.lock().unwrap();
        assert!(
            seen.is_empty(),
            "no XADDs for unconfigured / irrelevant events: {seen:?}"
        );
    }

    #[tokio::test]
    async fn event_listener_emits_for_verifier_fail() {
        let dispatch = Arc::new(RecordingDispatch::default());
        let listener =
            NotifyEventListener::new(config_with("verifier_fail", &["ntfy"]), dispatch.clone());

        listener
            .dispatch_event(
                "sess-z",
                &json!({
                    "kind": "verifier_verdict",
                    "verdict": "fail",
                    "task_id": "t-9",
                }),
            )
            .await;

        let seen = dispatch.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].trigger_kind, "verifier_fail");
        assert_eq!(seen[0].target_channels, vec!["ntfy"]);
    }
}
