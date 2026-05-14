use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tempfile::TempDir;
use tempfile::tempdir;
use tokio::sync::mpsc;

use crate::agent::init::briefing::{
    BriefingAction, PartialBrief, RunConfig, RunOutcome, UserResponse,
};
use crate::agent::init::{InitError, Initializer, derive_feature_list_for_test};
use crate::db::{self, DbPool};
use crate::events::{EventQuery, EventStore, EventType, sqlite::SqliteEventStore};
use crate::plan::{Phase, PhaseStatus, Plan, PlanManager};
use crate::project::brief::{Brief, BriefError, BriefPhase};
use crate::project::project::ProjectStore;
use crate::project::task::{NewTask, TaskStore};
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

// =====================================================================
// Story 2.8 — run_with_confirmation (Briefing + confirm gate)
// =====================================================================

struct ConfirmFixture {
    init: Initializer,
    events: Arc<SqliteEventStore>,
    task_store: Arc<TaskStore>,
    db: DbPool,
    session_id: String,
    task_id: String,
    // Retained so the workspace lives at least as long as the test.
    _workspace: TempDir,
}

async fn setup_confirm_fixture() -> ConfirmFixture {
    let db = db::open(":memory:").await.expect("db");
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, state) VALUES ('s1',0,0,'RUNNING')",
            [],
        )
        .unwrap();
    })
    .await;
    let redis = RedisPool::new("redis://127.0.0.1:6").expect("redis");
    let events = Arc::new(SqliteEventStore::with_redis(db.clone(), redis));
    let plan_manager = Arc::new(PlanManager::new(db.clone(), events.clone()));
    let router = Arc::new(SlotRouter::default_for_bifrost());
    let workspace = tempdir().expect("workspace");
    let sandbox = Arc::new(
        SandboxClient::new("ghcr.io/agent-infra/sandbox:1.0.0.152", workspace.path())
            .expect("sandbox"),
    );
    sandbox
        .insert_handle_for_test(SandboxHandle {
            session_id: "s1".into(),
            container_id: "c1".into(),
            api_url: "http://127.0.0.1:1".into(),
            novnc_url: "http://127.0.0.1:2".into(),
            ttyd_url: "ws://127.0.0.1:3".into(),
            workspace_host_path: workspace.path().join("s1"),
        })
        .await;
    let project_store = Arc::new(ProjectStore::new(db.clone()));
    let task_store = Arc::new(TaskStore::new(db.clone()));
    let project_id = project_store
        .find_or_create_inbox(None)
        .await
        .expect("inbox");
    let task_id = task_store
        .insert(NewTask {
            project_id,
            tenant_id: None,
            title: "test task".into(),
            expected_due_at: None,
        })
        .await
        .expect("task insert");
    let init = Initializer::new(router, plan_manager, sandbox, events.clone())
        .with_task_store(task_store.clone());
    ConfirmFixture {
        init,
        events,
        task_store,
        db,
        session_id: "s1".into(),
        task_id,
        _workspace: workspace,
    }
}

fn sample_brief() -> Brief {
    Brief {
        goal: "Build a thing".into(),
        phases: vec![
            BriefPhase {
                id: 1,
                title: "Plan".into(),
                capabilities: vec![],
            },
            BriefPhase {
                id: 2,
                title: "Build".into(),
                capabilities: vec![],
            },
        ],
        success_criteria: vec!["Works".into()],
        expected_deliverables: vec![],
    }
}

fn confirm(call_id: &str) -> UserResponse {
    UserResponse {
        in_reply_to_call_id: call_id.to_string(),
        action: BriefingAction::Confirm,
    }
}

fn cancel(call_id: &str) -> UserResponse {
    UserResponse {
        in_reply_to_call_id: call_id.to_string(),
        action: BriefingAction::Cancel,
    }
}

fn edit_goal(call_id: &str, new_goal: &str) -> UserResponse {
    UserResponse {
        in_reply_to_call_id: call_id.to_string(),
        action: BriefingAction::Edit {
            edits: PartialBrief {
                goal: Some(new_goal.into()),
                ..Default::default()
            },
        },
    }
}

async fn briefing_events(events: &SqliteEventStore, session_id: &str) -> Vec<Value> {
    events
        .query(session_id, EventQuery::default())
        .await
        .expect("query")
        .into_iter()
        .filter(|e| {
            e.event_type == EventType::Misc
                && e.data.get("kind").and_then(|v| v.as_str()) == Some("briefing")
        })
        .map(|e| e.data)
        .collect()
}

async fn task_status(db: &DbPool, task_id: &str) -> String {
    let id = task_id.to_string();
    db.with_conn(move |conn| -> rusqlite::Result<String> {
        conn.query_row(
            "SELECT status FROM tasks WHERE id = ?",
            rusqlite::params![id],
            |row| row.get::<_, String>(0),
        )
    })
    .await
    .expect("status query")
}

#[tokio::test]
async fn briefing_confirm_path_starts_run() {
    let fx = setup_confirm_fixture().await;
    let (tx, rx) = mpsc::channel::<UserResponse>(4);
    tx.send(confirm("")).await.expect("send confirm");
    let outcome = fx
        .init
        .run_with_confirmation_for_test(
            &fx.session_id,
            &fx.task_id,
            sample_brief(),
            RunConfig::default(),
            rx,
        )
        .await
        .expect("run");
    assert_eq!(outcome, RunOutcome::Started);
    assert_eq!(task_status(&fx.db, &fx.task_id).await, "running");
    let all = fx
        .events
        .query(&fx.session_id, EventQuery::default())
        .await
        .expect("events");
    assert!(all.iter().any(|e| {
        e.event_type == EventType::Misc
            && e.data.get("kind").and_then(|v| v.as_str()) == Some("briefing_pending")
    }));
    let briefings = briefing_events(&fx.events, &fx.session_id).await;
    assert_eq!(briefings.len(), 1);
    let _ = fx.task_store.get(&fx.task_id).await.expect("task row");
}

#[tokio::test]
async fn briefing_edit_emits_new_briefing() {
    let fx = setup_confirm_fixture().await;
    let (tx, rx) = mpsc::channel::<UserResponse>(4);
    tx.send(edit_goal("", "Build a better thing"))
        .await
        .expect("send edit");
    tx.send(confirm("")).await.expect("send confirm");
    let outcome = fx
        .init
        .run_with_confirmation_for_test(
            &fx.session_id,
            &fx.task_id,
            sample_brief(),
            RunConfig::default(),
            rx,
        )
        .await
        .expect("run");
    assert_eq!(outcome, RunOutcome::Started);
    let briefings = briefing_events(&fx.events, &fx.session_id).await;
    assert_eq!(briefings.len(), 2, "expected two Briefing emissions");
    let id1 = briefings[0]
        .get("briefing_call_id")
        .and_then(|v| v.as_str())
        .unwrap();
    let id2 = briefings[1]
        .get("briefing_call_id")
        .and_then(|v| v.as_str())
        .unwrap();
    assert_ne!(id1, id2, "edit must re-emit with a NEW briefing_call_id");
    // The re-emitted brief reflects the edit.
    let new_goal = briefings[1]
        .pointer("/brief/goal")
        .and_then(|v| v.as_str())
        .unwrap();
    assert_eq!(new_goal, "Build a better thing");
}

#[tokio::test]
async fn briefing_cancel_transitions_to_cancelled() {
    let fx = setup_confirm_fixture().await;
    let (tx, rx) = mpsc::channel::<UserResponse>(4);
    tx.send(cancel("")).await.expect("send cancel");
    let outcome = fx
        .init
        .run_with_confirmation_for_test(
            &fx.session_id,
            &fx.task_id,
            sample_brief(),
            RunConfig::default(),
            rx,
        )
        .await
        .expect("run");
    assert_eq!(outcome, RunOutcome::Cancelled);
    assert_eq!(task_status(&fx.db, &fx.task_id).await, "cancelled");
    let all = fx
        .events
        .query(&fx.session_id, EventQuery::default())
        .await
        .expect("events");
    let task_state_evt = all
        .iter()
        .find(|e| {
            e.event_type == EventType::Misc
                && e.data.get("kind").and_then(|v| v.as_str()) == Some("task_state")
        })
        .expect("task_state Misc");
    assert_eq!(
        task_state_evt
            .data
            .get("to")
            .and_then(|v| v.as_str())
            .unwrap(),
        "cancelled"
    );
    assert_eq!(
        task_state_evt
            .data
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap(),
        "user_cancelled"
    );
}

#[tokio::test(start_paused = true)]
async fn briefing_auto_confirm_after_timeout() {
    let fx = setup_confirm_fixture().await;
    let (_tx, rx) = mpsc::channel::<UserResponse>(4);
    // No response is sent. Auto-advance fires the 5-minute sleep.
    let outcome = fx
        .init
        .run_with_confirmation_for_test(
            &fx.session_id,
            &fx.task_id,
            sample_brief(),
            RunConfig {
                confirm_timeout: Duration::from_secs(300),
                require_confirm: false,
            },
            rx,
        )
        .await
        .expect("run");
    assert_eq!(outcome, RunOutcome::Started);
    assert_eq!(task_status(&fx.db, &fx.task_id).await, "running");
    let all = fx
        .events
        .query(&fx.session_id, EventQuery::default())
        .await
        .expect("events");
    assert!(
        all.iter().any(|e| {
            e.event_type == EventType::Misc
                && e.data.get("kind").and_then(|v| v.as_str()) == Some("briefing_auto_confirmed")
        }),
        "expected briefing_auto_confirmed Misc"
    );
}

#[tokio::test(start_paused = true)]
async fn briefing_require_confirm_disables_auto() {
    let fx = setup_confirm_fixture().await;
    let (tx, rx) = mpsc::channel::<UserResponse>(4);
    let session_id = fx.session_id.clone();
    let task_id = fx.task_id.clone();
    let init = fx.init.clone();
    let brief = sample_brief();
    let handle = tokio::spawn(async move {
        init.run_with_confirmation_for_test(
            &session_id,
            &task_id,
            brief,
            RunConfig {
                confirm_timeout: Duration::from_secs(60),
                require_confirm: true,
            },
            rx,
        )
        .await
    });

    // Advance well past the would-be timeout. Auto-confirm MUST NOT
    // fire because require_confirm = true disables the timer branch.
    tokio::time::sleep(Duration::from_secs(3600)).await;
    let auto_confirmed_before = fx
        .events
        .query(&fx.session_id, EventQuery::default())
        .await
        .expect("events")
        .iter()
        .any(|e| {
            e.event_type == EventType::Misc
                && e.data.get("kind").and_then(|v| v.as_str()) == Some("briefing_auto_confirmed")
        });
    assert!(
        !auto_confirmed_before,
        "auto-confirm fired despite require_confirm = true"
    );

    tx.send(confirm("")).await.expect("send confirm");
    let outcome = handle.await.expect("join").expect("run");
    assert_eq!(outcome, RunOutcome::Started);
    let auto_confirmed_after = fx
        .events
        .query(&fx.session_id, EventQuery::default())
        .await
        .expect("events")
        .iter()
        .any(|e| {
            e.event_type == EventType::Misc
                && e.data.get("kind").and_then(|v| v.as_str()) == Some("briefing_auto_confirmed")
        });
    assert!(
        !auto_confirmed_after,
        "auto-confirm fired even after manual confirm under require_confirm"
    );
}

#[tokio::test]
async fn briefing_caps_edit_cycles_at_5() {
    let fx = setup_confirm_fixture().await;
    let (tx, rx) = mpsc::channel::<UserResponse>(16);
    // Six edits — the 6th attempt must fail.
    for i in 0..6 {
        tx.send(edit_goal("", &format!("Build v{i}")))
            .await
            .expect("send edit");
    }
    let err = fx
        .init
        .run_with_confirmation_for_test(
            &fx.session_id,
            &fx.task_id,
            sample_brief(),
            RunConfig::default(),
            rx,
        )
        .await
        .expect_err("should reject 6th edit");
    assert!(
        matches!(err, InitError::Brief(BriefError::TooManyEdits)),
        "expected TooManyEdits, got: {err:?}"
    );
}
