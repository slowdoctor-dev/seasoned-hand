# Story 1.6 — Context Recitation (PRINCIPLE #4)

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 1.4 (`/workspace/progress.txt` exists)
> **Phase**: 1
> **Type**: backend
> **Reads first**: `/specs/phase-1/architecture.md` §2.3 row 4 (Todo
> recitation), §3.6 (`progress.txt` format), §3.4 (Misc
> `progress_recite` payload), `/specs/00-philosophy/PRINCIPLES.md` #4.

---

## Goal

Every 10 worker iterations, the runtime injects a synthetic
`Misc{kind:"progress_recite"}` event whose content is the tail (last
80 lines) of `/workspace/progress.txt`. The next agent iteration consumes
this event before tool selection — closing PRINCIPLE #4 (todo recitation
defeats lost-in-the-middle drift in long sessions).

## Acceptance criteria

- [ ] `seasoned-hand-core::agent::recite::ReciteScheduler` tracks
      per-session iteration counters. Threshold = 10 (constant
      `RECITE_EVERY_N = 10`).
- [ ] On the iteration whose counter is a multiple of 10 (`step > 0 &&
      step % 10 == 0`), the runner reads `/workspace/progress.txt` tail
      (last 80 lines) via the existing sandbox file API and emits
      `Misc{kind:"progress_recite", data: {progress_path:
      "/workspace/progress.txt", content_preview: "<tail>"}}`.
- [ ] The emitted event has event-id `< next_llm_call_event_id`, so the
      next `build_messages` call includes it in the recent-events window.
- [ ] If `progress.txt` is missing or empty, the recite tick emits
      `Misc{kind:"progress_recite_skipped", reason}` and the loop
      continues uninterrupted.
- [ ] When the sandbox file read takes > 1s, recite is skipped that tick
      (the timer should not block tool dispatch); emit `Misc{kind:
      "progress_recite_skipped", reason: "slow_read"}`.
- [ ] Tests:
      - `recite_fires_on_tenth_iteration` — drive a synthetic loop with
        iteration counter, assert event emission at step=10.
      - `recite_does_not_fire_on_step_zero` — explicit edge case.
      - `recite_truncates_to_80_lines` — synthesize a 200-line
        progress.txt; assert the emitted `content_preview` has 80 lines.
      - `recite_skip_on_missing_file_does_not_break_loop` — file absent;
        runner continues to step 11.
      - `recite_skip_on_slow_read` — sandbox stub delays 1.5s; assert
        skipped event + loop continues without delay.

## Non-goals

- Dynamic threshold (10 is the architecture-pinned default).
- Recitation of `feature-list.json` — the Verifier reads that (story 1.10);
  the agent already updates it via `feature_mark_done`.
- Token-cap on recite content beyond the 80-line cap (architecture defers
  fine-grained token accounting to a Phase 4 budget tool).
- Frontend rendering of `progress_recite` Misc — falls through to the
  existing Phase 0 muted-Misc rendering (acceptable; UI lane work is
  story 1.18).

---

## Implementation steps

### 1. Module

```
crates/seasoned-hand-core/src/agent/recite.rs
```

```rust
pub const RECITE_EVERY_N: u32 = 10;
pub const RECITE_TAIL_LINES: usize = 80;
pub const RECITE_READ_TIMEOUT: Duration = Duration::from_secs(1);

pub struct ReciteScheduler;

impl ReciteScheduler {
    pub fn should_fire(step: u32) -> bool {
        step > 0 && step % RECITE_EVERY_N == 0
    }
}

pub async fn recite_tick(
    sandbox: &SandboxClient,
    events: &dyn EventStore,
    session_id: &str,
) {
    let read_fut = sandbox.read_workspace_file(session_id, "/workspace/progress.txt");
    let bytes = match tokio::time::timeout(RECITE_READ_TIMEOUT, read_fut).await {
        Err(_) => {
            let _ = events.emit_misc(session_id, "progress_recite_skipped",
                json!({"reason": "slow_read"})).await;
            return;
        }
        Ok(Err(SandboxError::NotFound(_))) | Ok(Err(SandboxError::Empty)) => {
            let _ = events.emit_misc(session_id, "progress_recite_skipped",
                json!({"reason": "missing_or_empty"})).await;
            return;
        }
        Ok(Err(e)) => {
            let _ = events.emit_misc(session_id, "progress_recite_skipped",
                json!({"reason": e.to_string()})).await;
            return;
        }
        Ok(Ok(b)) => b,
    };
    let s = String::from_utf8_lossy(&bytes);
    let tail = s.lines().rev().take(RECITE_TAIL_LINES)
        .collect::<Vec<_>>().into_iter().rev()
        .collect::<Vec<_>>().join("\n");
    let _ = events.emit_misc(session_id, "progress_recite",
        json!({"progress_path": "/workspace/progress.txt", "content_preview": tail})).await;
}
```

### 2. Wire into AgentRunner

In `AgentRunner::run`, at the top of each iteration **after** building
messages but **before** the LLM call:

```rust
for step in 0..req.max_steps {
    if ReciteScheduler::should_fire(step) {
        recite_tick(&self.sandbox, &*self.events, &req.session_id).await;
    }
    let messages = self.build_messages(&req.session_id).await?;  // now sees the recite event
    ...
}
```

The recite event is emitted *before* `build_messages` is called for the
same iteration — so when `build_messages` queries recent events, the
recite event is included.

### 3. Misc-kind documentation

Append `progress_recite, progress_recite_skipped` to the documented
`Misc.kind` set in `crates/seasoned-hand-core/src/events/misc.rs`.

### 4. Tests

`agent::recite::tests`:

- Unit tests on `ReciteScheduler::should_fire`: `(0..30).map(|s| (s,
  should_fire(s)))` produces the expected pattern.
- Integration test with a real sandbox stub (using the existing
  `mockito` or in-process test sandbox) covering:
  - tail trimming to 80 lines,
  - missing file → skipped,
  - slow read → skipped (use `tokio::time::pause` + `advance`).

Integration test with a full `AgentRunner`:

- Drive a wiremock'd LLM that emits `idle` on iteration 11.
- Pre-populate `progress.txt` with 5 lines.
- Assert exactly one `Misc{kind:"progress_recite"}` event in the stream
  at event order < step-10's LLM call.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core agent::recite::
cargo test -p seasoned-hand-core agent::tests::recite_integration  # if added there
./scripts/spec-check.sh
```

---

## Files changed

- `crates/seasoned-hand-core/src/agent/mod.rs` (modify — `pub mod recite;`
  + call from loop)
- `crates/seasoned-hand-core/src/agent/recite.rs` (new)
- `crates/seasoned-hand-core/src/agent/tests.rs` (modify — integration test)
- `crates/seasoned-hand-core/src/events/misc.rs` (modify — document kinds)

---

## Spec references

- `/specs/phase-1/architecture.md` §2.3 row 4, §3.4.
- `/specs/00-philosophy/PRINCIPLES.md` #4 (recitation).

---

## Commit message

```
feat(phase-1): story 1.6 - context recitation (PRINCIPLE #4)

- agent::recite::{ReciteScheduler, recite_tick}: every 10 worker
  iterations, read /workspace/progress.txt tail (last 80 lines), emit
  Misc{kind:"progress_recite", progress_path, content_preview}; emitted
  *before* build_messages so the next iteration consumes it
- Failure modes: missing/empty/slow-read all emit
  Misc{kind:"progress_recite_skipped", reason} and the loop continues
- 1-second read timeout prevents recite from stalling tool dispatch
- 5 unit + 1 integration test

refs: /specs/phase-1/stories/story-1.6.md
```

---

## Notes for next story (1.7)

PRINCIPLE #4 is now enforced. The agent context grows by one Misc event
every 10 iterations on long tasks — at 50 steps that's 4 extra events
totalling ≤ ~6KB. Well under the 200KB sticky-context budget.

Story 1.7 (capability fallback) is independent and a parallelisable
follow-up. The Verifier-prep track (1.7 → 1.8 → 1.9) can start now.
