//! `DeliveryEventStore` — V008 `delivery_events` table persistence.
//!
//! refs: /specs/phase-2/architecture.md §2.9, §3 V008
//! refs: /specs/phase-2/stories/story-2.3.md

use rusqlite::params;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::channel::DeliveryTarget;
use crate::db::DbPool;

#[derive(Debug, Clone)]
pub struct NewDeliveryEvent {
    pub tenant_id: Option<String>,
    pub task_id: String,
    pub deliverable_id: String,
    pub channel: String,
    pub target: DeliveryTarget,
    pub ok: bool,
    pub external_id: Option<String>,
    pub error: Option<String>,
    pub delivered_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryEventRow {
    pub id: String,
    pub tenant_id: Option<String>,
    pub task_id: String,
    pub deliverable_id: String,
    pub channel: String,
    pub target: DeliveryTarget,
    pub ok: bool,
    pub external_id: Option<String>,
    pub error: Option<String>,
    pub delivered_at: i64,
}

#[derive(Debug, Error)]
pub enum DeliveryStoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone)]
pub struct DeliveryEventStore {
    pool: DbPool,
}

impl DeliveryEventStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, new: NewDeliveryEvent) -> Result<String, DeliveryStoreError> {
        let id = Uuid::new_v4().to_string();
        let target_text = serde_json::to_string(&new.target)?;
        let id_clone = id.clone();
        let res: Result<usize, rusqlite::Error> = self
            .pool
            .with_conn(move |conn| {
                conn.execute(
                    "INSERT INTO delivery_events (\
                       id, tenant_id, task_id, deliverable_id, channel, \
                       target, ok, external_id, error, delivered_at\
                     ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        id_clone,
                        new.tenant_id,
                        new.task_id,
                        new.deliverable_id,
                        new.channel,
                        target_text,
                        new.ok as i64,
                        new.external_id,
                        new.error,
                        new.delivered_at,
                    ],
                )
            })
            .await;
        res?;
        Ok(id)
    }

    pub async fn list_by_task(
        &self,
        task_id: &str,
    ) -> Result<Vec<DeliveryEventRow>, DeliveryStoreError> {
        let tid = task_id.to_string();
        let raws: Vec<RawRow> = self
            .pool
            .with_conn(move |conn| -> rusqlite::Result<Vec<RawRow>> {
                let mut stmt = conn.prepare(
                    "SELECT id, tenant_id, task_id, deliverable_id, channel, \
                            target, ok, external_id, error, delivered_at \
                       FROM delivery_events WHERE task_id = ? \
                      ORDER BY delivered_at ASC",
                )?;
                stmt.query_map(params![tid], RawRow::from_row)?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await?;
        raws.into_iter().map(RawRow::into_event).collect()
    }

    pub async fn list_by_deliverable(
        &self,
        deliverable_id: &str,
    ) -> Result<Vec<DeliveryEventRow>, DeliveryStoreError> {
        let did = deliverable_id.to_string();
        let raws: Vec<RawRow> = self
            .pool
            .with_conn(move |conn| -> rusqlite::Result<Vec<RawRow>> {
                let mut stmt = conn.prepare(
                    "SELECT id, tenant_id, task_id, deliverable_id, channel, \
                            target, ok, external_id, error, delivered_at \
                       FROM delivery_events WHERE deliverable_id = ? \
                      ORDER BY delivered_at ASC",
                )?;
                stmt.query_map(params![did], RawRow::from_row)?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await?;
        raws.into_iter().map(RawRow::into_event).collect()
    }
}

struct RawRow {
    id: String,
    tenant_id: Option<String>,
    task_id: String,
    deliverable_id: String,
    channel: String,
    target: String,
    ok: i64,
    external_id: Option<String>,
    error: Option<String>,
    delivered_at: i64,
}

impl RawRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            tenant_id: row.get(1)?,
            task_id: row.get(2)?,
            deliverable_id: row.get(3)?,
            channel: row.get(4)?,
            target: row.get(5)?,
            ok: row.get(6)?,
            external_id: row.get(7)?,
            error: row.get(8)?,
            delivered_at: row.get(9)?,
        })
    }

    fn into_event(self) -> Result<DeliveryEventRow, DeliveryStoreError> {
        let target = serde_json::from_str::<DeliveryTarget>(&self.target)?;
        Ok(DeliveryEventRow {
            id: self.id,
            tenant_id: self.tenant_id,
            task_id: self.task_id,
            deliverable_id: self.deliverable_id,
            channel: self.channel,
            target,
            ok: self.ok != 0,
            external_id: self.external_id,
            error: self.error,
            delivered_at: self.delivered_at,
        })
    }
}
