# Story 2.8b — IntakeRouter spawner wiring (DEBT #13 + #15 close-out)

> **Status**: complete
> **Estimated**: 2–3 hours
> **Dependencies**: 2.5, 2.8
> **Phase**: 2
> **Type**: backend
> **Reads first**: `/specs/phase-2/architecture.md` §2.2, §2.7, §2.8, §4

---

## Goal

Close Phase 2 DEBT #13 (`IntakeRouter` stops after creating the drafted
Task) and DEBT #15 (WS `task_create` still spawns the runner directly).
Story 2.8 landed `Initializer::run_with_confirmation` as a self-contained
surface; this story plugs it into the live intake path so chat-,
webhook-, and email-originated tasks actually flow end-to-end through
the briefing-confirm gate.

## Acceptance criteria

- [x] New `InitializerSpawner` trait in
      `crates/seasoned-hand-core/src/intake/spawner.rs` carrying
      `spawn(SpawnSpec) -> SpawnReceipt`. Trait lives in core so the
      router can call it; production impl lives in server.
- [x] `IntakeRouter` holds a `OnceLock<Arc<dyn InitializerSpawner>>`
      attached via `attach_initializer_spawner(...)`. After
      `handle_event(...)` Creates a task, the router invokes the
      spawner and surfaces `session_id` on `HandleOutcome::Created`.
      Spawner errors are non-fatal (drafted task + intake row stay
      persisted).
- [x] `AppState::briefing_senders: Arc<DashMap<String,
      mpsc::Sender<UserResponse>>>` keyed by `task_id`. Inserted by
      the spawner at gate-start, removed by the spawner's tokio task
      after the gate returns (Started / Cancelled / error).
- [x] `WsInitializerSpawner` (server-side) inserts the sessions row
      synchronously, registers the briefing sender, then fires a
      tokio task that runs `Initializer::run_with_confirmation`
      followed (on `Started`) by `AgentRunner::resume(req)`. The
      synchronous part returns `SpawnReceipt { session_id }` so the
      WS Ack contract is preserved.
- [x] WS `task_create` handler no longer inserts the sessions row or
      spawns `AgentRunner::run` directly. It builds the `IntakeEvent`
      with the pre-allocated session id in `metadata.session_id_hint`
      and pushes through `state.intake_router.handle_event(...)`.
- [x] WS `briefing_confirm` cmd handler: looks up the per-task sender
      in `briefing_senders`, forwards the `UserResponse`, Acks
      `ok=true` on success or `ok=false, error="no_pending_briefing"`
      when no sender is registered.
- [x] Core unit tests: spawner invoked + `session_id` reflected;
      spawner error tolerated.
- [x] Integration tests:
      - `task_create_returns_session_id_and_starts_runner` (kept,
        verifies session row exists post-Ack).
      - `ws_task_create_emits_briefing_then_confirm_acks_started`
        (new — full happy path).
      - `ws_briefing_confirm_unknown_task_acks_not_pending` (new —
        error surface).
- [x] DEBT #13 + #15 closed with resolution notes; new DEBT entries
      filed for loose call_id match (#20), non-chat briefing
      forwarding (#21), `#[ignore]` tests that don't yet send
      `briefing_confirm` (#22).

## Non-goals

- Strict `in_reply_to_call_id` enforcement inside the confirm gate
  (DEBT #20).
- Non-chat briefing forwarding (DEBT #21).
- Updating `#[ignore]` e2e / phase1_gaia tests to send
  `briefing_confirm` (DEBT #22).
- A new `task_id` field on the WS `Ack` envelope — the briefing
  events carry it via the existing Misc payload.

---

## Files changed

- `crates/seasoned-hand-core/src/intake/spawner.rs` (new)
- `crates/seasoned-hand-core/src/intake/mod.rs` (re-exports)
- `crates/seasoned-hand-core/src/intake/router.rs` (OnceLock spawner +
  attach + invoke; `Created.session_id`)
- `crates/seasoned-hand-core/src/intake/tests.rs` (2 new tests)
- `crates/seasoned-hand-server/src/initializer_spawner.rs` (new —
  `WsInitializerSpawner`)
- `crates/seasoned-hand-server/src/lib.rs` (briefing_senders field,
  attach helper, register_channel rebuild)
- `crates/seasoned-hand-server/src/ws.rs` (BriefingConfirm variant +
  handler; task_create refactor)
- `crates/seasoned-hand-server/tests/ws.rs` (2 new tests + commentary)
- `specs/phase-2/DEBT.md` (close #13 / #15; add #20 / #21 / #22)

---

## Spec references

- `/specs/phase-2/architecture.md` §2.2 (Briefing protocol)
- `/specs/phase-2/architecture.md` §2.7 (Channel framework)
- `/specs/phase-2/architecture.md` §2.8 (Intake protocol — "spawns
  Initializer with the brief_input")
- `/specs/phase-2/architecture.md` §4 (WS envelope additions —
  `briefing_confirm` cmd shape)

---

## Commit message

```
feat(phase-2): story 2.8b - IntakeRouter→Initializer wiring close-out (DEBT #13 + #15)

- New `InitializerSpawner` trait in `seasoned-hand-core/intake`; server-side
  `WsInitializerSpawner` impl inserts the sessions row, registers a
  per-task `mpsc::Sender<UserResponse>` in `AppState::briefing_senders`,
  and fires a tokio task that runs `Initializer::run_with_confirmation`
  → `AgentRunner::resume(req)`.
- `IntakeRouter` holds `OnceLock<Arc<dyn InitializerSpawner>>`; attached
  from `AppState::new` and re-attached after every `register_channel`.
  `HandleOutcome::Created` carries `session_id: Option<String>` so the
  caller can ack synchronously.
- WS `task_create` handler stops inserting sessions rows / spawning
  `AgentRunner::run` directly. New `briefing_confirm` cmd handler
  forwards `UserResponse` envelopes into the per-task mpsc; unknown
  tasks ack with `error: "no_pending_briefing"`.
- 2 new core unit tests + 2 new WS integration tests.
- DEBT #13 + #15 → CLOSED; new entries #20 (loose call_id match), #21
  (non-chat briefing forwarding), #22 (#[ignore] tests need confirm).

refs: /specs/phase-2/stories/story-2.8b.md
```
