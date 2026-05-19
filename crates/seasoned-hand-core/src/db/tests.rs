use super::*;

#[tokio::test]
async fn opens_in_memory_db_and_runs_migrations() {
    let pool = open(":memory:").await.expect("open in-memory db");
    pool.with_conn(|conn| {
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for required in ["events", "plans", "sessions"] {
            assert!(
                tables.contains(&required.to_string()),
                "missing table {required}; got {tables:?}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn migrations_are_idempotent() {
    let pool = open(":memory:").await.unwrap();
    pool.with_conn(|conn| run_migrations(conn).expect("idempotent re-run"))
        .await;
}

#[tokio::test]
async fn opens_file_db_and_creates_parent_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("nested/dir/test.db");
    let url = format!("sqlite:{}", path.display());
    let _pool = open(&url).await.expect("open file db");
    assert!(path.exists(), "db file was not created");
}

#[tokio::test]
async fn file_db_uses_wal_mode() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("wal.db");
    let url = format!("sqlite:{}", path.display());
    let pool = open(&url).await.expect("open file db");
    pool.with_conn(|conn| {
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
    })
    .await;
}

#[tokio::test]
async fn foreign_keys_are_enforced() {
    let pool = open(":memory:").await.unwrap();
    pool.with_conn(|conn| {
        let fk: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(fk, 1);

        let bad = conn.execute(
            "INSERT INTO events (session_id, timestamp, type, source, data) \
             VALUES ('nonexistent', 1, 'Misc', 'test', '{}')",
            [],
        );
        assert!(bad.is_err(), "expected foreign key violation");
    })
    .await;
}

#[tokio::test]
async fn migration_v010_creates_learning_artifact_tables_and_triggers() {
    let pool = open(":memory:").await.unwrap();
    pool.with_conn(|conn| {
        let has_table = |name: &str| -> bool {
            conn.query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM sqlite_master
                   WHERE type='table' AND name = ?
                 )",
                [name],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
                == 1
        };
        let has_trigger = |name: &str| -> bool {
            conn.query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM sqlite_master
                   WHERE type='trigger' AND name = ?
                 )",
                [name],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
                == 1
        };

        assert!(has_table("sops"));
        assert!(has_table("glossary"));
        assert!(has_table("session_search_index"));
        assert!(has_table("playbooks_fts"));
        assert!(has_table("session_search_fts"));
        assert!(has_trigger("playbooks_ai"));
        assert!(has_trigger("playbooks_ad"));
        assert!(has_trigger("playbooks_au"));
        assert!(has_trigger("session_search_index_ai"));
        assert!(has_trigger("session_search_index_ad"));
        assert!(has_trigger("session_search_index_au"));
    })
    .await;
}

#[tokio::test]
async fn migration_v011_creates_curator_tables_and_triggers() {
    let pool = open(":memory:").await.unwrap();
    pool.with_conn(|conn| {
        let has_table = |name: &str| -> bool {
            conn.query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM sqlite_master
                   WHERE type='table' AND name = ?
                 )",
                [name],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
                == 1
        };
        let has_trigger = |name: &str| -> bool {
            conn.query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM sqlite_master
                   WHERE type='trigger' AND name = ?
                 )",
                [name],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
                == 1
        };
        let has_column = |table: &str, col: &str| -> bool {
            let mut stmt = conn
                .prepare(&format!("PRAGMA table_info('{table}')"))
                .unwrap();
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            rows.iter().any(|c| c == col)
        };

        for table in [
            "playbook_revisions",
            "playbook_revision_outcomes",
            "curator_decisions",
            "curator_review_queue",
            "sop_conflicts",
            "knowledge_items",
            "datasource_items",
            "weekly_retrospectives",
            "retrospective_citations",
            "curator_search_index",
            "curator_search_fts",
        ] {
            assert!(has_table(table), "missing table {table}");
        }

        assert!(has_trigger("curator_search_index_ai"));
        assert!(has_trigger("curator_search_index_ad"));
        assert!(has_trigger("curator_search_index_au"));
        assert!(has_column("playbooks", "source_project_id"));
        assert!(has_column("playbooks", "active_revision_id"));
        assert!(has_column("playbooks", "archived_reason"));
        assert!(has_column("playbooks", "archived_at"));
    })
    .await;
}

#[tokio::test]
async fn migration_v011_idempotent_via_embedded_runner() {
    let pool = open(":memory:").await.unwrap();
    // V011 has already run during open(); running again through refinery must no-op.
    pool.with_conn(|conn| run_migrations(conn).expect("idempotent V011 re-run"))
        .await;
    pool.with_conn(|conn| {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='table' AND name='playbook_revisions'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    })
    .await;
}

#[tokio::test]
async fn migration_v012_creates_curator_decisions_summary() {
    let pool = open(":memory:").await.unwrap();
    pool.with_conn(|conn| {
        let table_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='table' AND name='curator_decisions_summary'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            table_exists, 1,
            "V012 must create curator_decisions_summary"
        );

        let index_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='index' AND name='idx_curator_decisions_summary_project_week'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(index_exists, 1, "project-week index must exist");

        // UNIQUE(project_id, week_start, week_end, decision_type) is what makes
        // CuratorRetentionJob's UPSERT idempotent — assert it is enforced.
        conn.execute(
            "INSERT INTO curator_decisions_summary (
                 id, tenant_id, project_id, week_start, week_end, decision_type,
                 decision_count, mean_confidence, created_at
             ) VALUES ('s1', NULL, 'proj-a', 0, 6048, 'merge', 5, 0.8, 0)",
            [],
        )
        .unwrap();
        let dup_result = conn.execute(
            "INSERT INTO curator_decisions_summary (
                 id, tenant_id, project_id, week_start, week_end, decision_type,
                 decision_count, mean_confidence, created_at
             ) VALUES ('s2', NULL, 'proj-a', 0, 6048, 'merge', 1, 0.9, 0)",
            [],
        );
        assert!(
            dup_result.is_err(),
            "UNIQUE(project_id, week_start, week_end, decision_type) must block duplicate bucket"
        );
    })
    .await;
}

#[test]
fn migration_v011_backfill_from_v010_rows() {
    use rusqlite::Connection;

    static V001: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../migrations/V001__sessions.sql"
    ));
    static V002: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../migrations/V002__events.sql"
    ));
    static V003: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../migrations/V003__plans.sql"
    ));
    static V004: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../migrations/V004__verifications.sql"
    ));
    static V005: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../migrations/V005__checkpoints.sql"
    ));
    static V006: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../migrations/V006__phase2_projects_tasks.sql"
    ));
    static V007: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../migrations/V007__phase2_deliverables.sql"
    ));
    static V008: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../migrations/V008__phase2_intake_delivery_notifications.sql"
    ));
    static V009: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../migrations/V009__phase2_skills_playbooks.sql"
    ));
    static V010: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../migrations/V010__phase3_learning_artifacts.sql"
    ));
    static V011: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../migrations/V011__phase4_curator.sql"
    ));

    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    for sql in [V001, V002, V003, V004, V005, V006, V007, V008, V009, V010] {
        conn.execute_batch(sql).unwrap();
    }

    conn.execute(
        "INSERT INTO projects (id, tenant_id, title, description, status, created_at, updated_at)
         VALUES (?1, ?2, ?3, NULL, 'active', 1, 1)",
        rusqlite::params!["proj-1", Option::<String>::None, "P1"],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO tasks (id, project_id, tenant_id, title, brief, status, expected_due_at, completed_at, failure_reason, parent_task_id, schedule, skill_attached_event_id, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, NULL, 'Completed', NULL, NULL, NULL, NULL, NULL, NULL, 2, 2)",
        rusqlite::params!["task-1", "proj-1", Option::<String>::None, "T1"],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO playbooks (id, tenant_id, title, content_path, schema_version, source_task_id, created_at, updated_at, trigger_keywords, content, success_count, failure_count, status, version)
         VALUES (?1, ?2, ?3, ?4, 1, ?5, 3, 4, ?6, ?7, 7, 2, 'active', 1)",
        rusqlite::params![
            "pb-1",
            Option::<String>::None,
            "Playbook One",
            "phase3/pb-1.md",
            "task-1",
            "[\"alpha\"]",
            "steps alpha",
        ],
    )
    .unwrap();

    conn.execute_batch(V011).unwrap();

    let source_project_id: String = conn
        .query_row(
            "SELECT source_project_id FROM playbooks WHERE id = 'pb-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(source_project_id, "proj-1");

    let active_revision_id: String = conn
        .query_row(
            "SELECT active_revision_id FROM playbooks WHERE id = 'pb-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(active_revision_id, "rev-pb-1-1");

    let rev_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM playbook_revisions WHERE playbook_id = 'pb-1' AND revision_no = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rev_count, 1);

    let (s, f): (i64, i64) = conn
        .query_row(
            "SELECT success_count, failure_count
             FROM playbook_revision_outcomes
             WHERE revision_id = 'rev-pb-1-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((s, f), (7, 2));

    // Trigger correctness smoke for the new FTS surface.
    conn.execute(
        "INSERT INTO curator_search_index (project_id, source_type, source_id, searchable_text, created_at)
         VALUES ('proj-1', 'decision', 'd1', 'hello retention world', 10)",
        [],
    )
    .unwrap();
    let hits: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM curator_search_fts WHERE curator_search_fts MATCH 'retention'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(hits, 1);
}

mod fts5 {
    mod trigger_correctness {
        use super::super::open;

        #[tokio::test]
        async fn playbooks_triggers_track_insert_update_delete_and_consistency() {
            let pool = open(":memory:").await.unwrap();
            pool.with_conn(|conn| {
                conn.execute(
                    "INSERT INTO playbooks (
                        id, tenant_id, title, content_path, schema_version, source_task_id,
                        created_at, updated_at, trigger_keywords, content
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    rusqlite::params![
                        "pb-1",
                        "tenant-1",
                        "Initial Playbook",
                        "phase3/pb-1.md",
                        1_i64,
                        Option::<String>::None,
                        1_i64,
                        1_i64,
                        "[\"trigger-one\"]",
                        "initial content tokenalpha",
                    ],
                )
                .unwrap();

                let initial_hits: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM playbooks_fts WHERE playbooks_fts MATCH 'tokenalpha'",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(initial_hits, 1);

                conn.execute(
                    "UPDATE playbooks
                     SET title = ?1, trigger_keywords = ?2, content = ?3, updated_at = ?4
                     WHERE id = ?5",
                    rusqlite::params![
                        "Updated Playbook",
                        "[\"trigger-two\"]",
                        "updated content tokenbeta",
                        2_i64,
                        "pb-1",
                    ],
                )
                .unwrap();

                let stale_hits: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM playbooks_fts WHERE playbooks_fts MATCH 'tokenalpha'",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                let updated_hits: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM playbooks_fts WHERE playbooks_fts MATCH 'tokenbeta'",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(stale_hits, 0);
                assert_eq!(updated_hits, 1);

                let source_count: i64 = conn
                    .query_row("SELECT COUNT(*) FROM playbooks", [], |row| row.get(0))
                    .unwrap();
                let fts_count: i64 = conn
                    .query_row("SELECT COUNT(*) FROM playbooks_fts", [], |row| row.get(0))
                    .unwrap();
                assert_eq!(source_count, fts_count);

                conn.execute("DELETE FROM playbooks WHERE id = ?1", ["pb-1"])
                    .unwrap();
                let post_delete_hits: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM playbooks_fts WHERE playbooks_fts MATCH 'tokenbeta'",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                let source_after_delete: i64 = conn
                    .query_row("SELECT COUNT(*) FROM playbooks", [], |row| row.get(0))
                    .unwrap();
                let fts_after_delete: i64 = conn
                    .query_row("SELECT COUNT(*) FROM playbooks_fts", [], |row| row.get(0))
                    .unwrap();
                assert_eq!(post_delete_hits, 0);
                assert_eq!(source_after_delete, 0);
                assert_eq!(fts_after_delete, 0);
            })
            .await;
        }

        #[tokio::test]
        async fn session_search_triggers_track_insert_update_delete_and_consistency() {
            let pool = open(":memory:").await.unwrap();
            pool.with_conn(|conn| {
                conn.execute(
                    "INSERT INTO session_search_index (
                        event_id, session_id, timestamp, event_type, source, searchable_text
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![
                        101_i64,
                        "session-1",
                        10_i64,
                        "Action",
                        "tool.call",
                        "initial search termone",
                    ],
                )
                .unwrap();

                let initial_hits: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM session_search_fts
                         WHERE session_search_fts MATCH 'termone'",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(initial_hits, 1);

                conn.execute(
                    "UPDATE session_search_index
                     SET searchable_text = ?1, source = ?2
                     WHERE event_id = ?3",
                    rusqlite::params!["updated search termtwo", "tool.result", 101_i64],
                )
                .unwrap();

                let stale_hits: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM session_search_fts
                         WHERE session_search_fts MATCH 'termone'",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                let updated_hits: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM session_search_fts
                         WHERE session_search_fts MATCH 'termtwo'",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(stale_hits, 0);
                assert_eq!(updated_hits, 1);

                let source_count: i64 = conn
                    .query_row("SELECT COUNT(*) FROM session_search_index", [], |row| {
                        row.get(0)
                    })
                    .unwrap();
                let fts_count: i64 = conn
                    .query_row("SELECT COUNT(*) FROM session_search_fts", [], |row| {
                        row.get(0)
                    })
                    .unwrap();
                assert_eq!(source_count, fts_count);

                conn.execute(
                    "DELETE FROM session_search_index WHERE event_id = ?1",
                    [101_i64],
                )
                .unwrap();
                let post_delete_hits: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM session_search_fts
                         WHERE session_search_fts MATCH 'termtwo'",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                let source_after_delete: i64 = conn
                    .query_row("SELECT COUNT(*) FROM session_search_index", [], |row| {
                        row.get(0)
                    })
                    .unwrap();
                let fts_after_delete: i64 = conn
                    .query_row("SELECT COUNT(*) FROM session_search_fts", [], |row| {
                        row.get(0)
                    })
                    .unwrap();
                assert_eq!(post_delete_hits, 0);
                assert_eq!(source_after_delete, 0);
                assert_eq!(fts_after_delete, 0);
            })
            .await;
        }
    }
}
