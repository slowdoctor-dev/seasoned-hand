//! Pure route layer for the provenance manifest — exposes
//! [`get_task_provenance`] as a `RouteOutcome` producer so the server
//! crate can adapt it to an axum handler with a thin wrapper.
//!
//! Read-time semantics:
//!   1. Resolve the target Deliverable (latest by `created_at` or by
//!      `?deliverable_id=`).
//!   2. Inflate the stored manifest — inline JSON is returned as-is;
//!      `{"$ref": ...}` is read back via [`SandboxClient`].
//!   3. Overlay live `delivery_events` for the deliverable so
//!      `delivered_to[]` always reflects what actually shipped.
//!
//! refs: /specs/phase-2/architecture.md §2.11
//! refs: /specs/phase-2/stories/story-2.15.md

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::builder::{ProvenanceError, list_sessions_for_task};
use super::manifest::{DeliveredTo, ProvenanceManifest};
use super::spill::parse_workspace_uri;
use crate::db::DbPool;
use crate::deliverable::{Deliverable, DeliverableError, DeliverableStore};
use crate::delivery::store::DeliveryEventStore;
use crate::routes::RouteOutcome;
use crate::sandbox::SandboxClient;

#[derive(Debug, Default, Deserialize)]
pub struct GetTaskProvenanceQuery {
    pub deliverable_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProvenanceResponse {
    pub deliverable_id: String,
    pub manifest: ProvenanceManifest,
}

pub struct GetTaskProvenanceDeps<'a> {
    pub deliverables: &'a DeliverableStore,
    pub delivery_events: &'a DeliveryEventStore,
    pub sandbox: &'a SandboxClient,
    pub db: &'a DbPool,
}

pub async fn get_task_provenance(
    task_id: &str,
    query: GetTaskProvenanceQuery,
    deps: GetTaskProvenanceDeps<'_>,
) -> RouteOutcome<ProvenanceResponse> {
    let deliverable =
        match resolve_deliverable(task_id, query.deliverable_id, deps.deliverables).await {
            Ok(d) => d,
            Err(outcome) => return outcome,
        };

    let session_id = match latest_session_for_task(deps.db, task_id).await {
        Ok(Some(s)) => s,
        Ok(None) => return RouteOutcome::NotFound("no_sessions_for_task".into()),
        Err(e) => return RouteOutcome::Internal(e.to_string()),
    };

    let mut manifest = match resolve_manifest(
        deliverable.provenance_manifest.clone(),
        &session_id,
        deps.sandbox,
    )
    .await
    {
        Ok(m) => m,
        Err(e) => return RouteOutcome::Internal(e.to_string()),
    };

    let live_deliveries = match deps
        .delivery_events
        .list_by_deliverable(&deliverable.id)
        .await
    {
        Ok(rows) => rows,
        Err(e) => return RouteOutcome::Internal(e.to_string()),
    };
    manifest.delivered_to = live_deliveries
        .into_iter()
        .map(|e| DeliveredTo {
            channel: e.channel,
            delivery_id: e.id,
            delivered_at: e.delivered_at,
            ok: e.ok,
            external_id: e.external_id,
        })
        .collect();

    RouteOutcome::Ok(ProvenanceResponse {
        deliverable_id: deliverable.id,
        manifest,
    })
}

async fn resolve_deliverable(
    task_id: &str,
    explicit: Option<String>,
    store: &DeliverableStore,
) -> Result<Deliverable, RouteOutcome<ProvenanceResponse>> {
    if let Some(did) = explicit {
        return match store.get(&did).await {
            Ok(d) if d.task_id == task_id => Ok(d),
            Ok(_) => Err(RouteOutcome::NotFound("deliverable_not_in_task".into())),
            Err(DeliverableError::NotFound(_)) => {
                Err(RouteOutcome::NotFound("deliverable_not_found".into()))
            }
            Err(e) => Err(RouteOutcome::Internal(e.to_string())),
        };
    }
    let mut rows = match store.list_by_task(task_id).await {
        Ok(r) => r,
        Err(e) => return Err(RouteOutcome::Internal(e.to_string())),
    };
    // list_by_task is `ORDER BY created_at ASC`; the latest is the
    // last row.
    rows.pop()
        .ok_or_else(|| RouteOutcome::NotFound("no_deliverables_for_task".into()))
}

/// Inflate either an inline manifest or a `{"$ref": "file://..."}`
/// pointer back into a typed [`ProvenanceManifest`].
pub async fn resolve_manifest(
    column_value: Value,
    session_id: &str,
    sandbox: &SandboxClient,
) -> Result<ProvenanceManifest, ProvenanceError> {
    if let Some(Value::String(uri)) = column_value.get("$ref") {
        let path = parse_workspace_uri(uri)?;
        let bytes = sandbox.read_workspace_file(session_id, &path).await?;
        return Ok(serde_json::from_slice(&bytes)?);
    }
    Ok(serde_json::from_value(column_value)?)
}

async fn latest_session_for_task(
    pool: &DbPool,
    task_id: &str,
) -> Result<Option<String>, ProvenanceError> {
    // Prefer the most-recently created session — it's the one most
    // likely to still have its sandbox workspace alive (Phase 2 spec
    // §2.6: pause-resume cycles share a workspace).
    let sessions = list_sessions_for_task(pool, task_id).await?;
    Ok(sessions.into_iter().next_back().map(|s| s.id))
}
