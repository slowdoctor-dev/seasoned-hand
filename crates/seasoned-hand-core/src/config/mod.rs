//! Shared configuration helpers.
//!
//! Phase 4 story 4.14 introduced strict-parse env helpers for curator
//! config. Phase 5 story 5.22 lifts them into core so server + CLI +
//! workers all use one canonical implementation; this closes Phase 5
//! DEBT #91 (global strict-config harmonization).
//!
//! refs: /specs/phase-5/stories/story-5.22.md
//! closes: Phase 5 DEBT #91

pub mod strict;

pub use strict::{
    env_bool_or_default, env_f32_or_default, env_u32_or_default, env_u64_or_default,
    parse_bool_strict, parse_f32_strict, parse_u32_strict, parse_u64_strict,
};
