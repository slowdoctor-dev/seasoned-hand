//! `resume_task` — Phase 2 durable-resume entry point.
//!
//! Looks up the Task's most recent Session, checks whether the
//! sandbox container is still alive (via the in-memory handle cache
//! — see Phase 2 DEBT note on cross-process-restart), and either:
//!
//! - takes the Phase 1 unpause path (`SandboxClient::resume` +
//!   sessions row → RUNNING + `task_resumed` Misc + spawn runner), or
//! - rebuilds: emits `task_resume_rebuild_required`, allocates a new
//!   sandbox under a fresh session_id linked to the same task_id,
//!   replays the prior session's event stream into the new session's
//!   Plan / feature-list / progress, and starts the runner.
//!
//! On any rebuild-step failure the Task transitions to
//! `failed{reason:"replay_failed:<step>"}` with a
//! `task_resume_rebuild_failed` Misc carrying the offending step
//! name. No silent recovery — the architecture §8 row explicitly
//! requires the operator-visible failure mode.
//!
//! refs: /specs/phase-2/architecture.md §2.6, §8
//! refs: /specs/phase-2/stories/story-2.16.md

use rusqlite::OptionalExtension;
use serde_json::json;
use thiserror::Error;
use uuid::Uuid;

use crate::db::DbPool;
use crate::events::{EventError, EventStore, EventType, NewEvent, sqlite::SqliteEventStore};
use crate::plan::PlanManager;
use crate::project::{TaskError, TaskStatus, TaskStore};
use crate::sandbox::{SandboxClient, SandboxError, SandboxHandle};
use crate::task::replay::{
    ReplayError, ReplayStep, WorkspaceWriter, replay_cost_baseline, replay_feature_list,
    replay_plan, replay_progress,
};

use crate::time::now_micros;
/// Minimal sandbox lifecycle surface the resume path needs. The
/// production impl is on [`SandboxClient`]; tests substitute a fake
/// that pre-registers handles so the rebuild path runs without docker.
#[allow(async_fn_in_trait)]
pub trait SandboxOps: WorkspaceWriter {
    async fn get_handle(&self, session_id: &str) -> Option<SandboxHandle>;
    async fn create_handle(&self, session_id: &str) -> Result<SandboxHandle, SandboxError>;
    async fn unpause(&self, session_id: &str) -> Result<(), SandboxError>;
}

impl SandboxOps for SandboxClient {
    async fn get_handle(&self, session_id: &str) -> Option<SandboxHandle> {
        Self::get(self, session_id).await
    }
    async fn create_handle(&self, session_id: &str) -> Result<SandboxHandle, SandboxError> {
        Self::create(self, session_id).await
    }
    async fn unpause(&self, session_id: &str) -> Result<(), SandboxError> {
        Self::resume(self, session_id).await
    }
}

#[derive(Debug, Error)]
pub enum ResumeError {
    #[error("task not found: {0}")]
    TaskNotFound(String),
    #[error("no session for task: {0}")]
    NoSession(String),
    #[error("task in wrong state: {0:?}")]
    WrongState(TaskStatus),
    #[error("sandbox: {0}")]
    Sandbox(#[from] SandboxError),
    #[error("task store: {0}")]
    Task(#[from] TaskError),
    #[error("events: {0}")]
    Events(#[from] EventError),
    #[error("db: {0}")]
    Db(#[from] crate::db::DbError),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("replay: {0}")]
    Replay(#[from] ReplayError),
}

/// Where `resume_task` ended up — surfaces to the WS / HTTP shim so
/// it can ack-and-spawn the runner against the right session id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeOutcome {
    /// Sandbox handle was still in the cache; the prior session id
    /// keeps its docker container and goes RUNNING.
    UnpausedExisting { session_id: String },
    /// Sandbox container was gone; rebuild + replay produced a new
    /// session row that the caller should now drive.
    Rebuilt {
        old_session_id: String,
        new_session_id: String,
    },
}

/// Bundle of references resume_task needs. All borrowed — the call is
/// short-lived and never escapes the WS handler.
pub struct ResumeDeps<'a, S: SandboxOps> {
    pub task_store: &'a TaskStore,
    pub events: &'a SqliteEventStore,
    pub plan_manager: &'a PlanManager,
    pub sandbox: &'a S,
    pub db: &'a DbPool,
}

pub async fn resume_task<S: SandboxOps>(
    task_id: &str,
    deps: ResumeDeps<'_, S>,
) -> Result<ResumeOutcome, ResumeError> {
    let task = deps.task_store.get(task_id).await?;
    if task.status != TaskStatus::Paused && task.status != TaskStatus::Running {
        // Allow Running (re-entrancy: the existing WS path drives
        // session-state directly via Phase 1 1.17; the matching Task
        // state move from Paused → Running may not yet have happened
        // through the task_store for legacy WS tasks).
        return Err(ResumeError::WrongState(task.status));
    }
    let session_id = latest_session_for_task(deps.db, task_id)
        .await?
        .ok_or_else(|| ResumeError::NoSession(task_id.to_string()))?;
    match deps.sandbox.get_handle(&session_id).await {
        Some(_) => {
            unpause_existing(task_id, &session_id, &deps).await?;
            Ok(ResumeOutcome::UnpausedExisting { session_id })
        }
        None => {
            let new_session_id = rebuild_and_replay(task_id, &session_id, &deps).await?;
            Ok(ResumeOutcome::Rebuilt {
                old_session_id: session_id,
                new_session_id,
            })
        }
    }
}

async fn unpause_existing<S: SandboxOps>(
    task_id: &str,
    session_id: &str,
    deps: &ResumeDeps<'_, S>,
) -> Result<(), ResumeError> {
    deps.sandbox.unpause(session_id).await?;
    set_session_state(deps.db, session_id, "RUNNING").await?;
    // Best-effort task → Running. Legacy WS-only tasks have no
    // TaskStore row; ignore NotFound. IllegalTransition can fire when
    // the task is already Running (re-entrant resume); also ignore.
    let _ = deps
        .task_store
        .set_status(task_id, TaskStatus::Running)
        .await;
    deps.events
        .append(NewEvent {
            session_id: session_id.to_string(),
            event_type: EventType::Misc,
            source: "ws".into(),
            data: json!({"kind": "task_resumed"}),
        })
        .await?;
    Ok(())
}

async fn rebuild_and_replay<S: SandboxOps>(
    task_id: &str,
    old_session_id: &str,
    deps: &ResumeDeps<'_, S>,
) -> Result<String, ResumeError> {
    // Step 0: announce intent on the OLD session timeline so any
    // subscriber sees the rebuild trigger before the new session
    // appears.
    deps.events
        .append(NewEvent {
            session_id: old_session_id.to_string(),
            event_type: EventType::Misc,
            source: "task::resume".into(),
            data: json!({
                "kind": "task_resume_rebuild_required",
                "old_session_id": old_session_id,
            }),
        })
        .await?;

    // Step 1: mint the new session_id + persist the row BEFORE
    // creating the sandbox (so event append on the new session's id
    // doesn't fail the FK check in SqliteEventStore::append).
    let new_session_id = Uuid::new_v4().to_string();
    let task_id_owned = task_id.to_string();
    let new_sid = new_session_id.clone();
    let now = now_micros();
    deps.db
        .with_conn(move |conn| {
            conn.execute(
                "INSERT INTO sessions (id, created_at, updated_at, state, task_id) \
                 VALUES (?, ?, ?, 'RUNNING', ?)",
                rusqlite::params![new_sid, now, now, task_id_owned],
            )
        })
        .await?;

    // Step 2: create the sandbox container. The renderer install
    // (story 2.6) re-runs unconditionally — adds ~30-60 s per
    // rebuild. `SANDBOX_SKIP_RENDERER_INSTALL=1` skips it in tests.
    // On failure: surface as a fatal rebuild error, fail the task.
    let create_result = deps.sandbox.create_handle(&new_session_id).await;
    if let Err(error) = create_result {
        record_rebuild_failure(
            deps,
            task_id,
            &new_session_id,
            "sandbox_create",
            &error.to_string(),
        )
        .await?;
        return Err(ResumeError::Sandbox(error));
    }

    // Step 3+: run the four replay helpers in fixed order. Each
    // surfaces a `ReplayStep` discriminator on failure so the Misc
    // payload + the task failure_reason can name the exact step.
    if let Err(error) = run_replay_steps(deps, old_session_id, &new_session_id).await {
        let step = match &error {
            ResumeError::Replay(e) => e.step().unwrap_or("unknown"),
            _ => "unknown",
        };
        record_rebuild_failure(deps, task_id, &new_session_id, step, &error.to_string()).await?;
        return Err(error);
    }

    // Step N: task → Running. If the task was Paused (durable pause
    // flow) the move is Paused → Running; if it was Running already
    // (transitional state in the WS shim) the call is a no-op error
    // we swallow.
    let _ = deps
        .task_store
        .set_status(task_id, TaskStatus::Running)
        .await;

    deps.events
        .append(NewEvent {
            session_id: new_session_id.clone(),
            event_type: EventType::Misc,
            source: "task::resume".into(),
            data: json!({
                "kind": "task_resumed",
                "rebuilt_from": old_session_id,
            }),
        })
        .await?;

    Ok(new_session_id)
}

async fn run_replay_steps<S: SandboxOps>(
    deps: &ResumeDeps<'_, S>,
    old_session_id: &str,
    new_session_id: &str,
) -> Result<(), ResumeError> {
    let plan = replay_plan(
        deps.events,
        deps.plan_manager,
        deps.db,
        old_session_id,
        new_session_id,
    )
    .await
    .map_err(|e| wrap_step(e, ReplayStep::Plan))?;

    replay_feature_list(
        deps.events,
        deps.sandbox,
        plan.as_ref(),
        old_session_id,
        new_session_id,
    )
    .await
    .map_err(|e| wrap_step(e, ReplayStep::FeatureList))?;

    replay_progress(
        deps.events,
        deps.sandbox,
        plan.as_ref(),
        old_session_id,
        new_session_id,
    )
    .await
    .map_err(|e| wrap_step(e, ReplayStep::Progress))?;

    replay_cost_baseline(deps.events, old_session_id, new_session_id)
        .await
        .map_err(|e| wrap_step(e, ReplayStep::CostBaseline))?;

    Ok(())
}

/// Tag a [`ReplayError`] with the step that produced it. If the error
/// is already a `Step` variant (from a nested helper) the inner step
/// is preserved.
fn wrap_step(error: ReplayError, step: ReplayStep) -> ResumeError {
    if matches!(error, ReplayError::Step { .. }) {
        return ResumeError::Replay(error);
    }
    ResumeError::Replay(ReplayError::Step {
        step: step.as_str(),
        source: Box::new(error),
    })
}

async fn record_rebuild_failure<S: SandboxOps>(
    deps: &ResumeDeps<'_, S>,
    task_id: &str,
    new_session_id: &str,
    step: &str,
    error: &str,
) -> Result<(), ResumeError> {
    if let Err(event_error) = deps
        .events
        .append(NewEvent {
            session_id: new_session_id.to_string(),
            event_type: EventType::Misc,
            source: "task::resume".into(),
            data: json!({
                "kind": "task_resume_rebuild_failed",
                "step": step,
                "error": error,
            }),
        })
        .await
    {
        tracing::warn!(
            task_id = %task_id,
            session_id = %new_session_id,
            %event_error,
            "task resume: failed to append rebuild failure event",
        );
    }
    let reason = format!("replay_failed:{step}");
    if let Err(set_failure_error) = deps.task_store.set_failure(task_id, &reason).await {
        tracing::warn!(
            %task_id,
            %set_failure_error,
            "task resume: failed to persist failure status after rebuild error",
        );
    }
    Ok(())
}

async fn latest_session_for_task(
    pool: &DbPool,
    task_id: &str,
) -> Result<Option<String>, ResumeError> {
    let tid = task_id.to_string();
    let result: rusqlite::Result<Option<String>> = pool
        .with_conn(move |conn| {
            conn.query_row(
                "SELECT id FROM sessions WHERE task_id = ? \
                  ORDER BY created_at DESC LIMIT 1",
                [&tid],
                |row| row.get::<_, String>(0),
            )
            .optional()
        })
        .await;
    Ok(result?)
}

async fn set_session_state(
    pool: &DbPool,
    session_id: &str,
    state_name: &str,
) -> Result<(), ResumeError> {
    let session_id = session_id.to_string();
    let state_name = state_name.to_string();
    pool.with_conn(move |conn| {
        conn.execute(
            "UPDATE sessions SET state = ?, updated_at = ? WHERE id = ?",
            (state_name, now_micros(), session_id),
        )
    })
    .await?;
    Ok(())
}
