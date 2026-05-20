# Story 5.18 — Optional org-wide curator aggregation flag (default off)

> **Status**: ready
> **Estimated**: 1.5 hours
> **Dependencies**: 5.17
> **Phase**: 5
> **Type**: backend+config

---

## Goal

Per architecture §8 OQ #12 Option B: org-wide aggregation flag exists but defaults false.
Admin-only flip via `SH_CURATOR_ORG_AGGREGATION` strict-parsed env var.

## Acceptance criteria

- [ ] `CuratorConfig::org_aggregation_enabled: bool` (default false).
- [ ] When enabled, curator's CandidateBuilder may pull from all tenants within the same
      `organization_id`. Still respects per-row `tenant_id` for audit attribution.
- [ ] When disabled (default), behavior is identical to Phase 4 (project-scoped).
- [ ] Strict-parsed env reading (story 4.14 helper pattern).

## Verification

```bash
cargo test -p seasoned-hand-core curator::org_aggregation
```

## Refs

- requirements: F-5.14
- architecture: §8, OQ §12
