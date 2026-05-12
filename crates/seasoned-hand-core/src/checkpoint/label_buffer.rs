//! In-memory one-shot label buffer.
//!
//! `checkpoint_label(label)` calls `set(session_id, label)` — the next
//! Plan{op:"advance"} for that session consumes the label via `take` and
//! the slot empties. Labels are user decoration; we do NOT persist across
//! worker restart (architecture §4.3 / acceptance criterion).
//!
//! refs: /specs/phase-1/stories/story-1.13.md
//! refs: /specs/phase-1/architecture.md §4.3

use dashmap::DashMap;

#[derive(Default)]
pub struct CheckpointLabelBuffer {
    inner: DashMap<String, String>,
}

impl CheckpointLabelBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Overwrites any previously pending label for `session_id`.
    pub fn set(&self, session_id: &str, label: &str) {
        self.inner.insert(session_id.to_string(), label.to_string());
    }

    /// Reads and clears the pending label (if any) in one shot.
    pub fn take(&self, session_id: &str) -> Option<String> {
        self.inner.remove(session_id).map(|(_, v)| v)
    }

    /// Peek without consuming — test-only.
    #[cfg(test)]
    pub fn peek(&self, session_id: &str) -> Option<String> {
        self.inner.get(session_id).map(|v| v.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_then_take_returns_label_and_clears() {
        let buf = CheckpointLabelBuffer::new();
        buf.set("s1", "milestone-1");
        assert_eq!(buf.take("s1").as_deref(), Some("milestone-1"));
        assert!(buf.take("s1").is_none());
    }

    #[test]
    fn second_set_overwrites_first() {
        let buf = CheckpointLabelBuffer::new();
        buf.set("s1", "first");
        buf.set("s1", "second");
        assert_eq!(buf.take("s1").as_deref(), Some("second"));
    }

    #[test]
    fn labels_per_session_do_not_collide() {
        let buf = CheckpointLabelBuffer::new();
        buf.set("s1", "alpha");
        buf.set("s2", "beta");
        assert_eq!(buf.take("s1").as_deref(), Some("alpha"));
        assert_eq!(buf.take("s2").as_deref(), Some("beta"));
    }
}
