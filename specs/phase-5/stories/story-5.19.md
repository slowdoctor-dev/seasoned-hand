# Story 5.19 — User invitation CLI + provisioning

> **Status**: ready
> **Estimated**: 2.5 hours
> **Dependencies**: 5.4, 5.10
> **Phase**: 5
> **Type**: cli+backend

---

## Goal

Implement the CLI-only invitation flow per architecture §12. Email/magic-link infrastructure
remains out-of-scope (Phase 6+, per §12 explicit pin).

## Acceptance criteria

- [ ] `seasoned-hand user invite <email> --org <slug> --role <admin|user|viewer>`:
  - inserts `users` row in `status='active'` and matching `organization_memberships` row
    in one transaction;
  - emits audit_log row `action='user.invite'`;
  - prints display name + login token for admin to share out-of-band.
- [ ] `seasoned-hand user list --org <slug>` shows current org members.
- [ ] Caller must have admin role (RBAC enforced via 5.6).

## Verification

```bash
cargo test -p seasoned-hand-cli user_invite
```

## Refs

- requirements: F-5.21
- architecture: §12
