//! refs: /specs/phase-1/stories/story-1.4.md
//!
//! Routes here are loopback-gated by `require_loopback` (DEBT #59 close in
//! `/specs/REVIEW.md`), so the test serves the app via a real `TcpListener`
//! with `into_make_service_with_connect_info` and hits it with `reqwest`
//! rather than `tower::ServiceExt::oneshot`. The bound peer addr resolves
//! to `127.0.0.1`, which satisfies the gate.

use axum::http::StatusCode;
use seasoned_hand_core::router::SlotRouter;
use seasoned_hand_core::sandbox::{SandboxClient, SandboxHandle};
use seasoned_hand_core::search::{SearchClient, SearchProvider};
use seasoned_hand_core::{db, pubsub};
use seasoned_hand_server::{AppState, app};
use tokio::net::TcpListener;

async fn boot_with_workspace(file: &str, contents: &[u8]) -> String {
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
        .write_workspace_file("s1", file, contents)
        .await
        .expect("write workspace file");
    // Keep the tempdirs alive for the test duration; dropping them would
    // tear the workspace down before the spawned axum task reads it.
    Box::leak(Box::new(tmp_root));
    Box::leak(Box::new(ws));

    let search = SearchClient::new(SearchProvider::Brave { api_key: None });
    let router = SlotRouter::default_for_bifrost();
    let state = AppState::new(pool, redis, sandbox, search, router, Default::default());

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(
            listener,
            app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .expect("serve");
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn http_feature_list_route_returns_json() {
    let base = boot_with_workspace(
        "feature-list.json",
        br#"{"version":1,"goal":"g","features":[]}"#,
    )
    .await;

    let resp = reqwest::get(format!("{base}/v1/sessions/s1/feature-list"))
        .await
        .expect("feature-list request");
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["version"], 1);
}

#[tokio::test]
async fn http_progress_route_returns_tail_with_default_lines() {
    let base = boot_with_workspace("progress.txt", b"line-a\nline-b\nline-c\n").await;

    let resp = reqwest::get(format!("{base}/v1/sessions/s1/progress"))
        .await
        .expect("progress request");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.text().await.expect("body");
    assert!(body.contains("line-c"));
}
