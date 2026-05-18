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

Phase 3 acceptance MUST run the complete AGENTS.md §6 gate list (not just the
Phase-3-specific subset — see REVIEW iter-1 F7):

```bash
# AGENTS.md §6 (full gate set, all 6)
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
pnpm typecheck
pnpm test
bash scripts/spec-check.sh

# Phase-3-specific evidence (subset that proves the learning loop ships)
cargo test phase3_warm_benchmark
cargo test sessions_tool_calls_matches_action_count
cargo test phase3_production_matcher_smoke
cargo test -p seasoned-hand-core fts5::trigger_correctness
```

The Phase-3-specific tests are evidence that the learning machinery works; the
AGENTS.md §6 gates are evidence that Phase 3 didn't regress the rest of the
workspace. Both must pass.

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
