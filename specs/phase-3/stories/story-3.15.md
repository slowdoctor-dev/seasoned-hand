# Story 3.15 — Benchmark fixture + gate-mode harness

> **Status**: done
> **Estimated**: 2 hours
> **Dependencies**: 3.2, 3.5, 3.6
> **Phase**: 3
> **Type**: test+infra

---

## Goal

Build deterministic benchmark harness inputs for Phase 3 acceptance: fixture identity,
brief normalization invariants, and cold-baseline capture wiring.

## Acceptance criteria

- [x] Deterministic `phase2_overnight_default_path`-style fixture is wired for phase3 tests.
- [x] Gate-mode second-run identity checks same fixture ID + normalized brief.
- [x] Cold baseline lineage source for warm test assertion is fixed and documented in test.

## Non-goals

- Final <=0.70x pass assertion.

---

## Implementation steps

1. Add fixture builder and deterministic run harness utilities.
2. Implement identity assertion helpers used by warm benchmark.
3. Persist/read cold baseline reference in test constants.

---

## Verification

```bash
cargo test phase3_benchmark_fixture_identity
```

---

## Refs

- requirements: F-3.2, F-3.4
- architecture: §11
