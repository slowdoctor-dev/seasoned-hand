//! `NotifyWorker` — symmetric outbound counterpart to the
//! [`crate::intake::IntakeRouter`]. Consumes the `notify_request`
//! Redis stream and dispatches each request to the
//! [`crate::channel::NotifySink`] impl for every listed channel.
//!
//! Worker shape mirrors the verifier worker (story 2.18 — closes
//! Phase 1 DEBT #15) so the Phase 2 notify path ships with a correct
//! XREADGROUP consumer from day 1 rather than the polling stub the
//! Phase 1 verifier started with.
//!
//! Retry policy (architecture §2.9 / story 2.12 ACs):
//! - Webhook 5xx (`ChannelError::RemoteRejected{status:5xx}` AND
//!   the channel is `"webhook"`): 1 retry after 30 s.
//! - Anything else (other adapters, 4xx, transport errors): best
//!   effort, no retry.
//! - XACK on success path AND on terminal-error path; only
//!   unparseable Redis payloads stay in the PEL for ops review.
//!
//! refs: /specs/phase-2/architecture.md §2.7, §2.9
//! refs: /specs/phase-2/stories/story-2.12.md

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::store::{NewNotificationSent, NotificationsSentStore, NotifyStoreError};
use crate::channel::{ChannelError, ChannelRegistry, NotifyEvent, NotifyTarget};
use crate::pubsub::RedisPool;

/// Stream name producers push [`NotifyRequest`]s onto.
pub const NOTIFY_STREAM: &str = "notify_request";

/// Redis consumer-group name shared by every NotifyWorker process.
/// One group → load-balanced delivery across consumers; PEL retention
/// across worker crashes / restarts.
pub const NOTIFY_CONSUMER_GROUP: &str = "notify-workers";

/// Default block window for each XREADGROUP call. 5 s matches the
/// verifier worker's idle pacing — long enough to amortise round-trip
/// cost, short enough that shutdown cancels promptly.
const DEFAULT_BLOCK_MS: usize = 5_000;

/// Default batch size per XREADGROUP. Notifies are tiny — 16 fits one
/// outbound burst comfortably.
const DEFAULT_COUNT: usize = 16;

/// Sleep window between failed XREADGROUP attempts (Redis briefly
/// unreachable, etc). Keeps the loop responsive to shutdown.
const READ_ERROR_BACKOFF: Duration = Duration::from_millis(500);

/// Webhook-only retry delay. Architecture §2.9: "1 retry after 30 s
/// (transient case); then mark failed". Tests override via
/// [`NotifyWorker::with_retry_delay`].
pub const DEFAULT_WEBHOOK_RETRY_DELAY: Duration = Duration::from_secs(30);

/// Per-message envelope on the `notify_request` Redis stream. The
/// listener writes one of these per trigger; the worker fans the
/// `target_channels` list across registered [`crate::channel::NotifySink`]s.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyRequest {
    pub trigger_kind: String,
    /// Optional — pre-task notifies (briefing escalations) may fire
    /// before a task is materialised.
    #[serde(default)]
    pub task_id: Option<String>,
    /// Channel-side render of the notify (title, body, priority,
    /// tags). Each `NotifySink` decides what subset of fields applies
    /// to its protocol (ntfy reads Title/Priority/Tags; webhook sends
    /// the whole blob in the body).
    pub payload: Value,
    /// Channel names in the registry the worker should fan out to.
    pub target_channels: Vec<String>,
    /// Optional override of the per-channel [`NotifyTarget`]. When
    /// `None`, the worker calls
    /// [`NotifyWorker::resolve_target`] which reads operator-side
    /// config (`config/notify.toml [channel.<name>].default_target`).
    /// Phase 2 keeps the lookup minimal — see
    /// [`crate::notify::config`] for the per-channel default shape.
    #[serde(default)]
    pub target_override: Option<NotifyTarget>,
    /// Optional tenant id for the audit log row. Empty for single-
    /// operator Phase 2.
    #[serde(default)]
    pub tenant_id: Option<String>,
}

#[derive(Debug, Error)]
pub enum WorkerError {
    #[error("notify store: {0}")]
    Store(#[from] NotifyStoreError),
    #[error("channel: {0}")]
    Channel(#[from] ChannelError),
}

/// Per-channel target resolver — the worker calls this when the
/// [`NotifyRequest::target_override`] is `None`. Production wiring
/// supplies the [`crate::notify::config::NotifyConfig`] read from
/// `config/notify.toml`; tests can plug a minimal closure that returns
/// a fixed target.
pub trait TargetResolver: Send + Sync {
    fn resolve(&self, channel: &str) -> Option<NotifyTarget>;
}

/// Trivial resolver returning a single per-channel target. Used by
/// tests and by the in-tree config-loader fallback when no config
/// file is supplied.
#[derive(Default)]
pub struct StaticTargetResolver {
    map: std::collections::HashMap<String, NotifyTarget>,
}

impl StaticTargetResolver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_target(mut self, channel: impl Into<String>, target: NotifyTarget) -> Self {
        self.map.insert(channel.into(), target);
        self
    }
}

impl TargetResolver for StaticTargetResolver {
    fn resolve(&self, channel: &str) -> Option<NotifyTarget> {
        self.map.get(channel).cloned()
    }
}

/// Runtime tunables — let main.rs read from env without touching the
/// worker's defaults.
#[derive(Debug, Clone)]
pub struct NotifyRuntimeConfig {
    pub stream: String,
    pub group: String,
    pub consumer_prefix: String,
    pub block_ms: usize,
    pub count: usize,
}

impl Default for NotifyRuntimeConfig {
    fn default() -> Self {
        Self {
            stream: NOTIFY_STREAM.into(),
            group: NOTIFY_CONSUMER_GROUP.into(),
            consumer_prefix: "notify".into(),
            block_ms: DEFAULT_BLOCK_MS,
            count: DEFAULT_COUNT,
        }
    }
}

/// Worker handle owned by AppState / main.rs. Internally Arc-shaped so
/// the spawn-helper can fire-and-forget without lifetimes.
#[derive(Clone)]
pub struct NotifyWorker {
    inner: Arc<Inner>,
}

struct Inner {
    channels: Arc<ChannelRegistry>,
    notifications_sent: Arc<NotificationsSentStore>,
    resolver: Arc<dyn TargetResolver>,
    runtime_config: NotifyRuntimeConfig,
    webhook_retry_delay: Duration,
}

impl NotifyWorker {
    pub fn new(
        channels: Arc<ChannelRegistry>,
        notifications_sent: Arc<NotificationsSentStore>,
        resolver: Arc<dyn TargetResolver>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                channels,
                notifications_sent,
                resolver,
                runtime_config: NotifyRuntimeConfig::default(),
                webhook_retry_delay: DEFAULT_WEBHOOK_RETRY_DELAY,
            }),
        }
    }

    pub fn with_runtime_config(mut self, cfg: NotifyRuntimeConfig) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.runtime_config = cfg;
        }
        self
    }

    pub fn with_retry_delay(mut self, d: Duration) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.webhook_retry_delay = d;
        }
        self
    }

    pub fn runtime_config(&self) -> &NotifyRuntimeConfig {
        &self.inner.runtime_config
    }

    /// Process exactly one [`NotifyRequest`]. Splitting this out from
    /// the Redis loop lets the listener tests (and the live-Redis
    /// `#[ignore]` worker tests) exercise the dispatch pipeline
    /// directly.
    ///
    /// Returns the count of *successful* dispatches. Per-channel
    /// failures (after retry, where applicable) are logged + persisted
    /// to `notifications_sent` but don't bubble out — the worker is
    /// best-effort.
    pub async fn handle_request(&self, req: &NotifyRequest) -> usize {
        let mut ok_count = 0_usize;
        for channel_name in &req.target_channels {
            let outcome = self.dispatch_one(channel_name, req).await;
            match outcome {
                Ok(()) => ok_count += 1,
                Err(error) => {
                    tracing::warn!(
                        channel = %channel_name,
                        trigger_kind = %req.trigger_kind,
                        task_id = ?req.task_id,
                        %error,
                        "notify dispatch failed (after any retries)",
                    );
                }
            }
        }
        ok_count
    }

    async fn dispatch_one(
        &self,
        channel_name: &str,
        req: &NotifyRequest,
    ) -> Result<(), WorkerError> {
        let Some(sink) = self.inner.channels.get_notify(channel_name) else {
            // Persist the miss so operators can see misconfigured
            // routes without rerunning the trigger.
            self.persist_outcome(
                channel_name,
                req,
                None,
                false,
                Some(format!("channel_not_registered: {channel_name}")),
            )
            .await?;
            return Err(WorkerError::Channel(ChannelError::Internal(format!(
                "channel_not_registered: {channel_name}"
            ))));
        };
        let target = self.resolve_target(channel_name, req)?;
        let event = NotifyEvent {
            trigger_kind: req.trigger_kind.clone(),
            task_id: req.task_id.clone(),
            payload: req.payload.clone(),
        };

        // First attempt.
        let first = sink.notify(&target, &event).await;
        let (ok, error) = match first {
            Ok(_) => (true, None),
            Err(err) => {
                let retryable = is_webhook_5xx(channel_name, &err)
                    && self.inner.webhook_retry_delay > Duration::ZERO;
                if retryable {
                    tokio::time::sleep(self.inner.webhook_retry_delay).await;
                    match sink.notify(&target, &event).await {
                        Ok(_) => (true, None),
                        Err(err2) => (false, Some(err2)),
                    }
                } else {
                    (false, Some(err))
                }
            }
        };
        self.persist_outcome(
            channel_name,
            req,
            Some(target),
            ok,
            error.as_ref().map(|e| e.to_string()),
        )
        .await?;
        if let Some(e) = error {
            return Err(WorkerError::Channel(e));
        }
        Ok(())
    }

    fn resolve_target(
        &self,
        channel: &str,
        req: &NotifyRequest,
    ) -> Result<NotifyTarget, WorkerError> {
        if let Some(t) = req.target_override.clone() {
            return Ok(t);
        }
        self.inner.resolver.resolve(channel).ok_or_else(|| {
            WorkerError::Channel(ChannelError::Internal(format!(
                "no_target_for_channel: {channel}"
            )))
        })
    }

    async fn persist_outcome(
        &self,
        channel: &str,
        req: &NotifyRequest,
        target: Option<NotifyTarget>,
        ok: bool,
        error: Option<String>,
    ) -> Result<(), WorkerError> {
        let row = NewNotificationSent {
            tenant_id: req.tenant_id.clone(),
            task_id: req.task_id.clone(),
            trigger_kind: req.trigger_kind.clone(),
            channel: channel.to_string(),
            target,
            payload: Some(req.payload.clone()),
            ok,
            error,
            sent_at: now_micros(),
        };
        self.inner.notifications_sent.insert(row).await?;
        Ok(())
    }

    /// Long-running entrypoint. Returns `Ok(())` immediately if the
    /// consumer group fails to materialise after several retries —
    /// callers can keep the worker registered and the rest of the
    /// kernel keeps running.
    ///
    /// Per-message lifecycle:
    /// 1. XREADGROUP COUNT N BLOCK M
    /// 2. Per entry: parse JSON → `handle_request` → XACK
    /// 3. Unparseable payloads: log + leave in PEL for ops review
    ///    (do NOT XACK — that would silently drop a malformed payload
    ///    a sysadmin would want to see).
    pub async fn run(&self, redis: Arc<RedisPool>, shutdown: CancellationToken) {
        let cfg = self.inner.runtime_config.clone();
        let consumer_id = make_consumer_id(&cfg.consumer_prefix);
        tracing::info!(
            stream = %cfg.stream,
            group = %cfg.group,
            consumer = %consumer_id,
            "notify worker booting",
        );

        let mut group_ready = false;
        let mut in_flight: Vec<JoinHandle<()>> = Vec::new();

        while !shutdown.is_cancelled() {
            if !group_ready {
                let res = tokio::select! {
                    _ = shutdown.cancelled() => break,
                    out = redis.xgroup_create_mkstream(&cfg.stream, &cfg.group) => out,
                };
                match res {
                    Ok(()) => group_ready = true,
                    Err(error) => {
                        tracing::warn!(
                            %error,
                            stream = %cfg.stream,
                            group = %cfg.group,
                            "notify worker: xgroup create failed; backing off",
                        );
                        if backoff_or_cancel(&shutdown, READ_ERROR_BACKOFF).await {
                            break;
                        }
                        continue;
                    }
                }
            }

            let read = tokio::select! {
                _ = shutdown.cancelled() => break,
                out = redis.xreadgroup_payloads(
                    &cfg.stream,
                    &cfg.group,
                    &consumer_id,
                    cfg.count,
                    cfg.block_ms,
                ) => out,
            };
            let entries = match read {
                Ok(v) => v,
                Err(error) => {
                    tracing::warn!(
                        %error,
                        stream = %cfg.stream,
                        group = %cfg.group,
                        "notify worker: xreadgroup failed; backing off",
                    );
                    if backoff_or_cancel(&shutdown, READ_ERROR_BACKOFF).await {
                        break;
                    }
                    continue;
                }
            };

            in_flight.retain(|h| !h.is_finished());
            for (msg_id, payload) in entries {
                let worker = self.clone();
                let redis_c = redis.clone();
                let cfg_c = cfg.clone();
                in_flight.push(tokio::spawn(async move {
                    process_entry(worker, redis_c, cfg_c, msg_id, payload).await;
                }));
            }
        }

        for h in in_flight {
            if let Err(error) = h.await {
                tracing::warn!(%error, "notify worker: in-flight task join failed");
            }
        }
    }
}

/// Process exactly one Redis stream entry.
///
/// Lifecycle: parse → handle → XACK. Unparseable payloads SKIP the
/// XACK (architecture intent per spec: PEL retention surfaces broken
/// messages to ops); terminal handler "errors" are already absorbed
/// inside [`NotifyWorker::handle_request`] (best-effort), so the
/// happy + sad paths both XACK once handling is complete.
async fn process_entry(
    worker: NotifyWorker,
    redis: Arc<RedisPool>,
    cfg: NotifyRuntimeConfig,
    msg_id: String,
    payload: Vec<u8>,
) {
    let req: NotifyRequest = match serde_json::from_slice(&payload) {
        Ok(req) => req,
        Err(error) => {
            tracing::warn!(
                %msg_id,
                %error,
                "notify worker: dropping malformed notify_request (PEL retains)",
            );
            return; // No XACK — operator sees the PEL entry.
        }
    };

    let _ok_count = worker.handle_request(&req).await;
    if let Err(error) = redis.xack(&cfg.stream, &cfg.group, &msg_id).await {
        tracing::warn!(
            %msg_id,
            %error,
            "notify worker: XACK failed; message will stay in PEL until next consumer",
        );
    }
}

fn make_consumer_id(prefix: &str) -> String {
    // Avoid the `hostname` crate dependency — host identity for the
    // consumer-id slot is best-effort (the prefix + pid already
    // distinguish co-located workers on one box). Falls back to env
    // `HOSTNAME` (set by most shells) then "unknown".
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".into());
    format!("{prefix}-{host}-{}", std::process::id())
}

/// Match the verifier worker's `backoff_or_cancel` helper — returns
/// `true` if the shutdown was signalled during the sleep.
async fn backoff_or_cancel(shutdown: &CancellationToken, dur: Duration) -> bool {
    tokio::select! {
        _ = shutdown.cancelled() => true,
        _ = tokio::time::sleep(dur) => false,
    }
}

fn is_webhook_5xx(channel: &str, error: &ChannelError) -> bool {
    if channel != "webhook" {
        return false;
    }
    matches!(error, ChannelError::RemoteRejected { status, .. } if (500..600).contains(status))
}

fn now_micros() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_micros()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{
        ChannelRegistration, ChannelRegistry, NotifyEvent, NotifyReceipt, NotifySink,
    };
    use crate::db;
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// In-memory NotifySink for unit tests.
    struct StubNotifySink {
        name: &'static str,
        ok: bool,
        calls: AtomicUsize,
    }

    impl StubNotifySink {
        fn new(name: &'static str, ok: bool) -> Self {
            Self {
                name,
                ok,
                calls: AtomicUsize::new(0),
            }
        }
        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl NotifySink for StubNotifySink {
        fn name(&self) -> &'static str {
            self.name
        }
        async fn notify(
            &self,
            _target: &NotifyTarget,
            _event: &NotifyEvent,
        ) -> Result<NotifyReceipt, ChannelError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.ok {
                Ok(NotifyReceipt {
                    channel: self.name.into(),
                    external_id: Some(format!("{}-ok", self.name)),
                    sent_at: 0,
                    raw_response: json!({}),
                })
            } else {
                Err(ChannelError::RemoteRejected {
                    status: 503,
                    message: "fake".into(),
                })
            }
        }
    }

    async fn worker_with_stubs(
        sink: Arc<StubNotifySink>,
        channel_name: &'static str,
    ) -> NotifyWorker {
        let pool = db::open(":memory:").await.unwrap();
        let store = Arc::new(NotificationsSentStore::new(pool));
        let mut reg = ChannelRegistry::new();
        reg.register(
            ChannelRegistration::new(channel_name).with_notify(sink as Arc<dyn NotifySink>),
        );
        let resolver = Arc::new(StaticTargetResolver::new().with_target(
            channel_name,
            NotifyTarget {
                channel: channel_name.into(),
                target_ref: "topic-x".into(),
                metadata: json!({}),
            },
        ));
        NotifyWorker::new(Arc::new(reg), store, resolver)
    }

    fn request_for(channel: &'static str) -> NotifyRequest {
        NotifyRequest {
            trigger_kind: "task_finished".into(),
            task_id: Some("t-1".into()),
            payload: json!({"text": "done"}),
            target_channels: vec![channel.into()],
            target_override: None,
            tenant_id: None,
        }
    }

    #[tokio::test]
    async fn handle_request_dispatches_to_registered_sink() {
        let sink = Arc::new(StubNotifySink::new("ntfy", true));
        let worker = worker_with_stubs(sink.clone(), "ntfy").await;

        let ok = worker.handle_request(&request_for("ntfy")).await;
        assert_eq!(ok, 1);
        assert_eq!(sink.call_count(), 1);
    }

    #[tokio::test]
    async fn handle_request_persists_failure_without_panicking() {
        let sink = Arc::new(StubNotifySink::new("ntfy", false));
        // No retry for non-webhook 503.
        let worker = worker_with_stubs(sink.clone(), "ntfy")
            .await
            .with_retry_delay(Duration::from_millis(0));

        let ok = worker.handle_request(&request_for("ntfy")).await;
        assert_eq!(ok, 0, "failed dispatch returns 0 ok count");
        assert_eq!(sink.call_count(), 1, "non-webhook 503: no retry");
    }

    #[tokio::test]
    async fn handle_request_skips_unregistered_channel() {
        let sink = Arc::new(StubNotifySink::new("ntfy", true));
        let worker = worker_with_stubs(sink.clone(), "ntfy").await;

        // Request asks for "email" — never registered.
        let mut req = request_for("ntfy");
        req.target_channels = vec!["email".into()];
        let ok = worker.handle_request(&req).await;
        assert_eq!(ok, 0);
        assert_eq!(
            sink.call_count(),
            0,
            "no dispatch attempted on unknown channel"
        );
    }
}
