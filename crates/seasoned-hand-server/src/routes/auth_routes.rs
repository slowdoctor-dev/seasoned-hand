//! Verified-session auth endpoints (`/v1/auth/login`, dev login) — issue #7 / ADR-018.
//! Moved from `lib.rs` (issue #43); pure code move.

use axum::{Json, extract::State, http::StatusCode};

use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::error::{ApiError, api_err};
use crate::guards::require_loopback;

// ---------------------------------------------------------------------------
// Issue #7 / ADR-018: verified-session auth endpoints.

#[derive(Deserialize)]
pub(crate) struct LoginRequest {
    invitation_token: String,
}

#[derive(Serialize)]
pub(crate) struct LoginResponse {
    access_token: String,
    expires_at: i64,
    tenant_id: String,
    organization_id: String,
    actor_user_id: String,
    org_role: String,
}

pub(crate) fn role_str(role: seasoned_hand_core::auth::Role) -> &'static str {
    use seasoned_hand_core::auth::Role;
    match role {
        Role::Admin => "admin",
        Role::User => "user",
        Role::Viewer => "viewer",
    }
}

pub(crate) fn login_response(result: seasoned_hand_core::auth::LoginResult) -> Json<LoginResponse> {
    Json(LoginResponse {
        access_token: result.token,
        expires_at: result.expires_at,
        tenant_id: result.context.tenant_id,
        organization_id: result.context.organization_id,
        actor_user_id: result.context.actor_user_id,
        org_role: role_str(result.context.org_role).to_string(),
    })
}

/// Exchange a single-use invitation token for a session token. Public
/// (unauthenticated) by necessity — it mints the first credential.
pub(crate) async fn post_auth_login_handler(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, Json<ApiError>)> {
    use seasoned_hand_core::auth::AuthLoginError;
    match state.auth_sessions.login(&body.invitation_token).await {
        Ok(result) => Ok(login_response(result)),
        Err(AuthLoginError::InvalidInvitation) => Err(api_err(
            StatusCode::UNAUTHORIZED,
            "invalid_invitation".into(),
        )),
        Err(AuthLoginError::NoMembership) => {
            Err(api_err(StatusCode::FORBIDDEN, "no_membership".into()))
        }
        Err(AuthLoginError::Db(_)) => Err(api_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "login_failed".into(),
        )),
    }
}

/// Loopback-gated dev affordance: issue a session for a default dev identity so
/// the browser UI works in local dev before the real client login flow (#26).
/// Refuses unless `SH_INSECURE_AUTH_HEADERS` is set AND the caller is loopback.
pub(crate) async fn post_auth_dev_login_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
) -> Result<Json<LoginResponse>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    if !state.allow_insecure_headers {
        return Err(api_err(StatusCode::FORBIDDEN, "dev_login_disabled".into()));
    }
    state
        .auth_sessions
        .issue_dev_session()
        .await
        .map(login_response)
        .map_err(|_| api_err(StatusCode::INTERNAL_SERVER_ERROR, "dev_login_failed".into()))
}
