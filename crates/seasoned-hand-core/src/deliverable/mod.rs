//! Deliverable entity + persistence (V007 `deliverables` table).
//!
//! Canonical home for the [`Deliverable`] shape (V007 column projection).
//! `channel::delivery` re-exports this type so the `DeliverySink` trait
//! signature is self-contained without dragging the V007 schema into
//! the channel module. Closes Phase 2 DEBT #10.
//!
//! refs: /specs/phase-2/architecture.md §2.3, §2.11, §3 V007
//! refs: /specs/phase-2/stories/story-2.3.md

pub mod store;

pub use store::{DeliverableError, DeliverableStore, NewDeliverable};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deliverable {
    pub id: String,
    pub task_id: String,
    pub tenant_id: Option<String>,
    /// One of: `docx | pdf | html | pptx | xlsx | csv | md | json | code | url`.
    pub format: String,
    /// Workspace path of the LLM-authored source (markdown / JSON);
    /// `None` for raw artifacts where the rendered file is also the
    /// source of truth (`url`, `code`).
    pub source_content_path: Option<String>,
    pub source_content_sha256: Option<String>,
    /// Workspace path of the rendered artifact (sandbox-relative).
    pub rendered_content_path: String,
    pub rendered_content_sha256: String,
    pub content_size: i64,
    /// JSON array of `event_id`s cited inline in the deliverable content.
    /// `None` until the renderer populates it.
    pub citations: Option<Vec<i64>>,
    /// Provenance manifest as defined in architecture §2.11. Stored as
    /// opaque JSON here so the channel module doesn't redefine the
    /// schema; story 2.15 owns the manifest builder.
    pub provenance_manifest: Value,
    pub created_at: i64,
}

#[cfg(test)]
mod tests;
