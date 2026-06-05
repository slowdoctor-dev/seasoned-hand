# Story 6.1 — Dioxus foundation scaffold

> **Status**: done

Stand up the `crates/seasoned-hand-ui` Dioxus crate as the foundation for the
ADR-016 migration, compiling to wasm against the existing API.

## Acceptance criteria

- [x] New workspace-excluded crate `crates/seasoned-hand-ui` (Dioxus 0.6, web).
- [x] `dto.rs` mirrors the REST + WS shapes (`api.ts` / `ws-types.ts`).
- [x] `api.rs` REST client (gloo-net) for the `/v1` routes.
- [x] `ws.rs` WebSocket client: connect, subscribe, event buffer, ping/pong,
      reconnect with backoff + subscription replay (ack-await deferred → 6.4).
- [x] RSX components: three-panel shell, project list, task list, chat.
- [x] AgentComputer panel with tab shell + JS-interop wrappers (Monaco/xterm/
      noVNC) mounted via `document::eval` against `index.html` shims.
- [x] `cargo check --target wasm32-unknown-unknown` green (`just check-ui`).
- [x] Backend gates unaffected (UI excluded from root workspace).

## Notes

Foundation only — not full fidelity. Mirror DTOs (→ 6.3), stub interop shims
(→ 6.2), and several richer tabs / the briefing card (→ 6.4) are follow-ups.
