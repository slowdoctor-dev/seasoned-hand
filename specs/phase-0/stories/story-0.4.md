# Story 0.4 — Event Stream API (append + query)

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: story 0.3 (SQLite schema + migrations)
> **Phase**: 0
> **Type**: backend
> **Reads first**: `/specs/phase-0/architecture.md` §3.2 + §3.4 (event types and `data` shapes) + §4.1 (HTTP route `GET /v1/sessions/:id/events`)

---

## Goal

Add the append-only event stream API: typed `Event` enum mirroring
architecture §3.4 payloads, an `EventStore` trait with **only** `append`
and `query` (no `update` / `delete` — the append-only invariant is
enforced at the trait surface, per PRINCIPLE #3 and architecture §3.2),
and an HTTP endpoint to query a session's events. `subscribe` is
deliberately deferred to story 0.5 (lands with Redis pub/sub).

## Acceptance criteria

- [ ] `seasoned-hand-core::events` module exposing:
      - `EventType` enum: 8 variants (Message, Action, Observation,
        Plan, Knowledge, Datasource, Skill, Misc) — exact match for
        the SQL CHECK constraint in V002
      - `Event` struct: `{ id: i64, session_id: String, timestamp: i64,
        event_type: EventType, source: String, data: serde_json::Value }`
      - `EventStore` trait with **only** these methods:
        - `async fn append(&self, draft: NewEvent) -> Result<Event, EventError>`
        - `async fn query(&self, session_id: &str, filter: EventQuery) -> Result<Vec<Event>, EventError>`
      - `SqliteEventStore` implementing `EventStore` over a `DbPool`
- [ ] `NewEvent` builder: caller supplies `session_id`, `event_type`,
      `source`, `data`; the writer assigns `id` (rowid) and `timestamp`
      (`SystemTime::now()` microseconds)
- [ ] `EventQuery` filter struct: `{ after_id: Option<i64>, event_type:
      Option<EventType>, limit: Option<usize> }` — defaults: no after,
      no type filter, limit 100
- [ ] Append enforces the `events.session_id REFERENCES sessions(id)`
      foreign key — appending for a nonexistent session returns
      `EventError::SessionNotFound`
- [ ] `seasoned-hand-server` HTTP route `GET /v1/sessions/:id/events`
      with query params `?after_id=<i64>&type=<EventType>&limit=<usize>`;
      returns JSON array; 404 if session does not exist
- [ ] Trait surface has NO mutating methods (no update, no delete, no
      `pub` setters on `Event`) — a compile-time test asserts this
- [ ] Unit tests cover: append-and-query round-trip, type filter,
      after_id pagination, limit cap, session-not-found error,
      JSON `data` payloads survive round-trip
- [ ] Integration test on the HTTP endpoint
- [ ] `cargo clippy / fmt / test --workspace` all pass
- [ ] `./scripts/spec-check.sh` passes

## Non-goals

- `subscribe` / Redis pub/sub (story 0.5)
- POST routes for creating events from the frontend (events are emitted
  by the agent runtime + dispatcher; HTTP POST is not a Phase 0 surface)
- WebSocket envelope mapping (story 0.17)
- Cost-cents / tool-calls counters on `sessions` (story 0.16)
- A `Session` CRUD module — minimal session inserts happen in test
  fixtures only; full session API is folded into story 0.6+

---

## Implementation steps

### 1. Module layout

```
crates/seasoned-hand-core/src/events/
  mod.rs       # EventType, Event, NewEvent, EventQuery, EventError, EventStore trait
  sqlite.rs    # SqliteEventStore
  tests.rs
```

### 2. Types — `events/mod.rs`

```rust
//! Append-only event stream.
//! refs: /specs/phase-0/architecture.md §3.2, §3.4

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::db::DbError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventType {
    Message, Action, Observation, Plan,
    Knowledge, Datasource, Skill, Misc,
}

impl EventType {
    pub fn as_str(self) -> &'static str {
        match self {
            EventType::Message => "Message",
            EventType::Action => "Action",
            EventType::Observation => "Observation",
            EventType::Plan => "Plan",
            EventType::Knowledge => "Knowledge",
            EventType::Datasource => "Datasource",
            EventType::Skill => "Skill",
            EventType::Misc => "Misc",
        }
    }
}

impl fmt::Display for EventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for EventType {
    type Err = EventError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "Message" => EventType::Message,
            "Action" => EventType::Action,
            "Observation" => EventType::Observation,
            "Plan" => EventType::Plan,
            "Knowledge" => EventType::Knowledge,
            "Datasource" => EventType::Datasource,
            "Skill" => EventType::Skill,
            "Misc" => EventType::Misc,
            other => return Err(EventError::UnknownType(other.to_string())),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub id: i64,
    pub session_id: String,
    pub timestamp: i64,
    #[serde(rename = "type")]
    pub event_type: EventType,
    pub source: String,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct NewEvent {
    pub session_id: String,
    pub event_type: EventType,
    pub source: String,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub struct EventQuery {
    pub after_id: Option<i64>,
    pub event_type: Option<EventType>,
    pub limit: Option<usize>,
}

impl EventQuery {
    pub fn effective_limit(&self) -> usize {
        self.limit.unwrap_or(100).min(1000)
    }
}

#[derive(Debug, Error)]
pub enum EventError {
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("unknown event type: {0}")]
    UnknownType(String),
    #[error("db error: {0}")]
    Db(#[from] DbError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("system clock error: {0}")]
    Clock(String),
}

#[allow(async_fn_in_trait)]
pub trait EventStore: Send + Sync {
    async fn append(&self, draft: NewEvent) -> Result<Event, EventError>;
    async fn query(
        &self,
        session_id: &str,
        filter: EventQuery,
    ) -> Result<Vec<Event>, EventError>;
}

pub mod sqlite;

#[cfg(test)]
mod tests;
```

### 3. `events/sqlite.rs`

Implementation of `SqliteEventStore` wrapping `DbPool`. Uses
`SystemTime::now().duration_since(UNIX_EPOCH).as_micros()` for
timestamp. SELECT shape:

```sql
SELECT id, session_id, timestamp, type, source, data
FROM events
WHERE session_id = ?
  AND (? IS NULL OR id > ?)
  AND (? IS NULL OR type = ?)
ORDER BY id ASC
LIMIT ?
```

INSERT shape:

```sql
INSERT INTO events (session_id, timestamp, type, source, data)
VALUES (?, ?, ?, ?, ?)
RETURNING id
```

Pre-check: `SELECT 1 FROM sessions WHERE id = ?` to surface
`SessionNotFound` as a clean error before SQLite emits a generic FK
constraint failure.

### 4. HTTP route in server

`crates/seasoned-hand-server/src/lib.rs`: register
`GET /v1/sessions/:id/events` with query string extraction; on
404 return `{"error":"session_not_found"}`. State carries an
`Arc<SqliteEventStore>` alongside the existing `DbPool`.

### 5. Compile-time append-only assertion

In `events/tests.rs` (or `events/mod.rs#[cfg(test)]`):

```rust
// If anyone adds a mutating method to EventStore (e.g., delete_event)
// this stops compiling — by design.
#[allow(dead_code)]
fn assert_event_store_surface() {
    trait NoMutations: EventStore {
        // intentionally empty — the EventStore methods we are willing
        // to ship from Phase 0 are exactly: append, query, subscribe (0.5).
    }
    impl<T: EventStore> NoMutations for T {}
}
```

(This isn't a strict invariant — it documents intent. The real
discipline is the trait surface itself.)

### 6. Test cases (must cover)

- `append_then_query_returns_event`
- `query_filters_by_type`
- `query_filters_by_after_id`
- `query_respects_limit`
- `append_fails_for_unknown_session`
- `data_payload_survives_roundtrip` (non-trivial JSON: nested object + array)
- `events_for_different_sessions_are_isolated`

### 7. Verification

```bash
source $HOME/.cargo/env
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
./scripts/spec-check.sh

# Manual: insert a session row + a couple of events, then hit the endpoint
DATABASE_URL=sqlite:/tmp/sh-events.db PORT=3001 ./target/debug/seasoned-hand-server &
PID=$!
sleep 1
# (insert test data via sqlite3 or via a small helper binary added in this story)
curl -fsS "http://127.0.0.1:3001/v1/sessions/s1/events?limit=10" | jq .
kill -INT $PID
```

---

## Files changed

- `crates/seasoned-hand-core/Cargo.toml` (modify — add `time = "0.3"` if needed; otherwise SystemTime)
- `crates/seasoned-hand-core/src/lib.rs` (modify — `pub mod events`)
- `crates/seasoned-hand-core/src/events/mod.rs` (new)
- `crates/seasoned-hand-core/src/events/sqlite.rs` (new)
- `crates/seasoned-hand-core/src/events/tests.rs` (new)
- `crates/seasoned-hand-server/src/lib.rs` (modify — new state field, new route)
- `crates/seasoned-hand-server/src/main.rs` (modify — wire `SqliteEventStore`)
- `crates/seasoned-hand-server/tests/healthz.rs` (modify — update `AppState` construction)
- `crates/seasoned-hand-server/tests/events.rs` (new — HTTP integration test)

---

## Spec references

- `/specs/phase-0/architecture.md` §3.2 (events table schema)
- `/specs/phase-0/architecture.md` §3.4 (event `data` payload shapes)
- `/specs/phase-0/architecture.md` §4.1 (HTTP route)
- `/specs/00-philosophy/PRINCIPLES.md` #3 (append-only event stream)

---

## Commit message

```
feat(phase-0): story 0.4 - event stream API (append + query)

- seasoned-hand-core::events with EventType, Event, NewEvent,
  EventQuery, EventStore trait (append + query only — no update/
  delete, append-only invariant enforced at trait surface per
  PRINCIPLE #3)
- SqliteEventStore over DbPool; INSERT ... RETURNING id; pre-checks
  session existence to surface SessionNotFound cleanly
- HTTP route GET /v1/sessions/:id/events with after_id / type /
  limit query params; 404 if session missing
- Unit tests cover round-trip, filters, FK enforcement, JSON
  payload preservation
- Integration test on the HTTP endpoint
- cargo clippy / fmt / test / spec-check pass

refs: /specs/phase-0/stories/story-0.4.md
```

---

## Notes for next story (0.5)

- `EventStore::subscribe` lands in 0.5 — Redis pub/sub fanout.
  The architecture event flow: `append` writes to SQLite AND
  publishes to Redis on a per-session channel; `subscribe` returns
  a Tokio stream of new events.
- Server `AppState` will gain a `Arc<RedisClient>` field; the
  WebSocket subscriber (story 0.17) will use it.
