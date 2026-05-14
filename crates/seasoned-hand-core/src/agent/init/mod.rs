use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;
use thiserror::Error;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::events::{EventStore, EventType, NewEvent, sqlite::SqliteEventStore};
use crate::llm::{ChatCompletionRequest, LlmClient, Message, Role};
use crate::plan::{PhaseStatus, Plan, PlanError, PlanManager};
use crate::project::brief::{Brief, BriefError, BriefPhase};
use crate::project::task::{TaskError, TaskStatus, TaskStore};
use crate::router::{SlotName, SlotRouter};
use crate::sandbox::{SandboxClient, SandboxError};

pub mod briefing;
pub mod feature_list;
pub mod progress;

use briefing::{BriefingAction, MAX_EDIT_CYCLES, RunConfig, RunOutcome, UserResponse, apply_edits};
use feature_list::{Feature, FeatureList, FeatureStatus};

const FALLBACK_REASON_PLANNER_ERROR: &str = "planner_error";
const FALLBACK_REASON_MALFORMED: &str = "malformed_plan";
const FALLBACK_REASON_ZERO_PHASE: &str = "zero_phase";

#[derive(Debug, Clone)]
pub struct InitReport {
    pub plan: Plan,
    pub feature_count: usize,
}

#[derive(Debug, Error)]
pub enum InitError {
    #[error("llm error: {0}")]
    Llm(#[from] crate::llm::LlmError),
    #[error("plan error: {0}")]
    Plan(#[from] PlanError),
    #[error("sandbox error: {0}")]
    Sandbox(#[from] SandboxError),
    #[error("event error: {0}")]
    Event(#[from] crate::events::EventError),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("task store error: {0}")]
    Task(#[from] TaskError),
    #[error("brief error: {0}")]
    Brief(#[from] BriefError),
    #[error("user_response channel closed before confirm")]
    UserResponseChannelClosed,
    #[error("task_store not configured on Initializer; required for run_with_confirmation")]
    TaskStoreNotConfigured,
}

#[derive(Clone)]
pub struct Initializer {
    router: Arc<SlotRouter>,
    plan_manager: Arc<PlanManager>,
    sandbox: Arc<SandboxClient>,
    events: Arc<SqliteEventStore>,
    /// Optional — required by [`Initializer::run_with_confirmation`] but
    /// unused by the Phase 1 legacy [`Initializer::run`] entry point.
    /// Builder method [`Initializer::with_task_store`] sets it.
    task_store: Option<Arc<TaskStore>>,
    planner_prompt: String,
}

impl Initializer {
    pub fn new(
        router: Arc<SlotRouter>,
        plan_manager: Arc<PlanManager>,
        sandbox: Arc<SandboxClient>,
        events: Arc<SqliteEventStore>,
    ) -> Self {
        Self {
            router,
            plan_manager,
            sandbox,
            events,
            task_store: None,
            planner_prompt: read_planner_prompt().unwrap_or_else(|_| DEFAULT_PLANNER_PROMPT.into()),
        }
    }

    /// Attach a [`TaskStore`] handle. Required for
    /// [`Self::run_with_confirmation`]; the Phase 1 [`Self::run`]
    /// entry point ignores it.
    pub fn with_task_store(mut self, task_store: Arc<TaskStore>) -> Self {
        self.task_store = Some(task_store);
        self
    }

    pub async fn run(&self, session_id: &str, briefing: &str) -> Result<InitReport, InitError> {
        let planned = match self.call_planner_slot(briefing).await {
            Ok(plan) => plan,
            Err(_) => {
                self.emit_fallback(session_id, FALLBACK_REASON_PLANNER_ERROR)
                    .await?;
                fallback_single_phase_plan(briefing)
            }
        };

        let normalized = match normalize_or_fallback(planned) {
            Ok(plan) => plan,
            Err(reason) => {
                self.emit_fallback(session_id, reason).await?;
                fallback_single_phase_plan(briefing)
            }
        };

        let goal = normalized.goal.clone();
        let plan = self
            .plan_manager
            .create(session_id, &goal, normalized.into_phases())
            .await?;

        let feature_list = derive_feature_list(&plan);
        self.sandbox
            .write_workspace_file_json(session_id, "feature-list.json", &feature_list)
            .await?;
        let progress_text = initial_progress_lines(&plan);
        self.sandbox
            .write_workspace_file(session_id, "progress.txt", progress_text.as_bytes())
            .await?;

        Ok(InitReport {
            plan,
            feature_count: feature_list.features.len(),
        })
    }

    /// Phase 2 entry point — author a [`Brief`], persist it onto the
    /// task row, emit `briefing_pending` + `briefing` Misc events, then
    /// wait for the user's `confirm` / `edit` / `cancel` action on the
    /// per-task `recv` receiver. Auto-confirms after
    /// `config.confirm_timeout` unless `config.require_confirm` disables
    /// the timer.
    ///
    /// On confirm (manual or auto): seeds the Plan, writes the
    /// feature-list / progress sandbox files (matching Phase 1
    /// [`Self::run`] semantics), moves the task `briefed → confirmed →
    /// running`, returns [`RunOutcome::Started`].
    ///
    /// On `cancel`: moves the task to `cancelled`, emits a `task_state`
    /// Misc with `reason: "user_cancelled"`, returns
    /// [`RunOutcome::Cancelled`].
    ///
    /// On `edit`: applies the [`briefing::PartialBrief`] overlay,
    /// re-validates, persists the updated brief, re-emits `briefing`
    /// with a fresh `briefing_call_id`, and loops back to wait. Capped
    /// at [`MAX_EDIT_CYCLES`] cycles — the next edit returns
    /// [`InitError::Brief`] (`BriefError::TooManyEdits`).
    ///
    /// refs: /specs/phase-2/architecture.md §2.2
    pub async fn run_with_confirmation(
        &self,
        session_id: &str,
        task_id: &str,
        raw_input: &str,
        config: RunConfig,
        recv: mpsc::Receiver<UserResponse>,
    ) -> Result<RunOutcome, InitError> {
        let task_store = self
            .task_store
            .clone()
            .ok_or(InitError::TaskStoreNotConfigured)?;
        let brief = self.author_brief(session_id, raw_input).await?;
        self.run_confirm_gate(session_id, task_id, brief, config, recv, task_store)
            .await
    }

    async fn author_brief(&self, session_id: &str, raw_input: &str) -> Result<Brief, InitError> {
        // Reuse the Phase 1 planner-slot LLM call; if either the call
        // itself or the Brief parse / validate fails, fall back to a
        // single-phase Brief and emit the existing fallback Misc so
        // observability stays uniform between the two entry points.
        match self.call_planner_slot(raw_input).await {
            Ok(planned) => {
                let candidate = planner_output_to_brief(planned);
                match candidate.validate() {
                    Ok(()) => Ok(candidate),
                    Err(_) => {
                        self.emit_fallback(session_id, FALLBACK_REASON_MALFORMED)
                            .await?;
                        Ok(fallback_brief(raw_input))
                    }
                }
            }
            Err(_) => {
                self.emit_fallback(session_id, FALLBACK_REASON_PLANNER_ERROR)
                    .await?;
                Ok(fallback_brief(raw_input))
            }
        }
    }

    async fn run_confirm_gate(
        &self,
        session_id: &str,
        task_id: &str,
        initial_brief: Brief,
        config: RunConfig,
        mut recv: mpsc::Receiver<UserResponse>,
        task_store: Arc<TaskStore>,
    ) -> Result<RunOutcome, InitError> {
        let mut current = initial_brief;
        task_store.set_brief(task_id, &current.serialize()).await?;
        // Drafted → Briefed. Phase 2 tests pre-create the task in
        // `drafted` (TaskStore::insert default); the legacy WS shim
        // (DEBT #15) does not call run_with_confirmation yet, so no
        // alternative starting state is in play.
        task_store.set_status(task_id, TaskStatus::Briefed).await?;

        let mut edits_applied: u32 = 0;
        loop {
            let call_id = Uuid::new_v4().to_string();
            self.emit_briefing_pending(session_id, &call_id, task_id)
                .await?;
            self.emit_briefing(session_id, &call_id, &current).await?;

            let resp =
                wait_for_response(&mut recv, &config, &self.events, session_id, &call_id).await?;
            match resp {
                WaitOutcome::AutoConfirmed => {
                    return self
                        .seed_plan_and_run(session_id, task_id, &current, &task_store)
                        .await
                        .map(|()| RunOutcome::Started);
                }
                WaitOutcome::Response(user) => match user.action {
                    BriefingAction::Confirm => {
                        return self
                            .seed_plan_and_run(session_id, task_id, &current, &task_store)
                            .await
                            .map(|()| RunOutcome::Started);
                    }
                    BriefingAction::Cancel => {
                        task_store
                            .set_status(task_id, TaskStatus::Cancelled)
                            .await?;
                        self.emit_task_state(session_id, "cancelled", "user_cancelled")
                            .await?;
                        return Ok(RunOutcome::Cancelled);
                    }
                    BriefingAction::Edit { edits } => {
                        if edits_applied >= MAX_EDIT_CYCLES {
                            return Err(InitError::Brief(BriefError::TooManyEdits));
                        }
                        edits_applied += 1;
                        apply_edits(&mut current, edits);
                        current.validate()?;
                        task_store.set_brief(task_id, &current.serialize()).await?;
                        // Loop — next iteration emits a fresh
                        // briefing_call_id, matching architecture §2.2's
                        // "re-emits Briefing event with a NEW briefing_call_id".
                    }
                },
            }
        }
    }

    async fn seed_plan_and_run(
        &self,
        session_id: &str,
        task_id: &str,
        brief: &Brief,
        task_store: &Arc<TaskStore>,
    ) -> Result<(), InitError> {
        let phases = brief_phases_to_plan_phases(&brief.phases);
        let plan = self
            .plan_manager
            .create(session_id, &brief.goal, phases)
            .await?;
        let feature_list = derive_feature_list(&plan);
        self.sandbox
            .write_workspace_file_json(session_id, "feature-list.json", &feature_list)
            .await?;
        let progress_text = initial_progress_lines(&plan);
        self.sandbox
            .write_workspace_file(session_id, "progress.txt", progress_text.as_bytes())
            .await?;
        task_store
            .set_status(task_id, TaskStatus::Confirmed)
            .await?;
        task_store.set_status(task_id, TaskStatus::Running).await?;
        Ok(())
    }

    async fn emit_briefing_pending(
        &self,
        session_id: &str,
        call_id: &str,
        task_id: &str,
    ) -> Result<(), InitError> {
        self.events
            .append(NewEvent {
                session_id: session_id.to_string(),
                event_type: EventType::Misc,
                source: "initializer".into(),
                data: json!({
                    "kind": "briefing_pending",
                    "briefing_call_id": call_id,
                    "task_id": task_id,
                }),
            })
            .await?;
        Ok(())
    }

    async fn emit_briefing(
        &self,
        session_id: &str,
        call_id: &str,
        brief: &Brief,
    ) -> Result<(), InitError> {
        self.events
            .append(NewEvent {
                session_id: session_id.to_string(),
                event_type: EventType::Misc,
                source: "initializer".into(),
                data: json!({
                    "kind": "briefing",
                    "briefing_call_id": call_id,
                    "brief": brief.serialize(),
                }),
            })
            .await?;
        Ok(())
    }

    async fn emit_task_state(
        &self,
        session_id: &str,
        to: &str,
        reason: &str,
    ) -> Result<(), InitError> {
        self.events
            .append(NewEvent {
                session_id: session_id.to_string(),
                event_type: EventType::Misc,
                source: "initializer".into(),
                data: json!({
                    "kind": "task_state",
                    "to": to,
                    "reason": reason,
                }),
            })
            .await?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) async fn run_with_confirmation_for_test(
        &self,
        session_id: &str,
        task_id: &str,
        brief: Brief,
        config: RunConfig,
        recv: mpsc::Receiver<UserResponse>,
    ) -> Result<RunOutcome, InitError> {
        let task_store = self
            .task_store
            .clone()
            .ok_or(InitError::TaskStoreNotConfigured)?;
        self.run_confirm_gate(session_id, task_id, brief, config, recv, task_store)
            .await
    }

    #[cfg(test)]
    pub(crate) async fn run_with_planner_output_for_test(
        &self,
        session_id: &str,
        briefing: &str,
        planned: PlannerOutput,
    ) -> Result<InitReport, InitError> {
        let normalized = match normalize_or_fallback(planned) {
            Ok(plan) => plan,
            Err(reason) => {
                self.emit_fallback(session_id, reason).await?;
                fallback_single_phase_plan(briefing)
            }
        };
        let goal = normalized.goal.clone();
        let plan = self
            .plan_manager
            .create(session_id, &goal, normalized.into_phases())
            .await?;
        let feature_list = derive_feature_list(&plan);
        self.sandbox
            .write_workspace_file_json(session_id, "feature-list.json", &feature_list)
            .await?;
        let progress_text = initial_progress_lines(&plan);
        self.sandbox
            .write_workspace_file(session_id, "progress.txt", progress_text.as_bytes())
            .await?;
        Ok(InitReport {
            plan,
            feature_count: feature_list.features.len(),
        })
    }

    async fn call_planner_slot(
        &self,
        briefing: &str,
    ) -> Result<PlannerOutput, crate::llm::LlmError> {
        let slot = self.router.resolve(SlotName::Planner);
        let planner_client = LlmClient::new(slot.base_url.clone(), slot.api_key.clone());
        let resp = planner_client
            .chat_completion(ChatCompletionRequest {
                model: slot.model.clone(),
                messages: vec![
                    Message {
                        role: Role::System,
                        content: Some(self.planner_prompt.clone()),
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
                    },
                    Message {
                        role: Role::User,
                        content: Some(briefing.to_string()),
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
                    },
                ],
                tools: None,
                tool_choice: None,
                temperature: Some(0.0),
                max_tokens: Some(800),
                top_p: None,
            })
            .await?;

        let content = resp
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .unwrap_or_default();
        serde_json::from_str(&content).map_err(|_| crate::llm::LlmError::MissingChoice)
    }

    async fn emit_fallback(&self, session_id: &str, reason: &str) -> Result<(), InitError> {
        self.events
            .append(NewEvent {
                session_id: session_id.to_string(),
                event_type: EventType::Misc,
                source: "initializer".into(),
                data: json!({"kind":"init_planner_fallback","reason":reason}),
            })
            .await?;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct PlannerOutput {
    goal: String,
    phases: Vec<PlannerPhase>,
}

impl PlannerOutput {
    fn into_phases(self) -> Vec<crate::plan::Phase> {
        self.phases
            .into_iter()
            .map(|p| crate::plan::Phase {
                id: p.id,
                title: p.title,
                capabilities: p.capabilities,
                status: crate::plan::PhaseStatus::Pending,
            })
            .collect()
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct PlannerPhase {
    id: u32,
    title: String,
    #[serde(default)]
    capabilities: Vec<String>,
}

enum WaitOutcome {
    Response(UserResponse),
    AutoConfirmed,
}

/// `tokio::select!` on `recv` vs the auto-confirm sleep, with the
/// `biased;` keyword so an arrived response always wins over a
/// coincident timeout boundary. When `config.require_confirm` is true
/// the sleep branch is disabled entirely — `recv.recv()` is unbounded.
async fn wait_for_response(
    recv: &mut mpsc::Receiver<UserResponse>,
    config: &RunConfig,
    events: &Arc<SqliteEventStore>,
    session_id: &str,
    briefing_call_id: &str,
) -> Result<WaitOutcome, InitError> {
    if config.require_confirm {
        let resp = recv
            .recv()
            .await
            .ok_or(InitError::UserResponseChannelClosed)?;
        return Ok(WaitOutcome::Response(resp));
    }
    let outcome = tokio::select! {
        biased;
        resp = recv.recv() => match resp {
            Some(user) => WaitOutcome::Response(user),
            None => return Err(InitError::UserResponseChannelClosed),
        },
        _ = tokio::time::sleep(config.confirm_timeout) => {
            events
                .append(NewEvent {
                    session_id: session_id.to_string(),
                    event_type: EventType::Misc,
                    source: "initializer".into(),
                    data: json!({
                        "kind": "briefing_auto_confirmed",
                        "briefing_call_id": briefing_call_id,
                    }),
                })
                .await?;
            WaitOutcome::AutoConfirmed
        }
    };
    Ok(outcome)
}

fn planner_output_to_brief(planned: PlannerOutput) -> Brief {
    Brief {
        goal: planned.goal,
        phases: planned
            .phases
            .into_iter()
            .map(|p| BriefPhase {
                id: p.id,
                title: p.title,
                capabilities: p.capabilities,
            })
            .collect(),
        success_criteria: Vec::new(),
        expected_deliverables: Vec::new(),
    }
}

fn brief_phases_to_plan_phases(phases: &[BriefPhase]) -> Vec<crate::plan::Phase> {
    phases
        .iter()
        .map(|p| crate::plan::Phase {
            id: p.id,
            title: p.title.clone(),
            capabilities: p.capabilities.clone(),
            status: PhaseStatus::Pending,
        })
        .collect()
}

fn fallback_brief(raw_input: &str) -> Brief {
    Brief {
        goal: raw_input.to_string(),
        phases: vec![BriefPhase {
            id: 1,
            title: raw_input.to_string(),
            capabilities: vec![],
        }],
        success_criteria: Vec::new(),
        expected_deliverables: Vec::new(),
    }
}

fn normalize_or_fallback(plan: PlannerOutput) -> Result<PlannerOutput, &'static str> {
    if plan.phases.is_empty() {
        return Err(FALLBACK_REASON_ZERO_PHASE);
    }
    if plan.goal.trim().is_empty() || plan.phases.iter().any(|p| p.title.trim().is_empty()) {
        return Err(FALLBACK_REASON_MALFORMED);
    }
    if plan.phases.iter().any(|p| p.id == 0) {
        return Err(FALLBACK_REASON_MALFORMED);
    }
    Ok(plan)
}

fn fallback_single_phase_plan(briefing: &str) -> PlannerOutput {
    PlannerOutput {
        goal: briefing.to_string(),
        phases: vec![PlannerPhase {
            id: 1,
            title: briefing.to_string(),
            capabilities: vec![],
        }],
    }
}

fn derive_feature_list(plan: &Plan) -> FeatureList {
    let mut phases = plan.phases.clone();
    phases.sort_by_key(|p| p.id);
    let active = plan.current_phase_id;
    let features = phases
        .iter()
        .enumerate()
        .map(|(idx, phase)| Feature {
            id: format!("f-{}", idx + 1),
            title: phase.title.clone(),
            status: if Some(phase.id) == active {
                FeatureStatus::Doing
            } else if phase.status == PhaseStatus::Done {
                FeatureStatus::Done
            } else {
                FeatureStatus::Todo
            },
            depends_on: if idx == 0 {
                vec![]
            } else {
                vec![format!("f-{}", idx)]
            },
            plan_phase_id: phase.id,
            completed_at: None,
            notes: None,
        })
        .collect();

    FeatureList {
        version: 1,
        goal: plan.goal.clone(),
        features,
    }
}

fn initial_progress_lines(plan: &Plan) -> String {
    let mut out = String::new();
    out.push_str(&format!("Goal: {}\n", plan.goal));
    for phase in &plan.phases {
        out.push_str(&format!("- Phase {}: {}\n", phase.id, phase.title));
    }
    out
}

fn read_planner_prompt() -> Result<String, std::io::Error> {
    std::fs::read_to_string(Path::new("config/prompts/planner.system.txt"))
}

const DEFAULT_PLANNER_PROMPT: &str = "You are Seasoned Hand planner. Return JSON: {\"goal\":\"...\",\"phases\":[{\"id\":1,\"title\":\"...\",\"capabilities\":[]}]}";

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) fn derive_feature_list_for_test(plan: &Plan) -> FeatureList {
    derive_feature_list(plan)
}
