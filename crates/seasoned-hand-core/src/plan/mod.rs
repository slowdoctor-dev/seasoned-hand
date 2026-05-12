use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;
use uuid::Uuid;

use crate::db::DbPool;
use crate::events::{EventError, EventStore, EventType, NewEvent, sqlite::SqliteEventStore};

pub mod render;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PhaseStatus {
    Pending,
    Active,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Phase {
    pub id: u32,
    pub title: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub status: PhaseStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Plan {
    pub id: String,
    pub session_id: String,
    pub goal: String,
    pub phases: Vec<Phase>,
    pub current_phase_id: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PlanMutationSource {
    Agent,
    Verifier,
    Runtime,
}
impl PlanMutationSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Verifier => "verifier",
            Self::Runtime => "runtime",
        }
    }
}

#[derive(Debug, Error)]
pub enum PlanError {
    #[error("db error: {0}")]
    Db(#[from] crate::db::DbError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("event error: {0}")]
    Event(#[from] EventError),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("clock error: {0}")]
    Clock(String),
    #[error("plan already exists for session {0}")]
    AlreadyExists(String),
    #[error("plan not found for session {0}")]
    NotFound(String),
    #[error("invalid phases: {0}")]
    InvalidPhases(&'static str),
}

#[derive(Clone)]
pub struct PlanManager {
    pool: DbPool,
    events: Arc<SqliteEventStore>,
}

impl PlanManager {
    pub fn new(pool: DbPool, events: Arc<SqliteEventStore>) -> Self {
        Self { pool, events }
    }

    pub async fn create(
        &self,
        session_id: &str,
        goal: &str,
        phases: Vec<Phase>,
    ) -> Result<Plan, PlanError> {
        if phases.is_empty() {
            return Err(PlanError::InvalidPhases("must include at least one phase"));
        }
        let now = now_micros()?;
        let plan_id = Uuid::new_v4().to_string();
        let session = session_id.to_string();
        let goal = goal.to_string();
        let normalized = normalize_create_phases(phases);
        let current_phase_id = normalized
            .iter()
            .find(|p| p.status == PhaseStatus::Active)
            .map(|p| p.id);
        let phases_json = serde_json::to_string(&normalized)?;

        let snapshot = self.pool.with_conn(move |conn| -> Result<Plan, PlanError> {
            let existing: Option<String> = conn.query_row("SELECT id FROM plans WHERE session_id = ?", rusqlite::params![&session], |row| row.get(0)).optional()?;
            if existing.is_some() { return Err(PlanError::AlreadyExists(session.clone())); }
            conn.execute("INSERT INTO plans (id, session_id, goal, phases, current_phase_id, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)", rusqlite::params![&plan_id, &session, &goal, &phases_json, current_phase_id, now, now])?;
            Ok(Plan { id: plan_id, session_id: session, goal, phases: normalized, current_phase_id })
        }).await?;

        self.emit_event(
            &snapshot.session_id,
            json!({"op":"create","snapshot":snapshot}),
        )
        .await?;
        Ok(snapshot)
    }

    pub async fn advance(&self, session_id: &str) -> Result<Plan, PlanError> {
        let now = now_micros()?;
        let session = session_id.to_string();
        let snapshot = self
            .pool
            .with_conn(move |conn| -> Result<Plan, PlanError> {
                let mut plan = load_plan(conn, &session)?;
                for phase in &mut plan.phases {
                    if phase.status == PhaseStatus::Active {
                        phase.status = PhaseStatus::Done;
                    }
                }
                if let Some(idx) = plan
                    .phases
                    .iter()
                    .position(|phase| phase.status == PhaseStatus::Pending)
                {
                    plan.phases[idx].status = PhaseStatus::Active;
                    plan.current_phase_id = Some(plan.phases[idx].id);
                } else {
                    plan.current_phase_id = None;
                }
                persist_plan(conn, &plan, now)?;
                Ok(plan)
            })
            .await?;

        self.emit_event(&snapshot.session_id, json!({"op":"advance","terminal":snapshot.current_phase_id.is_none(),"snapshot":snapshot})).await?;
        Ok(snapshot)
    }

    pub async fn update(
        &self,
        session_id: &str,
        phases: Vec<Phase>,
        source: PlanMutationSource,
    ) -> Result<Plan, PlanError> {
        if phases.is_empty() {
            return Err(PlanError::InvalidPhases("must include at least one phase"));
        }
        let now = now_micros()?;
        let session = session_id.to_string();
        let snapshot = self
            .pool
            .with_conn(move |conn| -> Result<Plan, PlanError> {
                let mut plan = load_plan(conn, &session)?;
                plan.phases = normalize_update_phases(phases);
                plan.current_phase_id = plan
                    .phases
                    .iter()
                    .find(|p| p.status == PhaseStatus::Active)
                    .map(|p| p.id);
                persist_plan(conn, &plan, now)?;
                Ok(plan)
            })
            .await?;

        self.emit_event(
            &snapshot.session_id,
            json!({"op":"update","source":source.as_str(),"snapshot":snapshot}),
        )
        .await?;
        Ok(snapshot)
    }

    pub async fn snapshot(&self, session_id: &str) -> Result<Plan, PlanError> {
        let session = session_id.to_string();
        self.pool
            .with_conn(move |conn| load_plan(conn, &session))
            .await
    }

    async fn emit_event(&self, session_id: &str, data: serde_json::Value) -> Result<(), PlanError> {
        self.events
            .append(NewEvent {
                session_id: session_id.to_string(),
                event_type: EventType::Plan,
                source: "plan_manager".into(),
                data,
            })
            .await?;
        Ok(())
    }
}

fn normalize_create_phases(mut phases: Vec<Phase>) -> Vec<Phase> {
    phases.sort_by_key(|p| p.id);
    for phase in &mut phases {
        phase.status = PhaseStatus::Pending;
    }
    if let Some(first) = phases.first_mut() {
        first.status = PhaseStatus::Active;
    }
    phases
}

fn normalize_update_phases(mut phases: Vec<Phase>) -> Vec<Phase> {
    phases.sort_by_key(|p| p.id);
    let pending_min = phases
        .iter()
        .filter(|p| p.status == PhaseStatus::Pending)
        .map(|p| p.id)
        .min();
    if let Some(id) = pending_min {
        for phase in &mut phases {
            phase.status = if phase.id == id {
                PhaseStatus::Active
            } else if phase.status == PhaseStatus::Done {
                PhaseStatus::Done
            } else {
                PhaseStatus::Pending
            };
        }
        return phases;
    }
    let active_min = phases
        .iter()
        .filter(|p| p.status == PhaseStatus::Active)
        .map(|p| p.id)
        .min();
    if let Some(id) = active_min {
        for phase in &mut phases {
            if phase.status == PhaseStatus::Active && phase.id != id {
                phase.status = PhaseStatus::Pending;
            }
        }
    }
    phases
}

fn persist_plan(conn: &rusqlite::Connection, plan: &Plan, now: i64) -> Result<(), PlanError> {
    let phases_json = serde_json::to_string(&plan.phases)?;
    conn.execute(
        "UPDATE plans SET phases = ?, current_phase_id = ?, updated_at = ? WHERE id = ?",
        rusqlite::params![phases_json, plan.current_phase_id, now, plan.id],
    )?;
    Ok(())
}

fn load_plan(conn: &rusqlite::Connection, session_id: &str) -> Result<Plan, PlanError> {
    let row: (String, String, String, String, Option<u32>) = conn
        .query_row(
            "SELECT id, session_id, goal, phases, current_phase_id FROM plans WHERE session_id = ?",
            rusqlite::params![session_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .map_err(|err| {
            if matches!(err, rusqlite::Error::QueryReturnedNoRows) {
                PlanError::NotFound(session_id.to_string())
            } else {
                PlanError::Sqlite(err)
            }
        })?;
    Ok(Plan {
        id: row.0,
        session_id: row.1,
        goal: row.2,
        phases: serde_json::from_str(&row.3)?,
        current_phase_id: row.4,
    })
}

fn now_micros() -> Result<i64, PlanError> {
    let micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| PlanError::Clock(e.to_string()))?
        .as_micros();
    i64::try_from(micros).map_err(|e| PlanError::Clock(e.to_string()))
}

#[cfg(test)]
mod tests;
