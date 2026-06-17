//! Route-classification + request-guard helpers (issue #22, batch F).
//!
//! Extracted from the `lib.rs` god-file: the explicit auth-classification wrappers
//! (`with_auth` / `public` / `self_gated`, issue #21), the coarse in-handler RBAC
//! gate, and the loopback guard. These are small and `AppState`-independent (only
//! the `MethodRouter<AppState>` *type* is referenced). The richer, AppState-coupled
//! tenant guards (`require_session_tenant`, …) stay in `lib.rs` for now — moving
//! them, `AppState` (`state.rs`), and the handlers (`routes/<domain>.rs`) is the
//! tracked follow-up.

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
        Err(api_err(StatusCode::FORBIDDEN, "forbidden_non_loopback".into()))
    }
}
