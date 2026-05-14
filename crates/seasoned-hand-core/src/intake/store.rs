//! `IntakeEventStore` — V008 `intake_events` table persistence.
//!
//! refs: /specs/phase-2/architecture.md §2.8, §3 V008
//! refs: /specs/phase-2/stories/story-2.3.md

use rusqlite::params;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::channel::{DeliveryTarget, IntakeEvent};
use crate::db::DbPool;

/// One row of the V008 `intake_events` table. Mirrors the channel-side
/// [`IntakeEvent`] but adds the persisted `id` and the late-bound
/// `task_id` once the IntakeRouter (story 2.5) wires the brief to a
/// task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntakeEventRow {
    pub id: String,
    pub tenant_id: Option<String>,
    pub channel: String,
    pub intake_id: String,
    pub brief_input: String,
    pub reply_target: Option<DeliveryTarget>,
    pub metadata: serde_json::Value,
    pub task_id: Option<String>,
    pub received_at: i64,
}

#[derive(Debug, Error)]
pub enum IntakeStoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("intake event not found: {0}")]
    NotFound(String),
}

#[derive(Clone)]
pub struct IntakeEventStore {
    pool: DbPool,
}

impl IntakeEventStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Insert a new intake event. `(channel, intake_id)` is the
    /// idempotency key — V008's UNIQUE constraint surfaces a duplicate
    /// as [`rusqlite::Error::SqliteFailure`] which the caller (story
    /// 2.5's IntakeRouter) inspects to short-circuit re-deliveries.
    pub async fn insert(&self, event: &IntakeEvent) -> Result<String, IntakeStoreError> {
        let id = Uuid::new_v4().to_string();
        let reply_target_text = match &event.reply_target {
            Some(t) => Some(serde_json::to_string(t)?),
            None => None,
        };
        let metadata_text = serde_json::to_string(&event.metadata)?;
        let id_clone = id.clone();
        let event = event.clone();
        let res: Result<usize, rusqlite::Error> = self
            .pool
            .with_conn(move |conn| {
                conn.execute(
                    "INSERT INTO intake_events (\
                       id, tenant_id, channel, intake_id, brief_input, \
                       reply_target, metadata, task_id, received_at\
                     ) VALUES (?, ?, ?, ?, ?, ?, ?, NULL, ?)",
                    params![
                        id_clone,
                        event.tenant_id,
                        event.channel,
                        event.intake_id,
                        event.brief_input,
                        reply_target_text,
                        metadata_text,
                        event.received_at,
                    ],
                )
            })
            .await;
        res?;
        Ok(id)
    }

    pub async fn get_by_intake_id(
        &self,
        channel: &str,
        intake_id: &str,
    ) -> Result<Option<IntakeEventRow>, IntakeStoreError> {
        let channel = channel.to_string();
        let intake_id = intake_id.to_string();
        let row: Option<RawRow> = self
            .pool
            .with_conn(move |conn| -> rusqlite::Result<Option<RawRow>> {
                let mut stmt = conn.prepare(
                    "SELECT id, tenant_id, channel, intake_id, brief_input, \
                            reply_target, metadata, task_id, received_at \
                       FROM intake_events WHERE channel = ? AND intake_id = ?",
                )?;
                let mut rows = stmt.query_map(params![channel, intake_id], RawRow::from_row)?;
                match rows.next() {
                    Some(row) => Ok(Some(row?)),
                    None => Ok(None),
                }
            })
            .await?;
        row.map(RawRow::into_event).transpose()
    }

    /// Late-bind a previously-inserted intake event to the Task that
    /// the IntakeRouter created from it.
    pub async fn link_to_task(
        &self,
        intake_event_id: &str,
        task_id: &str,
    ) -> Result<(), IntakeStoreError> {
        let id = intake_event_id.to_string();
        let tid = task_id.to_string();
        let affected: usize = self
            .pool
            .with_conn(move |conn| {
                conn.execute(
                    "UPDATE intake_events SET task_id = ? WHERE id = ?",
                    params![tid, id],
                )
            })
            .await?;
        if affected == 0 {
            return Err(IntakeStoreError::NotFound(intake_event_id.to_string()));
        }
        Ok(())
    }

    /// Newest-first paginated list scoped to a channel. `cursor` is the
    /// `received_at` of the last seen row (exclusive upper bound).
    pub async fn list_by_channel(
        &self,
        channel: &str,
        cursor: Option<i64>,
        limit: usize,
    ) -> Result<Vec<IntakeEventRow>, IntakeStoreError> {
        let channel = channel.to_string();
        let limit = limit.clamp(1, 200) as i64;
        let raws: Vec<RawRow> = self
            .pool
            .with_conn(move |conn| -> rusqlite::Result<Vec<RawRow>> {
                let rows = match cursor {
                    Some(c) => {
                        let mut stmt = conn.prepare(
                            "SELECT id, tenant_id, channel, intake_id, brief_input, \
                                    reply_target, metadata, task_id, received_at \
                               FROM intake_events \
                              WHERE channel = ? AND received_at < ? \
                              ORDER BY received_at DESC LIMIT ?",
                        )?;
                        stmt.query_map(params![channel, c, limit], RawRow::from_row)?
                            .collect::<rusqlite::Result<Vec<_>>>()?
                    }
                    None => {
                        let mut stmt = conn.prepare(
                            "SELECT id, tenant_id, channel, intake_id, brief_input, \
                                    reply_target, metadata, task_id, received_at \
                               FROM intake_events \
                              WHERE channel = ? \
                              ORDER BY received_at DESC LIMIT ?",
                        )?;
                        stmt.query_map(params![channel, limit], RawRow::from_row)?
                            .collect::<rusqlite::Result<Vec<_>>>()?
                    }
                };
                Ok(rows)
            })
            .await?;
        raws.into_iter().map(RawRow::into_event).collect()
    }
}

struct RawRow {
    id: String,
    tenant_id: Option<String>,
    channel: String,
    intake_id: String,
    brief_input: String,
    reply_target: Option<String>,
    metadata: String,
    task_id: Option<String>,
    received_at: i64,
}

impl RawRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            tenant_id: row.get(1)?,
            channel: row.get(2)?,
            intake_id: row.get(3)?,
            brief_input: row.get(4)?,
            reply_target: row.get(5)?,
            metadata: row.get(6)?,
            task_id: row.get(7)?,
            received_at: row.get(8)?,
        })
    }

    fn into_event(self) -> Result<IntakeEventRow, IntakeStoreError> {
        let reply_target = match self.reply_target {
            Some(text) => Some(serde_json::from_str::<DeliveryTarget>(&text)?),
            None => None,
        };
        let metadata = serde_json::from_str::<serde_json::Value>(&self.metadata)?;
        Ok(IntakeEventRow {
            id: self.id,
            tenant_id: self.tenant_id,
            channel: self.channel,
            intake_id: self.intake_id,
            brief_input: self.brief_input,
            reply_target,
            metadata,
            task_id: self.task_id,
            received_at: self.received_at,
        })
    }
}
