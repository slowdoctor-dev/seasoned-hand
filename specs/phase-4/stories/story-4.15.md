# Story 4.15 — Project-scope isolation enforcement regression

> **Status**: ready
> **Estimated**: 1.5 hours
> **Dependencies**: 4.2, 4.3, 4.5, 4.10
> **Phase**: 4
> **Type**: test

---

## Goal

Guarantee Curator reads/writes fail closed on cross-project attempts.

## Acceptance criteria

- [ ] Integration test attempts cross-project decision and verifies rejection.
- [ ] Query-level guards are validated for merge/archive/conflict/review writes.
- [ ] No cross-project decision row can be inserted through runtime APIs.

## Non-goals

- Cross-project analytics.

---

## Implementation steps

1. Add isolation guard tests across key write paths.
2. Add negative tests for forged project ids.

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
bash scripts/spec-check.sh
```

## Refs

- requirements: F-4.24
- architecture: §9, §12.18
- debt closed: #95 (close)
