# Story 4.9 — OperatorReviewQueue + curator CLI surfaces

> **Status**: ready
> **Estimated**: 3 hours
> **Dependencies**: 4.2, 4.3, 4.5, 4.6
> **Phase**: 4
> **Type**: backend+cli+test

---

## Goal

Ship production OperatorReviewQueue and required CLI surfaces:
`curator status|run|review list|approve|reject|suppress`.

## Acceptance criteria

- [ ] Production OperatorReviewQueue runtime exists (not trait-only/test-only scaffold).
- [ ] Queue persistence/state model is live in production.
- [ ] CLI commands from architecture §4.6 are implemented.
- [ ] Confidence-band gating policy is enforced into queueing behavior.
- [ ] Runtime wiring is enabled from `main.rs` via CuratorWorker dependency graph.
- [ ] Integration test covers command flows and queue transitions in end-to-end curator cycle
      execution with stub external services.

## Non-goals

- Frontend review UI.

---

## Implementation steps

1. Implement queue service and transition handlers.
2. Implement all required CLI command handlers.
3. Add integration tests for command + queue end-to-end path.

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
bash scripts/spec-check.sh
```

## Refs

- requirements: F-4.25, F-4.23
- architecture: §2.8, §3.2, §4.6, §12.19
- debt closed: —
