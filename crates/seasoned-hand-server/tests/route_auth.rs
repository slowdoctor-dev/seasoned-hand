//! Issue #21: route auth classification — public routes are reachable without
//! credentials, and a self-gated route enforces its own (loopback) guard. The
//! "protected route → 401 without a session" case is covered in `auth_session.rs`.

use std::net::SocketAddr;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use seasoned_hand_core::db;
use seasoned_hand_core::pubsub;
use seasoned_hand_core::router::SlotRouter;
use seasoned_hand_core::sandbox::SandboxClient;
use seasoned_hand_core::search::{SearchClient, SearchProvider};
use seasoned_hand_server::{AppState, app};
use tower::ServiceExt;

async fn state() -> AppState {
    let pool = db::open(":memory:").await.expect("db");
    let redis = pubsub::RedisPool::new("redis://127.0.0.1:6").expect("redis");
    let sandbox = SandboxClient::new(
        "ghcr.io/agent-infra/sandbox:1.0.0.152",
        std::env::temp_dir(),
    )
    .expect("sandbox");
    let search = SearchClient::new(SearchProvider::Brave { api_key: None });
    let router = SlotRouter::default_for_bifrost();
    AppState::new(pool, redis, sandbox, search, router, Default::default())
}

fn conn(addr: &str) -> axum::extract::ConnectInfo<SocketAddr> {
    axum::extract::ConnectInfo(addr.parse().unwrap())
}

#[tokio::test]
async fn public_healthz_is_reachable_without_credentials() {
    let response = app(state().await)
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .extension(conn("127.0.0.1:5000"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("response");
    // Public route: auth must not block it (200 healthy, or 503 degraded with
    // Redis down) — never 401/403.
    assert!(
        response.status() == StatusCode::OK || response.status() == StatusCode::SERVICE_UNAVAILABLE,
        "unexpected status {}",
        response.status()
    );
}

#[tokio::test]
async fn self_gated_cost_rejects_non_loopback() {
    let response = app(state().await)
        .oneshot(
            Request::builder()
                .uri("/v1/cost")
                .extension(conn("8.8.8.8:443"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("response");
    // #21: /v1/cost is self-gated on loopback; a non-loopback caller is forbidden.
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
