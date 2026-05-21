//! refs: /specs/phase-1/stories/story-1.4.md
//!
//! Routes here are loopback-gated by `require_loopback` (DEBT #59 close in
//! `/specs/REVIEW.md`), so the test serves the app via a real `TcpListener`
//! with `into_make_service_with_connect_info` and hits it with `reqwest`
//! rather than `tower::ServiceExt::oneshot`. The bound peer addr resolves
//! to `127.0.0.1`, which satisfies the gate.

use axum::http::StatusCode;
use seasoned_hand_core::project::{NewProject, NewTask, ProjectStore, TaskStore};
use seasoned_hand_core::router::SlotRouter;
use seasoned_hand_core::sandbox::{SandboxClient, SandboxHandle};
use seasoned_hand_core::search::{SearchClient, SearchProvider};
use seasoned_hand_core::{db, pubsub};
use seasoned_hand_server::{AppState, app};
use tokio::net::TcpListener;

async fn boot_with_workspace(file: &str, contents: &[u8]) -> String {
    let pool = db::open(":memory:").await.expect("open db");
    let projects = ProjectStore::new(pool.clone());
    let tasks = TaskStore::new(pool.clone());
    let project_id = projects
        .insert(NewProject {
            tenant_id: Some("legacy-default".to_string()),
            title: "Feature List Project".to_string(),
            description: None,
        })
        .await
        .expect("insert project");
    let task_id = tasks
        .insert(NewTask {
            project_id: project_id.clone(),
            tenant_id: Some("legacy-default".to_string()),
            title: "Feature List Task".to_string(),
            expected_due_at: None,
        })
        .await
        .expect("insert task");
    pool.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO sessions (
                id, task_id, project_id, created_at, updated_at, state, title, cost_cents, tool_calls
            ) VALUES ('s1', ?, ?, 0, 0, 'SUSPENDED', 'session-s1', 0, 0)",
            rusqlite::params![task_id, project_id],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .expect("insert session");

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

fn with_auth_headers(req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    req.header("x-seasoned-hand-tenant-id", "legacy-default")
        .header("x-seasoned-hand-organization-id", "org-legacy-default")
        .header("x-seasoned-hand-actor-user-id", "user-test")
        .header("x-seasoned-hand-org-role", "admin")
}

#[tokio::test]
async fn http_feature_list_route_returns_json() {
    let base = boot_with_workspace(
        "feature-list.json",
        br#"{"version":1,"goal":"g","features":[]}"#,
    )
    .await;

    let resp = with_auth_headers(
        reqwest::Client::new().get(format!("{base}/v1/sessions/s1/feature-list")),
    )
    .send()
    .await
    .expect("feature-list request");
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["version"], 1);
}

#[tokio::test]
async fn http_progress_route_returns_tail_with_default_lines() {
    let base = boot_with_workspace("progress.txt", b"line-a\nline-b\nline-c\n").await;

    let resp =
        with_auth_headers(reqwest::Client::new().get(format!("{base}/v1/sessions/s1/progress")))
            .send()
            .await
            .expect("progress request");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.text().await.expect("body");
    assert!(body.contains("line-c"));
}
