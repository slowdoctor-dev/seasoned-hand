use std::sync::Arc;

use serde_json::{Value, json};

use super::{CHANNEL_NAME, ChatChannel, TARGET_SESSION_PREFIX};
use crate::channel::{
    ChannelRegistration, ChannelRegistry, Deliverable, DeliverySink, DeliveryTarget,
};
use crate::db;
use crate::events::{EventQuery, EventStore, sqlite::SqliteEventStore};

async fn setup_store() -> (Arc<SqliteEventStore>, crate::db::DbPool) {
    let pool = db::open(":memory:").await.expect("db open");
    pool.with_conn(|conn| {
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, state) \
             VALUES ('chat-session-1', 0, 0, 'RUNNING')",
            [],
        )
        .expect("seed session");
    })
    .await;
    let store = Arc::new(SqliteEventStore::new(pool.clone()));
    (store, pool)
}

fn sample_deliverable() -> Deliverable {
    Deliverable {
        id: "deliv-1".into(),
        task_id: "task-1".into(),
        tenant_id: None,
        format: "md".into(),
        source_content_path: None,
        source_content_sha256: None,
        rendered_content_path: "/workspace/.deliverables/deliv-1.md".into(),
        rendered_content_sha256: "abc123".into(),
        content_size: 42,
        citations: Some(vec![17, 99]),
        provenance_manifest: json!({}),
        created_at: 0,
    }
}

#[tokio::test]
async fn chat_channel_deliver_emits_ws_event() {
    let (events, pool) = setup_store().await;
    let chat = ChatChannel::new(events.clone());

    let target = DeliveryTarget {
        channel: CHANNEL_NAME.into(),
        target_ref: format!("{TARGET_SESSION_PREFIX}chat-session-1"),
        metadata: json!({}),
    };
    let deliverable = sample_deliverable();

    let receipt = chat
        .deliver(&target, &deliverable)
        .await
        .expect("delivery ok");
    assert_eq!(receipt.channel, CHANNEL_NAME);
    assert!(!receipt.external_id.is_empty());

    // The Misc event must land on the session — that's how the existing
    // Redis pubsub → WS subscription pipeline surfaces it to the client.
    let evts = events
        .query("chat-session-1", EventQuery::default())
        .await
        .expect("query events");
    assert_eq!(evts.len(), 1, "exactly one event written");
    let evt = &evts[0];
    assert_eq!(evt.event_type.as_str(), "Misc");
    assert_eq!(evt.source, "channel:chat");
    assert_eq!(evt.data["kind"], Value::from("Deliverable"));
    assert_eq!(evt.data["deliverable_id"], Value::from("deliv-1"));
    assert_eq!(evt.data["format"], Value::from("md"));
    assert_eq!(
        evt.data["file_ref"],
        Value::from("/workspace/.deliverables/deliv-1.md")
    );
    assert_eq!(evt.data["citations"], json!([17, 99]));

    drop(pool);
}

#[tokio::test]
async fn chat_channel_deliver_handles_missing_citations() {
    let (events, _pool) = setup_store().await;
    let chat = ChatChannel::new(events.clone());

    let target = DeliveryTarget {
        channel: CHANNEL_NAME.into(),
        target_ref: format!("{TARGET_SESSION_PREFIX}chat-session-1"),
        metadata: json!({}),
    };
    let mut deliverable = sample_deliverable();
    deliverable.citations = None;

    chat.deliver(&target, &deliverable)
        .await
        .expect("delivery ok");

    let evts = events
        .query("chat-session-1", EventQuery::default())
        .await
        .expect("query events");
    assert_eq!(evts[0].data["citations"], json!([]));
}

#[tokio::test]
async fn chat_channel_deliver_rejects_malformed_target_ref() {
    let (events, _pool) = setup_store().await;
    let chat = ChatChannel::new(events.clone());

    let target = DeliveryTarget {
        channel: CHANNEL_NAME.into(),
        target_ref: "thread:not-a-session".into(),
        metadata: json!({}),
    };
    let err = chat
        .deliver(&target, &sample_deliverable())
        .await
        .expect_err("must reject");
    assert!(
        err.to_string().contains("target_ref"),
        "error should mention target_ref: {err}"
    );
}

#[tokio::test]
async fn chat_channel_no_notify_role() {
    let (events, _pool) = setup_store().await;
    let chat = Arc::new(ChatChannel::new(events));
    let mut registry = ChannelRegistry::new();
    registry.register(
        ChannelRegistration::new(CHANNEL_NAME)
            .with_intake(chat.clone())
            .with_delivery(chat),
    );

    assert!(
        registry.get_intake(CHANNEL_NAME).is_some(),
        "intake role registered"
    );
    assert!(
        registry.get_delivery(CHANNEL_NAME).is_some(),
        "delivery role registered"
    );
    assert!(
        registry.get_notify(CHANNEL_NAME).is_none(),
        "notify role NOT registered — chat has no push-notify semantics"
    );

    let health = registry.health();
    let chat_health = health
        .iter()
        .find(|h| h.name == CHANNEL_NAME)
        .expect("chat present in health snapshot");
    assert_eq!(chat_health.capabilities, vec!["intake", "delivery"]);
}
