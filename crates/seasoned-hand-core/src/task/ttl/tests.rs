//! Unit tests for [`WorkspaceTtlCron`] (Phase 2 story 2.17).
//!
//! Each test seeds a Project + Task at a known `(status, updated_at)`
//! plus a Session row that the cron will pick as the target. A
//! [`FakeJanitor`] records `destroy` calls without touching docker; the
//! workspace dir is a real tempdir so the rmdir step exercises a true
//! filesystem path.
//!
//! refs: /specs/phase-2/stories/story-2.17.md

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tempfile::TempDir;
use tokio::sync::Mutex;

use crate::db::{self, DbPool};
use crate::events::{EventStore, sqlite::SqliteEventStore};
use crate::project::{NewProject, ProjectStore, TaskStatus, TaskStore};
use crate::sandbox::{SandboxError, SandboxHandle};
use crate::task::ttl::{SandboxJanitor, TtlConfig, WorkspaceTtlCron};
use crate::time::now_micros;

// ---------- fake janitor ----------------------------------------------

struct FakeJanitor {
    root: PathBuf,
    handles: Mutex<std::collections::HashMap<String, SandboxHandle>>,
    destroyed: Mutex<Vec<String>>,
}

impl FakeJanitor {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            handles: Mutex::new(std::collections::HashMap::new()),
            destroyed: Mutex::new(Vec::new()),
        }
    }

    async fn insert_handle(&self, session_id: &str) -> PathBuf {
        let path = self.root.join(session_id);
        std::fs::create_dir_all(&path).unwrap();
        // Write a marker file so the rmdir step has something real to remove.
        std::fs::write(path.join("marker.txt"), b"present").unwrap();
        let handle = SandboxHandle {
            session_id: session_id.to_string(),
            container_id: format!("c-{session_id}"),
            api_url: "http://127.0.0.1:0".into(),
            novnc_url: "http://127.0.0.1:0".into(),
            ttyd_url: "ws://127.0.0.1:0".into(),
            workspace_host_path: path.clone(),
        };
        self.handles
            .lock()
            .await
            .insert(session_id.to_string(), handle);
        path
    }

    async fn destroyed(&self) -> Vec<String> {
        self.destroyed.lock().await.clone()
    }
}

impl SandboxJanitor for FakeJanitor {
    async fn get_handle(&self, session_id: &str) -> Option<SandboxHandle> {
        self.handles.lock().await.get(session_id).cloned()
    }
    async fn destroy(&self, session_id: &str) -> Result<(), SandboxError> {
        self.destroyed.lock().await.push(session_id.to_string());
        self.handles.lock().await.remove(session_id);
        Ok(())
    }
    fn workspace_root(&self) -> &Path {
        &self.root
    }
}

// ---------- fixture ---------------------------------------------------

struct Fixture {
    pool: DbPool,
    _tmp: TempDir,
    events: Arc<SqliteEventStore>,
    task_store: Arc<TaskStore>,
    sandbox: Arc<FakeJanitor>,
    project_id: String,
}

async fn fixture() -> Fixture {
    let pool = db::open(":memory:").await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let events = Arc::new(SqliteEventStore::new(pool.clone()));
    let task_store = Arc::new(TaskStore::new(pool.clone()));
    let project_store = ProjectStore::new(pool.clone());
    let project_id = project_store
        .insert(NewProject {
            tenant_id: None,
            title: "p".into(),
            description: None,
        })
        .await
        .unwrap();
    let sandbox = Arc::new(FakeJanitor::new(tmp.path().to_path_buf()));
    Fixture {
        pool,
        _tmp: tmp,
        events,
        task_store,
        sandbox,
        project_id,
    }
}

/// Insert a task row directly with the chosen status + `updated_at`,
/// bypassing the state-machine. Returns the new task id.
async fn seed_task(fx: &Fixture, status: TaskStatus, updated_at_micros: i64) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let id_c = id.clone();
    let project_id = fx.project_id.clone();
    let status_str = status.as_db_str().to_string();
    fx.pool
        .with_conn(move |conn| {
            conn.execute(
                 "INSERT INTO tasks (\
                   id, project_id, tenant_id, title, brief, status, \
                   expected_due_at, completed_at, failure_reason, \
                   parent_task_id, schedule, skill_attached_event_id, \
                   created_at, updated_at\
                 ) VALUES (?, ?, 'legacy-default', 't', NULL, ?, NULL, NULL, NULL, NULL, NULL, NULL, ?, ?)",
                rusqlite::params![
                    id_c,
                    project_id,
                    status_str,
                    updated_at_micros,
                    updated_at_micros
                ],
            )
            .unwrap();
        })
        .await;
    id
}

async fn seed_session(fx: &Fixture, task_id: &str) -> String {
    let session_id = format!("sess-{}", uuid::Uuid::new_v4());
    let sid = session_id.clone();
    let tid = task_id.to_string();
    fx.pool
        .with_conn(move |conn| {
            conn.execute(
                "INSERT INTO sessions (id, created_at, updated_at, state, task_id) \
                 VALUES (?, 100, 100, 'FINISHED', ?)",
                rusqlite::params![sid, tid],
            )
            .unwrap();
        })
        .await;
    session_id
}

fn micros_from_days(days: u64) -> i64 {
    i64::try_from(Duration::from_secs(days * 86_400).as_micros()).unwrap()
}

fn test_config() -> TtlConfig {
    // Production defaults — tests dial only the timestamps, not the
    // window itself, so the SQL boundary math is genuinely exercised.
    TtlConfig::defaults()
}

fn build_cron(fx: &Fixture, config: TtlConfig) -> WorkspaceTtlCron<FakeJanitor> {
    WorkspaceTtlCron::new(
        fx.task_store.clone(),
        fx.events.clone(),
        fx.sandbox.clone(),
        fx.pool.clone(),
        config,
    )
}

async fn misc_kinds(events: &SqliteEventStore, session_id: &str) -> Vec<String> {
    events
        .query(session_id, Default::default())
        .await
        .unwrap()
        .into_iter()
        .filter_map(|e| e.data.get("kind").and_then(Value::as_str).map(String::from))
        .collect()
}

// ---------- AC: cleans completed task after 30d -----------------------

#[tokio::test]
async fn ttl_cleans_completed_task_after_30d() {
    let fx = fixture().await;
    let aged = now_micros() - micros_from_days(31);
    let task_id = seed_task(&fx, TaskStatus::Completed, aged).await;
    let session_id = seed_session(&fx, &task_id).await;
    let workspace_path = fx.sandbox.insert_handle(&session_id).await;
    assert!(workspace_path.exists(), "workspace seeded");

    let cron = build_cron(&fx, test_config());
    let report = cron.cleanup_cycle().await;

    assert_eq!(report.cleaned, 1, "one row matches the 30d window");
    assert_eq!(report.failed, 0);
    assert_eq!(fx.sandbox.destroyed().await, vec![session_id.clone()]);
    assert!(
        !workspace_path.exists(),
        "workspace dir must be removed after cleanup",
    );
    let kinds = misc_kinds(fx.events.as_ref(), &session_id).await;
    assert!(
        kinds.iter().any(|k| k == "sandbox_cleaned"),
        "expected sandbox_cleaned Misc on OLD session, got {kinds:?}",
    );

    // Second cycle must NOT re-clean — `updated_at` was bumped to now,
    // so the row is outside the 30d window again.
    let report2 = cron.cleanup_cycle().await;
    assert_eq!(report2.cleaned, 0, "row already drained: {report2:?}");
}

// ---------- AC: skips running task ------------------------------------

#[tokio::test]
async fn ttl_skips_running_task() {
    let fx = fixture().await;
    let aged = now_micros() - micros_from_days(99); // way older than any TTL
    let task_id = seed_task(&fx, TaskStatus::Running, aged).await;
    let session_id = seed_session(&fx, &task_id).await;
    let workspace_path = fx.sandbox.insert_handle(&session_id).await;

    let cron = build_cron(&fx, test_config());
    let report = cron.cleanup_cycle().await;

    assert_eq!(report.cleaned, 0);
    assert_eq!(report.failed, 0);
    assert!(fx.sandbox.destroyed().await.is_empty());
    assert!(workspace_path.exists(), "running workspace untouched");

    // The row's updated_at must not have been bumped (would imply we
    // touched the row in some way).
    let row_updated_at: i64 = {
        let tid = task_id.clone();
        fx.pool
            .with_conn(move |conn| {
                conn.query_row(
                    "SELECT updated_at FROM tasks WHERE id = ?",
                    rusqlite::params![tid],
                    |row| row.get(0),
                )
                .unwrap()
            })
            .await
    };
    assert_eq!(row_updated_at, aged, "running rows must not be mutated");
}

// ---------- AC: skips paused task (durable-pause safety) -------------

#[tokio::test]
async fn ttl_skips_paused_task_for_durable_pause() {
    let fx = fixture().await;
    let aged = now_micros() - micros_from_days(99);
    let task_id = seed_task(&fx, TaskStatus::Paused, aged).await;
    let session_id = seed_session(&fx, &task_id).await;
    let workspace_path = fx.sandbox.insert_handle(&session_id).await;

    let cron = build_cron(&fx, test_config());
    let report = cron.cleanup_cycle().await;

    assert_eq!(report.cleaned, 0);
    assert!(fx.sandbox.destroyed().await.is_empty());
    assert!(workspace_path.exists());
}

// ---------- AC: cleans failed task after 7d ---------------------------

#[tokio::test]
async fn ttl_cleans_failed_task_after_7d() {
    let fx = fixture().await;
    let aged = now_micros() - micros_from_days(8);
    let task_id = seed_task(&fx, TaskStatus::Failed, aged).await;
    let session_id = seed_session(&fx, &task_id).await;
    let workspace_path = fx.sandbox.insert_handle(&session_id).await;

    let cron = build_cron(&fx, test_config());
    let report = cron.cleanup_cycle().await;

    assert_eq!(report.cleaned, 1);
    assert_eq!(fx.sandbox.destroyed().await, vec![session_id.clone()]);
    assert!(!workspace_path.exists());
    let kinds = misc_kinds(fx.events.as_ref(), &session_id).await;
    assert!(kinds.iter().any(|k| k == "sandbox_cleaned"));

    // Sanity: a 6-day-old failed task in the same fixture must NOT be
    // touched (boundary check).
    let young_id = seed_task(&fx, TaskStatus::Failed, now_micros() - micros_from_days(6)).await;
    let young_session = seed_session(&fx, &young_id).await;
    let young_path = fx.sandbox.insert_handle(&young_session).await;
    let report2 = cron.cleanup_cycle().await;
    assert_eq!(report2.cleaned, 0, "6-day-old must not match 7d window");
    assert!(young_path.exists());
}

// ---------- AC: missing container + missing workspace ---------------

#[tokio::test]
async fn ttl_handles_missing_container_gracefully() {
    let fx = fixture().await;
    let aged = now_micros() - micros_from_days(31);
    let task_id = seed_task(&fx, TaskStatus::Completed, aged).await;
    let session_id = seed_session(&fx, &task_id).await;
    // DO NOT insert a handle and DO NOT create the workspace dir:
    // simulates the cross-process restart case (Phase 2 DEBT #27) where
    // the handle cache is empty + a prior cleanup already removed disk
    // state. Both destroy() and rmdir must no-op (404 / ENOENT) and the
    // event must still be a clean `sandbox_cleaned`, not a `_failed`.

    let cron = build_cron(&fx, test_config());
    let report = cron.cleanup_cycle().await;

    assert_eq!(
        report.cleaned, 1,
        "missing artifacts must still count as cleaned"
    );
    assert_eq!(report.failed, 0);
    let kinds = misc_kinds(fx.events.as_ref(), &session_id).await;
    assert!(
        kinds.iter().any(|k| k == "sandbox_cleaned"),
        "expected sandbox_cleaned (not _failed), got {kinds:?}",
    );
    assert!(
        !kinds.iter().any(|k| k == "sandbox_cleanup_failed"),
        "missing-artifact path must not surface as failure",
    );
}
