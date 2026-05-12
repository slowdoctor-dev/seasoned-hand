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
use seasoned_hand_core::dispatch::{ToolDispatcher, hooks::EventEmittingHook};
use seasoned_hand_core::events::{EventQuery, EventStore, EventType, sqlite::SqliteEventStore};
use seasoned_hand_core::pubsub::RedisPool;
use seasoned_hand_core::router::SlotRouter;
use seasoned_hand_core::sandbox::SandboxClient;
use seasoned_hand_core::search::SearchClient;
use seasoned_hand_core::tools::register_builtin_tools;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct AppState {
    pub db: DbPool,
    pub redis: RedisPool,
    pub events: Arc<SqliteEventStore>,
    pub sandbox: Arc<SandboxClient>,
    pub search: Arc<SearchClient>,
    pub dispatcher: Arc<ToolDispatcher>,
    pub router: Arc<SlotRouter>,
}

impl AppState {
    pub fn new(
        db: DbPool,
        redis: RedisPool,
        sandbox: SandboxClient,
        search: SearchClient,
        router: SlotRouter,
    ) -> Self {
        let events = Arc::new(SqliteEventStore::with_redis(db.clone(), redis.clone()));
        let sandbox = Arc::new(sandbox);
        let search = Arc::new(search);
        let dispatcher = Arc::new(
            ToolDispatcher::new(register_builtin_tools())
                .with_hook(Arc::new(EventEmittingHook::new(events.clone()))),
        );
        Self {
            db,
            redis,
            events,
            sandbox,
            search,
            dispatcher,
            router: Arc::new(router),
        }
    }
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
    db: String,
    redis: String,
}

async fn healthz(State(state): State<AppState>) -> impl IntoResponse {
    let db_ok = state
        .db
        .with_conn(|conn| conn.prepare("SELECT 1").is_ok())
        .await;
    let redis_ok = state.redis.ping().await.is_ok();

    let (status_code, status_text) = if db_ok && redis_ok {
        (StatusCode::OK, "ok")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "degraded")
    };
    (
        status_code,
        Json(Health {
            status: status_text,
            version: seasoned_hand_core::version(),
            db: if db_ok { "ok" } else { "unreachable" }.into(),
            redis: if redis_ok { "ok" } else { "unreachable" }.into(),
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
