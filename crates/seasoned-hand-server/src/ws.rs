//! WebSocket endpoint and envelope protocol.
//! refs: /specs/phase-0/stories/story-0.17.md
//! refs: /specs/phase-0/architecture.md §4.2

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use seasoned_hand_core::agent::RunRequest;
use seasoned_hand_core::channel::{
    DeliveryTarget, IntakeEvent, chat::CHANNEL_NAME as CHAT_CHANNEL,
    chat::TARGET_SESSION_PREFIX as CHAT_TARGET_SESSION_PREFIX,
};
use seasoned_hand_core::events::{Event, EventQuery, EventStore, EventType, NewEvent};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::AppState;

const HEARTBEAT_SECONDS: u64 = 30;
const PONG_TIMEOUT_SECONDS: u64 = 10;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientEnvelope {
    Command {
        id: String,
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        ts: i64,
        payload: CommandPayload,
    },
    Pong {
        #[serde(default)]
        ts: i64,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum CommandPayload {
    Subscribe {
        session_id: String,
        from_event_id: Option<i64>,
    },
    TaskCreate {
        input: String,
        max_steps: Option<u32>,
        cost_cap_cents: Option<u32>,
    },
    TaskPause {
        session_id: String,
    },
    TaskResume {
        session_id: String,
    },
    TaskCancel {
        session_id: String,
    },
    UserResponse {
        session_id: String,
        in_reply_to_call_id: String,
        content: String,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEnvelope {
    Event {
        id: String,
        session_id: String,
        ts: i64,
        payload: Value,
    },
    Ack {
        id: String,
        r#ref: String,
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },
    Ping {
        ts: i64,
    },
    Pong {
        ts: i64,
    },
    Error {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        kind: String,
        message: String,
    },
}

pub async fn ws_upgrade(State(state): State<AppState>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |socket| ws_session(socket, state))
}

async fn ws_session(socket: WebSocket, state: AppState) {
    let (mut writer, mut reader) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerEnvelope>();
    let (close_tx, mut close_rx) = mpsc::unbounded_channel::<(u16, &'static str)>();
    let mut subscriptions: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(HEARTBEAT_SECONDS));
    let mut last_pong = Instant::now();

    let writer_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                Some(envelope) = rx.recv() => {
                    match serde_json::to_string(&envelope) {
                        Ok(text) => {
                            if writer.send(Message::Text(text)).await.is_err() {
                                break;
                            }
                        }
                        Err(error) => {
                            tracing::warn!(%error, "failed to encode ws envelope");
                        }
                    }
                }
                Some((code, reason)) = close_rx.recv() => {
                    let _ = writer
                        .send(Message::Close(Some(CloseFrame {
                            code,
                            reason: reason.into(),
                        })))
                        .await;
                    break;
                }
                else => break,
            }
        }
    });

    loop {
        tokio::select! {
            incoming = reader.next() => {
                let Some(message) = incoming else {
                    break;
                };
                match message {
                    Ok(Message::Text(text)) => {
                        match serde_json::from_str::<ClientEnvelope>(&text) {
                            Ok(ClientEnvelope::Pong { .. }) => {
                                // Client pong is a heartbeat reply, NOT a request — refresh the
                                // last-pong timer and do not echo. Echoing a `{type:"pong"}`
                                // envelope back made reads race with command replies (the
                                // `bad_json_does_not_close_connection` test caught this as
                                // intermittent `expected:"ack", got:"pong"`).
                                last_pong = Instant::now();
                            }
                            Ok(ClientEnvelope::Command { id, payload, .. }) => {
                                handle_command(&state, &tx, &mut subscriptions, id, payload).await;
                            }
                            Err(error) => {
                                let _ = tx.send(ServerEnvelope::Error {
                                    id: None,
                                    kind: "bad_envelope".into(),
                                    message: error.to_string(),
                                });
                            }
                        }
                    }
                    Ok(Message::Pong(_)) => {
                        last_pong = Instant::now();
                    }
                    Ok(Message::Close(_)) => break,
                    Ok(_) => {}
                    Err(error) => {
                        tracing::warn!(%error, "websocket read error");
                        break;
                    }
                }
            }
            _ = heartbeat.tick() => {
                let _ = tx.send(ServerEnvelope::Ping { ts: now_unix() });
                if last_pong.elapsed() > Duration::from_secs(HEARTBEAT_SECONDS + PONG_TIMEOUT_SECONDS) {
                    // architecture §4.2 / story 0.17: close with 1011 on pong timeout
                    let _ = close_tx.send((1011, "pong_timeout"));
                    break;
                }
            }
        }
    }

    for (_, handle) in subscriptions.drain() {
        handle.abort();
    }
    writer_handle.abort();
}

async fn handle_command(
    state: &AppState,
    tx: &mpsc::UnboundedSender<ServerEnvelope>,
    subscriptions: &mut HashMap<String, tokio::task::JoinHandle<()>>,
    cmd_id: String,
    payload: CommandPayload,
) {
    match payload {
        CommandPayload::Subscribe {
            session_id,
            from_event_id,
        } => {
            replay_events(state, tx, &session_id, from_event_id.unwrap_or(0)).await;
            attach_subscription(state, tx, subscriptions, &session_id).await;
            let _ = tx.send(ServerEnvelope::Ack {
                id: Uuid::new_v4().to_string(),
                r#ref: cmd_id,
                ok: true,
                error: None,
                session_id: Some(session_id),
            });
        }
        CommandPayload::TaskCreate {
            input,
            max_steps,
            cost_cap_cents,
        } => {
            let session_id = Uuid::new_v4().to_string();
            if let Err(error) = insert_session_row(state, &session_id, "RUNNING").await {
                let _ = tx.send(ServerEnvelope::Ack {
                    id: Uuid::new_v4().to_string(),
                    r#ref: cmd_id,
                    ok: false,
                    error: Some(error.to_string()),
                    session_id: None,
                });
                return;
            }

            // Story 2.9: route the brief through the channel framework's
            // IntakeRouter so the chat panel participates uniformly with
            // other intake channels. The router persists an
            // `intake_events` row, finds/creates the tenant's Inbox
            // project, and creates a `drafted` Task linked to the intake
            // id. The Initializer spawn that turns the drafted Task into
            // a running session is deferred to story 2.8 (Phase 2 DEBT
            // #13); until that lands the legacy runner-spawn below is a
            // backward-compatibility shim so existing chat clients keep
            // working.
            let intake_event = IntakeEvent {
                channel: CHAT_CHANNEL.into(),
                intake_id: format!("ws:{}", Uuid::new_v4()),
                brief_input: input.clone(),
                reply_target: Some(DeliveryTarget {
                    channel: CHAT_CHANNEL.into(),
                    target_ref: format!("{CHAT_TARGET_SESSION_PREFIX}{session_id}"),
                    metadata: json!({}),
                }),
                metadata: json!({ "ws_cmd_id": cmd_id }),
                tenant_id: None,
                received_at: now_unix(),
            };
            if let Err(error) = state.intake_router.handle_event(intake_event).await {
                // Non-fatal: the legacy session+runner still proceeds so
                // the WS contract (synchronous session_id) is preserved.
                // The intake_events row is the only loss, and we record
                // it as a warning for observability.
                tracing::warn!(error = %error, %session_id,
                    "ws task_create: IntakeRouter rejected event; falling back to legacy direct-run");
            }

            let runner = state.runner.clone();
            let run_session = session_id.clone();
            tokio::spawn(async move {
                let _ = runner
                    .run(RunRequest {
                        session_id: run_session,
                        input,
                        max_steps: max_steps.unwrap_or(24),
                        cost_cap_cents,
                    })
                    .await;
            });
            let _ = tx.send(ServerEnvelope::Ack {
                id: Uuid::new_v4().to_string(),
                r#ref: cmd_id,
                ok: true,
                error: None,
                session_id: Some(session_id),
            });
        }
        CommandPayload::TaskPause { session_id } => {
            let result = handle_task_pause(state, &session_id).await;
            let _ = tx.send(ServerEnvelope::Ack {
                id: Uuid::new_v4().to_string(),
                r#ref: cmd_id,
                ok: result.is_ok(),
                error: result.err(),
                session_id: Some(session_id),
            });
        }
        CommandPayload::TaskResume { session_id } => {
            let result = handle_task_resume(state, &session_id).await;
            let _ = tx.send(ServerEnvelope::Ack {
                id: Uuid::new_v4().to_string(),
                r#ref: cmd_id,
                ok: result.is_ok(),
                error: result.err(),
                session_id: Some(session_id),
            });
        }
        CommandPayload::TaskCancel { session_id } => {
            let result = handle_task_cancel(state, &session_id).await;
            let _ = tx.send(ServerEnvelope::Ack {
                id: Uuid::new_v4().to_string(),
                r#ref: cmd_id,
                ok: result.is_ok(),
                error: result.err(),
                session_id: Some(session_id),
            });
        }
        CommandPayload::UserResponse {
            session_id,
            in_reply_to_call_id,
            content,
        } => {
            let append = state
                .events
                .append(NewEvent {
                    session_id: session_id.clone(),
                    event_type: EventType::Message,
                    source: "user".into(),
                    data: json!({
                        "role":"user",
                        "content": content,
                        "in_reply_to_call_id": in_reply_to_call_id,
                    }),
                })
                .await;
            if let Err(error) = append {
                let _ = tx.send(ServerEnvelope::Ack {
                    id: Uuid::new_v4().to_string(),
                    r#ref: cmd_id,
                    ok: false,
                    error: Some(error.to_string()),
                    session_id: Some(session_id),
                });
                return;
            }
            if let Err(error) = set_session_state(state, &session_id, "RUNNING").await {
                let _ = tx.send(ServerEnvelope::Ack {
                    id: Uuid::new_v4().to_string(),
                    r#ref: cmd_id,
                    ok: false,
                    error: Some(error.to_string()),
                    session_id: Some(session_id),
                });
                return;
            }
            let runner = state.runner.clone();
            let resume_session = session_id.clone();
            tokio::spawn(async move {
                let _ = runner
                    .resume(RunRequest {
                        session_id: resume_session,
                        input: String::new(),
                        max_steps: 24,
                        cost_cap_cents: None,
                    })
                    .await;
            });
            let _ = tx.send(ServerEnvelope::Ack {
                id: Uuid::new_v4().to_string(),
                r#ref: cmd_id,
                ok: true,
                error: None,
                session_id: Some(session_id),
            });
        }
    }
}

async fn session_state(state: &AppState, session_id: &str) -> Result<Option<String>, String> {
    let sid = session_id.to_string();
    let found = state
        .db
        .with_conn(move |conn| {
            conn.query_row(
                "SELECT state FROM sessions WHERE id = ?",
                rusqlite::params![sid],
                |row| row.get::<_, String>(0),
            )
            .ok()
        })
        .await;
    Ok(found)
}

async fn handle_task_pause(state: &AppState, session_id: &str) -> Result<(), String> {
    let current = session_state(state, session_id)
        .await?
        .ok_or_else(|| "unknown_session".to_string())?;
    if current != "RUNNING" && current != "VERIFYING" {
        return Err("wrong_state".into());
    }
    state
        .sandbox
        .pause(session_id)
        .await
        .map_err(|_| "internal".to_string())?;
    set_session_state(state, session_id, "SUSPENDED").await?;
    let _ = state
        .events
        .append(NewEvent {
            session_id: session_id.to_string(),
            event_type: EventType::Misc,
            source: "ws".into(),
            data: json!({"kind":"task_paused"}),
        })
        .await;
    Ok(())
}

async fn handle_task_resume(state: &AppState, session_id: &str) -> Result<(), String> {
    let current = session_state(state, session_id)
        .await?
        .ok_or_else(|| "unknown_session".to_string())?;
    if current != "SUSPENDED" {
        return Err("wrong_state".into());
    }
    state
        .sandbox
        .resume(session_id)
        .await
        .map_err(|_| "internal".to_string())?;
    set_session_state(state, session_id, "RUNNING").await?;
    let _ = state
        .events
        .append(NewEvent {
            session_id: session_id.to_string(),
            event_type: EventType::Misc,
            source: "ws".into(),
            data: json!({"kind":"task_resumed"}),
        })
        .await;
    let runner = state.runner.clone();
    let sid = session_id.to_string();
    tokio::spawn(async move {
        let _ = runner.resume_session(&sid).await;
    });
    Ok(())
}

async fn handle_task_cancel(state: &AppState, session_id: &str) -> Result<(), String> {
    let current = session_state(state, session_id)
        .await?
        .ok_or_else(|| "unknown_session".to_string())?;
    if current == "FINISHED" || current == "ERROR" {
        return Err("wrong_state".into());
    }
    if let Some(token) = state.cancel_tokens.get(session_id) {
        token.cancel();
    }
    let _ = state
        .events
        .append(NewEvent {
            session_id: session_id.to_string(),
            event_type: EventType::Misc,
            source: "ws".into(),
            data: json!({"kind":"task_cancelled","by":"user"}),
        })
        .await;
    let _ = state.sandbox.destroy(session_id).await;
    set_session_state(state, session_id, "FINISHED").await?;
    state.cancel_tokens.remove(session_id);
    Ok(())
}

async fn replay_events(
    state: &AppState,
    tx: &mpsc::UnboundedSender<ServerEnvelope>,
    session_id: &str,
    from_event_id: i64,
) {
    match state
        .events
        .query(
            session_id,
            EventQuery {
                after_id: Some(from_event_id),
                ..Default::default()
            },
        )
        .await
    {
        Ok(events) => {
            for event in events {
                let _ = tx.send(event_envelope(&event));
            }
        }
        Err(error) => {
            let _ = tx.send(ServerEnvelope::Error {
                id: None,
                kind: "replay_failed".into(),
                message: error.to_string(),
            });
        }
    }
}

async fn attach_subscription(
    state: &AppState,
    tx: &mpsc::UnboundedSender<ServerEnvelope>,
    subscriptions: &mut HashMap<String, tokio::task::JoinHandle<()>>,
    session_id: &str,
) {
    if let Some(handle) = subscriptions.remove(session_id) {
        handle.abort();
    }

    let subscription = match state.redis.subscribe(session_id).await {
        Ok(subscription) => subscription,
        Err(error) => {
            let _ = tx.send(ServerEnvelope::Error {
                id: None,
                kind: "subscribe_failed".into(),
                message: error.to_string(),
            });
            return;
        }
    };
    let stream = subscription.into_stream();
    let tx_clone = tx.clone();
    let session_id = session_id.to_string();
    let session_key = session_id.clone();
    let handle = tokio::spawn(async move {
        tokio::pin!(stream);
        while let Some(payload) = stream.next().await {
            match serde_json::from_str::<Value>(&payload) {
                Ok(value) => {
                    let _ = tx_clone.send(event_envelope_from_value(value));
                }
                Err(error) => {
                    let _ = tx_clone.send(ServerEnvelope::Error {
                        id: None,
                        kind: "stream_decode_failed".into(),
                        message: format!("session {session_id}: {error}"),
                    });
                }
            }
        }
    });
    subscriptions.insert(session_key, handle);
}

/// Shape one of the five canonical event kinds (Message / Action /
/// Observation / Plan / Misc) into the WS client payload. Returns
/// `None` for any other event type — the two callers handle the
/// default branch themselves because the DB path emits a bare
/// `{kind, data}` envelope while the Redis-stream path also carries
/// the originating `source`.
///
/// Story 2.9 special-cases `Misc{kind:"Deliverable"}` — the ChatChannel
/// `DeliverySink` impl appends a Misc event with that shape, and the
/// architecture §4 ServerEvent payload contract renders it as a
/// dedicated `{kind:"Deliverable", deliverable_id, format, file_ref,
/// citations}` payload (frontend story 2.22 filters on this kind).
fn build_payload(event_type_str: &str, data: &Value) -> Option<Value> {
    if event_type_str == "Misc" && data.get("kind").and_then(Value::as_str) == Some("Deliverable") {
        return Some(json!({
            "kind": "Deliverable",
            "deliverable_id": data.get("deliverable_id").cloned().unwrap_or(Value::Null),
            "format": data.get("format").cloned().unwrap_or(Value::Null),
            "file_ref": data.get("file_ref").cloned().unwrap_or(Value::Null),
            "citations": data.get("citations").cloned().unwrap_or_else(|| json!([])),
        }));
    }
    Some(match event_type_str {
        "Message" => json!({
            "kind": "Message",
            "role": data.get("role").and_then(Value::as_str).unwrap_or("assistant"),
            "content": data.get("content").cloned().unwrap_or(Value::String(String::new())),
            "ui": data.get("ui").cloned().unwrap_or(Value::Null),
            "in_reply_to_call_id": data.get("in_reply_to_call_id").cloned().unwrap_or(Value::Null),
        }),
        "Action" => json!({
            "kind": "Action",
            "tool": data.get("tool").cloned().unwrap_or(Value::String(String::new())),
            "args": data.get("args").cloned().unwrap_or(Value::Object(serde_json::Map::new())),
            "call_id": data.get("call_id").cloned().unwrap_or(Value::Null),
        }),
        "Observation" => json!({
            "kind": "Observation",
            "call_id": data.get("call_id").cloned().unwrap_or(Value::Null),
            "ok": data.get("ok").cloned().unwrap_or(Value::Bool(false)),
            "output": data.get("output").cloned().unwrap_or(Value::Null),
            "error": data.get("error").cloned().unwrap_or(Value::Null),
            "file_ref": data.get("file_ref").cloned().unwrap_or(Value::Null),
        }),
        "Plan" => json!({
            "kind": "Plan",
            "op": data.get("op").cloned().unwrap_or(Value::Null),
            "plan_id": data.get("plan_id").cloned().unwrap_or(Value::Null),
            "snapshot": data.get("snapshot").cloned().unwrap_or(Value::Null),
        }),
        "Misc" => json!({
            "kind": "Misc",
            "kind_tag": data.get("kind").cloned().unwrap_or(Value::Null),
            "data": data.clone(),
        }),
        _ => return None,
    })
}

fn event_envelope(event: &Event) -> ServerEnvelope {
    let event_type_str = event.event_type.as_str();
    let payload = build_payload(event_type_str, &event.data).unwrap_or_else(|| {
        json!({
            "kind": event_type_str,
            "data": event.data.clone(),
        })
    });
    ServerEnvelope::Event {
        id: event.id.to_string(),
        session_id: event.session_id.clone(),
        ts: event.timestamp,
        payload,
    }
}

fn event_envelope_from_value(value: Value) -> ServerEnvelope {
    let event_id = value
        .get("id")
        .and_then(Value::as_i64)
        .unwrap_or_default()
        .to_string();
    let session_id = value
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let ts = value
        .get("timestamp")
        .or_else(|| value.get("ts"))
        .and_then(Value::as_i64)
        .unwrap_or_else(now_unix);
    let event_type = value.get("type").and_then(Value::as_str).unwrap_or("Misc");
    let source = value
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or("tool:unknown")
        .to_string();
    let data = value.get("data").cloned().unwrap_or(Value::Null);
    let payload = build_payload(event_type, &data).unwrap_or_else(|| {
        json!({
            "kind": event_type,
            "source": source,
            "data": data,
        })
    });
    ServerEnvelope::Event {
        id: event_id,
        session_id,
        ts,
        payload,
    }
}

async fn insert_session_row(
    state: &AppState,
    session_id: &str,
    state_name: &str,
) -> Result<(), String> {
    let session_id = session_id.to_string();
    let state_name = state_name.to_string();
    let now = now_micros();
    state
        .db
        .with_conn(move |conn| {
            conn.execute(
                "INSERT INTO sessions (id, created_at, updated_at, state) VALUES (?, ?, ?, ?)",
                (session_id, now, now, state_name),
            )
        })
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

async fn set_session_state(
    state: &AppState,
    session_id: &str,
    state_name: &str,
) -> Result<(), String> {
    let session_id = session_id.to_string();
    let state_name = state_name.to_string();
    state
        .db
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE sessions SET state = ?, updated_at = ? WHERE id = ?",
                (state_name, now_micros(), session_id),
            )
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn now_micros() -> i64 {
    let micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros();
    i64::try_from(micros).unwrap_or(i64::MAX)
}

fn now_unix() -> i64 {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    i64::try_from(seconds).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppState;
    use seasoned_hand_core::capability::ModelCapabilities;
    use seasoned_hand_core::db;
    use seasoned_hand_core::router::SlotRouter;
    use seasoned_hand_core::sandbox::SandboxClient;
    use seasoned_hand_core::search::SearchClient;

    #[test]
    fn envelope_roundtrip_event() {
        let envelope = ServerEnvelope::Event {
            id: "1".into(),
            session_id: "s1".into(),
            ts: 123,
            payload: json!({"kind":"Message","role":"assistant","content":"hi"}),
        };
        let json = serde_json::to_string(&envelope).unwrap();
        let value: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["type"], "event");
        assert_eq!(value["session_id"], "s1");
        assert_eq!(value["payload"]["kind"], "Message");
    }

    async fn test_state() -> AppState {
        let db = db::open(":memory:").await.unwrap();
        let redis = seasoned_hand_core::pubsub::RedisPool::new("redis://127.0.0.1:6").unwrap();
        let sandbox = SandboxClient::new(
            "ghcr.io/agent-infra/sandbox:1.0.0.152",
            std::env::temp_dir(),
        )
        .unwrap();
        AppState::new(
            db,
            redis,
            sandbox,
            SearchClient::brave_from_env(),
            SlotRouter::default_for_bifrost(),
            std::collections::HashMap::<String, ModelCapabilities>::new(),
        )
    }

    #[tokio::test]
    async fn pause_rejected_when_finished() {
        let state = test_state().await;
        state
            .db
            .with_conn(|conn| {
                conn.execute(
                    "INSERT INTO sessions (id, created_at, updated_at, state) VALUES ('s1',0,0,'FINISHED')",
                    [],
                )
                .unwrap();
            })
            .await;
        let err = handle_task_pause(&state, "s1")
            .await
            .expect_err("wrong_state");
        assert_eq!(err, "wrong_state");
    }

    #[tokio::test]
    async fn resume_rejected_when_running() {
        let state = test_state().await;
        state
            .db
            .with_conn(|conn| {
                conn.execute(
                    "INSERT INTO sessions (id, created_at, updated_at, state) VALUES ('s1',0,0,'RUNNING')",
                    [],
                )
                .unwrap();
            })
            .await;
        let err = handle_task_resume(&state, "s1")
            .await
            .expect_err("wrong_state");
        assert_eq!(err, "wrong_state");
    }
}
