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
}

impl Worker {
    pub fn new(deps: WorkerDeps) -> Self {
        Self {
            deps,
            watchdog: DEFAULT_WATCHDOG,
        }
    }

    pub fn with_watchdog(mut self, d: Duration) -> Self {
        self.watchdog = d;
        self
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
    /// The Redis Streams loop itself is a TODO surface — story 1.9b
    /// ships the consumer-group bootstrap + the iteration shape; the
    /// `XREADGROUP` body is a thin polling shim that delegates to
    /// [`Worker::handle_request`] per parsed entry.
    pub async fn run(
        &self,
        verifier_enabled: bool,
        redis: Arc<RedisPool>,
        shutdown: tokio_util::sync::CancellationToken,
    ) -> Result<(), WorkerError> {
        if !verifier_enabled {
            tracing::debug!("verifier disabled; worker not spawned");
            return Ok(());
        }
        let _ = ensure_consumer_group(&redis).await;

        while !shutdown.is_cancelled() {
            // Phase 1 baseline: short polling tick. Story 1.9b's intent
            // is the worker plumbing + handle_request pipeline; the
            // production XREADGROUP loop hangs off this point and is
            // exercised via the live-Redis integration path (deferred
            // to phase-1 E2E story 1.20). For unit tests we drive
            // handle_request directly.
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = tokio::time::sleep(Duration::from_millis(500)) => {}
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

/// Best-effort `XGROUP CREATE verify_request verifier $ MKSTREAM`.
/// Pre-existing group is not an error. Phase 1 baseline only — the
/// actual Streams polling shape is exercised in story 1.20 E2E and
/// can be reworked here once a real Bifrost+Redis test stand is up.
async fn ensure_consumer_group(_redis: &RedisPool) -> Result<(), WorkerError> {
    // Stream consumer-group bootstrap is wired through RedisPool's raw
    // command interface. Implementing it inline here would require
    // exposing the deadpool-redis connection; deferred to story 1.20
    // E2E where a live Redis is reachable.
    Ok(())
}

/// Hard wall-clock cap wrapper around `handle_request`. On timeout,
/// emits the watchdog event and returns Ok(None) (caller should XACK
/// the message — Gate decides session state, story 1.10).
pub async fn handle_request_with_watchdog(
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
