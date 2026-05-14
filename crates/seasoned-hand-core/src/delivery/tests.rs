use serde_json::json;

use super::{DeliveryEventStore, NewDeliveryEvent};
use crate::channel::DeliveryTarget;
use crate::db;
use crate::deliverable::{DeliverableStore, NewDeliverable};
use crate::project::{NewProject, NewTask, ProjectStore, TaskStore};

async fn seed_task_and_deliverable(pool: &db::DbPool) -> (String, String) {
    let projects = ProjectStore::new(pool.clone());
    let tasks = TaskStore::new(pool.clone());
    let deliverables = DeliverableStore::new(pool.clone());
    let pid = projects
        .insert(NewProject {
            tenant_id: None,
            title: "P".into(),
            description: None,
        })
        .await
        .unwrap();
    let task_id = tasks
        .insert(NewTask {
            project_id: pid,
            tenant_id: None,
            title: "T".into(),
            expected_due_at: None,
        })
        .await
        .unwrap();
    let did = deliverables
        .insert(NewDeliverable {
            task_id: task_id.clone(),
            tenant_id: None,
            format: "md".into(),
            source_content_path: None,
            source_content_sha256: None,
            rendered_content_path: "/workspace/.deliverables/d.md".into(),
            rendered_content_sha256: "b".repeat(64),
            content_size: 0,
            citations: None,
            provenance_manifest: json!({}),
        })
        .await
        .unwrap();
    (task_id, did)
}

fn target(channel: &str) -> DeliveryTarget {
    DeliveryTarget {
        channel: channel.into(),
        target_ref: "url:https://example.com/cb".into(),
        metadata: json!({}),
    }
}

#[tokio::test]
async fn delivery_event_store_crud() {
    let pool = db::open(":memory:").await.unwrap();
    let (task_id, did) = seed_task_and_deliverable(&pool).await;
    let store = DeliveryEventStore::new(pool);

    let ok_id = store
        .insert(NewDeliveryEvent {
            tenant_id: None,
            task_id: task_id.clone(),
            deliverable_id: did.clone(),
            channel: "webhook".into(),
            target: target("webhook"),
            ok: true,
            external_id: Some("ext-1".into()),
            error: None,
            delivered_at: 100,
        })
        .await
        .expect("insert ok");

    let fail_id = store
        .insert(NewDeliveryEvent {
            tenant_id: None,
            task_id: task_id.clone(),
            deliverable_id: did.clone(),
            channel: "webhook".into(),
            target: target("webhook"),
            ok: false,
            external_id: None,
            error: Some("connection refused".into()),
            delivered_at: 200,
        })
        .await
        .expect("insert fail");

    let by_task = store.list_by_task(&task_id).await.expect("by task");
    assert_eq!(by_task.len(), 2);
    assert_eq!(by_task[0].id, ok_id);
    assert!(by_task[0].ok);
    assert_eq!(by_task[0].external_id.as_deref(), Some("ext-1"));
    assert_eq!(by_task[1].id, fail_id);
    assert!(!by_task[1].ok);
    assert_eq!(by_task[1].error.as_deref(), Some("connection refused"));

    let by_deliv = store.list_by_deliverable(&did).await.expect("by deliv");
    assert_eq!(by_deliv.len(), 2);
    assert_eq!(by_deliv[0].deliverable_id, did);
}
