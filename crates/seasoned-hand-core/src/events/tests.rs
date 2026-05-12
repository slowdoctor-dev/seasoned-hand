use serde_json::json;

use super::sqlite::SqliteEventStore;
use super::{EventQuery, EventStore, EventType, NewEvent};
use crate::db;

async fn fixture() -> (db::DbPool, SqliteEventStore) {
    let pool = db::open(":memory:").await.unwrap();
    pool.with_conn(|conn| {
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, state) \
             VALUES (?, ?, ?, 'RUNNING')",
            rusqlite::params!["s1", 1i64, 1i64],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, state) \
             VALUES (?, ?, ?, 'RUNNING')",
            rusqlite::params!["s2", 1i64, 1i64],
        )
        .unwrap();
    })
    .await;
    let store = SqliteEventStore::new(pool.clone());
    (pool, store)
}

#[tokio::test]
async fn append_then_query_returns_event() {
    let (_pool, store) = fixture().await;
    let appended = store
        .append(NewEvent {
            session_id: "s1".into(),
            event_type: EventType::Message,
            source: "user".into(),
            data: json!({"content": "hi"}),
        })
        .await
        .unwrap();

    let events = store.query("s1", EventQuery::default()).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].id, appended.id);
    assert_eq!(events[0].event_type, EventType::Message);
    assert_eq!(events[0].source, "user");
    assert_eq!(events[0].data, json!({"content": "hi"}));
}

#[tokio::test]
async fn query_filters_by_type() {
    let (_pool, store) = fixture().await;
    for t in [EventType::Message, EventType::Action, EventType::Message] {
        store
            .append(NewEvent {
                session_id: "s1".into(),
                event_type: t,
                source: "x".into(),
                data: json!({}),
            })
            .await
            .unwrap();
    }
    let messages = store
        .query(
            "s1",
            EventQuery {
                event_type: Some(EventType::Message),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(messages.len(), 2);
}

#[tokio::test]
async fn query_filters_by_after_id() {
    let (_pool, store) = fixture().await;
    let first = store
        .append(NewEvent {
            session_id: "s1".into(),
            event_type: EventType::Misc,
            source: "x".into(),
            data: json!({}),
        })
        .await
        .unwrap();
    let _second = store
        .append(NewEvent {
            session_id: "s1".into(),
            event_type: EventType::Misc,
            source: "x".into(),
            data: json!({}),
        })
        .await
        .unwrap();

    let after = store
        .query(
            "s1",
            EventQuery {
                after_id: Some(first.id),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(after.len(), 1);
    assert!(after[0].id > first.id);
}

#[tokio::test]
async fn query_respects_limit() {
    let (_pool, store) = fixture().await;
    for _ in 0..5 {
        store
            .append(NewEvent {
                session_id: "s1".into(),
                event_type: EventType::Misc,
                source: "x".into(),
                data: json!({}),
            })
            .await
            .unwrap();
    }
    let limited = store
        .query(
            "s1",
            EventQuery {
                limit: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(limited.len(), 2);
}

#[tokio::test]
async fn append_fails_for_unknown_session() {
    let (_pool, store) = fixture().await;
    let err = store
        .append(NewEvent {
            session_id: "nope".into(),
            event_type: EventType::Misc,
            source: "x".into(),
            data: json!({}),
        })
        .await
        .unwrap_err();
    matches!(err, super::EventError::SessionNotFound(_));
}

#[tokio::test]
async fn data_payload_survives_roundtrip() {
    let (_pool, store) = fixture().await;
    let payload = json!({
        "nested": {"a": 1, "b": [true, null, "x"]},
        "list": [{"k": "v"}, 42],
    });
    store
        .append(NewEvent {
            session_id: "s1".into(),
            event_type: EventType::Observation,
            source: "tool:test".into(),
            data: payload.clone(),
        })
        .await
        .unwrap();
    let got = store.query("s1", EventQuery::default()).await.unwrap();
    assert_eq!(got[0].data, payload);
}

#[tokio::test]
async fn events_for_different_sessions_are_isolated() {
    let (_pool, store) = fixture().await;
    for sid in ["s1", "s1", "s2"] {
        store
            .append(NewEvent {
                session_id: sid.into(),
                event_type: EventType::Misc,
                source: "x".into(),
                data: json!({}),
            })
            .await
            .unwrap();
    }
    let s1 = store.query("s1", EventQuery::default()).await.unwrap();
    let s2 = store.query("s2", EventQuery::default()).await.unwrap();
    assert_eq!(s1.len(), 2);
    assert_eq!(s2.len(), 1);
}
