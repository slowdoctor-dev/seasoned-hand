//! Issue #7 / ADR-018: verify the Axum auth path is secure by default — legacy
//! `x-seasoned-hand-*` identity headers are rejected unless the insecure flag is
//! on, and an unverified bearer token never authenticates.

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

/// AppState with the DEFAULT (secure) posture — `allow_insecure_headers` is off.
async fn secure_state() -> AppState {
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

fn loopback() -> axum::extract::ConnectInfo<SocketAddr> {
    axum::extract::ConnectInfo("127.0.0.1:4000".parse().unwrap())
}

#[tokio::test]
async fn identity_headers_rejected_when_insecure_flag_off() {
    let state = secure_state().await;
    let request = Request::builder()
        .method("GET")
        .uri("/v1/sessions?limit=1")
        .header("x-seasoned-hand-tenant-id", "tenant-a")
        .header("x-seasoned-hand-organization-id", "org-a")
        .header("x-seasoned-hand-actor-user-id", "u-1")
        .header("x-seasoned-hand-org-role", "admin")
        .extension(loopback())
        .body(Body::empty())
        .expect("request");
    let response = app(state).oneshot(request).await.expect("response");
    // Secure by default: client-asserted headers are not trusted.
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn unverified_bearer_token_is_unauthorized() {
    let state = secure_state().await;
    let request = Request::builder()
        .method("GET")
        .uri("/v1/sessions?limit=1")
        .header("authorization", "Bearer deadbeefdeadbeef")
        .extension(loopback())
        .body(Body::empty())
        .expect("request");
    let response = app(state).oneshot(request).await.expect("response");
    // A presented-but-invalid token is rejected, never demoted to the header path.
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
