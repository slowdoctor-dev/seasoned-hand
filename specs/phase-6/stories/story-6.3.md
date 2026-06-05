# Story 6.3 — Extract wasm-safe `seasoned-hand-dto` shared crate

> **Status**: done

Deliver the ADR-016 "shared DTOs, no duplication" promise. The UI previously
mirrored core's DTOs in `seasoned-hand-ui/src/dto.rs` because `seasoned-hand-core`
pulls in rusqlite/bollard/tokio and cannot compile to wasm.

## Acceptance criteria

- [x] New workspace crate `crates/seasoned-hand-dto`: pure serde types
      (Project, Task, Deliverable, SessionSummary/State/Detail, Sandbox,
      TaskDeliverablesResponse, WS envelopes/commands), zero I/O dependencies,
      compiles to both native and `wasm32`.
- [x] `seasoned-hand-core` re-exports the domain entities (Project, Task,
      Deliverable + ProjectStatus/TaskStatus + `legal_transitions`) from `-dto`
      instead of defining its own; the DB-string mapping (`as_db_str`/
      `from_db_str`) moves to `-dto`, with `From<EnumParseError>` lifting into
      `ProjectError`/`TaskError` (call sites unchanged).
- [x] `seasoned-hand-ui` depends on `-dto` and deletes its mirror `dto.rs`.
- [x] `-dto` is a member of the root workspace; the UI crate stays excluded.
- [x] Gates green: `cargo check --workspace`, `clippy --all-targets -D warnings`,
      `fmt --check`, `spec-check` 10/10, and `just check-ui` (wasm).

## Follow-up — story 6.3b (server-side adoption)

> **Status**: done (session/deliverable types) — WS protocol types deferred to 6.3c

The server now uses `-dto`'s `SessionSummary` / `Sandbox` / `SessionDetail` /
`TaskDeliverablesResponse` directly (its private copies removed). `SessionSummary.state`
is now the `-dto` `SessionState` enum; the DB `state` String is mapped via
`SessionState::from_db_str` (DB values IDLE/RUNNING/FINISHED/ERROR/SUSPENDED
confirmed to match the UPPERCASE variants). Gates green (workspace check, clippy
`-D warnings`, fmt, wasm UI check).

### Story 6.3c (WS protocol types)

> **Status**: done (ServerEnvelope) — inbound CommandPayload intentionally not unified

The server now serializes the shared `seasoned_hand_dto::ServerEnvelope` (its
private copy removed); `-dto`'s `Ack`/`Error` optional fields gained
`skip_serializing_if` so the emitted JSON is byte-identical. The server→client
half of the protocol is now shared end-to-end.

The **inbound** `ClientEnvelope` / `CommandPayload` (+ `BriefingActionTag`) stay
server-local **by design**: they diverge from the UI's send-only mirror —
`TaskPause` carries a `durable` flag, `BriefingConfirm` uses a typed
`BriefingActionTag` + `PartialBrief edits` (a core type) for deserialize-time
dispatch. The UI's simpler send shape remains wire-compatible (server fields are
`#[serde(default)]`). Unifying the inbound side would pull core types into `-dto`
for no functional gain, so it is deliberately left split.
