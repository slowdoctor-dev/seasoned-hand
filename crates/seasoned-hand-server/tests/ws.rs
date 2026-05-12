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
        axum::serve(listener, app(serve_state)).await.unwrap();
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

#[tokio::test]
async fn subscribe_replays_then_streams() {
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
