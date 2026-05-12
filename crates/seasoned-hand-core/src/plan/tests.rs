use std::sync::Arc;

use super::{Phase, PhaseStatus, Plan, PlanManager, PlanMutationSource};
use crate::db;
use crate::events::{EventQuery, EventStore, EventType, sqlite::SqliteEventStore};
use crate::plan::render::{estimate_tokens, sticky_render};
use crate::pubsub::RedisPool;

async fn harness() -> (PlanManager, Arc<SqliteEventStore>) {
    let db = db::open(":memory:").await.expect("db open");
    db.with_conn(|conn| {
        conn.execute("INSERT INTO sessions (id, created_at, updated_at, state) VALUES ('s1', 0, 0, 'RUNNING')", []).expect("session");
    }).await;
    let redis = RedisPool::new("redis://127.0.0.1:6379").expect("redis parse");
    let events = Arc::new(SqliteEventStore::with_redis(db.clone(), redis));
    (PlanManager::new(db, events.clone()), events)
}

fn base_phases() -> Vec<Phase> {
    vec![
        Phase {
            id: 1,
            title: "One".into(),
            capabilities: vec![],
            status: PhaseStatus::Pending,
        },
        Phase {
            id: 2,
            title: "Two".into(),
            capabilities: vec![],
            status: PhaseStatus::Pending,
        },
        Phase {
            id: 3,
            title: "Three".into(),
            capabilities: vec![],
            status: PhaseStatus::Pending,
        },
    ]
}

#[tokio::test]
async fn plan_create_inserts_row_emits_event() {
    let (manager, events) = harness().await;
    let created = manager
        .create("s1", "Ship", base_phases())
        .await
        .expect("create");
    assert_eq!(created.current_phase_id, Some(1));
    let plan_events = events
        .query(
            "s1",
            EventQuery {
                event_type: Some(EventType::Plan),
                ..Default::default()
            },
        )
        .await
        .expect("events");
    assert_eq!(plan_events.len(), 1);
    assert_eq!(plan_events[0].data["op"], "create");
}

#[tokio::test]
async fn plan_advance_auto_picks_next_pending() {
    let (manager, _) = harness().await;
    manager
        .create("s1", "Ship", base_phases())
        .await
        .expect("create");
    let first = manager.advance("s1").await.expect("advance1");
    assert_eq!(first.current_phase_id, Some(2));
    assert_eq!(first.phases[2].status, PhaseStatus::Pending);
    let second = manager.advance("s1").await.expect("advance2");
    assert_eq!(second.current_phase_id, Some(3));
}

#[tokio::test]
async fn plan_update_replaces_phases_and_resets_current() {
    let (manager, _) = harness().await;
    manager
        .create("s1", "Ship", base_phases())
        .await
        .expect("create");
    let updated = manager
        .update(
            "s1",
            vec![
                Phase {
                    id: 10,
                    title: "done".into(),
                    capabilities: vec![],
                    status: PhaseStatus::Done,
                },
                Phase {
                    id: 11,
                    title: "pending".into(),
                    capabilities: vec![],
                    status: PhaseStatus::Pending,
                },
                Phase {
                    id: 12,
                    title: "pending2".into(),
                    capabilities: vec![],
                    status: PhaseStatus::Pending,
                },
            ],
            PlanMutationSource::Agent,
        )
        .await
        .expect("update");
    assert_eq!(updated.current_phase_id, Some(11));
}

#[tokio::test]
async fn plan_update_tags_source_in_event_data() {
    let (manager, events) = harness().await;
    manager
        .create("s1", "Ship", base_phases())
        .await
        .expect("create");
    manager
        .update("s1", base_phases(), PlanMutationSource::Verifier)
        .await
        .expect("update");
    let plan_events = events
        .query(
            "s1",
            EventQuery {
                event_type: Some(EventType::Plan),
                ..Default::default()
            },
        )
        .await
        .expect("events");
    let update = plan_events
        .into_iter()
        .find(|e| e.data["op"] == "update")
        .expect("update event");
    assert_eq!(update.data["source"], "verifier");
}

#[test]
fn sticky_render_under_1000_tokens_long_titles() {
    let long = "long title ".repeat(200);
    let plan = Plan {
        id: "p1".into(),
        session_id: "s1".into(),
        goal: "Goal".into(),
        phases: (1..=20)
            .map(|id| Phase {
                id,
                title: format!("{} {}", long, id),
                capabilities: vec![],
                status: if id == 1 {
                    PhaseStatus::Active
                } else {
                    PhaseStatus::Pending
                },
            })
            .collect(),
        current_phase_id: Some(1),
    };
    let rendered = sticky_render(&plan, 1000);
    assert!(rendered.contains("== PLAN =="));
    assert!(rendered.contains("== END PLAN =="));
    assert_eq!(rendered.matches("Phase ").count(), 20);
    assert!(
        estimate_tokens(&rendered) <= 1000,
        "{}",
        estimate_tokens(&rendered)
    );
}
