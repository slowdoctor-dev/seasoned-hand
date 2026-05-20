//! Per-user cost aggregation for tenant billing (Phase 5 story 5.12).
//!
//! V013 created the `user_cost_ledger` table — a monthly rollup keyed by
//! `(tenant_id, user_id, month_yyyymm)`. This module owns the nearline
//! writer that derives those rollups from `sessions` + `events` so the
//! per-user-cost surface (NFR-5.4) reflects actual usage without
//! double-counting across re-runs.
//!
//! The writer is idempotent by construction: each flush recomputes the
//! monthly totals from scratch and UPSERTs over the matching row. The
//! `source_high_watermark_event_id` and `reconciled_at` columns are
//! advisory (story 5.13's reconciliation job will compare them against
//! independent counts).
//!
//! refs: /specs/phase-5/architecture.md §9 (cost rollup pipeline)
//! refs: /specs/phase-5/stories/story-5.12.md

pub mod user_cost;

#[cfg(test)]
mod tests;

pub use user_cost::{
    DEFAULT_USER_COST_RECONCILE_INTERVAL_SECS, DriftFinding, FlushReport, NearlineWriter,
    NearlineWriterError, ReconciliationError, ReconciliationJob, ReconciliationReport,
};
