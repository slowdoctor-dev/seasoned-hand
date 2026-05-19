//! Curator worker runtime scaffold for Phase 4.
//! refs: /specs/phase-4/architecture.md §2.1, §4.1, §6.5, §7

use std::sync::Arc;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde_json::json;
use thiserror::Error;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

use crate::db::DbPool;
use crate::events::{EventStore, EventType, NewEvent, sqlite::SqliteEventStore};

#[derive(Debug, Clone)]
pub struct CuratorConfig {
    pub enabled: bool,
    pub interval_seconds: u64,
    pub backlog_threshold: u32,
    pub project_id: String,
}

impl Default for CuratorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_seconds: 300,
            backlog_threshold: 10,
            project_id: "default".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CuratorTrigger {
    IntervalTick,
    BacklogThreshold,
    Manual,
}

impl CuratorTrigger {
    fn as_str(self) -> &'static str {
        match self {
            CuratorTrigger::IntervalTick => "interval_tick",
            CuratorTrigger::BacklogThreshold => "backlog_threshold",
            CuratorTrigger::Manual => "manual",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CuratorCycleResult {
    pub cycle_id: String,
    pub project_id: String,
    pub decisions_total: u32,
    pub queued_for_review: u32,
    pub failures: u32,
    pub elapsed_ms: u64,
}

#[derive(Debug, Error)]
pub enum CuratorWorkerError {
    #[error("event: {0}")]
    Event(#[from] crate::events::EventError),
    #[error("db: {0}")]
    Db(#[from] crate::db::DbError),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("executor: {0}")]
    Executor(String),
}

#[async_trait]
pub trait CuratorCycleExecutor: Send + Sync {
    async fn execute(
        &self,
        project_id: &str,
        trigger: CuratorTrigger,
        backlog_count: u32,
    ) -> Result<CuratorCycleResult, CuratorWorkerError>;
}

#[async_trait]
pub trait BacklogProbe: Send + Sync {
    async fn pending_count(&self, project_id: &str) -> Result<u32, CuratorWorkerError>;
}

#[derive(Clone)]
pub struct SqliteBacklogProbe {
    db: DbPool,
}

impl SqliteBacklogProbe {
    pub fn new(db: DbPool) -> Self {
        Self { db }
    }
}

#[async_trait]
impl BacklogProbe for SqliteBacklogProbe {
    async fn pending_count(&self, project_id: &str) -> Result<u32, CuratorWorkerError> {
        let project_id = project_id.to_string();
        self.db
            .with_conn(move |conn| {
                conn.query_row(
                    "SELECT COUNT(*)
                     FROM playbook_revisions r
                     WHERE r.source_project_id = ?
                       AND NOT EXISTS (
                         SELECT 1
                         FROM curator_decisions d
                         WHERE d.subject_kind = 'revision'
                           AND d.subject_id = r.id
                           AND d.project_id = r.source_project_id
                       )",
                    [project_id],
                    |row| row.get::<_, u32>(0),
                )
                .map_err(CuratorWorkerError::from)
            })
            .await
    }
}

#[derive(Clone)]
pub struct NoopCycleExecutor;

#[async_trait]
impl CuratorCycleExecutor for NoopCycleExecutor {
    async fn execute(
        &self,
        project_id: &str,
        _trigger: CuratorTrigger,
        backlog_count: u32,
    ) -> Result<CuratorCycleResult, CuratorWorkerError> {
        Ok(CuratorCycleResult {
            cycle_id: format!("cycle-{}", uuid::Uuid::new_v4()),
            project_id: project_id.to_string(),
            decisions_total: backlog_count.min(50),
            queued_for_review: 0,
            failures: 0,
            elapsed_ms: 0,
        })
    }
}

#[derive(Clone)]
pub struct ProductionCuratorWorker {
    config: CuratorConfig,
    db: DbPool,
    events: Arc<SqliteEventStore>,
    backlog_probe: Arc<dyn BacklogProbe>,
    executor: Arc<dyn CuratorCycleExecutor>,
}

impl ProductionCuratorWorker {
    pub fn new(
        config: CuratorConfig,
        db: DbPool,
        events: Arc<SqliteEventStore>,
        backlog_probe: Arc<dyn BacklogProbe>,
        executor: Arc<dyn CuratorCycleExecutor>,
    ) -> Self {
        Self {
            config,
            db,
            events,
            backlog_probe,
            executor,
        }
    }

    pub async fn run(&self, cancel: CancellationToken) -> Result<(), CuratorWorkerError> {
        if !self.config.enabled {
            tracing::info!("curator worker disabled");
            return Ok(());
        }
        let interval = Duration::from_secs(self.config.interval_seconds.max(1));
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        ticker.tick().await;

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    tracing::info!("curator worker shutdown requested");
                    return Ok(());
                }
                _ = ticker.tick() => {
                    let backlog = match self.backlog_probe.pending_count(&self.config.project_id).await {
                        Ok(count) => count,
                        Err(error) => {
                            tracing::warn!(%error, "curator backlog probe failed");
                            continue;
                        }
                    };
                    let trigger = if backlog > self.config.backlog_threshold {
                        CuratorTrigger::BacklogThreshold
                    } else {
                        CuratorTrigger::IntervalTick
                    };
                    if let Err(error) = self.run_once(trigger, backlog).await {
                        tracing::warn!(%error, "curator cycle failed");
                    }
                }
            }
        }
    }

    pub async fn run_once(
        &self,
        trigger: CuratorTrigger,
        backlog_count: u32,
    ) -> Result<CuratorCycleResult, CuratorWorkerError> {
        let session_id = format!("curator:{}", self.config.project_id);
        self.ensure_session(&session_id).await?;
        self.events
            .append(NewEvent {
                session_id: session_id.clone(),
                event_type: EventType::Misc,
                source: "curator".to_string(),
                data: json!({
                    "kind": "curator_cycle_started",
                    "project_id": self.config.project_id,
                    "trigger": trigger.as_str(),
                    "backlog_count": backlog_count,
                    "backlog_threshold": self.config.backlog_threshold
                }),
            })
            .await?;
        let result = self
            .executor
            .execute(&self.config.project_id, trigger, backlog_count)
            .await?;
        self.events
            .append(NewEvent {
                session_id,
                event_type: EventType::Misc,
                source: "curator".to_string(),
                data: json!({
                    "kind": "curator_cycle_completed",
                    "project_id": result.project_id,
                    "cycle_id": result.cycle_id,
                    "trigger": trigger.as_str(),
                    "decisions_total": result.decisions_total,
                    "queued_for_review": result.queued_for_review,
                    "failures": result.failures,
                    "elapsed_ms": result.elapsed_ms
                }),
            })
            .await?;
        Ok(result)
    }

    async fn ensure_session(&self, session_id: &str) -> Result<(), CuratorWorkerError> {
        let session_id = session_id.to_string();
        let now_micros = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| CuratorWorkerError::Executor(error.to_string()))?
            .as_micros();
        let now_micros = i64::try_from(now_micros)
            .map_err(|error| CuratorWorkerError::Executor(error.to_string()))?;
        self.db
            .with_conn(move |conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO sessions (id, created_at, updated_at, state)
                     VALUES (?1, ?2, ?2, 'RUNNING')",
                    rusqlite::params![session_id, now_micros],
                )?;
                Ok::<_, CuratorWorkerError>(())
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::db;
    use crate::events::{EventQuery, EventType};

    struct StubExecutor {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl CuratorCycleExecutor for StubExecutor {
        async fn execute(
            &self,
            project_id: &str,
            _trigger: CuratorTrigger,
            backlog_count: u32,
        ) -> Result<CuratorCycleResult, CuratorWorkerError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(CuratorCycleResult {
                cycle_id: "cycle-test".to_string(),
                project_id: project_id.to_string(),
                decisions_total: backlog_count,
                queued_for_review: 0,
                failures: 0,
                elapsed_ms: 5,
            })
        }
    }

    struct StubBacklogProbe;

    #[async_trait]
    impl BacklogProbe for StubBacklogProbe {
        async fn pending_count(&self, _project_id: &str) -> Result<u32, CuratorWorkerError> {
            Ok(12)
        }
    }

    #[tokio::test]
    async fn run_once_emits_cycle_start_and_complete_events() {
        let db = db::open(":memory:").await.expect("db");
        let events = Arc::new(SqliteEventStore::new(db.clone()));
        let worker = ProductionCuratorWorker::new(
            CuratorConfig {
                enabled: true,
                interval_seconds: 1,
                backlog_threshold: 10,
                project_id: "proj-1".to_string(),
            },
            db,
            events.clone(),
            Arc::new(StubBacklogProbe),
            Arc::new(StubExecutor {
                calls: AtomicUsize::new(0),
            }),
        );

        let result = worker
            .run_once(CuratorTrigger::BacklogThreshold, 12)
            .await
            .expect("run_once");
        assert_eq!(result.project_id, "proj-1");
        assert_eq!(result.decisions_total, 12);

        let got = events
            .query(
                "curator:proj-1",
                EventQuery {
                    event_type: Some(EventType::Misc),
                    ..EventQuery::default()
                },
            )
            .await
            .expect("query");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].data["kind"], "curator_cycle_started");
        assert_eq!(got[1].data["kind"], "curator_cycle_completed");
    }
}
