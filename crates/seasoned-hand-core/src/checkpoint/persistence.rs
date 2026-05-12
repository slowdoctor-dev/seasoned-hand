//! Checkpoint persistence — insert + paginated list.
//! refs: /specs/phase-1/architecture.md §3.3
//! refs: /specs/phase-1/stories/story-1.13.md

use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::params;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::db::DbPool;

#[derive(Debug, Error)]
pub enum CheckpointPersistenceError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("checkpoint not found: {0}")]
    NotFound(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: String,
    pub session_id: String,
    pub plan_phase_id: i64,
    pub git_sha: String,
    pub label: Option<String>,
    pub triggered_by_event_id: i64,
    pub rolled_back_at: Option<i64>,
    pub rolled_back_by: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewCheckpoint {
    pub session_id: String,
    pub plan_phase_id: i64,
    pub git_sha: String,
    pub label: Option<String>,
    pub triggered_by_event_id: i64,
}

#[derive(Clone)]
pub struct CheckpointStore {
    pool: DbPool,
}

impl CheckpointStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, new: NewCheckpoint) -> Result<String, CheckpointPersistenceError> {
        let id = Uuid::new_v4().to_string();
        let created_at = now_micros();
        let id_clone = id.clone();
        let res: Result<usize, rusqlite::Error> = self
            .pool
            .with_conn(move |conn| {
                conn.execute(
                    "INSERT INTO checkpoints (\
                       id, session_id, plan_phase_id, git_sha, label, \
                       triggered_by_event_id, created_at\
                     ) VALUES (?, ?, ?, ?, ?, ?, ?)",
                    params![
                        id_clone,
                        new.session_id,
                        new.plan_phase_id,
                        new.git_sha,
                        new.label,
                        new.triggered_by_event_id,
                        created_at,
                    ],
                )
            })
            .await;
        res?;
        Ok(id)
    }

    pub async fn list_by_session(
        &self,
        session_id: &str,
        cursor: Option<i64>,
        limit: usize,
    ) -> Result<Vec<Checkpoint>, CheckpointPersistenceError> {
        let sid = session_id.to_string();
        let limit = limit.clamp(1, 200) as i64;
        let rows: Vec<Checkpoint> = self
            .pool
            .with_conn(move |conn| -> rusqlite::Result<Vec<Checkpoint>> {
                let (sql, has_cursor) = match cursor {
                    Some(_) => (
                        "SELECT id, session_id, plan_phase_id, git_sha, label, \
                                triggered_by_event_id, rolled_back_at, rolled_back_by, created_at \
                           FROM checkpoints \
                          WHERE session_id = ? AND created_at < ? \
                          ORDER BY created_at DESC LIMIT ?",
                        true,
                    ),
                    None => (
                        "SELECT id, session_id, plan_phase_id, git_sha, label, \
                                triggered_by_event_id, rolled_back_at, rolled_back_by, created_at \
                           FROM checkpoints \
                          WHERE session_id = ? \
                          ORDER BY created_at DESC LIMIT ?",
                        false,
                    ),
                };
                let mut stmt = conn.prepare(sql)?;
                let map_row = |row: &rusqlite::Row<'_>| {
                    Ok(Checkpoint {
                        id: row.get(0)?,
                        session_id: row.get(1)?,
                        plan_phase_id: row.get(2)?,
                        git_sha: row.get(3)?,
                        label: row.get(4)?,
                        triggered_by_event_id: row.get(5)?,
                        rolled_back_at: row.get(6)?,
                        rolled_back_by: row.get(7)?,
                        created_at: row.get(8)?,
                    })
                };
                let rows = if has_cursor {
                    stmt.query_map(params![sid, cursor.unwrap(), limit], map_row)?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                } else {
                    stmt.query_map(params![sid, limit], map_row)?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                };
                Ok(rows)
            })
            .await?;
        Ok(rows)
    }
}

fn now_micros() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0)
}
