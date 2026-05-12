# Story 1.9b — Verifier Worker runtime (Redis Streams + concurrency + watchdog)

> **Status**: ready
> **Estimated**: 3 hours
> **Dependencies**: 1.9 (DB layer, types, routes, system prompt loaded)
> **Phase**: 1
> **Type**: backend
> **Reads first**: `/specs/phase-1/architecture.md` §2.4 (Worker shape),
> §2.4.4 (fresh context construction), §2.4.5 (verdict handling — pass
> through), §7 (latency budget), §8 ("verifier unparseable" +
> "verifier watchdog" failure modes).

---

## Goal

Spin up the Verifier Worker Tokio task: subscribes to Redis Streams
`verify_request`, builds a fresh context per request, calls the
`verifier` slot, parses the JSON verdict, persists a `verifications`
row (via 1.9's persistence layer), emits a `Misc{kind:"verifier_verdict"}`
event. Per-session FIFO + global concurrency cap of 2 + 60-second
watchdog. No triggers yet — they are stories 1.10/1.11/1.12.

## Acceptance criteria

- [ ] `seasoned-hand-core::verifier::Worker::run(state, redis,
      shutdown)` spawned at server startup **only if**
      `AppState::verifier_enabled == true`. If false, the function
      returns Ok(()) immediately and logs a debug message.
- [ ] Worker creates the Redis Streams consumer group `verifier` on
      stream `verify_request` at boot (`XGROUP CREATE ... MKSTREAM`),
      idempotent — pre-existing group is not an error.
- [ ] Main loop: `XREADGROUP GROUP verifier <consumer> BLOCK 5000
      COUNT 16 STREAMS verify_request >`. Each message is parsed as
      a `VerifyRequest` (1.9 types); on parse failure, the message is
      `XACK`ed and a `tracing::warn!` logged (do not block on
      malformed entries).
- [ ] Concurrency:
      - **Global cap** = `verifier.max_concurrency` config (default 2).
        Implemented via `Arc<Semaphore>`.
      - **Per-session FIFO** = 1 verification in flight per
        `session_id`. Implemented via `DashMap<SessionId,
        Arc<Mutex<()>>>`. A request waits on its session lock then on
        the global semaphore.
- [ ] `handle_request(state, req)`:
      1. **Fresh context** built per architecture §2.4.4:
         (a) `PlanManager::snapshot(session_id)`,
         (b) `sandbox.read_workspace_file_json::<FeatureList>(...)`
         (skip silently if absent),
         (c) `events.query` with anchor `triggered_at_event_id` and
         window N=50 in each direction,
         (d) trigger description string (`format!` per
         `VerifyTrigger` variant),
         (e) system prompt = `state.verifier_system_prompt`.
      2. **LLM call**: `verifier` slot, `tool_choice: none`,
         `max_tokens: 1024` (verdict is small JSON).
      3. **Strict JSON parse** with serde over the schema in §2.4.3.
         On parse failure: **retry once** with a stricter suffix in
         the user message: `"Respond with ONLY a JSON object matching
         the schema. No prose."`. On second failure: synthesize a
         `Verdict { verdict: Fail, reason: "verifier_unparseable",
         evidence_event_ids: [], suggested_plan_update: None }`.
      4. **Suggested plan update execution** (only when verdict is
         `fail` + suggestion present): call `PlanManager::update(
         session_id, phases, Source::Verifier)` *before* persistence
         (so the downstream Gate from story 1.10 sees the new plan).
      5. **Persist**: `verifier::persistence::insert` (story 1.9
         CRUD) returns `verification_id`.
      6. **Emit**: `events.emit_misc(session_id, "verifier_verdict",
         { verdict, reason, evidence_event_ids,
         suggested_plan_update?, verification_id, trigger_kind })`.
- [ ] Watchdog: each `handle_request` is wrapped in
      `tokio::time::timeout(60s, ...)`. On timeout: emit Misc
      `verifier_watchdog{session_id, triggered_at_event_id}` and
      `XACK` the message. State transitions on watchdog are the
      Gate's responsibility (story 1.10).
- [ ] Graceful shutdown via `tokio::sync::CancellationToken` argument
      — loop exits on cancellation; in-flight tasks complete (no
      forced abort here; story 1.17 handles user-initiated cancel).
- [ ] Cost attribution: `cost_cents` for the verification = delta of
      Phase 0 `/cost` poll between handle_request start and end.
      Stored on the `verifications` row via 1.9 persistence.
- [ ] Tests:
      - `worker_skips_when_verifier_not_enabled` — `verifier_enabled=false`,
        assert `run()` returns immediately.
      - `worker_processes_synthetic_request_inserts_row_and_emits_misc`
        — `#[ignore]` integration with real Redis; or pure-unit via
        directly calling `handle_request` against a wiremock'd
        `verifier` slot returning canned JSON.
      - `worker_handles_malformed_json_with_fallback_fail` — wiremock
        returns plain prose twice; assert
        `verdict="fail",reason="verifier_unparseable"` row + Misc.
      - `worker_applies_suggested_plan_update_before_emitting_verdict`
        — wiremock returns `fail` + `suggested_plan_update`; assert
        `PlanManager::snapshot` reflects new plan and the Plan event
        timestamp is *before* the verifier_verdict event.
      - `worker_respects_per_session_fifo` — submit two requests for
        the same session in parallel; assert serial execution by
        timestamp order.
      - `worker_respects_global_concurrency_cap` — submit 5 requests
        across 5 sessions; assert ≤ 2 in-flight at any instant via
        instrumented semaphore probe.
      - `worker_watchdog_aborts_at_60s` — `tokio::time::pause` +
        wiremock that never responds; assert Misc `verifier_watchdog`
        emitted ≤ 60s and message XACKed.
      - `worker_graceful_shutdown` — cancel mid-loop; assert
        in-flight task completes, no panics.

## Non-goals

- Triggers (TaskComplete=1.10, Invalidation=1.11, CircuitBreaker=1.12).
- Verdict-driven state transitions on the session — VerifierGate
  (story 1.10) owns that.
- Cancellation of in-flight verification on user `task_cancel` — story
  1.17 adds that (it selects against the same cancel token).
- Multi-process worker coordination — single control plane in Phase 1.
- Verifier evidence pre-fetching for the frontend — story 1.18 (lazy).

## Implementation steps

### 1. Module additions

```
crates/seasoned-hand-core/src/verifier/
  worker.rs        — Worker::run, handle_request
  context.rs       — build_fresh_context()
  parse.rs         — Verdict struct + serde + strict-retry
```

### 2. Worker shape

```rust
pub async fn run(
    state: AppState,
    redis: Arc<RedisPool>,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    if !state.verifier_enabled {
        tracing::debug!("verifier disabled; worker not spawned");
        return Ok(());
    }
    let _ = ensure_consumer_group(&redis, "verify_request", "verifier").await;
    let sem = Arc::new(Semaphore::new(state.config.verifier_max_concurrency));
    let session_locks: Arc<DashMap<String, Arc<Mutex<()>>>> = Default::default();

    while !shutdown.is_cancelled() {
        let batch = read_group(&redis, "verify_request", "verifier",
                               Duration::from_secs(5), 16).await?;
        for (msg_id, req) in batch {
            let sem_c = sem.clone();
            let locks_c = session_locks.clone();
            let state_c = state.clone();
            let redis_c = redis.clone();
            tokio::spawn(async move {
                let lock = locks_c.entry(req.session_id.clone())
                    .or_insert_with(|| Arc::new(Mutex::new(())))
                    .clone();
                let _g = lock.lock().await;
                let _permit = sem_c.acquire_owned().await.unwrap();
                match tokio::time::timeout(Duration::from_secs(60),
                                           handle_request(&state_c, &req)).await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => tracing::warn!(error = %e, "verifier request failed"),
                    Err(_) => {
                        state_c.events.emit_misc(&req.session_id, "verifier_watchdog",
                            json!({"triggered_at_event_id": req.triggered_at_event_id})).await.ok();
                    }
                }
                let _ = ack(&redis_c, "verify_request", "verifier", &msg_id).await;
            });
        }
    }
    Ok(())
}
```

### 3. Fresh context

```rust
pub async fn build_fresh_context(state: &AppState, req: &VerifyRequest)
    -> Result<Vec<Message>, VerifyError>
{
    let plan = state.plan_manager.snapshot(&req.session_id).await?;
    let plan_block = plan::render::sticky_render(&plan, 1000);

    let feature_list = state.sandbox
        .read_workspace_file_json::<FeatureList>(&req.session_id, "/workspace/feature-list.json")
        .await.ok();

    let window = state.events.query(EventQuery {
        session_id: req.session_id.clone(),
        anchor_event_id: Some(req.triggered_at_event_id),
        window_before: Some(50),
        window_after: Some(50),
        ..Default::default()
    }).await?;

    let trigger_descr = describe_trigger(&req.trigger);
    let mut user_body = String::new();
    user_body.push_str("=== TRIGGER ===\n"); user_body.push_str(&trigger_descr); user_body.push('\n');
    user_body.push_str(&plan_block);
    if let Some(fl) = feature_list {
        user_body.push_str("\n=== FEATURE LIST ===\n");
        user_body.push_str(&serde_json::to_string_pretty(&fl)?);
    }
    user_body.push_str("\n=== EVENT WINDOW ===\n");
    for ev in &window { user_body.push_str(&format_event_for_verifier(ev)); }

    Ok(vec![
        Message::system(state.verifier_system_prompt.as_str()),
        Message::user(&user_body),
    ])
}
```

### 4. Strict JSON retry

```rust
pub async fn call_verifier_with_retry(state: &AppState, messages: Vec<Message>)
    -> Verdict
{
    let resp = state.llm.chat_completion(req_for(messages.clone())).await;
    if let Ok(v) = parse_verdict(&resp) { return v; }

    let mut retry = messages.clone();
    retry.last_mut().unwrap().push_text(
        "\n\nRespond with ONLY a JSON object matching the schema. No prose.");
    let resp2 = state.llm.chat_completion(req_for(retry)).await;
    parse_verdict(&resp2).unwrap_or(Verdict::unparseable())
}
```

### 5. Cost attribution

Cost-poll task (Phase 0 story 0.16) tracks per-session cumulative
spend. Worker reads `cost_cents = poll.snapshot(session_id)` before
and after `handle_request`; the delta is stored on the row.

### 6. AppState wiring

Spawn the worker from `crates/seasoned-hand-server/src/main.rs` after
the HTTP listener is built but before `listener.serve()` resolves —
exactly as Phase 0 spawns its background tasks.

```rust
let shutdown = CancellationToken::new();
let worker_handle = tokio::spawn(verifier::worker::run(
    state.clone(), redis.clone(), shutdown.clone(),
));
// ... HTTP listener serve ...
shutdown.cancel();
let _ = worker_handle.await;
```

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core verifier::worker::
cargo test -p seasoned-hand-core verifier::context::
./scripts/spec-check.sh
```

Live (Redis required): `XADD verify_request * payload '<json>'` and
observe `verifier_verdict` Misc + row in `verifications`.

---

## Files changed

- `crates/seasoned-hand-core/src/verifier/worker.rs` (new)
- `crates/seasoned-hand-core/src/verifier/context.rs` (new)
- `crates/seasoned-hand-core/src/verifier/parse.rs` (new)
- `crates/seasoned-hand-core/src/verifier/mod.rs` (modify — `pub mod
  worker; pub mod context; pub mod parse;`)
- `crates/seasoned-hand-server/src/main.rs` (modify — spawn worker)
- `crates/seasoned-hand-core/src/events/misc.rs` (modify — document
  `verifier_verdict, verifier_watchdog`)
- `config/seasoned-hand.toml` (modify — `[verifier] max_concurrency = 2`)

---

## Spec references

- `/specs/phase-1/architecture.md` §2.4 (worker shape), §2.4.4 (fresh
  context — verbatim 5-step list), §7 (< 6s p95 latency, < 5¢ cost),
  §8 ("verifier_unparseable" + "verifier_watchdog" failure modes).

---

## Commit message

```
feat(phase-1): story 1.9b - Verifier Worker runtime + Redis Streams + watchdog

- verifier::Worker::run spawned at startup when verifier_enabled=true;
  XREADGROUP loop over "verify_request" (consumer group "verifier"),
  BLOCK 5000 COUNT 16; idempotent XGROUP CREATE MKSTREAM
- Per-session FIFO via DashMap<sid, Mutex>; global semaphore cap = 2
  (config verifier.max_concurrency)
- handle_request: build_fresh_context (plan snapshot + feature-list +
  ±50 event window + trigger description + system prompt) → verifier
  slot LLM call (tool_choice:none, max_tokens 1024) → strict JSON parse
  with 1-shot retry → unparseable fallback verdict → suggested_plan_update
  application via PlanManager::update(Source::Verifier) → persist via
  story-1.9 CRUD → emit Misc verifier_verdict
- 60-second watchdog timeout; on timeout emit verifier_watchdog Misc +
  XACK (state transition deferred to story 1.10 Gate)
- Cost attribution via /cost delta around handle_request
- Graceful shutdown via CancellationToken; in-flight tasks complete
- 8 unit + integration tests (live-Redis ones #[ignore]'d)

refs: /specs/phase-1/stories/story-1.9b.md
```

---

## Notes for next story (1.10)

The Verifier pipeline is fully runnable end-to-end against synthetic
Redis Streams input. Story 1.10 wires the first real trigger
(TaskComplete) — emits `VerifyRequest::TaskComplete` from the `idle` /
`message_notify_user{final:true}` dispatch path and spawns the
VerifierGate that listens for `verifier_verdict` Misc events and
applies session-state transitions per architecture §2.4.5.
