# Story 0.5 — Redis pub/sub for event subscribe

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: story 0.4 (Event Stream API)
> **Phase**: 0
> **Type**: backend
> **Reads first**: `/specs/phase-0/architecture.md` §1 (Event Stream subscribe), §4.1 (`/healthz`), §5.1 (deadpool-redis 0.15), §5.2 (Redis container)

---

## Goal

Wire Redis pub/sub so that every successful `EventStore::append` is
fanned out to subscribers on a per-session channel, and so the server's
WebSocket subscriber (story 0.17) can stream live events. Also tighten
`docker-compose.yml`'s Redis binding to `127.0.0.1` (architecture §9
localhost-only Phase 0 policy).

## Acceptance criteria

- [ ] `docker-compose.yml` Redis service binds `127.0.0.1:6379:6379`
      (not `6379:6379`) — matches the Bifrost service style
- [ ] `seasoned-hand-core::redis` module exposing:
      - `pub async fn open(url: &str) -> Result<RedisPool, RedisError>`
        using deadpool-redis 0.15
      - `RedisPool::ping() -> Result<(), RedisError>` for healthcheck
      - `RedisPool::publish_event(session_id, event_json) -> Result<u64, RedisError>`
        returning the count of subscribers that received the event
      - `RedisPool::subscribe(session_id) -> Result<EventSubscription, RedisError>`
        where `EventSubscription` wraps a Tokio stream
- [ ] Channel naming: `sh:events:<session_id>` (prefix lets us add other
      channels later without collision)
- [ ] `EventStore` trait gains a `subscribe` method:
      `async fn subscribe(&self, session_id: &str) -> Result<EventSubscription, EventError>`
- [ ] `SqliteEventStore::append` publishes to Redis after the SQLite
      INSERT succeeds; **publish failures are logged but do NOT roll back
      the append** (failure-tolerant per PRINCIPLE #10)
- [ ] `/healthz` extended to also ping Redis; response gains
      `redis:"ok"|"unreachable"`; returns 503 if either DB or Redis is down
- [ ] `REDIS_URL` env var (default `redis://localhost:6379`) is read by
      the server binary on start
- [ ] Server `AppState` gains a `redis: RedisPool` field
- [ ] Unit + integration tests cover: ping success, publish-then-receive
      round-trip with two subscribers, subscribe receives only events for
      its session (not others), publish failures do not error `append`
- [ ] **Integration tests use a real Redis** via `testcontainers` (Phase
      0 has no real mock; an alpine-redis container is cheap to spin)
- [ ] `cargo clippy --all-targets -- -D warnings` passes
- [ ] `cargo fmt --check` passes
- [ ] `cargo test --workspace` passes
- [ ] `./scripts/spec-check.sh` passes

## Non-goals

- Redis Streams queue for Module Workers (deferred to Phase 1 per
  architecture §1 component diagram; Phase 0 uses pub/sub only)
- WebSocket integration (story 0.17)
- Authentication for Redis (Phase 5 — `127.0.0.1` bind is sufficient)
- Backpressure / event buffering with bounded channels (story 0.20
  will add reconnect + replay; Phase 0 subscribers either keep up or
  drop oldest)
- Reconnection logic for transient Redis outages (Phase 1 hardening)

---

## Implementation steps

### 1. docker-compose tightening

`docker-compose.yml` Redis service:

```yaml
ports:
  - "127.0.0.1:6379:6379"
```

(Was `"6379:6379"` — exposed on all interfaces. Phase 0 is
localhost-only, ADR-009.)

### 2. `seasoned-hand-core::redis` module

Add to `crates/seasoned-hand-core/Cargo.toml`:

```toml
deadpool-redis = "0.15"
redis = { version = "0.27", features = ["tokio-comp", "aio"] }
futures-util = "0.3"
```

`crates/seasoned-hand-core/src/redis/mod.rs`:

```rust
//! Redis pub/sub for live event fanout.
//! refs: /specs/phase-0/architecture.md §1, §5.1, §5.2

use deadpool_redis::{Config, Connection, Pool, Runtime};
use redis::AsyncCommands;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RedisError {
    #[error("pool error: {0}")]
    Pool(#[from] deadpool_redis::PoolError),
    #[error("redis error: {0}")]
    Redis(#[from] redis::RedisError),
    #[error("config error: {0}")]
    Config(String),
}

#[derive(Clone)]
pub struct RedisPool {
    pool: Pool,
    pub url: String,
}

impl RedisPool {
    pub fn new(url: impl Into<String>) -> Result<Self, RedisError> {
        let url = url.into();
        let cfg = Config::from_url(url.clone());
        let pool = cfg
            .create_pool(Some(Runtime::Tokio1))
            .map_err(|e| RedisError::Config(e.to_string()))?;
        Ok(Self { pool, url })
    }

    async fn conn(&self) -> Result<Connection, RedisError> {
        Ok(self.pool.get().await?)
    }

    pub async fn ping(&self) -> Result<(), RedisError> {
        let mut conn = self.conn().await?;
        let _: String = redis::cmd("PING").query_async(&mut *conn).await?;
        Ok(())
    }

    pub async fn publish_event(&self, session_id: &str, payload: &str) -> Result<i64, RedisError> {
        let mut conn = self.conn().await?;
        let channel = channel_for(session_id);
        let n: i64 = conn.publish(channel, payload).await?;
        Ok(n)
    }

    pub async fn subscribe(&self, session_id: &str) -> Result<EventSubscription, RedisError> {
        // Pub/Sub on the deadpool side needs a dedicated connection (PubSub mode
        // monopolizes the socket). Open a direct connection via the redis crate.
        let client = redis::Client::open(self.url.clone())?;
        let conn = client.get_async_connection().await?;
        let mut pubsub = conn.into_pubsub();
        pubsub.subscribe(channel_for(session_id)).await?;
        Ok(EventSubscription { pubsub })
    }
}

pub fn channel_for(session_id: &str) -> String {
    format!("sh:events:{session_id}")
}

pub struct EventSubscription {
    pubsub: redis::aio::PubSub,
}

impl EventSubscription {
    /// Returns a stream of JSON payloads as &str (caller deserializes
    /// into `Event` if desired). Returns `None` when the channel closes.
    pub fn into_stream(self) -> impl futures_util::Stream<Item = String> {
        use futures_util::StreamExt;
        self.pubsub
            .into_on_message()
            .filter_map(|msg| async move { msg.get_payload::<String>().ok() })
    }
}
```

### 3. `EventStore` trait extension

Add `subscribe` returning a Tokio stream. `SqliteEventStore` gains a
`RedisPool` field constructed via a new builder. The store's `new`
becomes `with_redis`:

```rust
pub struct SqliteEventStore {
    pool: DbPool,
    redis: Option<RedisPool>,    // None for unit tests that don't want Redis
}

impl SqliteEventStore {
    pub fn new(pool: DbPool) -> Self { Self { pool, redis: None } }
    pub fn with_redis(pool: DbPool, redis: RedisPool) -> Self {
        Self { pool, redis: Some(redis) }
    }
}
```

In `append`, after successful INSERT:

```rust
if let Some(redis) = &self.redis {
    let payload = serde_json::to_string(&event)?;
    if let Err(e) = redis.publish_event(&event.session_id, &payload).await {
        tracing::warn!(error = %e, session_id = %event.session_id,
                       "redis publish failed, append succeeded");
    }
}
```

### 4. /healthz update

Server `lib.rs`: add `redis: RedisPool` to `AppState`, ping in `/healthz`,
report combined health.

### 5. Server bootstrap

`main.rs` reads `REDIS_URL` env (default `redis://localhost:6379`).
Bails fast if the URL is invalid; tolerates Redis being down at startup
(server still boots; `/healthz` reports degraded).

### 6. Tests

Use `testcontainers` 0.20+:

```toml
[dev-dependencies]
testcontainers = "0.20"
testcontainers-modules = { version = "0.8", features = ["redis"] }
```

Test cases:
- `pool_ping_works`
- `publish_then_receive_round_trip` (two subscribers, both receive)
- `subscribe_isolates_by_session` (sub for s1 doesn't see s2 events)
- `append_publishes_to_redis` (full round-trip via SqliteEventStore)
- `append_succeeds_even_if_redis_down` (use an invalid URL → expect log,
  no error from append)

If `testcontainers` is unavailable in the dev env (no Docker for tests),
fall back: tests check the publish/subscribe API against a Redis container
the user starts manually with `docker compose up -d redis`, gated by
`#[ignore]` and a `REDIS_TEST_URL` env opt-in. Document the toggle in
the story commit.

### 7. Verification

```bash
docker compose up -d redis        # start Redis if not already running
source $HOME/.cargo/env
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
./scripts/spec-check.sh

# Manual round-trip:
DATABASE_URL=sqlite:/tmp/sh.db PORT=3001 ./target/debug/seasoned-hand-server &
SRV=$!
sleep 1
curl -fsS http://127.0.0.1:3001/healthz | jq '.redis == "ok"'
kill -INT $SRV
```

---

## Files changed

- `docker-compose.yml` (modify — `127.0.0.1:` bind for Redis)
- `crates/seasoned-hand-core/Cargo.toml` (modify — deadpool-redis, redis, futures-util)
- `crates/seasoned-hand-core/src/lib.rs` (modify — `pub mod redis`)
- `crates/seasoned-hand-core/src/redis/mod.rs` (new)
- `crates/seasoned-hand-core/src/redis/tests.rs` (new)
- `crates/seasoned-hand-core/src/events/mod.rs` (modify — `subscribe` trait method)
- `crates/seasoned-hand-core/src/events/sqlite.rs` (modify — Redis publish)
- `crates/seasoned-hand-core/src/events/tests.rs` (modify — publish round-trip)
- `crates/seasoned-hand-server/src/lib.rs` (modify — AppState.redis, /healthz update)
- `crates/seasoned-hand-server/src/main.rs` (modify — open Redis, pass to state)
- `crates/seasoned-hand-server/tests/healthz.rs` (modify — assert redis field)

---

## Spec references

- `/specs/phase-0/architecture.md` §1 (Event Stream subscribe via Redis pub/sub)
- `/specs/phase-0/architecture.md` §4.1 (/healthz combined DB+Redis+Bifrost)
- `/specs/phase-0/architecture.md` §5.1 (deadpool-redis 0.15)
- `/specs/phase-0/architecture.md` §9 (Redis bound to 127.0.0.1 in Phase 0)
- `/specs/00-philosophy/PRINCIPLES.md` #10 (failure-tolerant: redis publish
  failure logs but does not corrupt the append)

---

## Commit message

```
feat(phase-0): story 0.5 - Redis pub/sub for event subscribe

- seasoned-hand-core::redis with RedisPool over deadpool-redis 0.15;
  ping / publish_event / subscribe helpers
- Channel naming: sh:events:<session_id>
- EventStore::subscribe returns a Tokio stream of event JSON payloads
- SqliteEventStore.append publishes to Redis after SQLite INSERT;
  publish errors are logged but do NOT roll back the append
  (PRINCIPLE #10 failure-tolerant)
- /healthz pings Redis; response gains redis:"ok"|"unreachable";
  503 if either DB or Redis down
- REDIS_URL env (default redis://localhost:6379) on server bootstrap
- docker-compose Redis bound to 127.0.0.1 (Phase 0 localhost-only)
- Tests via testcontainers (round-trip, isolation, failure-tolerance)
- cargo clippy / fmt / test / spec-check all pass

refs: /specs/phase-0/stories/story-0.5.md
```

---

## Notes for next story (0.6)

- Both persistence and pub/sub are in place; story 0.6 (Tool trait + 5
  simplest tools) starts the agent runtime surface
- `EventStore` is now the unified write path: every tool dispatch in
  later stories emits Action + Observation via this trait
