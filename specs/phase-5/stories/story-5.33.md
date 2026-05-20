# Story 5.33 — Phase 5 acceptance gate + close-out

> **Status**: done
> **Estimated**: 2.5 hours
> **Dependencies**: 5.2-5.32
> **Phase**: 5
> **Type**: release+docs

---

## Goal

Run Phase 5 acceptance criteria end-to-end, close the F-5.* debt accountability matrix
(mirroring Phase 4's F-4.26), and publish phase close-out state updates. Mirrors Phase 4 story
4.22.

## Acceptance criteria

- [ ] Phase-level acceptance criteria in requirements §5 (six criteria) executed and recorded.
- [ ] `scripts/spec-check.sh` includes Phase 5 close-out hooks (V013, org/user tables,
      tenant_event_view, audit_log, user_cost_ledger, ARCH v1.4).
- [ ] `BASELINE.md`, `AGENTS.md` §13, and `CHANGELOG.md` flipped to "Phase 5 complete → Phase 6
      starting".
- [ ] DEBT close-out matrix for #76/#91/#92/#93/#94/#96/#97/#S-1 + any Phase 5-introduced
      entries recorded in `specs/phase-5/DEBT.md`.

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
bash scripts/spec-check.sh
```

## Refs

- requirements: §5 acceptance criteria (all six)
- architecture: §15 (all harnesses must pass)
- debt closed: full Phase 5 close-out matrix
