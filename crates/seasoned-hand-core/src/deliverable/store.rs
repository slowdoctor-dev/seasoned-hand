//! `DeliverableStore` — V007 `deliverables` table persistence.
//!
//! refs: /specs/phase-2/architecture.md §2.3, §2.11, §3 V007
//! refs: /specs/phase-2/stories/story-2.3.md

use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::params;
use thiserror::Error;
use uuid::Uuid;

use super::Deliverable;
use crate::db::DbPool;

#[derive(Debug, Clone)]
pub struct NewDeliverable {
    pub task_id: String,
    pub tenant_id: Option<String>,
    pub format: String,
    pub source_content_path: Option<String>,
    pub source_content_sha256: Option<String>,
    pub rendered_content_path: String,
    pub rendered_content_sha256: String,
    pub content_size: i64,
    pub citations: Option<Vec<i64>>,
    pub provenance_manifest: serde_json::Value,
}

#[derive(Debug, Error)]
pub enum DeliverableError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("deliverable not found: {0}")]
    NotFound(String),
}

#[derive(Clone)]
pub struct DeliverableStore {
    pool: DbPool,
}

impl DeliverableStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, new: NewDeliverable) -> Result<String, DeliverableError> {
        let id = Uuid::new_v4().to_string();
        let now = now_micros();
        let manifest_text = serde_json::to_string(&new.provenance_manifest)?;
        let citations_text = match &new.citations {
            Some(c) => Some(serde_json::to_string(c)?),
            None => None,
        };
        let id_clone = id.clone();
        let res: Result<usize, rusqlite::Error> = self
            .pool
            .with_conn(move |conn| {
                conn.execute(
                    "INSERT INTO deliverables (\
                       id, task_id, tenant_id, format, \
                       source_content_path, source_content_sha256, \
                       rendered_content_path, rendered_content_sha256, \
                       content_size, citations, provenance_manifest, created_at\
                     ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        id_clone,
                        new.task_id,
                        new.tenant_id,
                        new.format,
                        new.source_content_path,
                        new.source_content_sha256,
                        new.rendered_content_path,
                        new.rendered_content_sha256,
                        new.content_size,
                        citations_text,
                        manifest_text,
                        now,
                    ],
                )
            })
            .await;
        res?;
        Ok(id)
    }

    pub async fn get(&self, id: &str) -> Result<Deliverable, DeliverableError> {
        let id_owned = id.to_string();
        let row: Option<RawRow> = self
            .pool
            .with_conn(move |conn| -> rusqlite::Result<Option<RawRow>> {
                let mut stmt = conn.prepare(SELECT_COLUMNS_WHERE_ID)?;
                let mut rows = stmt.query_map(params![id_owned], RawRow::from_row)?;
                match rows.next() {
                    Some(row) => Ok(Some(row?)),
                    None => Ok(None),
                }
            })
            .await?;
        match row {
            Some(r) => r.into_deliverable(),
            None => Err(DeliverableError::NotFound(id.to_string())),
        }
    }

    pub async fn list_by_task(&self, task_id: &str) -> Result<Vec<Deliverable>, DeliverableError> {
        let tid = task_id.to_string();
        let raws: Vec<RawRow> = self
            .pool
            .with_conn(move |conn| -> rusqlite::Result<Vec<RawRow>> {
                let mut stmt = conn.prepare(SELECT_COLUMNS_BY_TASK)?;
                stmt.query_map(params![tid], RawRow::from_row)?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await?;
        raws.into_iter().map(RawRow::into_deliverable).collect()
    }

    /// Replace the `provenance_manifest` column. Story 2.15 (manifest
    /// builder) calls this once the manifest is composed; this store
    /// stays format-agnostic — it just writes JSON TEXT.
    pub async fn attach_provenance(
        &self,
        id: &str,
        manifest: &serde_json::Value,
    ) -> Result<(), DeliverableError> {
        let manifest_text = serde_json::to_string(manifest)?;
        let id_owned = id.to_string();
        let affected: usize = self
            .pool
            .with_conn(move |conn| {
                conn.execute(
                    "UPDATE deliverables SET provenance_manifest = ? WHERE id = ?",
                    params![manifest_text, id_owned],
                )
            })
            .await?;
        if affected == 0 {
            return Err(DeliverableError::NotFound(id.to_string()));
        }
        Ok(())
    }

    /// Confirm the deliverable row exists; returns `NotFound` if it
    /// doesn't. V007 has no `delivered_at` column — the audit trail
    /// lives in the V008 `delivery_events` table — so this method is
    /// the existence guard the DeliveryRouter (story 2.5) calls before
    /// it appends a delivery event. Provenance-side `delivered_to[]`
    /// updates happen via [`Self::attach_provenance`] from story 2.15.
    pub async fn mark_delivered(&self, id: &str) -> Result<(), DeliverableError> {
        let id_owned = id.to_string();
        let exists: bool = self
            .pool
            .with_conn(move |conn| {
                conn.query_row(
                    "SELECT 1 FROM deliverables WHERE id = ?",
                    params![id_owned],
                    |_| Ok(true),
                )
                .or_else(|err| match err {
                    rusqlite::Error::QueryReturnedNoRows => Ok(false),
                    other => Err(other),
                })
            })
            .await?;
        if !exists {
            return Err(DeliverableError::NotFound(id.to_string()));
        }
        Ok(())
    }
}

const SELECT_COLUMNS_WHERE_ID: &str = "SELECT id, task_id, tenant_id, format, source_content_path, source_content_sha256, \
            rendered_content_path, rendered_content_sha256, content_size, citations, \
            provenance_manifest, created_at \
       FROM deliverables WHERE id = ?";

const SELECT_COLUMNS_BY_TASK: &str = "SELECT id, task_id, tenant_id, format, source_content_path, source_content_sha256, \
            rendered_content_path, rendered_content_sha256, content_size, citations, \
            provenance_manifest, created_at \
       FROM deliverables WHERE task_id = ? ORDER BY created_at ASC";

struct RawRow {
    id: String,
    task_id: String,
    tenant_id: Option<String>,
    format: String,
    source_content_path: Option<String>,
    source_content_sha256: Option<String>,
    rendered_content_path: String,
    rendered_content_sha256: String,
    content_size: i64,
    citations: Option<String>,
    provenance_manifest: String,
    created_at: i64,
}

impl RawRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            task_id: row.get(1)?,
            tenant_id: row.get(2)?,
            format: row.get(3)?,
            source_content_path: row.get(4)?,
            source_content_sha256: row.get(5)?,
            rendered_content_path: row.get(6)?,
            rendered_content_sha256: row.get(7)?,
            content_size: row.get(8)?,
            citations: row.get(9)?,
            provenance_manifest: row.get(10)?,
            created_at: row.get(11)?,
        })
    }

    fn into_deliverable(self) -> Result<Deliverable, DeliverableError> {
        let citations = match self.citations {
            Some(text) => Some(serde_json::from_str::<Vec<i64>>(&text)?),
            None => None,
        };
        let provenance_manifest =
            serde_json::from_str::<serde_json::Value>(&self.provenance_manifest)?;
        Ok(Deliverable {
            id: self.id,
            task_id: self.task_id,
            tenant_id: self.tenant_id,
            format: self.format,
            source_content_path: self.source_content_path,
            source_content_sha256: self.source_content_sha256,
            rendered_content_path: self.rendered_content_path,
            rendered_content_sha256: self.rendered_content_sha256,
            content_size: self.content_size,
            citations,
            provenance_manifest,
            created_at: self.created_at,
        })
    }
}

fn now_micros() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0)
}
