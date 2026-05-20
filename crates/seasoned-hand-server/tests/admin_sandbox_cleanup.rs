//! Story 2.17 — admin manual workspace-cleanup endpoint integration test.
//!
//! `POST /v1/admin/sandbox/cleanup` runs one [`WorkspaceTtlCron`] cycle
//! synchronously and returns `{cleaned, failed}`. This test wires the
//! full HTTP path (loopback connect-info, admin-token guard) and
//! verifies the cycle actually fires: a completed task seeded with an
//! aged `updated_at` but no session row is a valid candidate — the
//! cron looks it up, finds no session to GC, and *bumps* the task's
//! `updated_at` so the next cycle won't re-scan it. That bump is the
//! observable side-effect we assert against, independent of whether
//! the host has docker reachable.
//!
//! refs: /specs/phase-2/stories/story-2.17.md

use std::net::SocketAddr;
use std::time::Duration;

use axum::http::StatusCode;
use seasoned_hand_core::project::{NewProject, ProjectStore, TaskStatus};
use seasoned_hand_core::router::SlotRouter;
use seasoned_hand_core::sandbox::SandboxClient;
use seasoned_hand_core::search::{SearchClient, SearchProvider};
use seasoned_hand_core::time::now_micros;
use seasoned_hand_core::{db, pubsub};
use seasoned_hand_server::{AppState, app};
use serde_json::json;
use tokio::net::TcpListener;

const TEST_TOKEN: &str = "ttl-admin-test-token";

struct Harness {
    addr: SocketAddr,
    db: db::DbPool,
    task_id: String,
    seeded_updated_at: i64,
}

async fn build_harness(admin_token: Option<&str>) -> Harness {
    let pool = db::open(":memory:").await.expect("db");

    // Seed a project + a completed task whose updated_at is 31 days in
    // the past. The cron's default `SANDBOX_TTL_COMPLETED_DAYS=30` will
    // include it; with no `sessions` row attached the cleanup branch
    // short-circuits to "bump updated_at + Skipped" — no docker contact
    // needed, so the test is portable across hosts without docker.
    let project_store = ProjectStore::new(pool.clone());
    let project_id = project_store
        .insert(NewProject {
            tenant_id: None,
            title: "p".into(),
            description: None,
        })
        .await
        .expect("project");
    let task_id = uuid::Uuid::new_v4().to_string();
    let aged = now_micros() - i64::try_from(Duration::from_secs(31 * 86_400).as_micros()).unwrap();
    let tid = task_id.clone();
    let pid = project_id.clone();
    let status_str = TaskStatus::Completed.as_db_str().to_string();
    pool.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO tasks (\
               id, project_id, tenant_id, title, brief, status, \
               expected_due_at, completed_at, failure_reason, \
               parent_task_id, schedule, skill_attached_event_id, \
               created_at, updated_at\
             ) VALUES (?, ?, 'legacy-default', 't', NULL, ?, NULL, NULL, NULL, NULL, NULL, NULL, ?, ?)",
            rusqlite::params![tid, pid, status_str, aged, aged],
        )
        .unwrap();
    })
    .await;

    let redis = pubsub::RedisPool::new("redis://127.0.0.1:6").expect("redis url");
    let sandbox = SandboxClient::new(
        "ghcr.io/agent-infra/sandbox:1.0.0.152",
        std::env::temp_dir(),
    )
    .expect("sandbox client");
    let search = SearchClient::new(SearchProvider::Brave { api_key: None });
    let router = SlotRouter::default_for_bifrost();

    let mut state = AppState::new(
        pool.clone(),
        redis,
        sandbox,
        search,
        router,
        Default::default(),
    );
    if let Some(token) = admin_token {
        state = state.with_admin_token(token);
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(
            listener,
            app(state).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .expect("serve");
    });

    Harness {
        addr,
        db: pool,
        task_id,
        seeded_updated_at: aged,
    }
}

fn url(addr: &SocketAddr) -> String {
    format!("http://{addr}/v1/admin/sandbox/cleanup")
}

#[tokio::test]
async fn admin_manual_cleanup_route_runs_one_cycle() {
    let h = build_harness(Some(TEST_TOKEN)).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(url(&h.addr))
        .header("X-Seasoned-Hand-Admin-Token", TEST_TOKEN)
        .send()
        .await
        .expect("POST");
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap_or(json!(null));
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(
        body["cleaned"].is_i64(),
        "body shape missing `cleaned`: {body}"
    );
    assert!(
        body["failed"].is_i64(),
        "body shape missing `failed`: {body}"
    );

    // Strong-evidence assertion: the seeded task's `updated_at` MUST
    // have been bumped past the aged seed value. The cycle's
    // "no-session" branch is the only code path that touches
    // `updated_at` for a non-running row that we didn't insert a
    // session for, so a forward bump proves the cron fired through
    // candidate collection.
    let tid = h.task_id.clone();
    let updated_at: i64 =
        h.db.with_conn(move |conn| {
            conn.query_row(
                "SELECT updated_at FROM tasks WHERE id = ?",
                rusqlite::params![tid],
                |row| row.get(0),
            )
            .unwrap()
        })
        .await;
    assert!(
        updated_at > h.seeded_updated_at,
        "expected cron to bump updated_at past {} but got {updated_at}",
        h.seeded_updated_at,
    );
}

#[tokio::test]
async fn admin_cleanup_refuses_without_token() {
    let h = build_harness(Some(TEST_TOKEN)).await;
    let client = reqwest::Client::new();
    let resp = client.post(url(&h.addr)).send().await.expect("POST");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "unauthorized_token");
}

#[tokio::test]
async fn admin_cleanup_refuses_with_wrong_token() {
    let h = build_harness(Some(TEST_TOKEN)).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(url(&h.addr))
        .header("X-Seasoned-Hand-Admin-Token", "not-the-token")
        .send()
        .await
        .expect("POST");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_cleanup_503_when_admin_token_unset() {
    let h = build_harness(None).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(url(&h.addr))
        .header("X-Seasoned-Hand-Admin-Token", TEST_TOKEN)
        .send()
        .await
        .expect("POST");
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "admin_token_not_configured");
}
