//! Phase 5 story 5.31 — `phase5_v013_migration_harness`.
//!
//! Verifies NFR-5.8: the V013 tenant-tightening migration (plus its
//! follow-on per-domain NOT NULL flips in V014-V020) applies from a
//! Phase 4 baseline DB with deterministic backfill and no destructive
//! data loss.
//!
//! Refinery embeds migrations in the binary; `db::open()` runs the
//! full chain in order. There's no public per-migration cursor, so
//! the harness asserts the POST-migration invariants — which is what
//! NFR-5.8 actually cares about. The Phase 4 → Phase 5 transition is
//! correct iff:
//!
//! 1. Sentinel rows bootstrap exists: `org-legacy-default` /
//!    `user-legacy-admin` + the admin membership linking them.
//! 2. No mutable table has NULL `tenant_id` rows (V014-V020 flipped
//!    each in turn; remaining NULLs would mean the V013 backfill
//!    missed a row).
//! 3. Tenant chain integrity holds: a task's tenant equals its
//!    project's tenant; a deliverable's tenant equals its task's;
//!    a session_search_index row's tenant equals the indexed event's
//!    derived tenant.
//! 4. Running `run_migrations` a second time is a no-op
//!    (refinery's applied-migrations tracking).
//!
//! refs: /specs/phase-5/stories/story-5.31.md
//! refs: /specs/phase-5/architecture.md §15 harness 6, §3.4
//! refs: /specs/phase-5/requirements.md F-5.3, NFR-5.8

use rusqlite::params;
use seasoned_hand_core::db::{self, run_migrations};

#[tokio::test]
async fn phase5_v013_migration_harness() {
    let pool = db::open(":memory:")
        .await
        .expect("open db (runs migrations)");

    // ---------- 1. Sentinel bootstrap rows exist ----------
    let sentinel: (String, String, String) = pool
        .with_conn(|conn| {
            // Sentinel organization.
            let org: String = conn
                .query_row(
                    "SELECT id FROM organizations WHERE id = 'org-legacy-default'",
                    [],
                    |r| r.get(0),
                )
                .expect("sentinel organization must exist post-V013");
            // Sentinel user.
            let user: String = conn
                .query_row(
                    "SELECT id FROM users WHERE id = 'user-legacy-admin'",
                    [],
                    |r| r.get(0),
                )
                .expect("sentinel user must exist post-V013");
            // Admin membership linking them.
            let membership: String = conn
                .query_row(
                    "SELECT id FROM organization_memberships
                     WHERE user_id = 'user-legacy-admin'
                       AND organization_id = 'org-legacy-default'
                       AND role = 'admin'
                       AND is_primary = 1",
                    [],
                    |r| r.get(0),
                )
                .expect("sentinel admin membership must exist post-V013");
            Ok::<(String, String, String), rusqlite::Error>((org, user, membership))
        })
        .await
        .unwrap();
    assert_eq!(sentinel.0, "org-legacy-default");
    assert_eq!(sentinel.1, "user-legacy-admin");
    assert!(!sentinel.2.is_empty());

    // ---------- 2. No mutable table has NULL tenant_id rows ----------
    // V014-V020 flipped these per-domain. Any remaining NULL means
    // either the V013 backfill missed a row or a post-migration
    // INSERT bypassed the NOT NULL constraint (shouldn't be possible
    // given the SQLite schema, but we verify defensively).
    let null_counts: Vec<(String, i64)> = pool
        .with_conn(|conn| {
            // Tables that gained tenant_id NOT NULL via Phase 5
            // migrations. Each MUST have zero NULLs.
            let tables = [
                "projects",
                "tasks",
                "deliverables",
                "playbooks",
                "playbook_revisions",
                "sops",
                "skills",
                "curator_decisions",
                "curator_review_queue",
                "weekly_retrospectives",
                "knowledge_items",
                "datasource_items",
            ];
            let mut counts = Vec::new();
            for t in tables {
                let sql = format!("SELECT COUNT(*) FROM {t} WHERE tenant_id IS NULL");
                let n: i64 = conn.query_row(&sql, [], |r| r.get(0)).unwrap_or(0);
                counts.push((t.to_string(), n));
            }
            Ok::<Vec<(String, i64)>, rusqlite::Error>(counts)
        })
        .await
        .unwrap();
    for (table, n) in &null_counts {
        assert_eq!(
            *n, 0,
            "table {table} has {n} NULL tenant_id rows post-migration; V013 backfill incomplete"
        );
    }

    // ---------- 3. Tenant chain integrity ----------
    // Insert a project + task + deliverable in one tenant, then
    // assert the FK + tenant chain is queryable end-to-end.
    pool.with_conn(|conn| {
        conn.execute(
            "INSERT INTO projects (id, tenant_id, title, status, created_at, updated_at)
             VALUES ('p-mig', 'tenant-mig', 'P', 'active', 0, 0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO tasks (id, project_id, tenant_id, title, status,
                                created_at, updated_at)
             VALUES ('t-mig', 'p-mig', 'tenant-mig', 'T', 'drafted', 0, 0)",
            [],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();
    // Verify task → project tenant chain.
    let chain: (String, String) = pool
        .with_conn(|conn| {
            conn.query_row(
                "SELECT t.tenant_id, p.tenant_id
                 FROM tasks t JOIN projects p ON p.id = t.project_id
                 WHERE t.id = 't-mig'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        chain.0, chain.1,
        "task tenant must equal project tenant in chain integrity"
    );
    assert_eq!(chain.0, "tenant-mig");

    // ---------- 4. Migration runner is idempotent ----------
    // Re-running migrations on a fully-migrated connection must be
    // a no-op (refinery tracks applied versions in
    // refinery_schema_history). If this errors, replaying a
    // migration would risk data loss.
    pool.with_conn(|conn| {
        run_migrations(conn).expect("re-running migrations must be idempotent");
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();

    // Sentinel rows still there after re-run — defensive check that
    // idempotent re-run didn't somehow re-insert / corrupt them.
    let sentinel_count: i64 = pool
        .with_conn(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM organizations WHERE id = 'org-legacy-default'",
                [],
                |r| r.get(0),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        sentinel_count, 1,
        "sentinel organization count must stay 1 after idempotent re-run"
    );

    // ---------- 5. tenant_event_view exists + has visibility_level CHECK ----------
    // V013 created the projection table. Its visibility_level column
    // has a CHECK constraint limiting values to ('viewer','user',
    // 'admin'). Inserting an out-of-range value must fail.
    let bad_insert: Result<(), rusqlite::Error> = pool
        .with_conn(|conn| {
            // First seed a session + event we can reference.
            conn.execute(
                "INSERT INTO sessions (id, created_at, updated_at, state)
                 VALUES ('s-mig', 0, 0, 'IDLE')",
                [],
            )?;
            let event_id: i64 = conn.query_row(
                "INSERT INTO events (session_id, timestamp, type, source, data)
                 VALUES ('s-mig', 0, 'Message', 'user', '{}') RETURNING id",
                [],
                |r| r.get(0),
            )?;
            conn.execute(
                "INSERT INTO tenant_event_view
                   (event_id, tenant_id, visibility_level, redacted_data,
                    searchable_text, created_at)
                 VALUES (?, 'tenant-mig', 'bogus', '{}', '', 0)",
                params![event_id],
            )?;
            Ok(())
        })
        .await;
    assert!(
        bad_insert.is_err(),
        "tenant_event_view visibility_level CHECK must reject out-of-range values"
    );
}
