# Story 0.17 — WebSocket server (Axum + envelope protocol)

> **Status**: done
> **Estimated**: 2 hours
> **Dependencies**: story 0.4 (events), 0.5 (pubsub), 0.14 (agent runner)
> **Phase**: 0
> **Type**: backend
> **Reads first**: `/specs/phase-0/architecture.md` §4.2 (WebSocket envelope protocol), §1 (component diagram)

---

## Goal

Add the `/ws` Axum endpoint that implements the envelope protocol
defined in architecture §4.2. After this story, a websocket client can:

1. Subscribe to a session and replay missed events
2. Submit `task_create` / `task_pause` / `task_resume` / `task_cancel`
   / `user_response` commands
3. Receive `event` messages in real time via Redis pub/sub fan-out

This is the bridge between the frontend (story 0.18+) and the backend.

## Acceptance criteria

- [ ] `seasoned-hand-server` exposes `/ws` Axum WebSocket handler at
      `GET /ws` (upgrade)
- [ ] All messages JSON-encoded matching architecture §4.2 envelope:
      `{type, id, session_id?, ts, payload}` for events and commands;
      `{type: "ack" | "ping" | "pong" | "error", ...}` for control
- [ ] Implements **subscribe** command:
      - Replay events from `from_event_id` (default 0) by `EventStore::query`
      - Then stream new events from `RedisPool::subscribe(session_id)`
      - One subscription per WS connection per session (multiple
        subscribes for different sessions on the same WS allowed)
- [ ] Implements **task_create** command:
      - Server allocates a session row (uuid4, state RUNNING)
      - Spawns the agent runner in a Tokio task: `state.runner.run(...)`
      - Returns `ack{id, ok:true, ref: <original cmd id>, session_id}`
      - The agent's events flow via the existing hook → Redis pub/sub
        → subscriber path
- [ ] Implements **task_pause** / **task_resume** / **task_cancel** as
      no-op stubs in Phase 0 (real semantics need a `cancel_token` per
      session, which the agent runner can listen on — wire that in
      story 0.27 E2E if needed; DEBT entry covers the gap)
- [ ] Implements **user_response** command:
      - Records a `Message` event with role=user, content=<text>,
        in_reply_to_call_id=<call_id from the prior message_ask_user>
      - Resumes the suspended session by re-spawning the runner with
        the prior state (Phase 0: just transition session SUSPENDED →
        RUNNING and let the agent loop's next iteration consume the new
        user message from the event stream)
- [ ] Heartbeat: server sends `ping` every 30 s; if no `pong` in 10 s,
      close the connection with a 1011 (internal error) code
- [ ] Connection on close: cancels all in-flight subscriptions for
      that connection; agent runs continue (they're owned by the
      session, not the connection)
- [ ] **Failure tolerance**: any malformed JSON from the client → reply
      with `error{kind:"bad_envelope"}`; do not close the connection
- [ ] Unit tests:
      - `envelope_roundtrip_event` (encode/decode JSON shape)
      - `subscribe_replays_then_streams` (in-memory DB + Redis)
      - `task_create_returns_session_id_and_starts_runner` (wiremock'd
        Bifrost serving idle on turn 1)
      - `user_response_resumes_suspended_session`
      - `bad_json_does_not_close_connection`
- [ ] Integration test using `tokio-tungstenite` client against a live
      ephemeral-port Axum server
- [ ] `cargo clippy / fmt / test / spec-check` all pass
- [ ] DEBT.md: add entry for pause/resume/cancel no-op + cancel token

## Non-goals

- Frontend integration (story 0.20)
- Auth (Phase 5)
- Compression of large events over WS (Phase 1)
- Backpressure / flow control (Phase 1 — Phase 0 trusts the local Redis pub/sub)
- Real pause/resume/cancel semantics (this story stubs them, story
  0.27 E2E or a Phase 1 follow-up wires real cancellation tokens)

---

## Implementation steps

### 1. Envelope types

```rust
// crates/seasoned-hand-server/src/ws.rs
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientEnvelope {
    Command {
        id: String,
        session_id: Option<String>,
        #[serde(default)]
        ts: i64,
        payload: CommandPayload,
    },
    Pong { ts: i64 },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum CommandPayload {
    Subscribe { session_id: String, from_event_id: Option<i64> },
    TaskCreate { input: String, max_steps: Option<u32>, cost_cap_cents: Option<u32> },
    TaskPause { session_id: String },
    TaskResume { session_id: String },
    TaskCancel { session_id: String },
    UserResponse { session_id: String, in_reply_to_call_id: String, content: String },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEnvelope {
    Event { id: String, session_id: String, ts: i64, payload: Value /* EventPayload */ },
    Ack   { id: String, r#ref: String, ok: bool, error: Option<String>, session_id: Option<String> },
    Ping  { ts: i64 },
    Pong  { ts: i64 },
    Error { id: Option<String>, kind: String, message: String },
}
```

### 2. Route

```rust
// in app() builder
.route("/ws", get(ws_upgrade))
```

```rust
async fn ws_upgrade(
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| ws_session(socket, state))
}

async fn ws_session(socket: WebSocket, state: AppState) {
    let (mut tx, mut rx) = socket.split();
    let mut subscriptions: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();
    let mut last_pong = std::time::Instant::now();
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(30));

    loop {
        tokio::select! {
            Some(msg) = rx.next() => { /* parse + handle */ }
            _ = heartbeat.tick() => {
                send(&mut tx, ServerEnvelope::Ping{ts: now_unix()}).await;
                if last_pong.elapsed() > std::time::Duration::from_secs(40) {
                    let _ = tx.send(Message::Close(None)).await;
                    break;
                }
            }
        }
    }

    // cleanup
    for (_, h) in subscriptions.drain() { h.abort(); }
}
```

### 3. Command handlers

For each `CommandPayload` variant, handle inline. `task_create` spawns
`state.runner.run(...)` in a `tokio::spawn`; the future's Output is
ignored (events flow through the hook).

For `subscribe`:

```rust
let from_id = from_event_id.unwrap_or(0);
let replay = state.events.query(&session_id, EventQuery {
    after_id: Some(from_id), ..Default::default()
}).await?;
for e in replay { send(&mut tx, event_envelope(&e)).await; }
let sub = state.redis.subscribe(&session_id).await?;
let mut stream = sub.into_stream();
let tx2 = tx.clone();  // need Arc<Mutex<>> around tx or split_sink/share
let handle = tokio::spawn(async move {
    while let Some(payload) = stream.next().await {
        // payload is the JSON-stringified Event from hooks.rs
        let _ = tx2.send(Message::Text(payload)).await;
    }
});
subscriptions.insert(session_id, handle);
```

### 4. Tests

- Unit: envelope (de)serialization round-trip.
- Integration: spin up an Axum server on `127.0.0.1:0`, connect a
  `tokio-tungstenite` client, exercise each command type.

### 5. DEBT entries

- pause/resume/cancel are stubs — real cancellation token wiring
  deferred to story 0.27 E2E or Phase 1.

---

## Files changed

- `crates/seasoned-hand-server/Cargo.toml` (`tokio-tungstenite`,
  `futures-util` if not already)
- `crates/seasoned-hand-server/src/lib.rs` (mount `/ws` route, AppState wiring if needed)
- `crates/seasoned-hand-server/src/ws.rs` (new)
- `crates/seasoned-hand-server/tests/ws.rs` (new — integration)
- `specs/phase-0/DEBT.md` (pause/resume/cancel stubs)

---

## Spec references

- `/specs/phase-0/architecture.md` §4.2 envelope protocol
- `/specs/phase-0/architecture.md` §1 component diagram
- `/specs/00-philosophy/PRINCIPLES.md` #10 failure-tolerant

---

## Commit message

```
feat(phase-0): story 0.17 - WebSocket server + envelope protocol

- /ws Axum endpoint implementing architecture §4.2 envelope protocol
- ClientEnvelope/ServerEnvelope JSON types via serde tagged enums
- subscribe: replay from from_event_id then stream new events via
  Redis pub/sub
- task_create: spawn AgentRunner in tokio task; events flow through
  existing hooks → Redis → subscriber
- task_pause/resume/cancel: stub (real cancellation token = Phase 1)
- user_response: append user Message event, flip session
  SUSPENDED→RUNNING, runner picks up on next iteration
- ping/pong heartbeat (30s/10s)
- bad-JSON tolerance: error envelope back, connection stays open
- N tests: envelope roundtrip, subscribe replay+stream, task_create
  spawns runner, user_response resume, malformed-json tolerance
- cargo clippy / fmt / test / spec-check all pass

Debt: pause/resume/cancel are no-op stubs in Phase 0.

refs: /specs/phase-0/stories/story-0.17.md
```
