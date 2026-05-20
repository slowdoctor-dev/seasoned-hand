# Story 5.13 — user_cost reconciliation job + drift alarm

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 5.12
> **Phase**: 5
> **Type**: backend

---

## Goal

Daily reconciliation job per arch §9: recompute monthly per-user totals from canonical sources
and emit `Misc{kind:"user_cost_reconciliation_drift"}` when drift > 0.5% (NFR-5.4 threshold).

## Acceptance criteria

- [ ] `crate::billing::user_cost::ReconciliationJob::run(month)` recomputes from
      sessions+events, compares to current ledger, emits drift events.
- [ ] CLI: `seasoned-hand user-cost reconcile --month YYYY-MM` for manual trigger.
- [ ] Drift events carry `expected_cost_cents`, `observed_cost_cents`, `delta_pct`,
      `tenant_id`, `user_id` fields.

## Verification

```bash
cargo test -p seasoned-hand-core billing::user_cost::reconciliation::tests
```

## Refs

- requirements: F-5.10, NFR-5.4
- architecture: §9
