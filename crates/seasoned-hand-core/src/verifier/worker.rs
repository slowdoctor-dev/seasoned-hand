//! Verifier Worker — owns the long-running Tokio task that consumes
//! `VerifyRequest`s and produces `verifications` rows + `Misc{kind:
//! "verifier_verdict"}` events.
//!
//! Public surface:
//! - [`Worker::new`] — assemble deps
//! - [`Worker::handle_request`] — process exactly one request
//!   (testable directly without Redis)
//! - [`Worker::run`] — block on Redis Streams `verify_request`,
//!   per-session FIFO + global semaphore + watchdog
//!
//! Architecture: §2.4 (worker shape), §2.4.4 (fresh context),
//! §2.4.5 (verdict handling — Gate downstream owns transitions),
//! §7 (latency budget), §8 (failure modes).
//!
//! refs: /specs/phase-1/stories/story-1.9b.md

use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use serde_json::{Value, json};
use thiserror::Error;
use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::cost::CostClient;
use crate::events::{EventError, EventStore, EventType, NewEvent, sqlite::SqliteEventStore};
use crate::llm::{LlmClient, LlmError, types::*};
use crate::plan::{Phase, PlanError, PlanManager, PlanMutationSource};
use crate::pubsub::RedisPool;
use crate::router::{SlotName, SlotRouter};
use crate::sandbox::SandboxClient;

use super::context::{ContextBuildError, build_fresh_context};
use super::parse::{Verdict, parse_verdict};
use super::persistence::VerifierPersistenceError;
use super::{NewVerification, VerificationStore, VerifyRequest};

/// Hard wallclock cap per verification before the watchdog fires
/// (architecture §8). Tests override via [`Worker::with_watchdog`].
pub const DEFAULT_WATCHDOG: Duration = Duration::from_secs(60);

/// `max_tokens` for verifier completions — the schema is small JSON.
const VERIFIER_MAX_TOKENS: u32 = 1024;

/// Strict-mode retry suffix appended to the user message when the
/// first parse fails.
const STRICT_RETRY_SUFFIX: &str =
    "\n\nRespond with ONLY a JSON object matching the schema. No prose.";

/// Stream name producers push `VerifyRequest`s onto (Phase 1 1.10-1.12).
pub const VERIFY_STREAM: &str = "verify_request";

/// Redis consumer-group name shared by every Verifier Worker process.
/// One group → load-balanced delivery across consumers; PEL retention
/// across worker crashes / restarts.
pub const VERIFIER_CONSUMER_GROUP: &str = "verifier-workers";

/// Backoff sleep between failed XREADGROUP attempts (e.g. Redis briefly
/// unreachable). Keeps the loop responsive to shutdown cancellation.
const READ_ERROR_BACKOFF: Duration = Duration::from_millis(500);

/// Tunables for the live XREADGROUP consumer loop. Defaults match
/// architecture §7 / story 2.18; values come from env on Worker boot
/// via [`VerifierRuntimeConfig::from_env`].
#[derive(Debug, Clone)]
pub struct VerifierRuntimeConfig {
    /// Redis stream name to consume from. Tests override for isolation.
    pub stream: String,
    /// Redis consumer-group name. Tests override for isolation.
    pub group: String,
    /// Consumer-id prefix; the full consumer id is
    /// `{prefix}-{hostname}-{pid}`.
    pub consumer_prefix: String,
    /// Global cap on concurrent `handle_request` runs across all
    /// sessions (per-session FIFO is enforced separately).
    pub max_concurrency: usize,
    /// `BLOCK` argument (milliseconds) for each XREADGROUP call.
    pub consumer_block_ms: usize,
    /// `COUNT` argument for each XREADGROUP call.
    pub read_count: usize,
}

impl Default for VerifierRuntimeConfig {
    fn default() -> Self {
        Self {
            stream: VERIFY_STREAM.to_string(),
            group: VERIFIER_CONSUMER_GROUP.to_string(),
            consumer_prefix: "worker".to_string(),
            max_concurrency: 2,
            consumer_block_ms: 5000,
            read_count: 16,
        }
    }
}

impl VerifierRuntimeConfig {
    /// Load tunables from environment with sensible defaults. Unset or
    /// unparseable env values fall back to defaults — boot must succeed
    /// even with a totally bare environment.
    pub fn from_env() -> Self {
        fn env_usize(name: &str, default: usize) -> usize {
            std::env::var(name)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        }
        let defaults = Self::default();
        Self {
            stream: defaults.stream,
            group: defaults.group,
            consumer_prefix: defaults.consumer_prefix,
            max_concurrency: env_usize("VERIFIER_MAX_CONCURRENCY", defaults.max_concurrency),
            consumer_block_ms: env_usize("VERIFIER_CONSUMER_BLOCK_MS", defaults.consumer_block_ms),
            read_count: env_usize("VERIFIER_READ_COUNT", defaults.read_count),
        }
    }
}

#[derive(Debug, Error)]
pub enum WorkerError {
    #[error("context: {0}")]
    Context(#[from] ContextBuildError),
    #[error("llm: {0}")]
    Llm(#[from] LlmError),
    #[error("plan: {0}")]
    Plan(#[from] PlanError),
    #[error("persistence: {0}")]
    Persistence(#[from] VerifierPersistenceError),
    #[error("event: {0}")]
    Event(#[from] EventError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Bundle of dependencies the worker needs. Built from `AppState` at
/// server bootstrap; tests construct it manually with mocked pieces.
#[derive(Clone)]
pub struct WorkerDeps {
    pub plan_manager: Arc<PlanManager>,
    pub events: Arc<SqliteEventStore>,
    pub sandbox: Arc<SandboxClient>,
    pub verifications: Arc<VerificationStore>,
    pub cost: Arc<CostClient>,
    pub system_prompt: Arc<String>,
    pub verifier_slot_model: String,
    pub verifier_llm: LlmClient,
    pub cancel_tokens: Arc<DashMap<String, CancellationToken>>,
}

impl WorkerDeps {
    /// Build from a `SlotRouter` + the shared dep arcs. Picks the
    /// verifier slot's routing target so the LLM client points at the
    /// right Bifrost alias.
    #[allow(clippy::too_many_arguments)]
    pub fn from_router(
        router: &SlotRouter,
        plan_manager: Arc<PlanManager>,
        events: Arc<SqliteEventStore>,
        sandbox: Arc<SandboxClient>,
        verifications: Arc<VerificationStore>,
        cost: Arc<CostClient>,
        system_prompt: Arc<String>,
        cancel_tokens: Arc<DashMap<String, CancellationToken>>,
    ) -> Self {
        let v_slot = router.resolve(SlotName::Verifier);
        let verifier_llm = LlmClient::new(v_slot.base_url.clone(), v_slot.api_key.clone());
        Self {
            plan_manager,
            events,
            sandbox,
            verifications,
            cost,
            system_prompt,
            verifier_slot_model: v_slot.model.clone(),
            verifier_llm,
            cancel_tokens,
        }
    }
}

#[derive(Clone)]
pub struct Worker {
    deps: WorkerDeps,
    watchdog: Duration,
    runtime_config: VerifierRuntimeConfig,
}

impl Worker {
    pub fn new(deps: WorkerDeps) -> Self {
        Self {
            deps,
            watchdog: DEFAULT_WATCHDOG,
            runtime_config: VerifierRuntimeConfig::default(),
        }
    }

    pub fn with_watchdog(mut self, d: Duration) -> Self {
        self.watchdog = d;
        self
    }

    /// Override the live-loop tunables (stream / group / concurrency /
    /// blocking). Production callers build the config via
    /// [`VerifierRuntimeConfig::from_env`]; tests use per-test stream
    /// names so live-Redis runs don't poison each other.
    pub fn with_runtime_config(mut self, cfg: VerifierRuntimeConfig) -> Self {
        self.runtime_config = cfg;
        self
    }

    /// Expose the worker's runtime tunables (read-only). Server boot
    /// wiring uses this to log the values it actually applied.
    pub fn runtime_config(&self) -> &VerifierRuntimeConfig {
        &self.runtime_config
    }

    /// Process exactly one verifier request end-to-end. Returns the
    /// freshly persisted verification id. Splitting `run` and
    /// `handle_request` lets tests exercise the full pipeline without
    /// Redis Streams.
    pub async fn handle_request(&self, req: &VerifyRequest) -> Result<String, WorkerError> {
        let cost_before = self.cost_snapshot_cents().await;

        let messages = build_fresh_context(
            &self.deps.plan_manager,
            &self.deps.events,
            &self.deps.sandbox,
            &self.deps.system_prompt,
            req,
        )
        .await?;

        let Some(verdict) = self.call_with_retry(req, messages).await else {
            self.deps
                .events
                .append(NewEvent {
                    session_id: req.session_id.clone(),
                    event_type: EventType::Misc,
                    source: "verifier".to_string(),
                    data: json!({
                        "kind":"verifier_cancelled",
                        "trigger_kind": req.trigger.kind_str(),
                        "triggered_at_event_id": req.triggered_at_event_id,
                    }),
                })
                .await?;
            return Ok("cancelled".to_string());
        };

        // If the verifier suggested a plan update, apply it BEFORE
        // persisting/emitting so the downstream Gate (story 1.10) sees
        // the new plan ordering.
        if verdict.verdict == super::VerdictKind::Fail
            && let Some(suggested) = verdict.suggested_plan_update.as_ref()
            && let Some(phases) = parse_suggested_phases(suggested)
            && !phases.is_empty()
        {
            let _ = self
                .deps
                .plan_manager
                .update(&req.session_id, phases, PlanMutationSource::Verifier)
                .await;
        }

        let cost_after = self.cost_snapshot_cents().await;
        let cost_cents = cost_after.saturating_sub(cost_before);

        let new_row = NewVerification {
            session_id: req.session_id.clone(),
            triggered_at_event_id: req.triggered_at_event_id as i64,
            trigger: req.trigger.clone(),
            verdict: verdict.verdict,
            reason: verdict.reason.clone(),
            evidence_event_ids: verdict.evidence_event_ids.clone(),
            suggested_plan_update: verdict.suggested_plan_update.clone(),
            model_id: self.deps.verifier_slot_model.clone(),
            cost_cents,
        };
        let verification_id = self.deps.verifications.insert(new_row).await?;

        emit_verifier_verdict_event(
            &self.deps.events,
            &req.session_id,
            req.trigger.kind_str(),
            &verification_id,
            &verdict,
        )
        .await?;

        Ok(verification_id)
    }

    /// Long-running entrypoint. Returns Ok(()) immediately when
    /// `verifier_enabled` is false (so callers can spawn unconditionally).
    ///
    /// Story 2.18 (closes Phase 1 DEBT #15): consumes `verify_request`
    /// entries via `XREADGROUP GROUP <group> <consumer> BLOCK <ms>
    /// COUNT <n> STREAMS <stream> >`. Per-session FIFO via
    /// `DashMap<SessionId, Arc<Mutex<()>>>`; global concurrency cap via
    /// `Arc<Semaphore>`. Watchdog wraps `handle_request`; the entry
    /// is `XACK`ed on success, on terminal handler error (logged as a
    /// `verifier_verdict_error` Misc), and on malformed payloads. Only
    /// crashes between read + ack leave the entry in the PEL — another
    /// consumer (or this same worker after restart) picks it up;
    /// `handle_request` is idempotent on `triggered_at_event_id`.
    pub async fn run(
        &self,
        verifier_enabled: bool,
        redis: Arc<RedisPool>,
        shutdown: CancellationToken,
    ) -> Result<(), WorkerError> {
        if !verifier_enabled {
            tracing::debug!("verifier disabled; worker not spawned");
            return Ok(());
        }

        let cfg = self.runtime_config.clone();
        let consumer_id = make_consumer_id(&cfg.consumer_prefix);
        tracing::info!(
            stream = %cfg.stream,
            group = %cfg.group,
            consumer = %consumer_id,
            max_concurrency = cfg.max_concurrency,
            block_ms = cfg.consumer_block_ms,
            count = cfg.read_count,
            "verifier worker booting",
        );

        let sem = Arc::new(Semaphore::new(cfg.max_concurrency.max(1)));
        let session_locks: Arc<DashMap<String, Arc<Mutex<()>>>> = Arc::new(DashMap::new());
        let mut group_ready = false;
        let mut in_flight = Vec::<tokio::task::JoinHandle<()>>::new();

        while !shutdown.is_cancelled() {
            if !group_ready {
                let res = tokio::select! {
                    _ = shutdown.cancelled() => break,
                    out = redis.xgroup_create_mkstream(&cfg.stream, &cfg.group) => out,
                };
                match res {
                    Ok(()) => {
                        group_ready = true;
                    }
                    Err(error) => {
                        tracing::warn!(
                            %error,
                            stream = %cfg.stream,
                            group = %cfg.group,
                            "verifier worker: xgroup create failed; retrying after backoff",
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
                    cfg.read_count,
                    cfg.consumer_block_ms,
                ) => out,
            };
            let entries = match read {
                Ok(v) => v,
                Err(error) => {
                    tracing::warn!(
                        %error,
                        stream = %cfg.stream,
                        group = %cfg.group,
                        "verifier worker: xreadgroup failed; backing off",
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
                let sem_c = sem.clone();
                let locks_c = session_locks.clone();
                let cfg_c = cfg.clone();
                in_flight.push(tokio::spawn(async move {
                    process_entry(worker, redis_c, sem_c, locks_c, cfg_c, msg_id, payload).await;
                }));
            }
        }

        for h in in_flight {
            if let Err(error) = h.await {
                tracing::warn!(%error, "verifier worker: entry task join failed");
            }
        }
        Ok(())
    }

    async fn call_with_retry(
        &self,
        req: &VerifyRequest,
        messages: Vec<Message>,
    ) -> Option<Verdict> {
        let cancel = self
            .deps
            .cancel_tokens
            .get(&req.session_id)
            .map(|t| t.clone());
        let first = match cancel.clone() {
            Some(token) => {
                tokio::select! {
                    _ = token.cancelled() => return None,
                    out = self.call_once(messages.clone()) => out,
                }
            }
            None => self.call_once(messages.clone()).await,
        };
        match first {
            Ok(Some(v)) => return Some(v),
            Ok(None) => {} // unparseable, fall through to retry
            Err(e) => {
                tracing::warn!(error = %e, "verifier LLM call failed on first attempt");
            }
        }
        let retry_messages = append_strict_suffix(messages);
        let second = match cancel {
            Some(token) => {
                tokio::select! {
                    _ = token.cancelled() => return None,
                    out = self.call_once(retry_messages) => out,
                }
            }
            None => self.call_once(retry_messages).await,
        };
        match second {
            Ok(Some(v)) => Some(v),
            Ok(None) | Err(_) => Some(Verdict::unparseable()),
        }
    }

    async fn call_once(&self, messages: Vec<Message>) -> Result<Option<Verdict>, LlmError> {
        let req = ChatCompletionRequest {
            model: self.deps.verifier_slot_model.clone(),
            messages,
            tools: None,
            tool_choice: Some(ToolChoice::String(ToolChoiceMode::None)),
            temperature: Some(0.0),
            max_tokens: Some(VERIFIER_MAX_TOKENS),
            top_p: None,
        };
        let resp = self.deps.verifier_llm.chat_completion(req).await?;
        let content = resp
            .choices
            .first()
            .and_then(|c| c.message.content.as_deref())
            .unwrap_or_default();
        Ok(parse_verdict(content))
    }

    async fn cost_snapshot_cents(&self) -> i64 {
        match self.deps.cost.snapshot().await {
            Ok(s) => s.total_cents,
            Err(_) => 0,
        }
    }
}

fn append_strict_suffix(mut messages: Vec<Message>) -> Vec<Message> {
    if let Some(last) = messages.last_mut()
        && last.role == Role::User
    {
        let mut content = last.content.clone().unwrap_or_default();
        content.push_str(STRICT_RETRY_SUFFIX);
        last.content = Some(content);
        return messages;
    }
    messages.push(Message {
        role: Role::User,
        content: Some(STRICT_RETRY_SUFFIX.trim_start_matches('\n').to_string()),
        name: None,
        tool_calls: None,
        tool_call_id: None,
    });
    messages
}

fn parse_suggested_phases(value: &Value) -> Option<Vec<Phase>> {
    let phases_v = value.get("phases")?;
    serde_json::from_value::<Vec<Phase>>(phases_v.clone()).ok()
}

async fn emit_verifier_verdict_event(
    events: &SqliteEventStore,
    session_id: &str,
    trigger_kind: &str,
    verification_id: &str,
    verdict: &Verdict,
) -> Result<(), EventError> {
    let data = json!({
        "kind": "verifier_verdict",
        "verdict": match verdict.verdict {
            super::VerdictKind::Pass => "pass",
            super::VerdictKind::Fail => "fail",
        },
        "reason": verdict.reason,
        "evidence_event_ids": verdict.evidence_event_ids,
        "suggested_plan_update": verdict.suggested_plan_update,
        "verification_id": verification_id,
        "trigger_kind": trigger_kind,
    });
    events
        .append(NewEvent {
            session_id: session_id.to_string(),
            event_type: EventType::Misc,
            source: "verifier".to_string(),
            data,
        })
        .await?;
    Ok(())
}

/// Emit the `verifier_watchdog` Misc event after a `handle_request`
/// times out under the watchdog. Pub-crate so [`run`] and a future
/// reusable timer (story 1.17 / Gate) can both reach it.
pub(crate) async fn emit_verifier_watchdog_event(
    events: &SqliteEventStore,
    session_id: &str,
    triggered_at_event_id: u64,
) -> Result<(), EventError> {
    events
        .append(NewEvent {
            session_id: session_id.to_string(),
            event_type: EventType::Misc,
            source: "verifier".to_string(),
            data: json!({
                "kind": "verifier_watchdog",
                "triggered_at_event_id": triggered_at_event_id,
            }),
        })
        .await?;
    Ok(())
}

/// Process exactly one `XREADGROUP` entry: parse → per-session FIFO
/// lock → global semaphore permit → `handle_request_with_watchdog` →
/// always `XACK`. Malformed payloads are XACKed (PEL retention would
/// just block the queue on garbage); terminal handler errors emit a
/// `verifier_verdict_error` Misc + XACK.
async fn process_entry(
    worker: Worker,
    redis: Arc<RedisPool>,
    sem: Arc<Semaphore>,
    session_locks: Arc<DashMap<String, Arc<Mutex<()>>>>,
    cfg: VerifierRuntimeConfig,
    msg_id: String,
    payload: Vec<u8>,
) {
    let req: VerifyRequest = match serde_json::from_slice(&payload) {
        Ok(req) => req,
        Err(error) => {
            tracing::warn!(
                %msg_id,
                %error,
                "verifier worker: dropping malformed verify_request",
            );
            if let Err(xack_error) = redis.xack(&cfg.stream, &cfg.group, &msg_id).await {
                tracing::warn!(
                    %msg_id,
                    %xack_error,
                    "verifier worker: xack failed after malformed verify_request",
                );
            }
            return;
        }
    };

    let lock = session_locks
        .entry(req.session_id.clone())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone();
    let _guard = lock.lock().await;
    let Ok(_permit) = sem.acquire().await else {
        // Semaphore can only fail if closed — we never close it.
        return;
    };

    match handle_request_with_watchdog(&worker, &req).await {
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(
                %msg_id,
                session_id = %req.session_id,
                %error,
                "verifier worker: handle_request failed; recording verifier_verdict_error Misc + XACK",
            );
            let _ = worker
                .deps
                .events
                .append(NewEvent {
                    session_id: req.session_id.clone(),
                    event_type: EventType::Misc,
                    source: "verifier".to_string(),
                    data: json!({
                        "kind": "verifier_verdict_error",
                        "trigger_kind": req.trigger.kind_str(),
                        "triggered_at_event_id": req.triggered_at_event_id,
                        "error": error.to_string(),
                    }),
                })
                .await;
        }
    }

    if let Err(error) = redis.xack(&cfg.stream, &cfg.group, &msg_id).await {
        tracing::warn!(
            %msg_id,
            %error,
            "verifier worker: XACK failed; message will stay in PEL until next consumer",
        );
    }
}

fn make_consumer_id(prefix: &str) -> String {
    format!("{prefix}-{}-{}", read_hostname(), std::process::id())
}

/// Best-effort hostname read. Linux-only deployment (per CLAUDE.md);
/// `/proc/sys/kernel/hostname` is the authoritative source. Falls back
/// to the `HOSTNAME` shell var, then a fixed sentinel.
fn read_hostname() -> String {
    if let Ok(s) = std::fs::read_to_string("/proc/sys/kernel/hostname") {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    std::env::var("HOSTNAME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Sleep `dur` unless `shutdown` fires first. Returns `true` if
/// cancellation fired (caller should break the loop), `false` if the
/// sleep elapsed normally.
async fn backoff_or_cancel(shutdown: &CancellationToken, dur: Duration) -> bool {
    tokio::select! {
        _ = shutdown.cancelled() => true,
        _ = tokio::time::sleep(dur) => false,
    }
}

/// Hard wall-clock cap wrapper around `handle_request`. On timeout,
/// emits the watchdog event and returns Ok(None) (caller should XACK
/// the message — Gate decides session state, story 1.10).
pub(crate) async fn handle_request_with_watchdog(
    worker: &Worker,
    req: &VerifyRequest,
) -> Result<Option<String>, WorkerError> {
    match tokio::time::timeout(worker.watchdog, worker.handle_request(req)).await {
        Ok(Ok(id)) => Ok(Some(id)),
        Ok(Err(e)) => Err(e),
        Err(_) => {
            emit_verifier_watchdog_event(
                &worker.deps.events,
                &req.session_id,
                req.triggered_at_event_id,
            )
            .await?;
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests;
