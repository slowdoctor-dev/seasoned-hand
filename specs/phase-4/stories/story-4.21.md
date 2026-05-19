# Story 4.21 — phase4_warm_full_loop_benchmark

> **Status**: done
> **Estimated**: 3 hours
> **Dependencies**: 4.3, 4.4, 4.5, 4.7, 4.8, 4.10, 4.12, 4.13
> **Phase**: 4
> **Type**: test+benchmark

---

## Goal

Implement the full warm-loop benchmark harness: extract -> curate -> consolidate -> inject -> warm
cycle measurement, with CI-friendly deterministic fixtures.

## Acceptance criteria

- [ ] `phase4_warm_full_loop_benchmark` test/bench target exists and is deterministic.
- [ ] Fixture corpus includes >=200 verified artifacts replay path.
- [ ] Cold->warm deltas report precision@3 and stale ratio movement.
- [ ] Benchmark enforces acceptance proxy wall-clock budget constraints.

## Non-goals

- Live-LLM benchmark dependence.

---

## Implementation steps

1. Build deterministic fixture and replay harness.
2. Implement cold->curate->warm sequence.
3. Emit measurable results and gate assertions.

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
bash scripts/spec-check.sh
```

## Refs

- requirements: F-4.21, §5 acceptance #1/#2
- architecture: §7, §11.3, §12.20
- debt closed: #87 (close), #101 (close)
