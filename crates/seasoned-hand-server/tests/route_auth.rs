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

/// Issue #21 regression guard: every route registered in `app()` must be wrapped
/// by exactly one classifier — `with_auth` (protected), `public`, or
/// `self_gated`. Since those wrappers are the only way auth is attached (or
/// explicitly waived), a bare `.route(path, handler)` would silently skip auth —
/// the original #21 fail-open. This audits the source of `app()` and fails CI if
/// any route registration lacks a classifier, so the markers can't drift into
/// being decorative.
#[test]
fn every_route_in_app_is_explicitly_classified() {
    let src = include_str!("../src/lib.rs");
    let app_start = src.find("pub fn app(").expect("app() fn present");
    let body = &src[app_start..];
    let app_end = body
        .find(".with_state(state)")
        .expect("app() ends with .with_state(state)");
    let app_body = &body[..app_end];

    let mut idx = 0;
    let mut routes = 0;
    while let Some(rel) = app_body[idx..].find(".route(") {
        let span_start = idx + rel + ".route(".len();
        // Each route registration spans until the next `.route(` (or app() end).
        let span_end = app_body[span_start..]
            .find(".route(")
            .map(|n| span_start + n)
            .unwrap_or(app_body.len());
        let span = &app_body[span_start..span_end];
        assert!(
            span.contains("with_auth(") || span.contains("public(") || span.contains("self_gated("),
            "unclassified route (add with_auth/public/self_gated): {:?}",
            span.lines().next().unwrap_or(span).trim()
        );
        routes += 1;
        idx = span_end;
    }
    assert!(
        routes >= 20,
        "expected to scan the full route table; only found {routes} routes"
    );
}
