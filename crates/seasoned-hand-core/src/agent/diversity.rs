//! Diversity injector for the stuck-tracker strategy-change prompt.
//!
//! Four hand-picked variant phrasings rotate per session via a DashMap
//! cursor. Architecture §5 principle #6 ("diversity injection") requires
//! varied phrasing to prevent the LLM from few-shot-locking onto a
//! repeated stuck-prompt; four was the simplest count that gives the
//! injector a full cycle before any single variant repeats inside a
//! typical 50-step task. Phase 1 DEBT #7 schedules the eventual promotion
//! of these to a Curator-managed DB table (Phase 4+); the Rust const
//! array is the deliberately-simple Phase 1 baseline.
//!
//! refs: /specs/01-architecture/ARCHITECTURE.md §5 principle #6,
//! /specs/phase-1/DEBT.md #7, /specs/phase-1/stories/story-1.12.md

use dashmap::DashMap;

pub const VARIANTS: [&str; 4] = [
    "Your last {n} attempts repeated. Try a different tool, re-read recent observations, or call message_ask_user to clarify.",
    "We have looped on the same response {n} times. Step back: which assumption could be wrong? Inspect a different file or query the user.",
    "{n} duplicates. Don't repeat: change what you observe (different path/query) before changing how you act.",
    "{n}x same response. Recall PRINCIPLE #5: failed observations are signal, not noise. Pick one and act on it differently.",
];

#[derive(Default)]
pub struct DiversityInjector {
    cursor: DashMap<String, usize>,
}

impl DiversityInjector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn next_prompt(
        &self,
        session_id: &str,
        count: u32,
        recent_event_id: u64,
        recent_summary: &str,
    ) -> String {
        let index = {
            let mut slot = self.cursor.entry(session_id.to_string()).or_insert(0);
            let index = *slot;
            *slot = (*slot + 1) % VARIANTS.len();
            index
        };
        let head = VARIANTS[index].replace("{n}", &count.to_string());
        let clipped = if recent_summary.chars().count() > 120 {
            let mut s = recent_summary.chars().take(119).collect::<String>();
            s.push('…');
            s
        } else {
            recent_summary.to_string()
        };
        format!("{head} Your last observation (event #{recent_event_id}): {clipped}.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diversity_injector_4_variants_rotate() {
        let d = DiversityInjector::new();
        let mut out = Vec::new();
        for _ in 0..4 {
            out.push(d.next_prompt("s1", 2, 9, "obs"));
        }
        assert_eq!(out.len(), 4);
        assert!(out[0].contains("Your last 2 attempts repeated"));
        assert!(out[1].contains("looped on the same response 2 times"));
        assert!(out[2].contains("2 duplicates"));
        assert!(out[3].contains("2x same response"));
    }

    #[test]
    fn diversity_injector_references_recent_observation() {
        let d = DiversityInjector::new();
        let p = d.next_prompt("s1", 3, 42, "hello");
        assert!(p.contains("event #42"));
        assert!(p.contains("hello"));
    }
}
