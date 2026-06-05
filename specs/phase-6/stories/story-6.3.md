# Story 6.3 — Extract wasm-safe `seasoned-hand-dto` shared crate

> **Status**: ready

Deliver the ADR-016 "shared DTOs, no duplication" promise. The UI currently
mirrors core's DTOs in `seasoned-hand-ui/src/dto.rs` because `seasoned-hand-core`
pulls in rusqlite/bollard/tokio and cannot compile to wasm.

## Acceptance criteria

- [ ] New workspace crate `crates/seasoned-hand-dto`: pure serde types
      (Project, Task, Deliverable, SessionSummary, WS envelopes/commands), zero
      I/O dependencies, compiles to both native and `wasm32`.
- [ ] `seasoned-hand-core` (and server) re-export / depend on `-dto` for these
      shapes instead of defining their own.
- [ ] `seasoned-hand-ui` depends on `-dto` and deletes its mirror `dto.rs`.
- [ ] `-dto` is a member of the root workspace (it is wasm-safe); the UI crate
      stays excluded.
- [ ] All gates green (`just verify` + `just check-ui`).
