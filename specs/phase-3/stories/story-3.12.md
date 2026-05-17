# Story 3.12 — Regression: sessions tool-call parity guard

> **Status**: done
> **Estimated**: 1.5 hours
> **Dependencies**: 3.1
> **Phase**: 3
> **Type**: test

---

## Goal

Protect acceptance-metric trust by adding `sessions_tool_calls_matches_action_count`
regression coverage.

## Acceptance criteria

- [ ] Test `sessions_tool_calls_matches_action_count` exists and passes.
- [ ] Test validates `sessions.tool_calls` equals counted Action events for baseline fixture.
- [ ] Failure output clearly reports mismatch and expected/actual counts.

## Non-goals

- Warm benchmark pass/fail gate itself.

---

## Implementation steps

1. Add regression test fixture setup.
2. Compare canonical counter vs action-event count.
3. Add deterministic assertions and diagnostics.

---

## Verification

```bash
cargo test sessions_tool_calls_matches_action_count
```

---

## Refs

- requirements: F-3.6
- architecture: §11
