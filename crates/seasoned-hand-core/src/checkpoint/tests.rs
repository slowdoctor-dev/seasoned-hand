use std::sync::Arc;
use std::sync::Mutex;

use serde_json::Value;

use super::git_in_sandbox::CheckpointGitError;
use super::label_buffer::CheckpointLabelBuffer;
use super::persistence::CheckpointStore;
use super::routes::{ListQuery, RouteOutcome, list_checkpoints};
use super::*;
use crate::db;
use crate::events::{EventQuery, EventType, sqlite::SqliteEventStore};
use crate::pubsub::RedisPool;

#[derive(Clone, Default)]
struct MockGit {
    /// Replies handed out in order; `None` slots emit non-zero exit.
    shas: Arc<Mutex<Vec<Result<String, String>>>>,
    /// Records every (session_id, phase_id, title) call.
    calls: Arc<Mutex<Vec<(String, i64, String)>>>,
}

impl MockGit {
    fn new_with_shas(shas: Vec<&str>) -> Self {
        Self {
            shas: Arc::new(Mutex::new(shas.into_iter().map(|s| Ok(s.into())).collect())),
            calls: Default::default(),
        }
    }

    fn new_with_failure(reason: &str) -> Self {
        Self {
            shas: Arc::new(Mutex::new(vec![Err(reason.into())])),
            calls: Default::default(),
        }
    }
}

#[async_trait::async_trait]
impl GitShell for MockGit {
    async fn commit_phase(
        &self,
        session_id: &str,
        phase_id: i64,
        title: &str,
    ) -> Result<String, CheckpointGitError> {
        self.calls
            .lock()
            .unwrap()
            .push((session_id.into(), phase_id, title.into()));
        let next = self.shas.lock().unwrap().remove(0);
        match next {
            Ok(s) => Ok(s),
            Err(reason) => Err(CheckpointGitError::NonZeroExit {
                cmd: "git -C /workspace commit ...".into(),
                exit_code: 1,
                stderr: reason,
            }),
        }
    }
}

async fn open_with_session(session_id: &str) -> (db::DbPool, Arc<SqliteEventStore>) {
    let pool = db::open(":memory:").await.expect("db");
    let id = session_id.to_string();
    pool.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, state) \
             VALUES (?, 0, 0, 'RUNNING')",
            rusqlite::params![id],
        )
        .expect("insert session");
    })
    .await;
    let redis = RedisPool::new("redis://127.0.0.1:6").unwrap();
    let events = Arc::new(SqliteEventStore::with_redis(pool.clone(), redis));
    (pool, events)
}

#[tokio::test]
async fn migration_v005_idempotent() {
    let pool = db::open(":memory:").await.expect("db");
    // Repeated migration via refinery embedded in db::open is idempotent.
    let count: i64 = pool
        .with_conn(|conn| {
            conn.query_row::<i64, _, _>("SELECT COUNT(*) FROM checkpoints", [], |r| r.get(0))
                .unwrap()
        })
        .await;
    assert_eq!(count, 0);
}

#[tokio::test]
async fn plan_advance_creates_checkpoint_row_and_misc() {
    let (pool, events) = open_with_session("sess-cp").await;
    let store = Arc::new(CheckpointStore::new(pool.clone()));
    let labels = Arc::new(CheckpointLabelBuffer::new());
    let git: Arc<dyn GitShell> = Arc::new(MockGit::new_with_shas(vec![
        "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
    ]));
    let mgr = CheckpointManager::new(CheckpointManagerDeps {
        store: store.clone(),
        labels,
        events: events.clone(),
        git,
    });

    let ev = PlanAdvanceEvent {
        session_id: "sess-cp".into(),
        plan_phase_id: 1,
        phase_title: "Initial scaffolding".into(),
        triggered_by_event_id: 7,
    };
    let id = mgr
        .handle_plan_advance(ev)
        .await
        .expect("handle Ok")
        .expect("checkpoint id returned on success");

    let rows = store
        .list_by_session("sess-cp", None, 50)
        .await
        .expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, id);
    assert_eq!(rows[0].plan_phase_id, 1);
    assert_eq!(rows[0].git_sha, "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
    assert_eq!(rows[0].triggered_by_event_id, 7);

    let misc = events
        .query(
            "sess-cp",
            EventQuery {
                after_id: None,
                event_type: Some(EventType::Misc),
                limit: Some(50),
            },
        )
        .await
        .expect("query Misc");
    let create_ev = misc
        .iter()
        .find(|e| e.data.get("kind").and_then(Value::as_str) == Some("checkpoint_create"))
        .expect("checkpoint_create Misc emitted");
    assert_eq!(create_ev.data["checkpoint_id"], serde_json::json!(id));
    assert_eq!(create_ev.data["plan_phase_id"], 1);
}

#[tokio::test]
async fn checkpoint_label_attaches_then_clears() {
    let (pool, events) = open_with_session("sess-lab").await;
    let store = Arc::new(CheckpointStore::new(pool.clone()));
    let labels = Arc::new(CheckpointLabelBuffer::new());
    let git: Arc<dyn GitShell> = Arc::new(MockGit::new_with_shas(vec!["sha1aaaaaa", "sha2bbbbbb"]));
    let mgr = CheckpointManager::new(CheckpointManagerDeps {
        store: store.clone(),
        labels: labels.clone(),
        events: events.clone(),
        git,
    });

    labels.set("sess-lab", "milestone-A");
    let _ = mgr
        .handle_plan_advance(PlanAdvanceEvent {
            session_id: "sess-lab".into(),
            plan_phase_id: 1,
            phase_title: "first".into(),
            triggered_by_event_id: 1,
        })
        .await
        .expect("first advance");
    let _ = mgr
        .handle_plan_advance(PlanAdvanceEvent {
            session_id: "sess-lab".into(),
            plan_phase_id: 2,
            phase_title: "second".into(),
            triggered_by_event_id: 2,
        })
        .await
        .expect("second advance");

    let rows = store
        .list_by_session("sess-lab", None, 50)
        .await
        .expect("list");
    assert_eq!(rows.len(), 2);
    // Newest-first ordering: [phase 2, phase 1]
    let labeled = rows.iter().find(|r| r.plan_phase_id == 1).unwrap();
    let unlabeled = rows.iter().find(|r| r.plan_phase_id == 2).unwrap();
    assert_eq!(labeled.label.as_deref(), Some("milestone-A"));
    assert!(
        unlabeled.label.is_none(),
        "label must clear after first take"
    );
}

#[tokio::test]
async fn commit_failure_emits_create_with_ok_false_and_no_row() {
    let (pool, events) = open_with_session("sess-fail").await;
    let store = Arc::new(CheckpointStore::new(pool.clone()));
    let labels = Arc::new(CheckpointLabelBuffer::new());
    let git: Arc<dyn GitShell> = Arc::new(MockGit::new_with_failure("git index corrupted"));
    let mgr = CheckpointManager::new(CheckpointManagerDeps {
        store: store.clone(),
        labels,
        events: events.clone(),
        git,
    });

    let res = mgr
        .handle_plan_advance(PlanAdvanceEvent {
            session_id: "sess-fail".into(),
            plan_phase_id: 1,
            phase_title: "broken".into(),
            triggered_by_event_id: 1,
        })
        .await
        .expect("infra Ok, commit Err logged");
    assert!(
        res.is_none(),
        "commit failure must NOT return a checkpoint id"
    );

    let rows = store
        .list_by_session("sess-fail", None, 50)
        .await
        .expect("list");
    assert_eq!(rows.len(), 0, "no row on commit failure");

    let misc = events
        .query(
            "sess-fail",
            EventQuery {
                after_id: None,
                event_type: Some(EventType::Misc),
                limit: Some(50),
            },
        )
        .await
        .expect("query Misc");
    let create_ev = misc
        .iter()
        .find(|e| e.data.get("kind").and_then(Value::as_str) == Some("checkpoint_create"))
        .expect("checkpoint_create Misc emitted on failure");
    assert_eq!(create_ev.data["ok"], serde_json::json!(false));
    assert_eq!(create_ev.data["plan_phase_id"], 1);
    let reason = create_ev.data["reason"].as_str().unwrap_or_default();
    assert!(
        reason.contains("git index corrupted"),
        "reason must surface the underlying error, got {reason:?}"
    );
}

#[tokio::test]
async fn http_checkpoints_list_route_returns_paginated_json() {
    let (pool, events) = open_with_session("sess-list").await;
    let store = Arc::new(CheckpointStore::new(pool.clone()));
    let labels = Arc::new(CheckpointLabelBuffer::new());
    let git: Arc<dyn GitShell> = Arc::new(MockGit::new_with_shas(vec!["a1", "b2", "c3"]));
    let mgr = CheckpointManager::new(CheckpointManagerDeps {
        store: store.clone(),
        labels,
        events,
        git,
    });
    for i in 1..=3 {
        let _ = mgr
            .handle_plan_advance(PlanAdvanceEvent {
                session_id: "sess-list".into(),
                plan_phase_id: i,
                phase_title: format!("phase {i}"),
                triggered_by_event_id: i,
            })
            .await
            .unwrap();
    }
    let outcome = list_checkpoints(
        &store,
        "sess-list",
        ListQuery {
            cursor: None,
            limit: Some(2),
        },
    )
    .await;
    match outcome {
        RouteOutcome::Ok(body) => {
            assert_eq!(body.rows.len(), 2);
            assert!(body.next_cursor.is_some());
        }
        _ => panic!("expected Ok"),
    }
}

#[tokio::test]
async fn manager_run_returns_when_cancellation_token_fires() {
    let (pool, events) = open_with_session("sess-run").await;
    let store = Arc::new(CheckpointStore::new(pool));
    let labels = Arc::new(CheckpointLabelBuffer::new());
    let git: Arc<dyn GitShell> = Arc::new(MockGit::new_with_shas(vec![]));
    let mgr = CheckpointManager::new(CheckpointManagerDeps {
        store,
        labels,
        events,
        git,
    });
    let token = tokio_util::sync::CancellationToken::new();
    let token_clone = token.clone();
    let h = tokio::spawn(async move { mgr.run(token_clone).await });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    token.cancel();
    tokio::time::timeout(std::time::Duration::from_secs(2), h)
        .await
        .expect("run exits within 2s after cancel")
        .expect("task joined")
        .expect("run returns Ok");
}
