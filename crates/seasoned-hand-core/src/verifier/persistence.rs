//! Verifier persistence — insert / get / paginated list.
//! refs: /specs/phase-1/architecture.md §3.1
//! refs: /specs/phase-1/stories/story-1.9.md

use rusqlite::params;
use thiserror::Error;
use uuid::Uuid;

use crate::db::DbPool;

use super::{NewVerification, VerdictKind, Verification};
use crate::time::now_micros;

#[derive(Debug, Error)]
pub enum VerifierPersistenceError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("verification not found: {0}")]
    NotFound(String),
    #[error("unknown verdict value in DB: {0}")]
    UnknownVerdict(String),
}

/// Thin handle over a [`DbPool`] scoped to the `verifications` table.
#[derive(Clone)]
pub struct VerificationStore {
    pool: DbPool,
}

impl VerificationStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, new: NewVerification) -> Result<String, VerifierPersistenceError> {
        let id = Uuid::new_v4().to_string();
        let created_at = now_micros();
        let trigger_kind = new.trigger.kind_str().to_string();
        let trigger_detail = serde_json::to_string(&new.trigger)?;
        let evidence_json = serde_json::to_string(&new.evidence_event_ids)?;
        let suggested_json = match &new.suggested_plan_update {
            Some(v) => Some(serde_json::to_string(v)?),
            None => None,
        };
        let verdict = new.verdict.as_db_str().to_string();

        let id_clone = id.clone();
        let result: Result<usize, rusqlite::Error> = self
            .pool
            .with_conn(move |conn| {
                conn.execute(
                    "INSERT INTO verifications (\
                       id, session_id, triggered_at_event_id, trigger_kind, \
                       trigger_detail, verdict, reason, evidence_event_ids, \
                       suggested_plan_update, model_id, cost_cents, created_at\
                     ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        id_clone,
                        new.session_id,
                        new.triggered_at_event_id,
                        trigger_kind,
                        trigger_detail,
                        verdict,
                        new.reason,
                        evidence_json,
                        suggested_json,
                        new.model_id,
                        new.cost_cents,
                        created_at,
                    ],
                )
            })
            .await;
        result?;
        Ok(id)
    }

    pub async fn get(&self, id: &str) -> Result<Verification, VerifierPersistenceError> {
        let id_owned = id.to_string();
        let row: Option<RawRow> = self
            .pool
            .with_conn(move |conn| -> rusqlite::Result<Option<RawRow>> {
                let mut stmt = conn.prepare(
                    "SELECT id, session_id, triggered_at_event_id, trigger_kind, \
                            trigger_detail, verdict, reason, evidence_event_ids, \
                            suggested_plan_update, model_id, cost_cents, created_at \
                       FROM verifications WHERE id = ?",
                )?;
                let mut rows = stmt.query_map([id_owned], RawRow::from_row)?;
                match rows.next() {
                    Some(row) => Ok(Some(row?)),
                    None => Ok(None),
                }
            })
            .await?;
        match row {
            Some(r) => r.into_verification(),
            None => Err(VerifierPersistenceError::NotFound(id.to_string())),
        }
    }

    /// Newest-first paginated list. `cursor` is the `created_at` of the
    /// last seen row (exclusive); pass `None` for the first page.
    /// Returns up to `limit` rows.
    pub async fn list_by_session(
        &self,
        session_id: &str,
        cursor: Option<i64>,
        limit: usize,
    ) -> Result<Vec<Verification>, VerifierPersistenceError> {
        let session = session_id.to_string();
        let limit = limit.clamp(1, 200) as i64;
        let raws: Vec<RawRow> = self
            .pool
            .with_conn(move |conn| -> rusqlite::Result<Vec<RawRow>> {
                let (sql, has_cursor) = match cursor {
                    Some(_) => (
                        "SELECT id, session_id, triggered_at_event_id, trigger_kind, \
                                trigger_detail, verdict, reason, evidence_event_ids, \
                                suggested_plan_update, model_id, cost_cents, created_at \
                           FROM verifications \
                          WHERE session_id = ? AND created_at < ? \
                          ORDER BY created_at DESC LIMIT ?",
                        true,
                    ),
                    None => (
                        "SELECT id, session_id, triggered_at_event_id, trigger_kind, \
                                trigger_detail, verdict, reason, evidence_event_ids, \
                                suggested_plan_update, model_id, cost_cents, created_at \
                           FROM verifications \
                          WHERE session_id = ? \
                          ORDER BY created_at DESC LIMIT ?",
                        false,
                    ),
                };
                let mut stmt = conn.prepare(sql)?;
                let rows = if has_cursor {
                    stmt.query_map(params![session, cursor.unwrap(), limit], RawRow::from_row)?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                } else {
                    stmt.query_map(params![session, limit], RawRow::from_row)?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                };
                Ok(rows)
            })
            .await?;
        raws.into_iter().map(RawRow::into_verification).collect()
    }
}

struct RawRow {
    id: String,
    session_id: String,
    triggered_at_event_id: i64,
    trigger_kind: String,
    trigger_detail: String,
    verdict: String,
    reason: String,
    evidence_event_ids: String,
    suggested_plan_update: Option<String>,
    model_id: String,
    cost_cents: i64,
    created_at: i64,
}

impl RawRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            session_id: row.get(1)?,
            triggered_at_event_id: row.get(2)?,
            trigger_kind: row.get(3)?,
            trigger_detail: row.get(4)?,
            verdict: row.get(5)?,
            reason: row.get(6)?,
            evidence_event_ids: row.get(7)?,
            suggested_plan_update: row.get(8)?,
            model_id: row.get(9)?,
            cost_cents: row.get(10)?,
            created_at: row.get(11)?,
        })
    }

    fn into_verification(self) -> Result<Verification, VerifierPersistenceError> {
        let verdict = match self.verdict.as_str() {
            "pass" => VerdictKind::Pass,
            "fail" => VerdictKind::Fail,
            other => return Err(VerifierPersistenceError::UnknownVerdict(other.into())),
        };
        let trigger_detail: serde_json::Value = serde_json::from_str(&self.trigger_detail)?;
        let evidence_event_ids: Vec<i64> = serde_json::from_str(&self.evidence_event_ids)?;
        let suggested_plan_update = match self.suggested_plan_update {
            Some(s) => Some(serde_json::from_str::<serde_json::Value>(&s)?),
            None => None,
        };
        Ok(Verification {
            id: self.id,
            session_id: self.session_id,
            triggered_at_event_id: self.triggered_at_event_id,
            trigger_kind: self.trigger_kind,
            trigger_detail,
            verdict,
            reason: self.reason,
            evidence_event_ids,
            suggested_plan_update,
            model_id: self.model_id,
            cost_cents: self.cost_cents,
            created_at: self.created_at,
        })
    }
}

#[cfg(test)]
impl VerificationStore {
    /// Test-only escape hatch: allows tests to pre-seed rows with
    /// controlled `created_at` values for cursor-pagination assertions.
    pub(crate) fn pool_for_test(&self) -> &DbPool {
        &self.pool
    }
}
