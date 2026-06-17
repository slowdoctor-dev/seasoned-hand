//! Story 5.10 regression tests.
//! refs: /specs/phase-5/stories/story-5.10.md

use super::*;
use crate::auth::{AuthContext, Role};
use crate::db::{self, DbPool};
use crate::events::sqlite::SqliteEventStore;
use crate::events::{EventQuery, EventStore};
use rusqlite::params;

const TENANT: &str = "tenant-test";

async fn setup() -> (DbPool, AuditLogger) {
    let pool = db::open(":memory:").await.unwrap();
    let events = std::sync::Arc::new(SqliteEventStore::new(pool.clone()));
    // Seed minimal org + actor + target users so FKs resolve.
    pool.with_conn(|conn| {
        conn.execute(
            "INSERT INTO organizations (id, tenant_id, slug, display_name, status, created_at, updated_at)
             VALUES ('org-test', 'tenant-test', 'org-test', 'T', 'active', 0, 0)",
            [],
        )?;
        for (uid, email) in [
            ("user-admin", "admin@example.com"),
            ("user-user", "user@example.com"),
            ("user-viewer", "viewer@example.com"),
            ("user-target", "target@example.com"),
        ] {
            conn.execute(
                "INSERT INTO users (id, tenant_id, email, display_name, status, created_at, updated_at)
                 VALUES (?, 'tenant-test', ?, 'X', 'active', 0, 0)",
                params![uid, email],
            )?;
        }
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();
    let logger = AuditLogger::new(pool.clone(), events);
    (pool, logger)
}

fn ctx(role: Role, actor: &str) -> AuthContext {
    AuthContext {
        tenant_id: TENANT.into(),
        organization_id: "org-test".into(),
        actor_user_id: actor.into(),
        org_role: role,
        project_override_role: None,
    }
}

fn rec<'a>(action: AuditAction, resource_id: &'a str) -> AuditRecord<'a> {
    AuditRecord {
        action,
        resource_type: "task",
        resource_id,
        target_user_id: Some("user-target"),
        decision: Some("allow"),
        reason: Some("test"),
        metadata: serde_json::json!({}),
    }
}

#[tokio::test]
async fn record_inserts_audit_row_and_emits_dual_write_event() {
    let (pool, logger) = setup().await;
    let id = logger
        .record(
            &ctx(Role::Admin, "user-admin"),
            rec(AuditAction::TaskHandoff, "t-1"),
        )
        .await
        .expect("record");

    // 1) audit_log row exists.
    let id_for_move = id.clone();
    let row: (String, String, String) = pool
        .with_conn(move |conn| {
            conn.query_row(
                "SELECT action, resource_type, resource_id FROM audit_log WHERE id = ?",
                params![id_for_move],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
        })
        .await
        .expect("read row");
    assert_eq!(row.0, "task.handoff");
    assert_eq!(row.1, "task");
    assert_eq!(row.2, "t-1");

    // 2) Dual-write Misc event lands on `audit:<tenant>` session.
    let events_store = SqliteEventStore::new(pool.clone());
    let evs = events_store
        .query("audit:tenant-test", EventQuery::default())
        .await
        .expect("query events");
    let saw_audit = evs.iter().any(|e| {
        e.data
            .get("kind")
            .and_then(|v| v.as_str())
            .is_some_and(|k| k == "audit_logged")
            && e.data.get("audit_log_id").and_then(|v| v.as_str()) == Some(id.as_str())
    });
    assert!(saw_audit, "dual-write audit_logged event must be emitted");
}

#[tokio::test]
async fn record_hash_chain_links_new_rows_and_verifies_cleanly() {
    let (_pool, logger) = setup().await;
    let first = logger
        .record(
            &ctx(Role::Admin, "user-admin"),
            rec(AuditAction::TaskHandoff, "t-1"),
        )
        .await
        .expect("first record");
    let second = logger
        .record(
            &ctx(Role::Admin, "user-admin"),
            rec(AuditAction::SopShare, "sop-1"),
        )
        .await
        .expect("second record");

    logger.verify_chain().await.expect("clean chain verifies");

    let first_chain = logger
        .chain_row(&first)
        .await
        .expect("first chain query")
        .expect("first chain row");
    let second_chain = logger
        .chain_row(&second)
        .await
        .expect("second chain query")
        .expect("second chain row");
    assert_eq!(
        first_chain.prev_hash.as_deref(),
        Some("0000000000000000000000000000000000000000000000000000000000000000")
    );
    assert_eq!(second_chain.prev_hash, first_chain.row_hash);
    assert!(
        second_chain.row_hash.is_some(),
        "new audit rows must carry row_hash"
    );
}

#[tokio::test]
async fn audit_log_rejects_update_and_delete() {
    let (pool, logger) = setup().await;
    let id = logger
        .record(
            &ctx(Role::Admin, "user-admin"),
            rec(AuditAction::TaskHandoff, "t-1"),
        )
        .await
        .expect("record");

    let update_id = id.clone();
    let update_err = pool
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE audit_log SET reason = 'tampered' WHERE id = ?",
                params![update_id],
            )
        })
        .await
        .expect_err("audit update must be rejected");
    assert!(update_err.to_string().contains("append-only"));

    let delete_id = id.clone();
    let delete_err = pool
        .with_conn(move |conn| {
            conn.execute("DELETE FROM audit_log WHERE id = ?", params![delete_id])
        })
        .await
        .expect_err("audit delete must be rejected");
    assert!(delete_err.to_string().contains("append-only"));
}

#[tokio::test]
async fn verify_chain_detects_bad_inserted_hash_link() {
    let (pool, logger) = setup().await;
    logger
        .record(
            &ctx(Role::Admin, "user-admin"),
            rec(AuditAction::TaskHandoff, "t-1"),
        )
        .await
        .expect("record");

    pool.with_conn(|conn| {
        conn.execute(
            "INSERT INTO audit_log (
               id, tenant_id, organization_id, actor_user_id, action,
               resource_type, resource_id, target_user_id, decision, reason,
               metadata, created_at, prev_hash, row_hash
             ) VALUES (
               'audit-bad-link', 'tenant-test', 'org-test', 'user-admin',
               'task.cancel', 'task', 't-bad', NULL, NULL, NULL,
               '{}', 999, 'bad-prev', 'bad-row'
             )",
            [],
        )
    })
    .await
    .expect("manual insert is allowed; verification should detect tamper");

    let err = logger
        .verify_chain()
        .await
        .expect_err("bad link must fail chain verification");
    assert!(matches!(
        err,
        AuditChainError::PrevHashMismatch { id, .. } if id == "audit-bad-link"
    ));
}

#[tokio::test]
async fn query_admin_sees_all_org_rows() {
    let (_pool, logger) = setup().await;
    logger
        .record(
            &ctx(Role::Admin, "user-admin"),
            rec(AuditAction::TaskHandoff, "t-1"),
        )
        .await
        .unwrap();
    logger
        .record(
            &ctx(Role::User, "user-user"),
            rec(AuditAction::SopShare, "sop-1"),
        )
        .await
        .unwrap();
    let rows = logger
        .query(&ctx(Role::Admin, "user-admin"), AuditQuery::default())
        .await
        .expect("admin query");
    assert_eq!(rows.len(), 2);
}

#[tokio::test]
async fn query_user_sees_only_own_actions() {
    let (_pool, logger) = setup().await;
    logger
        .record(
            &ctx(Role::Admin, "user-admin"),
            rec(AuditAction::TaskHandoff, "t-1"),
        )
        .await
        .unwrap();
    logger
        .record(
            &ctx(Role::User, "user-user"),
            rec(AuditAction::SopShare, "sop-1"),
        )
        .await
        .unwrap();
    let rows = logger
        .query(&ctx(Role::User, "user-user"), AuditQuery::default())
        .await
        .expect("user query");
    // User filter is applied even if the caller passed a different
    // actor_user_id; only their own rows surface.
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].actor_user_id, "user-user");
}

#[tokio::test]
async fn query_viewer_is_denied() {
    let (_pool, logger) = setup().await;
    let err = logger
        .query(&ctx(Role::Viewer, "user-viewer"), AuditQuery::default())
        .await
        .expect_err("viewer must be denied");
    assert!(matches!(err, AuditQueryError::Auth(_)));
}

#[tokio::test]
async fn query_applies_action_filter() {
    let (_pool, logger) = setup().await;
    logger
        .record(
            &ctx(Role::Admin, "user-admin"),
            rec(AuditAction::TaskHandoff, "t-1"),
        )
        .await
        .unwrap();
    logger
        .record(
            &ctx(Role::Admin, "user-admin"),
            rec(AuditAction::SopShare, "sop-1"),
        )
        .await
        .unwrap();
    let rows = logger
        .query(
            &ctx(Role::Admin, "user-admin"),
            AuditQuery {
                action: Some(AuditAction::TaskHandoff),
                ..Default::default()
            },
        )
        .await
        .expect("filtered");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].action, "task.handoff");
}
