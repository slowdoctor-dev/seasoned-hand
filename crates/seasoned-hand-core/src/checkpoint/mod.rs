//! Checkpoint Manager — commits sandbox workspace state to git on every
//! Plan{op:"advance"} event so phase progress has a stable HEAD anchor.
//!
//! refs: /specs/phase-1/stories/story-1.13.md
//! refs: /specs/phase-1/architecture.md §2.6 (commit half), §3.3
//!       (table), §4.3 (`checkpoint_label`), §8 (commit-fail mode)
//!
//! Phase 1 baseline ships `CheckpointManager::handle_plan_advance` as the
//! tested unit-of-work and a minimal poll-driven `run` loop. The
//! event-driven global subscribe pathway (Plan{op:"advance"} fanout) is
//! deferred to story 1.20 E2E where the full event broadcast bus is in
//! place — `handle_plan_advance` is the API the runner / tests / future
//! broadcaster call into directly.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use crate::events::{EventStore, EventType, NewEvent, sqlite::SqliteEventStore};

pub mod git_in_sandbox;
pub mod label_buffer;
pub mod persistence;
pub mod routes;

pub use git_in_sandbox::{CheckpointGitError, GitShell, SandboxGitShell};
pub use label_buffer::CheckpointLabelBuffer;
pub use persistence::{Checkpoint, CheckpointPersistenceError, CheckpointStore, NewCheckpoint};

/// Single `Plan{op:"advance"}` event payload the manager consumes.
#[derive(Debug, Clone)]
pub struct PlanAdvanceEvent {
    pub session_id: String,
    pub plan_phase_id: i64,
    pub phase_title: String,
    pub triggered_by_event_id: i64,
}

/// Dependencies for the Checkpoint Manager — assembled from `AppState`
/// at server bootstrap, but constructible directly in tests so they can
/// inject a mock `GitShell`.
#[derive(Clone)]
pub struct CheckpointManagerDeps {
    pub store: Arc<CheckpointStore>,
    pub labels: Arc<CheckpointLabelBuffer>,
    pub events: Arc<SqliteEventStore>,
    pub git: Arc<dyn GitShell>,
}

#[derive(Clone)]
pub struct CheckpointManager {
    deps: CheckpointManagerDeps,
}

impl CheckpointManager {
    pub fn new(deps: CheckpointManagerDeps) -> Self {
        Self { deps }
    }

    /// The unit of work: respond to a single Plan{op:"advance"} event by
    /// committing the workspace + persisting a row + emitting a Misc
    /// event. Returns `Ok(Some(checkpoint_id))` on success; `Ok(None)`
    /// when the commit shelled out but failed (caller already sees the
    /// failure Misc event); `Err` on infrastructure errors (DB / event
    /// store) that prevent emitting any audit event.
    pub async fn handle_plan_advance(
        &self,
        ev: PlanAdvanceEvent,
    ) -> Result<Option<String>, CheckpointPersistenceError> {
        let label = self.deps.labels.take(&ev.session_id);
        match self
            .deps
            .git
            .commit_phase(&ev.session_id, ev.plan_phase_id, &ev.phase_title)
            .await
        {
            Ok(git_sha) => {
                let checkpoint_id = self
                    .deps
                    .store
                    .insert(NewCheckpoint {
                        session_id: ev.session_id.clone(),
                        plan_phase_id: ev.plan_phase_id,
                        git_sha: git_sha.clone(),
                        label: label.clone(),
                        triggered_by_event_id: ev.triggered_by_event_id,
                    })
                    .await?;
                let data = json!({
                    "kind": "checkpoint_create",
                    "checkpoint_id": checkpoint_id,
                    "plan_phase_id": ev.plan_phase_id,
                    "git_sha": git_sha,
                    "label": label,
                });
                if let Err(error) = self.emit_misc(&ev.session_id, data).await {
                    tracing::warn!(
                        session_id = %ev.session_id,
                        %error,
                        "checkpoint manager: failed to emit checkpoint_create event",
                    );
                }
                Ok(Some(checkpoint_id))
            }
            Err(err) => {
                let data = json!({
                    "kind": "checkpoint_create",
                    "ok": false,
                    "reason": err.to_string(),
                    "plan_phase_id": ev.plan_phase_id,
                });
                if let Err(error) = self.emit_misc(&ev.session_id, data).await {
                    tracing::warn!(
                        session_id = %ev.session_id,
                        %error,
                        "checkpoint manager: failed to emit checkpoint_create error event",
                    );
                }
                Ok(None)
            }
        }
    }

    async fn emit_misc(
        &self,
        session_id: &str,
        data: serde_json::Value,
    ) -> Result<(), crate::events::EventError> {
        self.deps
            .events
            .append(NewEvent {
                session_id: session_id.to_string(),
                event_type: EventType::Misc,
                source: "checkpoint".to_string(),
                data,
            })
            .await
            .map(|_| ())
    }

    /// Long-running entrypoint. Phase 1 baseline: a poll-tick loop that
    /// honors the shutdown token. The XREADGROUP-style global Plan
    /// subscriber wiring is left as a story-1.20 E2E task; for unit
    /// tests, drive `handle_plan_advance` directly.
    pub async fn run(
        &self,
        shutdown: tokio_util::sync::CancellationToken,
    ) -> Result<(), CheckpointPersistenceError> {
        while !shutdown.is_cancelled() {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = tokio::time::sleep(Duration::from_millis(500)) => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
