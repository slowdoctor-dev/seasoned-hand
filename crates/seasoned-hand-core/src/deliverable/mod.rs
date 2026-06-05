//! Deliverable entity + persistence (V007 `deliverables` table).
//!
//! Canonical home for the [`Deliverable`] shape (V007 column projection).
//! `channel::delivery` re-exports this type so the `DeliverySink` trait
//! signature is self-contained without dragging the V007 schema into
//! the channel module. Closes Phase 2 DEBT #10.
//!
//! refs: /specs/phase-2/architecture.md §2.3, §2.11, §3 V007
//! refs: /specs/phase-2/stories/story-2.3.md

pub mod renderer;
pub mod store;
pub mod task_deliver;

pub use renderer::{RenderError, RenderedArtifact, RendererDispatcher};
pub use store::{DeliverableError, DeliverableStore, NewDeliverable};
pub use task_deliver::{
    PlannerSimplifyLlm, SimplifyLlm, TOOL_NAME as TASK_DELIVER_TOOL_NAME, TaskDeliver,
    TaskDeliverDeps,
};

// Canonical home for the `Deliverable` wire shape (V007 column projection) is
// `seasoned-hand-dto` (ADR-016 / story 6.3), shared by the backend and the
// wasm UI. The provenance manifest stays opaque JSON; story 2.15 owns its
// builder. `channel::delivery` re-exports this re-export, unchanged.
pub use seasoned_hand_dto::Deliverable;

#[cfg(test)]
mod tests;
