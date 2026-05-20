# Story 5.5 — HTTP middleware RBAC enforcement

> **Status**: done
> **Estimated**: 2 hours
> **Dependencies**: 5.3, 5.4
> **Phase**: 5
> **Type**: backend

---

## Goal

Axum middleware that wraps every route, extracts `AuthContext` from request headers/session,
and gates HTTP access per the §4.3 policy matrix BEFORE the route handler runs. Each handler
also re-calls `authorize(...)` for defense-in-depth (the §4.2 hybrid pattern).

## Acceptance criteria

- [ ] `crate::auth::middleware::require_auth_context()` rejects requests with missing context
      (`401 unauthorized_context`).
- [ ] Routes opt into `Action` via per-route metadata so middleware can call the policy engine
      automatically.
- [ ] Loopback admin routes (existing `require_loopback`) still work — middleware composes,
      doesn't replace.
- [ ] **Query scoping retrofit (F-5.13)**: every existing list/search handler (`GET /v1/tasks`,
      `GET /v1/projects`, `GET /v1/sessions`, `GET /v1/events/{session_id}`,
      `GET /v1/deliverables`, etc.) adds `WHERE tenant_id = :ctx.tenant_id` to its underlying
      query. No unscoped global list endpoint in multi-user mode. Verified by the
      cross-tenant isolation harness (story 5.26).
- [ ] Integration test: HTTP POST `/v1/tasks/handoff` from viewer role → 403.
- [ ] Integration test: forged tenant_id in request context → list endpoint returns 0 rows
      from other tenants.
- [ ] **V013 deferred NOT NULL flip** for projects/tasks/deliverables: apply the create-copy-rename pattern (architecture §3.4 schedule) in the same slice as this story's production change; update test fixtures to set explicit `tenant_id` where they previously relied on the column being nullable.

## Verification

```bash
cargo test -p seasoned-hand-server middleware::auth
cargo test -p seasoned-hand-server query_scoping_retrofit
```

## Refs

- requirements: F-5.5, F-5.13, NFR-5.2
- architecture: §4.2
