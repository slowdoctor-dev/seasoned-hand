# Story 5.2 — Atomic slice: V013 + ADR-014 + ARCH v1.4 + tenant backfill

> **Status**: ready
> **Estimated**: 3 hours
> **Dependencies**: 5.1
> **Phase**: 5
> **Type**: backend+docs

---

## Goal

Land the Phase 5 atomic schema/spec slice in one PR: V013 migration, ADR-014 (already drafted),
ARCHITECTURE.md v1.3→v1.4 reconciliation (already drafted), deterministic tenant backfill across
all Phase 2-4 mutable tables, and the integrity-check gates from architecture §3.5. Mirrors
Phase 4 story 4.2's discipline (ADR-013 / V011 atomic slice).

## Acceptance criteria

- [ ] `migrations/V013__phase5_tenant_rbac_audit.sql` extends the Architect skeleton with:
  - new tables (organizations, users, organization_memberships, project_role_overrides,
    sop_shares, playbook_shares, audit_log, user_cost_ledger, tenant_event_view) — already in
    skeleton.
  - **tenant backfill SQL** per architecture §3.5 (derive from canonical parent joins; fallback
    to `legacy-default` sentinel with audit warning records).
  - **table-rebuild NOT NULL flips** for every Phase 2-4 surface listed in §3.4
    (projects, tasks, deliverables, intake_events, deliveries, notifications, sessions,
    skills, playbooks, plus all Phase 4 curator tables from §3.4).
  - **integrity checks** asserting zero NULL `tenant_id` rows, FK-consistent tenant chains,
    and zero cross-tenant curator/revision references (§3.5 SQL examples).
  - **ALTER session_search_index** adding `tenant_id` + `visibility_level` columns (NOTE step
    (4) from skeleton — paired FTS trigger update).
- [ ] `ADR-014` and `ARCHITECTURE.md` v1.4 already shipped via Architect commit `7ed1009`;
      this story confirms no drift.
- [ ] Migration is idempotent: re-running V013 via `refinery::embed_migrations!` produces no
      schema delta and no integrity failure.
- [ ] Backfill audit emits `Misc{kind:"tenant_backfill_sentinel"}` per row routed to
      `legacy-default` so operators can remediate.

## Non-goals

- AuthContext resolver runtime (story 5.3).
- Org/user runtime CRUD (story 5.4).
- RBAC enforcement (stories 5.5/5.6).

## Implementation steps

1. Extend V013 SQL with backfill + integrity checks + session_search_index ALTER.
2. Add migration regression test
   `db::tests::migration_v013_creates_phase5_multiuser_tables_and_backfills`.
3. Verify idempotent re-run via embedded runner test.
4. Confirm ARCH v1.4 amendment paragraph + ADR-014 are referenced from `specs/01-architecture/`.

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
bash scripts/spec-check.sh
```

## Refs

- requirements: F-5.1, F-5.2, F-5.3, NFR-5.8
- architecture: §3, §13
- debt closed: —
