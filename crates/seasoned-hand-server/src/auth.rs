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
    use serde::Serialize;

    #[derive(Debug, Clone, Copy)]
    pub struct RouteAction(pub Action);

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

    fn parse_auth_context(headers: &HeaderMap) -> Result<AuthContext, StatusCode> {
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
}
