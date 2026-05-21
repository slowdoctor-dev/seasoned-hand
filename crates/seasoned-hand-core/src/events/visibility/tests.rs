//! Story 5.14 regression tests for [`crate::events::visibility`].
//! refs: /specs/phase-5/stories/story-5.14.md

use super::*;
use crate::audit::AuditLogger;
use crate::auth::{AuthContext, Role};
use crate::db::{self, DbPool};
use crate::events::sqlite::SqliteEventStore;
use crate::events::{EventStore, EventType, NewEvent};
use rusqlite::params;

async fn setup() -> DbPool {
    let pool = db::open(":memory:").await.unwrap();
    // Seed a tenant + session so the projection has a real tenant
    // (not the sentinel) for the happy-path tests.
    pool.with_conn(|conn| {
        conn.execute(
            "INSERT INTO projects (id, tenant_id, title, status, created_at, updated_at)
             VALUES ('p-test', 'tenant-test', 'P', 'active', 0, 0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO tasks (id, project_id, tenant_id, title, status,
                                created_at, updated_at)
             VALUES ('t-test', 'p-test', 'tenant-test', 'T', 'Drafted', 0, 0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, state, task_id)
             VALUES ('s-test', 0, 0, 'IDLE', 't-test')",
            [],
        )?;
        // Plus a sentinel-tenant session for the fallback test.
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, state)
             VALUES ('s-orphan', 0, 0, 'IDLE')",
            [],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();
    pool
}

async fn count_projections(pool: &DbPool, event_id: i64) -> usize {
    pool.with_conn(move |conn| {
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tenant_event_view WHERE event_id = ?",
                params![event_id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        Ok::<usize, rusqlite::Error>(n as usize)
    })
    .await
    .unwrap()
}

async fn projection_for(pool: &DbPool, event_id: i64) -> (String, String, String, String) {
    pool.with_conn(move |conn| {
        conn.query_row(
            "SELECT tenant_id, visibility_level, redacted_data, searchable_text
             FROM tenant_event_view WHERE event_id = ?",
            params![event_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn projection_row_written_for_normal_event() {
    let pool = setup().await;
    let store = SqliteEventStore::new(pool.clone());
    let event = store
        .append(NewEvent {
            session_id: "s-test".into(),
            event_type: EventType::Message,
            source: "user".into(),
            data: serde_json::json!({"text": "hello"}),
        })
        .await
        .unwrap();
    assert_eq!(count_projections(&pool, event.id).await, 1);
    let (tenant, vis, redacted, searchable) = projection_for(&pool, event.id).await;
    assert_eq!(tenant, "tenant-test");
    assert_eq!(vis, "user");
    assert!(redacted.contains("hello"));
    assert!(searchable.contains("hello"));
}

#[tokio::test]
async fn projection_falls_back_to_sentinel_tenant_for_orphan_session() {
    // Sessions whose task/project chain is missing must still produce
    // a projection row attributed to the V013 sentinel — silent drops
    // are a regression risk for the cross-source audit (NFR-5.6).
    let pool = setup().await;
    let store = SqliteEventStore::new(pool.clone());
    let event = store
        .append(NewEvent {
            session_id: "s-orphan".into(),
            event_type: EventType::Message,
            source: "user".into(),
            data: serde_json::json!({"text": "orphan-event"}),
        })
        .await
        .unwrap();
    let (tenant, _, _, _) = projection_for(&pool, event.id).await;
    assert_eq!(tenant, "legacy-default");
}

#[tokio::test]
async fn redacts_pem_private_key_in_projection() {
    let pool = setup().await;
    let store = SqliteEventStore::new(pool.clone());
    let pem = "-----BEGIN RSA PRIVATE KEY-----\nABCDEFGHabcdefgh1234567890+/==\n-----END RSA PRIVATE KEY-----";
    let event = store
        .append(NewEvent {
            session_id: "s-test".into(),
            event_type: EventType::Observation,
            source: "tool".into(),
            data: serde_json::json!({"tool_result": pem}),
        })
        .await
        .unwrap();
    let (_, _, redacted, searchable) = projection_for(&pool, event.id).await;
    assert!(redacted.contains("[REDACTED_PRIVATE_KEY]"));
    assert!(!redacted.contains("BEGIN RSA PRIVATE KEY"));
    assert!(!searchable.contains("BEGIN RSA PRIVATE KEY"));
}

#[tokio::test]
async fn redacts_ipv6_in_projection() {
    let pool = setup().await;
    let store = SqliteEventStore::new(pool.clone());
    let event = store
        .append(NewEvent {
            session_id: "s-test".into(),
            event_type: EventType::Action,
            source: "tool".into(),
            data: serde_json::json!({
                "tool_name": "curl",
                "tool_input": {"target": "2001:0db8:85a3:0000:0000:8a2e:0370:7334"},
            }),
        })
        .await
        .unwrap();
    let (_, _, redacted, _) = projection_for(&pool, event.id).await;
    assert!(redacted.contains("[REDACTED_IP]"));
    assert!(!redacted.contains("2001:0db8:85a3"));
}

#[tokio::test]
async fn redacts_authorization_header_in_projection() {
    let pool = setup().await;
    let store = SqliteEventStore::new(pool.clone());
    let event = store
        .append(NewEvent {
            session_id: "s-test".into(),
            event_type: EventType::Action,
            source: "tool".into(),
            data: serde_json::json!({
                "tool_name": "curl",
                "tool_input": {"headers": "Authorization: Bearer sk_live_abc123def456"},
            }),
        })
        .await
        .unwrap();
    let (_, _, redacted, _) = projection_for(&pool, event.id).await;
    assert!(redacted.contains("[REDACTED_AUTH_HEADER]"));
    assert!(!redacted.contains("sk_live_abc123def456"));
}

#[tokio::test]
async fn audit_sourced_event_gets_admin_visibility() {
    // Audit dual-write Misc events carry org-wide action metadata and
    // must not surface to non-admin roles via the timeline projection.
    let pool = setup().await;
    let store = SqliteEventStore::new(pool.clone());
    let event = store
        .append(NewEvent {
            session_id: "s-test".into(),
            event_type: EventType::Misc,
            source: "audit".into(),
            data: serde_json::json!({"kind": "audit_logged", "action": "task.handoff"}),
        })
        .await
        .unwrap();
    let (_, vis, _, _) = projection_for(&pool, event.id).await;
    assert_eq!(vis, "admin");
}

// --- 5.16 read-surface fixtures ---------------------------------------

/// Seed an admin user + matching membership in `tenant-test` so the
/// raw-read audit row's FK to users(id) resolves.
async fn seed_admin(pool: &DbPool) {
    pool.with_conn(|conn| {
        conn.execute(
            "INSERT INTO organizations (id, tenant_id, slug, display_name, status,
                                         created_at, updated_at)
             VALUES ('org-t', 'tenant-test', 'org-t', 'T', 'active', 0, 0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO users (id, tenant_id, email, display_name, status,
                                created_at, updated_at)
             VALUES ('user-admin', 'tenant-test', 'a@x.io', 'A', 'active', 0, 0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO users (id, tenant_id, email, display_name, status,
                                created_at, updated_at)
             VALUES ('user-user', 'tenant-test', 'u@x.io', 'U', 'active', 0, 0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO users (id, tenant_id, email, display_name, status,
                                created_at, updated_at)
             VALUES ('user-viewer', 'tenant-test', 'v@x.io', 'V', 'active', 0, 0)",
            [],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();
}

fn ctx(role: Role, actor: &str) -> AuthContext {
    AuthContext {
        tenant_id: "tenant-test".into(),
        organization_id: "org-t".into(),
        actor_user_id: actor.into(),
        org_role: role,
        project_override_role: None,
    }
}

#[tokio::test]
async fn query_returns_redacted_rows_filtered_by_visibility() {
    let pool = setup().await;
    seed_admin(&pool).await;
    let store = SqliteEventStore::new(pool.clone());
    // 'user' visibility (default user-sourced) + 'admin' visibility
    // (source=="audit") in the same session.
    store
        .append(NewEvent {
            session_id: "s-test".into(),
            event_type: EventType::Message,
            source: "user".into(),
            data: serde_json::json!({"text": "user-visible"}),
        })
        .await
        .unwrap();
    store
        .append(NewEvent {
            session_id: "s-test".into(),
            event_type: EventType::Misc,
            source: "audit".into(),
            data: serde_json::json!({"kind": "audit_logged"}),
        })
        .await
        .unwrap();

    // Admin sees both rows.
    let admin_rows = query(
        &pool,
        &ctx(Role::Admin, "user-admin"),
        "s-test",
        default_q(),
    )
    .await
    .unwrap();
    assert_eq!(admin_rows.len(), 2);

    // User sees only the 'user'-visibility row; the 'admin'-visibility
    // audit row is filtered out at the IN-clause predicate.
    let user_rows = query(&pool, &ctx(Role::User, "user-user"), "s-test", default_q())
        .await
        .unwrap();
    assert_eq!(user_rows.len(), 1);
    assert_eq!(user_rows[0].visibility_level, "user");

    // Viewer sees zero — no 'viewer' rows in this test.
    let viewer_rows = query(
        &pool,
        &ctx(Role::Viewer, "user-viewer"),
        "s-test",
        default_q(),
    )
    .await
    .unwrap();
    assert_eq!(viewer_rows.len(), 0);
}

#[tokio::test]
async fn query_does_not_cross_tenant_boundaries() {
    // Forged tenant in the AuthContext returns zero rows even for a
    // real session — the predicate IS the gate (NFR-5.1 primitive).
    let pool = setup().await;
    seed_admin(&pool).await;
    let store = SqliteEventStore::new(pool.clone());
    store
        .append(NewEvent {
            session_id: "s-test".into(),
            event_type: EventType::Message,
            source: "user".into(),
            data: serde_json::json!({"text": "hi"}),
        })
        .await
        .unwrap();
    let forged = AuthContext {
        tenant_id: "tenant-other".into(),
        ..ctx(Role::Admin, "user-admin")
    };
    let rows = query(&pool, &forged, "s-test", default_q()).await.unwrap();
    assert_eq!(rows.len(), 0);
}

#[tokio::test]
async fn query_raw_admin_returns_data_and_emits_audit_row() {
    let pool = setup().await;
    seed_admin(&pool).await;
    let events = std::sync::Arc::new(SqliteEventStore::new(pool.clone()));
    let audit = AuditLogger::new(pool.clone(), events.clone());
    events
        .append(NewEvent {
            session_id: "s-test".into(),
            event_type: EventType::Message,
            source: "user".into(),
            data: serde_json::json!({"text": "raw-payload"}),
        })
        .await
        .unwrap();
    let rows = query_raw(
        &pool,
        &ctx(Role::Admin, "user-admin"),
        &audit,
        "s-test",
        default_q(),
    )
    .await
    .unwrap();
    assert!(rows.iter().any(|r| r.data.contains("raw-payload")));
    // Audit row recorded.
    let count: i64 = pool
        .with_conn(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM audit_log
                 WHERE actor_user_id = 'user-admin' AND action = 'event.raw_read'
                   AND resource_id = 's-test'",
                [],
                |r| r.get(0),
            )
        })
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn query_raw_viewer_is_denied() {
    let pool = setup().await;
    seed_admin(&pool).await;
    let events = std::sync::Arc::new(SqliteEventStore::new(pool.clone()));
    let audit = AuditLogger::new(pool.clone(), events.clone());
    let err = query_raw(
        &pool,
        &ctx(Role::Viewer, "user-viewer"),
        &audit,
        "s-test",
        default_q(),
    )
    .await
    .expect_err("viewer must be denied raw-read");
    assert!(matches!(err, VisibilityQueryError::Auth(_)));
}

#[tokio::test]
async fn query_raw_user_role_is_denied() {
    let pool = setup().await;
    seed_admin(&pool).await;
    let events = std::sync::Arc::new(SqliteEventStore::new(pool.clone()));
    let audit = AuditLogger::new(pool.clone(), events.clone());
    let err = query_raw(
        &pool,
        &ctx(Role::User, "user-user"),
        &audit,
        "s-test",
        default_q(),
    )
    .await
    .expect_err("user role must be denied raw-read");
    assert!(matches!(err, VisibilityQueryError::Auth(_)));
}

#[tokio::test]
async fn query_raw_blocks_cross_tenant_admin() {
    // Even with `Action::EventRawRead`, an admin in tenant-other must
    // not see tenant-test's raw rows. Tenant boundary trumps role.
    let pool = setup().await;
    seed_admin(&pool).await;
    let events = std::sync::Arc::new(SqliteEventStore::new(pool.clone()));
    let audit = AuditLogger::new(pool.clone(), events.clone());
    // Seed the other tenant's admin user so audit FK resolves.
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
             VALUES ('user-other-admin', 'tenant-other', 'oa@x.io', 'OA', 'active', 0, 0)",
            [],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();
    events
        .append(NewEvent {
            session_id: "s-test".into(),
            event_type: EventType::Message,
            source: "user".into(),
            data: serde_json::json!({"text": "tenant-test secret"}),
        })
        .await
        .unwrap();
    let cross = AuthContext {
        tenant_id: "tenant-other".into(),
        organization_id: "org-other".into(),
        actor_user_id: "user-other-admin".into(),
        org_role: Role::Admin,
        project_override_role: None,
    };
    let rows = query_raw(&pool, &cross, &audit, "s-test", default_q())
        .await
        .unwrap();
    assert_eq!(rows.len(), 0);
}

fn default_q() -> EventReadQuery {
    EventReadQuery::default()
}

#[tokio::test]
async fn query_respects_project_override_downgrade() {
    // Hardening P5-HARD-IT1-H1: an org-admin downgraded to Viewer via a
    // project override must NOT see admin/user-visibility rows — the
    // read surface gates on effective_role(), not raw org_role.
    let pool = setup().await;
    seed_admin(&pool).await;
    let store = SqliteEventStore::new(pool.clone());
    // One 'admin'-visibility row (source=="audit") + one 'user' row.
    store
        .append(NewEvent {
            session_id: "s-test".into(),
            event_type: EventType::Misc,
            source: "audit".into(),
            data: serde_json::json!({"kind": "audit_logged"}),
        })
        .await
        .unwrap();
    store
        .append(NewEvent {
            session_id: "s-test".into(),
            event_type: EventType::Message,
            source: "user".into(),
            data: serde_json::json!({"text": "user-visible"}),
        })
        .await
        .unwrap();

    // org_role=Admin but project_override_role=Viewer → effective Viewer.
    let downgraded = AuthContext {
        tenant_id: "tenant-test".into(),
        organization_id: "org-t".into(),
        actor_user_id: "user-admin".into(),
        org_role: Role::Admin,
        project_override_role: Some(Role::Viewer),
    };
    let rows = query(&pool, &downgraded, "s-test", default_q())
        .await
        .unwrap();
    // Viewer scope = {viewer} only; neither the admin nor user row qualifies.
    assert!(
        rows.is_empty(),
        "project-downgraded admin must be limited to viewer visibility; saw {rows:?}"
    );

    // Sanity: the same actor WITHOUT the override (true org-admin) sees both.
    let full_admin = AuthContext {
        project_override_role: None,
        ..downgraded.clone()
    };
    let admin_rows = query(&pool, &full_admin, "s-test", default_q())
        .await
        .unwrap();
    assert_eq!(admin_rows.len(), 2);
}

#[tokio::test]
async fn projection_hook_does_not_recurse_on_internal_events() {
    // The post-commit quarantine path emits Misc events with the
    // projection-internal source prefix. The hook must skip them so we
    // don't get unbounded recursion or duplicate quarantine rows.
    // We can't easily force a real projection failure in unit tests
    // (the projection is constructed to be infallible-by-construction),
    // so this test verifies the recursion guard directly via the
    // synchronous `apply` function with an internal-source event.
    let pool = setup().await;
    let event = crate::events::Event {
        id: 0,
        session_id: "s-test".into(),
        timestamp: 0,
        event_type: EventType::Misc,
        source: format!("{}_internal", PROJECTION_INTERNAL_SOURCE),
        data: serde_json::json!({}),
    };
    let outcome = pool
        .with_conn(move |conn| Ok::<ProjectionOutcome, rusqlite::Error>(apply(conn, &event)))
        .await
        .unwrap();
    assert_eq!(outcome, ProjectionOutcome::Skipped);
}
