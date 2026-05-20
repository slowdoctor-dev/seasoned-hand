# Story 5.30 — phase5_user_cost_reconciliation_harness (NFR-5.4)

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 5.13
> **Phase**: 5
> **Type**: test

---

## Goal

Verify NFR-5.4: monthly per-user cost totals reconcile to source rows within +/-0.5%.

## Acceptance criteria

- [ ] `phase5_user_cost_reconciliation_harness` seeds N sessions across M users with known
      `cost_cents` totals.
- [ ] Nearline writer flushes; ledger matches source.
- [ ] Inject a drift scenario (manually corrupt a ledger row) and assert reconciliation
      detects + emits `Misc{kind:"user_cost_reconciliation_drift"}` with correct fields.
- [ ] CI budget < 2 min.

## Verification

```bash
cargo test -p seasoned-hand-core phase5_user_cost_reconciliation_harness
```

## Refs

- requirements: F-5.10, NFR-5.4
- architecture: §15 harness 5
