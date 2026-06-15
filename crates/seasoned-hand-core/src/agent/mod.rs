//! Agent ReAct runner.
//! refs: /specs/phase-0/stories/story-0.14.md
//! refs: /specs/phase-0/stories/story-0.15.md
//! refs: /specs/phase-0/stories/story-0.16.md
//! refs: /specs/phase-0/architecture.md §1, §4.3

use std::collections::HashMap;
use std::sync::Arc;

use dashmap::DashMap;
use serde_json::{Value, json};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::cost::{CostClient, CostSnapshot, delta_between};
use crate::db::{DbError, DbPool};
use crate::dispatch::ToolDispatcher;
use crate::dispatch::mask::{AgentMode, ToolMaskPolicy, apply_mask};
use crate::events::{EventError, EventStore, EventType, NewEvent, sqlite::SqliteEventStore};
use crate::llm::{ChatCompletionRequest, LlmClient, LlmError, ToolChoice, ToolSpec};
use crate::plan::PlanManager;
use crate::pubsub::RedisPool;
use crate::router::{SlotName, SlotRouter};
use crate::sandbox::SandboxClient;
use crate::search::SearchClient;
use crate::tools::ToolContext;

pub mod breaker;
pub mod diversity;
pub mod init;
pub mod narrate;
mod prompt;
pub mod recite;
pub mod stuck;

use breaker::{BreakerRegistry, CircuitBreaker};
use diversity::DiversityInjector;
pub(crate) use prompt::build_messages;
use recite::{ReciteScheduler, recite_tick};
use stuck::{StuckAction, StuckTracker};

/// One agent-loop invocation: which session, what the user asked, and
/// the per-task budgets. Built once by the runner's spawner; the loop
/// stores it in `run_config` so resume after pause can re-establish the
/// same shape.
#[derive(Debug, Clone)]
pub struct RunRequest {
    pub session_id: String,
    pub input: String,
    pub max_steps: u32,
    pub cost_cap_cents: Option<u32>,
}

/// Terminal outcome of one agent-loop run. `completed = true` means the
/// loop reached an `idle` tool call (clean finish); `false` means
/// step-cap / cost-cap / cancellation / stuck-termination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunResult {
    pub session_id: String,
    pub completed: bool,
    pub last_message: Option<String>,
    pub steps: u32,
}

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("llm error: {0}")]
    Llm(#[from] LlmError),
    #[error("db error: {0}")]
    Db(#[from] DbError),
    #[error("event error: {0}")]
    Event(#[from] EventError),
    #[error("initializer error: {0}")]
    Init(#[from] init::InitError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("cancelled")]
    Cancelled,
    #[error("stuck detection terminated after {count} repeated responses")]
    StuckTerminated { count: u32 },
    #[error("internal error: {0}")]
    Internal(String),
}

/// The ReAct agent loop runtime (Phase 0 story 0.14, widened across
/// Phases 1 and 2). One per server process; per-session state lives in
/// `run_config`, `cancel_tokens`, and the underlying DB. `run` is the
/// fresh-start entrypoint; `resume` re-enters after a pause-window or
/// briefing-confirm gate.
pub struct AgentRunner {
    llm: LlmClient,
    dispatcher: Arc<ToolDispatcher>,
    events: Arc<SqliteEventStore>,
    router: Arc<SlotRouter>,
    sandbox: Arc<SandboxClient>,
    search: Arc<SearchClient>,
    cost: Arc<CostClient>,
    sessions: DbPool,
    plan_manager: Arc<PlanManager>,
    mask_policy: Arc<dyn ToolMaskPolicy>,
    checkpoint_labels: Arc<crate::checkpoint::CheckpointLabelBuffer>,
    checkpoints: Arc<crate::checkpoint::CheckpointStore>,
    redis: Arc<RedisPool>,
    breakers: Arc<BreakerRegistry>,
    diversity: Arc<DiversityInjector>,
    run_config: Arc<tokio::sync::Mutex<HashMap<String, RunRequest>>>,
    cancel_tokens: Arc<DashMap<String, CancellationToken>>,
}

/// Builder bag for [`AgentRunner::new`]. Lets `AppState` (production)
/// and test harnesses (which inject mock sandbox / LLM / event stores)
/// assemble the runner with the same call shape.
pub struct AgentRunnerDeps {
    pub llm: LlmClient,
    pub dispatcher: Arc<ToolDispatcher>,
    pub events: Arc<SqliteEventStore>,
    pub router: Arc<SlotRouter>,
    pub sandbox: Arc<SandboxClient>,
    pub search: Arc<SearchClient>,
    pub cost: Arc<CostClient>,
    pub sessions: DbPool,
    pub plan_manager: Arc<PlanManager>,
    pub mask_policy: Arc<dyn ToolMaskPolicy>,
    pub checkpoint_labels: Arc<crate::checkpoint::CheckpointLabelBuffer>,
    pub checkpoints: Arc<crate::checkpoint::CheckpointStore>,
    pub redis: Arc<RedisPool>,
    pub breakers: Arc<BreakerRegistry>,
    pub cancel_tokens: Arc<DashMap<String, CancellationToken>>,
}

impl AgentRunner {
    pub fn new(deps: AgentRunnerDeps) -> Self {
        Self {
            llm: deps.llm,
            dispatcher: deps.dispatcher,
            events: deps.events,
            router: deps.router,
            sandbox: deps.sandbox,
            search: deps.search,
            cost: deps.cost,
            sessions: deps.sessions,
            plan_manager: deps.plan_manager,
            mask_policy: deps.mask_policy,
            checkpoint_labels: deps.checkpoint_labels,
            checkpoints: deps.checkpoints,
            redis: deps.redis,
            breakers: deps.breakers,
            diversity: Arc::new(DiversityInjector::new()),
            run_config: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            cancel_tokens: deps.cancel_tokens,
        }
    }

    pub async fn run(&self, req: RunRequest) -> Result<RunResult, AgentError> {
        self.cancel_tokens
            .insert(req.session_id.clone(), CancellationToken::new());
        self.run_config
            .lock()
            .await
            .insert(req.session_id.clone(), req.clone());
        self.run_loop(req, true, 0).await
    }

    pub async fn resume(&self, req: RunRequest) -> Result<RunResult, AgentError> {
        self.cancel_tokens
            .entry(req.session_id.clone())
            .or_default();
        self.run_config
            .lock()
            .await
            .entry(req.session_id.clone())
            .or_insert_with(|| req.clone());
        let start_step = self.action_count(&req.session_id).await?;
        self.run_loop(req, false, start_step).await
    }

    pub async fn resume_session(&self, session_id: &str) -> Result<RunResult, AgentError> {
        let req = self
            .run_config
            .lock()
            .await
            .get(session_id)
            .cloned()
            .unwrap_or(RunRequest {
                session_id: session_id.to_string(),
                input: String::new(),
                max_steps: 24,
                cost_cap_cents: None,
            });
        self.resume(req).await
    }

    async fn run_loop(
        &self,
        req: RunRequest,
        seed_task: bool,
        start_step: u32,
    ) -> Result<RunResult, AgentError> {
        self.set_session_state(&req.session_id, "RUNNING").await?;
        if seed_task {
            init::Initializer::new(
                self.router.clone(),
                self.plan_manager.clone(),
                self.sandbox.clone(),
                self.events.clone(),
            )
            .run(&req.session_id, &req.input)
            .await?;
            self.append_user_message(&req.session_id, &req.input)
                .await?;
        }

        let mut stuck = StuckTracker::default();
        let mut strategy_prompt = None;
        let mut last_message = None;
        let mut status_errors = 0u32;
        let mut steps_run = 0u32;
        let mut stopped_early = false;
        let mut cost_baseline = match self.cost.snapshot().await {
            Ok(snapshot) => Some(snapshot),
            Err(error) => {
                tracing::warn!(%error, "cost baseline poll failed; cost accounting starts at 0");
                None
            }
        };
        let breaker = self.breakers.for_session(&req.session_id).await;
        let cancel_token = self.cancel_tokens.get(&req.session_id).map(|t| t.clone());

        // Hot-loop invariant: the tool catalogue and mask policy are fixed for
        // the whole run, so build the masked ToolSpec list once instead of
        // rebuilding all ~38 tool schemas (a fresh `json!` per tool) every
        // iteration.
        let masked_tools = {
            let mut tools = self.tool_specs_from_registry();
            apply_mask(&mut tools, &*self.mask_policy, AgentMode::Worker);
            tools
        };

        for step in start_step..req.max_steps {
            steps_run = step + 1;
            if self
                .cancel_tokens
                .get(&req.session_id)
                .is_some_and(|t| t.is_cancelled())
            {
                self.emit_misc(&req.session_id, "task_cancelled", json!({"reason":"user"}))
                    .await?;
                self.set_session_state(&req.session_id, "SUSPENDED").await?;
                return Ok(RunResult {
                    session_id: req.session_id,
                    completed: false,
                    last_message,
                    steps: step + 1,
                });
            }
            if ReciteScheduler::should_fire(step) {
                recite_tick(&self.sandbox, &self.events, &req.session_id).await;
            }
            let mut messages =
                build_messages(&self.events, &self.plan_manager, &req.session_id).await?;
            if let Some(prompt) = strategy_prompt.take() {
                messages.insert(
                    0,
                    crate::llm::Message {
                        role: crate::llm::Role::System,
                        content: Some(prompt),
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
                    },
                );
            }
            let main_slot = self.router.resolve(SlotName::Main);
            let llm_call = self.llm.chat_completion(ChatCompletionRequest {
                model: main_slot.model.clone(),
                messages,
                tools: Some(masked_tools.clone()),
                tool_choice: Some(ToolChoice::required()),
                temperature: None,
                max_tokens: None,
                top_p: None,
            });
            let llm_result = match cancel_token.clone() {
                Some(token) => {
                    tokio::select! {
                        _ = token.cancelled() => {
                            self.emit_misc(&req.session_id, "task_cancelled", json!({"reason":"user"})).await?;
                            self.set_session_state(&req.session_id, "SUSPENDED").await?;
                            return Ok(RunResult {
                                session_id: req.session_id,
                                completed: false,
                                last_message,
                                steps: step + 1,
                            });
                        }
                        out = llm_call => out
                    }
                }
                None => llm_call.await,
            };
            let response = match llm_result {
                Ok(response) => {
                    status_errors = 0;
                    response
                }
                Err(error @ LlmError::Status { .. }) => {
                    status_errors += 1;
                    self.emit_misc(
                        &req.session_id,
                        "llm_status_error",
                        json!({"step": step + 1, "consecutive": status_errors}),
                    )
                    .await?;
                    if status_errors >= 4 {
                        self.set_session_state(&req.session_id, "ERROR").await?;
                        self.finalize(&req.session_id).await;
                        return Err(AgentError::Llm(error));
                    }
                    continue;
                }
                Err(error) => {
                    // A single non-status LLM failure (Http / JsonParse /
                    // MissingChoice) is a RESUMABLE infra error, not terminal —
                    // unlike the 4x-consecutive-status path above which sets ERROR.
                    // Leave the session state and its run-config / cancel-token
                    // intact so it can be retried, and do NOT finalize (issue #12:
                    // finalize is for terminal exits only; this isn't one).
                    return Err(AgentError::Llm(error));
                }
            };

            let assistant = response
                .choices
                .first()
                .map(|choice| &choice.message)
                .ok_or(LlmError::MissingChoice)?;
            last_message = assistant.content.clone();
            match stuck.observe(assistant) {
                StuckAction::Continue => {}
                StuckAction::InjectStrategyPrompt { count } => {
                    self.emit_misc(
                        &req.session_id,
                        "stuck_inject",
                        json!({"step": step + 1, "duplicate_count": count}),
                    )
                    .await?;
                    let (event_id, summary) = self
                        .latest_observation_summary(&req.session_id)
                        .await
                        .unwrap_or((0, "none".to_string()));
                    strategy_prompt = Some(self.diversity.next_prompt(
                        &req.session_id,
                        count,
                        event_id,
                        &summary,
                    ));
                }
                StuckAction::Terminate { count } => {
                    self.emit_misc(
                        &req.session_id,
                        "stuck_terminate",
                        json!({"step": step + 1, "duplicate_count": count}),
                    )
                    .await?;
                    if let Some(kind) = breaker.note_stuck_and_check(count).await {
                        self.emit_breaker_trigger(&req.session_id, kind).await?;
                    }
                    self.finalize(&req.session_id).await;
                    return Ok(RunResult {
                        session_id: req.session_id,
                        completed: false,
                        last_message,
                        steps: step + 1,
                    });
                }
            }

            let Some(calls) = assistant.tool_calls.as_ref() else {
                self.emit_misc(&req.session_id, "no_tool_call", json!({"step": step + 1}))
                    .await?;
                stopped_early = true;
                break;
            };
            let Some(call) = calls.first() else {
                self.emit_misc(&req.session_id, "no_tool_call", json!({"step": step + 1}))
                    .await?;
                stopped_early = true;
                break;
            };
            if calls.len() > 1 {
                self.emit_misc(
                    &req.session_id,
                    "multi_tool_warning",
                    json!({
                        "step": step + 1,
                        "kept": call.function.name,
                        "dropped_count": calls.len() - 1,
                    }),
                )
                .await?;
            }

            let args = parse_args(&call.function.arguments);
            let ctx = ToolContext {
                session_id: req.session_id.clone(),
                mask_mode: AgentMode::Worker,
                events: self.events.clone(),
                sandbox: self.sandbox.clone(),
                search: self.search.clone(),
                plan_manager: self.plan_manager.clone(),
                checkpoint_labels: self.checkpoint_labels.clone(),
                checkpoints: self.checkpoints.clone(),
                matcher_mode: crate::matcher::MatcherMode::Production,
            };
            let final_notify = call.function.name == "message_notify_user"
                && args.get("final").and_then(Value::as_bool).unwrap_or(false);
            // Story 1.10: only enter VERIFYING when a verifier slot is
            // configured. Sessions running without a verifier (Phase 0
            // single-slot configs, test harnesses) complete directly on
            // idle / final-message without going through the gate.
            let verifier_active = self.router.verifier_enabled();
            let dispatch_call = self.dispatcher.dispatch(&ctx, &call.function.name, args);
            let output = match cancel_token.clone() {
                Some(token) => {
                    tokio::select! {
                        _ = token.cancelled() => {
                            self.emit_misc(&req.session_id, "task_cancelled", json!({"reason":"user"})).await?;
                            self.set_session_state(&req.session_id, "SUSPENDED").await?;
                            return Ok(RunResult {
                                session_id: req.session_id,
                                completed: false,
                                last_message,
                                steps: step + 1,
                            });
                        }
                        out = dispatch_call => out
                    }
                }
                None => dispatch_call.await,
            };
            if let Some(kind) = breaker.note_observation_and_check(output.ok).await {
                self.emit_breaker_trigger(&req.session_id, kind).await?;
                self.finalize(&req.session_id).await;
                return Ok(RunResult {
                    session_id: req.session_id,
                    completed: false,
                    last_message,
                    steps: step + 1,
                });
            }
            let final_idle = call.function.name == "idle";
            if output.ok && (final_idle || final_notify) && verifier_active {
                self.mark_task_complete(&req.session_id, &call.id).await?;
                return Ok(RunResult {
                    session_id: req.session_id,
                    completed: false,
                    last_message,
                    steps: step + 1,
                });
            }

            let current_cost = self
                .record_step_cost(&req.session_id, &mut cost_baseline)
                .await;
            if let Some(cap) = req.cost_cap_cents
                && current_cost >= i64::from(cap)
            {
                self.emit_misc(
                    &req.session_id,
                    "cost_cap",
                    json!({"current_cents": current_cost, "cap_cents": cap}),
                )
                .await?;
                if let Some(kind) = breaker.note_cost_and_check(current_cost as u32, cap).await {
                    self.emit_breaker_trigger(&req.session_id, kind).await?;
                }
                self.finalize(&req.session_id).await;
                return Ok(RunResult {
                    session_id: req.session_id,
                    completed: false,
                    last_message,
                    steps: step + 1,
                });
            }

            if call.function.name == "idle" && output.ok {
                self.set_session_state(&req.session_id, "FINISHED").await?;
                self.finalize(&req.session_id).await;
                return Ok(RunResult {
                    session_id: req.session_id,
                    completed: true,
                    last_message,
                    steps: step + 1,
                });
            }

            if call.function.name == "message_ask_user" && output.ok {
                self.set_session_state(&req.session_id, "SUSPENDED").await?;
                return Ok(RunResult {
                    session_id: req.session_id,
                    completed: false,
                    last_message,
                    steps: step + 1,
                });
            }
        }

        if !stopped_early {
            self.emit_misc(
                &req.session_id,
                "max_steps_reached",
                json!({"max_steps": req.max_steps}),
            )
            .await?;
            if let Some(kind) = breaker
                .note_iteration_and_check(req.max_steps, req.max_steps)
                .await
            {
                self.emit_breaker_trigger(&req.session_id, kind).await?;
            }
        }
        // Terminal exit (max steps reached, or stopped early on no-tool-call).
        self.finalize(&req.session_id).await;
        Ok(RunResult {
            session_id: req.session_id,
            completed: false,
            last_message,
            steps: steps_run,
        })
    }

    fn tool_specs_from_registry(&self) -> Vec<ToolSpec> {
        let mut tools = self.dispatcher.registry().values().collect::<Vec<_>>();
        tools.sort_by_key(|tool| tool.name());
        tools
            .into_iter()
            .map(|tool| ToolSpec::function(tool.name(), tool.description(), tool.schema()))
            .collect()
    }

    async fn append_user_message(&self, session_id: &str, input: &str) -> Result<(), AgentError> {
        self.events
            .append(NewEvent {
                session_id: session_id.to_string(),
                event_type: EventType::Message,
                source: "user".into(),
                data: json!({"role": "user", "content": input, "ui": null}),
            })
            .await?;
        Ok(())
    }

    async fn emit_misc(
        &self,
        session_id: &str,
        kind: &str,
        mut data: Value,
    ) -> Result<(), AgentError> {
        if let Value::Object(obj) = &mut data {
            obj.insert("kind".into(), Value::String(kind.into()));
        } else {
            data = json!({"kind": kind, "data": data});
        }
        self.events
            .append(NewEvent {
                session_id: session_id.to_string(),
                event_type: EventType::Misc,
                source: "agent".into(),
                data,
            })
            .await?;
        Ok(())
    }

    async fn record_step_cost(
        &self,
        session_id: &str,
        cost_baseline: &mut Option<CostSnapshot>,
    ) -> i64 {
        match self.cost.snapshot().await {
            Ok(current) => {
                let delta = cost_baseline
                    .as_ref()
                    .map(|baseline| delta_between(baseline, &current))
                    .unwrap_or(0);
                *cost_baseline = Some(current);
                if delta > 0
                    && let Err(error) = self.bump_session_cost(session_id, delta).await
                {
                    tracing::warn!(%error, %session_id, "session cost update failed");
                }
            }
            Err(error) => {
                tracing::warn!(%error, %session_id, "cost delta poll failed; skipping accounting");
            }
        }

        match self.session_cost(session_id).await {
            Ok(cost) => cost,
            Err(error) => {
                tracing::warn!(%error, %session_id, "session cost read failed");
                0
            }
        }
    }

    async fn bump_session_cost(&self, session_id: &str, delta: i64) -> Result<(), AgentError> {
        let session_id = session_id.to_string();
        self.sessions
            .with_conn(move |conn| {
                conn.execute(
                    "UPDATE sessions SET cost_cents = cost_cents + ?, \
                     updated_at = unixepoch('subsec') * 1000000 WHERE id = ?",
                    rusqlite::params![delta, session_id],
                )
            })
            .await?;
        Ok(())
    }

    async fn session_cost(&self, session_id: &str) -> Result<i64, AgentError> {
        let session_id = session_id.to_string();
        let cost = self
            .sessions
            .with_conn(move |conn| {
                conn.query_row(
                    "SELECT cost_cents FROM sessions WHERE id = ?",
                    rusqlite::params![session_id],
                    |row| row.get::<_, i64>(0),
                )
            })
            .await?;
        Ok(cost)
    }

    async fn action_count(&self, session_id: &str) -> Result<u32, AgentError> {
        let session_id = session_id.to_string();
        let count = self
            .sessions
            .with_conn(move |conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM events WHERE session_id = ? AND type = 'Action'",
                    rusqlite::params![session_id],
                    |row| row.get::<_, i64>(0),
                )
            })
            .await?;
        Ok(u32::try_from(count).unwrap_or(0))
    }

    /// Issue #12: drop per-session bookkeeping (cancel token + cached run config)
    /// on TERMINAL exits so `cancel_tokens` / `run_config` don't grow without bound
    /// over the process lifetime. Deliberately NOT called on resumable exits
    /// (SUSPENDED / VERIFYING): those must keep their entries so `resume_session`
    /// can re-establish the run.
    async fn finalize(&self, session_id: &str) {
        self.cancel_tokens.remove(session_id);
        self.run_config.lock().await.remove(session_id);
    }

    async fn mark_task_complete(&self, session_id: &str, call_id: &str) -> Result<(), AgentError> {
        self.set_session_state(session_id, "VERIFYING").await?;
        let event = self
            .events
            .append(NewEvent {
                session_id: session_id.to_string(),
                event_type: EventType::Misc,
                source: "agent".into(),
                data: json!({
                    "kind":"verifier_request",
                    "trigger":"TaskComplete",
                    "final_message_call_id": call_id,
                }),
            })
            .await?;
        let req = crate::verifier::VerifyRequest {
            session_id: session_id.to_string(),
            trigger: crate::verifier::VerifyTrigger::TaskComplete {
                final_message_call_id: call_id.to_string(),
            },
            triggered_at_event_id: event.id as u64,
        };
        if let Err(error) = self.redis.xadd_json("verify_request", &req).await {
            // Issue #13: without the enqueued verdict the session would sit in
            // VERIFYING forever (nothing will ever advance it). Fail it to a
            // terminal ERROR and clean up rather than warn and strand it.
            tracing::error!(%error, %session_id, "verify_request enqueue failed; failing session to ERROR");
            self.set_session_state(session_id, "ERROR").await?;
            self.finalize(session_id).await;
            return Err(AgentError::Internal(format!(
                "failed to enqueue verify_request: {error}"
            )));
        }
        Ok(())
    }

    async fn emit_breaker_trigger(
        &self,
        session_id: &str,
        kind: crate::verifier::BreakerKind,
    ) -> Result<(), AgentError> {
        let event = self
            .events
            .append(NewEvent {
                session_id: session_id.to_string(),
                event_type: EventType::Misc,
                source: "agent".into(),
                data: json!({
                    "kind":"verifier_request",
                    "trigger":"CircuitBreaker",
                    "breaker_kind": kind,
                }),
            })
            .await?;
        let req = crate::verifier::VerifyRequest {
            session_id: session_id.to_string(),
            trigger: crate::verifier::VerifyTrigger::CircuitBreaker { kind },
            triggered_at_event_id: event.id as u64,
        };
        if let Err(error) = self.redis.xadd_json("verify_request", &req).await {
            tracing::warn!(%error, "failed to enqueue breaker verify_request");
        }
        Ok(())
    }

    async fn latest_observation_summary(
        &self,
        session_id: &str,
    ) -> Result<(u64, String), AgentError> {
        let rows = self
            .events
            .query(session_id, crate::events::EventQuery::default())
            .await?;
        if let Some(ev) = rows
            .iter()
            .rev()
            .find(|e| e.event_type == EventType::Observation)
        {
            let summary = ev
                .data
                .get("body")
                .map(|v| v.to_string())
                .unwrap_or_else(|| "observation".to_string());
            return Ok((ev.id as u64, summary));
        }
        Ok((0, "none".to_string()))
    }

    pub async fn breaker_for_session(&self, session_id: &str) -> CircuitBreaker {
        self.breakers.for_session(session_id).await
    }

    async fn set_session_state(&self, session_id: &str, state: &str) -> Result<(), AgentError> {
        let session_id = session_id.to_string();
        let state = state.to_string();
        let changed = self
            .sessions
            .with_conn(move |conn| {
                conn.execute(
                    "UPDATE sessions SET state = ?, updated_at = unixepoch('subsec') * 1000000 \
                     WHERE id = ?",
                    rusqlite::params![state, session_id],
                )
            })
            .await?;
        if changed == 0 {
            return Err(AgentError::Internal("session not found".into()));
        }
        Ok(())
    }
}

fn parse_args(raw: &str) -> Value {
    match serde_json::from_str(raw) {
        Ok(value) => value,
        Err(_) => Value::Null,
    }
}

#[cfg(test)]
mod tests;
