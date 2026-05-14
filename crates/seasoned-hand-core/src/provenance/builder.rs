//! Builder for [`ProvenanceManifest`] — loads every evidence row a
//! Deliverable depends on and stitches the manifest together.
//!
//! refs: /specs/phase-2/architecture.md §2.11
//! refs: /specs/phase-2/stories/story-2.15.md

use rusqlite::params;
use thiserror::Error;

use super::manifest::{
    BriefProvenance, CheckpointProvenance, DeliveredTo, IntakeProvenance, ProvenanceManifest,
    ProvenanceMetrics, SCHEMA_VERSION, SessionProvenance,
};
use crate::checkpoint::{CheckpointPersistenceError, CheckpointStore};
use crate::db::DbPool;
use crate::delivery::store::{DeliveryEventStore, DeliveryStoreError};
use crate::events::{EventError, EventQuery, EventStore, EventType, sqlite::SqliteEventStore};
use crate::intake::store::{IntakeEventStore, IntakeStoreError};
use crate::project::{ProjectError, ProjectStore, Task, TaskError, TaskStore};
use crate::verifier::{VerificationStore, VerifierPersistenceError};

const SYNTHETIC_INTAKE_CHANNEL: &str = "unknown";
const SYNTHETIC_INTAKE_ID: &str = "synthetic";
/// Per-session event page size when walking events to find briefing /
/// decision rows. The Phase 2 ceiling is `EventQuery::effective_limit`
/// (1000); a long task with > 1000 events per session paginates via
/// `after_id`.
const EVENT_PAGE_LIMIT: usize = 1000;

#[derive(Debug, Error)]
pub enum ProvenanceError {
    #[error("task not found: {0}")]
    TaskNotFound(String),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("sandbox: {0}")]
    Sandbox(#[from] crate::sandbox::SandboxError),
    #[error("task store: {0}")]
    Task(#[from] TaskError),
    #[error("project store: {0}")]
    Project(#[from] ProjectError),
    #[error("intake store: {0}")]
    Intake(#[from] IntakeStoreError),
    #[error("delivery store: {0}")]
    Delivery(#[from] DeliveryStoreError),
    #[error("verifier persistence: {0}")]
    Verifier(#[from] VerifierPersistenceError),
    #[error("checkpoint persistence: {0}")]
    Checkpoint(#[from] CheckpointPersistenceError),
    #[error("events: {0}")]
    Events(#[from] EventError),
    #[error("invalid file-ref: {0}")]
    InvalidFileRef(String),
}

/// Borrowed handles the builder needs. All references — the builder is
/// short-lived and never escapes the `task_deliver` call site.
pub struct BuildDeps<'a> {
    pub task_store: &'a TaskStore,
    pub project_store: &'a ProjectStore,
    pub intake_store: &'a IntakeEventStore,
    pub delivery_store: &'a DeliveryEventStore,
    pub events: &'a SqliteEventStore,
    pub verifications: &'a VerificationStore,
    pub checkpoints: &'a CheckpointStore,
    pub db: &'a DbPool,
}

/// Per-Deliverable inputs known to `task_deliver` BEFORE the row is
/// persisted (architecture §2.11 calls out that the manifest is built
/// at deliverable-persist time and stored on the same INSERT).
pub struct ManifestInputs<'a> {
    pub task_id: &'a str,
    pub deliverable_id: &'a str,
    pub rendered_content_sha256: &'a str,
    pub source_content_sha256: Option<&'a str>,
    pub citations: &'a [i64],
}

/// Build the full manifest. Loads task/project/intake/brief/sessions/
/// decisions/verdicts/checkpoints/deliveries and aggregates metrics.
pub async fn build_manifest(
    inputs: ManifestInputs<'_>,
    deps: &BuildDeps<'_>,
) -> Result<ProvenanceManifest, ProvenanceError> {
    let task = deps.task_store.get(inputs.task_id).await?;
    let project = deps.project_store.get(&task.project_id).await?;
    let intake = load_intake(deps.intake_store, &task).await?;

    let sessions_raw = list_sessions_for_task(deps.db, inputs.task_id).await?;
    let session_ids: Vec<String> = sessions_raw.iter().map(|s| s.id.clone()).collect();

    let (brief_event_id, decisions) = walk_misc_events(deps.events, &session_ids).await?;

    let mut verdict_ids: Vec<String> = Vec::new();
    let mut checkpoints: Vec<CheckpointProvenance> = Vec::new();
    let mut tool_calls: u64 = 0;
    for sid in &session_ids {
        for v in deps.verifications.list_by_session(sid, None, 200).await? {
            verdict_ids.push(v.id);
        }
        for cp in deps.checkpoints.list_by_session(sid, None, 200).await? {
            checkpoints.push(CheckpointProvenance {
                checkpoint_id: cp.id,
                git_sha: cp.git_sha,
                rolled_back: cp.rolled_back_at.is_some(),
            });
        }
        tool_calls += count_actions(deps.events, sid).await?;
    }

    let delivered_to: Vec<DeliveredTo> = deps
        .delivery_store
        .list_by_deliverable(inputs.deliverable_id)
        .await?
        .into_iter()
        .map(|e| DeliveredTo {
            channel: e.channel,
            delivery_id: e.id,
            delivered_at: e.delivered_at,
            ok: e.ok,
            external_id: e.external_id,
        })
        .collect();

    let metrics = compute_metrics(&sessions_raw, tool_calls, verdict_ids.len() as u32);

    let sessions = sessions_raw
        .into_iter()
        .map(SessionProvenance::from)
        .collect();

    Ok(ProvenanceManifest {
        schema_version: SCHEMA_VERSION,
        task_id: inputs.task_id.to_string(),
        project_id: project.id,
        tenant_id: task.tenant_id,
        intake,
        brief: BriefProvenance {
            brief_event_id,
            // Phase 2 stub: every brief is treated as confirmed once a
            // Deliverable lands. Real confirm/edit accounting lives in
            // story 2.8 (Initializer); the value here is a Phase 3
            // close-out (manifest reflects confirmed state, not consent
            // lineage).
            confirmed: true,
            confirmed_at: None,
            edits_applied: 0,
        },
        sessions,
        decisions,
        verifier_verdicts: verdict_ids,
        checkpoints,
        metrics,
        delivered_to,
        source_content_sha256: inputs.source_content_sha256.map(str::to_string),
        rendered_content_sha256: inputs.rendered_content_sha256.to_string(),
        citations: inputs.citations.to_vec(),
    })
}

async fn load_intake(
    store: &IntakeEventStore,
    task: &Task,
) -> Result<IntakeProvenance, ProvenanceError> {
    match store.get_by_task_id(&task.id).await? {
        Some(row) => Ok(IntakeProvenance {
            channel: row.channel,
            intake_id: row.intake_id,
            received_at: row.received_at,
            metadata: row.metadata,
        }),
        // Phase 2 also creates tasks via the legacy WS `task_create` path
        // (no intake row). The manifest schema (§2.11) treats `intake`
        // as mandatory, so we synthesize a minimal entry rather than
        // dropping the field — keeps downstream consumers schema-stable.
        None => Ok(IntakeProvenance {
            channel: SYNTHETIC_INTAKE_CHANNEL.into(),
            intake_id: SYNTHETIC_INTAKE_ID.into(),
            received_at: task.created_at,
            metadata: serde_json::json!({}),
        }),
    }
}

#[derive(Debug, Clone)]
pub(super) struct RawSession {
    pub id: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub state: String,
    pub cost_cents: i64,
}

impl From<RawSession> for SessionProvenance {
    fn from(s: RawSession) -> Self {
        let (ended_at, end_reason) = match s.state.as_str() {
            "FINISHED" => (Some(s.updated_at), Some("completed".into())),
            "SUSPENDED" => (Some(s.updated_at), Some("paused".into())),
            "ERROR" => (Some(s.updated_at), Some("failed".into())),
            // IDLE / RUNNING / VERIFYING — session still live.
            _ => (None, None),
        };
        SessionProvenance {
            id: s.id,
            started_at: s.created_at,
            ended_at,
            end_reason,
        }
    }
}

pub(super) async fn list_sessions_for_task(
    pool: &DbPool,
    task_id: &str,
) -> Result<Vec<RawSession>, ProvenanceError> {
    let tid = task_id.to_string();
    let rows = pool
        .with_conn(move |conn| -> rusqlite::Result<Vec<RawSession>> {
            let mut stmt = conn.prepare(
                "SELECT id, created_at, updated_at, state, cost_cents \
                   FROM sessions WHERE task_id = ? ORDER BY created_at ASC",
            )?;
            let rows = stmt
                .query_map(params![tid], |row| {
                    Ok(RawSession {
                        id: row.get(0)?,
                        created_at: row.get(1)?,
                        updated_at: row.get(2)?,
                        state: row.get(3)?,
                        cost_cents: row.get(4)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await?;
    Ok(rows)
}

/// Walk every Misc event across the task's sessions to find (a) the
/// FIRST briefing event id and (b) every decision event id. Single pass
/// per session, paginated by `after_id` so a long task still completes.
async fn walk_misc_events(
    events: &SqliteEventStore,
    session_ids: &[String],
) -> Result<(Option<i64>, Vec<i64>), ProvenanceError> {
    let mut brief: Option<i64> = None;
    let mut decisions: Vec<i64> = Vec::new();
    for sid in session_ids {
        let mut after: Option<i64> = None;
        loop {
            let page = events
                .query(
                    sid,
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
            for e in &page {
                let kind = e.data.get("kind").and_then(serde_json::Value::as_str);
                match kind {
                    Some("briefing") if brief.is_none() => brief = Some(e.id),
                    Some("decision") => decisions.push(e.id),
                    _ => {}
                }
            }
            after = page.last().map(|e| e.id);
            if page.len() < EVENT_PAGE_LIMIT {
                break;
            }
        }
    }
    Ok((brief, decisions))
}

async fn count_actions(
    events: &SqliteEventStore,
    session_id: &str,
) -> Result<u64, ProvenanceError> {
    let mut after: Option<i64> = None;
    let mut total: u64 = 0;
    loop {
        let page = events
            .query(
                session_id,
                EventQuery {
                    after_id: after,
                    event_type: Some(EventType::Action),
                    limit: Some(EVENT_PAGE_LIMIT),
                },
            )
            .await?;
        if page.is_empty() {
            break;
        }
        total += page.len() as u64;
        after = page.last().map(|e| e.id);
        if page.len() < EVENT_PAGE_LIMIT {
            break;
        }
    }
    Ok(total)
}

fn compute_metrics(
    sessions: &[RawSession],
    tool_calls: u64,
    verifier_runs: u32,
) -> ProvenanceMetrics {
    let sessions_count = sessions.len() as u32;
    let pause_resume_cycles = sessions_count.saturating_sub(1);
    let cost_cents = sessions.iter().map(|s| s.cost_cents).sum();
    // sessions.created_at/updated_at are micros (cf. `now_micros` in
    // every store). wall_seconds = (max(updated_at) - min(created_at)) /
    // 1_000_000.
    let wall_seconds = if sessions.is_empty() {
        0
    } else {
        let min_start = sessions.iter().map(|s| s.created_at).min().unwrap_or(0);
        let max_end = sessions.iter().map(|s| s.updated_at).max().unwrap_or(0);
        ((max_end - min_start) / 1_000_000).max(0)
    };
    ProvenanceMetrics {
        tool_calls,
        cost_cents,
        wall_seconds,
        sessions_count,
        pause_resume_cycles,
        verifier_runs,
    }
}
