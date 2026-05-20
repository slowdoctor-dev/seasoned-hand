# Story 5.7 — sop_shares ACL + CLI surfaces

> **Status**: ready
> **Estimated**: 2.5 hours
> **Dependencies**: 5.4, 5.5
> **Phase**: 5
> **Type**: backend+cli

---

## Goal

Implement the SOP sharing ACL per architecture §6.1 (Option B per-SOP). `sop_shares` table
already created by V013. Add service layer + CLI commands.

## Acceptance criteria

- [ ] `crate::sharing::sop::{share, unshare, list_for_sop, list_for_user}` service methods,
      all RBAC-gated via `Action::SopShare`.
- [ ] CLI: `seasoned-hand sop share <sop_id> --user <email> --permission <viewer|editor|owner>`
      + `seasoned-hand sop unshare ...`.
- [ ] Default-create policy: SOP owner gets `owner` row automatically.
- [ ] Tests: viewer cannot escalate own permission; admin can override any grant.
- [ ] Shared-permission visibility propagates within 5 seconds p95 for authorized users
      (NFR-5.5 consistency budget).
- [ ] **V013 deferred NOT NULL flip** for skills (if first writer lands): apply the create-copy-rename pattern (architecture §3.4 schedule) in the same slice as this story's production change; update test fixtures to set explicit `tenant_id` where they previously relied on the column being nullable.

## Verification

```bash
cargo test -p seasoned-hand-core sharing::sop::tests
```

## Refs

- requirements: F-5.6, F-5.23, NFR-5.5
- architecture: §6.1
