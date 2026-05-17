//! SQLite-backed implementation of [`EventStore`].
//! refs: /specs/phase-0/architecture.md §3.2

use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::OptionalExtension;

use super::{Event, EventError, EventQuery, EventStore, EventType, NewEvent};
use crate::db::DbPool;
use crate::pubsub::RedisPool;
use crate::events::session_search;

pub struct SqliteEventStore {
    pool: DbPool,
    redis: Option<RedisPool>,
}

impl SqliteEventStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool, redis: None }
    }

    pub fn with_redis(pool: DbPool, redis: RedisPool) -> Self {
        Self {
            pool,
            redis: Some(redis),
        }
    }

    pub async fn reserve_next_id(&self) -> Result<i64, EventError> {
        self.pool
            .with_conn(|conn| {
                conn.query_row("SELECT COALESCE(MAX(id), 0) + 1 FROM events", [], |row| {
                    row.get(0)
                })
                .map_err(EventError::from)
            })
            .await
    }

    pub async fn with_conn<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut rusqlite::Connection) -> R + Send,
        R: Send,
    {
        self.pool.with_conn(f).await
    }
}

fn now_micros() -> Result<i64, EventError> {
    let micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| EventError::Clock(e.to_string()))?
        .as_micros();
    i64::try_from(micros).map_err(|e| EventError::Clock(e.to_string()))
}

impl EventStore for SqliteEventStore {
    async fn append(&self, draft: NewEvent) -> Result<Event, EventError> {
        let timestamp = now_micros()?;
        let data_text = serde_json::to_string(&draft.data)?;
        let session_id = draft.session_id.clone();
        let event_type = draft.event_type;
        let source = draft.source.clone();
        let type_str = draft.event_type.as_str();

        let event = self
            .pool
            .with_conn(move |conn| -> Result<Event, EventError> {
                let exists: Option<i64> = conn
                    .query_row(
                        "SELECT 1 FROM sessions WHERE id = ?",
                        [&session_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                if exists.is_none() {
                    return Err(EventError::SessionNotFound(session_id.clone()));
                }

                let id: i64 = conn.query_row(
                    "INSERT INTO events (session_id, timestamp, type, source, data) \
                     VALUES (?, ?, ?, ?, ?) RETURNING id",
                    rusqlite::params![&session_id, timestamp, type_str, &source, &data_text],
                    |row| row.get(0),
                )?;
                let event = Event {
                    id,
                    session_id,
                    timestamp,
                    event_type,
                    source,
                    data: draft.data,
                };
                session_search::index_event_for_search(conn, &event)?;
                Ok(event)
            })
            .await?;

        if let Some(redis) = &self.redis {
            match serde_json::to_string(&event) {
                Ok(payload) => {
                    if let Err(e) = redis.publish_event(&event.session_id, &payload).await {
                        tracing::warn!(
                            error = %e,
                            session_id = %event.session_id,
                            event_id = event.id,
                            "redis publish failed; append succeeded"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        event_id = event.id,
                        "failed to serialize event for redis publish; append succeeded"
                    );
                }
            }
        }

        Ok(event)
    }

    async fn query(&self, session_id: &str, filter: EventQuery) -> Result<Vec<Event>, EventError> {
        let session_id = session_id.to_string();
        let limit = filter.effective_limit() as i64;
        let after_id = filter.after_id;
        let type_filter = filter.event_type.map(|t| t.as_str().to_string());

        self.pool
            .with_conn(move |conn| -> Result<Vec<Event>, EventError> {
                let mut sql = String::from(
                    "SELECT id, session_id, timestamp, type, source, data \
                     FROM events WHERE session_id = ?",
                );
                if after_id.is_some() {
                    sql.push_str(" AND id > ?");
                }
                if type_filter.is_some() {
                    sql.push_str(" AND type = ?");
                }
                sql.push_str(" ORDER BY id ASC LIMIT ?");

                let mut stmt = conn.prepare(&sql)?;
                let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(session_id.clone())];
                if let Some(a) = after_id {
                    params.push(Box::new(a));
                }
                if let Some(t) = &type_filter {
                    params.push(Box::new(t.clone()));
                }
                params.push(Box::new(limit));

                let refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

                let rows = stmt.query_map(refs.as_slice(), |row| {
                    let type_str: String = row.get(3)?;
                    let data_str: String = row.get(5)?;
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        type_str,
                        row.get::<_, String>(4)?,
                        data_str,
                    ))
                })?;

                let mut events = Vec::new();
                for row in rows {
                    let (id, session_id, timestamp, type_str, source, data_str) = row?;
                    let event_type = EventType::from_str(&type_str)?;
                    let data: serde_json::Value = serde_json::from_str(&data_str)?;
                    events.push(Event {
                        id,
                        session_id,
                        timestamp,
                        event_type,
                        source,
                        data,
                    });
                }
                Ok(events)
            })
            .await
    }
}
