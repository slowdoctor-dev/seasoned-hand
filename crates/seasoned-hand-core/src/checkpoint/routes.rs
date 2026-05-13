//! Checkpoint HTTP read routes.
//! refs: /specs/phase-1/stories/story-1.13.md

use serde::{Deserialize, Serialize};

use super::persistence::{Checkpoint, CheckpointPersistenceError, CheckpointStore};
pub use crate::routes::RouteOutcome;

#[derive(Debug, Deserialize, Default)]
pub struct ListQuery {
    pub cursor: Option<i64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct ListResponse {
    pub rows: Vec<Checkpoint>,
    pub next_cursor: Option<i64>,
}

pub async fn list_checkpoints(
    store: &CheckpointStore,
    session_id: &str,
    q: ListQuery,
) -> RouteOutcome<ListResponse> {
    let limit = q.limit.unwrap_or(50);
    let rows = match store.list_by_session(session_id, q.cursor, limit).await {
        Ok(v) => v,
        Err(CheckpointPersistenceError::NotFound(_)) => Vec::new(),
        Err(e) => return RouteOutcome::Internal(e.to_string()),
    };
    let next_cursor = rows.last().map(|r| r.created_at);
    RouteOutcome::Ok(ListResponse { rows, next_cursor })
}
