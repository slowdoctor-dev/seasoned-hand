//! Durable pause + replay-rebuild scaffolding (Phase 2 story 2.16).
//!
//! `task_pause { durable: true }` records sandbox + workspace + event
//! cursor metadata so a future `task_resume` survives the container
//! getting garbage-collected during the pause window. When that
//! happens, [`resume::resume_task`] spins a fresh container under a
//! new Session row (same Task) and replays the prior session's event
//! stream into a coherent Plan + feature-list + progress state via
//! the helpers in [`replay`].
//!
//! refs: /specs/phase-2/architecture.md §2.6, §8
//! refs: /specs/phase-2/stories/story-2.16.md

pub mod replay;
pub mod resume;
pub mod ttl;

pub use replay::{
    ReplayError, ReplayStep, replay_feature_list, replay_plan, replay_progress, restore_plan_row,
};
pub use resume::{ResumeDeps, ResumeError, ResumeOutcome, SandboxOps, resume_task};
pub use ttl::{SandboxJanitor, TtlCleanupReport, TtlConfig, WorkspaceTtlCron};

#[cfg(test)]
mod tests;
