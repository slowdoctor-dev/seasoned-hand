//! Story 5.14 regression tests for [`crate::events::visibility`].
//! refs: /specs/phase-5/stories/story-5.14.md

use super::*;
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
