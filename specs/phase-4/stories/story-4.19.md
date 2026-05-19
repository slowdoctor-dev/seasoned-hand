# Story 4.19 — Review queue state-machine regression + suppression TTL

> **Status**: done
> **Estimated**: 1.5 hours
> **Dependencies**: 4.9
> **Phase**: 4
> **Type**: test

---

## Goal

Protect operator queue transitions and suppression expiry behavior.

## Acceptance criteria

- [ ] Transition tests cover pending->approved/rejected/suppressed.
- [ ] Invalid transitions are rejected.
- [ ] Suppression TTL expiry returns entries to pending when configured.

## Non-goals

- UI workflow.

---

## Implementation steps

1. Add transition matrix tests.
2. Add TTL expiry timer/path tests.

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
bash scripts/spec-check.sh
```

## Refs

- requirements: F-4.25
- architecture: §3.2 (curator_review_queue), §4.6, §11.2
- debt closed: —
