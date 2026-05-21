use std::net::SocketAddr;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use seasoned_hand_core::db;
use seasoned_hand_core::project::{NewProject, NewTask, ProjectStore, TaskStore};
use seasoned_hand_core::pubsub;
use seasoned_hand_core::router::SlotRouter;
use seasoned_hand_core::sandbox::SandboxClient;
use seasoned_hand_core::search::{SearchClient, SearchProvider};
use seasoned_hand_server::{AppState, app};
use tower::ServiceExt;

fn auth_headers(
    req: axum::http::request::Builder,
    tenant_id: &str,
    role: &str,
) -> axum::http::request::Builder {
    req.header("x-seasoned-hand-tenant-id", tenant_id)
        .header(
            "x-seasoned-hand-organization-id",
            format!("org-{tenant_id}"),
        )
        .header("x-seasoned-hand-actor-user-id", "user-test")
        .header("x-seasoned-hand-org-role", role)
}

async fn test_state() -> AppState {
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

#[tokio::test]
async fn middleware_auth_viewer_handoff_returns_403() {
    let state = test_state().await;
    let projects = ProjectStore::new(state.db.clone());
    let tasks = TaskStore::new(state.db.clone());
    let project_id = projects
        .insert(NewProject {
            tenant_id: Some("tenant-a".to_string()),
            title: "P".to_string(),
            description: None,
        })
        .await
        .expect("insert project");
    let task_id = tasks
        .insert(NewTask {
            project_id,
            tenant_id: Some("tenant-a".to_string()),
            title: "T".to_string(),
            expected_due_at: None,
        })
        .await
        .expect("insert task");

    let request = auth_headers(
        Request::builder()
            .method("POST")
            .uri(format!("/v1/tasks/{task_id}/handoff")),
        "tenant-a",
        "viewer",
    )
    .extension(axum::extract::ConnectInfo::<SocketAddr>(
        "127.0.0.1:4000".parse().unwrap(),
    ))
    .header("content-type", "application/json")
    .body(Body::from(r#"{"to_user_id":"user-next"}"#))
    .expect("request");

    let response = app(state).oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn middleware_auth_forged_tenant_scopes_project_list_to_zero_rows() {
    let state = test_state().await;
    let projects = ProjectStore::new(state.db.clone());
    projects
        .insert(NewProject {
            tenant_id: Some("tenant-a".to_string()),
            title: "Tenant A Project".to_string(),
            description: None,
        })
        .await
        .expect("insert project");

    let request = auth_headers(
        Request::builder()
            .method("GET")
            .uri("/v1/projects?limit=50"),
        "tenant-b",
        "user",
    )
    .extension(axum::extract::ConnectInfo::<SocketAddr>(
        "127.0.0.1:4000".parse().unwrap(),
    ))
    .body(Body::empty())
    .expect("request");

    let response = app(state).oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    if status != StatusCode::OK {
        panic!(
            "unexpected status {status}; body={}",
            String::from_utf8_lossy(&bytes)
        );
    }
    let rows: Vec<seasoned_hand_core::project::Project> =
        serde_json::from_slice(&bytes).expect("json");
    assert!(rows.is_empty(), "tenant-b should not see tenant-a rows");
}

#[tokio::test]
async fn middleware_auth_tenant_a_gets_404_for_tenant_b_id_endpoints() {
    let state = test_state().await.with_admin_token("test-admin-token");
    let projects = ProjectStore::new(state.db.clone());
    let tasks = TaskStore::new(state.db.clone());

    let project_b = projects
        .insert(NewProject {
            tenant_id: Some("tenant-b".to_string()),
            title: "Tenant B Project".to_string(),
            description: None,
        })
        .await
        .expect("insert project");
    let task_b = tasks
        .insert(NewTask {
            project_id: project_b.clone(),
            tenant_id: Some("tenant-b".to_string()),
            title: "Tenant B Task".to_string(),
            expected_due_at: None,
        })
        .await
        .expect("insert task");

    let task_b_for_seed = task_b.clone();
    let project_b_for_seed = project_b.clone();
    state
        .db
        .with_conn(move |conn| -> rusqlite::Result<()> {
            conn.execute(
                "INSERT INTO sessions (id, task_id, project_id, created_at, updated_at, state, title, cost_cents, tool_calls)
                 VALUES ('sess-b', ?, ?, 10, 10, 'FINISHED', 'session-b', 0, 0)",
                rusqlite::params![task_b_for_seed, project_b_for_seed],
            )?;
            conn.execute(
                "INSERT INTO verifications (
                    id, session_id, triggered_at_event_id, trigger_kind,
                    trigger_detail, verdict, reason, evidence_event_ids,
                    suggested_plan_update, model_id, cost_cents, created_at
                ) VALUES (
                    'verif-b', 'sess-b', 1, 'TaskComplete',
                    'hardening-test', 'pass', 'ok', '[]',
                    '{}', 'gpt-test', 0, 11
                )",
                [],
            )?;
            conn.execute(
                "INSERT INTO checkpoints (
                    id, session_id, plan_phase_id, git_sha, label,
                    triggered_by_event_id, created_at
                ) VALUES (
                    'cp-b', 'sess-b', 1, 'abc123', 'cp-b',
                    1, 12
                )",
                [],
            )?;
            Ok(())
        })
        .await
        .expect("seed session fixtures");

    let app = app(state);

    let id_endpoints = [
        (format!("/v1/tasks/{task_b}"), "GET", false),
        (format!("/v1/tasks/{task_b}/deliverables"), "GET", false),
        (format!("/v1/tasks/{task_b}/provenance"), "GET", false),
        (format!("/v1/projects/{project_b}/tasks"), "GET", false),
        (format!("/v1/projects/{project_b}/archive"), "POST", false),
        ("/v1/sessions/sess-b".to_string(), "GET", false),
        ("/v1/sessions/sess-b/progress".to_string(), "GET", false),
        ("/v1/sessions/sess-b/feature-list".to_string(), "GET", false),
        (
            "/v1/sessions/sess-b/verifications".to_string(),
            "GET",
            false,
        ),
        ("/v1/sessions/sess-b/checkpoints".to_string(), "GET", false),
        (
            "/v1/sessions/sess-b/checkpoints/cp-b/rollback".to_string(),
            "POST",
            true,
        ),
        ("/v1/verifications/verif-b".to_string(), "GET", false),
    ];

    for (uri, method, needs_admin_token) in id_endpoints {
        let mut builder = auth_headers(
            Request::builder().method(method).uri(&uri),
            "tenant-a",
            "admin",
        )
        .extension(axum::extract::ConnectInfo::<SocketAddr>(
            "127.0.0.1:4000".parse().unwrap(),
        ));
        if needs_admin_token {
            builder = builder
                .header("x-seasoned-hand-admin-token", "test-admin-token")
                .header("content-type", "application/json");
        }
        let body = if method == "POST" {
            Body::from(r#"{"reason":"rollback test"}"#)
        } else {
            Body::empty()
        };
        let req = builder.body(body).expect("request");
        let res = app.clone().oneshot(req).await.expect("response");
        let status = res.status();
        let body = to_bytes(res.into_body(), usize::MAX)
            .await
            .expect("response body");
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "uri={uri} unexpected status={status} body={}",
            String::from_utf8_lossy(&body)
        );
    }
}
