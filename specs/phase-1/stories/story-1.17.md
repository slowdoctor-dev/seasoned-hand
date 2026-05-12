# Story 1.17 — WS `task_pause` / `task_resume` / `task_cancel` real (close DEBT #27)

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 1.10 (VERIFYING state machine — pause/resume/cancel
> guards reference it), 1.9b (Verifier Worker — `handle_request` selects
> against the same cancel token to abort in-flight verification)
> **Phase**: 1
> **Type**: backend
> **Reads first**: `/specs/phase-0/DEBT.md` #27 (origin), `/specs/phase-1/architecture.md`
> §6 row "WebSocket server task control" (pay-down), `/specs/phase-0/stories/story-0.17.md`
> (WS envelope; current stub bodies).

---

## Goal

Replace Phase 0's protocol-stub WebSocket commands `task_pause`,
`task_resume`, `task_cancel` with real implementations: wire per-session
cancellation tokens that `AgentRunner` checks between iterations and at
every `.await`, plus `bollard::pause` / `bollard::unpause` for the
sandbox container.

## Acceptance criteria

- [ ] `AppState::cancel_tokens: DashMap<SessionId, CancellationToken>`
      with `Arc<CancellationToken>` allocated at `AgentRunner::run`
      start and inserted in the map; removed on terminal state
      transition (FINISHED, ERROR).
- [ ] `AgentRunner::run` polls `cancel.is_cancelled()` between
      iterations *and* uses `tokio::select!` against `cancel.cancelled()`
      at every `.await` longer than 100 ms (the LLM call, sandbox HTTP
      calls, sleep loops). On cancellation: emit Misc
      `task_cancelled{reason}`, transition session to `SUSPENDED` (not
      ERROR — cancellation is user-initiated, not failure), return
      `RunResult { completed: false, ... }`.
- [ ] WebSocket command handlers:
      - `task_pause`: looks up the session's sandbox container handle;
        calls `bollard::pause()`; updates `sessions.state` to
        `SUSPENDED`; emits Misc `task_paused`. Returns `{ok:true}`.
        Reject if state ∉ `{RUNNING, VERIFYING}`.
      - `task_resume`: calls `bollard::unpause()`; transitions session
        `SUSPENDED → RUNNING`; emits Misc `task_resumed`; calls
        `AgentRunner::resume(session_id)` (from story 1.10) to rejoin
        the loop. Returns `{ok:true}`. Reject if state ≠ `SUSPENDED`.
      - `task_cancel`: cancels the token; tears down the sandbox
        container (existing Phase 0 destroy path); transitions to
        `FINISHED`. Returns `{ok:true}`. Reject if state ∈
        `{FINISHED, ERROR}`.
- [ ] Cancellation in `VERIFYING`: the Verifier Worker's
      `handle_request` selects against the same cancel token; on cancel,
      emits Misc `verifier_cancelled` and aborts the in-flight LLM
      call. Session state moves to `SUSPENDED` (matches
      `task_cancel` semantics — user-initiated stop, not failure).
- [ ] WebSocket reply envelope unchanged from Phase 0. Errors return
      `{ok:false, error:{code, message}}` with codes: `wrong_state`,
      `unknown_session`, `internal`.
- [ ] DEBT #27 closure in `specs/phase-0/DEBT.md`.
- [ ] Tests:
      - `agent_runner_aborts_on_cancel_between_iterations` —
        wiremock'd LLM in an infinite tool-call loop; cancel; assert
        return within < 500 ms.
      - `task_pause_invokes_bollard_pause` — mock bollard, assert call.
      - `task_resume_invokes_bollard_unpause_and_resumes_loop`.
      - `task_cancel_destroys_container_and_finishes_session`.
      - `pause_rejected_when_finished`.
      - `resume_rejected_when_running`.
      - `verifier_cancel_emits_verifier_cancelled_misc`.

## Non-goals

- Cross-session cancellation cascade.
- Persisting cancel tokens across server restart (the rehydration
  story 1.2 marks restored sessions; if a task was mid-cancel, it
  remains in SUSPENDED until user action).
- Granular pause that suspends only the LLM call but keeps the sandbox
  warm — Phase 1 uses container-level pause for simplicity.
- UI side of pause/resume buttons — frontend toggles are existing
  Phase 0 elements with task-control already wired to these commands
  via the envelope.

## Implementation steps

### 1. Cancel-token registry

```rust
// crates/seasoned-hand-server/src/state.rs
pub cancel_tokens: Arc<DashMap<String, CancellationToken>>,
```

`AgentRunner::run` and `AgentRunner::resume` start with:

```rust
let cancel = CancellationToken::new();
state.cancel_tokens.insert(session_id.clone(), cancel.clone());
```

Removal on terminal state transitions (a helper
`state.cancel_tokens.remove(session_id)`).

### 2. Loop integration

```rust
for step in 0..req.max_steps {
    if cancel.is_cancelled() { break; }
    let messages = self.build_messages(&req.session_id).await?;
    let resp = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Ok(self.cancelled_result(...)),
        r = self.llm.chat_completion(req2) => r?,
    };
    ...
}
```

### 3. WS handler bodies

```rust
async fn handle_task_pause(state: &AppState, session_id: &str) -> WsResponse {
    let s = state.sessions.state(session_id).await?;
    if !matches!(s.as_deref(), Some("RUNNING" | "VERIFYING")) {
        return WsResponse::err("wrong_state");
    }
    let handle = state.sandbox.handle(session_id).ok_or(WsResponse::err("unknown_session"))?;
    state.sandbox.pause(&handle).await?;
    state.sessions.transition(session_id, &s.unwrap(), "SUSPENDED").await?;
    state.events.emit_misc(session_id, "task_paused", json!({})).await?;
    WsResponse::ok(json!({}))
}

async fn handle_task_resume(state: &AppState, session_id: &str) -> WsResponse {
    let s = state.sessions.state(session_id).await?;
    if s.as_deref() != Some("SUSPENDED") { return WsResponse::err("wrong_state"); }
    let handle = state.sandbox.handle(session_id).ok_or(WsResponse::err("unknown_session"))?;
    state.sandbox.unpause(&handle).await?;
    state.sessions.transition(session_id, "SUSPENDED", "RUNNING").await?;
    state.events.emit_misc(session_id, "task_resumed", json!({})).await?;
    state.runner.spawn_resume(session_id).await;
    WsResponse::ok(json!({}))
}

async fn handle_task_cancel(state: &AppState, session_id: &str) -> WsResponse {
    let s = state.sessions.state(session_id).await?;
    if matches!(s.as_deref(), Some("FINISHED" | "ERROR")) {
        return WsResponse::err("wrong_state");
    }
    if let Some(tok) = state.cancel_tokens.get(session_id) { tok.cancel(); }
    state.events.emit_misc(session_id, "task_cancelled", json!({"by":"user"})).await?;
    state.sandbox.destroy_by_session(session_id).await?;
    state.sessions.transition_to(session_id, "FINISHED").await?;
    WsResponse::ok(json!({}))
}
```

### 4. Verifier Worker cancellation

In `verifier::Worker::handle_request`:

```rust
let cancel = state.cancel_tokens.get(&req.session_id).cloned();
let verdict_fut = self.run_verifier_call(...);
match cancel {
    Some(tok) => {
        tokio::select! {
            _ = tok.cancelled() => {
                state.events.emit_misc(&req.session_id, "verifier_cancelled",
                    json!({"trigger_kind": req.trigger.kind_tag()})).await.ok();
                return Ok(());
            }
            r = verdict_fut => { /* normal path */ }
        }
    }
    None => verdict_fut.await,
}
```

### 5. Sandbox pause/unpause

`SandboxClient::pause(handle)` / `unpause(handle)` wrap bollard's
`pause_container` / `unpause_container`. Handle-level (per session)
operations; the handle already exists from Phase 0 lifecycle.

### 6. DEBT close

`specs/phase-0/DEBT.md` #27: strike-through with date + commit ref.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core agent::cancel::
cargo test -p seasoned-hand-server ws::task_control
./scripts/spec-check.sh
```

Live (sandbox required): start a long task, send `{"command":"task_pause",
"session_id":"..."}` over WS; observe container in `docker ps` listed
as paused. Send `task_resume`; container unpauses and the agent loop
continues iteration. Send `task_cancel`; container destroyed, session
state = FINISHED.

---

## Files changed

- `crates/seasoned-hand-server/src/state.rs` (modify — `cancel_tokens`)
- `crates/seasoned-hand-server/src/ws/commands.rs` (modify — real
  bodies for pause/resume/cancel)
- `crates/seasoned-hand-core/src/agent/mod.rs` (modify — cancel
  integration in `run` and `resume`)
- `crates/seasoned-hand-core/src/sandbox/client.rs` (modify — pause /
  unpause / destroy_by_session helpers)
- `crates/seasoned-hand-core/src/verifier/mod.rs` (modify — cancel
  integration in `handle_request`)
- `crates/seasoned-hand-core/src/events/misc.rs` (modify — document
  `task_paused, task_resumed, task_cancelled, verifier_cancelled`)
- `specs/phase-0/DEBT.md` (close #27)

---

## Spec references

- `/specs/phase-1/architecture.md` §6 (pay-down), §3.2 (state
  transitions — SUSPENDED is the target for user-initiated stops).
- `/specs/phase-0/stories/story-0.17.md` (WS envelope).

---

## Commit message

```
fix(phase-1): story 1.17 - WS task_pause/resume/cancel real (DEBT #27)

- AppState::cancel_tokens DashMap<SessionId, CancellationToken>
  populated at AgentRunner::run/resume start, removed on terminal
  transitions; loop body uses tokio::select! against cancellation at
  every long await (LLM call, sandbox HTTP)
- task_pause: bollard::pause + state RUNNING|VERIFYING → SUSPENDED
  + Misc task_paused; rejects from terminal states
- task_resume: bollard::unpause + SUSPENDED → RUNNING + Misc
  task_resumed + AgentRunner::resume; rejects when not SUSPENDED
- task_cancel: cancels the token + destroys the container + state
  to FINISHED + Misc task_cancelled{by:"user"}
- Verifier Worker handle_request selects against the same cancel
  token, emits Misc verifier_cancelled on in-flight cancel
- 7 unit + integration tests

Closes Phase 0 DEBT #27.

refs: /specs/phase-1/stories/story-1.17.md
```

---

## Notes for next story (1.18)

User-initiated task control is now real. The frontend (story 1.18) does
not need to be re-wired to these commands — the Phase 0 WS envelope
already sends them; this story replaces the no-op backend with real
behavior. The new Misc kinds (`task_paused`, `task_resumed`,
`task_cancelled`) flow through the existing event stream — frontend can
render them as system messages without code changes.
