use futures_util::{SinkExt, StreamExt};
use seasoned_hand_core::router::SlotRouter;
use seasoned_hand_core::sandbox::SandboxClient;
use seasoned_hand_core::search::{SearchClient, SearchProvider};
use seasoned_hand_core::{db, pubsub};
use seasoned_hand_server::{AppState, app};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

mod common {
    pub mod gaia;
}

use common::gaia::load_all;

#[tokio::test]
#[ignore = "requires live server + llm keys; run with SEASONED_HAND_PHASE1_SMOKE=1"]
async fn phase1_gaia() {
    if std::env::var("SEASONED_HAND_PHASE1_SMOKE").ok().as_deref() != Some("1") {
        eprintln!("phase1_gaia skipped: set SEASONED_HAND_PHASE1_SMOKE=1");
        return;
    }

    let ws_url = boot_live().await;
    let fixtures =
        load_all("crates/seasoned-hand-server/tests/fixtures/phase1_gaia").expect("load fixtures");
    assert_eq!(
        fixtures.len(),
        10,
        "fixture corpus must be deterministic (10)"
    );

    let mut passed = 0usize;
    for (idx, f) in fixtures.iter().enumerate() {
        let ok = run_one(&ws_url, f).await;
        if ok {
            passed += 1;
        }
        eprintln!(
            "[phase1_gaia] {:02}. {} => {}",
            idx + 1,
            f.title,
            if ok { "PASS" } else { "FAIL" }
        );
    }

    eprintln!("[phase1_gaia] aggregate: {passed}/{}", fixtures.len());
    assert!(
        passed >= 8,
        "expected at least 8/10 GAIA tasks to pass; got {passed}/{}",
        fixtures.len()
    );
}

async fn boot_live() -> String {
    let bifrost_base =
        std::env::var("BIFROST_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:4000/v1".into());
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());

    let pool = db::open(":memory:").await.expect("db");
    let redis = pubsub::RedisPool::new(redis_url).expect("redis");
    let sandbox = SandboxClient::new(
        "ghcr.io/agent-infra/sandbox:1.0.0.152",
        std::env::temp_dir(),
    )
    .expect("sandbox");
    let search = SearchClient::new(SearchProvider::Brave { api_key: None });
    let router = SlotRouter::from_yaml_str(&format!(
        r#"
slots:
  main:
    provider: bifrost
    model: agent-primary
    base_url: {bifrost_base}
  verifier:
    provider: bifrost
    model: verifier-secondary
    base_url: {bifrost_base}
"#
    ))
    .expect("router");
    let state = AppState::new(pool, redis, sandbox, search, router, Default::default())
        .with_verifier_prompt(Arc::new(
            std::fs::read_to_string("config/prompts/verifier.system.txt")
                .unwrap_or_else(|_| "You are verifier.".to_string()),
        ));

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app(state)).await.expect("serve");
    });
    format!("ws://{addr}/ws")
}

async fn run_one(ws_url: &str, fixture: &common::gaia::GaiaFixture) -> bool {
    let (mut ws, _) = match tokio_tungstenite::connect_async(ws_url).await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("connect failed for '{}': {e}", fixture.title);
            return false;
        }
    };

    let create = json!({
        "type": "command",
        "id": format!("gaia-{}", fixture.title.replace(' ', "-")),
        "ts": 0,
        "payload": {
            "cmd": "task_create",
            "input": fixture.briefing,
            "max_steps": fixture.max_steps,
            "cost_cap_cents": fixture.cost_cap_cents,
        }
    });
    if ws.send(Message::Text(create.to_string())).await.is_err() {
        return false;
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    let mut final_msg: Option<String> = None;

    while std::time::Instant::now() < deadline {
        let Ok(Some(Ok(msg))) =
            tokio::time::timeout(std::time::Duration::from_secs(30), ws.next()).await
        else {
            break;
        };
        let Message::Text(text) = msg else { continue };
        let env: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if env.get("type").and_then(Value::as_str) == Some("ping") {
            let ts = env.get("ts").and_then(Value::as_i64).unwrap_or(0);
            let _ = ws
                .send(Message::Text(json!({"type":"pong","ts":ts}).to_string()))
                .await;
            continue;
        }

        if env.get("type").and_then(Value::as_str) != Some("event") {
            continue;
        }
        let payload = env.get("payload").cloned().unwrap_or(Value::Null);
        if payload.get("kind").and_then(Value::as_str) != Some("Message") {
            continue;
        }
        if payload.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        if payload.get("ui").and_then(Value::as_str) != Some("notify") {
            continue;
        }
        if let Some(content) = payload.get("content").and_then(Value::as_str) {
            final_msg = Some(content.to_string());
        }
    }

    let Some(msg) = final_msg else { return false };
    fixture
        .expected_in_final_message
        .iter()
        .all(|s| msg.to_lowercase().contains(&s.to_lowercase()))
}
