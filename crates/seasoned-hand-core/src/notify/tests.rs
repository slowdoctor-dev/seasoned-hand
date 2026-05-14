use serde_json::json;

use super::{NewNotificationSent, NotificationsSentStore};
use crate::channel::NotifyTarget;
use crate::db;
use crate::project::{NewProject, NewTask, ProjectStore, TaskStore};

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

#[tokio::test]
async fn notifications_sent_store_crud() {
    let pool = db::open(":memory:").await.unwrap();
    let task_id = seed_task(&pool).await;
    let store = NotificationsSentStore::new(pool);

    let task_finished = store
        .insert(NewNotificationSent {
            tenant_id: None,
            task_id: Some(task_id.clone()),
            trigger_kind: "task_finished".into(),
            channel: "ntfy".into(),
            target: Some(NotifyTarget {
                channel: "ntfy".into(),
                target_ref: "topic:seasoned-hand".into(),
                metadata: json!({}),
            }),
            payload: Some(json!({ "title": "Done" })),
            ok: true,
            error: None,
            sent_at: 100,
        })
        .await
        .expect("insert task_finished");

    // Pre-task notify (briefing escalation, no task_id).
    let _pre_task = store
        .insert(NewNotificationSent {
            tenant_id: None,
            task_id: None,
            trigger_kind: "briefing_escalation".into(),
            channel: "email".into(),
            target: None,
            payload: None,
            ok: false,
            error: Some("smtp timeout".into()),
            sent_at: 50,
        })
        .await
        .expect("insert pre-task");

    let by_task = store.list_by_task(&task_id).await.expect("list by task");
    assert_eq!(by_task.len(), 1);
    assert_eq!(by_task[0].id, task_finished);
    assert_eq!(by_task[0].trigger_kind, "task_finished");
    assert!(by_task[0].ok);
    assert_eq!(by_task[0].channel, "ntfy");
    assert_eq!(by_task[0].payload, Some(json!({ "title": "Done" })));
    assert_eq!(
        by_task[0].target.as_ref().unwrap().target_ref,
        "topic:seasoned-hand"
    );

    // Pre-task notify did NOT come back under the task filter.
    let unknown_task = store.list_by_task("nope").await.expect("list nope");
    assert!(unknown_task.is_empty());
}
