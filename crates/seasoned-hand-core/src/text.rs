//! Small string helpers shared across crates.
//!
//! Like [`crate::time`], this exists to collapse copies that had drifted
//! into several modules (`deliverable`, `channel`, `sandbox`) — keeping
//! one definition so the behaviour stays consistent.

/// Truncate `s` to at most `n` Unicode scalar values, preserving char
/// boundaries (never splits a multi-byte char). Returns `s` unchanged
/// when it is already `<= n` chars.
pub fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect()
    }
}
