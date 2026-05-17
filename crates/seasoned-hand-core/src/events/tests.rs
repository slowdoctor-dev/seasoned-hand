use serde_json::json;

use super::sqlite::SqliteEventStore;
use super::{EventQuery, EventStore, EventType, NewEvent};
use crate::db;
use crate::pubsub::RedisPool;

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
async fn append_succeeds_even_if_redis_unreachable() {
    // Pre-PRINCIPLE-#10: publish failure logs but never fails append.
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
    let redis = RedisPool::new("redis://127.0.0.1:6").unwrap(); // unreachable
    let store = SqliteEventStore::with_redis(pool.clone(), redis);
    let appended = store
        .append(NewEvent {
            session_id: "s1".into(),
            event_type: EventType::Misc,
            source: "test".into(),
            data: json!({"ok": true}),
        })
        .await
        .expect("append must succeed regardless of redis state");
    let queried = store.query("s1", EventQuery::default()).await.unwrap();
    assert_eq!(queried.len(), 1);
    assert_eq!(queried[0].id, appended.id);
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

mod session_search {
    use serde_json::json;

    use crate::events::session_search::{
        EventHit, SessionSearchQuery, search_session_events, summarize_hits_with_fallback,
    };
    use crate::router::SlotRouter;

    use super::*;

    #[tokio::test]
    async fn index_ingestion() {
        let (pool, store) = fixture().await;
        let appended = store
            .append(NewEvent {
                session_id: "s1".into(),
                event_type: EventType::Action,
                source: "tool:search".into(),
                data: json!({
                    "tool_name": "web_search",
                    "tool_input": {"query": "knee pain protocol", "limit": 3}
                }),
            })
            .await
            .unwrap();

        pool.with_conn(|conn| {
            let row: (i64, String, String, String) = conn
                .query_row(
                    "SELECT event_id, session_id, event_type, searchable_text
                     FROM session_search_index WHERE event_id = ?",
                    [appended.id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .unwrap();
            assert_eq!(row.0, appended.id);
            assert_eq!(row.1, "s1");
            assert_eq!(row.2, "Action");
            assert!(row.3.contains("web_search"));
            assert!(row.3.contains("knee pain protocol"));
        })
        .await;
    }

    #[tokio::test]
    async fn all_event_types_queryable() {
        let (pool, store) = fixture().await;
        let all = [
            (EventType::Message, "agent", json!({"role": "assistant", "text": "needle_alpha"})),
            (EventType::Action, "tool:x", json!({"tool_name": "needle_alpha", "tool_input": {"x": "needle_alpha"}})),
            (EventType::Observation, "tool:x", json!({"tool_name": "needle_alpha", "tool_result": "needle_alpha"})),
            (EventType::Plan, "planner", json!({"goal": "needle_alpha", "phases": [{"title": "needle_alpha"}]})),
            (EventType::Knowledge, "memory", json!({"fact": "needle_alpha"})),
            (EventType::Datasource, "web", json!({"url": "needle_alpha"})),
            (EventType::Skill, "learning", json!({"kind": "match", "playbook_id": "needle_alpha", "matcher_mode": "production"})),
            (EventType::Misc, "system", json!({"kind": "playbook_extraction_rejected", "reason": "needle_alpha"})),
        ];

        for (event_type, source, data) in all {
            store
                .append(NewEvent {
                    session_id: "s1".into(),
                    event_type,
                    source: source.into(),
                    data,
                })
                .await
                .unwrap();
        }

        let hits = pool
            .with_conn(|conn| {
                search_session_events(
                    conn,
                    "needle_alpha",
                    &SessionSearchQuery {
                        session_id: Some("s1".into()),
                        limit: Some(20),
                        ..Default::default()
                    },
                )
            })
            .await
            .unwrap();

        assert_eq!(hits.len(), 8);
        let mut types = hits.into_iter().map(|h| h.event_type).collect::<Vec<_>>();
        types.sort();
        assert_eq!(
            types,
            vec![
                "Action",
                "Datasource",
                "Knowledge",
                "Message",
                "Misc",
                "Observation",
                "Plan",
                "Skill"
            ]
        );
    }

    #[tokio::test]
    async fn summary_fallback() {
        let (_pool, store) = fixture().await;
        let hits = vec![EventHit {
            event_id: 1,
            session_id: "s1".into(),
            timestamp: 1,
            event_type: "Action".into(),
            source: "tool:web".into(),
            snippet: "needle_alpha snippet".into(),
        }];
        let router = SlotRouter::default_for_bifrost();
        let out = summarize_hits_with_fallback(&store, &router, "s1", "needle_alpha", &hits).await;
        assert!(out.degraded);
        assert!(out.summary.contains("raw hits returned"));

        let events = store.query("s1", EventQuery::default()).await.unwrap();
        let degraded = events.into_iter().any(|e| {
            e.event_type == EventType::Misc
                && e.data.get("kind").and_then(|v| v.as_str())
                    == Some("session_search_summary_degraded")
        });
        assert!(degraded);
    }
}
