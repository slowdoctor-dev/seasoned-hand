//! Story 1.13b — admin rollback endpoint integration tests.
//!
//! Covers the guards and happy path of
//! `POST /v1/sessions/:id/checkpoints/:cp/rollback`. Non-loopback
//! coverage is unit-tested at the handler level — the integration
//! suite always lands at `127.0.0.1`, so we can't synthesize a remote
//! `RemoteAddr` here without significantly more plumbing.
//!
//! refs: /specs/phase-1/stories/story-1.13b.md

use std::net::SocketAddr;

use axum::http::StatusCode;
use seasoned_hand_core::checkpoint::{CheckpointStore, NewCheckpoint};
use seasoned_hand_core::router::SlotRouter;
use seasoned_hand_core::sandbox::SandboxClient;
use seasoned_hand_core::search::{SearchClient, SearchProvider};
use seasoned_hand_core::{db, pubsub};
use seasoned_hand_server::{AppState, app};
use serde_json::json;
use tokio::net::TcpListener;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TEST_TOKEN: &str = "rollback-test-token-xyz";

struct Harness {
    addr: SocketAddr,
    db: db::DbPool,
    session_id: String,
    checkpoint_id: String,
}

async fn build_harness(session_state: &str, admin_token: Option<&str>) -> Harness {
    // Wiremock for the sandbox `/v1/shell/exec` endpoint.
    let sandbox_mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/shell/exec"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "exit_code": 0, "stdout": "", "stderr": ""
        })))
        .mount(&sandbox_mock)
        .await;
    let sandbox_uri = sandbox_mock.uri();

    let pool = db::open(":memory:").await.expect("db");
    let session_id = "sess-rollback".to_string();
    let sid_clone = session_id.clone();
    let state_str = session_state.to_string();
    pool.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, state) VALUES (?, 0, 0, ?)",
            rusqlite::params![sid_clone, state_str],
        )
        .unwrap();
    })
    .await;

    let cp_store = CheckpointStore::new(pool.clone());
    let cp_id = cp_store
        .insert(NewCheckpoint {
            session_id: session_id.clone(),
            plan_phase_id: 1,
            git_sha: "abc1234deadbeef".into(),
            label: None,
            triggered_by_event_id: 1,
        })
        .await
        .expect("insert checkpoint");

    let redis = pubsub::RedisPool::new("redis://127.0.0.1:6").expect("redis url");
    let sandbox = SandboxClient::new(
        "ghcr.io/agent-infra/sandbox:1.0.0.152",
        std::env::temp_dir(),
    )
    .expect("sandbox client");
    sandbox
        .insert_handle_for_test(seasoned_hand_core::sandbox::SandboxHandle {
            session_id: session_id.clone(),
            container_id: "c1".into(),
            api_url: sandbox_uri,
            novnc_url: "http://127.0.0.1:2".into(),
            ttyd_url: "ws://127.0.0.1:3".into(),
            workspace_host_path: std::env::temp_dir().join(&session_id),
        })
        .await;
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
    // Need to keep MockServer alive — leak it via Box::leak so the
    // mock keeps serving for the test's duration. (Dropping it would
    // tear down the listener.)
    Box::leak(Box::new(sandbox_mock));

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
        session_id,
        checkpoint_id: cp_id,
    }
}

fn url(addr: &SocketAddr, session_id: &str, checkpoint_id: &str) -> String {
    format!("http://{addr}/v1/sessions/{session_id}/checkpoints/{checkpoint_id}/rollback")
}

async fn checkpoint_row_state(
    pool: &db::DbPool,
    id: &str,
) -> Option<(Option<i64>, Option<String>)> {
    let id_owned = id.to_string();
    pool.with_conn(move |conn| {
        let mut stmt = conn
            .prepare("SELECT rolled_back_at, rolled_back_by FROM checkpoints WHERE id = ?")
            .unwrap();
        let mut rows = stmt
            .query_map(rusqlite::params![id_owned], |row| {
                Ok((
                    row.get::<_, Option<i64>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            })
            .unwrap();
        rows.next().map(|r| r.unwrap())
    })
    .await
}

#[tokio::test]
async fn admin_rollback_happy_path_returns_202_and_marks_row() {
    let h = build_harness("SUSPENDED", Some(TEST_TOKEN)).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(url(&h.addr, &h.session_id, &h.checkpoint_id))
        .header("X-Seasoned-Hand-Admin-Token", TEST_TOKEN)
        .json(&json!({"reason": "manual test"}))
        .send()
        .await
        .expect("POST");
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
    assert_eq!(status, StatusCode::ACCEPTED, "body: {body}");
    assert_eq!(body["checkpoint_id"], h.checkpoint_id);
    assert!(body["rolled_back_at"].as_i64().unwrap() > 0);

    let (rolled_back_at, rolled_back_by) =
        checkpoint_row_state(&h.db, &h.checkpoint_id).await.unwrap();
    assert!(rolled_back_at.is_some(), "row must carry rolled_back_at");
    assert_eq!(rolled_back_by.as_deref(), Some("admin:cli"));
}

#[tokio::test]
async fn admin_rollback_refuses_without_token() {
    let h = build_harness("SUSPENDED", Some(TEST_TOKEN)).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(url(&h.addr, &h.session_id, &h.checkpoint_id))
        .json(&json!({"reason": "no token here"}))
        .send()
        .await
        .expect("POST");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "unauthorized_token");
}

#[tokio::test]
async fn admin_rollback_refuses_with_wrong_token() {
    let h = build_harness("SUSPENDED", Some(TEST_TOKEN)).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(url(&h.addr, &h.session_id, &h.checkpoint_id))
        .header("X-Seasoned-Hand-Admin-Token", "not-the-real-token")
        .json(&json!({"reason": "x"}))
        .send()
        .await
        .expect("POST");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_rollback_refuses_while_running() {
    let h = build_harness("RUNNING", Some(TEST_TOKEN)).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(url(&h.addr, &h.session_id, &h.checkpoint_id))
        .header("X-Seasoned-Hand-Admin-Token", TEST_TOKEN)
        .json(&json!({"reason": "test"}))
        .send()
        .await
        .expect("POST");
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "wrong_state");
}

#[tokio::test]
async fn admin_rollback_refuses_while_verifying() {
    let h = build_harness("VERIFYING", Some(TEST_TOKEN)).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(url(&h.addr, &h.session_id, &h.checkpoint_id))
        .header("X-Seasoned-Hand-Admin-Token", TEST_TOKEN)
        .json(&json!({"reason": "test"}))
        .send()
        .await
        .expect("POST");
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "wrong_state");
}

#[tokio::test]
async fn admin_rollback_503_when_admin_token_unset() {
    // admin_token=None ⇒ AppState.admin_token is empty ⇒ 503.
    let h = build_harness("SUSPENDED", None).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(url(&h.addr, &h.session_id, &h.checkpoint_id))
        .header("X-Seasoned-Hand-Admin-Token", TEST_TOKEN)
        .json(&json!({"reason": "test"}))
        .send()
        .await
        .expect("POST");
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "admin_token_not_configured");
}

#[tokio::test]
async fn admin_rollback_404_when_checkpoint_missing() {
    let h = build_harness("SUSPENDED", Some(TEST_TOKEN)).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(url(&h.addr, &h.session_id, "no-such-checkpoint"))
        .header("X-Seasoned-Hand-Admin-Token", TEST_TOKEN)
        .json(&json!({"reason": "test"}))
        .send()
        .await
        .expect("POST");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "checkpoint_not_found");
}
