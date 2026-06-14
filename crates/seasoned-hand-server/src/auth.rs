//! Phase 5 HTTP auth middleware.
//!
//! refs: /specs/phase-5/architecture.md §4.2, §4.3
//! refs: /specs/phase-5/stories/story-5.5.md

pub mod middleware {
    use axum::Json;
    use axum::extract::Request;
    use axum::http::{HeaderMap, StatusCode};
    use axum::middleware::Next;
    use axum::response::Response;
    use seasoned_hand_core::auth::{Action, AuthContext, AuthError, AuthResource, Role, authorize};
    use serde::{Deserialize, Serialize};

    /// Fixed WebSocket subprotocol the browser offers alongside the bearer token
    /// (ADR-017). The client opens `new WebSocket(url, [WS_AUTH_SUBPROTOCOL, token])`;
    /// the server echoes this sentinel to complete the handshake and reads the
    /// token from the *other* offered subprotocol. Browser `WebSocket` cannot set
    /// request headers, so this is the only header-free way to carry a credential
    /// on the `/ws` upgrade.
    pub const WS_AUTH_SUBPROTOCOL: &str = "seasoned-hand-auth.v1";

    #[derive(Debug, Clone, Copy)]
    pub struct RouteAction(pub Action);

    /// Identity asserted by a browser bearer/subprotocol token (ADR-017). The
    /// token is the lowercase-hex encoding of this struct's JSON. NOTE: this is an
    /// *unsigned* assertion — transport only, no more trusted than the legacy
    /// `x-seasoned-hand-*` headers. Verifying it (signing/expiry against
    /// `user_invitation_tokens`) is issue #7 and replaces only `decode_identity_token`.
    #[derive(Deserialize)]
    struct IdentityToken {
        tenant_id: String,
        organization_id: String,
        actor_user_id: String,
        org_role: String,
        #[serde(default)]
        project_override_role: Option<String>,
    }

    #[derive(Debug, Serialize)]
    pub(crate) struct ApiError {
        error: String,
    }

    pub async fn require_auth_context(
        req: Request,
        next: Next,
    ) -> Result<Response, (StatusCode, Json<ApiError>)> {
        let action = req.extensions().get::<RouteAction>().copied();
        let Some(RouteAction(action)) = action else {
            return Ok(next.run(req).await);
        };

        let context = parse_auth_context(req.headers()).map_err(|code| {
            (
                code,
                Json(ApiError {
                    error: "unauthorized_context".to_string(),
                }),
            )
        })?;

        let resource = AuthResource {
            is_same_org: parse_bool_header(req.headers(), "x-seasoned-hand-resource-same-org")
                .unwrap_or(true),
            actor_can_share: parse_bool_header(req.headers(), "x-seasoned-hand-resource-can-share")
                .unwrap_or(true),
        };

        authorize(action, &resource, &context).map_err(|err| {
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
        req.extensions_mut().insert(resource);
        Ok(next.run(req).await)
    }

    /// Resolve the caller's identity. Priority (ADR-017):
    /// 1. `x-seasoned-hand-*` headers — service-to-service callers and tests
    ///    (active iff the tenant header is present).
    /// 2. `Authorization: Bearer <token>` — browser REST (`fetch`/`gloo-net`).
    /// 3. The non-sentinel `Sec-WebSocket-Protocol` entry — browser `/ws` upgrade.
    ///
    /// The header path is unchanged, so existing callers/tests keep working; the
    /// token paths are purely additive.
    fn parse_auth_context(headers: &HeaderMap) -> Result<AuthContext, StatusCode> {
        if headers.contains_key("x-seasoned-hand-tenant-id") {
            return parse_header_context(headers);
        }
        if let Some(token) = bearer_token(headers).or_else(|| subprotocol_token(headers)) {
            return decode_identity_token(&token);
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

    /// Decode a lowercase-hex `IdentityToken` JSON into an `AuthContext`.
    /// (Transport only — see `IdentityToken`; verification is issue #7.)
    fn decode_identity_token(token: &str) -> Result<AuthContext, StatusCode> {
        let bytes = hex_decode(token).ok_or(StatusCode::UNAUTHORIZED)?;
        let parsed: IdentityToken =
            serde_json::from_slice(&bytes).map_err(|_| StatusCode::UNAUTHORIZED)?;
        if parsed.tenant_id.trim().is_empty() {
            return Err(StatusCode::UNAUTHORIZED);
        }
        Ok(AuthContext {
            tenant_id: parsed.tenant_id,
            organization_id: parsed.organization_id,
            actor_user_id: parsed.actor_user_id,
            org_role: parse_role(&parsed.org_role)?,
            project_override_role: match parsed.project_override_role {
                Some(role) => Some(parse_role(&role)?),
                None => None,
            },
        })
    }

    /// Decode a lowercase-hex string to bytes. Dependency-free (avoids adding a
    /// `hex`/`base64` crate, which would require an ARCHITECTURE.md update).
    fn hex_decode(s: &str) -> Option<Vec<u8>> {
        if s.is_empty() || !s.len().is_multiple_of(2) {
            return None;
        }
        let bytes = s.as_bytes();
        let mut out = Vec::with_capacity(s.len() / 2);
        let mut i = 0;
        while i < bytes.len() {
            let hi = (bytes[i] as char).to_digit(16)?;
            let lo = (bytes[i + 1] as char).to_digit(16)?;
            out.push((hi * 16 + lo) as u8);
            i += 2;
        }
        Some(out)
    }

    fn parse_role(value: &str) -> Result<Role, StatusCode> {
        match value {
            "admin" => Ok(Role::Admin),
            "user" => Ok(Role::User),
            "viewer" => Ok(Role::Viewer),
            _ => Err(StatusCode::UNAUTHORIZED),
        }
    }

    fn parse_bool_header(headers: &HeaderMap, name: &str) -> Option<bool> {
        optional_header_str(headers, name).and_then(|v| match v.as_str() {
            "1" | "true" | "yes" => Some(true),
            "0" | "false" | "no" => Some(false),
            _ => None,
        })
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

        fn hex_encode(bytes: &[u8]) -> String {
            bytes.iter().map(|b| format!("{b:02x}")).collect()
        }

        fn dev_token(role: &str) -> String {
            let json = format!(
                r#"{{"tenant_id":"acme","organization_id":"org-1","actor_user_id":"u-1","org_role":"{role}"}}"#
            );
            hex_encode(json.as_bytes())
        }

        #[test]
        fn header_path_still_works() {
            let mut h = HeaderMap::new();
            h.insert("x-seasoned-hand-tenant-id", "acme".parse().unwrap());
            h.insert("x-seasoned-hand-organization-id", "org-1".parse().unwrap());
            h.insert("x-seasoned-hand-actor-user-id", "u-1".parse().unwrap());
            h.insert("x-seasoned-hand-org-role", "admin".parse().unwrap());
            let ctx = parse_auth_context(&h).expect("header context");
            assert_eq!(ctx.tenant_id, "acme");
            assert_eq!(ctx.org_role, Role::Admin);
        }

        #[test]
        fn bearer_token_authenticates() {
            let mut h = HeaderMap::new();
            h.insert(
                "authorization",
                format!("Bearer {}", dev_token("user")).parse().unwrap(),
            );
            let ctx = parse_auth_context(&h).expect("bearer context");
            assert_eq!(ctx.tenant_id, "acme");
            assert_eq!(ctx.actor_user_id, "u-1");
            assert_eq!(ctx.org_role, Role::User);
        }

        #[test]
        fn subprotocol_token_authenticates() {
            let mut h = HeaderMap::new();
            // Browser offers the sentinel + the token; order should not matter.
            h.insert(
                "sec-websocket-protocol",
                format!("{WS_AUTH_SUBPROTOCOL}, {}", dev_token("viewer"))
                    .parse()
                    .unwrap(),
            );
            let ctx = parse_auth_context(&h).expect("subprotocol context");
            assert_eq!(ctx.tenant_id, "acme");
            assert_eq!(ctx.org_role, Role::Viewer);
        }

        #[test]
        fn no_credential_is_unauthorized() {
            assert_eq!(
                parse_auth_context(&HeaderMap::new()).unwrap_err(),
                StatusCode::UNAUTHORIZED
            );
        }

        #[test]
        fn malformed_token_is_unauthorized() {
            let mut h = HeaderMap::new();
            h.insert("authorization", "Bearer zzzz".parse().unwrap());
            assert_eq!(
                parse_auth_context(&h).unwrap_err(),
                StatusCode::UNAUTHORIZED
            );
        }

        #[test]
        fn sentinel_only_subprotocol_is_unauthorized() {
            let mut h = HeaderMap::new();
            h.insert(
                "sec-websocket-protocol",
                WS_AUTH_SUBPROTOCOL.parse().unwrap(),
            );
            assert_eq!(
                parse_auth_context(&h).unwrap_err(),
                StatusCode::UNAUTHORIZED
            );
        }
    }
}
