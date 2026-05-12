//! Phase 0 end-to-end acceptance test.
//! refs: /specs/phase-0/requirements.md §5
//! refs: /specs/phase-0/architecture.md §11.3
//!
//! Gated `#[ignore]` because the test needs:
//!   - Bifrost reachable at $BIFROST_BASE_URL (default http://localhost:4000/v1)
//!   - At least ANTHROPIC_API_KEY or OPENAI_API_KEY in env
//!   - Redis at $REDIS_URL (default redis://127.0.0.1:6379)
//!   - Docker for the sandbox container
//!
//! Run with:
//!   docker compose up -d bifrost redis
//!   cargo test --workspace -- --ignored e2e_phase0
//!
//! CI wiring is its own follow-up story (see DEBT #14).

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
#[ignore = "requires live Bifrost + Redis + Docker + provider key"]
async fn e2e_phase0_acceptance() {
    let ws_url = std::env::var("SH_E2E_WS_URL")
        .unwrap_or_else(|_| "ws://127.0.0.1:3001/ws".to_string());

    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("connect /ws");

    let create = json!({
        "type": "command",
        "id": "e2e-1",
        "ts": 0,
        "payload": {
            "cmd": "task_create",
            "input": "Find the GitHub stars of FoundationAgents/OpenManus",
            "max_steps": 12
        }
    });
    ws.send(Message::Text(create.to_string()))
        .await
        .expect("send task_create");

    let mut session_id: Option<String> = None;
    let mut tool_calls = 0u32;
    let mut answer: Option<String> = None;

    let deadline =
        std::time::Instant::now() + std::time::Duration::from_secs(180);
    while std::time::Instant::now() < deadline {
        let Ok(Some(Ok(msg))) = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            ws.next(),
        )
        .await
        else {
            break;
        };
        let Message::Text(text) = msg else { continue };
        let env: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match env.get("type").and_then(|v| v.as_str()) {
            Some("ack") => {
                if session_id.is_none() {
                    session_id = env
                        .get("session_id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                }
            }
            Some("event") => {
                let payload = env.get("payload").cloned().unwrap_or(Value::Null);
                let kind = payload.get("kind").and_then(|v| v.as_str());
                if kind == Some("Action") {
                    tool_calls += 1;
                }
                if kind == Some("Message") {
                    let role = payload.get("role").and_then(|v| v.as_str());
                    let ui = payload.get("ui").and_then(|v| v.as_str());
                    if role == Some("assistant") && ui == Some("notify") {
                        if let Some(c) = payload.get("content").and_then(|v| v.as_str())
                        {
                            if c.chars().any(|ch| ch.is_ascii_digit()) {
                                answer = Some(c.to_string());
                                break;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    assert!(session_id.is_some(), "no session_id in ack");
    assert!(
        tool_calls > 0,
        "expected at least one tool dispatch (Action event)"
    );
    assert!(
        answer
            .as_deref()
            .map(|s| s.chars().any(|c| c.is_ascii_digit()))
            .unwrap_or(false),
        "no digit-bearing assistant message before timeout; got answer = {answer:?}"
    );
}
