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
use crate::dispatch::mask::{AgentMode, MaskContext, ToolMaskPolicy, apply_mask};
use crate::events::{EventError, EventStore, EventType, NewEvent, sqlite::SqliteEventStore};
use crate::llm::{ChatCompletionRequest, LlmClient, LlmError, ToolChoice, ToolSpec};
use crate::plan::PlanManager;
use crate::pubsub::RedisPool;
use crate::router::{SlotName, SlotRouter};
use crate::sandbox::SandboxClient;
use crate::search::SearchClient;
use crate::tools::ToolContext;

pub mod init;
mod prompt;
pub mod recite;
pub mod stuck;

pub use prompt::build_messages;
use recite::{ReciteScheduler, recite_tick};
use stuck::{StuckAction, StuckTracker};

#[derive(Debug, Clone)]
pub struct RunRequest {
    pub session_id: String,
    pub input: String,
    pub max_steps: u32,
    pub cost_cap_cents: Option<u32>,
}

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
    run_config: Arc<tokio::sync::Mutex<HashMap<String, RunRequest>>>,
    cancel_tokens: Arc<DashMap<String, CancellationToken>>,
}

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
            let mut tools = self.tool_specs_from_registry();
            let mask_ctx = MaskContext {
                session_id: req.session_id.clone(),
                iteration: step,
                mode: AgentMode::Worker,
            };
            apply_mask(&mut tools, &*self.mask_policy, &mask_ctx);
            let main_slot = self.router.resolve(SlotName::Main);
            let response = match self
                .llm
                .chat_completion(ChatCompletionRequest {
                    model: main_slot.model.clone(),
                    messages,
                    tools: Some(tools),
                    tool_choice: Some(ToolChoice::required()),
                    temperature: None,
                    max_tokens: None,
                    top_p: None,
                })
                .await
            {
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
                        return Err(AgentError::Llm(error));
                    }
                    continue;
                }
                Err(error) => return Err(AgentError::Llm(error)),
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
                    strategy_prompt = Some(strategy_change_prompt(count));
                }
                StuckAction::Terminate { count } => {
                    self.emit_misc(
                        &req.session_id,
                        "stuck_terminate",
                        json!({"step": step + 1, "duplicate_count": count}),
                    )
                    .await?;
                    self.set_session_state(&req.session_id, "ERROR").await?;
                    return Err(AgentError::StuckTerminated { count });
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
                mask_ctx,
                events: self.events.clone(),
                sandbox: self.sandbox.clone(),
                search: self.search.clone(),
                plan_manager: self.plan_manager.clone(),
                checkpoint_labels: self.checkpoint_labels.clone(),
                checkpoints: self.checkpoints.clone(),
            };
            let final_notify = call.function.name == "message_notify_user"
                && args.get("final").and_then(Value::as_bool).unwrap_or(false);
            // Story 1.10: only enter VERIFYING when a verifier slot is
            // configured. Sessions running without a verifier (Phase 0
            // single-slot configs, test harnesses) complete directly on
            // idle / final-message without going through the gate.
            let verifier_active = self.router.verifier_enabled();
            let output = self
                .dispatcher
                .dispatch(&ctx, &call.function.name, args)
                .await;
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
                self.set_session_state(&req.session_id, "SUSPENDED").await?;
                return Ok(RunResult {
                    session_id: req.session_id,
                    completed: false,
                    last_message,
                    steps: step + 1,
                });
            }

            if call.function.name == "idle" && output.ok {
                self.set_session_state(&req.session_id, "FINISHED").await?;
                self.cancel_tokens.remove(&req.session_id);
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
        }
        self.set_session_state(&req.session_id, "FINISHED").await?;
        self.cancel_tokens.remove(&req.session_id);
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
            context_hint: Default::default(),
        };
        if let Err(error) = self.redis.xadd_json("verify_request", &req).await {
            tracing::warn!(%error, "failed to enqueue verify_request");
        }
        Ok(())
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

fn strategy_change_prompt(count: u32) -> String {
    format!(
        "You have repeated the same response {count} times. Try a different strategy: consider \
         a different tool, re-read recent observations, or call message_ask_user to clarify."
    )
}

#[cfg(test)]
mod tests;
