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
                    .query_row("SELECT COUNT(*) FROM session_search_index", [], |row| row.get(0))
                    .unwrap();
                let fts_count: i64 = conn
                    .query_row("SELECT COUNT(*) FROM session_search_fts", [], |row| row.get(0))
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
                    .query_row("SELECT COUNT(*) FROM session_search_index", [], |row| row.get(0))
                    .unwrap();
                let fts_after_delete: i64 = conn
                    .query_row("SELECT COUNT(*) FROM session_search_fts", [], |row| row.get(0))
                    .unwrap();
                assert_eq!(post_delete_hits, 0);
                assert_eq!(source_after_delete, 0);
                assert_eq!(fts_after_delete, 0);
            })
            .await;
        }
    }
}
