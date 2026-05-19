# Story 4.17 — Revision-chain integrity regression

> **Status**: done
> **Estimated**: 2 hours
> **Dependencies**: 4.2, 4.5
> **Phase**: 4
> **Type**: test

---

## Goal

Protect revision graph and revision-key consistency across outcomes, consolidation, and
recommendation provenance.

## Acceptance criteria

- [ ] `parent_revision_id` FK constraint is enforced in schema/runtime tests.
- [ ] Outcome counters key on revision_id, not playbook_id.
- [ ] Consolidation target revision and recommendation subject ids remain consistent.
- [ ] Regression tests cover orphan-parent rejection and lineage traversal.

## Non-goals

- New consolidation policies.

---

## Implementation steps

1. Add migration/schema assertion tests.
2. Add runtime consistency tests for F-4.3/F-4.6/F-4.20 alignment.

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
bash scripts/spec-check.sh
```

## Refs

- requirements: F-4.3, F-4.6, F-4.7, F-4.20
- architecture: §3.2, §12.10
- debt closed: #102 (close), #93 (close)
