//! refs: /specs/phase-1/stories/story-1.4.md

use axum::body::Body;
use axum::http::{Request, StatusCode};
use seasoned_hand_core::router::SlotRouter;
use seasoned_hand_core::sandbox::{SandboxClient, SandboxHandle};
use seasoned_hand_core::search::{SearchClient, SearchProvider};
use seasoned_hand_core::{db, pubsub};
use seasoned_hand_server::{AppState, app};
use tower::ServiceExt;

#[tokio::test]
async fn http_feature_list_route_returns_json() {
    let pool = db::open(":memory:").await.expect("open db");
    let redis = pubsub::RedisPool::new("redis://127.0.0.1:6").expect("redis pool");
    let tmp_root = tempfile::tempdir().expect("tmp root");
    let sandbox = SandboxClient::new("ghcr.io/agent-infra/sandbox:1.0.0.152", tmp_root.path())
        .expect("sandbox client");
    let ws = tempfile::tempdir().expect("workspace");
    sandbox
        .insert_handle_for_test(SandboxHandle {
            session_id: "s1".into(),
            container_id: "c1".into(),
            api_url: "http://127.0.0.1:1".into(),
            novnc_url: "http://127.0.0.1:2".into(),
            ttyd_url: "ws://127.0.0.1:3".into(),
            workspace_host_path: ws.path().join("s1"),
        })
        .await;
    sandbox
        .write_workspace_file(
            "s1",
            "feature-list.json",
            br#"{"version":1,"goal":"g","features":[]}"#,
        )
        .await
        .expect("write feature-list");

    let search = SearchClient::new(SearchProvider::Brave { api_key: None });
    let router = SlotRouter::default_for_bifrost();
    let state = AppState::new(pool, redis, sandbox, search, router, Default::default());

    let resp = app(state)
        .oneshot(
            Request::builder()
                .uri("/v1/sessions/s1/feature-list")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("feature-list request");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(body["version"], 1);
}

#[tokio::test]
async fn http_progress_route_returns_tail_with_default_lines() {
    let pool = db::open(":memory:").await.expect("open db");
    let redis = pubsub::RedisPool::new("redis://127.0.0.1:6").expect("redis pool");
    let tmp_root = tempfile::tempdir().expect("tmp root");
    let sandbox = SandboxClient::new("ghcr.io/agent-infra/sandbox:1.0.0.152", tmp_root.path())
        .expect("sandbox client");
    let ws = tempfile::tempdir().expect("workspace");
    sandbox
        .insert_handle_for_test(SandboxHandle {
            session_id: "s1".into(),
            container_id: "c1".into(),
            api_url: "http://127.0.0.1:1".into(),
            novnc_url: "http://127.0.0.1:2".into(),
            ttyd_url: "ws://127.0.0.1:3".into(),
            workspace_host_path: ws.path().join("s1"),
        })
        .await;
    sandbox
        .write_workspace_file("s1", "progress.txt", b"line-a\nline-b\nline-c\n")
        .await
        .expect("write progress");

    let search = SearchClient::new(SearchProvider::Brave { api_key: None });
    let router = SlotRouter::default_for_bifrost();
    let state = AppState::new(pool, redis, sandbox, search, router, Default::default());

    let resp = app(state)
        .oneshot(
            Request::builder()
                .uri("/v1/sessions/s1/progress")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("progress request");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let body = String::from_utf8_lossy(&body);
    assert!(body.contains("line-c"));
}
