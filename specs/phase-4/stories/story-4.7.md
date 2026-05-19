# Story 4.7 — RetrospectiveGenerator weekly cadence + citation validator

> **Status**: done
> **Estimated**: 3 hours
> **Dependencies**: 4.2, 4.3, 4.6
> **Phase**: 4
> **Type**: backend+test

---

## Goal

Ship production RetrospectiveGenerator with weekly minimum cadence, activity-triggered extras,
strict citation coverage validation, and refusal on weak evidence.

## Acceptance criteria

- [ ] Production RetrospectiveGenerator exists (not trait-only/test-only scaffold) and persists
      weekly outputs.
- [ ] Citation tag format and coverage calculation follow architecture §12.16.
- [ ] Weekly schedule + retry semantics implemented.
- [ ] Runtime wiring is enabled from `main.rs` via CuratorWorker dependency graph.
- [ ] Integration test covers success/refusal/retry behavior with stub summaries in end-to-end
      curator cycle execution.

## Non-goals

- Work-pattern recommendations.

---

## Implementation steps

1. Implement generator over session_search + curator ledger inputs.
2. Implement citation extractor/validator and refusal path.
3. Implement cadence and retry controls.

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
bash scripts/spec-check.sh
```

## Refs

- requirements: F-4.17, F-4.18, NFR-4.8
- architecture: §2.6, §3.2, §4.4, §11.3, §12.6, §12.7, §12.16
- debt closed: #75 (partial), #89 (partial)
