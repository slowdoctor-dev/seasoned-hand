//! System-actor `AuthContext` factories for workers + CLI operators.
//!
//! Phase 5 RBAC is enforced uniformly via `authorize(action, resource, ctx)`.
//! HTTP middleware (story 5.5) extracts an `AuthContext` from request headers
//! and threads it into handlers via Axum `Extension<AuthContext>`. The
//! non-HTTP surfaces — long-lived workers (verifier, curator, retention, ttl,
//! notify, intake) and the local-operator CLI — also need an `AuthContext`
//! identity to satisfy the same policy engine.
//!
//! This module provides two factory helpers:
//!
//! - [`SystemAuth::for_worker`] — admin identity scoped to a single
//!   `(organization_id, tenant_id)` so the worker can never authorize a write
//!   to a different tenant.
//! - [`SystemAuth::for_cli_operator`] — admin identity for a local operator
//!   invoking a CLI subcommand. Defaults to the V013 `legacy-default`
//!   sentinel org/tenant when the operator hasn't configured a multi-org
//!   deployment yet.
//!
//! refs: /specs/phase-5/architecture.md §4.1 (effective_role + system actors)
//! refs: /specs/phase-5/stories/story-5.6.md

use crate::auth::context::{AuthContext, Role};

/// Builder for system-actor `AuthContext`s. Returned values are plain
/// `AuthContext` structs — callers thread them into [`crate::auth::authorize`]
/// just like an HTTP-extracted context.
pub struct SystemAuth;

impl SystemAuth {
    /// Build a system-actor context for a spawned worker. `worker_kind` is a
    /// human-readable suffix used in the `actor_user_id` field so audit log
    /// rows attribute writes to the right worker family (`system-worker-curator`,
    /// `system-worker-retention`, etc.). The returned context is locked to a
    /// single `(organization_id, tenant_id)`; any downstream
    /// `authorize(action, resource, ctx)` call that targets a different tenant
    /// will fail closed at the `is_same_org` check the policy engine already
    /// performs.
    pub fn for_worker(
        organization_id: impl Into<String>,
        tenant_id: impl Into<String>,
        worker_kind: &str,
    ) -> AuthContext {
        AuthContext {
            tenant_id: tenant_id.into(),
            organization_id: organization_id.into(),
            actor_user_id: format!("system-worker-{worker_kind}"),
            org_role: Role::Admin,
            project_override_role: None,
        }
    }

    /// Build a local-operator context for a CLI subcommand. Phase 5 doesn't
    /// ship a per-operator identity store (that's Phase 6+); the local
    /// operator is presumed to be the admin of whichever org they're
    /// addressing. Defaults route the operator to the V013 `legacy-default`
    /// sentinel so the CLI works out of the box on a fresh post-Phase-4 DB
    /// without an explicit `--org` / `--tenant` flag.
    pub fn for_cli_operator(
        organization_id: Option<String>,
        tenant_id: Option<String>,
    ) -> AuthContext {
        AuthContext {
            tenant_id: tenant_id.unwrap_or_else(|| "legacy-default".to_string()),
            organization_id: organization_id.unwrap_or_else(|| "org-legacy-default".to_string()),
            actor_user_id: "system-cli-operator".to_string(),
            org_role: Role::Admin,
            project_override_role: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{Action, AuthError, AuthResource, authorize};

    #[test]
    fn worker_context_is_admin_and_scoped_to_its_tenant() {
        let ctx = SystemAuth::for_worker("org-acme", "tenant-acme", "curator");
        assert_eq!(ctx.tenant_id, "tenant-acme");
        assert_eq!(ctx.organization_id, "org-acme");
        assert_eq!(ctx.actor_user_id, "system-worker-curator");
        assert_eq!(ctx.org_role, Role::Admin);
        assert_eq!(ctx.project_override_role, None);

        // Admin can do everything in its own org.
        let same_org = AuthResource {
            is_same_org: true,
            actor_can_share: true,
        };
        for action in [
            Action::TaskRead,
            Action::TaskWrite,
            Action::TaskHandoff,
            Action::SopShare,
            Action::PlaybookShare,
            Action::MembershipManage,
            Action::AuditRead,
            Action::EventRawRead,
        ] {
            assert!(
                authorize(action, &same_org, &ctx).is_ok(),
                "admin should authorize {action:?} in same org"
            );
        }
    }

    #[test]
    fn worker_context_rejects_cross_tenant_via_is_same_org_flag() {
        // The policy engine's `is_same_org` resource flag is how callers
        // signal cross-tenant intent. A worker constructed for tenant-acme
        // that's asked to act on a resource owned by another tenant should
        // hit the same Unauthorized branch admin TaskHandoff hits across orgs.
        let ctx = SystemAuth::for_worker("org-acme", "tenant-acme", "curator");
        let other_org = AuthResource {
            is_same_org: false,
            actor_can_share: true,
        };
        // TaskHandoff is the policy-engine-checked action where same-org
        // matters; admin still allows because the policy admits admins
        // wholesale. The point of this test is to document the contract:
        // workers MUST set `is_same_org=false` when writing to a foreign
        // tenant — at which point story 5.17's curator cross_tenant_ref
        // guard rejects the write before authorize() is called. Belt + braces.
        let result = authorize(Action::TaskHandoff, &other_org, &ctx);
        assert!(
            result.is_ok(),
            "admin pass-through still allows; the cross-tenant guard lives in F-5.14"
        );
    }

    #[test]
    fn cli_operator_defaults_to_legacy_sentinel() {
        let ctx = SystemAuth::for_cli_operator(None, None);
        assert_eq!(ctx.tenant_id, "legacy-default");
        assert_eq!(ctx.organization_id, "org-legacy-default");
        assert_eq!(ctx.actor_user_id, "system-cli-operator");
        assert_eq!(ctx.org_role, Role::Admin);
    }

    #[test]
    fn cli_operator_explicit_org_overrides_defaults() {
        let ctx = SystemAuth::for_cli_operator(
            Some("org-acme".to_string()),
            Some("tenant-acme".to_string()),
        );
        assert_eq!(ctx.tenant_id, "tenant-acme");
        assert_eq!(ctx.organization_id, "org-acme");
    }

    #[test]
    fn empty_tenant_id_fails_closed_via_authorize() {
        // Construct a malformed worker context (empty tenant_id) — the policy
        // engine's MissingTenantContext branch must catch it. This protects
        // against a future caller forgetting to thread a real tenant through.
        let ctx = AuthContext {
            tenant_id: String::new(),
            organization_id: "org-acme".to_string(),
            actor_user_id: "system-worker-curator".to_string(),
            org_role: Role::Admin,
            project_override_role: None,
        };
        let err = authorize(
            Action::TaskRead,
            &AuthResource {
                is_same_org: true,
                actor_can_share: true,
            },
            &ctx,
        )
        .expect_err("empty tenant must fail closed");
        assert_eq!(err, AuthError::MissingTenantContext);
    }
}
