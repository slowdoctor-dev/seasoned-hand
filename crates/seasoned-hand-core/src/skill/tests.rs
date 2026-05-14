use super::{PlaybookStore, SkillStore};
use crate::db;

#[tokio::test]
async fn skill_and_playbook_tables_exist_and_are_empty() {
    let pool = db::open(":memory:").await.unwrap();
    let skills = SkillStore::new(pool.clone());
    let playbooks = PlaybookStore::new(pool);

    // V009 ships the schema; Phase 2 logic never writes. Confirm both
    // tables are present and empty so a stray Phase 2 write would
    // surface in CI.
    let skill_rows: i64 = skills
        .pool_for_test()
        .with_conn(|conn| {
            conn.query_row("SELECT COUNT(*) FROM skills", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap()
        })
        .await;
    assert_eq!(skill_rows, 0);

    let playbook_rows: i64 = playbooks
        .pool_for_test()
        .with_conn(|conn| {
            conn.query_row("SELECT COUNT(*) FROM playbooks", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap()
        })
        .await;
    assert_eq!(playbook_rows, 0);
}
