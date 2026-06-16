//! Checkpoint persistence — insert + paginated list.
//! refs: /specs/phase-1/architecture.md §3.3
//! refs: /specs/phase-1/stories/story-1.13.md

use rusqlite::params;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::db::DbPool;
use crate::time::now_micros;

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

    /// Fetch a single checkpoint row by id. Returns `Ok(None)` when the
    /// row does not exist; the rollback path treats that as a 404.
    pub async fn get(&self, id: &str) -> Result<Option<Checkpoint>, CheckpointPersistenceError> {
        let id_owned = id.to_string();
        let row = self
            .pool
            .with_conn(move |conn| -> rusqlite::Result<Option<Checkpoint>> {
                let mut stmt = conn.prepare(
                    "SELECT id, session_id, plan_phase_id, git_sha, label, \
                            triggered_by_event_id, rolled_back_at, rolled_back_by, created_at \
                       FROM checkpoints WHERE id = ?",
                )?;
                let mut rows = stmt.query_map(params![id_owned], |row| {
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
                })?;
                match rows.next() {
                    Some(row) => Ok(Some(row?)),
                    None => Ok(None),
                }
            })
            .await?;
        Ok(row)
    }

    /// Story 1.13b: mark a checkpoint as rolled back. Updates
    /// `rolled_back_at` and `rolled_back_by` atomically. Returns the
    /// number of affected rows (0 when the id does not exist).
    pub async fn mark_rolled_back(
        &self,
        id: &str,
        rolled_back_at: i64,
        rolled_back_by: &str,
    ) -> Result<usize, CheckpointPersistenceError> {
        let id_owned = id.to_string();
        let by_owned = rolled_back_by.to_string();
        let n = self
            .pool
            .with_conn(move |conn| {
                conn.execute(
                    "UPDATE checkpoints SET rolled_back_at = ?, rolled_back_by = ? \
                     WHERE id = ?",
                    params![rolled_back_at, by_owned, id_owned],
                )
            })
            .await?;
        Ok(n)
    }

    /// Story 1.13b: return the most recent (highest `created_at`) NON
    /// rolled-back checkpoint for a session. Used by the opt-in
    /// Verifier-driven rollback path to pick the row to revert.
    pub async fn latest_for_session(
        &self,
        session_id: &str,
    ) -> Result<Option<Checkpoint>, CheckpointPersistenceError> {
        let sid = session_id.to_string();
        let row = self
            .pool
            .with_conn(move |conn| -> rusqlite::Result<Option<Checkpoint>> {
                let mut stmt = conn.prepare(
                    "SELECT id, session_id, plan_phase_id, git_sha, label, \
                            triggered_by_event_id, rolled_back_at, rolled_back_by, created_at \
                       FROM checkpoints \
                      WHERE session_id = ? AND rolled_back_at IS NULL \
                      ORDER BY created_at DESC LIMIT 1",
                )?;
                let mut rows = stmt.query_map(params![sid], |row| {
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
                })?;
                match rows.next() {
                    Some(row) => Ok(Some(row?)),
                    None => Ok(None),
                }
            })
            .await?;
        Ok(row)
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
                // Issue #22: select the SQL by cursor; the cursor value itself is
                // bound in the query match below (no `cursor.unwrap()` — the
                // previous `has_cursor` bool let the value desync from the flag,
                // a latent panic that violated the no-unwrap rule).
                let sql = match cursor {
                    Some(_) => {
                        "SELECT id, session_id, plan_phase_id, git_sha, label, \
                                triggered_by_event_id, rolled_back_at, rolled_back_by, created_at \
                           FROM checkpoints \
                          WHERE session_id = ? AND created_at < ? \
                          ORDER BY created_at DESC LIMIT ?"
                    }
                    None => {
                        "SELECT id, session_id, plan_phase_id, git_sha, label, \
                                triggered_by_event_id, rolled_back_at, rolled_back_by, created_at \
                           FROM checkpoints \
                          WHERE session_id = ? \
                          ORDER BY created_at DESC LIMIT ?"
                    }
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
                let rows = match cursor {
                    Some(c) => stmt
                        .query_map(params![sid, c, limit], map_row)?
                        .collect::<rusqlite::Result<Vec<_>>>()?,
                    None => stmt
                        .query_map(params![sid, limit], map_row)?
                        .collect::<rusqlite::Result<Vec<_>>>()?,
                };
                Ok(rows)
            })
            .await?;
        Ok(rows)
    }
}
