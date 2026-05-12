//! Seasoned Hand HTTP server.
//! refs: /specs/phase-0/architecture.md §4.1

use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::get};
use seasoned_hand_core::db::DbPool;
use serde::Serialize;

#[derive(Clone)]
pub struct AppState {
    pub db: DbPool,
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

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .with_state(state)
}
