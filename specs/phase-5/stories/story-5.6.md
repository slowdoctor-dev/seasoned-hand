# Story 5.6 — CLI + worker RBAC enforcement (hybrid defense)

> **Status**: ready
> **Estimated**: 2.5 hours
> **Dependencies**: 5.3, 5.4
> **Phase**: 5
> **Type**: backend

---

## Goal

Apply the same `authorize(action, resource, ctx)` policy engine inside CLI command paths and
inside spawned workers (verifier, curator, retention, ttl, notify, intake). Closes the §4.2
hybrid defense (HTTP middleware + core service re-check + CLI/worker direct-call).

## Acceptance criteria

- [ ] Every CLI command that mutates state resolves an `AuthContext` (from `--as-user` flag
      or local-operator default) and calls `authorize(...)` before the underlying core method.
- [ ] Worker spawn sites obtain a worker-context `AuthContext` (system-actor identity, scoped
      to the project/tenant they own) and re-check on every cross-tenant boundary.
- [ ] No worker path can write to a tenant other than the one its config pins.

## Verification

```bash
cargo test -p seasoned-hand-cli auth::tests
cargo test -p seasoned-hand-core curator auth::tests
```

## Refs

- requirements: F-5.5
- architecture: §4.2
