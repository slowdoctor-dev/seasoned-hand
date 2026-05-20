# Story 5.10 — audit_log writer + admin read API

> **Status**: ready
> **Estimated**: 2.5 hours
> **Dependencies**: 5.4, 5.5
> **Phase**: 5
> **Type**: backend+api

---

## Goal

`audit_log` table already created by V013. Add the writer service (immutable inserts only) and
the admin read API per arch §4.3 matrix ("View audit log (org)").

## Acceptance criteria

- [ ] `crate::audit::ledger::AuditLogger::record(action, resource, actor, target, decision, reason)`
      — append-only, never updates.
- [ ] Admin HTTP route `GET /v1/audit?actor=...&action=...&since=...` returns paginated rows
      scoped to caller's org (admin-only per §4.3).
- [ ] User role gets `limited` view: only their own actions.
- [ ] Viewer role: 403.
- [ ] NFR-5.3: every mutating operation that emits to audit_log MUST also emit a summarized
      `Misc{kind:"audit_logged"}` event for the timeline view (OQ #8 Option C dual-write).

## Verification

```bash
cargo test -p seasoned-hand-core audit::ledger::tests
cargo test -p seasoned-hand-server audit_api
```

## Refs

- requirements: F-5.9, NFR-5.3
- architecture: §4.3 (matrix row), OQ §8 (Option C)
