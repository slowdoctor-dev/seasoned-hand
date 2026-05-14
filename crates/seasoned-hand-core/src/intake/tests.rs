use serde_json::json;

use super::{IntakeEventStore, IntakeStoreError};
use crate::channel::{DeliveryTarget, IntakeEvent};
use crate::db;
use crate::project::{NewProject, NewTask, ProjectStore, TaskStore};

async fn open_pool() -> db::DbPool {
    db::open(":memory:").await.expect("open in-memory db")
}

fn sample_event(channel: &str, intake_id: &str, received_at: i64) -> IntakeEvent {
    IntakeEvent {
        channel: channel.into(),
        intake_id: intake_id.into(),
        brief_input: "Summarize Q4 board deck".into(),
        reply_target: Some(DeliveryTarget {
            channel: "email".into(),
            target_ref: "msgid:<abc@example.com>".into(),
            metadata: json!({ "to": "user@example.com" }),
        }),
        metadata: json!({ "subject": "Board deck" }),
        tenant_id: None,
        received_at,
    }
}

#[tokio::test]
async fn intake_event_store_crud() {
    let pool = open_pool().await;
    let store = IntakeEventStore::new(pool);

    let evt = sample_event("webhook", "req-001", 1_000_000);
    let id = store.insert(&evt).await.expect("insert");

    let row = store
        .get_by_intake_id("webhook", "req-001")
        .await
        .expect("get")
        .expect("row present");
    assert_eq!(row.id, id);
    assert_eq!(row.channel, "webhook");
    assert_eq!(row.intake_id, "req-001");
    assert_eq!(row.brief_input, "Summarize Q4 board deck");
    assert!(row.task_id.is_none());
    let target = row.reply_target.as_ref().unwrap();
    assert_eq!(target.channel, "email");
    assert_eq!(target.target_ref, "msgid:<abc@example.com>");

    // Unknown lookup is None, not an error.
    assert!(
        store
            .get_by_intake_id("webhook", "nope")
            .await
            .expect("get nope")
            .is_none()
    );
}

#[tokio::test]
async fn intake_event_unique_channel_intake_id() {
    let pool = open_pool().await;
    let store = IntakeEventStore::new(pool);

    let evt = sample_event("webhook", "dup-1", 1);
    store.insert(&evt).await.expect("first insert");

    let err = store.insert(&evt).await.expect_err("duplicate must fail");
    // refinery + rusqlite surface UNIQUE violation as Sqlite error;
    // exact variant depends on rusqlite version. Match by name.
    let msg = format!("{err}");
    assert!(
        msg.contains("UNIQUE") || msg.contains("unique"),
        "expected UNIQUE constraint error, got: {msg}"
    );
}

#[tokio::test]
async fn intake_event_link_to_task() {
    let pool = open_pool().await;
    let projects = ProjectStore::new(pool.clone());
    let tasks = TaskStore::new(pool.clone());
    let store = IntakeEventStore::new(pool);

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

    let id = store
        .insert(&sample_event("webhook", "link-1", 2))
        .await
        .expect("insert");
    store.link_to_task(&id, &task_id).await.expect("link");

    let row = store
        .get_by_intake_id("webhook", "link-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.task_id.as_deref(), Some(task_id.as_str()));

    // Unknown intake-event id is NotFound.
    let err = store
        .link_to_task("nope", &task_id)
        .await
        .expect_err("not found");
    assert!(matches!(err, IntakeStoreError::NotFound(_)));
}

#[tokio::test]
async fn intake_event_list_by_channel_paginates() {
    let pool = open_pool().await;
    let store = IntakeEventStore::new(pool);

    // Insert 5 webhook events + 1 email event; only webhook events
    // should come back from list_by_channel("webhook").
    for i in 0..5_i64 {
        store
            .insert(&sample_event("webhook", &format!("rw-{i}"), 100 + i))
            .await
            .unwrap();
    }
    store
        .insert(&sample_event("email", "re-0", 999))
        .await
        .unwrap();

    let page = store
        .list_by_channel("webhook", None, 50)
        .await
        .expect("list webhook");
    assert_eq!(page.len(), 5);
    // Newest first.
    assert_eq!(page.first().unwrap().received_at, 104);
    assert_eq!(page.last().unwrap().received_at, 100);

    // Cursor: only rows strictly older than 102.
    let older = store
        .list_by_channel("webhook", Some(102), 50)
        .await
        .expect("list cursor");
    assert_eq!(older.len(), 2);
    assert_eq!(older.first().unwrap().received_at, 101);
    assert_eq!(older.last().unwrap().received_at, 100);

    // Different channel isolated.
    let emails = store
        .list_by_channel("email", None, 50)
        .await
        .expect("list email");
    assert_eq!(emails.len(), 1);
}
