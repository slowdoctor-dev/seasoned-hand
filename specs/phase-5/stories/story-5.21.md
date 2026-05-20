# Story 5.21 — Optimistic concurrency for shared artifacts

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 5.7, 5.8
> **Phase**: 5
> **Type**: backend

---

## Goal

Per architecture §11 (OQ #14 Option B): SOP/playbook updates carry `expected_updated_at`
precondition. Mismatch → 409 with current revision metadata. No hard locks in Phase 5.

## Acceptance criteria

- [ ] Service methods accept `expected_updated_at` parameter (None = first-write).
- [ ] HTTP layer translates mismatch to `409 conflict` with body
      `{"error":"stale_revision","current_updated_at":...,"current_revision_id":...}`.
- [ ] CLI surfaces operator-readable error and suggests `--force` or refresh.
- [ ] Tests cover concurrent-update scenario via two service calls.

## Verification

```bash
cargo test -p seasoned-hand-core sharing::concurrency
```

## Refs

- requirements: F-5.22
- architecture: §11, OQ §14
