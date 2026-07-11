//! Loopback/admin-token guards + admin routes (checkpoint rollback, sandbox cleanup).
//! Moved from `lib.rs` (issue #43); pure code move.

use std::net::SocketAddr;

use axum::{
    Extension, Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};

use seasoned_hand_core::auth::{Action, AuthContext};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

use crate::AppState;
use crate::error::{ApiResult, api_err};
use crate::guards::{authorize_in_handler, require_loopback, require_session_tenant};

// ---------------------------------------------------------------------------
// Loopback guard helper — shared by Phase 1 admin routes (1.13b rollback,
// 2.17 cleanup) AND the Phase 2 CLI surface (2.21a /v1/projects + /v1/tasks).
// Phase 2 single-operator deployments bind the server to `127.0.0.1`;
// Phase 5 multi-user will replace this with real auth, but the
// guard's job stays the same — keep these routes off the public surface.
// ---------------------------------------------------------------------------

pub(crate) const ADMIN_TOKEN_HEADER: &str = "X-Seasoned-Hand-Admin-Token";

pub(crate) fn require_admin_token_configured(state: &AppState) -> ApiResult<()> {
    if state.admin_token.is_empty() {
        return Err(api_err(
            StatusCode::SERVICE_UNAVAILABLE,
            "admin_token_not_configured".into(),
        ));
    }
    Ok(())
}

pub(crate) fn require_admin_token_header(state: &AppState, headers: &HeaderMap) -> ApiResult<()> {
    let token_hdr = headers
        .get(ADMIN_TOKEN_HEADER)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let ok: bool = token_hdr
        .as_bytes()
        .ct_eq(state.admin_token.as_bytes())
        .into();
    if ok {
        Ok(())
    } else {
        Err(api_err(
            StatusCode::UNAUTHORIZED,
            "unauthorized_token".into(),
        ))
    }
}

pub(crate) fn require_admin_route(
    state: &AppState,
    remote: SocketAddr,
    headers: &HeaderMap,
) -> ApiResult<()> {
    // Guard order is intentional:
    // 1. Missing server config is a local operator setup error.
    // 2. Non-loopback peers stop before token comparison, preserving
    //    the timing/status behavior pinned by the admin route tests.
    // 3. Token comparison is constant-time defense in depth.
    require_admin_token_configured(state)?;
    require_loopback(remote)?;
    require_admin_token_header(state, headers)
}

// ---------------------------------------------------------------------------
// Story 1.13b: admin rollback handler. Loopback-bound, token-gated.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct RollbackBody {
    pub(crate) reason: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct RollbackResponse {
    checkpoint_id: String,
    rolled_back_at: i64,
}

pub(crate) async fn post_checkpoint_rollback_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    headers: HeaderMap,
    Path((session_id, checkpoint_id)): Path<(String, String)>,
    Json(body): Json<RollbackBody>,
) -> ApiResult<(StatusCode, Json<RollbackResponse>)> {
    require_admin_route(&state, remote, &headers)?;
    authorize_in_handler(Action::TaskWrite, &auth_ctx)?;
    require_session_tenant(&state, &session_id, &auth_ctx).await?;
    // Guard 4: reason length.
    if body.reason.len() > 200 {
        return Err(api_err(StatusCode::BAD_REQUEST, "reason_too_long".into()));
    }

    // Guard 5: session state must NOT be RUNNING or VERIFYING.
    let session_state = state
        .db
        .with_conn({
            let sid = session_id.clone();
            move |conn| -> rusqlite::Result<Option<String>> {
                let mut stmt = conn.prepare("SELECT state FROM sessions WHERE id = ?")?;
                let mut rows =
                    stmt.query_map(rusqlite::params![sid], |row| row.get::<_, String>(0))?;
                match rows.next() {
                    Some(r) => Ok(Some(r?)),
                    None => Ok(None),
                }
            }
        })
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "rollback: session state query");
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
        })?;
    match session_state.as_deref() {
        Some("RUNNING") | Some("VERIFYING") => {
            return Err(api_err(StatusCode::CONFLICT, "wrong_state".into()));
        }
        None => {
            return Err(api_err(StatusCode::NOT_FOUND, "session_not_found".into()));
        }
        _ => {}
    }

    // Guard 6: sandbox must not be paused.
    let paused = state.sandbox.is_paused(&session_id).await.map_err(|e| {
        tracing::warn!(error = %e, "rollback: sandbox paused-state probe failed");
        api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
    })?;
    if paused {
        return Err(api_err(StatusCode::CONFLICT, "sandbox_paused".into()));
    }

    // All guards passed — dispatch the internal tool. The mask layer
    // affects only what's exposed to the LLM, so direct dispatch works.
    let ctx = seasoned_hand_core::tools::ToolContext {
        session_id: session_id.clone(),
        mask_mode: seasoned_hand_core::dispatch::mask::AgentMode::Internal,
        events: state.events.clone(),
        sandbox: state.sandbox.clone(),
        search: state.search.clone(),
        plan_manager: state.plan_manager.clone(),
        checkpoint_labels: state.checkpoint_labels.clone(),
        checkpoints: state.checkpoints.clone(),
        matcher_mode: seasoned_hand_core::matcher::MatcherMode::Production,
    };
    let out = state
        .dispatcher
        .dispatch(
            &ctx,
            "checkpoint_rollback",
            serde_json::json!({
                "checkpoint_id": checkpoint_id,
                "reason": body.reason,
                "rolled_back_by": "admin:cli",
            }),
        )
        .await;
    if !out.ok {
        let err_kind = out
            .error
            .as_ref()
            .map(|e| e.kind.clone())
            .unwrap_or_else(|| "tool_error".to_string());
        let status = match err_kind.as_str() {
            "checkpoint_not_found" => StatusCode::NOT_FOUND,
            "reason_too_long" => StatusCode::BAD_REQUEST,
            "revert_failed" => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        return Err(api_err(status, err_kind));
    }
    let rolled_back_at = out
        .output
        .get("rolled_back_at")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    Ok((
        StatusCode::ACCEPTED,
        Json(RollbackResponse {
            checkpoint_id,
            rolled_back_at,
        }),
    ))
}

// ---------------------------------------------------------------------------
// Story 2.17: admin workspace-cleanup handler.
// ---------------------------------------------------------------------------

pub(crate) async fn post_admin_sandbox_cleanup_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
) -> ApiResult<(StatusCode, Json<seasoned_hand_core::task::TtlCleanupReport>)> {
    require_admin_route(&state, remote, &headers)?;
    let report = state.workspace_ttl_cron.cleanup_cycle().await;
    Ok((StatusCode::OK, Json(report)))
}
