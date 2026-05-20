//! Named FTS5 column weights for the three search-bearing tables
//! (Phase 5 story 5.24 / Phase 5 DEBT #76 partial close).
//!
//! Today's weights are uniform 1.0 across every column — this matches
//! the FTS5 default rank and preserves the Phase 4 warm-loop benchmark
//! behavior. The point of this module is the **named surface**: future
//! retunes touch one place rather than scattered `bm25(t, w1, w2, ...)`
//! literals across the matcher / session_search / curator queries.
//!
//! ## Measurement procedure (for the eventual full retune)
//!
//! See `/specs/phase-5/dogfood_fts_retune.md` for the dogfood capture
//! protocol. Summary:
//!
//! 1. Capture a representative production corpus (Phase 4 warm-loop
//!    benchmark synthetic data is insufficient — needs real operator
//!    queries against a populated repository).
//! 2. Define the eval relevance set per query (the top-k playbooks /
//!    events / curator decisions the operator marks as relevant).
//! 3. Compute precision@3 + MRR with the current uniform weights.
//! 4. Grid-search candidate weight tuples on (title, keywords, content)
//!    for playbooks; (event_type, source, searchable_text) for
//!    session_search; (decision_kind, rationale, raw_payload) for
//!    curator.
//! 5. Accept the weight set that improves precision@3 + MRR without
//!    regressing the warm-loop benchmark.
//! 6. Land the new constants here; rerun the benchmark; commit.
//!
//! Full retune is **deferred to Phase 6** when production dogfood data
//! exists (DEBT #76 partial close — see `specs/phase-5/DEBT.md`).
//!
//! refs: /specs/phase-5/stories/story-5.24.md
//! refs: /specs/phase-5/requirements.md F-5.19
//! debt: #76 partial close (Phase 5 → Phase 6 successor)

/// Column weights for `playbooks_fts` (title, trigger_keywords, content).
/// Intent: title carries the most explicit authoring signal, then
/// trigger keywords, then prose content. Uniform today; ratio reserved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaybooksFtsWeights {
    pub title: f64,
    pub trigger_keywords: f64,
    pub content: f64,
}

impl PlaybooksFtsWeights {
    /// Current uniform baseline. Future retune will likely set
    /// `(title, trigger_keywords, content) = (5.0, 2.0, 1.0)` once
    /// dogfood data validates the prior. Stay uniform until then so
    /// the warm-loop benchmark precision@3 stays stable.
    pub const UNIFORM: Self = Self {
        title: 1.0,
        trigger_keywords: 1.0,
        content: 1.0,
    };
}

/// Column weights for `session_search_fts`
/// (event_type, source, searchable_text — see V018 schema).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SessionSearchFtsWeights {
    pub event_type: f64,
    pub source: f64,
    pub searchable_text: f64,
}

impl SessionSearchFtsWeights {
    pub const UNIFORM: Self = Self {
        event_type: 1.0,
        source: 1.0,
        searchable_text: 1.0,
    };
}

/// Column weights for `curator_search_fts` (see V019 schema).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CuratorSearchFtsWeights {
    pub decision_kind: f64,
    pub rationale: f64,
    pub raw_payload: f64,
}

impl CuratorSearchFtsWeights {
    pub const UNIFORM: Self = Self {
        decision_kind: 1.0,
        rationale: 1.0,
        raw_payload: 1.0,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_weights_are_all_one() {
        // Pin the current baseline so a future tuning PR has to
        // explicitly update this test, surfacing the weight change
        // in code review.
        assert_eq!(PlaybooksFtsWeights::UNIFORM.title, 1.0);
        assert_eq!(PlaybooksFtsWeights::UNIFORM.trigger_keywords, 1.0);
        assert_eq!(PlaybooksFtsWeights::UNIFORM.content, 1.0);
        assert_eq!(SessionSearchFtsWeights::UNIFORM.event_type, 1.0);
        assert_eq!(CuratorSearchFtsWeights::UNIFORM.decision_kind, 1.0);
    }
}
