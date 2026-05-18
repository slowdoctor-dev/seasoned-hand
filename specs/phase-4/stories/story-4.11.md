# Story 4.11 — Curator failure containment + adversarial confidence bounds

> **Status**: ready
> **Estimated**: 2.5 hours
> **Dependencies**: 4.3, 4.4, 4.5, 4.6
> **Phase**: 4
> **Type**: backend+test

---

## Goal

Implement 7-category failure containment and adversarial confidence guardrails as production logic.

## Acceptance criteria

- [ ] All 7 F-4.22 categories map to concrete runtime handling paths.
- [ ] Quarantine + retry/backoff behavior is implemented and observable.
- [ ] Compositional confidence bounds from §9.1 are enforced in decision pipeline.
- [ ] Integration tests cover each failure category and adversarial high-confidence attempt.

## Non-goals

- Benchmark optimization.

---

## Implementation steps

1. Add failure category discriminants and handlers.
2. Add bounded confidence composer with deterministic floor checks.
3. Add failure-category integration tests.

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
bash scripts/spec-check.sh
```

## Refs

- requirements: F-4.22, F-4.18, NFR-4.1
- architecture: §8, §9.1
- debt closed: #89 (close), #100 (close)
