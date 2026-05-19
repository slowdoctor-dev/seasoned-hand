//! Workspace TTL cleanup cron (Phase 0 DEBT #16, Phase 2 story 2.17).
//!
//! `SandboxClient::destroy` removes the container but leaves
//! `{workspace_root}/{session_id}/` on disk forever. This module wakes
//! every hour (default), walks the `tasks` table for terminal-state
//! rows older than the configured TTL, and:
//!
//! 1. tears down the most-recent Session's docker container,
//! 2. recursively removes the on-disk workspace directory,
//! 3. emits a `sandbox_cleaned` Misc event against the OLD session, and
//! 4. bumps `tasks.updated_at` so the row doesn't re-match the next
//!    cycle's TTL window immediately. On a future cycle the row will be
//!    picked again — the destroy/rmdir calls are idempotent (404 OK,
//!    ENOENT OK) so the re-pick is a cheap no-op until something else
//!    mutates the task.
//!
//! Active tasks (`running`, `paused`) are NEVER GC'd. The SQL `WHERE`
//! excludes them up-front; a second-look re-read of the row's status
//! inside the loop guards against a resume-during-tick race.
//!
//! Failures inside a single candidate (docker error, fs removal error)
//! are logged + emitted as `sandbox_cleanup_failed` Misc on the OLD
//! session and never crash the cron — one bad candidate must not block
//! the rest of the cycle.
//!
//! refs: /specs/phase-2/architecture.md §6
//! refs: /specs/phase-2/stories/story-2.17.md
//! refs: /specs/phase-0/DEBT.md #16

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::params;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::db::DbPool;
use crate::events::{EventStore, EventType, NewEvent, sqlite::SqliteEventStore};
use crate::project::{TaskError, TaskStatus, TaskStore};
use crate::sandbox::{SandboxClient, SandboxError, SandboxHandle, is_safe_session_id};

/// Minimal lifecycle surface the cron needs from the sandbox layer.
/// Production impl is on [`SandboxClient`]; tests substitute a fake so
/// the cleanup loop runs without docker.
#[allow(async_fn_in_trait)]
pub trait SandboxJanitor: Send + Sync {
    async fn get_handle(&self, session_id: &str) -> Option<SandboxHandle>;
    async fn destroy(&self, session_id: &str) -> Result<(), SandboxError>;
    fn workspace_root(&self) -> &Path;
}

impl SandboxJanitor for SandboxClient {
    async fn get_handle(&self, session_id: &str) -> Option<SandboxHandle> {
        Self::get(self, session_id).await
    }
    async fn destroy(&self, session_id: &str) -> Result<(), SandboxError> {
        Self::destroy(self, session_id).await
    }
    fn workspace_root(&self) -> &Path {
        Self::workspace_root(self).as_path()
    }
}

/// Per-status TTL window + cycle interval. Constructed via
/// [`TtlConfig::defaults`] or [`TtlConfig::from_env`]; tests bypass both
/// and construct the struct literal so they can dial timings down.
#[derive(Debug, Clone)]
pub struct TtlConfig {
    pub interval: Duration,
    pub completed_ttl: Duration,
    pub failed_cancelled_ttl: Duration,
    pub draft_ttl: Duration,
}

impl TtlConfig {
    /// Architecture §6 defaults: cycle hourly, completed 30 d, failed /
    /// cancelled 7 d, drafted / briefed 1 d. `running` / `paused` aren't
    /// listed — they're never GC'd (durable pause's replay rebuild path
    /// from story 2.16 made the prior "Paused: 7-day" rule unsafe).
    pub fn defaults() -> Self {
        Self {
            interval: Duration::from_secs(3600),
            completed_ttl: Duration::from_secs(30 * 86_400),
            failed_cancelled_ttl: Duration::from_secs(7 * 86_400),
            draft_ttl: Duration::from_secs(86_400),
        }
    }

    /// Read every override from env. Missing / unparseable falls back to
    /// the matching default. Zero is allowed (lets operators force a
    /// drain on a stale install).
    pub fn from_env() -> Self {
        let mut cfg = Self::defaults();
        if let Some(secs) = env_secs("SANDBOX_CLEANUP_INTERVAL_SEC") {
            cfg.interval = Duration::from_secs(secs);
        }
        if let Some(days) = env_days("SANDBOX_TTL_COMPLETED_DAYS") {
            cfg.completed_ttl = Duration::from_secs(days * 86_400);
        }
        if let Some(days) = env_days("SANDBOX_TTL_FAILED_CANCELLED_DAYS") {
            cfg.failed_cancelled_ttl = Duration::from_secs(days * 86_400);
        }
        if let Some(days) = env_days("SANDBOX_TTL_DRAFT_DAYS") {
            cfg.draft_ttl = Duration::from_secs(days * 86_400);
        }
        cfg
    }

    fn ttl_for(&self, status: TaskStatus) -> Option<Duration> {
        match status {
            TaskStatus::Completed => Some(self.completed_ttl),
            TaskStatus::Failed | TaskStatus::Cancelled => Some(self.failed_cancelled_ttl),
            TaskStatus::Drafted | TaskStatus::Briefed => Some(self.draft_ttl),
            // `running`, `paused`, `confirmed` are never GC'd. `confirmed`
            // never reached the running gate so technically has no
            // workspace yet, but we skip it for safety regardless.
            _ => None,
        }
    }
}

fn env_secs(key: &str) -> Option<u64> {
    std::env::var(key).ok().and_then(|v| v.parse().ok())
}

fn env_days(key: &str) -> Option<u64> {
    std::env::var(key).ok().and_then(|v| v.parse().ok())
}

/// Summary returned by [`WorkspaceTtlCron::cleanup_cycle`]. Surfaces in
/// the admin endpoint's JSON response.
#[derive(Debug, Default, Clone, Copy, serde::Serialize)]
pub struct TtlCleanupReport {
    pub cleaned: usize,
    pub failed: usize,
}

/// Bundle of refs the cron needs. All `Arc`-shaped so the cron can be
/// constructed once at boot and `clone()`d into `tokio::spawn` plus the
/// admin route handler.
#[derive(Clone)]
pub struct WorkspaceTtlCron<S: SandboxJanitor> {
    task_store: Arc<TaskStore>,
    events: Arc<SqliteEventStore>,
    sandbox: Arc<S>,
    db: DbPool,
    config: TtlConfig,
}

impl<S: SandboxJanitor + 'static> WorkspaceTtlCron<S> {
    pub fn new(
        task_store: Arc<TaskStore>,
        events: Arc<SqliteEventStore>,
        sandbox: Arc<S>,
        db: DbPool,
        config: TtlConfig,
    ) -> Self {
        Self {
            task_store,
            events,
            sandbox,
            db,
            config,
        }
    }

    pub fn config(&self) -> &TtlConfig {
        &self.config
    }

    /// Loop until `shutdown` fires. Each tick runs one [`cleanup_cycle`]
    /// then sleeps `interval`. Cycle errors are swallowed (already
    /// logged + emitted per-candidate); the loop itself never returns
    /// Err so the spawn-and-await pattern in `main.rs` mirrors the
    /// existing NotifyWorker / CheckpointManager shape.
    pub async fn run(&self, shutdown: CancellationToken) {
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return,
                _ = tokio::time::sleep(self.config.interval) => {
                    let report = self.cleanup_cycle().await;
                    if report.cleaned > 0 || report.failed > 0 {
                        tracing::info!(
                            cleaned = report.cleaned,
                            failed = report.failed,
                            "workspace ttl cycle complete",
                        );
                    }
                }
            }
        }
    }

    /// Run one cycle: pick every (status, updated_at) candidate, GC the
    /// matching session's container + workspace, bump `updated_at`.
    /// Returns counts; per-candidate errors are absorbed into `failed`.
    pub async fn cleanup_cycle(&self) -> TtlCleanupReport {
        let mut report = TtlCleanupReport::default();
        let now = now_micros();
        let candidates = match self.collect_candidates(now).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "workspace ttl: candidate query failed");
                return report;
            }
        };
        for cand in candidates {
            match self.clean_one(&cand).await {
                CleanupOutcome::Cleaned => report.cleaned += 1,
                CleanupOutcome::Skipped => {}
                CleanupOutcome::Failed => report.failed += 1,
            }
        }
        report
    }

    async fn collect_candidates(&self, now: i64) -> rusqlite::Result<Vec<Candidate>> {
        let completed_cutoff = saturating_cutoff(now, self.config.completed_ttl);
        let failed_cutoff = saturating_cutoff(now, self.config.failed_cancelled_ttl);
        let draft_cutoff = saturating_cutoff(now, self.config.draft_ttl);
        self.db
            .with_conn(move |conn| -> rusqlite::Result<Vec<Candidate>> {
                let mut stmt = conn.prepare(
                    "SELECT id, status, updated_at FROM tasks \
                      WHERE (status = 'completed' AND updated_at < ?) \
                         OR (status IN ('failed', 'cancelled') AND updated_at < ?) \
                         OR (status IN ('drafted', 'briefed') AND updated_at < ?)",
                )?;
                let rows = stmt.query_map(
                    params![completed_cutoff, failed_cutoff, draft_cutoff],
                    |row| {
                        Ok(Candidate {
                            task_id: row.get::<_, String>(0)?,
                            status_raw: row.get::<_, String>(1)?,
                            updated_at: row.get::<_, i64>(2)?,
                        })
                    },
                )?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    async fn clean_one(&self, cand: &Candidate) -> CleanupOutcome {
        // Race guard: re-read the row's current status. If a user
        // resume() flipped it back to running/paused between the SELECT
        // above and this point, skip — never touch active work. There's
        // no global lock; the cron and a WS resume can interleave.
        let live = match self.task_store.get(&cand.task_id).await {
            Ok(t) => t,
            Err(TaskError::NotFound(_)) => return CleanupOutcome::Skipped,
            Err(e) => {
                tracing::warn!(task_id = %cand.task_id, error = %e, "workspace ttl: re-read failed");
                return CleanupOutcome::Skipped;
            }
        };
        if self.config.ttl_for(live.status).is_none() {
            return CleanupOutcome::Skipped;
        }
        let reason = format!("ttl_{}", live.status.as_db_str());

        let session_id = match self.latest_session(&cand.task_id).await {
            Ok(Some(id)) => id,
            Ok(None) => {
                // No session ever existed (drafted/briefed before the
                // initializer ran). Still bump updated_at so we don't
                // re-scan on the next cycle.
                if let Err(error) = self.bump_updated_at(&cand.task_id).await {
                    tracing::warn!(
                        task_id = %cand.task_id,
                        %error,
                        "workspace ttl: failed to bump updated_at for task with no session",
                    );
                }
                return CleanupOutcome::Skipped;
            }
            Err(e) => {
                tracing::warn!(task_id = %cand.task_id, error = %e, "workspace ttl: session lookup failed");
                return CleanupOutcome::Failed;
            }
        };

        // Capture the workspace path BEFORE destroy clears the in-memory
        // handle. Cross-process restart (Phase 2 DEBT #27) loses the
        // cache, so fall back to `workspace_root/{session_id}` — the
        // canonical path Phase 0 0.8 wrote during create().
        //
        // Phase 4 security hardening iter-2 F1: validate session_id before
        // the fallback join. Intake already filters via `is_safe_session_id`
        // but defense-in-depth here protects against a future bypass —
        // a `..` segment in `sessions.id` (whatever its provenance) must
        // never resolve `remove_dir_all` outside `workspace_root`.
        let workspace_path = match self.sandbox.get_handle(&session_id).await {
            Some(h) => h.workspace_host_path,
            None => {
                if !is_safe_session_id(&session_id) {
                    tracing::warn!(
                        task_id = %cand.task_id,
                        session_id = %session_id,
                        "ttl cron: unsafe session_id rejected; skipping rmdir",
                    );
                    return CleanupOutcome::Skipped;
                }
                self.sandbox.workspace_root().join(&session_id)
            }
        };

        let destroy_err = self.sandbox.destroy(&session_id).await.err();
        let rmdir_err = remove_workspace(&workspace_path).await.err();
        let bump_err = self.bump_updated_at(&cand.task_id).await;

        let combined = match (destroy_err, rmdir_err) {
            (None, None) => None,
            (Some(d), None) => Some(format!("destroy: {d}")),
            (None, Some(r)) => Some(format!("rmdir: {r}")),
            (Some(d), Some(r)) => Some(format!("destroy: {d}; rmdir: {r}")),
        };

        // Bump-failure is its own warning but doesn't change the
        // cleanup outcome — the destroy/rmdir already ran. Next cycle
        // will re-pick the row, which is harmless (idempotent).
        if let Err(e) = bump_err {
            tracing::warn!(task_id = %cand.task_id, error = %e, "workspace ttl: updated_at bump failed");
        }

        match combined {
            None => {
                let _ = self
                    .events
                    .append(NewEvent {
                        session_id: session_id.clone(),
                        event_type: EventType::Misc,
                        source: "task::ttl".into(),
                        data: json!({
                            "kind": "sandbox_cleaned",
                            "session_id": session_id,
                            "task_id": cand.task_id,
                            "reason": reason,
                        }),
                    })
                    .await;
                CleanupOutcome::Cleaned
            }
            Some(err_text) => {
                tracing::warn!(
                    session_id = %session_id,
                    task_id = %cand.task_id,
                    error = %err_text,
                    "workspace ttl: cleanup failed",
                );
                let _ = self
                    .events
                    .append(NewEvent {
                        session_id: session_id.clone(),
                        event_type: EventType::Misc,
                        source: "task::ttl".into(),
                        data: json!({
                            "kind": "sandbox_cleanup_failed",
                            "session_id": session_id,
                            "task_id": cand.task_id,
                            "error": err_text,
                        }),
                    })
                    .await;
                CleanupOutcome::Failed
            }
        }
    }

    async fn latest_session(&self, task_id: &str) -> rusqlite::Result<Option<String>> {
        use rusqlite::OptionalExtension;
        let tid = task_id.to_string();
        self.db
            .with_conn(move |conn| {
                conn.query_row(
                    "SELECT id FROM sessions WHERE task_id = ? \
                      ORDER BY created_at DESC LIMIT 1",
                    [&tid],
                    |row| row.get::<_, String>(0),
                )
                .optional()
            })
            .await
    }

    async fn bump_updated_at(&self, task_id: &str) -> rusqlite::Result<()> {
        let tid = task_id.to_string();
        let now = now_micros();
        self.db
            .with_conn(move |conn| {
                conn.execute(
                    "UPDATE tasks SET updated_at = ? WHERE id = ?",
                    params![now, tid],
                )?;
                Ok(())
            })
            .await
    }
}

#[derive(Debug, Clone)]
struct Candidate {
    task_id: String,
    #[allow(dead_code)] // kept for tracing context; status is re-read in clean_one
    status_raw: String,
    #[allow(dead_code)]
    updated_at: i64,
}

enum CleanupOutcome {
    Cleaned,
    Skipped,
    Failed,
}

async fn remove_workspace(path: &Path) -> std::io::Result<()> {
    match tokio::fs::remove_dir_all(path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn saturating_cutoff(now: i64, ttl: Duration) -> i64 {
    let ttl_micros = i64::try_from(ttl.as_micros()).unwrap_or(i64::MAX);
    now.saturating_sub(ttl_micros)
}

fn now_micros() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_micros()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
