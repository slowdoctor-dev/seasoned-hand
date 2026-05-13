//! Verifier HTTP routes — read-only.
//!
//! - `GET /v1/sessions/:id/verifications?cursor=&limit=` — newest-first
//!   paginated list.
//! - `GET /v1/verifications/:id` — one row.
//!
//! refs: /specs/phase-1/architecture.md §4.1
//! refs: /specs/phase-1/stories/story-1.9.md

use serde::{Deserialize, Serialize};

use super::{Verification, VerificationStore, VerifierPersistenceError};
pub use crate::routes::RouteOutcome;

#[derive(Debug, Deserialize, Default)]
pub struct ListQuery {
    pub cursor: Option<i64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct ListResponse {
    pub rows: Vec<Verification>,
    pub next_cursor: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct RouteError {
    pub error: String,
}

pub async fn list_verifications(
    store: &VerificationStore,
    session_id: &str,
    query: ListQuery,
) -> RouteOutcome<ListResponse> {
    let limit = query.limit.unwrap_or(50);
    let rows = match store.list_by_session(session_id, query.cursor, limit).await {
        Ok(v) => v,
        Err(e) => return RouteOutcome::Internal(e.to_string()),
    };
    let next_cursor = rows.last().map(|r| r.created_at);
    RouteOutcome::Ok(ListResponse { rows, next_cursor })
}

pub async fn get_verification(store: &VerificationStore, id: &str) -> RouteOutcome<Verification> {
    match store.get(id).await {
        Ok(v) => RouteOutcome::Ok(v),
        Err(VerifierPersistenceError::NotFound(_)) => {
            RouteOutcome::NotFound("verification_not_found".into())
        }
        Err(e) => RouteOutcome::Internal(e.to_string()),
    }
}
