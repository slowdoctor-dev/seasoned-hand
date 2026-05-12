# Story 0.15 — Stuck detection (real pump)

> **Status**: ready
> **Estimated**: 1 hour
> **Dependencies**: story 0.14 (agent runner)
> **Phase**: 0
> **Type**: backend
> **Reads first**: `/specs/phase-0/stories/story-0.14.md` (scaffold in §3), `/specs/00-philosophy/PRINCIPLES.md` #10

---

## Goal

Replace the stuck-detection scaffold from story 0.14 with the real
pump: when the LLM produces 2+ identical assistant messages in a row,
inject a strategy-change system prompt before the next iteration. If
duplicates reach 4 in a row, hard-terminate the session as ERROR.

## Acceptance criteria

- [ ] `agent::stuck` (or inline in `agent/mod.rs`) module:
      - `StuckTracker { last_hash: Option<u64>, duplicate_count: u32 }`
      - `observe(message: &AssistantMessage) -> StuckAction` where
        `StuckAction` enum has `Continue`, `InjectStrategyPrompt`, `Terminate`
- [ ] Threshold constants: `STUCK_WARN_AT = 2` (inject), `STUCK_HARD_AT = 4` (terminate)
- [ ] Hash function: stable SipHash over `(role, content_normalized, tool_calls_signature)`
      where content_normalized trims whitespace and collapses
      consecutive whitespace; tool_calls_signature is a sorted list of
      `(name, arguments)` tuples
- [ ] When `InjectStrategyPrompt` fires:
      - The agent runner prepends a system message to the next
        iteration's `messages`: `"You have repeated the same response
        N times. Try a different strategy: consider a different tool,
        re-read recent observations, or call message_ask_user to clarify."`
      - A `Misc` event with `{kind: "stuck_inject", duplicate_count: N}`
        is emitted
- [ ] When `Terminate` fires (count ≥ 4):
      - Session state → ERROR
      - A `Misc` event with `{kind: "stuck_terminate", duplicate_count: N}`
      - Runner returns `AgentError::Cancelled` or a new
        `AgentError::StuckTerminated` with the count
- [ ] Counter resets to 0 on any non-duplicate response
- [ ] Unit tests:
      - `single_unique_response_returns_continue`
      - `two_duplicates_returns_inject`
      - `three_duplicates_still_inject_with_higher_count`
      - `four_duplicates_returns_terminate`
      - `alternating_responses_keep_counter_at_zero`
      - `whitespace_differences_count_as_duplicate`
      - `tool_args_differ_count_as_unique`
      - `agent_runner_emits_stuck_inject_event_and_continues` (full
        runner loop test via wiremock'd Bifrost serving the same
        tool_call response twice, asserts a `Misc{kind:"stuck_inject"}`
        event appears and the run continues to a 3rd iteration)
      - `agent_runner_terminates_on_four_duplicates` (full runner
        test: serves identical responses 4 times, asserts session
        ends ERROR)
- [ ] DEBT.md update: close the "stuck-detection scaffold pending"
      entry from story 0.14
- [ ] `cargo clippy / fmt / test / spec-check` all pass

## Non-goals

- Stuck detection across sessions (Phase 1+)
- Adaptive strategy prompts based on recent error patterns (Phase 1+)
- Detection in the LLM's reasoning (Phase 1+; we only look at the
  emitted `AssistantMessage`)

---

## Implementation steps

### 1. Module

```rust
// crates/seasoned-hand-core/src/agent/stuck.rs
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use crate::llm::AssistantMessage;

pub const STUCK_WARN_AT: u32 = 2;
pub const STUCK_HARD_AT: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StuckAction {
    Continue,
    InjectStrategyPrompt { count: u32 },
    Terminate { count: u32 },
}

#[derive(Default)]
pub struct StuckTracker {
    last_hash: Option<u64>,
    duplicate_count: u32,
}

impl StuckTracker {
    pub fn observe(&mut self, msg: &AssistantMessage) -> StuckAction {
        let h = hash_message(msg);
        if Some(h) == self.last_hash {
            self.duplicate_count += 1;
        } else {
            self.duplicate_count = 0;
            self.last_hash = Some(h);
            return StuckAction::Continue;
        }
        if self.duplicate_count >= STUCK_HARD_AT - 1 {
            // 0 duplicates after the original = 1st; HARD_AT=4 means 4 identical messages.
            // Hash matched 3 times after the original → 4 total messages.
            StuckAction::Terminate { count: self.duplicate_count + 1 }
        } else if self.duplicate_count >= STUCK_WARN_AT - 1 {
            StuckAction::InjectStrategyPrompt { count: self.duplicate_count + 1 }
        } else {
            StuckAction::Continue
        }
    }
}

fn hash_message(m: &AssistantMessage) -> u64 {
    let mut hasher = DefaultHasher::new();
    "assistant".hash(&mut hasher);
    let content = m.content.as_deref().unwrap_or("");
    normalize_whitespace(content).hash(&mut hasher);
    let mut sig: Vec<(String, String)> = m
        .tool_calls
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .map(|c| (c.function.name.clone(), c.function.arguments.clone()))
        .collect();
    sig.sort();
    sig.hash(&mut hasher);
    hasher.finish()
}

fn normalize_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
```

### 2. Wire into AgentRunner

Replace the scaffold from story 0.14:

```rust
let action = self.stuck.observe(asst);
match action {
    StuckAction::Continue => {}
    StuckAction::InjectStrategyPrompt { count } => {
        self.emit_misc_with("stuck_inject", json!({"duplicate_count": count})).await?;
        next_iteration_extra_system.push(format!(
            "You have repeated the same response {count} times. Try a different strategy: \
             a different tool, re-read recent observations, or call message_ask_user."
        ));
    }
    StuckAction::Terminate { count } => {
        self.emit_misc_with("stuck_terminate", json!({"duplicate_count": count})).await?;
        self.set_session_state(&req.session_id, "ERROR").await?;
        return Err(AgentError::StuckTerminated { count });
    }
}
```

`AgentError::StuckTerminated { count: u32 }` is new.

### 3. Tests

Pure-unit tests over `StuckTracker::observe()`. Then two full
agent_runner integration tests via wiremock'd Bifrost.

---

## Files changed

- `crates/seasoned-hand-core/src/agent/mod.rs` (modify)
- `crates/seasoned-hand-core/src/agent/stuck.rs` (new)
- `crates/seasoned-hand-core/src/agent/tests.rs` (modify — add 2 runner tests)
- `specs/phase-0/DEBT.md` (close 0.14 stuck-detection entry)

---

## Spec references

- `/specs/phase-0/stories/story-0.14.md` §3 (stuck scaffold)
- `/specs/00-philosophy/PRINCIPLES.md` #10 (failure-tolerant), #11 (audit trail)

---

## Commit message

```
feat(phase-0): story 0.15 - stuck detection (real pump)

- agent::stuck::StuckTracker tracks last assistant-message hash +
  duplicate count; observe() returns Continue/InjectStrategyPrompt/
  Terminate per WARN_AT=2 / HARD_AT=4 thresholds
- Hash: SipHash over (role, normalized content, sorted tool-call
  signature); whitespace differences count as duplicate, tool-arg
  differences count as unique
- AgentRunner: on InjectStrategyPrompt, emit Misc{kind:"stuck_inject"}
  and prepend a strategy-change system message to the next iteration's
  context; on Terminate, set session ERROR + emit Misc and return
  AgentError::StuckTerminated
- 7 unit tests on StuckTracker + 2 full-runner tests via wiremock'd
  Bifrost
- cargo clippy / fmt / test / spec-check all pass

Debt: closes 0.14's stuck-detection-scaffold entry in DEBT.md.

refs: /specs/phase-0/stories/story-0.15.md
```
