# ADR-014: ARCHITECTURE.md v1.3 -> v1.4 (Phase 5 V013 tenant/RBAC reconciliation)

Status: Accepted
Date: 2026-05-20
Deciders: Project lead

## Context

Phase 5 introduces the first multi-user organizational runtime slice. The roadmap contract requires:

1. multi-tenant data model (organization -> user),
2. role-based access (`admin`, `user`, `viewer`),
3. SOP/playbook sharing inside org,
4. task handoff + delegation audit,
5. per-user cost tracking.

From Phase 2 onward, `tenant_id` columns were intentionally introduced as nullable forward-compat
fields. Phase 5 is the planned tightening point. If V013 migration ships without same-slice ADR +
ARCH reconciliation, architecture text would again drift from runtime schema/policy surfaces.

ADR-012 and ADR-013 established the reconciliation discipline used here.

## Decision

Bump `/specs/01-architecture/ARCHITECTURE.md` from **v1.3** to **v1.4** and reconcile with V013:

1. **§2.5 schema surface** gains:
   - org/user/role domain tables:
     - `organizations`
     - `users`
     - `organization_memberships`
     - `project_role_overrides`
   - collaboration sharing ACL tables:
     - `sop_shares`
     - `playbook_shares`
   - immutable operation ledger:
     - `audit_log`
   - per-user billing rollups:
     - `user_cost_ledger`
   - tenant-safe event projection:
     - `tenant_event_view`
2. **tenant tightening**: Phase 2-4 mutable tables flip `tenant_id` from nullable to NOT NULL via
   deterministic backfill + validation checks.
3. **§2.1 event taxonomy note** is expanded for Phase 5 multi-user/audit `Misc` kinds while keeping
   append-only stream invariants.
4. **atomic-slice rule reaffirmed**: V013 migration + ADR-014 + ARCH v1.4 amendments ship together.

## Consequences

Positive:
- Immutable architecture remains truthful to runtime multi-user schema.
- PM story breakdown can depend on concrete V013 surfaces.
- Tenant boundary policy moves from implied behavior to explicit architecture.

Negative:
- Schema complexity and migration surface area increase materially.
- Authorization and query paths require stricter policy/test coverage.

Neutral:
- No stack pivot; Rust/Axum/SQLite/Redis/Bifrost architecture remains unchanged.

## Migration steps (V013 contract)

1. Create new multi-user tables (`organizations`, `users`, memberships/overrides, sharing ACL,
   audit, cost ledger, tenant event projection).
2. Backfill tenant ids for existing rows from canonical parent relations.
3. Route unresolved legacy rows to deterministic sentinel tenant with audit warning records.
4. Rebuild target tables as needed to enforce NOT NULL `tenant_id` constraints.
5. Run integrity checks (NULL count = 0, tenant-chain equality checks, cross-tenant ref checks).
6. Keep rollback path at transaction boundary and archive validation query outputs.

## Alternatives considered

### Alternative A: Keep nullable tenant columns and enforce only in app layer
Rejected: violates Phase 5 headline requirement and leaves persistent drift risk.

### Alternative B: Defer org/user schema to Phase 6
Rejected: conflicts with roadmap acceptance criteria for 5-person shared-instance operation.

### Alternative C: Ship V013 first, ADR/ARCH updates later
Rejected: violates AGENTS reconciliation rule and the ADR-012/013 pattern.

## References

- `/specs/phase-5/requirements.md` (F-5.1..F-5.24, NFR-5.1..NFR-5.8)
- `/specs/phase-5/architecture.md` (§3, §4, §7, §13, §14)
- `/specs/phase-5/OPEN_QUESTIONS.md` (#1-#16)
- `/specs/phase-5/DEBT.md` (carry-forward mapping)
- `/specs/SECURITY_REVIEW.md` (Phase 4 iter-3 multi-tenant event redaction carry-in)
- ADR-012, ADR-013 (reconciliation precedents)
- AGENTS.md §8 (same-PR spec/code reconciliation)
