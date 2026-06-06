# Phase 6 — Architecture (Dioxus migration + release)

> Governed by [ADR-016](../01-architecture/decisions/ADR-016-dioxus-unified-rust-frontend.md)
> (frontend swap) and ARCHITECTURE.md §1.1 (v1.5). This file is the phase-level
> architect artifact; ADR-016 holds the decision rationale and migration plan.

## 1. Scope and invariants

Phase 6 replaces the frontend layer **only**. The control plane (Axum + Tokio +
Rig), Bifrost, the sandbox, persistence, and the entire `/v1` REST + `/ws`
WebSocket contract are **unchanged** (ADR-016; NFR-6.2). The boundary is the
contract: the new UI is just a different client of the same API.

```
┌─────────────────────────────────────────────┐
│ seasoned-hand-ui (Dioxus, Rust → wasm/native)│   replaces frontend/ (Next.js)
│  - RSX components (3-panel console)           │
│  - api.rs (gloo-net) ──────────┐              │
│  - ws.rs (gloo-net websocket) ─┤              │
│  - interop.rs: Monaco/xterm/noVNC via         │
│    document::eval → index.html shims          │
└────────────────────────────────┼─────────────┘
                                  │  /v1 REST + /ws  (UNCHANGED)
                                  ▼
                 Control Plane (Rust) — unchanged
```

## 2. Crate topology

- `crates/seasoned-hand-ui` — the Dioxus app. **Excluded from the root
  workspace** (root `Cargo.toml`): it targets `wasm32` and pulls
  wasm-bindgen/web-sys, which do not build for the native host, so excluding it
  keeps `cargo clippy --all-targets` / `cargo test --workspace` native-only and
  green (NFR-6.3). Built via `dx` or `cargo build --target wasm32-unknown-unknown`.
- `crates/seasoned-hand-dto` (story 6.3) — wasm-safe pure-serde DTOs shared by
  the server and the UI. A **member** of the root workspace (it is wasm-safe).
  Until it lands, the UI mirrors DTOs in `seasoned-hand-ui/src/dto.rs`.

## 3. State and data flow

- Shared state (`Selection`: active project/task/session + `AgentSocket`) lives
  in the `App` root and is distributed via Dioxus **context** (replaces the
  `HomeShell` prop-drilling). Signals are `Copy` handles.
- REST reads use `use_resource` keyed on the relevant signal (e.g. the task list
  re-fetches when `active_project` changes).
- The event stream is a `use_coroutine` owning the WebSocket: it pushes
  `ServerEvent`s into a `Signal<Vec<ServerEvent>>` (capped at 1000), replies to
  pings, and reconnects with backoff + subscription replay. Ack-correlated
  command results are reintroduced in story 6.4.

## 4. The interop boundary (the load-bearing risk)

Monaco, xterm.js, and noVNC have no native Rust equivalent. Each is owned by a
Rust wrapper component (`interop.rs`) that renders a stable mount `<div id>` and
calls a `window.__mount*` shim (defined in `index.html`, which loads the vendor
bundles) through `document::eval`. On web and desktop (webview) the same shims
work; on mobile these surfaces degrade to read-only. Story 6.2 is the explicit
go/no-go gate that proves this before the full port (ADR-016 step 1).

## 5. Sequencing

Stories 6.1 (done) → 6.2 (interop gate) → 6.3 (shared DTOs) → 6.4 (full
fidelity + acks) → 6.5 (desktop/mobile) → 6.6 (cutover, delete Next.js) → 6.7
(release). See `requirements.md` and ADR-016's migration plan.
