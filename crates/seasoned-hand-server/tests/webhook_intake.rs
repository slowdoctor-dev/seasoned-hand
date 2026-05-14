//! Story 2.10 — `POST /v1/intake/webhook` integration coverage.
//!
//! Exercises the WebhookChannel intake surface end-to-end: token gate
//! (503 unset, 401 wrong, 202 happy path) and the persistence side
//! effects (intake_events row + drafted Task row created via
//! IntakeRouter).
//!
//! refs: /specs/phase-2/stories/story-2.10.md
//! refs: /specs/phase-2/architecture.md §2.8, §9 "Webhook intake authentication"

use std::sync::Arc;

use axum::http::StatusCode;
use seasoned_hand_core::router::SlotRouter;
use seasoned_hand_core::sandbox::SandboxClient;
use seasoned_hand_core::search::{SearchClient, SearchProvider};
use seasoned_hand_core::{db, pubsub};
use seasoned_hand_server::{AppState, app};
use serde_json::{Value, json};
use tokio::net::TcpListener;

const TEST_TOKEN: &str = "intake-test-token-xyz";

async fn boot(intake_token: Option<&str>) -> (String, db::DbPool) {
    let pool = db::open(":memory:").await.expect("db");
    let redis = pubsub::RedisPool::new("redis://127.0.0.1:6").expect("redis url");
    let sandbox = SandboxClient::new(
        "ghcr.io/agent-infra/sandbox:1.0.0.152",
        std::env::temp_dir(),
    )
    .expect("sandbox client");
    let search = SearchClient::new(SearchProvider::Brave { api_key: None });
    let router = SlotRouter::default_for_bifrost();

    let token = Arc::new(intake_token.unwrap_or("").to_string());
    let state = AppState::new(
        pool.clone(),
        redis,
        sandbox,
        search,
        router,
        Default::default(),
    )
    .register_webhook_channel(token, Vec::new());

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app(state)).await.expect("serve");
    });
    (format!("http://{addr}"), pool)
}

/// `webhook_intake_returns_503_when_token_unset` — empty
/// `SEASONED_HAND_INTAKE_TOKEN` disables the endpoint entirely.
#[tokio::test]
async fn webhook_intake_returns_503_when_token_unset() {
    let (base, _pool) = boot(None).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/v1/intake/webhook"))
        .json(&json!({"brief": "anything"}))
        .send()
        .await
        .expect("POST");
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "intake_token_not_configured");
}

/// `webhook_intake_rejects_without_token` — token configured but the
/// request omits or sends the wrong header → 401.
#[tokio::test]
async fn webhook_intake_rejects_without_token() {
    let (base, _pool) = boot(Some(TEST_TOKEN)).await;
    let client = reqwest::Client::new();

    // Missing header.
    let missing = client
        .post(format!("{base}/v1/intake/webhook"))
        .json(&json!({"brief": "test"}))
        .send()
        .await
        .expect("POST");
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

    // Wrong token.
    let wrong = client
        .post(format!("{base}/v1/intake/webhook"))
        .header("X-Seasoned-Hand-Intake-Token", "nope")
        .json(&json!({"brief": "test"}))
        .send()
        .await
        .expect("POST");
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
}

/// `webhook_intake_creates_event_and_returns_task_id` — happy path
/// returns 202 + task_id, and the intake_events + tasks rows land in
/// SQLite.
#[tokio::test]
async fn webhook_intake_creates_event_and_returns_task_id() {
    let (base, pool) = boot(Some(TEST_TOKEN)).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/v1/intake/webhook"))
        .header("X-Seasoned-Hand-Intake-Token", TEST_TOKEN)
        .json(&json!({
            "brief": "Summarize the Q4 board deck",
            "metadata": {"from_system": "test"},
        }))
        .send()
        .await
        .expect("POST");
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    assert_eq!(status, StatusCode::ACCEPTED, "body: {body}");
    let task_id = body["task_id"].as_str().expect("task_id").to_string();
    assert!(!task_id.is_empty());
    assert!(body["briefing_call_id"].is_null());

    // Persistence: the IntakeRouter should have inserted both the
    // intake_events row (V008) and the drafted Task row (V006).
    let count = pool
        .with_conn(move |conn| -> rusqlite::Result<(i64, i64)> {
            let intake: i64 = conn.query_row(
                "SELECT COUNT(*) FROM intake_events WHERE channel = 'webhook'",
                [],
                |row| row.get(0),
            )?;
            let tasks: i64 = conn.query_row(
                "SELECT COUNT(*) FROM tasks WHERE status = 'drafted'",
                [],
                |row| row.get(0),
            )?;
            Ok((intake, tasks))
        })
        .await
        .expect("count rows");
    assert_eq!(count.0, 1, "intake row landed");
    assert_eq!(count.1, 1, "drafted task landed");
}

/// Empty brief → spec-shaped `intake_rejected:empty_brief` 400 (closes
/// Phase 2 DEBT #12 for the webhook surface).
#[tokio::test]
async fn webhook_intake_rejects_empty_brief_with_4xx() {
    let (base, _pool) = boot(Some(TEST_TOKEN)).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/v1/intake/webhook"))
        .header("X-Seasoned-Hand-Intake-Token", TEST_TOKEN)
        .json(&json!({"brief": "   "}))
        .send()
        .await
        .expect("POST");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "intake_rejected:empty_brief");
}
