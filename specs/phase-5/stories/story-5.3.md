# Story 5.3 — AuthContext resolver + Policy engine core

> **Status**: ready
> **Estimated**: 3 hours
> **Dependencies**: 5.2
> **Phase**: 5
> **Type**: backend

---

## Goal

Land the `crate::auth::{context, policy}` modules per architecture §2 + §4. AuthContext
resolves `tenant_id`, `organization_id`, `actor_user_id`, `org_role`, optional
`project_override_role`. Policy engine exposes `authorize(action, resource, context) -> Result<(), AuthError>`
callable from HTTP, CLI, and worker surfaces.

## Acceptance criteria

- [ ] `crate::auth::context::AuthContext` struct + `Role` enum + `Action` enum per arch §4.4.
- [ ] `crate::auth::policy::authorize(action, resource, ctx)` evaluates the §4.3 matrix.
- [ ] Effective role resolver applies project override per arch §4.1.
- [ ] Fail-closed on missing tenant context (deny + structured error).
- [ ] Unit tests cover every cell in the §4.3 matrix (admin/user/viewer × 8 actions).
- [ ] No HTTP/CLI/worker wiring yet — those come in 5.5/5.6.

## Verification

```bash
cargo test -p seasoned-hand-core auth::policy::tests
cargo clippy --all-targets -- -D warnings
```

## Refs

- requirements: F-5.4, F-5.5, NFR-5.2
- architecture: §4
