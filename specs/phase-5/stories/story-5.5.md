# Story 5.5 — HTTP middleware RBAC enforcement

> **Status**: ready
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
- [ ] Integration test: HTTP POST `/v1/tasks/handoff` from viewer role → 403.

## Verification

```bash
cargo test -p seasoned-hand-server middleware::auth
```

## Refs

- requirements: F-5.5, NFR-5.2
- architecture: §4.2
