//! Seasoned Hand HTTP server.
//! refs: /specs/phase-0/architecture.md §4.1

use axum::{Json, Router, routing::get};
use serde::Serialize;

#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
}

async fn healthz() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: seasoned_hand_core::version(),
    })
}

pub fn app() -> Router {
    Router::new().route("/healthz", get(healthz))
}
