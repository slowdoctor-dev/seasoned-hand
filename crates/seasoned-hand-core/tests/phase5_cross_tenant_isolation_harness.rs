//! Phase 5 story 5.26 — `phase5_cross_tenant_isolation_harness`.
//!
//! Headline NFR-5.1 acceptance harness. For every load-bearing service
//! surface, this test seeds two tenants (`tenant-a`, `tenant-b`) with
//! independent organizations, users, and data, then attempts a
//! forged-tenant call from tenant-A against tenant-B's data and
//! asserts:
//!
//! 1. **No cross-tenant read** — tenant-A's caller never observes
//!    tenant-B's rows.
//! 2. **No cross-tenant write** — tenant-A's caller cannot mutate
//!    tenant-B's rows.
//! 3. **Missing-context fail-closed** — calls without a resolvable
//!    tenant are denied at the policy gate.
//!
//! The harness is intentionally one large test (not a 10-way split)
//! because the cross-tenant property is global: the cheapest way to
//! catch a regression is to exercise every surface against the same
//! seeded fixture in one run.
//!
//! Worker-spawn surfaces (verifier / curator / retention / ttl /
//! notify / intake) are covered transitively: every worker uses
//! `SystemAuth::for_worker(org, tenant, kind)` to construct its
//! `AuthContext`, and every service the worker calls is exercised
//! below with that AuthContext shape. The tenant_id field is the
//! single source of truth; the worker's kind/actor is irrelevant to
//! the tenant-scoping predicates.
//!
//! refs: /specs/phase-5/stories/story-5.26.md
//! refs: /specs/phase-5/architecture.md §15 (harness 1)
//! refs: /specs/phase-5/requirements.md NFR-5.1

use rusqlite::params;
use seasoned_hand_core::audit::{AuditLogger, AuditQuery, AuditRecord};
use seasoned_hand_core::auth::{
    Action, AuthContext, AuthError, AuthResource, Role, SystemAuth, authorize,
};
use seasoned_hand_core::db::{self, DbPool};
use seasoned_hand_core::events::sqlite::SqliteEventStore;
use seasoned_hand_core::events::visibility::{self, EventReadQuery};
use seasoned_hand_core::events::{EventStore, EventType, NewEvent};
use seasoned_hand_core::handoff::{HandoffError, HandoffRequest, TaskHandoffService};
use seasoned_hand_core::org::UserDeactivationService;
use std::sync::Arc;

/// Seed two tenants with independent org / user / project / task /
/// SOP / playbook rows. Both tenants are 'active'; their data must
/// stay isolated from each other under every service call below.
async fn seed_two_tenants(pool: &DbPool) {
    pool.with_conn(|conn| {
        // Two organizations, one per tenant. organizations.tenant_id
        // is UNIQUE per V013 schema so 1 org = 1 tenant.
        for (org_id, tenant, slug) in [
            ("org-a", "tenant-a", "org-a"),
            ("org-b", "tenant-b", "org-b"),
        ] {
            conn.execute(
                "INSERT INTO organizations (id, tenant_id, slug, display_name, status,
                                             created_at, updated_at)
                 VALUES (?, ?, ?, 'O', 'active', 0, 0)",
                params![org_id, tenant, slug],
            )?;
        }
        // Two admins, one user per tenant + a viewer.
        for (uid, tenant, email) in [
            ("user-a-admin", "tenant-a", "a-admin@x.io"),
            ("user-a-user", "tenant-a", "a-user@x.io"),
            ("user-a-viewer", "tenant-a", "a-viewer@x.io"),
            ("user-a-target", "tenant-a", "a-target@x.io"),
            ("user-b-admin", "tenant-b", "b-admin@x.io"),
            ("user-b-user", "tenant-b", "b-user@x.io"),
        ] {
            conn.execute(
                "INSERT INTO users (id, tenant_id, email, display_name, status,
                                    created_at, updated_at)
                 VALUES (?, ?, ?, 'X', 'active', 0, 0)",
                params![uid, tenant, email],
            )?;
            let org_id = if tenant == "tenant-a" {
                "org-a"
            } else {
                "org-b"
            };
            conn.execute(
                "INSERT INTO organization_memberships
                   (id, tenant_id, organization_id, user_id, role,
                    is_primary, created_at, updated_at)
                 VALUES (?, ?, ?, ?, 'admin', 1, 0, 0)",
                params![format!("mem-{uid}"), tenant, org_id, uid],
            )?;
        }
        // One project + task per tenant.
        for (pid, tid, tenant) in [
            ("proj-a", "task-a", "tenant-a"),
            ("proj-b", "task-b", "tenant-b"),
        ] {
            conn.execute(
                "INSERT INTO projects (id, tenant_id, title, status, created_at, updated_at)
                 VALUES (?, ?, 'P', 'active', 0, 0)",
                params![pid, tenant],
            )?;
            conn.execute(
                "INSERT INTO tasks (id, project_id, tenant_id, owner_user_id, title,
                                    status, created_at, updated_at)
                 VALUES (?, ?, ?, NULL, 'T', 'drafted', 0, 0)",
                params![tid, pid, tenant],
            )?;
        }
        // One session per tenant linked to that tenant's task.
        for (sid, tid) in [("sess-a", "task-a"), ("sess-b", "task-b")] {
            conn.execute(
                "INSERT INTO sessions (id, created_at, updated_at, state, task_id)
                 VALUES (?, 0, 0, 'IDLE', ?)",
                params![sid, tid],
            )?;
        }
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();
}

fn auth(tenant: &str, org: &str, actor: &str, role: Role) -> AuthContext {
    AuthContext {
        tenant_id: tenant.into(),
        organization_id: org.into(),
        actor_user_id: actor.into(),
        org_role: role,
        project_override_role: None,
    }
}

#[tokio::test]
async fn phase5_cross_tenant_isolation_harness() {
    let pool = db::open(":memory:").await.expect("open db");
    seed_two_tenants(&pool).await;
    let events_store = Arc::new(SqliteEventStore::new(pool.clone()));
    let audit = AuditLogger::new(pool.clone(), events_store.clone());

    let auth_a_admin = auth("tenant-a", "org-a", "user-a-admin", Role::Admin);
    let auth_b_admin = auth("tenant-b", "org-b", "user-b-admin", Role::Admin);

    // ---------- 1. Event visibility query is tenant-scoped ----------
    // Append events to both tenants' sessions; the visibility::query
    // surface must show each tenant only its own rows.
    for sid in ["sess-a", "sess-b"] {
        events_store
            .append(NewEvent {
                session_id: sid.into(),
                event_type: EventType::Message,
                source: "user".into(),
                data: serde_json::json!({"text": format!("hello from {sid}")}),
            })
            .await
            .expect("append event");
    }
    let a_rows = visibility::query(&pool, &auth_a_admin, "sess-a", EventReadQuery::default())
        .await
        .expect("query a");
    assert_eq!(a_rows.len(), 1);
    assert_eq!(a_rows[0].session_id, "sess-a");

    // tenant-A admin querying tenant-B's session → 0 rows
    // (predicate-as-gate: no error, just empty result).
    let cross = visibility::query(&pool, &auth_a_admin, "sess-b", EventReadQuery::default())
        .await
        .expect("query cross");
    assert!(
        cross.is_empty(),
        "tenant-A admin must not see tenant-B's session events"
    );

    // ---------- 2. Raw-event read enforces tenant boundary even for admin ----------
    // Admin in tenant-A has `Action::EventRawRead` per the matrix, but
    // the session's tenant must match. Cross-tenant raw read → 0 rows
    // (the resolved session tenant != caller tenant short-circuits).
    let raw_cross = visibility::query_raw(
        &pool,
        &auth_a_admin,
        &audit,
        "sess-b",
        EventReadQuery::default(),
    )
    .await
    .expect("query_raw cross");
    assert!(
        raw_cross.is_empty(),
        "tenant-A admin raw-read must not leak tenant-B rows"
    );

    // Same call against own-tenant session yields rows.
    let raw_own = visibility::query_raw(
        &pool,
        &auth_a_admin,
        &audit,
        "sess-a",
        EventReadQuery::default(),
    )
    .await
    .expect("query_raw own");
    assert!(!raw_own.is_empty(), "own-tenant raw read must succeed");

    // ---------- 3. AuditLogger::query is tenant-scoped ----------
    // Each tenant writes one audit row; admin-A must see only A's,
    // admin-B only B's.
    audit
        .record(
            &auth_a_admin,
            AuditRecord {
                action: seasoned_hand_core::audit::AuditAction::TaskHandoff,
                resource_type: "task",
                resource_id: "task-a",
                target_user_id: None,
                decision: Some("allow"),
                reason: None,
                metadata: serde_json::json!({}),
            },
        )
        .await
        .expect("audit a");
    audit
        .record(
            &auth_b_admin,
            AuditRecord {
                action: seasoned_hand_core::audit::AuditAction::TaskHandoff,
                resource_type: "task",
                resource_id: "task-b",
                target_user_id: None,
                decision: Some("allow"),
                reason: None,
                metadata: serde_json::json!({}),
            },
        )
        .await
        .expect("audit b");
    let a_audit = audit
        .query(&auth_a_admin, AuditQuery::default())
        .await
        .expect("audit query a");
    let b_audit = audit
        .query(&auth_b_admin, AuditQuery::default())
        .await
        .expect("audit query b");
    assert!(
        a_audit.iter().all(|r| r.tenant_id == "tenant-a"),
        "admin-A audit query must only return tenant-A rows; saw: {a_audit:?}"
    );
    assert!(
        b_audit.iter().all(|r| r.tenant_id == "tenant-b"),
        "admin-B audit query must only return tenant-B rows; saw: {b_audit:?}"
    );

    // ---------- 4. Handoff service enforces tenant boundary on target lookup ----------
    let handoff_service =
        TaskHandoffService::new(pool.clone(), events_store.clone(), audit.clone());
    // Admin-A tries to hand off tenant-A's task to a user that exists
    // only in tenant-B → UserNotFound (tenant-scoped lookup returns
    // None even though the email exists in another tenant).
    let cross_handoff = handoff_service
        .handoff(
            &auth_a_admin,
            HandoffRequest {
                task_id: "task-a".into(),
                to_user_email: "b-user@x.io".into(),
                reason: None,
                expected_updated_at: None,
            },
        )
        .await;
    assert!(
        matches!(cross_handoff, Err(HandoffError::UserNotFound(_))),
        "cross-tenant handoff target must surface as UserNotFound; got {cross_handoff:?}"
    );

    // Cross-tenant WRITE attempt with a real foreign task-id (`task-b`):
    // tenant-A admin must not mutate tenant-B task ownership.
    let cross_task_handoff = handoff_service
        .handoff(
            &auth_a_admin,
            HandoffRequest {
                task_id: "task-b".into(),
                to_user_email: "a-user@x.io".into(),
                reason: Some("forged foreign task".into()),
                expected_updated_at: None,
            },
        )
        .await;
    assert!(
        matches!(cross_task_handoff, Err(HandoffError::TaskNotFound(_))),
        "cross-tenant handoff using foreign task-id must fail as TaskNotFound; got {cross_task_handoff:?}"
    );
    let owner_b: Option<String> = pool
        .with_conn(|conn| {
            conn.query_row(
                "SELECT owner_user_id FROM tasks WHERE id='task-b'",
                [],
                |r| r.get(0),
            )
        })
        .await
        .expect("task-b remains queryable");
    assert!(
        owner_b.is_none(),
        "cross-tenant forged handoff must not mutate tenant-B owner_user_id"
    );

    // ---------- 5. User deactivation rejects cross-tenant target ----------
    // NOTE: the "cross-org same-tenant" path is structurally unreachable in
    // this schema because `organizations.tenant_id` is UNIQUE (V013): one
    // tenant maps to exactly one org. So we test the meaningful boundary here
    // (cross-tenant target) and leave cross-org as defensive-but-unreachable.
    let deactivation =
        UserDeactivationService::new(pool.clone(), audit.clone(), handoff_service.clone());
    let cross_deact = deactivation
        .deactivate(&auth_a_admin, "a-user@x.io", "b-user@x.io", None)
        .await;
    assert!(
        cross_deact.is_err(),
        "cross-tenant deactivation target must be rejected"
    );

    // ---------- 6. Missing-context fail-closed at policy gate ----------
    // An AuthContext with an empty tenant_id must fail `authorize` for
    // any mutating action — the gate refuses to even check the role.
    let no_tenant = AuthContext {
        tenant_id: "".into(),
        organization_id: "org-a".into(),
        actor_user_id: "user-a-admin".into(),
        org_role: Role::Admin,
        project_override_role: None,
    };
    let err = authorize(
        Action::TaskWrite,
        &AuthResource {
            is_same_org: true,
            actor_can_share: true,
        },
        &no_tenant,
    )
    .expect_err("missing tenant must fail-closed");
    assert!(
        matches!(err, AuthError::MissingTenantContext),
        "fail-closed must surface as MissingTenantContext, got {err:?}"
    );

    // ---------- 7. Worker SystemAuth identities are tenant-pinned ----------
    // Every worker spawn (curator / retention / ttl / verifier / notify
    // / intake / user-cost / user-cost-reconcile) goes through
    // SystemAuth::for_worker. Verify the produced context carries the
    // exact tenant + org we pass in — no fallback to a sentinel that
    // could blur tenants.
    let curator_auth =
        SystemAuth::for_worker("org-a".to_string(), "tenant-a".to_string(), "curator");
    assert_eq!(curator_auth.tenant_id, "tenant-a");
    assert_eq!(curator_auth.organization_id, "org-a");
    assert_eq!(curator_auth.org_role, Role::Admin);
    let retention_auth =
        SystemAuth::for_worker("org-b".to_string(), "tenant-b".to_string(), "retention");
    assert_eq!(retention_auth.tenant_id, "tenant-b");
    assert_eq!(retention_auth.organization_id, "org-b");
    // Curator-A and Retention-B contexts must NOT share tenant — if
    // SystemAuth ever defaulted to the sentinel on missing input, this
    // would catch it.
    assert_ne!(curator_auth.tenant_id, retention_auth.tenant_id);

    // ---------- 8. CLI / HTTP surface (no separate test — auth middleware sits
    //     above visibility::query / audit::query / handoff::handoff which
    //     are all exercised above). The middleware parses headers into the
    //     same AuthContext shape this harness uses; if the
    //     AuthContext-shaped surfaces are tenant-tight, the middleware
    //     surfaces are tenant-tight by construction.
}
