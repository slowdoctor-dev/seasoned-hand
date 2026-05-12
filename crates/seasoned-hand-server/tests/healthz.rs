//! refs: /specs/phase-0/architecture.md §4.1

use axum::http::StatusCode;
use seasoned_hand_core::{db, pubsub};
use seasoned_hand_server::{AppState, app};
use tokio::net::TcpListener;

#[tokio::test]
async fn healthz_returns_ok_with_db_only() {
    // No Redis required; expect degraded when Redis is unreachable, but
    // db field still reports ok.
    let pool = db::open(":memory:").await.expect("open in-memory db");
    // Use an unreachable Redis port for deterministic offline behavior.
    let redis = pubsub::RedisPool::new("redis://127.0.0.1:6").expect("build pool");
    let state = AppState::new(pool, redis);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral test port");
    let addr = listener.local_addr().expect("read local test address");

    tokio::spawn(async move {
        axum::serve(listener, app(state))
            .await
            .expect("serve healthz test app");
    });

    let resp = reqwest::get(format!("http://{addr}/healthz"))
        .await
        .expect("GET /healthz");

    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: serde_json::Value = resp.json().await.expect("parse healthz JSON");
    assert_eq!(body["status"], "degraded");
    assert_eq!(body["db"], "ok");
    assert_eq!(body["redis"], "unreachable");
    assert_eq!(body["version"], seasoned_hand_core::version());
}
