# Story 3.16 — Phase 3 acceptance gate and close-out

> **Status**: done
> **Estimated**: 2 hours
> **Dependencies**: 3.12, 3.13, 3.14, 3.15
> **Phase**: 3
> **Type**: test+docs

---

## Goal

Close Phase 3 with deterministic acceptance: `phase3_warm_benchmark` pass at
`<=0.70 x cold_baseline`, regression gates green, and `spec-check.sh` updated for
Phase 3 version-hook discipline.

## Acceptance criteria

- [x] `cargo test phase3_warm_benchmark` asserts and passes `sessions.tool_calls <= 0.70 x cold_baseline`.
- [x] Acceptance suite depends on parity + production-matcher + FTS5-trigger regressions.
- [x] `scripts/spec-check.sh` Phase 3 hook is added/updated (carry-forward DEBT #62).
- [x] Phase 3 close-out notes update DEBT statuses and acceptance evidence.
- [x] Full `bash scripts/spec-check.sh` remains 7/7.

## Non-goals

- Phase 4 curator policy work.

---

## Implementation steps

1. Wire final warm-benchmark assertion and CI gate path.
2. Update spec-check phase-version hook.
3. Land close-out documentation updates and debt status flips.

---

## Verification

```bash
cargo test phase3_warm_benchmark
cargo test sessions_tool_calls_matches_action_count
cargo test phase3_production_matcher_smoke
cargo test -p seasoned-hand-core fts5::trigger_correctness
bash scripts/spec-check.sh
```

---

## Refs

- requirements: F-3.3, F-3.6
- architecture: §10.6, §11
- debt closed: Phase 2 DEBT #62

## Notes from execution

- Acceptance evidence was captured by running:
  - `cargo test phase3_warm_benchmark`
  - `cargo test sessions_tool_calls_matches_action_count`
  - `cargo test phase3_production_matcher_smoke`
  - `cargo test -p seasoned-hand-core fts5::trigger_correctness`
  - `bash scripts/spec-check.sh`
