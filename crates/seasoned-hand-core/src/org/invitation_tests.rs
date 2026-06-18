use std::sync::Arc;

use super::invitation::InvitationService;
use crate::audit::AuditLogger;
use crate::auth::{AuthContext, Role};
use crate::db;
use crate::events::sqlite::SqliteEventStore;

fn admin_ctx(tenant: &str, org: &str) -> AuthContext {
    AuthContext {
        tenant_id: tenant.to_string(),
        organization_id: org.to_string(),
        actor_user_id: "u-admin".to_string(),
        org_role: Role::Admin,
        project_override_role: None,
    }
}

async fn seed(pool: &crate::db::DbPool) {
    pool.with_conn(|conn| {
        conn.execute(
            "INSERT INTO organizations (id, tenant_id, slug, display_name, status, created_at, updated_at)
             VALUES ('org-a', 'tenant-a', 'acme', 'Acme', 'active', 0, 0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO organizations (id, tenant_id, slug, display_name, status, created_at, updated_at)
             VALUES ('org-b', 'tenant-b', 'beta', 'Beta', 'active', 0, 0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO users (id, tenant_id, email, display_name, status, created_at, updated_at)
             VALUES ('u-admin', 'tenant-a', 'admin@acme.com', 'Admin', 'active', 0, 0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO organization_memberships (id, tenant_id, organization_id, user_id, role, is_primary, created_at, updated_at)
             VALUES ('m-admin', 'tenant-a', 'org-a', 'u-admin', 'admin', 1, 0, 0)",
            [],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .expect("seed");
}

#[tokio::test]
async fn invite_happy_path_emits_audit_and_token() {
    let pool = db::open(":memory:").await.unwrap();
    seed(&pool).await;
    let events = Arc::new(SqliteEventStore::new(pool.clone()));
    let audit = AuditLogger::new(pool.clone(), events);
    let service = InvitationService::new(pool.clone(), audit);

    let out = service
        .invite_user(
            &admin_ctx("tenant-a", "org-a"),
            "acme",
            "new@acme.com",
            "viewer",
        )
        .await
        .expect("invite");
    assert!(out.user_id.starts_with("user-"));
    assert!(!out.login_token.is_empty());

    pool.with_conn(|conn| {
        let token_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM user_invitation_tokens WHERE user_id = ?1 AND consumed_at IS NULL",
                [out.user_id.clone()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(token_rows, 1);
        let audit_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_log WHERE action = 'user.invite' AND target_user_id = ?1",
                [out.user_id.clone()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(audit_rows, 1);
    })
    .await;
}

#[tokio::test]
async fn invite_rolls_back_on_fk_failure() {
    let pool = db::open(":memory:").await.unwrap();
    seed(&pool).await;
    let events = Arc::new(SqliteEventStore::new(pool.clone()));
    let audit = AuditLogger::new(pool.clone(), events);
    let service = InvitationService::new(pool.clone(), audit);

    pool.with_conn(|conn| {
        conn.execute_batch(
            "CREATE TRIGGER users_delete_org_a_after_insert
             AFTER INSERT ON users
             WHEN NEW.email = 'boom@acme.com'
             BEGIN
               DELETE FROM organizations WHERE id='org-a';
             END;",
        )
        .unwrap();
    })
    .await;

    let err = service
        .invite_user(
            &admin_ctx("tenant-a", "org-a"),
            "acme",
            "boom@acme.com",
            "user",
        )
        .await
        .expect_err("must fail");
    assert!(err.to_string().contains("sqlite"));

    pool.with_conn(|conn| {
        let users: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM users WHERE email='boom@acme.com'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(users, 0);
    })
    .await;
}

#[tokio::test]
async fn cross_tenant_invite_denied() {
    let pool = db::open(":memory:").await.unwrap();
    seed(&pool).await;
    let events = Arc::new(SqliteEventStore::new(pool.clone()));
    let audit = AuditLogger::new(pool.clone(), events);
    let service = InvitationService::new(pool, audit);

    let err = service
        .invite_user(
            &admin_ctx("tenant-a", "org-a"),
            "beta",
            "x@beta.com",
            "user",
        )
        .await
        .expect_err("cross tenant denied");
    assert!(err.to_string().contains("cross-tenant"));
}

// NOTE: invitation login-token verify / consume / expiry is now exercised
// against the live path (`auth::AuthSessionStore::login`) in
// `auth/session.rs` tests — including the issue #6 org-binding + TTL coverage.
// The standalone `InvitationService::verify_and_consume_login_token` helper
// these tests used was removed (it was never wired to a route and resolved no
// org); see the note in `invitation.rs`.
