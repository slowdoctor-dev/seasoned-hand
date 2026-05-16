use axum::http::StatusCode;
use futures_util::{SinkExt, StreamExt};
use seasoned_hand_core::events::{EventStore, EventType, NewEvent};
use seasoned_hand_core::router::SlotRouter;
use seasoned_hand_core::sandbox::SandboxClient;
use seasoned_hand_core::search::{SearchClient, SearchProvider};
use seasoned_hand_core::{db, pubsub};
use seasoned_hand_server::{AppState, app};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn boot() -> (String, AppState) {
    let bifrost = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "cmpl-1",
            "model": "agent-primary",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call-1",
                        "type": "function",
                        "function": { "name": "idle", "arguments": "{}" }
                    }]
                }
            }]
        })))
        .mount(&bifrost)
        .await;
    Mock::given(method("GET"))
        .and(path("/cost"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"total_cents": 0})))
        .mount(&bifrost)
        .await;

    let pool = db::open(":memory:").await.unwrap();
    pool.with_conn(|conn| {
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, state) \
             VALUES ('s1', 1, 1, 'RUNNING')",
            [],
        )
        .unwrap();
    })
    .await;
    let redis = pubsub::RedisPool::new("redis://127.0.0.1:6").unwrap();
    let sandbox = SandboxClient::new(
        "ghcr.io/agent-infra/sandbox:1.0.0.152",
        std::env::temp_dir(),
    )
    .unwrap();
    let search = SearchClient::new(SearchProvider::Brave { api_key: None });
    let router = SlotRouter::from_yaml_str(&format!(
        r#"
slots:
  main:
    provider: bifrost
    model: agent-primary
    base_url: {}/v1
"#,
        bifrost.uri()
    ))
    .unwrap();
    let state = AppState::new(pool, redis, sandbox, search, router, Default::default());

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
    (format!("ws://{addr}/ws"), state)
}

async fn recv_envelope(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Value {
    loop {
        let msg = ws.next().await.unwrap().unwrap();
        let text = msg.into_text().unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        if value["type"] == "ping" {
            ws.send(Message::Text(
                json!({ "type": "pong", "ts": value["ts"].as_i64().unwrap_or(0) }).to_string(),
            ))
            .await
            .unwrap();
            continue;
        }
        return value;
    }
}

#[tokio::test]
async fn bad_json_does_not_close_connection() {
    let (url, _) = boot().await;
    let (mut ws, _) = connect_async(url).await.unwrap();

    ws.send(Message::Text("{".into())).await.unwrap();
    let value = recv_envelope(&mut ws).await;
    assert_eq!(value["type"], "error");
    assert_eq!(value["kind"], "bad_envelope");

    ws.send(Message::Text(
        json!({
            "type": "command",
            "id": "c1",
            "ts": 1,
            "payload": { "cmd": "task_pause", "session_id": "s1" }
        })
        .to_string(),
    ))
    .await
    .unwrap();
    let value = recv_envelope(&mut ws).await;
    assert_eq!(value["type"], "ack");
}

/// Story 2.8b: contract pin for the WS `task_create` Ack.
///
/// New flow (post-DEBT #15 close-out): the handler pushes the brief
/// through the IntakeRouter, the attached `WsInitializerSpawner`
/// inserts the sessions row + fires a tokio task that runs the
/// confirm-gate Initializer, and the Ack hands back the pre-allocated
/// `session_id`. The runner itself only starts once a `briefing_confirm`
/// arrives — see [`ws_task_create_emits_briefing_then_confirm_acks_started`]
/// for the full happy path. This test stays focused on the synchronous
/// post-Ack contract (session row exists) so existing chat clients keep
/// working unchanged.
#[tokio::test]
async fn task_create_returns_session_id_and_starts_runner() {
    let (url, state) = boot().await;
    let (mut ws, _) = connect_async(url).await.unwrap();
    ws.send(Message::Text(
        json!({
            "type": "command",
            "id": "create-1",
            "ts": 1,
            "payload": { "cmd": "task_create", "input": "Say hi", "max_steps": 4, "cost_cap_cents": 10 }
        })
        .to_string(),
    ))
    .await
    .unwrap();
    let value = recv_envelope(&mut ws).await;
    assert_eq!(value["type"], "ack");
    assert_eq!(value["ref"], "create-1");
    let session_id = value["session_id"].as_str().unwrap().to_string();

    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    let exists = state
        .db
        .with_conn(move |conn| {
            conn.query_row::<i64, _, _>(
                "SELECT 1 FROM sessions WHERE id = ?",
                [session_id],
                |row| row.get(0),
            )
            .is_ok()
        })
        .await;
    assert!(exists);
}

/// Story 2.8b: full happy-path round-trip across the new wiring.
/// 1. `task_create` Acks with a session_id and triggers the IntakeRouter
///    + WsInitializerSpawner pipeline.
/// 2. The Initializer's confirm gate emits `briefing_pending` + `briefing`
///    Misc events into the session — verify they land in the events store.
/// 3. The per-task briefing sender is registered in
///    `AppState::briefing_senders` keyed by the new task_id.
/// 4. `briefing_confirm` with `action: "confirm"` Acks ok=true; the
///    forward into the per-task mpsc unblocks the confirm gate.
/// 5. After cleanup the briefing sender slot is removed (drop guard
///    inside the spawner's tokio task).
#[tokio::test]
async fn ws_task_create_emits_briefing_then_confirm_acks_started() {
    let (url, state) = boot().await;
    let (mut ws, _) = connect_async(url).await.unwrap();
    ws.send(Message::Text(
        json!({
            "type": "command",
            "id": "create-2",
            "ts": 1,
            "payload": { "cmd": "task_create", "input": "Plan my week" }
        })
        .to_string(),
    ))
    .await
    .unwrap();
    let value = recv_envelope(&mut ws).await;
    assert_eq!(value["type"], "ack");
    assert_eq!(value["ok"], true);
    let session_id = value["session_id"].as_str().unwrap().to_string();

    // Poll the events table until the briefing_pending Misc lands —
    // the spawner's tokio task is fire-and-forget so the wall-clock
    // gap depends on scheduler + planner-fallback latency.
    let mut briefing_call_id: Option<String> = None;
    let mut task_id: Option<String> = None;
    for _ in 0..50 {
        let session = session_id.clone();
        let rows = state
            .events
            .query(
                &session,
                seasoned_hand_core::events::EventQuery {
                    event_type: Some(seasoned_hand_core::events::EventType::Misc),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        for ev in rows {
            let kind = ev.data.get("kind").and_then(|v| v.as_str());
            if kind == Some("briefing_pending") {
                briefing_call_id = ev
                    .data
                    .get("briefing_call_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                task_id = ev
                    .data
                    .get("task_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                break;
            }
        }
        if briefing_call_id.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let call_id = briefing_call_id.expect("briefing_pending event did not arrive");
    let task_id = task_id.expect("briefing_pending Misc carries task_id");

    // The spawner registered a per-task briefing sender — `forward_briefing_confirm`
    // will find it.
    assert!(
        state.briefing_senders.contains_key(&task_id),
        "briefing_senders has entry for task_id while gate is waiting"
    );

    // Confirm the briefing — the Ack must be ok=true.
    ws.send(Message::Text(
        json!({
            "type": "command",
            "id": "confirm-1",
            "ts": 2,
            "payload": {
                "cmd": "briefing_confirm",
                "task_id": task_id,
                "in_reply_to_call_id": call_id,
                "action": "confirm",
            }
        })
        .to_string(),
    ))
    .await
    .unwrap();
    let value = recv_envelope(&mut ws).await;
    assert_eq!(value["type"], "ack");
    assert_eq!(
        value["ok"], true,
        "briefing_confirm ack ok=true (sender found and message forwarded)"
    );

    // The spawner's tokio task removes the sender from the map after
    // the gate returns. The Initializer's `seed_plan_and_run` failure
    // (no sandbox in the test) drops the gate with Err, which still
    // triggers the cleanup branch.
    for _ in 0..40 {
        if !state.briefing_senders.contains_key(&task_id) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(
        !state.briefing_senders.contains_key(&task_id),
        "briefing sender removed after gate returns"
    );
}

/// Story 2.8b: confirming a task with no pending briefing surfaces a
/// descriptive error instead of silently 200'ing. The chat UI uses
/// this to stop showing the briefing card if the user double-clicks
/// confirm or arrives via deep link after auto-confirm fired.
#[tokio::test]
async fn ws_briefing_confirm_unknown_task_acks_not_pending() {
    let (url, _state) = boot().await;
    let (mut ws, _) = connect_async(url).await.unwrap();
    ws.send(Message::Text(
        json!({
            "type": "command",
            "id": "cf-x",
            "ts": 1,
            "payload": {
                "cmd": "briefing_confirm",
                "task_id": "no-such-task",
                "in_reply_to_call_id": "no-such-call",
                "action": "confirm",
            }
        })
        .to_string(),
    ))
    .await
    .unwrap();
    let value = recv_envelope(&mut ws).await;
    assert_eq!(value["type"], "ack");
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"], "no_pending_briefing");
}

/// Story 2.9: the WS `task_create` command now routes through the
/// channel framework's IntakeRouter, which persists a row into V008
/// `intake_events` with `channel = "chat"`. The legacy session+runner
/// path still runs (backward-compat shim until Phase 2 DEBT #13 closes
/// in story 2.8) — this test only checks the new intake side-effect.
#[tokio::test]
async fn ws_task_create_creates_intake_event() {
    let (url, state) = boot().await;
    let (mut ws, _) = connect_async(url).await.unwrap();
    ws.send(Message::Text(
        json!({
            "type": "command",
            "id": "create-intake-1",
            "ts": 1,
            "payload": { "cmd": "task_create", "input": "Summarize this week" }
        })
        .to_string(),
    ))
    .await
    .unwrap();
    let value = recv_envelope(&mut ws).await;
    assert_eq!(value["type"], "ack", "ack envelope received");
    assert_eq!(value["ok"], true, "ack reports ok");
    let session_id = value["session_id"].as_str().unwrap().to_string();

    // handle_event is synchronous from the WS handler's perspective —
    // we still nudge the runtime in case the inner db ops yielded.
    tokio::time::sleep(std::time::Duration::from_millis(60)).await;

    let (count, brief, target_ref): (i64, String, Option<String>) = state
        .db
        .with_conn(move |conn| {
            conn.query_row(
                "SELECT COUNT(*), \
                        COALESCE(MAX(brief_input), ''), \
                        MAX(reply_target) \
                   FROM intake_events WHERE channel = 'chat'",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .expect("query intake_events")
        })
        .await;
    assert_eq!(count, 1, "exactly one intake_events row was persisted");
    assert_eq!(brief, "Summarize this week");
    let target = target_ref.expect("reply_target persisted");
    assert!(
        target.contains(&format!("session:{session_id}")),
        "reply_target encodes the chat session id: {target}"
    );
}

#[tokio::test]
async fn user_response_resumes_suspended_session() {
    let (url, state) = boot().await;
    state
        .db
        .with_conn(|conn| {
            conn.execute("UPDATE sessions SET state='SUSPENDED' WHERE id='s1'", [])
                .unwrap()
        })
        .await;
    let (mut ws, _) = connect_async(url).await.unwrap();

    ws.send(Message::Text(
        json!({
            "type": "command",
            "id": "ur-1",
            "session_id": "s1",
            "ts": 1,
            "payload": {
                "cmd": "user_response",
                "session_id": "s1",
                "in_reply_to_call_id": "call-123",
                "content": "continue"
            }
        })
        .to_string(),
    ))
    .await
    .unwrap();
    let value = recv_envelope(&mut ws).await;
    assert_eq!(value["type"], "ack");
    assert_eq!(value["ok"], true);

    let state_now = state
        .db
        .with_conn(|conn| {
            conn.query_row("SELECT state FROM sessions WHERE id='s1'", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap()
        })
        .await;
    assert_eq!(state_now, "RUNNING");
}

/// Replay-from-DB is fully exercised; the live streaming path is only
/// smoke-tested against an intentionally-broken Redis URL so we observe the
/// `subscribe_failed` error envelope. A real streaming happy-path test
/// would require live Redis — wired in the Phase 1 CI workflow (DEBT #14).
#[tokio::test]
async fn subscribe_replays_then_redis_subscribe_failure() {
    let (url, state) = boot().await;
    state
        .events
        .append(NewEvent {
            session_id: "s1".into(),
            event_type: EventType::Message,
            source: "user".into(),
            data: json!({"role":"user", "content":"hello"}),
        })
        .await
        .unwrap();

    let (mut ws, _) = connect_async(url).await.unwrap();
    ws.send(Message::Text(
        json!({
            "type": "command",
            "id": "sub-1",
            "ts": 1,
            "payload": { "cmd": "subscribe", "session_id": "s1", "from_event_id": 0 }
        })
        .to_string(),
    ))
    .await
    .unwrap();

    let first = recv_envelope(&mut ws).await;
    assert_eq!(first["type"], "event");
    assert_eq!(first["session_id"], "s1");
    assert_eq!(first["payload"]["kind"], "Message");

    let second = recv_envelope(&mut ws).await;
    assert_eq!(second["type"], "error");
    assert_eq!(second["kind"], "subscribe_failed");
}

#[tokio::test]
async fn cost_route_still_works_with_ws_route() {
    let (url, _) = boot().await;
    let http = url.replace("ws://", "http://").replace("/ws", "/v1/cost");
    let resp = reqwest::get(http).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
