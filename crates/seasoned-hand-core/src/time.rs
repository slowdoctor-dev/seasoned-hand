//! Canonical clock helpers.
//!
//! Every persisted timestamp in the project is `i64` microseconds since
//! the Unix epoch (SQLite `INTEGER`). Before this module, ~25 files
//! re-implemented `fn now_micros() -> i64` locally — most with the
//! saturating `i64::try_from(...).unwrap_or(i64::MAX)` shape, a handful
//! with the older `d.as_micros() as i64` wrapping cast. The drift was
//! benign (the i64-micros overflow horizon is year 294247) but pulling
//! one canonical helper makes future timestamp changes a single edit
//! and keeps overflow semantics consistent.

use std::time::{SystemTime, UNIX_EPOCH};

/// Microseconds since the Unix epoch, saturating to `i64::MAX` on the
/// pathological-future overflow.
///
/// Returns `0` only if the system clock reports a pre-1970 time — the
/// same fail-safe the prior local copies used so saved timestamps stay
/// monotonically non-negative.
pub fn now_micros() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_micros()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Stringified [`now_micros`] — convenience for log/event payloads that
/// carry the integer timestamp as a string.
pub fn now_micros_str() -> String {
    now_micros().to_string()
}
