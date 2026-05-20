//! Phase 5 story 5.29 (part 2) — `phase5_search_rbac_harness`.
//!
//! Verifies the session-search RBAC gate from story 5.15: forged
//! tenant_id in the search query returns 0 rows because the predicate
//! `i.tenant_id = ?` filters at the DB layer. The harness seeds two
//! tenants, each with one session containing one matchable event,
//! then runs the FTS query with cross-tenant filters and asserts the
//! result set is empty.
//!
//! The DB-predicate enforcement model means there is no
//! `forged_tenant_query_rejected` event — the wrong-tenant query just
//! returns the empty set. That's the spec's "0 rows" branch.
//!
//! refs: /specs/phase-5/stories/story-5.29.md
//! refs: /specs/phase-5/architecture.md §15 harness 8, §10
//! refs: /specs/phase-5/requirements.md F-5.11, NFR-5.6

use rusqlite::params;
use seasoned_hand_core::auth::Role;
use seasoned_hand_core::db::{self, DbPool};
use seasoned_hand_core::events::session_search::{
    SessionSearchQuery, allowed_visibility_levels_for_role, search_session_events,
};
use seasoned_hand_core::events::sqlite::SqliteEventStore;
use seasoned_hand_core::events::{EventStore, EventType, NewEvent};

async fn seed_two_tenants_with_events(pool: &DbPool) {
    pool.with_conn(|conn| {
        for (org, tenant, slug) in [("org-a", "tenant-a", "a"), ("org-b", "tenant-b", "b")] {
            conn.execute(
                "INSERT INTO organizations (id, tenant_id, slug, display_name, status,
                                             created_at, updated_at)
                 VALUES (?, ?, ?, 'X', 'active', 0, 0)",
                params![org, tenant, slug],
            )?;
        }
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

#[tokio::test]
async fn phase5_search_rbac_harness() {
    let pool = db::open(":memory:").await.expect("open db");
    seed_two_tenants_with_events(&pool).await;
    let store = SqliteEventStore::new(pool.clone());

    // Each tenant gets one event containing a distinctive token. The
    // token is the same word ("alpenglow") so the FTS query matches
    // both tenants' data — without the RBAC predicate, the search
    // would return both rows.
    for (sid, payload) in [
        ("sess-a", "alpenglow over mountains"),
        ("sess-b", "alpenglow across sea"),
    ] {
        store
            .append(NewEvent {
                session_id: sid.into(),
                event_type: EventType::Message,
                source: "user".into(),
                data: serde_json::json!({"text": payload}),
            })
            .await
            .expect("append event");
    }

    // ---------- 1. Tenant-A admin query → only sess-a row ----------
    let a_allowed = allowed_visibility_levels_for_role(Role::Admin);
    let a_hits = pool
        .with_conn(move |conn| {
            search_session_events(
                conn,
                "alpenglow",
                &SessionSearchQuery {
                    tenant_id: Some("tenant-a".to_string()),
                    allowed_visibility_levels: Some(a_allowed),
                    ..SessionSearchQuery::default()
                },
            )
        })
        .await
        .expect("search a");
    assert_eq!(
        a_hits.len(),
        1,
        "tenant-A query must return exactly tenant-A's row"
    );
    assert_eq!(a_hits[0].session_id, "sess-a");

    // ---------- 2. Forged tenant_id ("tenant-z") → 0 rows ----------
    let z_allowed = allowed_visibility_levels_for_role(Role::Admin);
    let z_hits = pool
        .with_conn(move |conn| {
            search_session_events(
                conn,
                "alpenglow",
                &SessionSearchQuery {
                    tenant_id: Some("tenant-z".to_string()),
                    allowed_visibility_levels: Some(z_allowed),
                    ..SessionSearchQuery::default()
                },
            )
        })
        .await
        .expect("search forged");
    assert!(
        z_hits.is_empty(),
        "forged tenant_id must return zero rows; got {} hits",
        z_hits.len()
    );

    // ---------- 3. Tenant-A query for tenant-B's specific session id → 0 rows ----------
    // Predicate is `(tenant_id = A) AND (session_id = sess-b)` — the
    // tenant gate wins even if the caller pins a specific session.
    let cross_allowed = allowed_visibility_levels_for_role(Role::Admin);
    let cross_hits = pool
        .with_conn(move |conn| {
            search_session_events(
                conn,
                "alpenglow",
                &SessionSearchQuery {
                    tenant_id: Some("tenant-a".to_string()),
                    allowed_visibility_levels: Some(cross_allowed),
                    session_id: Some("sess-b".to_string()),
                    ..SessionSearchQuery::default()
                },
            )
        })
        .await
        .expect("search cross");
    assert!(
        cross_hits.is_empty(),
        "tenant-A caller asking for tenant-B's session must return zero rows"
    );

    // ---------- 4. Viewer role sees only 'viewer' visibility events ----------
    // The seeded events default to 'user' visibility, so a Viewer
    // query returns 0 rows (correct — viewers can only see
    // 'viewer'-tagged events).
    let viewer_allowed = allowed_visibility_levels_for_role(Role::Viewer);
    assert_eq!(viewer_allowed, vec!["viewer".to_string()]);
    let viewer_hits = pool
        .with_conn(move |conn| {
            search_session_events(
                conn,
                "alpenglow",
                &SessionSearchQuery {
                    tenant_id: Some("tenant-a".to_string()),
                    allowed_visibility_levels: Some(viewer_allowed),
                    ..SessionSearchQuery::default()
                },
            )
        })
        .await
        .expect("search viewer");
    assert!(
        viewer_hits.is_empty(),
        "viewer role must not see 'user'-visibility events"
    );
}
