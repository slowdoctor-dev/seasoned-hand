# Story 4.14 — Strict SH_CURATOR_* config parsing + feature flags

> **Status**: done
> **Estimated**: 2 hours
> **Dependencies**: 4.3
> **Phase**: 4
> **Type**: backend+test

---

## Goal

Implement strict parsing for Curator flags and split embedding cap config into soft/hard fields.

## Acceptance criteria

- [x] `SH_CURATOR_*` flags are strictly parsed with explicit invalid-value errors.
- [x] Config surface includes `embedding_budget_soft_cap_pct` and
      `embedding_budget_hard_breaker_pct`.
- [x] Zero-baseline fallback budget behavior is implemented.
- [x] Integration test covers valid/invalid env values and breaker behavior.

## Non-goals

- Global config framework rewrite.

---

## Implementation steps

1. Add strict parsing helpers and typed config fields.
2. Wire flags through CuratorWorker and reranker.
3. Add env parsing + breaker tests.

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
bash scripts/spec-check.sh
```

## Refs

- requirements: F-4.23, NFR-4.6
- architecture: §4.1, §6.5, §7
- debt closed: #91 (close), #98 (close)
