# Story 5.4 — org/user/membership persistence + project_role_overrides

> **Status**: done
> **Estimated**: 2.5 hours
> **Dependencies**: 5.2
> **Phase**: 5
> **Type**: backend

---

## Goal

Rusqlite-backed stores for `organizations`, `users`, `organization_memberships`,
`project_role_overrides` (created by V013 in story 5.2). Standard CRUD + tenant-scoped
queries used by AuthContext resolver (5.3) and RBAC enforcement (5.5/5.6).

## Acceptance criteria

- [ ] `crate::org::{OrganizationStore, UserStore, MembershipStore, ProjectRoleOverrideStore}` with
      `insert / get / list / update_role / soft_deactivate` methods.
- [ ] One primary membership invariant enforced via partial unique index
      `idx_membership_primary_per_user` (V013 already creates it; tested here).
- [ ] All SQL parameterized; tenant_id required on every read/write.
- [ ] Unit tests: round-trip CRUD per store, primary-membership UNIQUE enforcement,
      project override precedence.

## Verification

```bash
cargo test -p seasoned-hand-core org::tests
```

## Refs

- requirements: F-5.1, F-5.4
- architecture: §3.2, §4.1
