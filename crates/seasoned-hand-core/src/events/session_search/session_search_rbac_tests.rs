use super::{SessionSearchQuery, allowed_visibility_levels_for_role, search_session_events};
use crate::auth::Role;
use crate::db;
use crate::events::{EventStore, EventType, NewEvent, sqlite::SqliteEventStore};

#[tokio::test]
async fn forged_tenant_returns_zero_rows() {
    let pool = db::open(":memory:").await.unwrap();
    seed_project_chain(&pool, "tenant-a", "proj-a", "task-a").await;
    seed_project_chain(&pool, "tenant-b", "proj-b", "task-b").await;
    seed_session(&pool, "sess-a", "task-a", "proj-a").await;
    seed_session(&pool, "sess-b", "task-b", "proj-b").await;

    let store = SqliteEventStore::new(pool.clone());
    store
        .append(NewEvent {
            session_id: "sess-a".into(),
            event_type: EventType::Message,
            source: "user".into(),
            data: serde_json::json!({ "text": "alpha tenant event" }),
        })
        .await
        .unwrap();
    store
        .append(NewEvent {
            session_id: "sess-b".into(),
            event_type: EventType::Message,
            source: "user".into(),
            data: serde_json::json!({ "text": "beta tenant event" }),
        })
        .await
        .unwrap();

    let hits = pool
        .with_conn(|conn| {
            search_session_events(
                conn,
                "alpha",
                &SessionSearchQuery {
                    tenant_id: Some("tenant-z".into()),
                    allowed_visibility_levels: Some(allowed_visibility_levels_for_role(
                        Role::Admin,
                    )),
                    ..SessionSearchQuery::default()
                },
            )
        })
        .await
        .unwrap();
    assert!(hits.is_empty());
}

#[tokio::test]
async fn omitted_tenant_scope_fails_closed() {
    let pool = db::open(":memory:").await.unwrap();
    seed_project_chain(&pool, "tenant-a", "proj-a", "task-a").await;
    seed_project_chain(&pool, "tenant-b", "proj-b", "task-b").await;
    seed_session(&pool, "sess-a", "task-a", "proj-a").await;
    seed_session(&pool, "sess-b", "task-b", "proj-b").await;
    let store = SqliteEventStore::new(pool.clone());

    store
        .append(NewEvent {
            session_id: "sess-a".into(),
            event_type: EventType::Message,
            source: "user".into(),
            data: serde_json::json!({ "text": "alpha-only" }),
        })
        .await
        .unwrap();
    store
        .append(NewEvent {
            session_id: "sess-b".into(),
            event_type: EventType::Message,
            source: "user".into(),
            data: serde_json::json!({ "text": "alpha-only" }),
        })
        .await
        .unwrap();

    let hits = pool
        .with_conn(|conn| {
            search_session_events(
                conn,
                "alpha-only",
                &SessionSearchQuery {
                    // Intentionally omitted tenant_id and visibility levels:
                    // fail-closed must return 0 rows.
                    tenant_id: None,
                    allowed_visibility_levels: None,
                    ..SessionSearchQuery::default()
                },
            )
        })
        .await
        .unwrap();

    assert!(
        hits.is_empty(),
        "omitted tenant/visibility scope must fail closed (no rows)"
    );
}

#[tokio::test]
async fn role_visibility_predicates_are_enforced() {
    let pool = db::open(":memory:").await.unwrap();
    seed_project_chain(&pool, "tenant-a", "proj-a", "task-a").await;
    seed_session(&pool, "sess-a", "task-a", "proj-a").await;
    let store = SqliteEventStore::new(pool.clone());

    let ev_user = store
        .append(NewEvent {
            session_id: "sess-a".into(),
            event_type: EventType::Message,
            source: "user".into(),
            data: serde_json::json!({ "text": "shared note" }),
        })
        .await
        .unwrap();
    let ev_admin = store
        .append(NewEvent {
            session_id: "sess-a".into(),
            event_type: EventType::Misc,
            source: "audit".into(),
            data: serde_json::json!({ "kind": "audit_only", "detail": "admin-only evidence" }),
        })
        .await
        .unwrap();
    // Force one row to viewer visibility so we can exercise all 3 classes.
    pool.with_conn(move |conn| {
        conn.execute(
            "UPDATE tenant_event_view SET visibility_level='viewer' WHERE event_id=?",
            [ev_user.id],
        )?;
        conn.execute(
            "UPDATE session_search_index SET visibility_level='viewer' WHERE event_id=?",
            [ev_user.id],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();

    let viewer_hits = pool
        .with_conn(|conn| {
            search_session_events(
                conn,
                "note OR \"admin-only\"",
                &SessionSearchQuery {
                    tenant_id: Some("tenant-a".into()),
                    allowed_visibility_levels: Some(allowed_visibility_levels_for_role(
                        Role::Viewer,
                    )),
                    ..SessionSearchQuery::default()
                },
            )
        })
        .await
        .unwrap();
    assert!(viewer_hits.iter().all(|h| h.event_id != ev_admin.id));
    assert!(viewer_hits.iter().any(|h| h.event_id == ev_user.id));

    let user_hits = pool
        .with_conn(|conn| {
            search_session_events(
                conn,
                "note OR \"admin-only\"",
                &SessionSearchQuery {
                    tenant_id: Some("tenant-a".into()),
                    allowed_visibility_levels: Some(allowed_visibility_levels_for_role(Role::User)),
                    ..SessionSearchQuery::default()
                },
            )
        })
        .await
        .unwrap();
    assert!(user_hits.iter().all(|h| h.event_id != ev_admin.id));

    let admin_hits = pool
        .with_conn(|conn| {
            search_session_events(
                conn,
                "note OR \"admin-only\"",
                &SessionSearchQuery {
                    tenant_id: Some("tenant-a".into()),
                    allowed_visibility_levels: Some(allowed_visibility_levels_for_role(
                        Role::Admin,
                    )),
                    ..SessionSearchQuery::default()
                },
            )
        })
        .await
        .unwrap();
    assert!(admin_hits.iter().any(|h| h.event_id == ev_admin.id));
}

#[tokio::test]
async fn indexed_snippet_is_redacted_source_text() {
    let pool = db::open(":memory:").await.unwrap();
    seed_project_chain(&pool, "tenant-a", "proj-a", "task-a").await;
    seed_session(&pool, "sess-a", "task-a", "proj-a").await;
    let store = SqliteEventStore::new(pool.clone());
    store
        .append(NewEvent {
            session_id: "sess-a".into(),
            event_type: EventType::Observation,
            source: "tool".into(),
            data: serde_json::json!({
                "tool_result": "BEGIN PRIVATE KEY\nMII...\nEND PRIVATE KEY\ncontact 2001:db8::1"
            }),
        })
        .await
        .unwrap();

    let hits = pool
        .with_conn(|conn| {
            search_session_events(
                conn,
                "PRIVATE",
                &SessionSearchQuery {
                    tenant_id: Some("tenant-a".into()),
                    allowed_visibility_levels: Some(allowed_visibility_levels_for_role(
                        Role::Admin,
                    )),
                    ..SessionSearchQuery::default()
                },
            )
        })
        .await
        .unwrap();

    if let Some(hit) = hits.first() {
        assert!(!hit.snippet.contains("BEGIN PRIVATE KEY"));
        assert!(!hit.snippet.contains("2001:db8::1"));
    }
}

async fn seed_project_chain(
    pool: &crate::db::DbPool,
    tenant_id: &str,
    project_id: &str,
    task_id: &str,
) {
    let tenant_id = tenant_id.to_string();
    let project_id = project_id.to_string();
    let task_id = task_id.to_string();
    pool.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO projects (id, tenant_id, title, status, created_at, updated_at)
             VALUES (?, ?, 'P', 'active', 0, 0)",
            rusqlite::params![project_id, tenant_id],
        )?;
        conn.execute(
            "INSERT INTO tasks (id, project_id, tenant_id, title, status, created_at, updated_at)
             VALUES (?, ?, ?, 'T', 'Running', 0, 0)",
            rusqlite::params![task_id, project_id, tenant_id],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();
}

async fn seed_session(pool: &crate::db::DbPool, session_id: &str, task_id: &str, project_id: &str) {
    let session_id = session_id.to_string();
    let task_id = task_id.to_string();
    let project_id = project_id.to_string();
    pool.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, state, project_id, task_id)
             VALUES (?, 0, 0, 'RUNNING', ?, ?)",
            rusqlite::params![session_id, project_id, task_id],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();
}
