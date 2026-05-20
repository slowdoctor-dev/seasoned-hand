//! Central RBAC policy evaluator for Phase 5.
//!
//! refs: /specs/phase-5/architecture.md §4.3

use thiserror::Error;

use crate::auth::context::{Action, AuthContext, Role};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthResource {
    pub is_same_org: bool,
    pub actor_can_share: bool,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AuthError {
    #[error("missing tenant context")]
    MissingTenantContext,
    #[error("unauthorized action: role={role:?} action={action:?} reason={reason}")]
    Unauthorized {
        role: Role,
        action: Action,
        reason: &'static str,
    },
}

pub fn authorize(
    action: Action,
    resource: &AuthResource,
    context: &AuthContext,
) -> Result<(), AuthError> {
    if context.tenant_id.trim().is_empty() {
        return Err(AuthError::MissingTenantContext);
    }

    let role = context.effective_role();
    match (role, action) {
        (Role::Admin, _) => Ok(()),
        (Role::User, Action::TaskRead) => Ok(()),
        (Role::User, Action::TaskWrite) => Ok(()),
        (Role::User, Action::TaskHandoff) => {
            if resource.is_same_org {
                Ok(())
            } else {
                Err(AuthError::Unauthorized {
                    role,
                    action,
                    reason: "handoff requires same organization",
                })
            }
        }
        (Role::User, Action::SopShare | Action::PlaybookShare) => {
            if resource.actor_can_share {
                Ok(())
            } else {
                Err(AuthError::Unauthorized {
                    role,
                    action,
                    reason: "sharing requires owner/editor capability",
                })
            }
        }
        (Role::User, Action::AuditRead) => Ok(()),
        (Role::User, Action::MembershipManage | Action::EventRawRead) => {
            Err(AuthError::Unauthorized {
                role,
                action,
                reason: "action is admin-only",
            })
        }
        (Role::Viewer, Action::TaskRead) => Ok(()),
        (
            Role::Viewer,
            Action::TaskWrite
            | Action::TaskHandoff
            | Action::SopShare
            | Action::PlaybookShare
            | Action::MembershipManage
            | Action::AuditRead
            | Action::EventRawRead,
        ) => Err(AuthError::Unauthorized {
            role,
            action,
            reason: "viewer has read-only scope",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{AuthError, AuthResource, authorize};
    use crate::auth::context::{Action, AuthContext, Role, effective_role};

    fn context(role: Role) -> AuthContext {
        AuthContext {
            tenant_id: "tenant-a".to_string(),
            organization_id: "org-a".to_string(),
            actor_user_id: "user-a".to_string(),
            org_role: role,
            project_override_role: None,
        }
    }

    #[test]
    fn effective_role_prefers_override_over_org_role() {
        assert_eq!(effective_role(Role::Viewer, Some(Role::Admin)), Role::Admin);
        assert_eq!(effective_role(Role::User, None), Role::User);
    }

    #[test]
    fn matrix_admin_user_viewer_by_action() {
        let actions = [
            Action::TaskRead,
            Action::TaskWrite,
            Action::TaskHandoff,
            Action::SopShare,
            Action::PlaybookShare,
            Action::MembershipManage,
            Action::AuditRead,
            Action::EventRawRead,
        ];

        let admin = context(Role::Admin);
        for action in actions {
            let result = authorize(
                action,
                &AuthResource {
                    is_same_org: true,
                    actor_can_share: true,
                },
                &admin,
            );
            assert!(result.is_ok(), "admin should allow {action:?}");
        }

        let user = context(Role::User);
        let user_expected_allow = [
            Action::TaskRead,
            Action::TaskWrite,
            Action::TaskHandoff,
            Action::SopShare,
            Action::PlaybookShare,
            Action::AuditRead,
        ];
        for action in actions {
            let result = authorize(
                action,
                &AuthResource {
                    is_same_org: true,
                    actor_can_share: true,
                },
                &user,
            );
            let should_allow = user_expected_allow.contains(&action);
            assert_eq!(
                result.is_ok(),
                should_allow,
                "user matrix mismatch for {action:?}"
            );
        }

        let viewer = context(Role::Viewer);
        for action in actions {
            let result = authorize(
                action,
                &AuthResource {
                    is_same_org: true,
                    actor_can_share: true,
                },
                &viewer,
            );
            let should_allow = matches!(action, Action::TaskRead);
            assert_eq!(
                result.is_ok(),
                should_allow,
                "viewer matrix mismatch for {action:?}"
            );
        }
    }

    #[test]
    fn user_handoff_requires_same_org() {
        let user = context(Role::User);
        let err = authorize(
            Action::TaskHandoff,
            &AuthResource {
                is_same_org: false,
                actor_can_share: true,
            },
            &user,
        )
        .expect_err("cross-org handoff should deny");
        assert!(matches!(err, AuthError::Unauthorized { .. }));
    }

    #[test]
    fn user_share_requires_owner_or_editor_capability() {
        let user = context(Role::User);
        let err = authorize(
            Action::PlaybookShare,
            &AuthResource {
                is_same_org: true,
                actor_can_share: false,
            },
            &user,
        )
        .expect_err("sharing without owner/editor should deny");
        assert!(matches!(err, AuthError::Unauthorized { .. }));
    }

    #[test]
    fn missing_tenant_context_fails_closed() {
        let mut ctx = context(Role::Admin);
        ctx.tenant_id = "   ".to_string();
        let err = authorize(Action::TaskRead, &AuthResource::default(), &ctx)
            .expect_err("blank tenant context should deny");
        assert_eq!(err, AuthError::MissingTenantContext);
    }

    #[test]
    fn project_override_role_is_used_for_policy_decision() {
        let mut ctx = context(Role::Viewer);
        ctx.project_override_role = Some(Role::Admin);
        let result = authorize(
            Action::MembershipManage,
            &AuthResource {
                is_same_org: true,
                actor_can_share: true,
            },
            &ctx,
        );
        assert!(result.is_ok(), "override admin role should allow action");
    }
}
