//! refs: /specs/phase-0/architecture.md §3.2, §3.4, §4.1

use axum::http::StatusCode;
use seasoned_hand_core::events::{EventStore, EventType, NewEvent};
use seasoned_hand_core::{db, pubsub};
use seasoned_hand_server::{AppState, app};
use serde_json::json;
use tokio::net::TcpListener;

fn auth_client() -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "x-seasoned-hand-tenant-id",
        "legacy-default".parse().unwrap(),
    );
    headers.insert(
        "x-seasoned-hand-organization-id",
        "org-legacy-default".parse().unwrap(),
    );
    headers.insert(
        "x-seasoned-hand-actor-user-id",
        "user-admin".parse().unwrap(),
    );
    headers.insert("x-seasoned-hand-org-role", "admin".parse().unwrap());
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap()
}

async fn boot() -> (String, AppState) {
    let pool = db::open(":memory:").await.unwrap();
    pool.with_conn(|conn| {
        conn.execute(
            "INSERT INTO projects (id, tenant_id, title, status, created_at, updated_at) \
             VALUES ('p1', 'legacy-default', 'proj', 'active', 1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, project_id, created_at, updated_at, state) \
             VALUES ('s1', 'p1', 1, 1, 'RUNNING')",
            [],
        )
        .unwrap();
    })
    .await;
    // Tests use an unreachable redis URL — append() will log a publish-failure
    // warning but still succeed (PRINCIPLE #10 failure-tolerance).
    let redis = pubsub::RedisPool::new("redis://127.0.0.1:6").unwrap();
    let sandbox = seasoned_hand_core::sandbox::SandboxClient::new(
        "ghcr.io/agent-infra/sandbox:1.0.0.152",
        std::env::temp_dir(),
    )
    .unwrap();
    let search = seasoned_hand_core::search::SearchClient::new(
        seasoned_hand_core::search::SearchProvider::Brave { api_key: None },
    );
    let router = seasoned_hand_core::router::SlotRouter::default_for_bifrost();
    let state = AppState::new(pool, redis, sandbox, search, router, Default::default())
        .allow_insecure_auth_headers();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve_state = state.clone();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app(serve_state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });

    (format!("http://{addr}"), state)
}

#[tokio::test]
async fn list_events_returns_appended_events() {
    let (base, state) = boot().await;
    let client = auth_client();
    state
        .events
        .append(NewEvent {
            session_id: "s1".into(),
            event_type: EventType::Message,
            source: "user".into(),
            data: json!({"content": "hello"}),
        })
        .await
        .unwrap();
    state
        .events
        .append(NewEvent {
            session_id: "s1".into(),
            event_type: EventType::Action,
            source: "agent".into(),
            data: json!({"tool": "ping"}),
        })
        .await
        .unwrap();

    let resp = client
        .get(format!("{base}/v1/sessions/s1/events"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["type"], "Message");
    assert_eq!(arr[1]["type"], "Action");
    assert_eq!(arr[0]["data"]["content"], "hello");
}

#[tokio::test]
async fn list_events_filters_by_type() {
    let (base, state) = boot().await;
    let client = auth_client();
    for t in [EventType::Message, EventType::Action, EventType::Action] {
        state
            .events
            .append(NewEvent {
                session_id: "s1".into(),
                event_type: t,
                source: "x".into(),
                data: json!({}),
            })
            .await
            .unwrap();
    }
    let resp = client
        .get(format!("{base}/v1/sessions/s1/events?type=Action"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn list_events_404_for_unknown_session() {
    let (base, _state) = boot().await;
    let client = auth_client();
    let resp = client
        .get(format!("{base}/v1/sessions/nope/events"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "session_not_found");
}

#[tokio::test]
async fn list_events_400_for_unknown_type() {
    let (base, _state) = boot().await;
    let client = auth_client();
    let resp = client
        .get(format!("{base}/v1/sessions/s1/events?type=Bogus"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// --- Issue #22 batch B: tenant-isolation correctness ------------------------

fn auth_client_for(tenant: &str, org: &str, user: &str, role: &str) -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("x-seasoned-hand-tenant-id", tenant.parse().unwrap());
    headers.insert("x-seasoned-hand-organization-id", org.parse().unwrap());
    headers.insert("x-seasoned-hand-actor-user-id", user.parse().unwrap());
    headers.insert("x-seasoned-hand-org-role", role.parse().unwrap());
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap()
}

/// Seed a chat-spawned session whose tenancy comes ONLY from `task_id`
/// (project_id NULL) — the path `initializer_spawner` takes.
async fn seed_task_spawned_session(state: &AppState) {
    state
        .db
        .with_conn(|conn| {
            conn.execute(
                "INSERT INTO tasks (id, project_id, tenant_id, title, brief, status, \
                   expected_due_at, completed_at, failure_reason, parent_task_id, schedule, \
                   skill_attached_event_id, created_at, updated_at) \
                 VALUES ('tk1', 'p1', 'legacy-default', 'chat', NULL, 'running', NULL, NULL, \
                   NULL, NULL, NULL, NULL, 2, 2)",
                [],
            )
            .unwrap();
            // project_id NULL: tenancy must resolve via task_id.
            conn.execute(
                "INSERT INTO sessions (id, project_id, task_id, created_at, updated_at, state) \
                 VALUES ('s_chat', NULL, 'tk1', 2, 2, 'RUNNING')",
                [],
            )
            .unwrap();
        })
        .await;
}

#[tokio::test]
async fn list_events_reaches_task_spawned_session() {
    // B1: a chat-spawned session (project_id NULL, tenancy from task_id) must be
    // reachable by its tenant. The old inline `JOIN projects` 404'd it.
    let (base, state) = boot().await;
    seed_task_spawned_session(&state).await;
    let resp = auth_client()
        .get(format!("{base}/v1/sessions/s_chat/events"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "task-spawned session must not 404 for its tenant"
    );
}

#[tokio::test]
async fn list_events_task_spawned_session_404_for_other_tenant() {
    // B1 isolation: a different tenant still gets 404 for the task-spawned session.
    let (base, state) = boot().await;
    seed_task_spawned_session(&state).await;
    let resp = auth_client_for("tenant-b", "org-b", "user-b", "admin")
        .get(format!("{base}/v1/sessions/s_chat/events"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn list_sessions_includes_project_and_task_spawned_sessions() {
    // B2: the canonical join returns BOTH project-spawned (s1) and task-spawned
    // (s_chat) sessions for the tenant. The old `project_id IN (SELECT id FROM
    // tasks ...)` filter matched the wrong column and returned neither.
    let (base, state) = boot().await;
    seed_task_spawned_session(&state).await;
    let resp = auth_client()
        .get(format!("{base}/v1/sessions"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let rows: serde_json::Value = resp.json().await.unwrap();
    let ids: Vec<&str> = rows
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();
    assert!(
        ids.contains(&"s1"),
        "project-spawned session missing: {ids:?}"
    );
    assert!(
        ids.contains(&"s_chat"),
        "task-spawned session missing: {ids:?}"
    );
}

/// Seed a session whose direct parents disagree on tenant: project_id → tenant-A,
/// task_id → tenant-B. A corrupt/partially-migrated shape that must belong to
/// NEITHER tenant (fail-closed), not leak to whichever parent a tenant shares.
async fn seed_mismatched_parent_session(state: &AppState) {
    state
        .db
        .with_conn(|conn| {
            conn.execute(
                "INSERT INTO projects (id, tenant_id, title, status, created_at, updated_at) \
                 VALUES ('pA', 'tenant-a', 'projA', 'active', 1, 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO projects (id, tenant_id, title, status, created_at, updated_at) \
                 VALUES ('pBb', 'tenant-b', 'projBb', 'active', 1, 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO tasks (id, project_id, tenant_id, title, brief, status, \
                   expected_due_at, completed_at, failure_reason, parent_task_id, schedule, \
                   skill_attached_event_id, created_at, updated_at) \
                 VALUES ('tkB', 'pBb', 'tenant-b', 'taskB', NULL, 'running', NULL, NULL, \
                   NULL, NULL, NULL, NULL, 3, 3)",
                [],
            )
            .unwrap();
            // project_id -> tenant-a, task_id -> tenant-b: parents disagree.
            conn.execute(
                "INSERT INTO sessions (id, project_id, task_id, created_at, updated_at, state) \
                 VALUES ('s_mismatch', 'pA', 'tkB', 3, 3, 'RUNNING')",
                [],
            )
            .unwrap();
        })
        .await;
}

#[tokio::test]
async fn mismatched_parent_session_is_invisible_to_both_tenants() {
    // Issue #22 batch B review: fail-closed when a session's project and task
    // resolve to different tenants — neither tenant may read it (raw events) or
    // list it. A plain COALESCE(project, task) would have leaked it to tenant-A.
    let (base, state) = boot().await;
    seed_mismatched_parent_session(&state).await;

    for (tenant, org, user) in [
        ("tenant-a", "org-a", "user-a"),
        ("tenant-b", "org-b", "user-b"),
    ] {
        let client = auth_client_for(tenant, org, user, "admin");
        // Raw events feed: 404 for both.
        let events = client
            .get(format!("{base}/v1/sessions/s_mismatch/events"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            events.status(),
            StatusCode::NOT_FOUND,
            "{tenant} must not read raw events of a mismatched-parent session"
        );
        // Session listing: absent for both.
        let list = client
            .get(format!("{base}/v1/sessions"))
            .send()
            .await
            .unwrap();
        assert_eq!(list.status(), StatusCode::OK);
        let rows: serde_json::Value = list.json().await.unwrap();
        let has = rows
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"].as_str() == Some("s_mismatch"));
        assert!(!has, "{tenant} must not list a mismatched-parent session");
    }
}

#[tokio::test]
async fn redacted_feed_isolates_cross_tenant() {
    // B3: GET /v1/events/:session_id (the redacted feed) filters by the caller's
    // tenant via visibility::query. A tenant-A caller must NOT read a tenant-B
    // session's rows; the tenant-B caller sees its own.
    let (base, state) = boot().await;
    state
        .db
        .with_conn(|conn| {
            conn.execute(
                "INSERT INTO projects (id, tenant_id, title, status, created_at, updated_at) \
                 VALUES ('pB', 'tenant-b', 'projB', 'active', 1, 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions (id, project_id, created_at, updated_at, state) \
                 VALUES ('sB', 'pB', 1, 1, 'RUNNING')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO events (session_id, timestamp, type, source, data) \
                 VALUES ('sB', 1, 'Misc', 'test', '{}')",
                [],
            )
            .unwrap();
            let eid: i64 = conn
                .query_row("SELECT id FROM events WHERE session_id = 'sB'", [], |r| {
                    r.get(0)
                })
                .unwrap();
            conn.execute(
                "INSERT INTO tenant_event_view \
                   (event_id, tenant_id, visibility_level, redacted_data, searchable_text, created_at) \
                 VALUES (?, 'tenant-b', 'user', '{\"k\":\"secretB\"}', 'secretB', 1)",
                rusqlite::params![eid],
            )
            .unwrap();
        })
        .await;

    // tenant-A (legacy-default) must see NONE of tenant-B's rows.
    let a = auth_client();
    let resp = a.get(format!("{base}/v1/events/sB")).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let rows: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        rows.as_array().unwrap().len(),
        0,
        "cross-tenant leak: tenant-A read tenant-B's redacted events"
    );

    // Positive control: tenant-B sees its own redacted row.
    let b = auth_client_for("tenant-b", "org-b", "user-b", "admin");
    let resp = b.get(format!("{base}/v1/events/sB")).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let rows: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        rows.as_array().unwrap().len(),
        1,
        "tenant-B must see its own redacted event"
    );
}
