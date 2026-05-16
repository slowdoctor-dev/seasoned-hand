use serde_json::json;

use super::{Deliverable, DeliverableError, DeliverableStore, NewDeliverable};
use crate::db;
use crate::project::{NewProject, NewTask, ProjectStore, TaskStore};

async fn open_pool() -> db::DbPool {
    db::open(":memory:").await.expect("open in-memory db")
}

async fn seed_task(pool: &db::DbPool) -> String {
    let projects = ProjectStore::new(pool.clone());
    let tasks = TaskStore::new(pool.clone());
    let pid = projects
        .insert(NewProject {
            tenant_id: None,
            title: "P".into(),
            description: None,
        })
        .await
        .unwrap();
    tasks
        .insert(NewTask {
            project_id: pid,
            tenant_id: None,
            title: "T".into(),
            expected_due_at: None,
        })
        .await
        .unwrap()
}

fn sample_new(task_id: &str) -> NewDeliverable {
    NewDeliverable {
        task_id: task_id.to_string(),
        tenant_id: None,
        format: "md".into(),
        source_content_path: Some("/workspace/.source/d1.md".into()),
        source_content_sha256: Some("a".repeat(64)),
        rendered_content_path: "/workspace/.deliverables/d1.md".into(),
        rendered_content_sha256: "b".repeat(64),
        content_size: 1234,
        citations: Some(vec![1, 2, 3]),
        provenance_manifest: json!({ "schema_version": 1 }),
    }
}

#[tokio::test]
async fn deliverable_store_crud() {
    let pool = open_pool().await;
    let task_id = seed_task(&pool).await;
    let store = DeliverableStore::new(pool);

    let id = store.insert(sample_new(&task_id)).await.expect("insert");
    let row: Deliverable = store.get(&id).await.expect("get");
    assert_eq!(row.id, id);
    assert_eq!(row.task_id, task_id);
    assert_eq!(row.format, "md");
    assert_eq!(row.content_size, 1234);
    assert_eq!(row.citations.as_deref(), Some(&[1_i64, 2, 3][..]));
    assert_eq!(row.provenance_manifest, json!({ "schema_version": 1 }));

    // list_by_task returns the inserted row.
    let listed = store.list_by_task(&task_id).await.expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, id);

    // attach_provenance replaces the column.
    store
        .attach_provenance(&id, &json!({ "schema_version": 1, "decisions": [42] }))
        .await
        .expect("attach_provenance");
    let row = store.get(&id).await.unwrap();
    assert_eq!(
        row.provenance_manifest,
        json!({ "schema_version": 1, "decisions": [42] })
    );

    // assert_exists passes for an existing row.
    store.assert_exists(&id).await.expect("assert_exists");

    // NotFound for unknown ids on every method that takes one.
    assert!(matches!(
        store.get("nope").await.expect_err("get nope"),
        DeliverableError::NotFound(_)
    ));
    assert!(matches!(
        store
            .attach_provenance("nope", &json!({}))
            .await
            .expect_err("attach nope"),
        DeliverableError::NotFound(_)
    ));
    assert!(matches!(
        store
            .assert_exists("nope")
            .await
            .expect_err("assert_exists nope"),
        DeliverableError::NotFound(_)
    ));
}
