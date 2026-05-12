# Story 1.12 — Circuit Breaker unification + CircuitBreaker trigger + Diversity Injector

> **Status**: ready
> **Estimated**: 3 hours
> **Dependencies**: 1.10 (verdict-handling), 1.11 (Invalidation trigger
> precedent for non-state-transitioning verdicts)
> **Phase**: 1
> **Type**: backend
> **Reads first**: `/specs/phase-1/architecture.md` §2.5 (Circuit Breaker
> table), §2.3 row 6 (Diversity Injection — PRINCIPLE #6),
> `/specs/phase-0/stories/story-0.15.md` (stuck-tracker current
> behavior), `/specs/phase-0/stories/story-0.16.md` (cost cap).

---

## Goal

Replace Phase 0's scattered breakers (stuck-tracker terminates ERROR;
cost-cap suspends; max-steps fires Misc event) with one unified
**CircuitBreaker** state machine that handles four conditions and
routes each trip through the Verifier for a salvageability verdict.
Plus: land the **Diversity Injector** (PRINCIPLE #6) which rotates the
strategy-change prompt through four phrasings + a specific recent
observation reference — done here because it modifies the stuck-tracker
emission path that this story's breaker integrates with.

## Acceptance criteria

- [ ] `seasoned-hand-core::agent::breaker::CircuitBreaker` Tokio actor,
      one per session, subscribed to the event stream. Holds counters:
      - `stuck_count: u32` (duplicate assistant messages)
      - `cost_cents: u32` (last observed via `/cost` poll)
      - `iteration_count: u32`
      - `recent_obs_ok: ArrayDeque<bool, 10>` (sliding window)
- [ ] Four conditions and their trip rules:
      | Breaker | Condition |
      |---|---|
      | Stuck | `stuck_count >= 4` (architecture §2.5 unchanged) |
      | Cost | `cost_cents >= session.cost_cap_cents` |
      | MaxSteps | `iteration_count >= session.max_steps` |
      | ErrorRate | `recent_obs_ok` contains ≥5 `false` values |
- [ ] On trip: emit Misc `verifier_request{trigger:"CircuitBreaker",
      kind:<Stuck|Cost|MaxSteps|ErrorRate>}`, push
      `VerifyRequest::CircuitBreaker { kind }` to Redis Streams.
      **Session state stays `RUNNING` during the breaker-driven
      verification call** — the `VERIFYING` state is reserved for the
      TaskComplete trigger only (architecture §3.2 transition table
      lists exactly one path into VERIFYING: `RUNNING → VERIFYING on
      TaskComplete trigger`). Verdict drives action.
- [ ] Verdict handling (extends VerifierGate from story 1.10):
      | Verdict | Breaker | Action |
      |---|---|---|
      | `pass` (salvageable) | Stuck | Reset `stuck_count = 0`; loop continues |
      | `pass` (salvageable) | ErrorRate | Reset `recent_obs_ok`; loop continues |
      | `pass` (salvageable) | Cost | SUSPEND anyway (Cost is hard); verdict surfaces achievement |
      | `pass` (salvageable) | MaxSteps | SUSPEND with verdict surfaced |
      | `fail` with suggestion | (any) | Worker applies `plan_update` (story 1.9 path), loop continues |
      | `fail` without suggestion | Stuck / ErrorRate | ERROR (terminate) |
      | `fail` without suggestion | Cost / MaxSteps | SUSPEND (hard cap reached) |
- [ ] Diversity Injector (`agent::diversity`):
      - Const array of 4 prompt phrasings.
      - On stuck-tracker `InjectStrategyPrompt`, the agent runner picks
        the next phrasing (round-robin per-session) and inserts one
        sentence referencing the most recent Observation event
        (formatted as `"Your last observation (event #N): <summary>"`).
      - After all 4 variants are used in a stuck cycle, fall back to
        the existing Phase 0 hard-termination at 4 duplicates
        (`stuck_count >= 4` → ERROR path, unchanged).
- [ ] Phase 0 stuck-tracker's local `ERROR` transition is replaced by
      this story's CircuitBreaker → Verifier path. Phase 0 DEBT note: the
      Stuck-tracker no longer terminates directly.
- [ ] Tests:
      - `breaker_trips_on_stuck_at_4`.
      - `breaker_trips_on_cost_at_cap`.
      - `breaker_trips_on_max_steps`.
      - `breaker_trips_on_error_rate_5_of_10`.
      - `breaker_passes_verdict_resets_counter_for_stuck`.
      - `breaker_pass_on_cost_still_suspends` — Cost is a hard cap.
      - `breaker_fail_with_suggestion_continues`.
      - `breaker_fail_without_suggestion_errors_for_stuck`,
        `_suspends_for_cost`.
      - `diversity_injector_4_variants_rotate`.
      - `diversity_injector_references_recent_observation`.

## Non-goals

- New breaker conditions beyond the four listed.
- Externalizing the diversity variants to a DB table — phase-1/DEBT.md
  #7 explicitly defers that to Phase 4 Curator.
- Per-breaker cost-cap configuration — single `cost_cap_cents` per
  session unchanged.
- ML-driven stuck detection (Phase 4+).

## Implementation steps

### 1. Module layout

```
crates/seasoned-hand-core/src/agent/breaker/
  mod.rs       — CircuitBreaker actor + BreakerKind enum
  conditions.rs — trip rule implementations
  tests.rs
crates/seasoned-hand-core/src/agent/diversity.rs
```

### 2. BreakerKind + actor

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BreakerKind { Stuck, Cost, MaxSteps, ErrorRate }

pub struct CircuitBreaker {
    session_id: String,
    events: Arc<dyn EventStore>,
    redis: Arc<RedisPool>,
    sessions: Arc<SessionStore>,
    inner: Mutex<BreakerState>,
}

#[derive(Default)]
struct BreakerState {
    stuck_count: u32,
    cost_cents: u32,
    iteration_count: u32,
    recent_obs_ok: VecDeque<bool>, // cap 10
    armed: bool,                    // true while no verdict pending
}
```

Actor loop subscribes to (a) `Action`/`Observation` events for
iteration count + error rate, (b) the cost-cap poll task already in
Phase 0, (c) the StuckTracker (story 0.15) `StuckAction` channel —
intercept the `Terminate` branch and redirect to a trip instead.

### 3. Trip emission

```rust
async fn maybe_trip(&self, kind: BreakerKind) {
    let mut s = self.inner.lock().await;
    if !s.armed { return; }
    s.armed = false; // one trip in flight at a time
    drop(s);
    let event_id = self.events.emit_misc(&self.session_id, "verifier_request", json!({
        "trigger": "CircuitBreaker", "kind": kind,
    })).await.unwrap_or(0);
    let req = VerifyRequest {
        session_id: self.session_id.clone(),
        trigger: VerifyTrigger::CircuitBreaker { kind },
        triggered_at_event_id: event_id,
        context_hint: VerifyContextHint::default(),
    };
    let _ = self.redis.xadd_json("verify_request", &req).await;
}
```

`armed` re-arms after the verdict callback (Gate calls
`CircuitBreaker::on_verdict`).

### 4. Gate verdict arm

```rust
(Some("CircuitBreaker"), Some(verdict)) => {
    let kind: BreakerKind = parse_kind(&ev.data);
    let suggested = ev.data.get("suggested_plan_update").is_some();
    match (kind, verdict, suggested) {
        (BreakerKind::Stuck,    "pass", _) => { state.breakers.reset_stuck(&ev.session_id).await; }
        (BreakerKind::ErrorRate,"pass", _) => { state.breakers.reset_error_rate(&ev.session_id).await; }
        (BreakerKind::Cost,     "pass", _) | (BreakerKind::MaxSteps, "pass", _) => {
            state.sessions.transition(&ev.session_id, "RUNNING", "SUSPENDED").await.ok();
        }
        (_, "fail", true) => { /* plan_update already applied by Worker; resume */ }
        (BreakerKind::Stuck,     "fail", false) |
        (BreakerKind::ErrorRate, "fail", false) => {
            state.sessions.transition(&ev.session_id, "RUNNING", "ERROR").await.ok();
        }
        (BreakerKind::Cost,      "fail", false) |
        (BreakerKind::MaxSteps,  "fail", false) => {
            state.sessions.transition(&ev.session_id, "RUNNING", "SUSPENDED").await.ok();
        }
        _ => {}
    }
    state.breakers.rearm(&ev.session_id).await;
}
```

### 5. Diversity Injector

```rust
// crates/seasoned-hand-core/src/agent/diversity.rs
pub const VARIANTS: [&str; 4] = [
    "Your last {n} attempts repeated. Try a different tool, re-read recent observations, or call message_ask_user to clarify.",
    "We have looped on the same response {n} times. Step back: which assumption could be wrong? Inspect a different file or query the user.",
    "{n} duplicates. Don't repeat — change *what* you observe (different path/query) before changing how you act.",
    "{n}× same response. Recall PRINCIPLE #5 — the failed observations in the recent stream are signal, not noise. Pick one and act on it differently.",
];

pub struct DiversityInjector {
    cursor: DashMap<String /* session */, usize /* next variant index */>,
}

impl DiversityInjector {
    pub fn next_prompt(&self, session_id: &str, count: u32, recent_obs: &Observation) -> String {
        let i = {
            let mut e = self.cursor.entry(session_id.into()).or_insert(0);
            let v = *e; *e = (*e + 1) % VARIANTS.len();
            v
        };
        let template = VARIANTS[i].replace("{n}", &count.to_string());
        let suffix = format!(" Your last observation (event #{}): {}.",
            recent_obs.event_id, summarize(&recent_obs.body, 120));
        template + &suffix
    }
}
```

`summarize` truncates to 120 chars with `…`. The stuck-tracker
(story 0.15) `agent_runner` integration site (where it currently
constructs `"You have repeated the same response N times..."`) now
calls `DiversityInjector::next_prompt(session_id, count, recent_obs)`
instead.

### 6. Cost-cap polling integration

Phase 0 story 0.16 already polls `/cost` and updates a per-session
counter. Replace its `SUSPEND` transition with a
`breaker.maybe_trip(BreakerKind::Cost)` call. Same for max-steps in
`AgentRunner::run` — the existing Misc `max_steps_reached` emission is
preserved, but the SUSPEND now waits for the verdict.

### 7. Wiring

`AppState::breakers: Arc<BreakerRegistry>` (per-session lookup); created
in `AppState::new`. AgentRunner constructs/looks-up the breaker per
`run` invocation.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core agent::breaker::
cargo test -p seasoned-hand-core agent::diversity::
cargo test -p seasoned-hand-core verifier::gate::tests::circuit_breaker
./scripts/spec-check.sh
```

---

## Files changed

- `crates/seasoned-hand-core/src/agent/breaker/mod.rs` (new)
- `crates/seasoned-hand-core/src/agent/breaker/conditions.rs` (new)
- `crates/seasoned-hand-core/src/agent/breaker/tests.rs` (new)
- `crates/seasoned-hand-core/src/agent/diversity.rs` (new)
- `crates/seasoned-hand-core/src/agent/stuck.rs` (modify — Terminate
  branch now defers to breaker actor)
- `crates/seasoned-hand-core/src/agent/mod.rs` (modify — `pub mod
  breaker; pub mod diversity;`, wire injector at StuckAction::InjectStrategyPrompt
  site)
- `crates/seasoned-hand-core/src/cost/poller.rs` (modify if Phase 0
  named it differently — call breaker.maybe_trip(Cost) instead of
  direct SUSPEND)
- `crates/seasoned-hand-core/src/verifier/gate.rs` (modify — new
  match arm for `Some("CircuitBreaker")`)
- `crates/seasoned-hand-core/src/verifier/mod.rs` (modify — `BreakerKind`
  type and `VerifyTrigger::CircuitBreaker` deserialise)
- `crates/seasoned-hand-server/src/state.rs` (modify — `breakers`
  field)

---

## Spec references

- `/specs/phase-1/architecture.md` §2.5 (4-condition table), §2.3 row 6
  (Diversity Injection details), §12 q3 (variants are a Rust constant
  array — phase-1/DEBT.md #7), §8 ("Diversity injector exhausts variant
  set" failure mode).
- `/specs/00-philosophy/PRINCIPLES.md` #5 (errors preserved), #6
  (diversity injection — kept colloquially as "rotate phrasings").

---

## Commit message

```
feat(phase-1): story 1.12 - unified Circuit Breaker + Diversity Injector

- agent::breaker::CircuitBreaker unifies four conditions (Stuck, Cost,
  MaxSteps, ErrorRate) into one per-session actor; trip emits
  Misc verifier_request{trigger:"CircuitBreaker", kind} + XADD
  verify_request, then awaits a verdict instead of terminating locally
- VerifierGate gains a CircuitBreaker arm: pass → reset counter (Stuck,
  ErrorRate) or SUSPEND (Cost, MaxSteps); fail+suggestion → continue
  (Worker already applied plan_update); fail-no-suggestion → ERROR
  (Stuck/ErrorRate) or SUSPEND (Cost/MaxSteps)
- agent::diversity::DiversityInjector rotates 4 prompt phrasings per
  session + appends a reference to the most recent Observation event;
  closes PRINCIPLE #6
- Stuck-tracker (Phase 0 story 0.15) and cost-cap poller (story 0.16)
  now defer their previous direct ERROR/SUSPEND transitions to the
  breaker actor

refs: /specs/phase-1/stories/story-1.12.md
```

---

## Notes for next story (1.13)

All three Verifier triggers (TaskComplete, Invalidation, CircuitBreaker)
are live and the Gate's state-transition table is fully wired. Story
1.13 (Checkpoint Manager) is independent of the breaker work and can be
done in parallel by the Codex pair after this story lands.
