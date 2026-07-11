//! Checkpoints / verifications / provenance read routes (+ shared `render_outcome`).
//! Moved from `lib.rs` (issue #43); pure code move.

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};

use seasoned_hand_core::auth::{Action, AuthContext};
use seasoned_hand_core::verifier::routes::{
    ListQuery as VerifyListQuery, get_verification, list_verifications,
};

use crate::AppState;
use crate::error::{ApiError, api_err};
use crate::guards::{
    authorize_in_handler, require_loopback, require_session_tenant, require_task_tenant,
    require_verification_tenant,
};

// ---------------------------------------------------------------------------
// Story 1.13: checkpoints list HTTP handler.
// ---------------------------------------------------------------------------

/// Translate the shared `RouteOutcome<T>` into an axum response. `label`
/// is logged on the Internal arm so the access log carries which route
/// failed. The Ok arm hand-rolls the Response so a serde failure doesn't
/// panic the request (we fall back to an empty body — the caller will
/// see a 200 with no JSON, which is preferable to a 500 from a panic
/// during error rendering).
pub(crate) fn render_outcome<T: serde::Serialize>(
    label: &'static str,
    outcome: seasoned_hand_core::routes::RouteOutcome<T>,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    use axum::http::header;
    use axum::response::Response;
    use seasoned_hand_core::routes::RouteOutcome;
    match outcome {
        RouteOutcome::Ok(body) => {
            let bytes = serde_json::to_vec(&body).unwrap_or_default();
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/json")
                .body(axum::body::Body::from(bytes))
                .unwrap_or_else(|_| Response::new(axum::body::Body::empty())))
        }
        RouteOutcome::NotFound(msg) => Err(api_err(StatusCode::NOT_FOUND, msg)),
        RouteOutcome::Internal(msg) => {
            tracing::error!(error = %msg, route = label, "route failed");
            Err(api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error".into(),
            ))
        }
    }
}

pub(crate) async fn list_checkpoints_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(session_id): Path<String>,
    Query(q): Query<seasoned_hand_core::checkpoint::routes::ListQuery>,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskRead, &auth_ctx)?;
    require_session_tenant(&state, &session_id, &auth_ctx).await?;
    use seasoned_hand_core::checkpoint::routes::list_checkpoints;
    render_outcome(
        "list_checkpoints",
        list_checkpoints(&state.checkpoints, &session_id, q).await,
    )
}

// ---------------------------------------------------------------------------
// Story 1.9: verifier HTTP route handlers — thin axum wrappers over the
// pure RouteOutcome layer in seasoned_hand_core::verifier::routes.
// ---------------------------------------------------------------------------

pub(crate) async fn list_verifications_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(session_id): Path<String>,
    Query(q): Query<VerifyListQuery>,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskRead, &auth_ctx)?;
    require_session_tenant(&state, &session_id, &auth_ctx).await?;
    render_outcome(
        "list_verifications",
        list_verifications(&state.verifications, &session_id, q).await,
    )
}

pub(crate) async fn get_verification_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(id): Path<String>,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskRead, &auth_ctx)?;
    require_verification_tenant(&state, &id, &auth_ctx).await?;
    render_outcome(
        "get_verification",
        get_verification(&state.verifications, &id).await,
    )
}

// ---------------------------------------------------------------------------
// Story 2.15: per-task provenance manifest.
// ---------------------------------------------------------------------------

pub(crate) async fn get_task_provenance_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(task_id): Path<String>,
    Query(q): Query<seasoned_hand_core::provenance::GetTaskProvenanceQuery>,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    // Loopback gate matches every sibling /v1/tasks/:id/* handler; provenance
    // manifests can include PII (sender addresses, brief content, intake
    // metadata) so they must not leak at HOST=0.0.0.0 binds. See REVIEW
    // §5 cross-cutting issue #1 / proposed DEBT #34.
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskRead, &auth_ctx)?;
    require_task_tenant(&state, &task_id, &auth_ctx).await?;
    use seasoned_hand_core::provenance::{GetTaskProvenanceDeps, get_task_provenance};
    let deps = GetTaskProvenanceDeps {
        deliverables: state.deliverables.as_ref(),
        delivery_events: state.delivery_events.as_ref(),
        sandbox: state.sandbox.as_ref(),
        db: &state.db,
    };
    render_outcome(
        "get_task_provenance",
        get_task_provenance(&task_id, q, deps).await,
    )
}
