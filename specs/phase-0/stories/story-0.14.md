# Story 0.14 — Agent runner (ReAct loop, one-tool-per-iteration)

> **Status**: done
> **Estimated**: 4 hours
> **Dependencies**: story 0.4 (events), 0.6 (Tool trait), 0.9 (dispatcher), 0.10 (hooks), 0.11 (LLM client), 0.12 (slot router), 0.13 (capabilities)
> **Phase**: 0
> **Type**: backend
> **Reads first**: `/specs/01-architecture/ARCHITECTURE.md` §4 (agent loop, one tool per iteration), `/specs/phase-0/architecture.md` §1 + §4.3, ADR-010 (Plan as PCB)

---

## Goal

Wire the actual ReAct loop: think (LLM call) → act (dispatch tool) →
observe (read event stream) → repeat. This is the heart of Phase 0
and the biggest single story in the phase. After this story, a
hand-crafted test can drive a real task end-to-end against Bifrost
(with cloud keys) using `info_search_web` + `idle`.

## Acceptance criteria

- [ ] `seasoned-hand-core::agent` module with:
      - `AgentRunner` struct holding handles to LLM client, dispatcher,
        event store, slot router
      - `RunRequest { session_id, input: String, max_steps: u32, cost_cap_cents: Option<u32> }`
      - `RunResult { session_id, completed: bool, last_message: Option<String>, steps: u32 }`
      - `async fn run(&self, req: RunRequest) -> Result<RunResult, AgentError>`
- [ ] Plan create at task start: emit a single-phase `Plan` event with
      the user input as the goal. Full Plan Manager (multi-phase,
      structured updates) is folded into this story per the requirements
      doc; ADR-010's `plan_create / plan_advance / plan_update` tools are
      already registered (story 0.7) so the agent can call them, but the
      runtime also seeds a baseline plan
- [ ] `tool_choice: required` enforced via `LlmClient`
- [ ] ONE tool per iteration (architecture §4 HARD constraint): if the
      LLM returns multiple `tool_calls`, only the first is dispatched
      and a warning event is emitted with the rest
- [ ] The LLM is given the **full registered tool spec list** built
      from `dispatcher.registry()` schemas
- [ ] Sticky context per iteration (ADR-010): build messages from
      (system prompt, plan snapshot, recent events in chronological
      order). Phase 0 uses raw event records as messages; pretty
      formatting is Phase 1
- [ ] Termination conditions (any one ends the loop):
      - `idle` tool called → `completed: true`
      - `max_steps` reached → `completed: false` + Misc event
        `{kind: "max_steps_reached"}`
      - `cost_cap_cents` exceeded → `completed: false` + Misc event
        `{kind: "cost_cap"}`  (cost lookup is story 0.16; Phase 0
        runner accepts the cap arg but the check is a no-op until 0.16)
      - LLM call returns Status error 4 times in a row → ERROR
- [ ] `message_ask_user` pause path: when the dispatcher returns
      `{ok:true, output:{call_id, ...}}` for a tool whose source is
      `tool:message_ask_user`, the runner suspends the session
      (state = SUSPENDED) and returns control to the caller. Resume on
      WebSocket `user_response` lands in story 0.17; for now, expose a
      synchronous `resume()` helper for tests
- [ ] Stuck detection scaffold: track the last assistant message hash;
      if 2+ identical responses in a row, inject a "strategy change"
      system message and continue. Real implementation is story 0.15
      (already split out); this story just wires the hash comparison
- [ ] AgentError variants:
      - `Llm` (LlmError)
      - `Db` (event-store / session table)
      - `Cancelled` (cooperative cancel via a `tokio::sync::watch` token)
      - `Internal(String)`
- [ ] `AppState` exposes `runner: Arc<AgentRunner>` (built in `new`)
- [ ] Unit tests using a mocked `LlmClient` (wiremock) to simulate
      multi-turn conversations:
      - `single_turn_idle_completes` — LLM returns one tool_call to
        `idle` on turn 1; assert RunResult.completed=true, steps=1
      - `multi_turn_search_then_idle` — turn 1 returns
        `info_search_web` call; runner dispatches (search returns
        `missing_api_key` since BRAVE_API_KEY unset in tests; runner
        records the Observation and proceeds); turn 2 returns
        `message_notify_user` then `idle`; complete in 3 steps
      - `one_tool_per_iteration_enforced` — LLM returns 2 tool_calls;
        runner dispatches the first only, emits a Misc warning event
      - `max_steps_terminates` — LLM keeps returning non-idle tool
        calls; loop terminates at max_steps with completed=false
      - `message_ask_user_suspends_run` — LLM calls
        `message_ask_user`; runner returns with state=SUSPENDED
- [ ] `cargo clippy / fmt / test / spec-check` all pass

## Non-goals

- Stuck detection real-pump (story 0.15)
- Cost cap real-check (story 0.16; this story takes the arg as a no-op)
- WebSocket `user_response` resume path (story 0.17)
- Frontend integration (story 0.20+)
- Verifier slot meta-cognition (Phase 1; architecture §6 L4)
- Multi-phase plan management with `plan_advance` / `plan_update` real
  bodies (deferred — agent calls them via the registry, but Phase 0
  treats them as stubs returning `not_implemented`)

---

## Implementation steps

### 1. Types + module layout

```
crates/seasoned-hand-core/src/agent/
  mod.rs       — AgentRunner, RunRequest, RunResult, AgentError
  prompt.rs    — build_messages(session_id, plan, events) -> Vec<Message>
  tests.rs
```

### 2. AgentRunner state

```rust
pub struct AgentRunner {
    llm: LlmClient,
    dispatcher: Arc<ToolDispatcher>,
    events: Arc<SqliteEventStore>,
    router: Arc<SlotRouter>,
    sandbox: Arc<SandboxClient>,
    search: Arc<SearchClient>,
    sessions: Arc<DbPool>,
}
```

### 3. Loop skeleton

```rust
pub async fn run(&self, req: RunRequest) -> Result<RunResult, AgentError> {
    self.set_session_state(&req.session_id, "RUNNING").await?;
    self.create_baseline_plan(&req.session_id, &req.input).await?;
    self.append_user_message(&req.session_id, &req.input).await?;

    let mut last_assistant_hash: Option<u64> = None;
    let mut duplicate_count = 0u32;

    for step in 0..req.max_steps {
        // 1. Build sticky context (plan + recent events)
        let messages = self.build_messages(&req.session_id).await?;
        let tools = self.tool_specs_from_registry();

        // 2. Call LLM with tool_choice=required
        let main_slot = self.router.resolve(SlotName::Main);
        let resp = self.llm.chat_completion(ChatCompletionRequest {
            model: main_slot.model.clone(),
            messages,
            tools: Some(tools),
            tool_choice: Some(ToolChoice::required()),
            ..Default::default()
        }).await?;

        // 3. Stuck detection scaffold
        let asst = resp.choices.first().map(|c| &c.message);
        if let Some(m) = asst {
            let h = hash_message(m);
            if Some(h) == last_assistant_hash {
                duplicate_count += 1;
                if duplicate_count >= 2 {
                    // strategy-change prompt injection placeholder (story 0.15)
                    self.emit_misc("stuck_detected").await?;
                }
            } else { duplicate_count = 0; }
            last_assistant_hash = Some(h);
        }

        // 4. Dispatch the FIRST tool_call only (architecture §4)
        let calls = asst.and_then(|m| m.tool_calls.as_ref());
        let Some(call) = calls.and_then(|c| c.first()) else {
            // No tool — emit a Misc warning and break
            self.emit_misc_with("no_tool_call", json!({"step": step})).await?;
            break;
        };
        if calls.map(|c| c.len() > 1).unwrap_or(false) {
            self.emit_misc_with("multi_tool_warning", json!({
                "step": step, "kept": call.function.name,
                "dropped_count": calls.unwrap().len() - 1
            })).await?;
        }
        let args: Value = serde_json::from_str(&call.function.arguments).unwrap_or(Value::Null);

        // 5. Dispatch (hooks emit Action + Observation)
        let ctx = ToolContext {
            session_id: req.session_id.clone(),
            events: self.events.clone(),
            sandbox: self.sandbox.clone(),
            search: self.search.clone(),
        };
        let output = self.dispatcher.dispatch(&ctx, &call.function.name, args).await;

        // 6. Termination checks
        if call.function.name == "idle" && output.ok {
            self.set_session_state(&req.session_id, "FINISHED").await?;
            return Ok(RunResult { session_id: req.session_id, completed: true, last_message: None, steps: step + 1 });
        }
        if call.function.name == "message_ask_user" {
            self.set_session_state(&req.session_id, "SUSPENDED").await?;
            return Ok(RunResult { session_id: req.session_id, completed: false, last_message: None, steps: step + 1 });
        }

        // 7. (cost cap check — story 0.16 will add real lookup; no-op here)
    }

    self.emit_misc_with("max_steps_reached", json!({"max_steps": req.max_steps})).await?;
    self.set_session_state(&req.session_id, "FINISHED").await?;
    Ok(RunResult { session_id: req.session_id, completed: false, last_message: None, steps: req.max_steps })
}
```

### 4. `tool_specs_from_registry`

Iterate `dispatcher.registry()` and produce a `Vec<ToolSpec>`. Each
tool's `schema()` becomes the `parameters` JSON Schema in the
`ToolSpec::function` builder. Plan tools (`plan_advance`,
`plan_update`) are LLM-callable per ADR-010 so they go in too.

### 5. `build_messages`

```rust
async fn build_messages(&self, session_id: &str) -> Result<Vec<Message>, AgentError> {
    let mut out = vec![system_message()];
    // sticky plan snapshot
    if let Some(plan_event) = self.latest_plan_event(session_id).await? {
        out.push(Message {
            role: Role::System,
            content: Some(format!("PLAN: {}", plan_event.data)),
            ..Default::default()
        });
    }
    let events = self.events.query(session_id, EventQuery {
        limit: Some(100), ..Default::default()
    }).await?;
    for e in events {
        out.push(event_to_message(&e));
    }
    Ok(out)
}
```

Phase 0 keeps `event_to_message` simple — assistant messages become
`Role::Assistant`, observations become `Role::Tool` with the
`call_id` as `tool_call_id`. User messages become `Role::User`.

### 6. AppState wiring

`AppState::new` builds and stores the `AgentRunner`. Story 0.17 will
add a WebSocket command handler that calls `runner.run(...)`.

### 7. Tests

All tests use a wiremock'd Bifrost. Use `LlmClient::new(mock.uri(),
None)` and feed scripted responses. The dispatcher, sandbox, search
clients are constructed as before (sandbox unused since none of the
agent test scenarios touch a real sandbox tool — they exercise
`message_notify_user`, `info_search_web` (returns missing_api_key
gracefully), `idle`).

---

## Files changed

- `crates/seasoned-hand-core/src/lib.rs` (`pub mod agent`)
- `crates/seasoned-hand-core/src/agent/mod.rs` (new)
- `crates/seasoned-hand-core/src/agent/prompt.rs` (new)
- `crates/seasoned-hand-core/src/agent/tests.rs` (new)
- `crates/seasoned-hand-server/src/lib.rs` (`AppState.runner`)
- `crates/seasoned-hand-server/src/main.rs` (build runner)
- `crates/seasoned-hand-server/tests/healthz.rs` + `events.rs` (update)
- `specs/phase-0/DEBT.md` (new entries: stuck detection scaffolding,
  cost cap arg-but-no-op, plan tools still stubs)

---

## Spec references

- `/specs/01-architecture/ARCHITECTURE.md` §4 (agent loop, one tool
  per iteration, tool_choice=required)
- `/specs/01-architecture/decisions/ADR-010-plan-as-process-control-block.md`
- `/specs/00-philosophy/PRINCIPLES.md` #2 (one tool per iteration),
  #3 (append-only), #10 (failure-tolerant), #17 (plan stickiness)
- `/specs/phase-0/architecture.md` §1, §4.3

---

## Commit message

```
feat(phase-0): story 0.14 - agent runner ReAct loop

- seasoned-hand-core::agent::AgentRunner: think → act → observe loop
- tool_choice="required" + first-tool-only enforcement per
  architecture §4 hard constraint (multi-call → Misc warning event,
  only first dispatched)
- Sticky context: system message + plan snapshot + recent event
  history (up to 100 events). Plan tools are still stubs; baseline
  Plan event seeded at task start
- Termination: idle → FINISHED, message_ask_user → SUSPENDED,
  max_steps → Misc{kind:max_steps_reached}+FINISHED, cost_cap arg
  accepted but check is no-op pending story 0.16
- Stuck detection scaffold: hash of last assistant message, increments
  duplicate counter, emits Misc{kind:stuck_detected} when ≥2 — full
  pump in story 0.15
- AgentError: Llm / Db / Cancelled / Internal
- AppState gains runner: Arc<AgentRunner>; server main.rs builds it
- N tests via wiremock'd Bifrost cover single-turn, multi-turn,
  multi-call enforcement, max_steps, ask_user suspend
- cargo clippy / fmt / test / spec-check all pass

Debt: 3 new items — stuck-detection pump deferred to 0.15, cost cap
no-op until 0.16, plan tools still stubs (full PlanManager is a
follow-up).

refs: /specs/phase-0/stories/story-0.14.md
```

---

## Notes for next story (0.15)

- Stuck-detection hook scaffold exists; story 0.15 (1h) implements the
  real "strategy change" prompt injection: when duplicate_count ≥ 2,
  prepend a system message like "Your last 2 attempts repeated. Try a
  different strategy: …" before the next LLM call; track up to 4
  duplicates before forcing a hard ERROR
