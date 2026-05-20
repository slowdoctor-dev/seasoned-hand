# Story 5.9 — Task hand-off lifecycle (pause -> transfer -> resume)

> **Status**: done
> **Estimated**: 3 hours
> **Dependencies**: 5.5, 5.6
> **Phase**: 5
> **Type**: backend+state-machine

---

## Goal

Implement the task hand-off state machine per architecture §5. Running tasks must follow
pause → transfer → resume; non-running terminal states reject reassignment.

## Acceptance criteria

- [ ] `crate::handoff::task::{handoff, can_handoff}` core service.
- [ ] State transition guards:
  - Drafted/Briefed/Confirmed/Paused → direct reassignment allowed.
  - Running → enforce pause first; reject with actionable error if not paused.
  - Completed/Failed/Cancelled → reject (terminal).
- [ ] Atomic update: task owner field + `task_paused_for_handoff` event + audit_log row in one
      transaction.
- [ ] CLI: `seasoned-hand task handoff <task_id> --to <user_email> [--reason "..."]`.
- [ ] Optimistic concurrency: `expected_updated_at` precondition (paired with 5.21).

## Verification

```bash
cargo test -p seasoned-hand-core handoff::task::tests
```

## Refs

- requirements: F-5.8
- architecture: §5
