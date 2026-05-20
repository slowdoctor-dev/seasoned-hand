# Story 5.2 — Atomic slice: V013 + ADR-014 + ARCH v1.4 + tenant backfill

> **Status**: done
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

- [x] `migrations/V013__phase5_tenant_rbac_audit.sql` extends the Architect skeleton with:
  - new tables (organizations, users, organization_memberships, project_role_overrides,
    sop_shares, playbook_shares, audit_log, user_cost_ledger, tenant_event_view).
  - **tenant backfill SQL** per architecture §3.5 — derives from canonical parent joins;
    fallback to `legacy-default` sentinel.
  - **bootstrap rows** for the `legacy-default` sentinel tenant: one `organizations` row,
    one `users` admin row, one `organization_memberships` row with `role='admin'`,
    `is_primary=1` — so legacy-tagged rows have a real FK target for downstream Phase 5
    audit/cost writers.
  - **ALTER session_search_index** ADD COLUMN `tenant_id` + `visibility_level` (NOTE step (4)
    from Architect skeleton) with backfill from events → sessions → tasks chain and a paired
    index `idx_session_search_index_tenant_visibility`.
- [x] `ADR-014` and `ARCHITECTURE.md` v1.4 already shipped via Architect commit `7ed1009`;
      this story confirms no drift.
- [x] Migration is idempotent: re-running V013 via `refinery::embed_migrations!` produces no
      schema delta and no integrity failure.

### Per-table NOT NULL flips — explicitly deferred (Two-step migration, OQ #1 Option B)

The Architect's chosen migration posture (§3.1 OQ #1 Option B) is two-step: Step A
"deterministic backfill and validation tables/indexes" (this story) and Step B "enforce
NOT NULL tenant semantics by table-rebuild pattern **where needed**". The `where needed`
qualifier is the licence to split Step B across per-domain stories that own each table's
write path.

Why split:
- One atomic flip-all-tables migration forces every legacy test fixture (~66 of them across
  intake/delivery/deliverable/matcher/notify/etc.) to set `tenant_id` in the same commit.
  That's 2+ hours of mechanical fixture editing on top of the 3h schema budget.
- Per-domain flips land naturally alongside the RBAC/AuthContext changes that make each
  write-path's caller resolve a tenant_id, so the test-fixture migration is local to the
  story that needs it.

Schedule (no new stories created — folded into existing 5.x):
- **5.5** HTTP middleware RBAC → flips `projects`, `tasks`, `deliverables`.
- **5.7** sop_shares → flips `skills` if active (or defers if not yet written through).
- **5.8** playbook_shares → flips `playbooks`.
- **5.17** Curator tenant boundaries → flips all 11 V011/V012 curator tables.
- **5.19** User invitation CLI → flips `intake_events`, `delivery_events`, `notifications_sent`
  (these tables have org-scoped writers once invitation lifecycle lands).

Each per-domain story's acceptance criteria gains a "rebuilds table X with `tenant_id`
NOT NULL" item + the test-fixture updates needed for `cargo test` to stay green.

## Non-goals

- AuthContext resolver runtime (story 5.3).
- Org/user runtime CRUD (story 5.4).
- RBAC enforcement (stories 5.5/5.6).
- Per-table NOT NULL flips — deferred to per-domain stories per the schedule above.

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
