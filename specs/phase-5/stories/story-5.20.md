# Story 5.20 — User deactivation + mandatory reassignment

> **Status**: done
> **Estimated**: 2.5 hours
> **Dependencies**: 5.19, 5.9
> **Phase**: 5
> **Type**: cli+backend

---

## Goal

Per architecture §12: deactivation requires reassignment of active task ownership and
owner-level shares before the user can be deactivated. Historical audit attribution stays
unchanged.

## Acceptance criteria

- [ ] `seasoned-hand user deactivate <email> --reassign-to <other-email>`:
  - validates target user has admin/user role in same org;
  - moves all active task ownership from deactivated user to `--reassign-to` via 5.9
    hand-off semantics;
  - transfers owner-level SOP/playbook shares;
  - sets `users.status='deactivated'`;
  - emits `audit_log` row for the lifecycle event.
- [ ] Deactivation fails closed if active assets exist and no `--reassign-to` was given.

## Verification

```bash
cargo test -p seasoned-hand-cli user_deactivate
```

## Refs

- requirements: F-5.21
- architecture: §12
