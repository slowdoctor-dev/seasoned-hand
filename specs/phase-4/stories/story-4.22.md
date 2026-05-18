# Story 4.22 — Phase 4 acceptance gate + close-out

> **Status**: ready
> **Estimated**: 2.5 hours
> **Dependencies**: 4.2-4.21
> **Phase**: 4
> **Type**: release+docs

---

## Goal

Run Phase 4 acceptance gates end-to-end, close debt-accountability matrix, and publish phase
close-out state updates.

## Acceptance criteria

- [ ] Phase-level acceptance criteria in requirements §5 are executed and recorded.
- [ ] `scripts/spec-check.sh` includes Phase 4 close-out hooks needed at release boundary.
- [ ] `BASELINE.md`, `AGENTS.md` §13, and `CHANGELOG.md` are updated to "Phase 4 complete ->
      Phase 5 starting".
- [ ] DEBT close-out matrix for #72/#73/#75/#76/#77/#87/#88/#89/#90/#91 and #92-#102 is recorded
      with close/partial/defer evidence per F-4.26.

## Non-goals

- Phase 5 implementation.

---

## Implementation steps

1. Run acceptance test suite and collect metrics.
2. Update phase status docs and changelog.
3. Finalize DEBT closure matrix and residual deferrals.

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
bash scripts/spec-check.sh
```

## Refs

- requirements: F-4.26, §5 acceptance criteria
- architecture: §10, §11, §13
- debt closed: #72, #73, #75, #76, #77, #87, #88, #89, #90, #91, #92, #93, #94, #95, #96, #97, #98, #99, #100, #101, #102
