# Story 4.8 — WorkPatternExtractor + pattern-to-recommendation loop

> **Status**: done
> **Estimated**: 2.5 hours
> **Dependencies**: 4.2, 4.3
> **Phase**: 4
> **Type**: backend+test

---

## Goal

Implement WorkPatternExtractor and produce recommendation records that connect recurring patterns
to playbook revision improvement opportunities.

## Acceptance criteria

- [ ] Production WorkPatternExtractor exists (not trait-only/test-only scaffold) and mines patterns
      with hybrid signal source.
- [ ] Recommendation write path links patterns to revision/proposal subjects.
- [ ] Runtime wiring is enabled from `main.rs` via CuratorWorker dependency graph.
- [ ] Integration test validates deterministic pattern ranking from fixture events in end-to-end
      curator cycle execution.

## Non-goals

- Auto-merge decisions.

---

## Implementation steps

1. Build hybrid replay + aggregate pattern miner.
2. Add recommendation emission path.
3. Add integration tests for pattern extraction and recommendation creation.

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
bash scripts/spec-check.sh
```

## Refs

- requirements: F-4.19, F-4.20
- architecture: §2.7, §4.3, §12.8, §12.20
- debt closed: —
