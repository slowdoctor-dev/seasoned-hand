# Story 0.10 — Hooks (PreToolUse + PostToolUse + PostToolUseFailure)

> **Status**: done
> **Estimated**: 2 hours
> **Dependencies**: story 0.4 (events), story 0.9 (dispatcher + hook scaffold)
> **Phase**: 0
> **Type**: backend
> **Reads first**: `/specs/phase-0/architecture.md` §4.3 (hooks), `/specs/00-philosophy/PRINCIPLES.md` #3 (append-only), #10 (failure-tolerant), #11 (audit trail)

---

## Goal

Implement the `EventEmittingHook` body that wraps every tool dispatch
with append-only events:

- **PreToolUse** (before invoke) → emit an `Action` event with
  `{ tool, args, call_id }`
- **PostToolUse** (on success) → emit an `Observation` event with
  `{ call_id, ok: true, output, file_ref? }`
- **PostToolUseFailure** (on error) → emit an `Observation` event
  with `{ call_id, ok: false, error: { kind, message } }` —
  PRINCIPLE #10: failures are preserved, never silently retried

After this story, every tool dispatch in the system writes an
auditable trail: each call gets a generated `call_id` (UUID v4) that
links the Action to its Observation. Pays down DEBT.md item #20.

## Acceptance criteria

- [ ] `dispatch/hooks.rs` adds `EventEmittingHook { events: Arc<SqliteEventStore> }`
      implementing `Hook` trait
- [ ] `pre` generates a UUID v4 `call_id`, stashes it in
      `ToolContext` somewhere the post/failure callbacks can read it
- [ ] Action event source = `tool:<name>`, type = `EventType::Action`,
      data = `{tool: <name>, args: <Value>, call_id: <uuid>}`
- [ ] Observation event source = `tool:<name>`, type =
      `EventType::Observation`, data = `{call_id, ok, output?, error?, file_ref?}`
- [ ] `call_id` propagation between `pre` and `post`/`failure`:
      Phase 0 uses a thread-local / task-local registry keyed on
      `(session_id, tool_name)`. Simpler alternative: ToolContext gains
      a `call_id_slot: Arc<Mutex<Option<String>>>` field that the
      dispatcher resets per call. Implementer picks the cleanest path.
- [ ] `AppState::new` wires `EventEmittingHook` into the dispatcher
      automatically. Tests that don't want event emission can build a
      raw `ToolDispatcher::new(registry)` with no hooks.
- [ ] Hook failures (e.g., event-store append fails) are **logged
      with tracing::warn but do NOT short-circuit the tool dispatch** —
      this is PRINCIPLE #10 again
- [ ] Output truncation: if `output` serialized exceeds 16 KB (per
      architecture §3.4), the hook writes the full body to
      `/workspace/.observations/<call_id>.txt` via `sandbox_post` AND
      stores `{output: <1KB preview>, file_ref: <path>}` in the event.
      **Phase 0 simplification**: if no sandbox is ready or the write
      fails, fall back to inline truncation with a `...<truncated>` marker.
- [ ] Unit tests:
      - `pre_emits_action_event_with_args_and_call_id`
      - `post_emits_observation_event_linked_by_call_id`
      - `failure_emits_observation_event_with_error_payload`
      - `hook_failures_do_not_break_dispatch` (use a closed DB to make
        the hook's append fail; assert dispatch still returns the tool's
        ToolOutput unchanged)
      - `large_output_writes_file_ref` (the truncation path; can be
        partially mocked since full sandbox isn't running)
- [ ] `cargo clippy --all-targets -- -D warnings` passes
- [ ] `cargo fmt --check` passes
- [ ] `cargo test --workspace` passes
- [ ] `./scripts/spec-check.sh` passes
- [ ] DEBT.md item #20 is resolved (struck-through with date)

## Non-goals

- L1 deterministic verification (re-read tool output to confirm it
  happened) — that lands with story 0.14 agent runner
- L2 cross-source verification (Phase 1)
- L4 meta-cognition verifier (Phase 1, slot needs LLM)
- Frontend WebSocket emission from hook (story 0.17)
- Hook configuration via env / runtime registration (Phase 1)
- Removing the hooks from tests (each test still constructs the
  dispatcher it wants)

---

## Implementation steps

### 1. `call_id` propagation

Add `call_id_slot: Arc<tokio::sync::Mutex<Option<String>>>` to
`ToolContext`. The `ToolDispatcher::dispatch` method generates a UUID
per call, sets the slot before calling `pre`, clears after `post`/
`failure`. Hooks read the slot to learn the current call's id.

Simpler alternative considered: pass `call_id` as an argument to `pre`/
`post`/`failure`. This is cleaner — change the `Hook` trait signature.
**Picked**: add `call_id: &str` to the trait methods. Reason: explicit
beats threading state via a mutex (PRINCIPLE #9 "explicit over implicit").

### 2. Updated trait

```rust
#[async_trait]
pub trait Hook: Send + Sync {
    async fn pre(&self, call_id: &str, name: &str, args: &Value, ctx: &ToolContext);
    async fn post(&self, call_id: &str, name: &str, args: &Value, output: &ToolOutput, ctx: &ToolContext);
    async fn failure(&self, call_id: &str, name: &str, args: &Value, err: &ToolError, ctx: &ToolContext);
}
```

`NoopHook` updates to match. `ToolDispatcher::dispatch` generates a
`uuid::Uuid::new_v4().to_string()` per call, passes to all hooks.

### 3. `EventEmittingHook`

```rust
pub struct EventEmittingHook {
    pub events: Arc<SqliteEventStore>,
}

#[async_trait]
impl Hook for EventEmittingHook {
    async fn pre(&self, call_id, name, args, ctx) {
        let _ = self.events.append(NewEvent {
            session_id: ctx.session_id.clone(),
            event_type: EventType::Action,
            source: format!("tool:{name}"),
            data: json!({ "tool": name, "args": args, "call_id": call_id }),
        }).await
        .map_err(|e| tracing::warn!(error = %e, "hook pre append failed"));
    }
    // post + failure similar
}
```

Large-output handling: serialize `output` to bytes; if > 16 KB, replace
with `{output: <first 1024 chars>, truncated: true}` and add a debt
entry if file_ref upload to sandbox isn't wired yet.

### 4. Dispatcher wiring

`ToolDispatcher::dispatch` body:

```rust
let call_id = uuid::Uuid::new_v4().to_string();
for h in &self.hooks { h.pre(&call_id, tool_name, &args, ctx).await; }
let result = tool.invoke(args.clone(), ctx).await;
match result {
    Ok(out) => {
        for h in &self.hooks { h.post(&call_id, tool_name, &args, &out, ctx).await; }
        out
    }
    Err(err) => {
        for h in &self.hooks { h.failure(&call_id, tool_name, &args, &err, ctx).await; }
        wrap_err(&err)
    }
}
```

### 5. `AppState::new` wires the hook

```rust
let dispatcher = Arc::new(
    ToolDispatcher::new(register_builtin_tools())
        .with_hook(Arc::new(EventEmittingHook { events: events.clone() })),
);
```

### 6. Add `uuid` to `seasoned-hand-core/Cargo.toml`

```toml
uuid = { version = "1", features = ["v4"] }
```

### 7. Tests

- New test file `dispatch/hook_tests.rs` (or fold into `dispatch/tests.rs`)
- Each test builds a dispatcher with `EventEmittingHook` registered, runs
  one tool call, asserts the event store has the expected Action +
  Observation events with linked `call_id`
- For the "hook failures don't break dispatch" test: simulate by passing
  an `EventStore` impl that returns Err. Easiest: create a tiny
  `BrokenEventStore` mock or pass a DbPool that's been closed. Probably
  pick a dispatcher with no real hooks for the rest of dispatch tests
  to avoid noise.

---

## Files changed

- `crates/seasoned-hand-core/Cargo.toml` (add `uuid = { version = "1", features = ["v4", "serde"] }`)
- `crates/seasoned-hand-core/src/dispatch/hooks.rs` (add `EventEmittingHook`, update trait sig)
- `crates/seasoned-hand-core/src/dispatch/mod.rs` (call_id per dispatch, pass to hooks)
- `crates/seasoned-hand-core/src/dispatch/tests.rs` (new hook tests)
- `crates/seasoned-hand-server/src/lib.rs` (`AppState::new` registers hook)
- `specs/phase-0/DEBT.md` (item #20 resolved with date)

---

## Spec references

- `/specs/phase-0/architecture.md` §4.3 (hooks)
- `/specs/00-philosophy/PRINCIPLES.md` #3 (append-only), #10 (failure-tolerant), #11 (audit trail)
- `/specs/phase-0/architecture.md` §3.4 (event data shapes — Observation
  with `call_id` link to Action)

---

## Commit message

```
feat(phase-0): story 0.10 - EventEmittingHook (Pre/Post/Failure)

- Hook trait gains explicit call_id parameter (PRINCIPLE #9 explicit
  over implicit; rejected mutex-slot alternative)
- ToolDispatcher::dispatch generates UUID v4 per call, threads it
  through pre / post / failure
- EventEmittingHook emits:
  - Action event before invoke: {tool, args, call_id}
  - Observation event on success: {call_id, ok:true, output, file_ref?}
  - Observation event on failure: {call_id, ok:false, error}
- Hook append failures logged via tracing::warn but never break
  tool dispatch (PRINCIPLE #10)
- AppState::new registers EventEmittingHook into the dispatcher
- uuid v4 dep added to seasoned-hand-core
- Tests: Pre/Post/Failure event emission, call_id linkage, hook
  failure tolerance
- cargo clippy / fmt / test / spec-check all pass

Debt: closes DEBT.md item #20 (hooks were unregistered after 0.9).

refs: /specs/phase-0/stories/story-0.10.md
```

---

## Notes for next story (0.11)

- Audit trail now exists: every dispatch produces 2 events (Action
  + Observation)
- Story 0.11 (LLM client) consumes the agent loop's tool_call response
  and dispatches via `state.dispatcher`; the events created here flow
  naturally into the agent loop's "Layer 3 observation analysis"
  (architecture §6)
