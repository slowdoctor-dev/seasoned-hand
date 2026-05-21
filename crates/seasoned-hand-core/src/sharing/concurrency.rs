//! Optimistic concurrency control for shared artifacts (Phase 5 story
//! 5.21).
//!
//! Per architecture §11 (OQ #14 Option B): SOP/playbook mutations carry
//! an `expected_updated_at` precondition. When the caller passes
//! `Some(ts)` and the row's current `updated_at != ts`, the mutation
//! fails with `StaleRevision` carrying the live revision metadata so
//! the client can refresh and retry. No hard locks — first-writer wins,
//! second-writer must reconcile.
//!
//! `expected_updated_at = None` means "first-write" (or "I don't care
//! about concurrency"); existing rows are overwritten without a check.
//!
//! refs: /specs/phase-5/architecture.md §11, OQ §14
//! refs: /specs/phase-5/stories/story-5.21.md

use serde::{Deserialize, Serialize};

/// Returned alongside the per-service error variants when a concurrent
/// update wins the race. Carries the live row's `updated_at` so a
/// client can refresh and retry, plus the row id so audit links stay
/// resolvable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StaleRevision {
    pub current_updated_at: i64,
    pub current_revision_id: String,
}

/// Compare a caller-supplied `expected_updated_at` against the row's
/// current value. Returns `Ok(())` when the precondition holds (or
/// when the caller didn't supply one). Returns `Err(StaleRevision)`
/// with the live row metadata otherwise.
///
/// Designed to be called from inside a service method's transaction
/// AFTER reading the row, BEFORE mutating it — so the check + write
/// land atomically.
///
/// ## Atomicity invariant (hardening P5-HARD-IT1-L1)
///
/// All five callers (`SopShareService::{share,unshare}`,
/// `PlaybookShareService::{share,unshare,update_visibility_state}`) do
/// SELECT-`updated_at` → `check_precondition` → mutate inside a single
/// [`crate::db::DbPool::with_conn`] closure. That closure holds the
/// pool's `tokio::Mutex` over the **one** SQLite connection for its
/// whole duration (see `db/mod.rs` — Phase 0 DEBT #1), so the
/// read-check-write is atomic today WITHOUT an explicit SQL
/// transaction: no other writer can interleave.
///
/// **When DEBT #1 is paid down to a real multi-connection pool**, that
/// guarantee disappears and these sequences become a live TOCTOU race.
/// At that point each caller MUST wrap its read-check-write in an
/// explicit `conn.transaction()` (the handoff service already does this
/// in `handoff/task.rs`). This is a hard prerequisite of the
/// pool-paydown work, recorded here at the shared chokepoint so it
/// cannot be missed.
pub fn check_precondition(
    expected_updated_at: Option<i64>,
    current_updated_at: i64,
    current_revision_id: &str,
) -> Result<(), StaleRevision> {
    match expected_updated_at {
        Some(expected) if expected != current_updated_at => Err(StaleRevision {
            current_updated_at,
            current_revision_id: current_revision_id.to_string(),
        }),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_precondition_always_passes() {
        assert!(check_precondition(None, 12345, "row-1").is_ok());
        assert!(check_precondition(None, 0, "row-1").is_ok());
    }

    #[test]
    fn matching_precondition_passes() {
        assert!(check_precondition(Some(12345), 12345, "row-1").is_ok());
    }

    #[test]
    fn mismatched_precondition_returns_live_metadata() {
        let err = check_precondition(Some(11_111), 22_222, "row-7")
            .expect_err("must reject stale revision");
        assert_eq!(err.current_updated_at, 22_222);
        assert_eq!(err.current_revision_id, "row-7");
    }
}
