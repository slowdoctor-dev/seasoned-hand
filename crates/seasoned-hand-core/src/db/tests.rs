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
