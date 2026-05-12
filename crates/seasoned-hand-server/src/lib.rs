//! Seasoned Hand HTTP server.
//! refs: /specs/phase-0/architecture.md §4.1

use std::str::FromStr;
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use seasoned_hand_core::db::DbPool;
use seasoned_hand_core::events::{EventQuery, EventStore, EventType, sqlite::SqliteEventStore};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct AppState {
    pub db: DbPool,
    pub events: Arc<SqliteEventStore>,
}

impl AppState {
    pub fn new(db: DbPool) -> Self {
        let events = Arc::new(SqliteEventStore::new(db.clone()));
        Self { db, events }
    }
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
    db: String,
}

async fn healthz(State(state): State<AppState>) -> impl IntoResponse {
    let db_ok = state
        .db
        .with_conn(|conn| conn.prepare("SELECT 1").is_ok())
        .await;
    let (status_code, status_text, db_text) = if db_ok {
        (StatusCode::OK, "ok", "ok".to_string())
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "degraded",
            "unreachable".to_string(),
        )
    };
    (
        status_code,
        Json(Health {
            status: status_text,
            version: seasoned_hand_core::version(),
            db: db_text,
        }),
    )
}

#[derive(Debug, Deserialize, Default)]
pub struct EventsQueryParams {
    pub after_id: Option<i64>,
    #[serde(rename = "type")]
    pub event_type: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Serialize)]
struct ApiError {
    error: String,
}

async fn list_events(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(params): Query<EventsQueryParams>,
) -> Result<Json<Vec<seasoned_hand_core::events::Event>>, (StatusCode, Json<ApiError>)> {
    let event_type = match params.event_type.as_deref() {
        Some(s) => Some(EventType::from_str(s).map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiError {
                    error: format!("unknown event type: {s}"),
                }),
            )
        })?),
        None => None,
    };

    let filter = EventQuery {
        after_id: params.after_id,
        event_type,
        limit: params.limit,
    };

    let session_exists = state
        .db
        .with_conn({
            let session_id = session_id.clone();
            move |conn| {
                conn.query_row::<i64, _, _>(
                    "SELECT 1 FROM sessions WHERE id = ?",
                    [&session_id],
                    |row| row.get(0),
                )
                .is_ok()
            }
        })
        .await;
    if !session_exists {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: "session_not_found".into(),
            }),
        ));
    }

    match state.events.query(&session_id, filter).await {
        Ok(events) => Ok(Json(events)),
        Err(seasoned_hand_core::events::EventError::SessionNotFound(_)) => Err((
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: "session_not_found".into(),
            }),
        )),
        Err(other) => {
            tracing::error!(error = %other, "events query failed");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: "internal_error".into(),
                }),
            ))
        }
    }
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/sessions/:id/events", get(list_events))
        .with_state(state)
}
