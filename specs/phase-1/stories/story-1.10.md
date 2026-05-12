# Story 1.10 — TaskComplete trigger + VERIFYING state + verdict handling

> **Status**: ready
> **Estimated**: 3 hours
> **Dependencies**: 1.9b (Verifier Worker runtime; emits
> `verifier_verdict` Misc), 1.1 (PlanManager accepts `Source::Verifier`
> updates)
> **Phase**: 1
> **Type**: backend
> **Reads first**: `/specs/phase-1/architecture.md` §2.4.2 trigger A
> (TaskComplete), §2.4.5 (verdict handling table), §3.2 (state
> transitions), `/specs/phase-1/stories/story-1.9.md` (handler API).

---

## Goal

Wire the first of three Verifier triggers: when the agent calls `idle` or
`message_notify_user` with `final:true`, suspend the session into a new
`VERIFYING` state, push a `VerifyRequest::TaskComplete` onto Redis
Streams, and act on the verdict — `pass` → `FINISHED`, `fail` with
suggestion → Verifier-driven `plan_update` + `RUNNING`, `fail` without
suggestion → `SUSPENDED`. After this story, the happy-path of a Phase 1
task is end-to-end verifiable.

## Acceptance criteria

- [ ] `idle` and `message_notify_user` tools accept a new optional
      `final: bool` argument (default `false`). `idle` is implicitly
      final by Phase 0 contract; for backwards compatibility, treat
      `idle` invocations as `final:true` always.
- [ ] When the dispatcher dispatches `idle` (any args) or
      `message_notify_user` with `final:true`, the runtime transitions
      the session `RUNNING → VERIFYING`, emits Misc `verifier_request{
      trigger:"TaskComplete", final_message_call_id}`, and pushes a
      `VerifyRequest` onto Redis Streams `verify_request`.
- [ ] AgentRunner pauses iteration when state is `VERIFYING`. The
      `AgentRunner::run` returns with `RunResult { completed: false,
      ... }` immediately after the trigger; a separate `VerifierGate`
      Tokio task awaits the verdict (subscribes to Misc events) and
      transitions the session.
- [ ] Verdict-handling state transitions:
      | Verdict | Action |
      |---|---|
      | `pass` | `VERIFYING → FINISHED`; deliver agent's final message; emit Misc `task_complete` |
      | `fail` with `suggested_plan_update` | Verifier Worker calls `PlanManager::update(phases, Source::Verifier)`; session `VERIFYING → RUNNING`; AgentRunner is re-invoked to resume the loop |
      | `fail` without suggestion | `VERIFYING → SUSPENDED` with reason carried in the existing `session.last_error` column or a new `Misc{kind:"task_suspended_by_verifier"}` event |
- [ ] Re-invocation of the agent after `RUNNING` resumption: the
      `VerifierGate` calls back into `AgentRunner::resume(session_id)`
      which constructs a fresh `RunRequest` (using the session's
      original `max_steps`/`cost_cap_cents`) but continues the iteration
      counter — does **not** re-run the Initializer.
- [ ] Verifier-Worker code path (story 1.9 `handle_request`) gains:
      - On `verdict=pass` for TaskComplete trigger: emit Misc
        `verifier_verdict` only. State transition is the
        VerifierGate's job.
      - On `verdict=fail` + suggestion: call `PlanManager::update`
        directly *before* emitting `verifier_verdict`, so the
        downstream gate sees a fresh plan.
- [ ] Misc events tagged `verifier_verdict` carry the
      `verification_id` so the gate can join back to the row.
- [ ] Tests:
      - `idle_call_pushes_verify_request_and_transitions_to_verifying`
        — wiremock'd Bifrost returns one `idle` call; assert Redis
        Streams `XLEN verify_request == 1` and session state ==
        `VERIFYING`.
      - `verdict_pass_transitions_to_finished` — feed canned verdict;
        assert state == `FINISHED` and a `task_complete` Misc event.
      - `verdict_fail_with_suggestion_resumes_with_new_plan` — canned
        verdict carries `suggested_plan_update.phases`; assert
        `PlanManager::snapshot` reflects the new plan and state ==
        `RUNNING` and Plan event has `source:"verifier"`.
      - `verdict_fail_without_suggestion_suspends` — state ==
        `SUSPENDED`.
      - `message_notify_user_without_final_does_not_trigger` —
        `final:false` (or absent) emits no `verify_request`.
      - `resume_continues_iteration_counter` — verify the resumed run
        does not re-initialize and the iteration counter picks up
        where it left off.

## Non-goals

- Invalidation trigger (story 1.11) — separate trigger source.
- CircuitBreaker trigger (story 1.12).
- Frontend rendering of VERIFYING state — story 1.18 surfaces the
  verdict pane; the chat pane treats VERIFYING as a non-input state.
- `task_resume` from the user side while VERIFYING — out of scope
  (architecture §4.1 admin endpoint refuses rollback while RUNNING; this
  story preserves Phase 0 task_resume semantics).
- Automatic rollback on `fail` — opt-in per architecture §2.6 +
  phase-1/DEBT.md #3; default off and not exercised by this story.

## Implementation steps

### 1. Tool changes (`idle` + `message_notify_user`)

```rust
// crates/seasoned-hand-core/src/tools/idle.rs
// schema unchanged; dispatch body now emits a VERIFYING transition
async fn dispatch(...) -> ToolOutput {
    ctx.session_runtime.mark_task_complete(&ctx.session_id, /* call_id */).await?;
    ToolOutput::ok(json!({"final": true}))
}

// crates/seasoned-hand-core/src/tools/message_notify_user.rs
async fn dispatch(...) -> ToolOutput {
    let final_: bool = args.get("final").and_then(Value::as_bool).unwrap_or(false);
    if final_ {
        ctx.session_runtime.mark_task_complete(&ctx.session_id, /* call_id */).await?;
    }
    // existing message delivery body
    ToolOutput::ok(...)
}
```

`SessionRuntime::mark_task_complete` does the state transition, the Misc
event, and the Redis Streams push. Implemented in
`crates/seasoned-hand-core/src/agent/runtime.rs`:

```rust
pub async fn mark_task_complete(&self, session_id: &str, call_id: &str) -> Result<()> {
    self.sessions.transition(session_id, "RUNNING", "VERIFYING").await?;
    let event_id = self.events.emit_misc(session_id, "verifier_request", json!({
        "trigger": "TaskComplete",
        "final_message_call_id": call_id,
    })).await?;
    let req = VerifyRequest {
        session_id: session_id.into(),
        trigger: VerifyTrigger::TaskComplete { final_message_call_id: call_id.into() },
        triggered_at_event_id: event_id,
        context_hint: VerifyContextHint::default(),
    };
    self.redis.xadd_json("verify_request", &req).await?;
    Ok(())
}
```

### 2. AgentRunner loop change

After dispatching `idle` or `message_notify_user{final:true}`, the runner
inspects session state. If `VERIFYING`, return immediately:

```rust
let new_state = self.sessions.state(&req.session_id).await?;
if new_state == "VERIFYING" {
    return Ok(RunResult { session_id: req.session_id, completed: false,
        last_message: None, steps: step + 1 });
}
```

### 3. VerifierGate (state-transition listener)

```
crates/seasoned-hand-core/src/verifier/gate.rs
```

A long-running Tokio task spawned at startup (alongside the Verifier
Worker). Subscribes to `Misc{kind:"verifier_verdict"}` events from the
event stream (Phase 0 Redis pub/sub `subscribe(session_id)` is unsuitable
for cross-session — use a global Misc subscriber, or a new Redis Pub/Sub
channel `verifier_verdict_global` that the Worker publishes alongside
its DB INSERT).

```rust
pub async fn run_gate(state: AppState, shutdown: CancellationToken) {
    let mut rx = state.events.subscribe_misc_global("verifier_verdict").await;
    while !shutdown.is_cancelled() {
        let Some(ev) = rx.recv().await else { break };
        let trigger = ev.data.get("trigger_kind").and_then(Value::as_str);
        let verdict = ev.data.get("verdict").and_then(Value::as_str);
        match (trigger, verdict) {
            (Some("TaskComplete"), Some("pass")) => {
                state.sessions.transition(&ev.session_id, "VERIFYING", "FINISHED").await.ok();
                state.events.emit_misc(&ev.session_id, "task_complete", json!({})).await.ok();
            }
            (Some("TaskComplete"), Some("fail")) => {
                if ev.data.get("suggested_plan_update").is_some() {
                    // Worker already called PlanManager::update; just resume the agent.
                    state.sessions.transition(&ev.session_id, "VERIFYING", "RUNNING").await.ok();
                    state.runner.spawn_resume(&ev.session_id).await.ok();
                } else {
                    state.sessions.transition(&ev.session_id, "VERIFYING", "SUSPENDED").await.ok();
                    state.events.emit_misc(&ev.session_id, "task_suspended_by_verifier", json!({
                        "reason": ev.data.get("reason"),
                    })).await.ok();
                }
            }
            _ => {} // Invalidation / CircuitBreaker handled in stories 1.11/1.12
        }
    }
}
```

`subscribe_misc_global` is a new helper if not already present; it
subscribes to the existing Redis pub/sub channel pattern used by the
event store. If a per-session subscription is the only Phase 0 idiom,
add a global channel: when `EventStore::emit_misc` is called with
`kind` starting with `verifier_`, also `PUBLISH verifier_misc_global`.

### 4. AgentRunner::resume

```rust
pub async fn resume(&self, session_id: &str) -> Result<RunResult, AgentError> {
    self.sessions.transition(session_id, "RUNNING", "RUNNING").await.ok(); // no-op state confirm
    let mut iteration_counter = self.sessions.iteration_counter(session_id).await?;
    // Loop body identical to `run`, minus the Initializer call.
    loop { /* think → act → observe, incrementing iteration_counter */ }
}
```

Add a column or per-session counter for iterations if Phase 0 didn't
already persist one. The existing `events` table can derive iteration
count by counting Action events for the session, so adding an in-memory
counter inside `AgentRunner` keyed by `session_id` is sufficient.

### 5. Verifier Worker callback for suggested plan_update

In `verifier::handle_request` (story 1.9), after parsing the verdict:

```rust
if matches!(verdict.verdict, VerdictKind::Fail) {
    if let Some(spu) = &verdict.suggested_plan_update {
        // Verifier IS authorized to mutate the plan.
        state.plan_manager.update(
            &req.session_id,
            spu.phases.clone(),
            PlanMutationSource::Verifier,
        ).await?;
    }
}
let verification_id = persistence::insert(state, &req, &verdict).await?;
state.events.emit_misc(&req.session_id, "verifier_verdict", json!({
    "verdict": verdict.verdict,
    "reason": verdict.reason,
    "evidence_event_ids": verdict.evidence_event_ids,
    "suggested_plan_update": verdict.suggested_plan_update,
    "verification_id": verification_id,
    "trigger_kind": req.trigger.kind_tag(),  // "TaskComplete" / "Invalidation" / "CircuitBreaker"
})).await?;
```

### 6. Test harness

Tests live in `crates/seasoned-hand-core/src/verifier/tests.rs` and
`agent/tests.rs`. Use wiremock'd Bifrost for both the `main` slot (drives
the agent loop) and the `verifier` slot (drives canned verdicts). Use a
real local Redis under `#[ignore]`-by-default integration tests; provide
pure-unit alternatives that hand-call `VerifierGate::run_gate` against a
synthesized Misc event stream.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core verifier::trigger::task_complete
cargo test -p seasoned-hand-core verifier::gate::tests
cargo test -p seasoned-hand-core agent::tests::task_complete_to_verifying
./scripts/spec-check.sh
```

---

## Files changed

- `crates/seasoned-hand-core/src/tools/idle.rs` (modify — emit VERIFYING)
- `crates/seasoned-hand-core/src/tools/message_notify_user.rs` (modify
  — `final` flag)
- `crates/seasoned-hand-core/src/agent/runtime.rs` (new — `SessionRuntime`
  with `mark_task_complete`, holds `events`+`sessions`+`redis` handles)
- `crates/seasoned-hand-core/src/agent/mod.rs` (modify — `AgentRunner::resume`,
  early return on VERIFYING)
- `crates/seasoned-hand-core/src/verifier/gate.rs` (new)
- `crates/seasoned-hand-core/src/verifier/mod.rs` (modify — `pub mod
  gate;`, expose `Worker::handle_request` for the call into PlanManager)
- `crates/seasoned-hand-server/src/main.rs` (modify — spawn `VerifierGate`
  alongside Worker)
- `crates/seasoned-hand-core/src/events/store.rs` (modify if a global
  Misc subscription channel is added)
- `crates/seasoned-hand-core/src/events/misc.rs` (modify — document
  `task_complete, task_suspended_by_verifier`)

---

## Spec references

- `/specs/phase-1/architecture.md` §2.4.2 trigger A, §2.4.5 (verdict
  table — verbatim semantics), §3.2 (state transitions), §9
  (Verifier-as-actuator), §12 q8 (event-id stability after plan_update).

---

## Commit message

```
feat(phase-1): story 1.10 - TaskComplete trigger + VERIFYING + verdict handling

- idle and message_notify_user{final:true} mark the task complete:
  RUNNING → VERIFYING, Misc verifier_request{trigger:"TaskComplete"},
  VerifyRequest pushed onto Redis Streams "verify_request"
- AgentRunner returns immediately after a task-complete dispatch when
  the new session state is VERIFYING; resume() rejoins the loop on
  fail-with-suggestion verdicts (preserves iteration counter; no
  Initializer re-run)
- VerifierGate Tokio task subscribes to verifier_verdict Misc events
  and applies the §2.4.5 transition table:
    pass               → VERIFYING → FINISHED + Misc task_complete
    fail+suggestion    → VERIFYING → RUNNING (Verifier already wrote
                          the new plan via PlanManager::update with
                          source=Verifier in handle_request) + resume
    fail without sug.  → VERIFYING → SUSPENDED + Misc
                          task_suspended_by_verifier
- Verifier worker (story 1.9 handle_request) now calls
  PlanManager::update before emitting verifier_verdict on
  fail-with-suggestion verdicts
- 6 tests cover the four verdict paths plus the no-trigger negative

refs: /specs/phase-1/stories/story-1.10.md
```

---

## Notes for next story (1.11)

The Verifier Worker now has its first live trigger. Story 1.11 adds the
Invalidation Detector + Invalidation trigger. The VerifierGate's `match`
arms already pattern-match against `Invalidation`/`CircuitBreaker`
trigger kinds (currently noops); 1.11 only needs to add the emission
side + the Gate's behavior on those verdicts (which is mostly: emit Misc
event, do not transition session — the agent loop continues unchanged).
