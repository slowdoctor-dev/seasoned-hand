//! Project / Task module — Phase 2 OS-shape persistence substrate.
//!
//! Exposes [`ProjectStore`] and [`TaskStore`], thin wrappers around the
//! shared [`crate::db::DbPool`] that own the V006 `projects` / `tasks`
//! tables. Story 2.2 ships the stores in isolation; story 2.3 wires
//! them into `AppState` once V007 / V008 / V009 stores also exist.
//!
//! refs: /specs/phase-2/architecture.md §2.1, §3 V006
//! refs: /specs/phase-2/stories/story-2.2.md

// File layout pinned by story-2.2 spec (`project.rs` + `task.rs`); the
// inner module is never named via `project::project::` thanks to the
// re-exports below, so the inception is purely on-disk.
pub mod brief;
#[allow(clippy::module_inception)]
pub mod project;
pub mod task;

pub use brief::{
    Brief, BriefError, BriefPhase, DeliverableFormat, DeliverableSpec, MAX_DELIVERABLES,
    MAX_GOAL_LEN, MAX_PHASE_TITLE_LEN, MAX_PHASES, MAX_SUCCESS_CRITERIA, MAX_SUCCESS_CRITERION_LEN,
};
pub use project::{NewProject, Project, ProjectError, ProjectPatch, ProjectStatus, ProjectStore};
pub use task::{NewTask, Task, TaskError, TaskStatus, TaskStore, legal_transitions};

#[cfg(test)]
mod tests;
