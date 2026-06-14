//! Phase 5 HTTP auth middleware.
//!
//! refs: /specs/phase-5/architecture.md §4.2, §4.3
//! refs: /specs/phase-5/stories/story-5.5.md

pub mod middleware {
    use axum::Extension;
    use axum::Json;
    use axum::extract::Request;
    use axum::http::{HeaderMap, StatusCode};
    use axum::middleware::Next;
    use axum::response::Response;
    use seasoned_hand_core::auth::{
        Action, AuthContext, AuthError, AuthSessionStore, Role, authorize_coarse,
    };
    use serde::Serialize;
    use std::sync::Arc;

    /// Fixed WebSocket subprotocol the browser offers alongside the bearer token
    /// (ADR-017). The client opens `new WebSocket(url, [WS_AUTH_SUBPROTOCOL, token])`;
    /// the server echoes this sentinel to complete the handshake and reads the
    /// token from the *other* offered subprotocol. Browser `WebSocket` cannot set
    /// request headers, so this is the only header-free way to carry a credential
    /// on the `/ws` upgrade.
    pub const WS_AUTH_SUBPROTOCOL: &str = "seasoned-hand-auth.v1";

    #[derive(Debug, Clone, Copy)]
    pub struct RouteAction(pub Action);

    /// DB-backed dependencies the auth middleware needs to verify session tokens
    /// (issue #7 / ADR-018) and to decide whether the insecure header path is
    /// permitted. Injected as a request extension by `app()` so the per-route
    /// `from_fn` middleware can reach them without threading state through every
    /// `with_auth` call site.
    #[derive(Clone)]
    pub struct AuthDeps {
        pub sessions: Arc<AuthSessionStore>,
        /// True only when `SH_INSECURE_AUTH_HEADERS` is set (loopback dev / tests
        /// / CLI). When false, `x-seasoned-hand-*` headers are rejected and only a
        /// verified session token authenticates.
        pub allow_insecure_headers: bool,
    }

    #[derive(Debug, Serialize)]
    pub(crate) struct ApiError {
        error: String,
    }

    pub async fn require_auth_context(
        Extension(deps): Extension<AuthDeps>,
        req: Request,
        next: Next,
    ) -> Result<Response, (StatusCode, Json<ApiError>)> {
        // Issue #21: this middleware only runs on routes wrapped by `with_auth`,
        // which always attach a `RouteAction`. A missing action therefore means a
        // misconfigured route — fail CLOSED (deny) rather than letting the request
        // through unauthenticated.
        let action = req.extensions().get::<RouteAction>().copied();
        let Some(RouteAction(action)) = action else {
            return Err((
                StatusCode::FORBIDDEN,
                Json(ApiError {
                    error: "auth_route_unclassified".to_string(),
                }),
            ));
        };

        let remote = req
            .extensions()
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|ci| ci.0);
        let context = resolve_identity(&deps, req.headers(), remote)
            .await
            .map_err(|code| {
                (
                    code,
                    Json(ApiError {
                        error: "unauthorized_context".to_string(),
                    }),
                )
            })?;

        // Issue #8: coarse, resource-INDEPENDENT RBAC. The forgeable
        // `x-seasoned-hand-resource-*` headers are gone; per-resource
        // authorization (handoff tenant-scoping, share `actor_can_share` from the
        // DB) is enforced downstream by the service layer.
        authorize_coarse(action, &context).map_err(|err| {
            let status = match err {
                AuthError::MissingTenantContext => StatusCode::UNAUTHORIZED,
                AuthError::Unauthorized { .. } => StatusCode::FORBIDDEN,
            };
            (
                status,
                Json(ApiError {
                    error: "forbidden_action".to_string(),
                }),
            )
        })?;

        let mut req = req;
        req.extensions_mut().insert(context);
        Ok(next.run(req).await)
    }

    /// Resolve the caller's identity (issue #7 / ADR-018). Priority:
    /// 1. A **verified** session token (ADR-018) — `Authorization: Bearer` (REST)
    ///    or the non-sentinel `Sec-WebSocket-Protocol` entry (browser `/ws`),
    ///    looked up against `auth_sessions`. Checked first so a stray
    ///    proxy-forwarded header can't override a real credential, and a
    ///    presented-but-invalid token is rejected rather than demoted to headers.
    /// 2. Legacy `x-seasoned-hand-*` headers — **only** when `allow_insecure_headers`
    ///    is set AND the caller is loopback (dev / tests / CLI). Off by default →
    ///    client-asserted identity is not trusted, and even when enabled it cannot
    ///    be forged from a non-loopback caller.
    async fn resolve_identity(
        deps: &AuthDeps,
        headers: &HeaderMap,
        remote: Option<std::net::SocketAddr>,
    ) -> Result<AuthContext, StatusCode> {
        if let Some(token) = bearer_token(headers).or_else(|| subprotocol_token(headers)) {
            return deps
                .sessions
                .verify(&token)
                .await
                .ok_or(StatusCode::UNAUTHORIZED);
        }
        // The insecure header path requires BOTH the flag and a loopback caller, so
        // a flagged server is not forgeable from a non-loopback address even on a
        // `with_auth` route that lacks its own handler-level loopback guard.
        let from_loopback = remote.map(|r| r.ip().is_loopback()).unwrap_or(false);
        if deps.allow_insecure_headers
            && from_loopback
            && headers.contains_key("x-seasoned-hand-tenant-id")
        {
            return parse_header_context(headers);
        }
        Err(StatusCode::UNAUTHORIZED)
    }

    fn parse_header_context(headers: &HeaderMap) -> Result<AuthContext, StatusCode> {
        Ok(AuthContext {
            tenant_id: header_str(headers, "x-seasoned-hand-tenant-id")?,
            organization_id: header_str(headers, "x-seasoned-hand-organization-id")?,
            actor_user_id: header_str(headers, "x-seasoned-hand-actor-user-id")?,
            org_role: parse_role(&header_str(headers, "x-seasoned-hand-org-role")?)?,
            project_override_role: match optional_header_str(
                headers,
                "x-seasoned-hand-project-override-role",
            ) {
                Some(role) => Some(parse_role(&role)?),
                None => None,
            },
        })
    }

    /// Extract a bearer token from `Authorization: Bearer <token>` (REST).
    fn bearer_token(headers: &HeaderMap) -> Option<String> {
        let value = headers.get("authorization")?.to_str().ok()?;
        let token = value
            .strip_prefix("Bearer ")
            .or_else(|| value.strip_prefix("bearer "))?
            .trim();
        (!token.is_empty()).then(|| token.to_string())
    }

    /// Extract the token offered alongside the sentinel in `Sec-WebSocket-Protocol`
    /// (browser `/ws`). The client offers `[WS_AUTH_SUBPROTOCOL, token]`; the token
    /// is whichever offered protocol is not the sentinel.
    fn subprotocol_token(headers: &HeaderMap) -> Option<String> {
        let value = headers.get("sec-websocket-protocol")?.to_str().ok()?;
        value
            .split(',')
            .map(str::trim)
            .find(|p| !p.is_empty() && *p != WS_AUTH_SUBPROTOCOL)
            .map(str::to_string)
    }

    fn parse_role(value: &str) -> Result<Role, StatusCode> {
        match value {
            "admin" => Ok(Role::Admin),
            "user" => Ok(Role::User),
            "viewer" => Ok(Role::Viewer),
            _ => Err(StatusCode::UNAUTHORIZED),
        }
    }

    fn header_str(headers: &HeaderMap, name: &str) -> Result<String, StatusCode> {
        optional_header_str(headers, name)
            .filter(|v| !v.trim().is_empty())
            .ok_or(StatusCode::UNAUTHORIZED)
    }

    fn optional_header_str(headers: &HeaderMap, name: &str) -> Option<String> {
        headers
            .get(name)
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn bearer_token_extracts_value() {
            let mut h = HeaderMap::new();
            h.insert("authorization", "Bearer abc123".parse().unwrap());
            assert_eq!(bearer_token(&h).as_deref(), Some("abc123"));

            // Case-insensitive scheme, trimmed.
            let mut h2 = HeaderMap::new();
            h2.insert("authorization", "bearer  xyz ".parse().unwrap());
            assert_eq!(bearer_token(&h2).as_deref(), Some("xyz"));

            // Empty token and missing header yield None.
            let mut h3 = HeaderMap::new();
            h3.insert("authorization", "Bearer ".parse().unwrap());
            assert_eq!(bearer_token(&h3), None);
            assert_eq!(bearer_token(&HeaderMap::new()), None);
        }

        #[test]
        fn subprotocol_token_picks_non_sentinel() {
            let mut h = HeaderMap::new();
            h.insert(
                "sec-websocket-protocol",
                format!("{WS_AUTH_SUBPROTOCOL}, tok-42").parse().unwrap(),
            );
            assert_eq!(subprotocol_token(&h).as_deref(), Some("tok-42"));

            // Sentinel alone carries no token.
            let mut only = HeaderMap::new();
            only.insert(
                "sec-websocket-protocol",
                WS_AUTH_SUBPROTOCOL.parse().unwrap(),
            );
            assert_eq!(subprotocol_token(&only), None);
            assert_eq!(subprotocol_token(&HeaderMap::new()), None);
        }

        #[test]
        fn header_context_reads_all_fields() {
            let mut h = HeaderMap::new();
            h.insert("x-seasoned-hand-tenant-id", "acme".parse().unwrap());
            h.insert("x-seasoned-hand-organization-id", "org-1".parse().unwrap());
            h.insert("x-seasoned-hand-actor-user-id", "u-1".parse().unwrap());
            h.insert("x-seasoned-hand-org-role", "admin".parse().unwrap());
            let ctx = parse_header_context(&h).expect("header context");
            assert_eq!(ctx.tenant_id, "acme");
            assert_eq!(ctx.actor_user_id, "u-1");
            assert_eq!(ctx.org_role, Role::Admin);
        }

        #[test]
        fn header_context_rejects_blank_fields() {
            let mut h = HeaderMap::new();
            h.insert("x-seasoned-hand-tenant-id", "acme".parse().unwrap());
            h.insert("x-seasoned-hand-organization-id", "".parse().unwrap());
            h.insert("x-seasoned-hand-actor-user-id", "u-1".parse().unwrap());
            h.insert("x-seasoned-hand-org-role", "admin".parse().unwrap());
            assert_eq!(
                parse_header_context(&h).unwrap_err(),
                StatusCode::UNAUTHORIZED
            );
        }
    }
}
