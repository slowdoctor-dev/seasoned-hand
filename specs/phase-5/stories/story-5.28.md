# Story 5.28 — phase5_handoff_lifecycle_harness + phase5_curator_tenant_failure_harness

> **Status**: done
> **Estimated**: 3 hours
> **Dependencies**: 5.9, 5.17
> **Phase**: 5
> **Type**: test

---

## Goal

Two harnesses bundled (both small):

1. `phase5_handoff_lifecycle_harness`: running task → pause → transfer → resume sequence,
   audit_log row + owner update atomicity.
2. `phase5_curator_tenant_failure_harness`: assert all three F-5.14 categories
   (`tenant_unresolved`, `cross_tenant_ref`, `curator_cycle_refused`) emit correctly with
   correct quarantine behavior.

## Acceptance criteria

- [ ] Handoff harness drives a Running task through pause-transfer-resume and asserts:
  - new owner set, old owner cleared,
  - `task_paused_for_handoff` event + `task_handoff_completed` Misc event + audit_log row,
  - resume by new owner works.
- [ ] Curator failure harness seeds each of the 3 failure conditions and asserts the
      corresponding Misc event lands with the right `failure_category`.
- [ ] CI budget < 4 min total for both.

## Verification

```bash
cargo test -p seasoned-hand-core phase5_handoff_lifecycle_harness
cargo test -p seasoned-hand-core phase5_curator_tenant_failure_harness
```

## Refs

- requirements: F-5.8, F-5.14
- architecture: §15 harness 3 + 7
