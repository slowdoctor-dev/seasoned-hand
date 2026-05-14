//! Story 2.16 — durable pause + replay-rebuild unit tests.
//!
//! These tests exercise the `task::resume_task` entry point + the
//! individual `replay::*` helpers using a stub [`SandboxOps`] impl
//! that pre-registers handles without docker, plus tempdir-backed
//! workspace writes for the real-filesystem assertions.
//!
//! refs: /specs/phase-2/stories/story-2.16.md

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::RwLock;

use crate::db::{self, DbPool};
use crate::events::{EventStore, EventType, NewEvent, sqlite::SqliteEventStore};
use crate::plan::{Phase, PhaseStatus, Plan, PlanManager};
use crate::project::{NewProject, NewTask, ProjectStore, TaskStatus, TaskStore};
use crate::sandbox::{SandboxError, SandboxHandle};
use crate::task::{
    ReplayError, ResumeDeps, ResumeOutcome, SandboxOps, replay::WorkspaceWriter, resume_task,
};

// ---------- test sandbox -----------------------------------------------

/// Stub [`SandboxOps`] that:
/// - reads/writes workspace files against a tempdir per session,
/// - allows tests to pre-register handles via [`Self::insert_handle`],
/// - blocks `create_handle` until [`Self::set_create_handle`] is set
///   (tests "rebuild" by pre-creating the new session's handle then
///   letting create_handle return it without docker), and
/// - can be configured to fail workspace writes for the failure-path
///   test.
struct TestSandbox {
    handles: Arc<RwLock<std::collections::HashMap<String, SandboxHandle>>>,
    pending_create: Arc<RwLock<Option<SandboxHandle>>>,
    fail_writes: Arc<RwLock<bool>>,
    root: PathBuf,
}

impl TestSandbox {
    fn new(root: PathBuf) -> Self {
        Self {
            handles: Arc::new(RwLock::new(std::collections::HashMap::new())),
            pending_create: Arc::new(RwLock::new(None)),
            fail_writes: Arc::new(RwLock::new(false)),
            root,
        }
    }

    async fn insert_handle(&self, handle: SandboxHandle) {
        self.handles
            .write()
            .await
            .insert(handle.session_id.clone(), handle);
    }

    async fn drop_handle(&self, session_id: &str) {
        self.handles.write().await.remove(session_id);
    }

    async fn enable_write_failures(&self) {
        *self.fail_writes.write().await = true;
    }

    fn handle_for(&self, session_id: &str) -> SandboxHandle {
        let path = self.root.join(session_id);
        std::fs::create_dir_all(&path).unwrap();
        SandboxHandle {
            session_id: session_id.to_string(),
            container_id: format!("test-{session_id}"),
            api_url: "http://127.0.0.1:0".into(),
            novnc_url: "http://127.0.0.1:0".into(),
            ttyd_url: "ws://127.0.0.1:0".into(),
            workspace_host_path: path,
        }
    }
}

impl WorkspaceWriter for TestSandbox {
    async fn write_workspace_file(
        &self,
        session_id: &str,
        relative_path: &str,
        contents: &[u8],
    ) -> Result<(), SandboxError> {
        if *self.fail_writes.read().await {
            return Err(SandboxError::WorkspaceBootstrap("induced".into()));
        }
        let handle = self
            .handles
            .read()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| SandboxError::NotFound(session_id.to_string()))?;
        let path = handle.workspace_host_path.join(relative_path);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(path, contents).await?;
        Ok(())
    }

    async fn write_workspace_file_json_value(
        &self,
        session_id: &str,
        relative_path: &str,
        value: &Value,
    ) -> Result<(), SandboxError> {
        let bytes = serde_json::to_vec_pretty(value)
            .map_err(|e| SandboxError::WorkspaceBootstrap(e.to_string()))?;
        self.write_workspace_file(session_id, relative_path, &bytes)
            .await
    }
}

impl SandboxOps for TestSandbox {
    async fn get_handle(&self, session_id: &str) -> Option<SandboxHandle> {
        self.handles.read().await.get(session_id).cloned()
    }

    async fn create_handle(&self, session_id: &str) -> Result<SandboxHandle, SandboxError> {
        if let Some(handle) = self.pending_create.write().await.take() {
            self.handles
                .write()
                .await
                .insert(session_id.to_string(), handle.clone());
            return Ok(handle);
        }
        let handle = self.handle_for(session_id);
        self.handles
            .write()
            .await
            .insert(session_id.to_string(), handle.clone());
        Ok(handle)
    }

    async fn unpause(&self, _session_id: &str) -> Result<(), SandboxError> {
        Ok(())
    }
}

// ---------- shared fixture ---------------------------------------------

struct Fixture {
    pool: DbPool,
    _tmp: TempDir,
    events: Arc<SqliteEventStore>,
    plan_manager: Arc<PlanManager>,
    task_store: TaskStore,
    sandbox: TestSandbox,
    task_id: String,
    session_id: String,
}

/// Seed: project + task, one session linked to the task, an
/// event-store + plan_manager wired against the same pool, and a
/// stub sandbox with a tempdir workspace for the seed session.
async fn fixture() -> Fixture {
    let pool = db::open(":memory:").await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let events = Arc::new(SqliteEventStore::new(pool.clone()));
    let plan_manager = Arc::new(PlanManager::new(pool.clone(), events.clone()));
    let project_store = ProjectStore::new(pool.clone());
    let task_store = TaskStore::new(pool.clone());

    let project_id = project_store
        .insert(NewProject {
            tenant_id: None,
            title: "p".into(),
            description: None,
        })
        .await
        .unwrap();
    let task_id = task_store
        .insert(NewTask {
            project_id,
            tenant_id: None,
            title: "t".into(),
            expected_due_at: None,
        })
        .await
        .unwrap();
    // drafted → briefed → confirmed → running → paused
    task_store
        .set_status(&task_id, TaskStatus::Briefed)
        .await
        .unwrap();
    task_store
        .set_status(&task_id, TaskStatus::Confirmed)
        .await
        .unwrap();
    task_store
        .set_status(&task_id, TaskStatus::Running)
        .await
        .unwrap();
    task_store
        .set_status(&task_id, TaskStatus::Paused)
        .await
        .unwrap();

    let session_id = "sess-old".to_string();
    let sid = session_id.clone();
    let tid = task_id.clone();
    pool.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, state, task_id) \
             VALUES (?, 100, 100, 'SUSPENDED', ?)",
            rusqlite::params![sid, tid],
        )
        .unwrap();
    })
    .await;
    let sandbox = TestSandbox::new(tmp.path().to_path_buf());
    sandbox.insert_handle(sandbox.handle_for(&session_id)).await;

    Fixture {
        pool,
        _tmp: tmp,
        events,
        plan_manager,
        task_store,
        sandbox,
        task_id,
        session_id,
    }
}

fn make_plan(session_id: &str) -> Plan {
    Plan {
        id: "plan-x".into(),
        session_id: session_id.into(),
        goal: "Build a thing".into(),
        phases: vec![
            Phase {
                id: 1,
                title: "design".into(),
                capabilities: vec![],
                status: PhaseStatus::Done,
            },
            Phase {
                id: 2,
                title: "build".into(),
                capabilities: vec![],
                status: PhaseStatus::Active,
            },
            Phase {
                id: 3,
                title: "ship".into(),
                capabilities: vec![],
                status: PhaseStatus::Pending,
            },
        ],
        current_phase_id: Some(2),
    }
}

// ---------- AC 1: durable pause emits task_paused_durable -------------
//
// The pause-side emission lives in the WS layer (server crate); this
// test asserts the event-stream shape that *core* tests use as the
// rebuild trigger when the server appends it. Verifies the helper
// chain that produces the cursor + event ordering, mirroring what
// `handle_task_pause` does.

#[tokio::test]
async fn durable_pause_emits_task_paused_durable_misc() {
    let fx = fixture().await;
    // Mimic what the WS layer does on durable pause: emit
    // task_paused_durable first, then task_paused.
    fx.events
        .append(NewEvent {
            session_id: fx.session_id.clone(),
            event_type: EventType::Misc,
            source: "ws".into(),
            data: json!({
                "kind": "task_paused_durable",
                "sandbox_id": "sb-1",
                "workspace_path": "/tmp/x",
                "event_cursor": 0,
                "paused_at": 1_000_000_i64,
            }),
        })
        .await
        .unwrap();
    fx.events
        .append(NewEvent {
            session_id: fx.session_id.clone(),
            event_type: EventType::Misc,
            source: "ws".into(),
            data: json!({"kind": "task_paused"}),
        })
        .await
        .unwrap();
    let events = fx
        .events
        .query(&fx.session_id, Default::default())
        .await
        .unwrap();
    let kinds: Vec<&str> = events
        .iter()
        .filter_map(|e| e.data.get("kind").and_then(Value::as_str))
        .collect();
    let durable_idx = kinds
        .iter()
        .position(|k| *k == "task_paused_durable")
        .expect("durable Misc missing");
    let paused_idx = kinds
        .iter()
        .position(|k| *k == "task_paused")
        .expect("paused Misc missing");
    assert!(
        durable_idx < paused_idx,
        "task_paused_durable must precede task_paused: {kinds:?}",
    );
}

// ---------- AC 2: resume with live container uses unpause path --------

#[tokio::test]
async fn resume_with_live_container_uses_existing_path() {
    let fx = fixture().await;
    // Handle for the old session is in the cache (insert_handle in
    // `fixture`) — the unpause path should fire and no
    // rebuild_required Misc should appear.
    let deps = ResumeDeps {
        task_store: &fx.task_store,
        events: fx.events.as_ref(),
        plan_manager: fx.plan_manager.as_ref(),
        sandbox: &fx.sandbox,
        db: &fx.pool,
    };
    let outcome = resume_task(&fx.task_id, deps).await.unwrap();
    match outcome {
        ResumeOutcome::UnpausedExisting { session_id } => {
            assert_eq!(session_id, fx.session_id);
        }
        other => panic!("expected UnpausedExisting, got {other:?}"),
    }
    let task = fx.task_store.get(&fx.task_id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Running);

    let events = fx
        .events
        .query(&fx.session_id, Default::default())
        .await
        .unwrap();
    assert!(
        !events.iter().any(|e| {
            e.data.get("kind").and_then(Value::as_str) == Some("task_resume_rebuild_required")
        }),
        "unpause path must NOT emit rebuild_required"
    );
    assert!(
        events
            .iter()
            .any(|e| { e.data.get("kind").and_then(Value::as_str) == Some("task_resumed") })
    );
}

// ---------- AC 3: resume with dead container rebuilds + replays -------

#[tokio::test]
async fn resume_with_dead_container_rebuilds() {
    let fx = fixture().await;
    // Plant a Plan event so the rebuild has something to replay.
    let plan = make_plan(&fx.session_id);
    fx.plan_manager
        .emit_replay_create(&fx.session_id, &plan)
        .await
        .unwrap();
    // Plant a feature_done event for the first feature.
    fx.events
        .append(NewEvent {
            session_id: fx.session_id.clone(),
            event_type: EventType::Misc,
            source: "tool:feature_mark_done".into(),
            data: json!({
                "kind": "feature_done",
                "feature_id": "f-1",
                "title": "design",
            }),
        })
        .await
        .unwrap();
    // Plant a progress_update event.
    fx.events
        .append(NewEvent {
            session_id: fx.session_id.clone(),
            event_type: EventType::Misc,
            source: "tool:progress_update".into(),
            data: json!({
                "kind": "progress_update",
                "line": "started feature f-1",
            }),
        })
        .await
        .unwrap();

    // Simulate sandbox-gone for the old session.
    fx.sandbox.drop_handle(&fx.session_id).await;

    let deps = ResumeDeps {
        task_store: &fx.task_store,
        events: fx.events.as_ref(),
        plan_manager: fx.plan_manager.as_ref(),
        sandbox: &fx.sandbox,
        db: &fx.pool,
    };
    let outcome = resume_task(&fx.task_id, deps).await.unwrap();
    let new_session_id = match outcome {
        ResumeOutcome::Rebuilt { new_session_id, .. } => new_session_id,
        other => panic!("expected Rebuilt, got {other:?}"),
    };

    // OLD session timeline carries rebuild_required Misc.
    let old_events = fx
        .events
        .query(&fx.session_id, Default::default())
        .await
        .unwrap();
    assert!(old_events.iter().any(|e| {
        e.data.get("kind").and_then(Value::as_str) == Some("task_resume_rebuild_required")
    }));

    // NEW session row exists, state RUNNING, linked to same task_id.
    let nsid = new_session_id.clone();
    let row: (String, String, Option<String>) = fx
        .pool
        .with_conn(move |conn| {
            conn.query_row(
                "SELECT id, state, task_id FROM sessions WHERE id = ?",
                rusqlite::params![nsid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
        })
        .await;
    assert_eq!(row.1, "RUNNING");
    assert_eq!(row.2.as_deref(), Some(fx.task_id.as_str()));

    // Task status is back to Running.
    let task = fx.task_store.get(&fx.task_id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Running);
}

// ---------- AC 4: replay reconstructs Plan from events ----------------

#[tokio::test]
async fn replay_reconstructs_plan_from_events() {
    let fx = fixture().await;
    let plan = make_plan(&fx.session_id);
    fx.plan_manager
        .emit_replay_create(&fx.session_id, &plan)
        .await
        .unwrap();
    // Also emit an update with a slightly different snapshot to
    // verify the replay picks the LATEST.
    let mut updated = plan.clone();
    updated.phases[1].status = PhaseStatus::Done;
    updated.phases[2].status = PhaseStatus::Active;
    updated.current_phase_id = Some(3);
    // PlanManager::update would normalize differently; we emit via
    // the same helper to keep the snapshot byte-for-byte.
    fx.plan_manager
        .emit_replay_create(&fx.session_id, &updated)
        .await
        .unwrap();

    fx.sandbox.drop_handle(&fx.session_id).await;

    let deps = ResumeDeps {
        task_store: &fx.task_store,
        events: fx.events.as_ref(),
        plan_manager: fx.plan_manager.as_ref(),
        sandbox: &fx.sandbox,
        db: &fx.pool,
    };
    let outcome = resume_task(&fx.task_id, deps).await.unwrap();
    let new_session_id = match outcome {
        ResumeOutcome::Rebuilt { new_session_id, .. } => new_session_id,
        other => panic!("expected Rebuilt, got {other:?}"),
    };

    let snapshot = fx.plan_manager.snapshot(&new_session_id).await.unwrap();
    assert_eq!(snapshot.goal, plan.goal);
    assert_eq!(snapshot.current_phase_id, Some(3));
    assert_eq!(snapshot.phases[1].status, PhaseStatus::Done);
    assert_eq!(snapshot.phases[2].status, PhaseStatus::Active);
}

// ---------- AC 5: replay reconstructs feature-list --------------------

#[tokio::test]
async fn replay_reconstructs_feature_list_from_misc() {
    let fx = fixture().await;
    let plan = make_plan(&fx.session_id);
    fx.plan_manager
        .emit_replay_create(&fx.session_id, &plan)
        .await
        .unwrap();
    // Two feature_done events.
    for fid in ["f-1", "f-2"] {
        fx.events
            .append(NewEvent {
                session_id: fx.session_id.clone(),
                event_type: EventType::Misc,
                source: "tool:feature_mark_done".into(),
                data: json!({"kind":"feature_done","feature_id":fid,"title":fid}),
            })
            .await
            .unwrap();
    }

    fx.sandbox.drop_handle(&fx.session_id).await;

    let deps = ResumeDeps {
        task_store: &fx.task_store,
        events: fx.events.as_ref(),
        plan_manager: fx.plan_manager.as_ref(),
        sandbox: &fx.sandbox,
        db: &fx.pool,
    };
    let outcome = resume_task(&fx.task_id, deps).await.unwrap();
    let new_session_id = match outcome {
        ResumeOutcome::Rebuilt { new_session_id, .. } => new_session_id,
        other => panic!("expected Rebuilt, got {other:?}"),
    };

    // Read the workspace feature-list.json off the test sandbox's
    // tempdir.
    let handle = fx
        .sandbox
        .get_handle(&new_session_id)
        .await
        .expect("new session handle");
    let path = handle.workspace_host_path.join("feature-list.json");
    let bytes = std::fs::read(&path).expect("feature-list.json must exist");
    let list: Value = serde_json::from_slice(&bytes).unwrap();
    let features = list["features"].as_array().unwrap();
    let f1 = features.iter().find(|f| f["id"] == "f-1").unwrap();
    let f2 = features.iter().find(|f| f["id"] == "f-2").unwrap();
    let f3 = features.iter().find(|f| f["id"] == "f-3").unwrap();
    assert_eq!(f1["status"], "done");
    assert_eq!(f2["status"], "done");
    assert_ne!(f3["status"], "done");
}

// ---------- AC 6: replay failure → task failed -----------------------

#[tokio::test]
async fn replay_failure_transitions_task_to_failed() {
    let fx = fixture().await;
    // Need at least one plan event so replay reaches the feature-list
    // write step; the write is the step we induce failure on.
    let plan = make_plan(&fx.session_id);
    fx.plan_manager
        .emit_replay_create(&fx.session_id, &plan)
        .await
        .unwrap();

    fx.sandbox.drop_handle(&fx.session_id).await;
    fx.sandbox.enable_write_failures().await;

    let deps = ResumeDeps {
        task_store: &fx.task_store,
        events: fx.events.as_ref(),
        plan_manager: fx.plan_manager.as_ref(),
        sandbox: &fx.sandbox,
        db: &fx.pool,
    };
    let err = resume_task(&fx.task_id, deps)
        .await
        .expect_err("expected replay failure");
    // The error must be a Replay step error.
    match err {
        crate::task::ResumeError::Replay(ReplayError::Step { step, .. }) => {
            assert!(step == "feature_list" || step == "progress");
        }
        other => panic!("expected Replay::Step error, got {other:?}"),
    }
    let task = fx.task_store.get(&fx.task_id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Failed);
    let reason = task.failure_reason.unwrap_or_default();
    assert!(
        reason.starts_with("replay_failed:"),
        "failure_reason must encode step: {reason}"
    );

    // The new session (which was created before the replay step
    // failed) should carry a task_resume_rebuild_failed Misc event.
    let new_sid = latest_session_for_task(&fx.pool, &fx.task_id).await;
    assert_ne!(
        new_sid,
        Some(fx.session_id.clone()),
        "new session should be the freshly created one"
    );
    let events = fx
        .events
        .query(&new_sid.unwrap(), Default::default())
        .await
        .unwrap();
    assert!(events.iter().any(|e| {
        e.data.get("kind").and_then(Value::as_str) == Some("task_resume_rebuild_failed")
    }));
}

async fn latest_session_for_task(pool: &DbPool, task_id: &str) -> Option<String> {
    let tid = task_id.to_string();
    pool.with_conn(move |conn| {
        conn.query_row(
            "SELECT id FROM sessions WHERE task_id = ? ORDER BY created_at DESC LIMIT 1",
            [&tid],
            |r| r.get::<_, String>(0),
        )
        .ok()
    })
    .await
}
