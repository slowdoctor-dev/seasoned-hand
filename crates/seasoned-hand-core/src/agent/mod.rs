//! Agent ReAct runner.
//! refs: /specs/phase-0/stories/story-0.14.md
//! refs: /specs/phase-0/stories/story-0.15.md
//! refs: /specs/phase-0/architecture.md §1, §4.3

use std::sync::Arc;

use serde_json::{Value, json};
use thiserror::Error;

use crate::db::{DbError, DbPool};
use crate::dispatch::ToolDispatcher;
use crate::events::{EventError, EventStore, EventType, NewEvent, sqlite::SqliteEventStore};
use crate::llm::{ChatCompletionRequest, LlmClient, LlmError, ToolChoice, ToolSpec};
use crate::router::{SlotName, SlotRouter};
use crate::sandbox::SandboxClient;
use crate::search::SearchClient;
use crate::tools::ToolContext;

mod prompt;
pub mod stuck;

pub use prompt::build_messages;
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
    sessions: DbPool,
}

impl AgentRunner {
    pub fn new(
        llm: LlmClient,
        dispatcher: Arc<ToolDispatcher>,
        events: Arc<SqliteEventStore>,
        router: Arc<SlotRouter>,
        sandbox: Arc<SandboxClient>,
        search: Arc<SearchClient>,
        sessions: DbPool,
    ) -> Self {
        Self {
            llm,
            dispatcher,
            events,
            router,
            sandbox,
            search,
            sessions,
        }
    }

    pub async fn run(&self, req: RunRequest) -> Result<RunResult, AgentError> {
        self.set_session_state(&req.session_id, "RUNNING").await?;
        self.create_baseline_plan(&req.session_id, &req.input)
            .await?;
        self.append_user_message(&req.session_id, &req.input)
            .await?;

        let mut stuck = StuckTracker::default();
        let mut strategy_prompt = None;
        let mut last_message = None;
        let mut status_errors = 0u32;
        let mut steps_run = 0u32;
        let mut stopped_early = false;
        let _cost_cap_cents = req.cost_cap_cents;

        for step in 0..req.max_steps {
            steps_run = step + 1;
            let mut messages = build_messages(&self.events, &req.session_id).await?;
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
            let tools = self.tool_specs_from_registry();
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
                events: self.events.clone(),
                sandbox: self.sandbox.clone(),
                search: self.search.clone(),
            };
            let output = self
                .dispatcher
                .dispatch(&ctx, &call.function.name, args)
                .await;

            if call.function.name == "idle" && output.ok {
                self.set_session_state(&req.session_id, "FINISHED").await?;
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
        Ok(RunResult {
            session_id: req.session_id,
            completed: false,
            last_message,
            steps: steps_run,
        })
    }

    pub async fn resume(&self, req: RunRequest) -> Result<RunResult, AgentError> {
        self.run(req).await
    }

    fn tool_specs_from_registry(&self) -> Vec<ToolSpec> {
        let mut tools = self.dispatcher.registry().values().collect::<Vec<_>>();
        tools.sort_by_key(|tool| tool.name());
        tools
            .into_iter()
            .map(|tool| ToolSpec::function(tool.name(), tool.description(), tool.schema()))
            .collect()
    }

    async fn create_baseline_plan(&self, session_id: &str, input: &str) -> Result<(), AgentError> {
        self.events
            .append(NewEvent {
                session_id: session_id.to_string(),
                event_type: EventType::Plan,
                source: "agent".into(),
                data: json!({
                    "op": "create",
                    "plan_id": "baseline",
                    "snapshot": {
                        "id": "baseline",
                        "session_id": session_id,
                        "goal": input,
                        "phases": [{
                            "id": 1,
                            "title": input,
                            "capabilities": [],
                            "status": "active",
                        }],
                        "current_phase_id": 1,
                    },
                }),
            })
            .await?;
        Ok(())
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
