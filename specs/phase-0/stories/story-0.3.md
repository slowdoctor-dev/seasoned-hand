# Story 0.3 — SQLite schema + migrations (sessions, events, plans)

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: story 0.2 (Rust workspace + Axum healthz)
> **Phase**: 0
> **Type**: backend
> **Reads first**: `/specs/phase-0/architecture.md` §3 (Data model changes) + §4.1 (`/healthz`) + §5.1 (rusqlite + refinery versions)

---

## Goal

Add SQLite persistence to the Rust workspace per architecture §3.5:
a `db` module that opens a WAL-mode connection, runs embedded migrations
on startup, and exposes a connection pool. Phase 0 ships three tables —
`sessions`, `events`, `plans` — exactly matching architecture §3.1–§3.3.
After this story, `/healthz` is extended to verify the DB is reachable
(per architecture §4.1 "200 if SQLite, Redis, Bifrost reachable" — Redis
+ Bifrost reachability lands in 0.5 + 0.27).

## Acceptance criteria

- [ ] `/migrations/` directory with three numbered files:
      `001_sessions.sql`, `002_events.sql`, `003_plans.sql` —
      schemas **byte-identical** to architecture.md §3.1–§3.3
- [ ] `seasoned-hand-core::db` module exposing:
      - `pub async fn open(database_url: &str) -> Result<DbPool, DbError>`
      - `pub fn run_migrations(conn: &mut Connection) -> Result<(), DbError>`
      - a thin `DbPool` newtype (Phase 0: `Arc<Mutex<Connection>>` is
        sufficient — single-writer is fine for Phase 0; concrete
        connection pooling is a Phase 1 enhancement)
- [ ] `PRAGMA journal_mode = WAL` set on connection open; assertion
      that the pragma returned `wal` (not silently downgraded)
- [ ] `PRAGMA foreign_keys = ON` set on connection open
- [ ] `refinery` v0.8 embedded migrations (`embed_migrations!()`) so the
      SQL files ship inside the binary
- [ ] Migrations idempotent: running on an already-migrated DB is a no-op
- [ ] `seasoned-hand-server` `main.rs` calls `db::open` at startup using
      `DATABASE_URL` env (default `sqlite:./data/seasoned-hand.db`),
      creates the parent directory if missing, and bails fast with a
      tracing error if migrations fail
- [ ] `/healthz` returns `{"status":"ok","version":"…","db":"ok"}` when
      the DB is reachable, `{"status":"degraded","version":"…","db":"<err>"}`
      with HTTP 503 if the DB ping fails
- [ ] `data/` directory added to `.gitignore`
- [ ] Unit tests cover: connection open creates the file; WAL pragma
      asserted; migrations apply cleanly to a fresh in-memory DB;
      re-running migrations is idempotent; tables `sessions`, `events`,
      `plans` exist with the expected columns
- [ ] Integration test extends `tests/healthz.rs` (or new
      `tests/healthz_with_db.rs`) to assert `/healthz` reports `db:"ok"`
- [ ] `cargo clippy --all-targets -- -D warnings` passes
- [ ] `cargo fmt --check` passes
- [ ] `cargo test --workspace` passes
- [ ] `./scripts/spec-check.sh` passes

## Non-goals

- Redis (story 0.5)
- Event API (`append`, `query`, `subscribe`) — story 0.4 builds on this
- `EventStore` trait (story 0.4)
- Plan Manager (folded into stories 0.6 + 0.14)
- Multi-tenant `user_id` enforcement (Phase 5 — column exists, NULL allowed)
- Learning tables (`sops`, `playbooks`, `playbooks_fts`, `glossary`) —
  Phase 3+, not added here

---

## Implementation steps

### 1. Migrations

`/migrations/001_sessions.sql` — verbatim from architecture §3.1:

```sql
CREATE TABLE sessions (
  id            TEXT PRIMARY KEY,
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL,
  state         TEXT NOT NULL CHECK(state IN
                  ('IDLE','RUNNING','FINISHED','ERROR','SUSPENDED')),
  project_id    TEXT,
  user_id       TEXT,
  title         TEXT,
  cost_cents    INTEGER NOT NULL DEFAULT 0,
  tool_calls    INTEGER NOT NULL DEFAULT 0,
  metadata      TEXT
);
CREATE INDEX idx_sessions_state ON sessions(state);
```

`/migrations/002_events.sql` — verbatim from architecture §3.2:

```sql
CREATE TABLE events (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id    TEXT NOT NULL REFERENCES sessions(id),
  timestamp     INTEGER NOT NULL,
  type          TEXT NOT NULL CHECK(type IN
                  ('Message','Action','Observation','Plan',
                   'Knowledge','Datasource','Skill','Misc')),
  source        TEXT NOT NULL,
  data          TEXT NOT NULL
);
CREATE INDEX idx_events_session_time ON events(session_id, timestamp);
CREATE INDEX idx_events_type ON events(type);
```

`/migrations/003_plans.sql` — verbatim from architecture §3.3:

```sql
CREATE TABLE plans (
  id                TEXT PRIMARY KEY,
  session_id        TEXT NOT NULL REFERENCES sessions(id),
  goal              TEXT NOT NULL,
  phases            TEXT NOT NULL,
  current_phase_id  INTEGER NOT NULL,
  created_at        INTEGER NOT NULL,
  updated_at        INTEGER NOT NULL
);
CREATE INDEX idx_plans_session ON plans(session_id);
```

### 2. `seasoned-hand-core::db` module

Add to `crates/seasoned-hand-core/Cargo.toml`:

```toml
rusqlite = { version = "0.31", features = ["bundled", "serde_json"] }
refinery = { version = "0.8", features = ["rusqlite"] }
tokio = { workspace = true }
thiserror = { workspace = true }
```

`crates/seasoned-hand-core/src/db/mod.rs`:

```rust
//! SQLite persistence.
//! refs: /specs/phase-0/architecture.md §3

use std::path::Path;
use std::sync::Arc;

use rusqlite::Connection;
use thiserror::Error;
use tokio::sync::Mutex;

refinery::embed_migrations!("../../migrations");

#[derive(Debug, Error)]
pub enum DbError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("migration error: {0}")]
    Migration(#[from] refinery::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid database url: {0}")]
    InvalidUrl(String),
    #[error("WAL not enabled: returned {0}")]
    WalNotEnabled(String),
}

#[derive(Clone)]
pub struct DbPool {
    inner: Arc<Mutex<Connection>>,
}

impl DbPool {
    pub async fn with_conn<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Connection) -> R,
    {
        let mut conn = self.inner.lock().await;
        f(&mut *conn)
    }
}

/// Opens a SQLite connection at the given URL (sqlite:./path or :memory:),
/// sets WAL mode and foreign keys, runs migrations.
pub async fn open(database_url: &str) -> Result<DbPool, DbError> {
    let conn = if database_url == ":memory:" || database_url == "sqlite::memory:" {
        Connection::open_in_memory()?
    } else {
        let path = database_url
            .strip_prefix("sqlite:")
            .unwrap_or(database_url);
        if let Some(parent) = Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        Connection::open(path)?
    };

    set_pragmas(&conn)?;
    let mut conn = conn;
    run_migrations(&mut conn)?;

    Ok(DbPool {
        inner: Arc::new(Mutex::new(conn)),
    })
}

fn set_pragmas(conn: &Connection) -> Result<(), DbError> {
    let mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    if mode.to_lowercase() != "wal" && mode.to_lowercase() != "memory" {
        return Err(DbError::WalNotEnabled(mode));
    }
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    Ok(())
}

pub fn run_migrations(conn: &mut Connection) -> Result<(), DbError> {
    migrations::runner().run(conn)?;
    Ok(())
}

#[cfg(test)]
mod tests;
```

`crates/seasoned-hand-core/src/db/tests.rs`:

```rust
use super::*;

#[tokio::test]
async fn opens_in_memory_db_and_runs_migrations() {
    let pool = open(":memory:").await.expect("open in-memory db");
    pool.with_conn(|conn| {
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for required in ["events", "plans", "sessions"] {
            assert!(tables.contains(&required.to_string()), "missing table {required}");
        }
    })
    .await;
}

#[tokio::test]
async fn migrations_are_idempotent() {
    let pool = open(":memory:").await.unwrap();
    pool.with_conn(|conn| run_migrations(conn).expect("idempotent re-run"))
        .await;
}

#[tokio::test]
async fn opens_file_db_and_creates_parent_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("nested/dir/test.db");
    let url = format!("sqlite:{}", path.display());
    let _pool = open(&url).await.expect("open file db");
    assert!(path.exists(), "db file was not created");
}
```

`tempfile` as dev-dependency in core's Cargo.toml:

```toml
[dev-dependencies]
tempfile = "3"
tokio = { workspace = true, features = ["macros", "rt"] }
```

### 3. Re-export from `lib.rs`

`crates/seasoned-hand-core/src/lib.rs`:

```rust
//! Seasoned Hand core library.
//! refs: /specs/phase-0/architecture.md §2

pub mod db;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
```

### 4. Extend `/healthz`

`crates/seasoned-hand-server/src/lib.rs` (Phase 0 simplification — the
DB pool is passed as state):

```rust
//! Seasoned Hand HTTP server.
//! refs: /specs/phase-0/architecture.md §4.1

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use seasoned_hand_core::db::DbPool;
use serde::Serialize;

#[derive(Clone)]
pub struct AppState {
    pub db: DbPool,
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
    db: String,
}

async fn healthz(State(state): State<AppState>) -> impl IntoResponse {
    let db_ok = state
        .db
        .with_conn(|conn| conn.query_row("SELECT 1", [], |_| Ok(())).is_ok())
        .await;
    let (status_code, status_text, db_text) = if db_ok {
        (StatusCode::OK, "ok", "ok".to_string())
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "degraded", "unreachable".to_string())
    };
    (
        status_code,
        Json(Health {
            status: status_text,
            version: seasoned_hand_core::version(),
            db: db_text,
        }),
    )
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .with_state(state)
}
```

`crates/seasoned-hand-server/src/main.rs`:

```rust
//! refs: /specs/phase-0/architecture.md §4.1

use std::net::SocketAddr;

use seasoned_hand_core::db;
use seasoned_hand_server::{AppState, app};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite:./data/seasoned-hand.db".to_string());
    let db = db::open(&database_url).await?;

    let addr = bind_addr()?;
    tracing::info!(%addr, "seasoned-hand-server starting");

    let state = AppState { db };
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

fn bind_addr() -> Result<SocketAddr, std::net::AddrParseError> {
    let host = std::env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = std::env::var("PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(3000);
    format!("{host}:{port}").parse()
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::warn!(%error, "failed to listen for shutdown signal");
    }
    tracing::info!("shutdown signal received");
}
```

### 5. Integration test update

`crates/seasoned-hand-server/tests/healthz.rs`: rewire to construct an
in-memory `DbPool`, build the `Router` via `app(state)`, and assert
`/healthz` body now includes `db:"ok"`.

### 6. `.gitignore`

Add `data/` (already partially covered? Verify and add if missing).

---

## Verification

```bash
source $HOME/.cargo/env
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
./scripts/spec-check.sh

# Manual: start the server, hit healthz
PORT=3001 ./target/debug/seasoned-hand-server &
sleep 1
curl -fsS http://127.0.0.1:3001/healthz | jq -e '.db == "ok"'
kill %1
```

Expected: all green.

---

## Files changed

- `crates/seasoned-hand-core/Cargo.toml` (modify — add rusqlite, refinery, tokio, tempfile dev)
- `crates/seasoned-hand-core/src/lib.rs` (modify — `pub mod db`)
- `crates/seasoned-hand-core/src/db/mod.rs` (new)
- `crates/seasoned-hand-core/src/db/tests.rs` (new)
- `crates/seasoned-hand-server/src/lib.rs` (modify — AppState, /healthz with DB)
- `crates/seasoned-hand-server/src/main.rs` (modify — open DB, pass state)
- `crates/seasoned-hand-server/tests/healthz.rs` (modify — in-memory DB)
- `migrations/001_sessions.sql` (new)
- `migrations/002_events.sql` (new)
- `migrations/003_plans.sql` (new)
- `.gitignore` (modify — add `data/`)

---

## Spec references

- `/specs/phase-0/architecture.md` §3 (Data model changes — exact SQL)
- `/specs/phase-0/architecture.md` §3.5 (Migrations — refinery)
- `/specs/phase-0/architecture.md` §4.1 (`/healthz` reports DB readiness)
- `/specs/phase-0/architecture.md` §5.1 (rusqlite 0.31, refinery 0.8)
- `/AGENTS.md` §6 (verification gates)

---

## Commit message

```
feat(phase-0): story 0.3 - SQLite schema + WAL migrations

- 3 migration files (sessions/events/plans) verbatim from
  architecture §3.1–§3.3
- seasoned-hand-core::db: open() sets PRAGMA journal_mode=WAL +
  foreign_keys=ON, runs refinery embedded migrations, returns a
  DbPool newtype (Arc<Mutex<Connection>>, single-writer is enough
  for Phase 0)
- server main.rs opens DB at startup (DATABASE_URL env, default
  sqlite:./data/seasoned-hand.db); fails fast on migration error
- /healthz extended to ping DB; returns 503 + db:"unreachable"
  when DB down, 200 + db:"ok" otherwise
- Unit tests: in-memory open + migrations, idempotency,
  parent-dir creation on file URLs
- Integration test asserts /healthz reports db:"ok"
- cargo clippy / fmt / test / spec-check pass

refs: /specs/phase-0/stories/story-0.3.md
```

---

## Notes for next story (0.4)

- DbPool exists; story 0.4 (Event Stream API) wraps it with an
  `EventStore` trait having only `append` and `query` methods (no
  update/delete — append-only invariant enforced at trait level per
  architecture.md §3.2)
- `subscribe` lands in story 0.5 (Redis pub/sub)
- The `DbPool::with_conn` closure-based API is intentionally
  conservative for Phase 0; if contention becomes visible in story
  0.27 E2E, switch to `r2d2` or `deadpool-sqlite` in Phase 1
