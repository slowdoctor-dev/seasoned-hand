# Story 5.12 — user_cost_ledger nearline writer

> **Status**: ready
> **Estimated**: 2.5 hours
> **Dependencies**: 5.4
> **Phase**: 5
> **Type**: backend

---

## Goal

`user_cost_ledger` table already created by V013. Add the nearline writer per arch §9:
update on session close + periodic 1h checkpoint, scoped to
`tenant_id + organization_id + actor_user_id + month_yyyymm`. Source: existing
`sessions.cost_cents` + Action-event tool counts.

## Acceptance criteria

- [ ] `crate::billing::user_cost::NearlineWriter::flush(window)` aggregates from sessions +
      events for closed sessions and upserts the matching monthly rollup row.
- [ ] Idempotent re-runs: UNIQUE(tenant_id, user_id, month_yyyymm) + watermark fields prevent
      double-counting.
- [ ] Spawned by `main.rs` as a sibling cron alongside curator-retention (1h default;
      `SH_USER_COST_INTERVAL_SEC` strict-parsed override).

## Verification

```bash
cargo test -p seasoned-hand-core billing::user_cost::tests
```

## Refs

- requirements: F-5.10, NFR-5.4
- architecture: §9
