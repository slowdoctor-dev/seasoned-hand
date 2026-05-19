# Story 4.20 — Embedding cost circuit-breaker regression

> **Status**: done
> **Estimated**: 2 hours
> **Dependencies**: 4.4, 4.14
> **Phase**: 4
> **Type**: test

---

## Goal

Verify NFR-4.6 breaker behavior and zero-baseline fallback budget enforcement.

## Acceptance criteria

- [ ] Soft cap behavior is observable at 8% threshold.
- [ ] Hard breaker opens at 12% and forces lexical-only fallback.
- [ ] Zero-baseline project gets 50k token startup fallback then transitions.

## Non-goals

- Cost dashboarding.

---

## Implementation steps

1. Add token-accounting fixture harness.
2. Add tests for soft-cap, hard-breaker, zero-baseline transitions.

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
bash scripts/spec-check.sh
```

## Refs

- requirements: NFR-4.6, F-4.23
- architecture: §4.2, §7, §11.2
- debt closed: #98 (verification), #99 (verification)
