use std::sync::Arc;

use tempfile::tempdir;

use crate::agent::init::{Initializer, derive_feature_list_for_test};
use crate::db;
use crate::events::{EventQuery, EventStore, EventType, sqlite::SqliteEventStore};
use crate::plan::{Phase, PhaseStatus, Plan};
use crate::pubsub::RedisPool;
use crate::router::SlotRouter;
use crate::sandbox::{SandboxClient, SandboxHandle};

#[tokio::test]
async fn initializer_writes_feature_list_and_progress() {
    let db = db::open(":memory:").await.expect("db");
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, state) VALUES ('s1',0,0,'RUNNING')",
            [],
        )
        .unwrap();
    })
    .await;
    let redis = RedisPool::new("redis://127.0.0.1:6379").expect("redis");
    let events = Arc::new(SqliteEventStore::with_redis(db.clone(), redis));
    let plan_manager = Arc::new(crate::plan::PlanManager::new(db, events.clone()));

    let router = Arc::new(SlotRouter::default_for_bifrost());

    let ws = tempdir().expect("tmp");
    let sandbox = Arc::new(
        SandboxClient::new("ghcr.io/agent-infra/sandbox:1.0.0.152", ws.path()).expect("sandbox"),
    );
    sandbox
        .insert_handle_for_test(SandboxHandle {
            session_id: "s1".into(),
            container_id: "c1".into(),
            api_url: "http://127.0.0.1:1".into(),
            novnc_url: "http://127.0.0.1:2".into(),
            ttyd_url: "ws://127.0.0.1:3".into(),
            workspace_host_path: ws.path().join("s1"),
        })
        .await;

    let init = Initializer::new(router, plan_manager, sandbox.clone(), events);
    let report = init
        .run_with_planner_output_for_test(
            "s1",
            "do the thing",
            super::PlannerOutput {
                goal: "Build".into(),
                phases: vec![
                    super::PlannerPhase {
                        id: 1,
                        title: "Plan".into(),
                        capabilities: vec![],
                    },
                    super::PlannerPhase {
                        id: 2,
                        title: "Code".into(),
                        capabilities: vec![],
                    },
                    super::PlannerPhase {
                        id: 3,
                        title: "Verify".into(),
                        capabilities: vec![],
                    },
                ],
            },
        )
        .await
        .expect("init run");
    assert_eq!(report.feature_count, 3);

    let fl = sandbox
        .read_workspace_file("s1", "feature-list.json")
        .await
        .expect("feature list");
    let text = String::from_utf8_lossy(&fl);
    assert!(text.contains("\"features\""));

    let pg = sandbox
        .read_workspace_file("s1", "progress.txt")
        .await
        .expect("progress");
    let pg = String::from_utf8_lossy(&pg);
    assert!(pg.contains("Goal:"));
    assert!(pg.contains("Phase 1"));
}

#[tokio::test]
async fn initializer_falls_back_on_zero_phase_plan() {
    let db = db::open(":memory:").await.expect("db");
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, state) VALUES ('s1',0,0,'RUNNING')",
            [],
        )
        .unwrap();
    })
    .await;
    let redis = RedisPool::new("redis://127.0.0.1:6379").expect("redis");
    let events = Arc::new(SqliteEventStore::with_redis(db.clone(), redis));
    let plan_manager = Arc::new(crate::plan::PlanManager::new(db, events.clone()));
    let router = Arc::new(SlotRouter::default_for_bifrost());

    let ws = tempdir().expect("tmp");
    let sandbox = Arc::new(
        SandboxClient::new("ghcr.io/agent-infra/sandbox:1.0.0.152", ws.path()).expect("sandbox"),
    );
    sandbox
        .insert_handle_for_test(SandboxHandle {
            session_id: "s1".into(),
            container_id: "c1".into(),
            api_url: "http://127.0.0.1:1".into(),
            novnc_url: "http://127.0.0.1:2".into(),
            ttyd_url: "ws://127.0.0.1:3".into(),
            workspace_host_path: ws.path().join("s1"),
        })
        .await;

    let init = Initializer::new(router, plan_manager, sandbox, events.clone());
    let report = init
        .run_with_planner_output_for_test(
            "s1",
            "fallback me",
            super::PlannerOutput {
                goal: "Oops".into(),
                phases: vec![],
            },
        )
        .await
        .expect("init run");
    assert_eq!(report.feature_count, 1);

    let misc = events
        .query("s1", EventQuery::default())
        .await
        .expect("events");
    assert!(misc.iter().any(|e| {
        e.event_type == EventType::Misc
            && e.data.get("kind").and_then(|v| v.as_str()) == Some("init_planner_fallback")
    }));
}

#[test]
fn derive_feature_list_marks_active_phase_doing() {
    let plan = Plan {
        id: "p1".into(),
        session_id: "s1".into(),
        goal: "Goal".into(),
        phases: vec![
            Phase {
                id: 1,
                title: "A".into(),
                capabilities: vec![],
                status: PhaseStatus::Done,
            },
            Phase {
                id: 2,
                title: "B".into(),
                capabilities: vec![],
                status: PhaseStatus::Active,
            },
        ],
        current_phase_id: Some(2),
    };
    let list = derive_feature_list_for_test(&plan);
    assert_eq!(list.version, 1);
    assert_eq!(list.features.len(), 2);
    assert!(matches!(
        list.features[0].status,
        crate::agent::init::feature_list::FeatureStatus::Done
    ));
    assert!(matches!(
        list.features[1].status,
        crate::agent::init::feature_list::FeatureStatus::Doing
    ));
}
