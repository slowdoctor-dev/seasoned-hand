//! Task entity + store (V006 `tasks` table) + status state machine.
//!
//! State machine:
//! `drafted → briefed → confirmed → running ⇄ paused → completed | failed | cancelled`
//!
//! Illegal moves return [`TaskError::IllegalTransition`]; they never
//! panic. Terminal states (`Completed`, `Failed`, `Cancelled`) accept no
//! further transitions.
//!
//! refs: /specs/phase-2/architecture.md §2.1, §3 V006
//! refs: /specs/phase-2/stories/story-2.2.md

use rusqlite::params;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::db::DbPool;
use crate::time::now_micros;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Drafted,
    Briefed,
    Confirmed,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

impl TaskStatus {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            TaskStatus::Drafted => "drafted",
            TaskStatus::Briefed => "briefed",
            TaskStatus::Confirmed => "confirmed",
            TaskStatus::Running => "running",
            TaskStatus::Paused => "paused",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
            TaskStatus::Cancelled => "cancelled",
        }
    }

    pub fn from_db_str(s: &str) -> Result<Self, TaskError> {
        match s {
            "drafted" => Ok(TaskStatus::Drafted),
            "briefed" => Ok(TaskStatus::Briefed),
            "confirmed" => Ok(TaskStatus::Confirmed),
            "running" => Ok(TaskStatus::Running),
            "paused" => Ok(TaskStatus::Paused),
            "completed" => Ok(TaskStatus::Completed),
            "failed" => Ok(TaskStatus::Failed),
            "cancelled" => Ok(TaskStatus::Cancelled),
            other => Err(TaskError::UnknownStatus(other.to_string())),
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
        )
    }
}

/// Single inspectable table of legal `from → to` transitions. Returning a
/// static slice keeps the validity matrix in one place and avoids the
/// match-arm explosion of nested `(from, to)` pairs.
///
/// User-cancel paths (`*  → Cancelled`) are allowed from every
/// pre-running state so the story-2.8 Briefing gate can honor a
/// `cancel` action without first walking the task through to `running`.
pub fn legal_transitions(from: TaskStatus) -> &'static [TaskStatus] {
    use TaskStatus::*;
    match from {
        Drafted => &[Briefed, Cancelled],
        Briefed => &[Confirmed, Cancelled],
        Confirmed => &[Running, Cancelled],
        Running => &[Paused, Completed, Failed, Cancelled],
        Paused => &[Running, Completed, Failed, Cancelled],
        Completed | Failed | Cancelled => &[],
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub project_id: String,
    pub tenant_id: Option<String>,
    pub title: String,
    /// Phase 2 stores the structured `Brief` (see architecture §2.2) as
    /// raw JSON text; this column is `NULL` until the Initializer writes
    /// the brief.
    pub brief: Option<serde_json::Value>,
    pub status: TaskStatus,
    pub expected_due_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub failure_reason: Option<String>,
    pub parent_task_id: Option<String>,
    pub schedule: Option<String>,
    pub skill_attached_event_id: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewTask {
    pub project_id: String,
    pub tenant_id: Option<String>,
    pub title: String,
    pub expected_due_at: Option<i64>,
}

#[derive(Debug, Error)]
pub enum TaskError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("task not found: {0}")]
    NotFound(String),
    #[error("unknown task status in DB: {0}")]
    UnknownStatus(String),
    #[error("illegal task status transition: {from:?} → {to:?}")]
    IllegalTransition { from: TaskStatus, to: TaskStatus },
}

#[derive(Clone)]
pub struct TaskStore {
    pool: DbPool,
}

impl TaskStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, new: NewTask) -> Result<String, TaskError> {
        let id = Uuid::new_v4().to_string();
        let now = now_micros();
        let id_clone = id.clone();
        let res: Result<usize, rusqlite::Error> = self
            .pool
            .with_conn(move |conn| {
                conn.execute(
                    "INSERT INTO tasks (\
                       id, project_id, tenant_id, title, brief, status, \
                       expected_due_at, completed_at, failure_reason, \
                       parent_task_id, schedule, skill_attached_event_id, \
                       created_at, updated_at\
                     ) VALUES (?, ?, ?, ?, NULL, ?, ?, NULL, NULL, NULL, NULL, NULL, ?, ?)",
                    params![
                        id_clone,
                        new.project_id,
                        new.tenant_id,
                        new.title,
                        TaskStatus::Drafted.as_db_str(),
                        new.expected_due_at,
                        now,
                        now,
                    ],
                )
            })
            .await;
        res?;
        Ok(id)
    }

    pub async fn get(&self, id: &str) -> Result<Task, TaskError> {
        let id_owned = id.to_string();
        let row: Option<RawRow> = self
            .pool
            .with_conn(move |conn| -> rusqlite::Result<Option<RawRow>> {
                let mut stmt = conn.prepare(SELECT_COLUMNS_WITH_FROM)?;
                let mut rows = stmt.query_map(params![id_owned], RawRow::from_row)?;
                match rows.next() {
                    Some(row) => Ok(Some(row?)),
                    None => Ok(None),
                }
            })
            .await?;
        match row {
            Some(r) => r.into_task(),
            None => Err(TaskError::NotFound(id.to_string())),
        }
    }

    /// Newest-first paginated list scoped to a project. `cursor` is the
    /// `created_at` of the last seen row (exclusive upper bound for the
    /// next page); optional `status` filter narrows by current state.
    pub async fn list_by_project(
        &self,
        project_id: &str,
        status: Option<TaskStatus>,
        cursor: Option<i64>,
        limit: usize,
    ) -> Result<Vec<Task>, TaskError> {
        let pid = project_id.to_string();
        let limit = limit.clamp(1, 200) as i64;
        let status_str = status.map(|s| s.as_db_str().to_string());
        let raws: Vec<RawRow> = self
            .pool
            .with_conn(move |conn| -> rusqlite::Result<Vec<RawRow>> {
                let rows = match (status_str.as_deref(), cursor) {
                    (Some(s), Some(c)) => {
                        let mut stmt = conn.prepare(
                            "SELECT id, project_id, tenant_id, title, brief, status, \
                                    expected_due_at, completed_at, failure_reason, \
                                    parent_task_id, schedule, skill_attached_event_id, \
                                    created_at, updated_at \
                               FROM tasks \
                              WHERE project_id = ? AND status = ? AND created_at < ? \
                              ORDER BY created_at DESC LIMIT ?",
                        )?;
                        stmt.query_map(params![pid, s, c, limit], RawRow::from_row)?
                            .collect::<rusqlite::Result<Vec<_>>>()?
                    }
                    (Some(s), None) => {
                        let mut stmt = conn.prepare(
                            "SELECT id, project_id, tenant_id, title, brief, status, \
                                    expected_due_at, completed_at, failure_reason, \
                                    parent_task_id, schedule, skill_attached_event_id, \
                                    created_at, updated_at \
                               FROM tasks \
                              WHERE project_id = ? AND status = ? \
                              ORDER BY created_at DESC LIMIT ?",
                        )?;
                        stmt.query_map(params![pid, s, limit], RawRow::from_row)?
                            .collect::<rusqlite::Result<Vec<_>>>()?
                    }
                    (None, Some(c)) => {
                        let mut stmt = conn.prepare(
                            "SELECT id, project_id, tenant_id, title, brief, status, \
                                    expected_due_at, completed_at, failure_reason, \
                                    parent_task_id, schedule, skill_attached_event_id, \
                                    created_at, updated_at \
                               FROM tasks \
                              WHERE project_id = ? AND created_at < ? \
                              ORDER BY created_at DESC LIMIT ?",
                        )?;
                        stmt.query_map(params![pid, c, limit], RawRow::from_row)?
                            .collect::<rusqlite::Result<Vec<_>>>()?
                    }
                    (None, None) => {
                        let mut stmt = conn.prepare(
                            "SELECT id, project_id, tenant_id, title, brief, status, \
                                    expected_due_at, completed_at, failure_reason, \
                                    parent_task_id, schedule, skill_attached_event_id, \
                                    created_at, updated_at \
                               FROM tasks \
                              WHERE project_id = ? \
                              ORDER BY created_at DESC LIMIT ?",
                        )?;
                        stmt.query_map(params![pid, limit], RawRow::from_row)?
                            .collect::<rusqlite::Result<Vec<_>>>()?
                    }
                };
                Ok(rows)
            })
            .await?;
        raws.into_iter().map(RawRow::into_task).collect()
    }

    /// Move a task to `to`, validating the state machine. Reads the
    /// current status, checks that `to` is in `legal_transitions(from)`,
    /// then writes. Side-effect columns (`completed_at`, `failure_reason`)
    /// are not touched here; use [`Self::set_completed`] / [`Self::set_failure`]
    /// for atomic state-plus-payload moves.
    pub async fn set_status(&self, id: &str, to: TaskStatus) -> Result<(), TaskError> {
        let current = self.get(id).await?.status;
        if !legal_transitions(current).contains(&to) {
            return Err(TaskError::IllegalTransition { from: current, to });
        }
        self.write_status(id, to, None, None).await
    }

    /// Atomic move to [`TaskStatus::Failed`] + persist `failure_reason`.
    /// Validates that `Failed` is reachable from the current state.
    pub async fn set_failure(&self, id: &str, reason: &str) -> Result<(), TaskError> {
        let current = self.get(id).await?.status;
        if !legal_transitions(current).contains(&TaskStatus::Failed) {
            return Err(TaskError::IllegalTransition {
                from: current,
                to: TaskStatus::Failed,
            });
        }
        self.write_status(id, TaskStatus::Failed, None, Some(reason.to_string()))
            .await
    }

    /// Atomic move to [`TaskStatus::Completed`] + stamp `completed_at`.
    /// Validates that `Completed` is reachable from the current state.
    pub async fn set_completed(&self, id: &str) -> Result<(), TaskError> {
        let current = self.get(id).await?.status;
        if !legal_transitions(current).contains(&TaskStatus::Completed) {
            return Err(TaskError::IllegalTransition {
                from: current,
                to: TaskStatus::Completed,
            });
        }
        let now = now_micros();
        self.write_status(id, TaskStatus::Completed, Some(now), None)
            .await
    }

    /// Persist a structured brief as JSON text on the row. Does not move
    /// the state machine; the caller (Initializer) issues the matching
    /// `set_status(Briefed)` separately so brief authorship and state
    /// progression stay independently auditable.
    pub async fn set_brief(&self, id: &str, brief: &serde_json::Value) -> Result<(), TaskError> {
        let id_owned = id.to_string();
        let now = now_micros();
        let brief_text = serde_json::to_string(brief)?;
        let affected: usize = self
            .pool
            .with_conn(move |conn| {
                conn.execute(
                    "UPDATE tasks SET brief = ?, updated_at = ? WHERE id = ?",
                    params![brief_text, now, id_owned],
                )
            })
            .await?;
        if affected == 0 {
            return Err(TaskError::NotFound(id.to_string()));
        }
        Ok(())
    }

    async fn write_status(
        &self,
        id: &str,
        to: TaskStatus,
        completed_at: Option<i64>,
        failure_reason: Option<String>,
    ) -> Result<(), TaskError> {
        let id_owned = id.to_string();
        let now = now_micros();
        let to_str = to.as_db_str().to_string();
        let affected: usize = self
            .pool
            .with_conn(move |conn| -> rusqlite::Result<usize> {
                match (completed_at, failure_reason) {
                    (Some(ts), _) => conn.execute(
                        "UPDATE tasks SET status = ?, completed_at = ?, updated_at = ? \
                         WHERE id = ?",
                        params![to_str, ts, now, id_owned],
                    ),
                    (None, Some(reason)) => conn.execute(
                        "UPDATE tasks SET status = ?, failure_reason = ?, updated_at = ? \
                         WHERE id = ?",
                        params![to_str, reason, now, id_owned],
                    ),
                    (None, None) => conn.execute(
                        "UPDATE tasks SET status = ?, updated_at = ? WHERE id = ?",
                        params![to_str, now, id_owned],
                    ),
                }
            })
            .await?;
        if affected == 0 {
            return Err(TaskError::NotFound(id.to_string()));
        }
        Ok(())
    }
}

const SELECT_COLUMNS_WITH_FROM: &str = "SELECT id, project_id, tenant_id, title, brief, status, \
            expected_due_at, completed_at, failure_reason, \
            parent_task_id, schedule, skill_attached_event_id, \
            created_at, updated_at \
       FROM tasks WHERE id = ?";

struct RawRow {
    id: String,
    project_id: String,
    tenant_id: Option<String>,
    title: String,
    brief: Option<String>,
    status: String,
    expected_due_at: Option<i64>,
    completed_at: Option<i64>,
    failure_reason: Option<String>,
    parent_task_id: Option<String>,
    schedule: Option<String>,
    skill_attached_event_id: Option<i64>,
    created_at: i64,
    updated_at: i64,
}

impl RawRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            project_id: row.get(1)?,
            tenant_id: row.get(2)?,
            title: row.get(3)?,
            brief: row.get(4)?,
            status: row.get(5)?,
            expected_due_at: row.get(6)?,
            completed_at: row.get(7)?,
            failure_reason: row.get(8)?,
            parent_task_id: row.get(9)?,
            schedule: row.get(10)?,
            skill_attached_event_id: row.get(11)?,
            created_at: row.get(12)?,
            updated_at: row.get(13)?,
        })
    }

    fn into_task(self) -> Result<Task, TaskError> {
        let status = TaskStatus::from_db_str(&self.status)?;
        let brief = match self.brief {
            Some(text) => Some(serde_json::from_str::<serde_json::Value>(&text)?),
            None => None,
        };
        Ok(Task {
            id: self.id,
            project_id: self.project_id,
            tenant_id: self.tenant_id,
            title: self.title,
            brief,
            status,
            expected_due_at: self.expected_due_at,
            completed_at: self.completed_at,
            failure_reason: self.failure_reason,
            parent_task_id: self.parent_task_id,
            schedule: self.schedule,
            skill_attached_event_id: self.skill_attached_event_id,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[cfg(test)]
impl TaskStore {
    /// Test-only escape hatch: lets tests pre-seed rows with controlled
    /// `created_at` values for cursor-pagination assertions.
    pub(crate) fn pool_for_test(&self) -> &DbPool {
        &self.pool
    }
}
