//! Project + task HTTP routes backing the CLI (story 2.21a) + lifecycle mapping.
//! Moved from `lib.rs` (issue #43); pure code move.

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};

use seasoned_hand_core::auth::{Action, AuthContext};
use serde::Deserialize;

use crate::AppState;
use crate::error::{ApiError, api_err};
use crate::guards::{
    authorize_in_handler, require_loopback, require_project_tenant, require_task_tenant,
};
use crate::ws;

// ---------------------------------------------------------------------------
// Story 2.21a: project + task HTTP routes that back the
// `seasoned-hand` CLI binary. Loopback-only — Phase 5 multi-user will
// add real auth and lift the constraint (BASELINE §8). The pause /
// resume / cancel routes delegate to the shared `ws::handle_task_*`
// helpers so the WS and HTTP entrypoints stay structurally identical.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
pub(crate) struct ProjectsListQuery {
    limit: Option<usize>,
    cursor: Option<i64>,
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateProjectBody {
    title: String,
    #[serde(default)]
    description: Option<String>,
}

pub(crate) async fn list_projects_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Query(q): Query<ProjectsListQuery>,
) -> Result<Json<Vec<seasoned_hand_core::project::Project>>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskRead, &auth_ctx)?;
    let status = match q.status.as_deref() {
        Some("active") => Some(seasoned_hand_core::project::ProjectStatus::Active),
        Some("archived") => Some(seasoned_hand_core::project::ProjectStatus::Archived),
        Some(_other) => {
            return Err(api_err(StatusCode::BAD_REQUEST, "unknown_status".into()));
        }
        None => None,
    };
    let limit = q.limit.unwrap_or(50);
    state
        .projects
        .list_by_tenant(&auth_ctx.tenant_id, status, q.cursor, limit)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!(error = %e, "list_projects");
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
        })
}

pub(crate) async fn create_project_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Json(body): Json<CreateProjectBody>,
) -> Result<(StatusCode, Json<seasoned_hand_core::project::Project>), (StatusCode, Json<ApiError>)>
{
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskWrite, &auth_ctx)?;
    if body.title.trim().is_empty() {
        return Err(api_err(StatusCode::BAD_REQUEST, "empty_title".into()));
    }
    let id = state
        .projects
        .insert(seasoned_hand_core::project::NewProject {
            tenant_id: Some(auth_ctx.tenant_id.clone()),
            title: body.title,
            description: body.description,
        })
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "create_project");
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
        })?;
    let row = state.projects.get(&id).await.map_err(|e| {
        tracing::error!(error = %e, "create_project::get");
        api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
    })?;
    Ok((StatusCode::CREATED, Json(row)))
}

pub(crate) async fn archive_project_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskWrite, &auth_ctx)?;
    require_project_tenant(&state, &id, &auth_ctx).await?;
    match state
        .projects
        .set_status(&id, seasoned_hand_core::project::ProjectStatus::Archived)
        .await
    {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(seasoned_hand_core::project::ProjectError::NotFound(_)) => {
            Err(api_err(StatusCode::NOT_FOUND, "project_not_found".into()))
        }
        Err(e) => {
            tracing::error!(error = %e, "archive_project");
            Err(api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error".into(),
            ))
        }
    }
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct TasksListQuery {
    limit: Option<usize>,
    cursor: Option<i64>,
    status: Option<String>,
}

pub(crate) async fn list_project_tasks_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(project_id): Path<String>,
    Query(q): Query<TasksListQuery>,
) -> Result<Json<Vec<seasoned_hand_core::project::Task>>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskRead, &auth_ctx)?;
    require_project_tenant(&state, &project_id, &auth_ctx).await?;
    let status = match q.status.as_deref() {
        Some(s) => match seasoned_hand_core::project::TaskStatus::from_db_str(s) {
            Ok(st) => Some(st),
            Err(_) => {
                return Err(api_err(StatusCode::BAD_REQUEST, "unknown_status".into()));
            }
        },
        None => None,
    };
    let limit = q.limit.unwrap_or(50);
    state
        .tasks
        .list_by_project_and_tenant(&project_id, &auth_ctx.tenant_id, status, q.cursor, limit)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!(error = %e, "list_project_tasks");
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
        })
}

pub(crate) async fn get_task_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(id): Path<String>,
) -> Result<Json<seasoned_hand_core::project::Task>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskRead, &auth_ctx)?;
    require_task_tenant(&state, &id, &auth_ctx).await?;
    match state.tasks.get(&id).await {
        Ok(task) => Ok(Json(task)),
        Err(seasoned_hand_core::project::TaskError::NotFound(_)) => {
            Err(api_err(StatusCode::NOT_FOUND, "task_not_found".into()))
        }
        Err(e) => {
            tracing::error!(error = %e, "get_task");
            Err(api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error".into(),
            ))
        }
    }
}

/// Story 2.22 backend: list every Deliverable row for a task and return
/// the latest session_id alongside. The frontend AgentComputer
/// `DeliverablesTab` joins these to build a download URL via the
/// existing `GET /v1/workspace/:session_id/*sub_path` proxy.
// Shared with the wasm UI via seasoned-hand-dto (story 6.3b); wraps the
// re-exported Deliverable (itself a dto type) + the latest session id.
use seasoned_hand_dto::TaskDeliverablesResponse;

pub(crate) async fn list_task_deliverables_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(task_id): Path<String>,
) -> Result<Json<TaskDeliverablesResponse>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskRead, &auth_ctx)?;
    require_task_tenant(&state, &task_id, &auth_ctx).await?;
    let deliverables = state
        .deliverables
        .list_by_task_and_tenant(&task_id, &auth_ctx.tenant_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "list_task_deliverables");
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
        })?;
    let latest_session_id = ws::lookup_latest_session_for_task(&state, &task_id).await;
    Ok(Json(TaskDeliverablesResponse {
        deliverables,
        latest_session_id,
    }))
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct TaskPauseBody {
    #[serde(default)]
    durable: Option<bool>,
}

pub(crate) async fn post_task_pause_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(task_id): Path<String>,
    body: Option<Json<TaskPauseBody>>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskWrite, &auth_ctx)?;
    // Confirm the task exists AND belongs to the caller's tenant
    // (P5-HARD-IT3-H4) before we touch session state.
    require_task_tenant(&state, &task_id, &auth_ctx).await?;
    let durable = body.and_then(|Json(b)| b.durable).unwrap_or(true);
    let session_id = ws::lookup_latest_session_for_task(&state, &task_id)
        .await
        .ok_or(api_err(StatusCode::CONFLICT, "no_active_session".into()))?;
    map_lifecycle_result(ws::handle_task_pause(&state, &session_id, durable).await)
}

pub(crate) async fn post_task_resume_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(task_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskWrite, &auth_ctx)?;
    // P5-HARD-IT3-H4: tenant-scope the task before resuming.
    require_task_tenant(&state, &task_id, &auth_ctx).await?;
    let session_id = ws::lookup_latest_session_for_task(&state, &task_id)
        .await
        .ok_or(api_err(StatusCode::CONFLICT, "no_active_session".into()))?;
    map_lifecycle_result(ws::handle_task_resume(&state, &session_id).await)
}

pub(crate) async fn post_task_cancel_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(task_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskWrite, &auth_ctx)?;
    // P5-HARD-IT3-H4: tenant-scope BEFORE the set_status write — cancel
    // directly mutates the row, so a missing tenant check here is a
    // cross-tenant write (the worst of the H4 class).
    require_task_tenant(&state, &task_id, &auth_ctx).await?;
    // Drive the Task state machine first — Phase 2 widened
    // `legal_transitions` so Drafted/Briefed/Confirmed/Running/Paused
    // all → Cancelled. Terminal task → 409 wrong_state. NotFound → 404.
    match state
        .tasks
        .set_status(&task_id, seasoned_hand_core::project::TaskStatus::Cancelled)
        .await
    {
        Ok(()) => {}
        Err(seasoned_hand_core::project::TaskError::NotFound(_)) => {
            return Err(api_err(StatusCode::NOT_FOUND, "task_not_found".into()));
        }
        Err(seasoned_hand_core::project::TaskError::IllegalTransition { from, .. }) => {
            return Err(api_err(
                StatusCode::CONFLICT,
                format!("wrong_state:{}", from.as_db_str()),
            ));
        }
        Err(other) => {
            tracing::error!(error = %other, "task_cancel::set_status");
            return Err(api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error".into(),
            ));
        }
    }
    // If there's a live session, cascade the cancel through the same
    // ws helper so sandbox teardown + Misc emission run exactly once.
    // No active session is fine — Drafted/Briefed task cancels never
    // had one to begin with.
    if let Some(session_id) = ws::lookup_latest_session_for_task(&state, &task_id).await
        && let Err(reason) = ws::handle_task_cancel(&state, &session_id).await
    {
        // Already-terminal session is fine on a cancel — the task row
        // is now Cancelled regardless. Surface other errors.
        if reason != "wrong_state" {
            tracing::warn!(
                %reason,
                %session_id,
                "task_cancel: session-side teardown reported a non-terminal error"
            );
        }
    }
    Ok(StatusCode::ACCEPTED)
}

pub(crate) fn map_lifecycle_result(
    res: Result<(), String>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    match res {
        Ok(()) => Ok(StatusCode::ACCEPTED),
        Err(reason) => {
            let status = match reason.as_str() {
                "wrong_state" => StatusCode::CONFLICT,
                "unknown_session" => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            Err(api_err(status, reason))
        }
    }
}
