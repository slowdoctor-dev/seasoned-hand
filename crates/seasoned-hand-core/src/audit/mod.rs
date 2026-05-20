//! Audit log — immutable record of mutating operations (story 5.10).
//!
//! V013 created the `audit_log` table. This module wraps inserts behind
//! an `AuditLogger` service so every mutating story (hand-off, sharing,
//! invitation, deactivation) writes through one canonical surface, and
//! callers don't reinvent the column list at each call site.
//!
//! Per architecture OQ #8 Option C (dual-write), every `record(...)` call
//! ALSO emits a summarized `Misc{kind:"audit_logged"}` event so the
//! existing timeline view picks up the audit signal without joining
//! cross-table. Phase 4's curator and Phase 0–3 worker streams already
//! query `events`; the audit_log table is the structured-reporting
//! complement.
//!
//! refs: /specs/phase-5/architecture.md §4.3 (admin/user/viewer matrix
//!       for AuditRead), OQ §8 (dual-write decision)
//! refs: /specs/phase-5/stories/story-5.10.md

pub mod ledger;
pub use ledger::{
    AuditAction, AuditLogger, AuditQuery, AuditQueryError, AuditRecord, AuditRow, AuditWriteError,
};

#[cfg(test)]
mod tests;
