//! Provenance manifest module — builds, persists, and serves the
//! per-Deliverable JSON manifest defined in architecture §2.11.
//!
//! - [`manifest`] — the typed `ProvenanceManifest` schema.
//! - [`builder`] — composes a manifest from task / project / intake /
//!   sessions / decisions / verdicts / checkpoints / deliveries +
//!   aggregate metrics.
//! - [`spill`] — 100 KB inline budget; spills to
//!   `/workspace/.provenance/<task_id>.json` above.
//! - [`routes`] — pure `RouteOutcome` layer for
//!   `GET /v1/tasks/:id/provenance`.
//!
//! refs: /specs/phase-2/architecture.md §2.11, §12 q5
//! refs: /specs/phase-2/stories/story-2.15.md

pub mod builder;
pub mod manifest;
pub mod routes;
pub mod spill;

pub use builder::{BuildDeps, ManifestInputs, ProvenanceError, build_manifest};
pub use manifest::{
    BriefProvenance, CheckpointProvenance, DeliveredTo, IntakeProvenance, ProvenanceManifest,
    ProvenanceMetrics, SCHEMA_VERSION, SessionProvenance,
};
pub use routes::{
    GetTaskProvenanceDeps, GetTaskProvenanceQuery, ProvenanceResponse, get_task_provenance,
    resolve_manifest,
};
pub use spill::{INLINE_THRESHOLD_BYTES, ManifestColumn, persist_or_spill};

#[cfg(test)]
mod tests;
