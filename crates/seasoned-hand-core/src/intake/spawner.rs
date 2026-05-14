//! `InitializerSpawner` — the surface that lets the [`IntakeRouter`]
//! hand off a freshly-created `drafted` Task to the Phase 2 confirm-gate
//! [`Initializer::run_with_confirmation`] without coupling the router
//! to AppState's session / sandbox / runner concerns.
//!
//! Story 2.8b closes Phase 2 DEBT #13 (router previously stopped after
//! creating the task) by:
//!
//! 1. Defining the trait + payload types here (in `core` so the router
//!    can call them).
//! 2. Letting the server crate implement the trait against its
//!    `AppState` (session row insert, mpsc sender registration, tokio
//!    spawn of the actual confirm-gate run).
//!
//! The split keeps `IntakeRouter` testable with an in-memory mock
//! spawner and keeps the heavy server-side wiring (DB, sandbox, runner)
//! out of `seasoned-hand-core`.
//!
//! refs: /specs/phase-2/architecture.md §2.7, §2.8
//! refs: /specs/phase-2/stories/story-2.8.md
//! refs: /specs/phase-2/DEBT.md #13

use async_trait::async_trait;
use thiserror::Error;

use crate::channel::DeliveryTarget;

/// Envelope handed to [`InitializerSpawner::spawn`] for one drafted
/// Task. Carries everything the spawner needs to materialise a session,
/// register the briefing sender, and kick off the confirm-gate run.
#[derive(Debug, Clone)]
pub struct SpawnSpec {
    pub task_id: String,
    /// Raw natural-language brief — fed to the planner slot inside
    /// [`crate::agent::init::Initializer::run_with_confirmation`].
    pub brief_input: String,
    /// Where the originating channel expects to receive the deliverable
    /// once the task completes. Pre-stored on the intake row;
    /// re-attached to the task by the spawner so the delivery router
    /// can find it later (chat-channel sets `session:<id>`).
    pub reply_target: Option<DeliveryTarget>,
    /// Optional caller-supplied session id. The WS chat path supplies
    /// this so it can ack the client with a stable `session_id` AND
    /// build the `session:<id>` reply target up-front; webhook / email
    /// intake supplies `None` and lets the spawner mint a fresh UUID.
    pub session_id_hint: Option<String>,
    /// `task_create` cmd carries these knobs; webhook / email use
    /// defaults. `None` means "spawner picks the default".
    pub max_steps: Option<u32>,
    pub cost_cap_cents: Option<u32>,
}

/// Synchronous receipt the spawner returns to the [`IntakeRouter`].
/// The post-confirm work (Initializer run, agent loop) is fire-and-forget
/// and never propagates back to the router.
#[derive(Debug, Clone)]
pub struct SpawnReceipt {
    pub session_id: String,
}

#[derive(Debug, Error)]
pub enum SpawnError {
    #[error("spawner error: {0}")]
    Other(String),
}

/// The router-side handle. One AppState owns one spawner; the spawner
/// itself clones its dependencies internally so each invocation is
/// independent.
#[async_trait]
pub trait InitializerSpawner: Send + Sync {
    async fn spawn(&self, spec: SpawnSpec) -> Result<SpawnReceipt, SpawnError>;
}
