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

> **Status**: ready

The **server** still defines its own `SessionSummary` / `SandboxInfo` /
`SessionDetail` / `TaskDeliverablesResponse` and the WS envelope/command types in
`ws.rs` (wire-compatible with `-dto`). 6.3b makes the server emit/consume the
`-dto` types directly so those shapes are shared end-to-end too. Note: the
server's `SessionSummary.state` is currently a `String`; adoption switches it to
`-dto`'s `SessionState` enum (verify the emitted values match the UPPERCASE
variants).
