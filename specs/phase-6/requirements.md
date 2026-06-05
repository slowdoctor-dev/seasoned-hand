# Phase 6 — Open Source Release + Dioxus Migration — Requirements

> Status: CURRENT (started 2026-06-05). See `/specs/06-roadmap/ROADMAP.md` and
> [ADR-016](../01-architecture/decisions/ADR-016-dioxus-unified-rust-frontend.md).

Phase 6 has two intertwined goals:

1. **Frontend migration (ADR-016)** — replace the Next.js + React + TypeScript
   app with a unified-Rust **Dioxus** frontend targeting Web / Desktop / Mobile
   from one codebase, against the unchanged `/v1` REST + `/ws` WebSocket boundary.
2. **Open-source release** — external users can install and run in < 30 minutes
   (polished docs, one-command deploy, CI/CD, community channel).

A **release-readiness checklist** (perf-track seal, Docker-host test pass, doc
reconciliation, hardening-track confirmation, open-decision resolution — see
ROADMAP) runs in parallel and **gates the public-release tag**, not the start of
Phase 6 work.

## Functional requirements (migration)

- **F-6.1** The Dioxus UI reproduces the 3-panel operator console
  (Projects/Tasks · Chat · AgentComputer) and the briefing/approval flow.
- **F-6.2** Monaco, xterm.js, and noVNC are embedded on web + desktop via JS
  interop; the mobile AgentComputer degrades to a read-only status/log view.
- **F-6.3** DTOs are shared (not duplicated) between server and UI via a
  wasm-safe `seasoned-hand-dto` crate.
- **F-6.4** The WebSocket client matches `lib/ws.ts` behaviour: reconnect with
  backoff, subscription replay, ping/pong, and ack-correlated command results.
- **F-6.5** Cutover removes `frontend/` (Next.js) once parity is verified.

## Non-functional

- **NFR-6.1** UI builds reproducibly via the Dioxus CLI (`dx`); CI produces the
  wasm bundle.
- **NFR-6.2** No change to the control plane, Bifrost, or sandbox — frontend swap
  only (ADR-016).
- **NFR-6.3** `cargo check --target wasm32-unknown-unknown` is green for the UI
  crate (gate; `just check-ui`). Backend gates remain native-only and green (the
  UI crate is excluded from the root workspace).

## Stories

| # | Story | Status |
|---|---|---|
| 6.1 | Dioxus foundation scaffold | done |
| 6.2 | Interop spike — real Monaco/xterm/noVNC bundles (ADR-016 step-1 gate) | in-progress (drafted; verify needs Docker) |
| 6.3 | Extract wasm-safe `seasoned-hand-dto` shared crate (core + UI) | done |
| 6.3b | Server-side adoption of `-dto` (session/deliverable types) | done |
| 6.3c | Server-side adoption of `-dto` (ServerEnvelope; inbound stays local) | done |
| 6.4 | Full-fidelity port + ack handling (acks/briefing/deliverables done) | in-progress |
| 6.5 | Desktop + mobile targets | ready |
| 6.6 | Cutover — remove Next.js, compiled Tailwind, docker/justfile/docs | ready |
| 6.7 | Release-readiness — docs, one-command deploy, CI/CD, community + checklist | ready |
