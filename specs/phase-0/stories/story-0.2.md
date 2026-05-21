# Story 0.2 — Rust workspace initialization (Cargo + Axum hello)

> **Status**: done
> **Estimated**: 1 hour
> **Dependencies**: story 0.1 (Bifrost running)
> **Phase**: 0
> **Type**: infrastructure
> **Reads first**: `/specs/phase-0/architecture.md` §2 (workspace layout) + §4.1 (HTTP routes) + §5.1 (Rust crate versions)

---

## Goal

Stand up the two-crate Rust workspace defined in architecture §2 with a
minimal Axum HTTP server bound to `127.0.0.1:3000` exposing a `/healthz`
endpoint. After this story the repo has a real Rust build that compiles,
runs, and answers `curl http://localhost:3000/healthz`. No agent logic,
no DB, no WebSocket yet — those are stories 0.3+.

## Acceptance criteria

- [ ] `/Cargo.toml` defines a workspace listing both member crates
- [ ] `crates/seasoned-hand-core/Cargo.toml` exists (library crate, edition 2024)
- [ ] `crates/seasoned-hand-server/Cargo.toml` exists (binary crate, edition 2024)
- [ ] `crates/seasoned-hand-core/src/lib.rs` exposes a `version()` function
      returning the cargo-package version string
- [ ] `crates/seasoned-hand-server/src/main.rs` boots an Axum 0.7 server on
      `${HOST:-127.0.0.1}:${PORT:-3000}` reading from `.env` if present
- [ ] `GET /healthz` returns HTTP 200 with JSON body
      `{"status":"ok","version":"<x.y.z>"}` — version sourced from
      `seasoned-hand-core::version()`
- [ ] `cargo build --workspace` succeeds with **zero warnings** on a clean target
- [ ] `cargo clippy --all-targets -- -D warnings` passes (`AGENTS.md` §6 gate)
- [ ] `cargo fmt --check` passes
- [ ] `cargo test --workspace` passes (at minimum: one integration test that
      hits `/healthz` and asserts the JSON)
- [ ] `scripts/spec-check.sh` continues to pass
- [ ] `tracing-subscriber` configured with `EnvFilter` reading `RUST_LOG`
      (see `.env.example` line `RUST_LOG=agent_os=debug,tower_http=debug` —
      update that line if the crate name differs from `agent_os`; the binary
      is `seasoned-hand-server`)
- [ ] Graceful shutdown via `tokio::signal::ctrl_c` (no orphaned port)
- [ ] `.gitignore` includes `target/`

## Non-goals

- WebSocket endpoint (story 0.17)
- SQLite / Redis (stories 0.3, 0.5)
- Agent runtime, tool dispatcher (stories 0.6+)
- Frontend (stories 0.18+)
- Configuration via YAML — `.env` + env vars only in this story
- `ts-rs` codegen (deferred to Phase 1 per architecture §2)

---

## Implementation steps

### 1. Workspace root

`/Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = [
    "crates/seasoned-hand-core",
    "crates/seasoned-hand-server",
]

[workspace.package]
edition = "2024"
license = "Apache-2.0"
authors = ["Seasoned Hand contributors"]
repository = "https://github.com/slowdoctor-dev/seasoned-hand"

[workspace.dependencies]
axum            = "0.7"
tokio           = { version = "1.40", features = ["full"] }
tracing         = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
serde           = { version = "1", features = ["derive"] }
serde_json      = "1"
thiserror       = "1"
```

### 2. Core library crate

`crates/seasoned-hand-core/Cargo.toml`:

```toml
[package]
name = "seasoned-hand-core"
version = "0.1.0"
edition.workspace = true
license.workspace = true

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
```

`crates/seasoned-hand-core/src/lib.rs`:

```rust
//! Seasoned Hand core library.
//! refs: /specs/phase-0/architecture.md §2

/// Returns the workspace version (sourced from the server crate's Cargo metadata
/// in later stories). For story 0.2 we hard-link to this crate's version, which
/// matches the workspace.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
```

### 3. Server binary crate

`crates/seasoned-hand-server/Cargo.toml`:

```toml
[package]
name = "seasoned-hand-server"
version = "0.1.0"
edition.workspace = true
license.workspace = true

[dependencies]
seasoned-hand-core = { path = "../seasoned-hand-core" }
axum = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }

[dev-dependencies]
tokio = { workspace = true }
reqwest = { version = "0.12", features = ["json"] }
```

`crates/seasoned-hand-server/src/main.rs`:

```rust
//! Seasoned Hand server binary.
//! refs: /specs/phase-0/architecture.md §4.1

use axum::{routing::get, Json, Router};
use serde::Serialize;
use std::net::SocketAddr;
use tracing_subscriber::EnvFilter;

#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
}

async fn healthz() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: seasoned_hand_core::version(),
    })
}

fn app() -> Router {
    Router::new().route("/healthz", get(healthz))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let host = std::env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(3000);
    let addr: SocketAddr = format!("{host}:{port}").parse()?;

    tracing::info!(%addr, "seasoned-hand-server starting");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app())
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutdown signal received");
}
```

### 4. Integration test

`crates/seasoned-hand-server/tests/healthz.rs`:

```rust
//! refs: /specs/phase-0/architecture.md §4.1

use axum::http::StatusCode;
use tokio::net::TcpListener;

#[tokio::test]
async fn healthz_returns_ok() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = axum::Router::new().route("/healthz", axum::routing::get(super::healthz_handler));
    // For story 0.2 simplicity: spin the same router by re-declaring; in story
    // 0.17 we extract a public `app()` helper. Implementer may instead make
    // `app()` `pub` in main.rs and import it here — either is fine.
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let resp = reqwest::get(format!("http://{addr}/healthz")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
    assert!(body["version"].is_string());
}
```

Implementer note: the cleanest path is to expose `app()` from `main.rs`
as `pub fn app()` and `pub use` it in a `lib.rs` for the server crate.
Either approach is acceptable for story 0.2 — keep it simple.

### 5. `.gitignore`

Append `target/` if not already present.

### 6. Verify with `just`

`just verify` should pass end-to-end. If `just` isn't installed in the
shell (Codex hit this with story 0.1), document the manual fallback in
this story's commit:

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
./scripts/spec-check.sh
```

---

## Verification

```bash
cargo build --workspace
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
./scripts/spec-check.sh

# Manual: start the server, hit it.
cargo run --bin seasoned-hand-server &
sleep 1
curl -fsS http://127.0.0.1:3000/healthz | jq -e '.status == "ok"'
kill %1
```

Expected: all green; `jq` returns `true`.

---

## Files changed

- `Cargo.toml` (new — workspace root)
- `crates/seasoned-hand-core/Cargo.toml` (new)
- `crates/seasoned-hand-core/src/lib.rs` (new)
- `crates/seasoned-hand-server/Cargo.toml` (new)
- `crates/seasoned-hand-server/src/main.rs` (new)
- `crates/seasoned-hand-server/tests/healthz.rs` (new)
- `.gitignore` (modify if `target/` missing)
- `Cargo.lock` (new, auto-generated)

---

## Spec references

- `/specs/phase-0/architecture.md` §2 (workspace layout, 2-crate decision)
- `/specs/phase-0/architecture.md` §4.1 (HTTP routes — `/healthz` is the first one)
- `/specs/phase-0/architecture.md` §5.1 (pinned Rust crate versions)
- `/AGENTS.md` §6 (verification gates)
- `/AGENTS.md` §7 (Rust code style: edition 2024, zero clippy warnings, no `unwrap()` in non-test code, `thiserror` for errors)

---

## Commit message

```
feat(phase-0): story 0.2 - Rust workspace + Axum healthz

- 2-crate workspace per architecture §2: seasoned-hand-core (lib),
  seasoned-hand-server (bin)
- Axum 0.7 server on 127.0.0.1:3000, /healthz returns
  {status:"ok", version:"<x.y.z>"}
- tracing-subscriber + EnvFilter for structured logs
- Graceful shutdown on Ctrl-C
- Integration test asserts /healthz response
- cargo clippy / fmt / test / spec-check all pass

refs: /specs/phase-0/stories/story-0.2.md
```

---

## Notes for next story (0.3)

- Workspace exists; story 0.3 (SQLite schema + migrations) adds the
  `db/` module and `migrations/` directory at workspace root.
- `seasoned-hand-server/src/main.rs` will gain a `db_pool` constructor
  used to bootstrap rusqlite + refinery on start.
- `/healthz` will be extended in story 0.4 to report DB and Redis
  readiness (per architecture §4.1 "200 if SQLite, Redis, Bifrost reachable").
