# Story 5.31 — phase5_v013_migration_harness (NFR-5.8)

> **Status**: done
> **Estimated**: 2.5 hours
> **Dependencies**: 5.2
> **Phase**: 5
> **Type**: test+migration

---

## Goal

Verify NFR-5.8: V013 applies from a Phase 4 baseline DB fixture with deterministic backfill
and no destructive data loss.

## Acceptance criteria

- [ ] Test fixture: a SQLite DB seeded to the Phase 4 V012 state with N projects, M tasks, K
      curator decisions (some with NULL tenant_id, some without).
- [ ] Apply V013; assert:
  - zero NULL tenant_id rows post-migration;
  - tenant chain integrity (task tenant == project tenant, etc.);
  - org/user bootstrap rows created (`legacy-default` sentinel + initial admin);
  - rollback-safe: replay V013 idempotently.
- [ ] Asserts `tenant_backfill_sentinel` audit events emitted for each unresolved row.
- [ ] CI budget < 3 min.

## Verification

```bash
cargo test -p seasoned-hand-core phase5_v013_migration_harness
```

## Refs

- requirements: F-5.3, NFR-5.8
- architecture: §15 harness 6
