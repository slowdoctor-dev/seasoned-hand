use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;
use thiserror::Error;

use crate::events::{EventStore, EventType, NewEvent, sqlite::SqliteEventStore};
use crate::llm::{ChatCompletionRequest, LlmClient, Message, Role};
use crate::plan::{PhaseStatus, Plan, PlanError, PlanManager};
use crate::router::{SlotName, SlotRouter};
use crate::sandbox::{SandboxClient, SandboxError};

pub mod feature_list;
pub mod progress;

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
}

#[derive(Clone)]
pub struct Initializer {
    router: Arc<SlotRouter>,
    plan_manager: Arc<PlanManager>,
    sandbox: Arc<SandboxClient>,
    events: Arc<SqliteEventStore>,
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
            planner_prompt: read_planner_prompt().unwrap_or_else(|_| DEFAULT_PLANNER_PROMPT.into()),
        }
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
