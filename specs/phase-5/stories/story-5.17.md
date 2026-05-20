# Story 5.17 — Curator tenant boundaries + failure taxonomy

> **Status**: ready
> **Estimated**: 3 hours
> **Dependencies**: 5.2, 5.4
> **Phase**: 5
> **Type**: backend+curator

---

## Goal

Apply tenant filters to every curator read/write per architecture §8. Implement the F-5.14
failure taxonomy: `tenant_unresolved` quarantine, `cross_tenant_ref` rejection, full-cycle
`curator_cycle_refused`.

## Acceptance criteria

- [ ] Every curator SQL query in `crate::curator::*` has explicit `tenant_id = :tenant`.
- [ ] `validate_decision_scope` (already exists from Phase 4 story 4.15) extended to also
      reject cross-tenant references (not just cross-project).
- [ ] Failure modes emit deterministic events:
  - `Misc{kind:"curator_decision_quarantined", failure_category:"tenant_unresolved", ...}`
  - `Misc{kind:"curator_decision_quarantined", failure_category:"cross_tenant_ref", ...}`
  - `Misc{kind:"curator_cycle_refused", failure_category:"tenant_unresolved", ...}`
- [ ] Tests cover all three failure modes.

## Verification

```bash
cargo test -p seasoned-hand-core curator::tenant_boundaries
```

## Refs

- requirements: F-5.14
- architecture: §8
