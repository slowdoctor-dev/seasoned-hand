//! Event-stream read routes: `/v1/sessions/:id/events` (+ redacted / raw-admin views).
//! Moved from `lib.rs` (issue #43); pure code move.

use std::str::FromStr;

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};

use seasoned_hand_core::audit::AuditLogger;
use seasoned_hand_core::auth::{Action, AuthContext};
use seasoned_hand_core::events::{EventQuery, EventStore, EventType};
use serde::Deserialize;

use crate::AppState;
use crate::error::{ApiError, api_err};
use crate::guards::{authorize_in_handler, require_loopback, require_session_tenant};

#[derive(Debug, Deserialize, Default)]
pub struct EventsQueryParams {
    pub after_id: Option<i64>,
    #[serde(rename = "type")]
    pub event_type: Option<String>,
    pub limit: Option<usize>,
}

pub(crate) async fn list_events(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(session_id): Path<String>,
    Query(params): Query<EventsQueryParams>,
) -> Result<Json<Vec<seasoned_hand_core::events::Event>>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskRead, &auth_ctx)?;
    let event_type = match params.event_type.as_deref() {
        Some(s) => Some(
            EventType::from_str(s)
                .map_err(|_| api_err(StatusCode::BAD_REQUEST, "unknown_event_type".into()))?,
        ),
        None => None,
    };

    let filter = EventQuery {
        after_id: params.after_id,
        event_type,
        limit: params.limit,
    };

    // Issue #22: route through the canonical tenant guard instead of an inline
    // `JOIN projects ... p.tenant_id = ?`. The inner join excluded chat-spawned
    // sessions (project_id NULL, tenancy from task_id); `require_session_tenant`
    // applies the shared fail-closed `SESSION_TENANT_PREDICATE`, so the legitimate
    // owner of a task-spawned session no longer gets a spurious 404 (and a
    // mismatched-parent session stays invisible to every tenant).
    require_session_tenant(&state, &session_id, &auth_ctx).await?;

    match state.events.query(&session_id, filter).await {
        Ok(events) => Ok(Json(events)),
        Err(seasoned_hand_core::events::EventError::SessionNotFound(_)) => {
            Err(api_err(StatusCode::NOT_FOUND, "session_not_found".into()))
        }
        Err(other) => {
            tracing::error!(error = %other, "events query failed");
            Err(api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error".into(),
            ))
        }
    }
}

// --- Story 5.16: tenant-visible event read + admin raw-event route -----

#[derive(Debug, Deserialize, Default)]
pub struct VisibleEventsQueryParams {
    pub after_event_id: Option<i64>,
    pub limit: Option<usize>,
}

/// `GET /v1/events/:session_id` — returns rows from `tenant_event_view`
/// filtered by the caller's tenant + role visibility. Redacted at write
/// time (story 5.14); no raw `events.data` is exposed here regardless
/// of role.
pub(crate) async fn list_redacted_events(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(session_id): Path<String>,
    Query(params): Query<VisibleEventsQueryParams>,
) -> Result<
    Json<Vec<seasoned_hand_core::events::visibility::VisibleEventRow>>,
    (StatusCode, Json<ApiError>),
> {
    require_loopback(remote)?;
    // No `Action`-level gate here — the tenant + visibility predicates
    // inside `visibility::query` ARE the gate (architecture §7).
    let q = seasoned_hand_core::events::visibility::EventReadQuery {
        after_event_id: params.after_event_id,
        limit: params.limit,
    };
    match seasoned_hand_core::events::visibility::query(&state.db, &auth_ctx, &session_id, q).await
    {
        Ok(rows) => Ok(Json(rows)),
        Err(err) => {
            tracing::error!(error = %err, "visibility::query failed");
            Err(api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error".into(),
            ))
        }
    }
}

/// `GET /v1/admin/events/:session_id/raw` — admin-only forensic read of
/// raw `events.data`. Gated by `Action::EventRawRead`; every call
/// writes an `audit_log` row via [`AuditLogger`] before returning, so
/// the access is non-repudiable. Cross-tenant admins are blocked even
/// with the action right — the session's tenant must match the
/// caller's tenant.
pub(crate) async fn list_raw_events_admin(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(session_id): Path<String>,
    Query(params): Query<VisibleEventsQueryParams>,
) -> Result<
    Json<Vec<seasoned_hand_core::events::visibility::RawEventRow>>,
    (StatusCode, Json<ApiError>),
> {
    require_loopback(remote)?;
    let audit = AuditLogger::new(state.db.clone(), state.events.clone());
    let q = seasoned_hand_core::events::visibility::EventReadQuery {
        after_event_id: params.after_event_id,
        limit: params.limit,
    };
    match seasoned_hand_core::events::visibility::query_raw(
        &state.db,
        &auth_ctx,
        &audit,
        &session_id,
        q,
    )
    .await
    {
        Ok(rows) => Ok(Json(rows)),
        Err(seasoned_hand_core::events::visibility::VisibilityQueryError::Auth(_)) => {
            Err(api_err(StatusCode::FORBIDDEN, "forbidden_action".into()))
        }
        Err(err) => {
            tracing::error!(error = %err, "visibility::query_raw failed");
            Err(api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error".into(),
            ))
        }
    }
}
