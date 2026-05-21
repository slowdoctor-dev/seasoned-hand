//! Story 5.20 regression tests for [`UserDeactivationService`].
//! refs: /specs/phase-5/stories/story-5.20.md

use super::*;
use crate::audit::AuditLogger;
use crate::auth::{AuthContext, Role};
use crate::db::{self, DbPool};
use crate::events::sqlite::SqliteEventStore;
use crate::handoff::TaskHandoffService;
use rusqlite::params;

const TENANT: &str = "tenant-test";

async fn setup() -> (DbPool, UserDeactivationService) {
    let pool = db::open(":memory:").await.unwrap();
    let events = std::sync::Arc::new(SqliteEventStore::new(pool.clone()));
    let audit = AuditLogger::new(pool.clone(), events.clone());
    let handoff = TaskHandoffService::new(pool.clone(), events, audit.clone());
    let svc = UserDeactivationService::new(pool.clone(), audit, handoff);

    pool.with_conn(|conn| {
        conn.execute(
            "INSERT INTO organizations (id, tenant_id, slug, display_name, status,
                                         created_at, updated_at)
             VALUES ('org-t', 'tenant-test', 'org-t', 'T', 'active', 0, 0)",
            [],
        )?;
        for (uid, email) in [
            ("user-admin", "admin@x.io"),
            ("user-src", "src@x.io"),
            ("user-tgt", "tgt@x.io"),
            ("user-viewer", "viewer@x.io"),
        ] {
            conn.execute(
                "INSERT INTO users (id, tenant_id, email, display_name, status,
                                    created_at, updated_at)
                 VALUES (?, 'tenant-test', ?, 'X', 'active', 0, 0)",
                params![uid, email],
            )?;
            conn.execute(
                "INSERT INTO organization_memberships (id, tenant_id, organization_id,
                                                       user_id, role, is_primary,
                                                       created_at, updated_at)
                 VALUES (?, 'tenant-test', 'org-t', ?, 'user', 1, 0, 0)",
                params![format!("mem-{uid}"), uid],
            )?;
        }
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();

    (pool, svc)
}

fn ctx_admin() -> AuthContext {
    AuthContext {
        tenant_id: TENANT.into(),
        organization_id: "org-t".into(),
        actor_user_id: "user-admin".into(),
        org_role: Role::Admin,
        project_override_role: None,
    }
}

/// Seed one project + one task owned by `owner_id` in `Drafted` status.
async fn seed_task(pool: &DbPool, task_id: &str, owner_id: &str) {
    let task_id = task_id.to_string();
    let owner_id = owner_id.to_string();
    pool.with_conn(move |conn| {
        // Project insert is idempotent so multiple tasks can share it.
        conn.execute(
            "INSERT OR IGNORE INTO projects (id, tenant_id, title, status,
                                              created_at, updated_at)
             VALUES ('p-1', 'tenant-test', 'P', 'active', 0, 0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO tasks (id, project_id, tenant_id, owner_user_id, title,
                                status, created_at, updated_at)
             VALUES (?, 'p-1', 'tenant-test', ?, 'T', 'drafted', 0, 0)",
            params![task_id, owner_id],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn deactivate_reassigns_tasks_and_flips_status() {
    let (pool, svc) = setup().await;
    seed_task(&pool, "task-1", "user-src").await;
    seed_task(&pool, "task-2", "user-src").await;
    let outcome = svc
        .deactivate(&ctx_admin(), "src@x.io", "tgt@x.io", Some("rotating off"))
        .await
        .expect("deactivate");
    assert_eq!(outcome.tasks_reassigned, 2);
    assert_eq!(outcome.source_user_id, "user-src");
    assert_eq!(outcome.target_user_id, "user-tgt");

    // Source flipped to deactivated.
    let status: String = pool
        .with_conn(|conn| {
            conn.query_row("SELECT status FROM users WHERE id = 'user-src'", [], |r| {
                r.get(0)
            })
        })
        .await
        .unwrap();
    assert_eq!(status, "deactivated");

    // Every active task now owned by user-tgt.
    let target_count: i64 = pool
        .with_conn(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM tasks WHERE owner_user_id = 'user-tgt'",
                [],
                |r| r.get(0),
            )
        })
        .await
        .unwrap();
    assert_eq!(target_count, 2);

    // Audit row recorded.
    let audit_n: i64 = pool
        .with_conn(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM audit_log
                 WHERE action = 'user.deactivate' AND resource_id = 'user-src'",
                [],
                |r| r.get(0),
            )
        })
        .await
        .unwrap();
    assert_eq!(audit_n, 1);
}

#[tokio::test]
async fn deactivate_with_zero_active_tasks_still_succeeds() {
    // Edge case: user owns nothing. Deactivation just flips status +
    // audits — no tasks to reassign.
    let (pool, svc) = setup().await;
    let outcome = svc
        .deactivate(&ctx_admin(), "src@x.io", "tgt@x.io", None)
        .await
        .expect("deactivate");
    assert_eq!(outcome.tasks_reassigned, 0);
    let status: String = pool
        .with_conn(|conn| {
            conn.query_row("SELECT status FROM users WHERE id = 'user-src'", [], |r| {
                r.get(0)
            })
        })
        .await
        .unwrap();
    assert_eq!(status, "deactivated");
}

#[tokio::test]
async fn deactivate_rejects_same_user() {
    let (_pool, svc) = setup().await;
    let err = svc
        .deactivate(&ctx_admin(), "src@x.io", "src@x.io", None)
        .await
        .expect_err("must reject self-target");
    assert!(matches!(err, DeactivationError::SameUser));
}

#[tokio::test]
async fn deactivate_rejects_already_deactivated_source() {
    let (pool, svc) = setup().await;
    pool.with_conn(|conn| {
        conn.execute(
            "UPDATE users SET status = 'deactivated' WHERE id = 'user-src'",
            [],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();
    let err = svc
        .deactivate(&ctx_admin(), "src@x.io", "tgt@x.io", None)
        .await
        .expect_err("must reject already-deactivated");
    assert!(matches!(err, DeactivationError::AlreadyDeactivated));
}

#[tokio::test]
async fn deactivate_rejects_cross_org_target() {
    // Seed a separate org + target user in a DIFFERENT tenant; the
    // tenant boundary check fires before the org-id mismatch path, so
    // the target shows up as "not found" in tenant-test.
    let (pool, svc) = setup().await;
    pool.with_conn(|conn| {
        conn.execute(
            "INSERT INTO organizations (id, tenant_id, slug, display_name, status,
                                         created_at, updated_at)
             VALUES ('org-other', 'tenant-other', 'org-other', 'O', 'active', 0, 0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO users (id, tenant_id, email, display_name, status,
                                created_at, updated_at)
             VALUES ('user-other', 'tenant-other', 'other@x.io', 'O', 'active', 0, 0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO organization_memberships (id, tenant_id, organization_id,
                                                   user_id, role, is_primary,
                                                   created_at, updated_at)
             VALUES ('mem-other', 'tenant-other', 'org-other', 'user-other',
                     'admin', 1, 0, 0)",
            [],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();
    let err = svc
        .deactivate(&ctx_admin(), "src@x.io", "other@x.io", None)
        .await
        .expect_err("must reject cross-tenant target");
    // The lookup is tenant-scoped, so the cross-tenant target shows up
    // as not found rather than CrossOrgTarget — either is a hard fail.
    assert!(matches!(err, DeactivationError::TargetNotFound(_)));
}

#[tokio::test]
async fn deactivate_denies_viewer_role() {
    let (_pool, svc) = setup().await;
    let viewer = AuthContext {
        tenant_id: TENANT.into(),
        organization_id: "org-t".into(),
        actor_user_id: "user-viewer".into(),
        org_role: Role::Viewer,
        project_override_role: None,
    };
    let err = svc
        .deactivate(&viewer, "src@x.io", "tgt@x.io", None)
        .await
        .expect_err("viewer must be denied");
    assert!(matches!(err, DeactivationError::Auth(_)));
}

// --- Hardening P5-HARD-IT1-M1: last-admin lockout guard ---

/// Promote a seeded user's membership role (helper for the lockout tests;
/// the default setup seeds everyone as 'user').
async fn set_role(pool: &DbPool, user_id: &str, role: &str) {
    let user_id = user_id.to_string();
    let role = role.to_string();
    pool.with_conn(move |conn| {
        conn.execute(
            "UPDATE organization_memberships SET role = ? WHERE user_id = ?",
            params![role, user_id],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn deactivate_last_admin_is_rejected() {
    // user-src is the ONLY admin in org-t. Deactivating it would leave
    // the org with zero admins → must be refused (LastAdminLockout).
    let (pool, svc) = setup().await;
    set_role(&pool, "user-src", "admin").await;
    let err = svc
        .deactivate(&ctx_admin(), "src@x.io", "tgt@x.io", None)
        .await
        .expect_err("deactivating the sole admin must be rejected");
    assert!(
        matches!(err, DeactivationError::LastAdminLockout { .. }),
        "expected LastAdminLockout, got {err:?}"
    );
    // Source must remain active — the guard fails BEFORE any state change.
    let status: String = pool
        .with_conn(|conn| {
            conn.query_row("SELECT status FROM users WHERE id = 'user-src'", [], |r| {
                r.get(0)
            })
        })
        .await
        .unwrap();
    assert_eq!(status, "active");
}

#[tokio::test]
async fn deactivate_non_last_admin_succeeds() {
    // Two admins (user-src + user-tgt). Deactivating user-src is fine —
    // user-tgt remains as an active admin.
    let (pool, svc) = setup().await;
    set_role(&pool, "user-src", "admin").await;
    set_role(&pool, "user-tgt", "admin").await;
    let outcome = svc
        .deactivate(&ctx_admin(), "src@x.io", "tgt@x.io", None)
        .await
        .expect("deactivating a non-last admin must succeed");
    assert_eq!(outcome.source_user_id, "user-src");
    let status: String = pool
        .with_conn(|conn| {
            conn.query_row("SELECT status FROM users WHERE id = 'user-src'", [], |r| {
                r.get(0)
            })
        })
        .await
        .unwrap();
    assert_eq!(status, "deactivated");
}

#[tokio::test]
async fn deactivate_non_admin_source_is_unaffected_by_lockout_guard() {
    // The guard only fires for admin sources. A plain 'user' source
    // deactivates normally even if it happens to be the only 'user'.
    let (pool, svc) = setup().await;
    // user-admin is promoted so the org always has an admin; user-src
    // stays a 'user'.
    set_role(&pool, "user-admin", "admin").await;
    let outcome = svc
        .deactivate(&ctx_admin(), "src@x.io", "tgt@x.io", None)
        .await
        .expect("deactivating a non-admin user must succeed");
    assert_eq!(outcome.source_user_id, "user-src");
}
