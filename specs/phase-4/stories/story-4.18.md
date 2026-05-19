# Story 4.18 — curator_search_fts maintenance-trigger correctness regression

> **Status**: done
> **Estimated**: 1.5 hours
> **Dependencies**: 4.2
> **Phase**: 4
> **Type**: test

---

## Goal

Validate INSERT/UPDATE/DELETE maintenance triggers for `curator_search_fts` and rebuild behavior.

## Acceptance criteria

- [ ] Trigger tests prove `ai/ad/au` correctness.
- [ ] Updated text becomes searchable; deleted text disappears.
- [ ] Rebuild operation yields same corpus as trigger-fed index.

## Non-goals

- Query ranking retune.

---

## Implementation steps

1. Add migration tests for FTS trigger behavior.
2. Add rebuild parity test.

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
bash scripts/spec-check.sh
```

## Refs

- requirements: F-4.16
- architecture: §3.3, §11.2
- debt closed: #76 (partial verification)
