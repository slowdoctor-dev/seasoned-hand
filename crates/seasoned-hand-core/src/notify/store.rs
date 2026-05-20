//! `NotificationsSentStore` — V008 `notifications_sent` table persistence.
//!
//! refs: /specs/phase-2/architecture.md §2.7, §3 V008
//! refs: /specs/phase-2/stories/story-2.3.md

use rusqlite::params;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::channel::NotifyTarget;
use crate::db::DbPool;

#[derive(Debug, Clone)]
pub struct NewNotificationSent {
    pub tenant_id: Option<String>,
    /// Nullable — pre-task notifies (briefing escalations) fire before
    /// a task exists.
    pub task_id: Option<String>,
    pub trigger_kind: String,
    pub channel: String,
    pub target: Option<NotifyTarget>,
    pub payload: Option<serde_json::Value>,
    pub ok: bool,
    pub error: Option<String>,
    pub sent_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationSentRow {
    pub id: String,
    pub tenant_id: Option<String>,
    pub task_id: Option<String>,
    pub trigger_kind: String,
    pub channel: String,
    pub target: Option<NotifyTarget>,
    pub payload: Option<serde_json::Value>,
    pub ok: bool,
    pub error: Option<String>,
    pub sent_at: i64,
}

#[derive(Debug, Error)]
pub enum NotifyStoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone)]
pub struct NotificationsSentStore {
    pool: DbPool,
}

impl NotificationsSentStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, new: NewNotificationSent) -> Result<String, NotifyStoreError> {
        let id = Uuid::new_v4().to_string();
        let tenant_id = new
            .tenant_id
            .clone()
            .unwrap_or_else(|| "legacy-default".to_string());
        let target_text = match &new.target {
            Some(t) => Some(serde_json::to_string(t)?),
            None => None,
        };
        let payload_text = match &new.payload {
            Some(p) => Some(serde_json::to_string(p)?),
            None => None,
        };
        let id_clone = id.clone();
        let res: Result<usize, rusqlite::Error> = self
            .pool
            .with_conn(move |conn| {
                conn.execute(
                    "INSERT INTO notifications_sent (\
                       id, tenant_id, task_id, trigger_kind, channel, \
                       target, payload, ok, error, sent_at\
                     ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        id_clone,
                        tenant_id,
                        new.task_id,
                        new.trigger_kind,
                        new.channel,
                        target_text,
                        payload_text,
                        new.ok as i64,
                        new.error,
                        new.sent_at,
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
    ) -> Result<Vec<NotificationSentRow>, NotifyStoreError> {
        let tid = task_id.to_string();
        let raws: Vec<RawRow> = self
            .pool
            .with_conn(move |conn| -> rusqlite::Result<Vec<RawRow>> {
                let mut stmt = conn.prepare(
                    "SELECT id, tenant_id, task_id, trigger_kind, channel, \
                            target, payload, ok, error, sent_at \
                       FROM notifications_sent WHERE task_id = ? \
                      ORDER BY sent_at ASC",
                )?;
                stmt.query_map(params![tid], RawRow::from_row)?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await?;
        raws.into_iter().map(RawRow::into_row).collect()
    }
}

struct RawRow {
    id: String,
    tenant_id: Option<String>,
    task_id: Option<String>,
    trigger_kind: String,
    channel: String,
    target: Option<String>,
    payload: Option<String>,
    ok: i64,
    error: Option<String>,
    sent_at: i64,
}

impl RawRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            tenant_id: row.get(1)?,
            task_id: row.get(2)?,
            trigger_kind: row.get(3)?,
            channel: row.get(4)?,
            target: row.get(5)?,
            payload: row.get(6)?,
            ok: row.get(7)?,
            error: row.get(8)?,
            sent_at: row.get(9)?,
        })
    }

    fn into_row(self) -> Result<NotificationSentRow, NotifyStoreError> {
        let target = match self.target {
            Some(text) => Some(serde_json::from_str::<NotifyTarget>(&text)?),
            None => None,
        };
        let payload = match self.payload {
            Some(text) => Some(serde_json::from_str::<serde_json::Value>(&text)?),
            None => None,
        };
        Ok(NotificationSentRow {
            id: self.id,
            tenant_id: self.tenant_id,
            task_id: self.task_id,
            trigger_kind: self.trigger_kind,
            channel: self.channel,
            target,
            payload,
            ok: self.ok != 0,
            error: self.error,
            sent_at: self.sent_at,
        })
    }
}
