//! HTTP error response machinery for the control-plane API (issue #22, batch F).
//!
//! Extracted from the `lib.rs` god-file: the `ApiError` envelope, the
//! `ApiResult`/`ApiErrorResponse` aliases, the `api_err` constructor, and the
//! `map_*_error` helpers that translate core-crate error enums into `(StatusCode,
//! Json<ApiError>)`. Pure over their inputs — no `AppState`, no I/O — so they live
//! cleanly apart from the route handlers. Further decomposition (guards, state,
//! per-domain route modules) is tracked as a follow-up.

use axum::Json;
use axum::http::StatusCode;
use serde::Serialize;

use seasoned_hand_core::org::InvitationError;
use seasoned_hand_core::sharing::sop::SopShareError;

#[derive(Serialize)]
pub(crate) struct ApiError {
    pub(crate) error: String,
}

pub(crate) type ApiErrorResponse = (StatusCode, Json<ApiError>);
pub(crate) type ApiResult<T> = Result<T, ApiErrorResponse>;

pub(crate) fn api_err(status: StatusCode, code: String) -> ApiErrorResponse {
    (status, Json(ApiError { error: code }))
}

pub(crate) fn map_invitation_error(err: InvitationError) -> ApiErrorResponse {
    match err {
        InvitationError::Auth(seasoned_hand_core::auth::AuthError::MissingTenantContext) => {
            api_err(StatusCode::UNAUTHORIZED, "unauthorized_context".into())
        }
        InvitationError::Auth(seasoned_hand_core::auth::AuthError::Unauthorized { .. }) => {
            api_err(StatusCode::FORBIDDEN, "forbidden_action".into())
        }
        InvitationError::OrganizationNotFound(_) => {
            api_err(StatusCode::NOT_FOUND, "organization_not_found".into())
        }
        InvitationError::CrossTenantDenied => {
            api_err(StatusCode::FORBIDDEN, "cross_tenant_denied".into())
        }
        InvitationError::InvalidRole(_) => api_err(StatusCode::BAD_REQUEST, "invalid_role".into()),
        // Issue #22: login-token verification outcomes. Collapse all three to a
        // single opaque 401 so a caller can't distinguish unknown / expired /
        // already-consumed (no token-state oracle).
        InvitationError::InvalidToken
        | InvitationError::TokenExpired
        | InvitationError::TokenAlreadyConsumed => {
            api_err(StatusCode::UNAUTHORIZED, "invalid_login_token".into())
        }
        InvitationError::Sqlite(_) | InvitationError::AuditWrite(_) => {
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
        }
    }
}

pub(crate) fn map_handoff_error(
    err: seasoned_hand_core::handoff::HandoffError,
) -> ApiErrorResponse {
    use seasoned_hand_core::handoff::HandoffError;
    match err {
        HandoffError::Auth(seasoned_hand_core::auth::AuthError::MissingTenantContext) => {
            api_err(StatusCode::UNAUTHORIZED, "unauthorized_context".into())
        }
        HandoffError::Auth(seasoned_hand_core::auth::AuthError::Unauthorized { .. }) => {
            api_err(StatusCode::FORBIDDEN, "forbidden_action".into())
        }
        HandoffError::TaskNotFound(_) => api_err(StatusCode::NOT_FOUND, "task_not_found".into()),
        HandoffError::UserNotFound(_) => api_err(StatusCode::NOT_FOUND, "user_not_found".into()),
        HandoffError::TerminalState(_) => api_err(StatusCode::CONFLICT, "task_terminal".into()),
        HandoffError::MustPauseFirst(_) => api_err(StatusCode::CONFLICT, "pause_required".into()),
        HandoffError::StaleRevision { .. } => {
            api_err(StatusCode::CONFLICT, "stale_revision".into())
        }
        HandoffError::InvalidStatus(_) => {
            api_err(StatusCode::CONFLICT, "invalid_task_status".into())
        }
        HandoffError::Sqlite(error) => {
            tracing::error!(%error, "handoff sqlite error");
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
        }
        HandoffError::Event(error) => {
            tracing::error!(%error, "handoff event error");
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
        }
    }
}

pub(crate) fn map_audit_query_error(
    err: seasoned_hand_core::audit::AuditQueryError,
) -> ApiErrorResponse {
    use seasoned_hand_core::audit::AuditQueryError;
    match err {
        AuditQueryError::Auth(seasoned_hand_core::auth::AuthError::MissingTenantContext) => {
            api_err(StatusCode::UNAUTHORIZED, "unauthorized_context".into())
        }
        AuditQueryError::Auth(seasoned_hand_core::auth::AuthError::Unauthorized { .. }) => {
            api_err(StatusCode::FORBIDDEN, "forbidden_action".into())
        }
        AuditQueryError::InvalidAction(_) => {
            api_err(StatusCode::BAD_REQUEST, "invalid_action_db".into())
        }
        AuditQueryError::Sqlite(error) => {
            tracing::error!(%error, "audit query sqlite error");
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
        }
    }
}

pub(crate) fn map_sop_share_error(err: SopShareError) -> ApiErrorResponse {
    match err {
        SopShareError::Auth(seasoned_hand_core::auth::AuthError::MissingTenantContext) => {
            api_err(StatusCode::UNAUTHORIZED, "unauthorized_context".into())
        }
        SopShareError::Auth(seasoned_hand_core::auth::AuthError::Unauthorized { .. }) => {
            api_err(StatusCode::FORBIDDEN, "forbidden_action".into())
        }
        SopShareError::SopNotFound(_) => api_err(StatusCode::NOT_FOUND, "sop_not_found".into()),
        SopShareError::UserNotFound(_) => api_err(StatusCode::NOT_FOUND, "user_not_found".into()),
        SopShareError::InvalidPermission(_) => api_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid_permission_db".into(),
        ),
        SopShareError::StaleRevision(_) => api_err(StatusCode::CONFLICT, "stale_revision".into()),
        SopShareError::Db(error) => {
            tracing::error!(%error, "sop_share db error");
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
        }
    }
}
