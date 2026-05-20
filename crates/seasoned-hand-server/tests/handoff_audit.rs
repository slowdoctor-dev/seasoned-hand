use std::net::SocketAddr;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use seasoned_hand_core::audit::{AuditAction, AuditLogger, AuditQuery, AuditRecord};
use seasoned_hand_core::auth::{AuthContext, Role};
use seasoned_hand_core::db;
use seasoned_hand_core::project::{NewProject, NewTask, ProjectStore, TaskStatus, TaskStore};
use seasoned_hand_core::pubsub;
use seasoned_hand_core::router::SlotRouter;
use seasoned_hand_core::sandbox::SandboxClient;
use seasoned_hand_core::search::{SearchClient, SearchProvider};
use seasoned_hand_server::{AppState, app};
use tower::ServiceExt;

fn auth_headers(
    req: axum::http::request::Builder,
    tenant_id: &str,
    actor_user_id: &str,
    role: &str,
) -> axum::http::request::Builder {
    req.header("x-seasoned-hand-tenant-id", tenant_id)
        .header(
            "x-seasoned-hand-organization-id",
            format!("org-{tenant_id}"),
        )
        .header("x-seasoned-hand-actor-user-id", actor_user_id)
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
    let state = AppState::new(
        pool.clone(),
        redis,
        sandbox,
        search,
        router,
        Default::default(),
    );
    pool.with_conn(|conn| {
        conn.execute(
            "INSERT INTO organizations (id, tenant_id, slug, display_name, status, created_at, updated_at)
             VALUES ('org-tenant-a', 'tenant-a', 'tenant-a', 'Tenant A', 'active', 1, 1)",
            [],
        )?;
        conn.execute(
            "INSERT INTO users (id, tenant_id, email, display_name, status, created_at, updated_at)
             VALUES ('u-admin', 'tenant-a', 'admin@acme.dev', 'Admin', 'active', 1, 1)",
            [],
        )?;
        conn.execute(
            "INSERT INTO users (id, tenant_id, email, display_name, status, created_at, updated_at)
             VALUES ('u-user', 'tenant-a', 'user@acme.dev', 'User', 'active', 1, 1)",
            [],
        )?;
        conn.execute(
            "INSERT INTO users (id, tenant_id, email, display_name, status, created_at, updated_at)
             VALUES ('u-target', 'tenant-a', 'target@acme.dev', 'Target', 'active', 1, 1)",
            [],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .expect("seed users");
    state
}

async fn seed_task(state: &AppState) -> String {
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
    tasks
        .set_status(&task_id, TaskStatus::Briefed)
        .await
        .expect("briefed");
    task_id
}

#[tokio::test]
async fn handoff_endpoint_admin_200_and_emits_audit_id() {
    let state = test_state().await;
    let task_id = seed_task(&state).await;
    state
        .db
        .with_conn({
            let task_id = task_id.clone();
            move |conn| {
                conn.execute(
                    "UPDATE tasks SET owner_user_id = 'u-user' WHERE id = ?",
                    rusqlite::params![task_id],
                )?;
                Ok::<(), rusqlite::Error>(())
            }
        })
        .await
        .expect("owner seed");
    let request = auth_headers(
        Request::builder()
            .method("POST")
            .uri(format!("/v1/tasks/{task_id}/handoff")),
        "tenant-a",
        "u-admin",
        "admin",
    )
    .extension(axum::extract::ConnectInfo::<SocketAddr>(
        "127.0.0.1:4000".parse().unwrap(),
    ))
    .header("content-type", "application/json")
    .body(Body::from(
        r#"{"to_user_email":"target@acme.dev","reason":"coverage"}"#,
    ))
    .expect("request");

    let response = app(state.clone()).oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let out: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert!(
        out["audit_log_id"].as_str().is_some(),
        "handoff must surface audit id"
    );
}

#[tokio::test]
async fn audit_endpoint_viewer_403_admin_200() {
    let state = test_state().await;
    let logger = AuditLogger::new(state.db.clone(), state.events.clone());
    logger
        .record(
            &AuthContext {
                tenant_id: "tenant-a".into(),
                organization_id: "org-tenant-a".into(),
                actor_user_id: "u-admin".into(),
                org_role: Role::Admin,
                project_override_role: None,
            },
            AuditRecord {
                action: AuditAction::TaskHandoff,
                resource_type: "task",
                resource_id: "t-1",
                target_user_id: Some("u-target"),
                decision: Some("allow"),
                reason: Some("seed"),
                metadata: serde_json::json!({}),
            },
        )
        .await
        .expect("seed audit");

    let viewer_req = auth_headers(
        Request::builder().method("GET").uri("/v1/audit?limit=10"),
        "tenant-a",
        "u-user",
        "viewer",
    )
    .extension(axum::extract::ConnectInfo::<SocketAddr>(
        "127.0.0.1:4000".parse().unwrap(),
    ))
    .body(Body::empty())
    .expect("request");
    let viewer_resp = app(state.clone())
        .oneshot(viewer_req)
        .await
        .expect("viewer response");
    assert_eq!(viewer_resp.status(), StatusCode::FORBIDDEN);

    let admin_req = auth_headers(
        Request::builder().method("GET").uri("/v1/audit?limit=10"),
        "tenant-a",
        "u-admin",
        "admin",
    )
    .extension(axum::extract::ConnectInfo::<SocketAddr>(
        "127.0.0.1:4000".parse().unwrap(),
    ))
    .body(Body::empty())
    .expect("request");
    let admin_resp = app(state).oneshot(admin_req).await.expect("admin response");
    assert_eq!(admin_resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn audit_endpoint_user_limited_view() {
    let state = test_state().await;
    let logger = AuditLogger::new(state.db.clone(), state.events.clone());
    logger
        .record(
            &AuthContext {
                tenant_id: "tenant-a".into(),
                organization_id: "org-tenant-a".into(),
                actor_user_id: "u-admin".into(),
                org_role: Role::Admin,
                project_override_role: None,
            },
            AuditRecord {
                action: AuditAction::TaskHandoff,
                resource_type: "task",
                resource_id: "t-1",
                target_user_id: Some("u-target"),
                decision: Some("allow"),
                reason: Some("admin-row"),
                metadata: serde_json::json!({}),
            },
        )
        .await
        .expect("seed admin audit");
    logger
        .record(
            &AuthContext {
                tenant_id: "tenant-a".into(),
                organization_id: "org-tenant-a".into(),
                actor_user_id: "u-user".into(),
                org_role: Role::User,
                project_override_role: None,
            },
            AuditRecord {
                action: AuditAction::TaskCancel,
                resource_type: "task",
                resource_id: "t-2",
                target_user_id: None,
                decision: Some("allow"),
                reason: Some("user-row"),
                metadata: serde_json::json!({}),
            },
        )
        .await
        .expect("seed user audit");

    let req = auth_headers(
        Request::builder().method("GET").uri("/v1/audit?limit=10"),
        "tenant-a",
        "u-user",
        "user",
    )
    .extension(axum::extract::ConnectInfo::<SocketAddr>(
        "127.0.0.1:4000".parse().unwrap(),
    ))
    .body(Body::empty())
    .expect("request");
    let resp = app(state).oneshot(req).await.expect("response");
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.expect("body");
    let rows: Vec<seasoned_hand_core::audit::AuditRow> =
        serde_json::from_slice(&bytes).expect("json");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].actor_user_id, "u-user");
    // guard against accidental widening
    let _ = logger
        .query(
            &AuthContext {
                tenant_id: "tenant-a".into(),
                organization_id: "org-tenant-a".into(),
                actor_user_id: "u-user".into(),
                org_role: Role::User,
                project_override_role: None,
            },
            AuditQuery::default(),
        )
        .await
        .expect("query");
}
