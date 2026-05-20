//! Phase 5 story 5.27 — `phase5_rbac_matrix_harness`.
//!
//! Verifies every cell of the architecture §4.3 RBAC matrix (8 actions
//! × 3 roles = 24 cells), the project-role override precedence
//! (`AuthContext::effective_role`), and the NFR-5.2 auth-check latency
//! budget (p95 ≤ 10ms, p99 ≤ 25ms on baseline runner).
//!
//! The matrix per `crate::auth::policy::authorize`:
//!
//! | Action            | Admin | User                                    | Viewer |
//! |-------------------|-------|-----------------------------------------|--------|
//! | TaskRead          | ✓     | ✓                                       | ✓      |
//! | TaskWrite         | ✓     | ✓                                       | ✗      |
//! | TaskHandoff       | ✓     | ✓ if same-org                           | ✗      |
//! | SopShare          | ✓     | ✓ if actor_can_share                    | ✗      |
//! | PlaybookShare     | ✓     | ✓ if actor_can_share                    | ✗      |
//! | MembershipManage  | ✓     | ✗ (admin-only)                          | ✗      |
//! | AuditRead         | ✓     | ✓ (User row sees only own actions)      | ✗      |
//! | EventRawRead      | ✓     | ✗ (admin-only)                          | ✗      |
//!
//! refs: /specs/phase-5/stories/story-5.27.md
//! refs: /specs/phase-5/architecture.md §4.3 (RBAC matrix), §15 harness 2
//! refs: /specs/phase-5/requirements.md NFR-5.2, F-5.5

use seasoned_hand_core::auth::context::effective_role;
use seasoned_hand_core::auth::{Action, AuthContext, AuthError, AuthResource, Role, authorize};
use std::time::Instant;

fn ctx(role: Role) -> AuthContext {
    AuthContext {
        tenant_id: "tenant-a".into(),
        organization_id: "org-a".into(),
        actor_user_id: "user-a".into(),
        org_role: role,
        project_override_role: None,
    }
}

/// All 8 actions in declaration order — must match `crate::auth::Action`.
const ALL_ACTIONS: [Action; 8] = [
    Action::TaskRead,
    Action::TaskWrite,
    Action::TaskHandoff,
    Action::SopShare,
    Action::PlaybookShare,
    Action::MembershipManage,
    Action::AuditRead,
    Action::EventRawRead,
];

/// Expected decision per (role, action). `true` = allow.
fn expected_allow(role: Role, action: Action) -> bool {
    match (role, action) {
        (Role::Admin, _) => true,
        (Role::User, Action::MembershipManage) => false,
        (Role::User, Action::EventRawRead) => false,
        (Role::User, _) => true,
        (Role::Viewer, Action::TaskRead) => true,
        (Role::Viewer, _) => false,
    }
}

#[test]
fn phase5_rbac_matrix_harness() {
    // 24 cells: 3 roles × 8 actions.
    let resource = AuthResource {
        is_same_org: true,
        actor_can_share: true,
    };
    for role in [Role::Admin, Role::User, Role::Viewer] {
        for action in ALL_ACTIONS {
            let auth = ctx(role);
            let outcome = authorize(action, &resource, &auth);
            let want = expected_allow(role, action);
            match (outcome, want) {
                (Ok(()), true) | (Err(AuthError::Unauthorized { .. }), false) => {
                    // matches matrix expectation
                }
                (Ok(()), false) => panic!(
                    "matrix cell ({role:?}, {action:?}): policy returned Ok but matrix says deny"
                ),
                (Err(err), true) => panic!(
                    "matrix cell ({role:?}, {action:?}): policy returned {err:?} but matrix says allow"
                ),
                (Err(other), false) => panic!(
                    "matrix cell ({role:?}, {action:?}): policy returned wrong error {other:?}"
                ),
            }
        }
    }

    // Same-org conditional: User + TaskHandoff with is_same_org=false → deny.
    let cross_org = AuthResource {
        is_same_org: false,
        actor_can_share: true,
    };
    let err = authorize(Action::TaskHandoff, &cross_org, &ctx(Role::User))
        .expect_err("User cross-org handoff must deny");
    assert!(matches!(err, AuthError::Unauthorized { .. }));

    // Actor-can-share conditional: User + SopShare with actor_can_share=false → deny.
    let no_share = AuthResource {
        is_same_org: true,
        actor_can_share: false,
    };
    let err = authorize(Action::SopShare, &no_share, &ctx(Role::User))
        .expect_err("User without share capability must deny");
    assert!(matches!(err, AuthError::Unauthorized { .. }));

    // Missing-context fail-closed (empty tenant_id).
    let missing = AuthContext {
        tenant_id: "".into(),
        organization_id: "org-a".into(),
        actor_user_id: "user-a".into(),
        org_role: Role::Admin,
        project_override_role: None,
    };
    let err = authorize(Action::TaskWrite, &resource, &missing)
        .expect_err("missing-tenant must fail-closed");
    assert!(matches!(err, AuthError::MissingTenantContext));
}

#[test]
fn phase5_rbac_project_override_precedence() {
    // Per arch §4.2: `project_override_role.unwrap_or(org_role)`.
    // A user with org-Viewer + project-User override gets User
    // permissions on that project (write/handoff/etc).
    let viewer_with_user_override = AuthContext {
        tenant_id: "tenant-a".into(),
        organization_id: "org-a".into(),
        actor_user_id: "user-a".into(),
        org_role: Role::Viewer,
        project_override_role: Some(Role::User),
    };
    // The `authorize` policy ALWAYS reads `auth.org_role` — overrides
    // are resolved upstream via `effective_role()`. So the harness
    // resolves the role first, then constructs the AuthContext used
    // by the policy gate. This mirrors the HTTP middleware path:
    // it computes effective_role from headers, then sets that into
    // org_role on the context it hands to handlers.
    assert_eq!(
        effective_role(Role::Viewer, Some(Role::User)),
        Role::User,
        "project override must take precedence over org role"
    );
    // And the unwrap chain on the context itself:
    assert_eq!(viewer_with_user_override.effective_role(), Role::User);

    // A user with org-Admin + project-Viewer override DOWNGRADES to
    // Viewer on that project — useful for "read-only sandbox" use
    // cases where an admin wants to limit their own write privilege.
    assert_eq!(
        effective_role(Role::Admin, Some(Role::Viewer)),
        Role::Viewer
    );

    // Unset override → org_role.
    assert_eq!(effective_role(Role::User, None), Role::User);
}

#[test]
fn phase5_rbac_latency_p95_p99_under_budget() {
    // NFR-5.2: p95 ≤ 10ms, p99 ≤ 25ms on baseline runner.
    // We sample 10,000 calls across the matrix to get reliable
    // percentile estimates. The policy gate is a small match
    // expression; expected p99 is well under a microsecond. The
    // budget here is generous — its real purpose is to catch
    // regressions that introduce hidden DB lookups or async waits
    // into what should be a pure-Rust decision.
    const SAMPLES: usize = 10_000;
    let resource = AuthResource {
        is_same_org: true,
        actor_can_share: true,
    };
    let auth = ctx(Role::User);
    let mut latencies_ns = Vec::with_capacity(SAMPLES);
    for i in 0..SAMPLES {
        // Cycle through all 8 actions to exercise every arm of the
        // policy match expression.
        let action = ALL_ACTIONS[i % ALL_ACTIONS.len()];
        let start = Instant::now();
        let _ = authorize(action, &resource, &auth);
        latencies_ns.push(start.elapsed().as_nanos() as u64);
    }
    latencies_ns.sort_unstable();
    let p95 = latencies_ns[(SAMPLES * 95 / 100).saturating_sub(1)];
    let p99 = latencies_ns[(SAMPLES * 99 / 100).saturating_sub(1)];
    let p95_ms = p95 as f64 / 1_000_000.0;
    let p99_ms = p99 as f64 / 1_000_000.0;
    eprintln!(
        "phase5_rbac_latency: p95={:.4}ms p99={:.4}ms (budget 10ms / 25ms)",
        p95_ms, p99_ms
    );
    assert!(
        p95_ms <= 10.0,
        "p95 auth-check latency {p95_ms:.4}ms exceeds NFR-5.2 budget 10ms"
    );
    assert!(
        p99_ms <= 25.0,
        "p99 auth-check latency {p99_ms:.4}ms exceeds NFR-5.2 budget 25ms"
    );
}
