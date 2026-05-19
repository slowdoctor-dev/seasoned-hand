//! Event-stream replay helpers used by [`super::resume::resume_task`]
//! when the sandbox container is gone at resume time.
//!
//! Each helper reads from a single `SqliteEventStore` (filtered by
//! `session_id` — the OLD session's events) and writes the
//! reconstructed artifact under the NEW session's id. The helpers
//! never read the live runtime state because the old container is
//! gone; the event stream is the only source of truth.
//!
//! refs: /specs/phase-2/architecture.md §2.6, §8
//! refs: /specs/phase-2/stories/story-2.16.md

use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

use crate::agent::init::feature_list::{Feature, FeatureList, FeatureStatus};
use crate::db::DbPool;
use crate::events::{EventError, EventQuery, EventStore, EventType, sqlite::SqliteEventStore};
use crate::plan::{PhaseStatus, Plan, PlanError, PlanManager};
use crate::sandbox::SandboxError;
use crate::time::now_micros;

/// Discriminator for [`ReplayError`] — the rebuild path embeds this in
/// the `task_resume_rebuild_failed` Misc + the Task's `failure_reason`
/// so an operator can see which replay step blew up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayStep {
    Plan,
    FeatureList,
    Progress,
    CostBaseline,
}

impl ReplayStep {
    pub fn as_str(self) -> &'static str {
        match self {
            ReplayStep::Plan => "plan",
            ReplayStep::FeatureList => "feature_list",
            ReplayStep::Progress => "progress",
            ReplayStep::CostBaseline => "cost_baseline",
        }
    }
}

#[derive(Debug, Error)]
pub enum ReplayError {
    #[error("replay step '{step}' failed: {source}")]
    Step {
        step: &'static str,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("events: {0}")]
    Events(#[from] EventError),
    #[error("plan: {0}")]
    Plan(#[from] PlanError),
    #[error("sandbox: {0}")]
    Sandbox(#[from] SandboxError),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("db: {0}")]
    Db(#[from] crate::db::DbError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

impl ReplayError {
    pub fn step(&self) -> Option<&'static str> {
        if let ReplayError::Step { step, .. } = self {
            Some(step)
        } else {
            None
        }
    }
}

/// Per-session paging limit — matches the provenance builder's choice.
/// Long sessions paginate via `after_id` until the page count drops
/// below the limit.
const EVENT_PAGE_LIMIT: usize = 1000;

/// Lift the latest `Plan{op:"create"|"update"|"advance"}` event from
/// `old_session_id` and install it as the plan for `new_session_id`.
///
/// Returns `Ok(None)` if no plan event ever fired in the old session
/// (legacy Phase 0 / Phase 1 sessions are valid without plans). On
/// success, the new session has a `plans` row whose `phases` blob
/// matches the snapshot byte-for-byte and a `Plan{op:"create"}` Misc
/// event so downstream replay (status, feature-list, progress) lines
/// up against a real plan.
pub async fn replay_plan(
    events: &SqliteEventStore,
    plan_manager: &PlanManager,
    db: &DbPool,
    old_session_id: &str,
    new_session_id: &str,
) -> Result<Option<Plan>, ReplayError> {
    let latest = latest_plan_event(events, old_session_id).await?;
    let Some(snapshot) = latest else {
        return Ok(None);
    };
    let installed = restore_plan_row(db, new_session_id, &snapshot).await?;
    // Emit a synthetic Plan{op:"create"} so the new session's event
    // timeline reflects the install — keeps PlanManager invariants
    // (`PlanManager::create` always emits one) without touching the
    // private DB writer twice.
    plan_manager
        .emit_replay_create(new_session_id, &installed)
        .await
        .map_err(ReplayError::Plan)?;
    Ok(Some(installed))
}

/// Pull every `feature_done` Misc event from `old_session_id`, project
/// it onto the freshly-seeded FeatureList that `seed_plan` (story 2.8)
/// would have written, and write the result to
/// `/workspace/feature-list.json` under `new_session_id`.
///
/// The new session's initial FeatureList comes from `derive_feature_list`
/// (the same helper the Initializer used to seed the old session). If
/// `plan` is `None`, no feature-list exists on the new session — the
/// resume path skips the write (matches the no-plan branch).
pub async fn replay_feature_list<W: WorkspaceWriter>(
    events: &SqliteEventStore,
    sandbox: &W,
    plan: Option<&Plan>,
    old_session_id: &str,
    new_session_id: &str,
) -> Result<Option<FeatureList>, ReplayError> {
    let Some(plan) = plan else {
        return Ok(None);
    };
    let mut list = derive_feature_list_from_plan(plan);
    let done_ids = collect_misc_with_kind(events, old_session_id, "feature_done").await?;
    let now = now_micros();
    let mut done_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    for ev in done_ids {
        if let Some(fid) = ev.get("feature_id").and_then(Value::as_str) {
            done_set.insert(fid.to_string());
        }
    }
    for feature in list.features.iter_mut() {
        if done_set.contains(&feature.id) {
            feature.status = FeatureStatus::Done;
            feature.completed_at = Some(now);
        }
    }
    sandbox
        .write_workspace_file_json_value(
            new_session_id,
            "feature-list.json",
            &serde_json::to_value(&list)?,
        )
        .await?;
    Ok(Some(list))
}

/// Replay every `progress_update` event from `old_session_id` into a
/// fresh progress.txt under `new_session_id`. The initial lines come
/// from the plan (matches Initializer's `initial_progress_lines`).
///
/// `progress_recite` events are skipped — they're tail snapshots, not
/// new content. Aggregating them would double-write existing lines.
pub async fn replay_progress<W: WorkspaceWriter>(
    events: &SqliteEventStore,
    sandbox: &W,
    plan: Option<&Plan>,
    old_session_id: &str,
    new_session_id: &str,
) -> Result<(), ReplayError> {
    let mut text = match plan {
        Some(p) => initial_progress_text_from_plan(p),
        None => String::new(),
    };
    let updates = collect_misc_with_kind(events, old_session_id, "progress_update").await?;
    for ev in updates {
        if let Some(line) = ev.get("line").and_then(Value::as_str) {
            text = append_replay_line(&text, line);
        }
    }
    sandbox
        .write_workspace_file(new_session_id, "progress.txt", text.as_bytes())
        .await?;
    Ok(())
}

/// Phase 2 stub: the rebuild path does NOT reset a per-session cost
/// baseline. Phase 0/1 `CostClient` is stateless (the caller holds the
/// baseline as a snapshot value), and the new session's
/// `sessions.cost_cents` row starts at 0 — cost accounting resumes
/// from zero on the new session. The cumulative-cost-across-rebuild
/// case is Phase 2 DEBT (see DEBT ledger entry added by story 2.16).
///
/// Returns `Ok(())` always; kept as an explicit step so the
/// rebuild-and-replay caller can sequence it consistently with the
/// other replay helpers and emit a single `task_resume_rebuild_failed`
/// event shape on failure.
pub async fn replay_cost_baseline(
    _events: &SqliteEventStore,
    _old_session_id: &str,
    _new_session_id: &str,
) -> Result<(), ReplayError> {
    Ok(())
}

/// Workspace-write seam consumed by the replay helpers. The production
/// impl is on `SandboxClient`; tests substitute a tempdir-backed or
/// always-fail impl to exercise the failure transition path without
/// docker.
#[allow(async_fn_in_trait)]
pub trait WorkspaceWriter: Send + Sync {
    async fn write_workspace_file(
        &self,
        session_id: &str,
        relative_path: &str,
        contents: &[u8],
    ) -> Result<(), SandboxError>;
    async fn write_workspace_file_json_value(
        &self,
        session_id: &str,
        relative_path: &str,
        value: &Value,
    ) -> Result<(), SandboxError>;
}

impl WorkspaceWriter for crate::sandbox::SandboxClient {
    async fn write_workspace_file(
        &self,
        session_id: &str,
        relative_path: &str,
        contents: &[u8],
    ) -> Result<(), SandboxError> {
        Self::write_workspace_file(self, session_id, relative_path, contents).await
    }
    async fn write_workspace_file_json_value(
        &self,
        session_id: &str,
        relative_path: &str,
        value: &Value,
    ) -> Result<(), SandboxError> {
        Self::write_workspace_file_json(self, session_id, relative_path, value).await
    }
}

/// Direct INSERT into `plans` for the rebuild path. Bypasses
/// `PlanManager::create`'s `normalize_create_phases` (which would
/// reset every phase status to `Pending` / first → `Active`) so the
/// restored snapshot keeps its real `Done` / `Active` markers.
///
/// `pub(crate)` so tests / external callers can re-use the same
/// install for fixtures; the resume path is the primary caller.
pub async fn restore_plan_row(
    db: &DbPool,
    new_session_id: &str,
    snapshot: &Plan,
) -> Result<Plan, ReplayError> {
    let plan_id = Uuid::new_v4().to_string();
    let session = new_session_id.to_string();
    let goal = snapshot.goal.clone();
    let phases = snapshot.phases.clone();
    let current_phase_id = snapshot.current_phase_id;
    let phases_json = serde_json::to_string(&phases)?;
    let now = now_micros();
    let plan_id_clone = plan_id.clone();
    let installed: Plan = db
        .with_conn(move |conn| -> rusqlite::Result<Plan> {
            conn.execute(
                "INSERT INTO plans (id, session_id, goal, phases, current_phase_id, \
                 created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    &plan_id_clone,
                    &session,
                    &goal,
                    &phases_json,
                    current_phase_id,
                    now,
                    now,
                ],
            )?;
            Ok(Plan {
                id: plan_id_clone,
                session_id: session,
                goal,
                phases,
                current_phase_id,
            })
        })
        .await?;
    Ok(installed)
}

async fn latest_plan_event(
    events: &SqliteEventStore,
    session_id: &str,
) -> Result<Option<Plan>, ReplayError> {
    let mut after: Option<i64> = None;
    let mut latest: Option<Plan> = None;
    loop {
        let page = events
            .query(
                session_id,
                EventQuery {
                    after_id: after,
                    event_type: Some(EventType::Plan),
                    limit: Some(EVENT_PAGE_LIMIT),
                },
            )
            .await?;
        if page.is_empty() {
            break;
        }
        for ev in &page {
            if let Some(snapshot) = ev.data.get("snapshot")
                && let Ok(p) = serde_json::from_value::<Plan>(snapshot.clone())
            {
                latest = Some(p);
            }
        }
        after = page.last().map(|e| e.id);
        if page.len() < EVENT_PAGE_LIMIT {
            break;
        }
    }
    Ok(latest)
}

async fn collect_misc_with_kind(
    events: &SqliteEventStore,
    session_id: &str,
    kind: &str,
) -> Result<Vec<Value>, ReplayError> {
    let mut after: Option<i64> = None;
    let mut out = Vec::new();
    loop {
        let page = events
            .query(
                session_id,
                EventQuery {
                    after_id: after,
                    event_type: Some(EventType::Misc),
                    limit: Some(EVENT_PAGE_LIMIT),
                },
            )
            .await?;
        if page.is_empty() {
            break;
        }
        for ev in &page {
            if ev.data.get("kind").and_then(Value::as_str) == Some(kind) {
                out.push(ev.data.clone());
            }
        }
        after = page.last().map(|e| e.id);
        if page.len() < EVENT_PAGE_LIMIT {
            break;
        }
    }
    Ok(out)
}

fn derive_feature_list_from_plan(plan: &Plan) -> FeatureList {
    let mut phases = plan.phases.clone();
    phases.sort_by_key(|p| p.id);
    let active = plan.current_phase_id;
    let features = phases
        .iter()
        .enumerate()
        .map(|(idx, phase)| Feature {
            id: format!("f-{}", idx + 1),
            title: phase.title.clone(),
            status: if Some(phase.id) == active {
                FeatureStatus::Doing
            } else if phase.status == PhaseStatus::Done {
                FeatureStatus::Done
            } else {
                FeatureStatus::Todo
            },
            depends_on: if idx == 0 {
                vec![]
            } else {
                vec![format!("f-{}", idx)]
            },
            plan_phase_id: phase.id,
            completed_at: None,
            notes: None,
        })
        .collect();
    FeatureList {
        version: 1,
        goal: plan.goal.clone(),
        features,
    }
}

fn initial_progress_text_from_plan(plan: &Plan) -> String {
    let mut out = String::new();
    out.push_str(&format!("Goal: {}\n", plan.goal));
    for phase in &plan.phases {
        out.push_str(&format!("- Phase {}: {}\n", phase.id, phase.title));
    }
    out
}

/// Mirror of [`crate::agent::init::progress::append_line`] but with a
/// deterministic timestamp prefix (`replay`) so a replayed progress.txt
/// is distinguishable from an organically-grown one without bloating
/// the line format.
fn append_replay_line(existing: &str, line: &str) -> String {
    let truncated = crate::agent::init::progress::truncate_line(line);
    let mut out = String::with_capacity(existing.len() + truncated.len() + 32);
    out.push_str(existing);
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!("replay         user           {truncated}\n"));
    out
}
