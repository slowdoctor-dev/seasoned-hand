use std::sync::Arc;

use serde_json::{Value, json};

use super::{ToolContext, register_builtin_tools};
use crate::db;
use crate::events::sqlite::SqliteEventStore;
use crate::events::{EventQuery, EventStore};

async fn ctx() -> (super::ToolContext, Arc<SqliteEventStore>) {
    let pool = db::open(":memory:").await.unwrap();
    pool.with_conn(|conn| {
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, state) \
             VALUES ('s1', 1, 1, 'RUNNING')",
            [],
        )
        .unwrap();
    })
    .await;
    let store = Arc::new(SqliteEventStore::new(pool));
    let ctx = ToolContext {
        session_id: "s1".into(),
        events: store.clone(),
    };
    (ctx, store)
}

#[test]
fn registry_has_five_entries() {
    let reg = register_builtin_tools();
    assert_eq!(reg.len(), 5);
    for name in [
        "message_notify_user",
        "message_ask_user",
        "idle",
        "sop_read",
        "glossary_lookup",
    ] {
        assert!(reg.contains_key(name), "missing {name}");
    }
}

#[test]
fn all_schemas_are_objects() {
    let reg = register_builtin_tools();
    for (name, tool) in reg.iter() {
        let schema = tool.schema();
        assert_eq!(schema["type"], "object", "{name} schema must be object");
        assert!(
            schema["properties"].is_object(),
            "{name} missing properties"
        );
    }
}

#[tokio::test]
async fn message_notify_user_rejects_missing_content() {
    let (cx, _store) = ctx().await;
    let reg = register_builtin_tools();
    let tool = reg.get("message_notify_user").unwrap();
    let err = tool.invoke(json!({}), &cx).await.unwrap_err();
    matches!(err, super::ToolError::InvalidArgs(_));
}

#[tokio::test]
async fn message_notify_user_emits_message_event() {
    let (cx, store) = ctx().await;
    let reg = register_builtin_tools();
    let tool = reg.get("message_notify_user").unwrap();
    let out = tool.invoke(json!({"content": "hello"}), &cx).await.unwrap();
    assert!(out.ok);

    let events = store.query("s1", EventQuery::default()).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, crate::events::EventType::Message);
    assert_eq!(events[0].source, "tool:message_notify_user");
    assert_eq!(events[0].data["ui"], "notify");
    assert_eq!(events[0].data["content"], "hello");
}

#[tokio::test]
async fn message_ask_user_emits_ask_event() {
    let (cx, store) = ctx().await;
    let reg = register_builtin_tools();
    let tool = reg.get("message_ask_user").unwrap();
    let out = tool.invoke(json!({"content": "what?"}), &cx).await.unwrap();
    assert!(out.ok);
    assert!(out.output["call_id"].is_i64());

    let events = store.query("s1", EventQuery::default()).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data["ui"], "ask");
}

#[tokio::test]
async fn idle_returns_task_complete_signal() {
    let (cx, _store) = ctx().await;
    let reg = register_builtin_tools();
    let tool = reg.get("idle").unwrap();
    let out = tool.invoke(Value::Null, &cx).await.unwrap();
    assert!(out.ok);
    assert_eq!(out.output["signal"], "task_complete");
}

#[tokio::test]
async fn sop_read_is_not_implemented() {
    let (cx, _store) = ctx().await;
    let reg = register_builtin_tools();
    let tool = reg.get("sop_read").unwrap();
    let out = tool.invoke(json!({"id": "anything"}), &cx).await.unwrap();
    assert!(!out.ok);
    assert_eq!(out.error.as_ref().unwrap().kind, "not_implemented");
}

#[tokio::test]
async fn glossary_lookup_is_not_implemented() {
    let (cx, _store) = ctx().await;
    let reg = register_builtin_tools();
    let tool = reg.get("glossary_lookup").unwrap();
    let out = tool.invoke(json!({"term": "X"}), &cx).await.unwrap();
    assert!(!out.ok);
    assert_eq!(out.error.as_ref().unwrap().kind, "not_implemented");
}
