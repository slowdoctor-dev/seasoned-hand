//! Route-classification + request-guard helpers (issue #22, batch F).
//!
//! Extracted from the `lib.rs` god-file: the explicit auth-classification wrappers
//! (`with_auth` / `public` / `self_gated`, issue #21), the coarse in-handler RBAC
//! gate, and the loopback guard. These are small and `AppState`-independent (only
//! the `MethodRouter<AppState>` *type* is referenced), plus — issue #43 round 2 —
//! the AppState-coupled tenant guards (`require_task_tenant` /
//! `require_project_tenant` / `require_session_tenant` /
//! `require_verification_tenant`) and the shared fail-closed
//! `SESSION_TENANT_PREDICATE` (issue #22 batch B). Moved byte-identical from
//! `lib.rs`; behavior is pinned by the `events.rs` / `route_auth.rs`
//! integration suites. The per-domain `routes/<domain>.rs` split is the
//! remaining #43 follow-up.

use std::net::SocketAddr;

use axum::Extension;
use axum::http::StatusCode;
use axum::middleware;
use axum::routing::MethodRouter;

use seasoned_hand_core::auth::{Action, AuthContext, authorize_coarse};

use crate::AppState;
use crate::auth;
use crate::error::{ApiResult, api_err};

/// Issue #21 — explicit route auth classification. Every route in `app()` is
/// wrapped by exactly one of `with_auth` (protected), `public`, or `self_gated`,
/// so there are no bare/unclassified routes that silently skip auth. `with_auth`
/// attaches the verified-session + coarse-RBAC middleware; `public`/`self_gated`
/// are explicit markers that document why a route carries no session gate.
pub(crate) fn with_auth(route: MethodRouter<AppState>, action: Action) -> MethodRouter<AppState> {
    route
        .route_layer(middleware::from_fn(auth::middleware::require_auth_context))
        .layer(Extension(auth::middleware::RouteAction(action)))
}

/// A genuinely public route (no authentication): health + the login endpoints
/// that mint the first credential. Identity wrapper — the classification is the
/// documentation.
pub(crate) fn public(route: MethodRouter<AppState>) -> MethodRouter<AppState> {
    route
}

/// A route that performs its OWN authentication in the handler (loopback and/or
/// admin/webhook token) rather than via a verified session — operational /
/// machine endpoints. Identity wrapper; the handler MUST self-guard.
pub(crate) fn self_gated(route: MethodRouter<AppState>) -> MethodRouter<AppState> {
    route
}

pub(crate) fn authorize_in_handler(action: Action, ctx: &AuthContext) -> ApiResult<()> {
    authorize_coarse(action, ctx).map_err(|err| match err {
        seasoned_hand_core::auth::AuthError::MissingTenantContext => {
            api_err(StatusCode::UNAUTHORIZED, "unauthorized_context".into())
        }
        seasoned_hand_core::auth::AuthError::Unauthorized { .. } => {
            api_err(StatusCode::FORBIDDEN, "forbidden_action".into())
        }
    })
}

pub(crate) fn require_loopback(remote: SocketAddr) -> ApiResult<()> {
    if remote.ip().is_loopback() {
        Ok(())
    } else {
        Err(api_err(
            StatusCode::FORBIDDEN,
            "forbidden_non_loopback".into(),
        ))
    }
}

/// Hardening P5-HARD-IT3-H4: confirm a task belongs to the caller's
/// tenant before any single-resource `:id` operation. The RBAC
/// `with_auth(..., Action::TaskWrite/TaskRead)` layer gates the *verb*
/// but not the *row* — without this guard a tenant-A caller could
/// pause/resume/cancel/read tenant-B's task by id. Returns 404 (not
/// 403) on a tenant mismatch so cross-tenant existence isn't leaked,
/// identical to a genuinely missing id.
pub(crate) async fn require_task_tenant(
    state: &AppState,
    task_id: &str,
    auth: &AuthContext,
) -> ApiResult<()> {
    let task = state.tasks.get(task_id).await.map_err(|e| match e {
        seasoned_hand_core::project::TaskError::NotFound(_) => {
            api_err(StatusCode::NOT_FOUND, "task_not_found".into())
        }
        other => {
            tracing::error!(error = %other, "require_task_tenant::lookup");
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
        }
    })?;
    if task.tenant_id.as_deref() != Some(auth.tenant_id.as_str()) {
        return Err(api_err(StatusCode::NOT_FOUND, "task_not_found".into()));
    }
    Ok(())
}

pub(crate) async fn require_project_tenant(
    state: &AppState,
    project_id: &str,
    auth: &AuthContext,
) -> ApiResult<()> {
    let project = state.projects.get(project_id).await.map_err(|e| match e {
        seasoned_hand_core::project::ProjectError::NotFound(_) => {
            api_err(StatusCode::NOT_FOUND, "project_not_found".into())
        }
        other => {
            tracing::error!(error = %other, "require_project_tenant::lookup");
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
        }
    })?;
    if project.tenant_id.as_deref() != Some(auth.tenant_id.as_str()) {
        return Err(api_err(StatusCode::NOT_FOUND, "project_not_found".into()));
    }
    Ok(())
}

/// Canonical "session `s` belongs to the bound tenant" predicate (issue #22
/// batch B review). **Fail-closed**: every present *direct* parent (project via
/// `s.project_id`, task via `s.task_id`) must match the tenant, and orphan
/// sessions with no parent are excluded. A row whose project and task resolve to
/// *different* tenants therefore belongs to **neither** — it is corrupt and
/// invisible to all, instead of leaking to whichever parent a tenant happens to
/// share (a plain `COALESCE(p, t)` trusted one parent and ignored a conflicting
/// other). Requires `LEFT JOIN projects p ON p.id = s.project_id` and
/// `LEFT JOIN tasks t ON t.id = s.task_id`, and binds the tenant parameter
/// **twice** (in this clause's order). FK enforcement means a non-null
/// `project_id`/`task_id` with a NULL joined `tenant_id` can only be a dangling
/// reference, which this clause also rejects.
pub(crate) const SESSION_TENANT_PREDICATE: &str = "(s.project_id IS NULL OR p.tenant_id = ?) \
     AND (s.task_id IS NULL OR t.tenant_id = ?) \
     AND (s.project_id IS NOT NULL OR s.task_id IS NOT NULL)";

pub(crate) async fn require_session_tenant(
    state: &AppState,
    session_id: &str,
    auth: &AuthContext,
) -> ApiResult<()> {
    let sid = session_id.to_string();
    let tenant = auth.tenant_id.clone();
    let exists = state
        .db
        .with_conn(move |conn| {
            conn.query_row::<i64, _, _>(
                &format!(
                    "SELECT 1
                       FROM sessions s
                       LEFT JOIN projects p ON p.id = s.project_id
                       LEFT JOIN tasks t ON t.id = s.task_id
                      WHERE s.id = ? AND {SESSION_TENANT_PREDICATE}"
                ),
                rusqlite::params![sid, tenant, tenant],
                |row| row.get(0),
            )
            .is_ok()
        })
        .await;
    if !exists {
        return Err(api_err(StatusCode::NOT_FOUND, "session_not_found".into()));
    }
    Ok(())
}

pub(crate) async fn require_verification_tenant(
    state: &AppState,
    verification_id: &str,
    auth: &AuthContext,
) -> ApiResult<()> {
    let verification_id = verification_id.to_string();
    let tenant_id = auth.tenant_id.clone();
    let exists = state
        .db
        .with_conn(move |conn| {
            conn.query_row::<i64, _, _>(
                &format!(
                    "SELECT 1
                       FROM verifications v
                       JOIN sessions s ON s.id = v.session_id
                  LEFT JOIN projects p ON p.id = s.project_id
                  LEFT JOIN tasks t ON t.id = s.task_id
                      WHERE v.id = ? AND {SESSION_TENANT_PREDICATE}"
                ),
                rusqlite::params![verification_id, tenant_id, tenant_id],
                |row| row.get(0),
            )
            .is_ok()
        })
        .await;
    if !exists {
        return Err(api_err(
            StatusCode::NOT_FOUND,
            "verification_not_found".into(),
        ));
    }
    Ok(())
}
