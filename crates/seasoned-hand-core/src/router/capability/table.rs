//! Static capability table: maps an upstream provider model ID
//! (as returned by Bifrost's `/v1/models/<alias>`) to a tri-state
//! [`CapabilityFlags`]. Unknown IDs return all-`None` flags — callers
//! decide how to handle the unknown case.
//!
//! refs: /specs/phase-1/stories/story-1.7.md
//! refs: /specs/phase-1/architecture.md §6 row "Capability table fallback"

use super::CapabilityFlags;

/// Look up capability flags for a resolved provider model ID.
///
/// The list is intentionally narrow in Phase 1 — Phase 4+ may grow this
/// into a discovery mechanism. Unrecognised IDs return [`CapabilityFlags::unknown`].
pub fn capabilities_for(model_id: &str) -> CapabilityFlags {
    match model_id {
        // Claude 4.x family — tool calling + vision via Bifrost.
        "claude-sonnet-4-6" | "claude-opus-4-7" | "claude-haiku-4-5" => CapabilityFlags {
            tool_calling: Some(true),
            json_mode: Some(true),
            vision: Some(true),
        },
        // GPT-5 family.
        "gpt-5.1" => CapabilityFlags {
            tool_calling: Some(true),
            json_mode: Some(true),
            vision: Some(true),
        },
        "gpt-5.3-codex" => CapabilityFlags {
            tool_calling: Some(true),
            json_mode: Some(false),
            vision: Some(false),
        },
        // Local models via Ollama.
        "llama3.2:3b" => CapabilityFlags {
            tool_calling: Some(true),
            json_mode: Some(true),
            vision: Some(false),
        },
        _ => CapabilityFlags::unknown(),
    }
}
