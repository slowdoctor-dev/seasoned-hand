# Story 5.26 — phase5_cross_tenant_isolation_harness (NFR-5.1)

> **Status**: ready
> **Estimated**: 3 hours
> **Dependencies**: 5.5, 5.6, 5.17
> **Phase**: 5
> **Type**: test

---

## Goal

The headline acceptance harness for NFR-5.1: cross-tenant read/write leakage rate must be 0
across every surface — API, CLI, verifier worker, curator worker, retention scheduler, ttl
cron, notify worker, intake router.

## Acceptance criteria

- [ ] `phase5_cross_tenant_isolation_harness` test exists.
- [ ] For each surface, attempts a forged-tenant request and asserts:
  - rejection with structured error,
  - no row writes outside the actor's tenant,
  - no row reads outside the actor's tenant.
- [ ] Missing-context test: requests without tenant resolution → fail-closed deny.
- [ ] CI budget < 5 min.

## Verification

```bash
cargo test -p seasoned-hand-core phase5_cross_tenant_isolation_harness
```

## Refs

- requirements: NFR-5.1
- architecture: §15 harness 1
