use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::audit::{AuditAction, AuditLogger, AuditRecord};
use crate::auth::{Action, AuthContext, AuthError, AuthResource, authorize};
use crate::db::DbPool;
use crate::events::{EventStore, EventType, NewEvent, sqlite::SqliteEventStore};
use crate::project::TaskStatus;
use crate::time::now_micros;

#[derive(Debug, Clone)]
pub struct HandoffRequest {
    pub task_id: String,
    pub to_user_email: String,
    /// Optional human-readable reason for the audit_log row.
    pub reason: Option<String>,
    /// Optimistic concurrency precondition — must match the row's current
    /// `updated_at` or the transition rejects with `StaleRevision`.
    pub expected_updated_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HandoffOutcome {
    pub task_id: String,
    pub from_user_id: String,
    pub to_user_id: String,
    pub previous_status: String,
    pub new_status: String,
    pub audit_log_id: String,
    pub task_paused_event_id: i64,
    pub updated_at: i64,
}

#[derive(Debug, Error)]
pub enum HandoffError {
    #[error("auth: {0}")]
    Auth(#[from] AuthError),
    #[error("task not found: {0}")]
    TaskNotFound(String),
    #[error("user not found for email: {0}")]
    UserNotFound(String),
    #[error("task is in a terminal state and cannot be handed off: {0}")]
    TerminalState(String),
    #[error("task is running; pause it first (Phase 5 §5 / pause→transfer→resume): {0}")]
    MustPauseFirst(String),
    #[error("stale_revision: expected updated_at {expected} but row has {actual}")]
    StaleRevision { expected: i64, actual: i64 },
    #[error("invalid task status in DB: {0}")]
    InvalidStatus(String),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("event store: {0}")]
    Event(#[from] crate::events::EventError),
}

#[derive(Clone)]
pub struct TaskHandoffService {
    db: DbPool,
    events: std::sync::Arc<SqliteEventStore>,
    audit: AuditLogger,
}

impl TaskHandoffService {
    pub fn new(db: DbPool, events: std::sync::Arc<SqliteEventStore>, audit: AuditLogger) -> Self {
        Self { db, events, audit }
    }

    /// Cheap predicate — returns Ok(()) if the given task is in a state
    /// that admits direct reassignment (no pause first needed).
    pub async fn can_handoff(&self, task_id: &str) -> Result<bool, HandoffError> {
        let task_id = task_id.to_string();
        let status_str: Option<String> = self
            .db
            .with_conn(move |conn| {
                conn.query_row(
                    "SELECT status FROM tasks WHERE id = ?",
                    params![task_id],
                    |r| r.get::<_, String>(0),
                )
                .optional()
            })
            .await?;
        let Some(status_str) = status_str else {
            return Ok(false);
        };
        let status = TaskStatus::from_db_str(&status_str)
            .map_err(|_| HandoffError::InvalidStatus(status_str.clone()))?;
        Ok(matches!(
            status,
            TaskStatus::Drafted | TaskStatus::Briefed | TaskStatus::Confirmed | TaskStatus::Paused
        ))
    }

    /// Execute one hand-off. Returns the canonical `HandoffOutcome` for the
    /// caller to surface (CLI prints the audit_log id; HTTP returns the
    /// outcome JSON).
    pub async fn handoff(
        &self,
        auth: &AuthContext,
        req: HandoffRequest,
    ) -> Result<HandoffOutcome, HandoffError> {
        // §4.2 hybrid: policy engine gates the operation regardless of
        // surface (HTTP middleware already pre-checked, but the worker /
        // CLI paths land here without that pre-check).
        //
        // `is_same_org: true` is sound, not a stub (hardening
        // P5-HARD-IT1-M2): `organizations.tenant_id` is UNIQUE (V013), so
        // one tenant maps to exactly one org. The target-user lookup
        // below is tenant-scoped (`WHERE tenant_id = ? AND email = ?`), so
        // a resolvable target is necessarily in the actor's org. There is
        // no cross-org-within-tenant case to guard against.
        authorize(
            Action::TaskHandoff,
            &AuthResource {
                is_same_org: true,
                actor_can_share: true,
            },
            auth,
        )?;

        let task_id = req.task_id.clone();
        let to_email = req.to_user_email.clone();
        let reason = req.reason.clone();
        let expected_updated_at = req.expected_updated_at;
        let tenant = auth.tenant_id.clone();

        // Append the `task_paused_for_handoff` event OUTSIDE the transaction
        // — `EventStore::append` is an async fn that grabs its own
        // connection via `DbPool::with_conn`, so nesting it inside our
        // `with_conn` block would deadlock the single-connection pool. We
        // append after the transaction succeeds; the event lands at the
        // shared session_id derived from the task.
        let outcome_inner = self
            .db
            .with_conn(move |conn| -> Result<HandoffOutcomeInner, HandoffError> {
                let tx = conn.transaction()?;
                // 1. Resolve task row (status + updated_at + owner + project).
                //    Hardening P5-HARD-IT2-H3 (caught by the M4 cross-tenant
                //    write probe): the lookup MUST be tenant-scoped. Without
                //    `AND tenant_id = ?` a caller in tenant A could hand off
                //    tenant B's task (the id is globally unique but the row
                //    belongs to another tenant) and mutate its ownership —
                //    a direct NFR-5.1 cross-tenant write. A foreign task-id
                //    now surfaces as TaskNotFound, identical to a missing id.
                let row: Option<(String, i64, Option<String>, String)> = tx
                    .query_row(
                        "SELECT status, updated_at, owner_user_id, project_id
                         FROM tasks WHERE id = ? AND tenant_id = ?",
                        params![task_id, tenant],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                    )
                    .optional()?;
                let Some((status_str, updated_at, current_owner_opt, _project_id)) = row else {
                    return Err(HandoffError::TaskNotFound(task_id.clone()));
                };
                let status = TaskStatus::from_db_str(&status_str)
                    .map_err(|_| HandoffError::InvalidStatus(status_str.clone()))?;
                // 2. State gate.
                match status {
                    TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled => {
                        return Err(HandoffError::TerminalState(status_str.clone()));
                    }
                    TaskStatus::Running => {
                        return Err(HandoffError::MustPauseFirst(task_id.clone()));
                    }
                    _ => {}
                }
                // 3. Optimistic concurrency check.
                if let Some(expected) = expected_updated_at
                    && expected != updated_at
                {
                    return Err(HandoffError::StaleRevision {
                        expected,
                        actual: updated_at,
                    });
                }
                // 4. Resolve target user id from email (tenant-scoped).
                let to_user_id: String = tx
                    .query_row(
                        "SELECT id FROM users WHERE tenant_id = ? AND email = ?",
                        params![tenant, to_email],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()?
                    .ok_or_else(|| HandoffError::UserNotFound(to_email.clone()))?;
                let from_user_id = current_owner_opt.unwrap_or_else(|| "user-legacy-admin".into());
                let now = now_micros();
                // 5. Update task owner + updated_at.
                tx.execute(
                    "UPDATE tasks SET owner_user_id = ?, updated_at = ? WHERE id = ?",
                    params![to_user_id, now, task_id],
                )?;
                tx.commit()?;
                Ok(HandoffOutcomeInner {
                    task_id: task_id.clone(),
                    from_user_id,
                    to_user_id,
                    previous_status: status_str.clone(),
                    new_status: status_str,
                    updated_at: now,
                })
            })
            .await?;

        // 6. Write immutable audit row through the canonical service after
        //    task transaction commit. This mirrors the post-commit event append
        //    pattern and keeps a single writer shape for audit_log.
        let audit_log_id = self
            .audit
            .record(
                auth,
                AuditRecord {
                    action: AuditAction::TaskHandoff,
                    resource_type: "task",
                    resource_id: &outcome_inner.task_id,
                    target_user_id: Some(&outcome_inner.to_user_id),
                    decision: Some("allow"),
                    reason: reason.as_deref(),
                    metadata: serde_json::json!({
                        "task_id": outcome_inner.task_id,
                        "previous_status": outcome_inner.previous_status,
                        "reason": reason,
                    }),
                },
            )
            .await
            .map_err(|e| match e {
                crate::audit::AuditWriteError::Sqlite(err) => HandoffError::Sqlite(err),
                crate::audit::AuditWriteError::Event(err) => HandoffError::Event(err),
                crate::audit::AuditWriteError::Json(err) => {
                    HandoffError::Sqlite(rusqlite::Error::ToSqlConversionFailure(Box::new(err)))
                }
            })?;

        // 7. Append the task_paused_for_handoff event (post-transaction so
        //    the single-connection pool doesn't deadlock — see note above).
        //    Session id derives from the task's most-recent session row;
        //    if none exists (drafted task never ran) we synthesize a
        //    per-task session id so the event still lands in a queryable
        //    timeline.
        let session_id = self.derive_session_id(&outcome_inner.task_id).await?;
        let event = self
            .events
            .append(NewEvent {
                session_id,
                event_type: EventType::Misc,
                source: "handoff".to_string(),
                data: serde_json::json!({
                    "kind": "task_paused_for_handoff",
                    "task_id": outcome_inner.task_id,
                    "from_user_id": outcome_inner.from_user_id,
                    "to_user_id": outcome_inner.to_user_id,
                    "audit_log_id": audit_log_id,
                }),
            })
            .await?;

        Ok(HandoffOutcome {
            task_id: outcome_inner.task_id,
            from_user_id: outcome_inner.from_user_id,
            to_user_id: outcome_inner.to_user_id,
            previous_status: outcome_inner.previous_status,
            new_status: outcome_inner.new_status,
            audit_log_id,
            task_paused_event_id: event.id,
            updated_at: outcome_inner.updated_at,
        })
    }

    async fn derive_session_id(&self, task_id: &str) -> Result<String, HandoffError> {
        let task_id_for_lookup = task_id.to_string();
        let session_id: Option<String> = self
            .db
            .with_conn(move |conn| {
                conn.query_row(
                    "SELECT id FROM sessions WHERE task_id = ? ORDER BY created_at DESC LIMIT 1",
                    params![task_id_for_lookup],
                    |r| r.get::<_, String>(0),
                )
                .optional()
            })
            .await?;
        if let Some(sid) = session_id {
            return Ok(sid);
        }
        // Synthesize + insert a marker session row so the FK on events
        // resolves. Idempotent via INSERT OR IGNORE: if two concurrent
        // hand-offs of the same never-run task race, only the first row
        // sticks.
        let synthetic = format!("handoff-sess-{task_id}");
        let synthetic_for_move = synthetic.clone();
        let now = now_micros();
        self.db
            .with_conn(move |conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO sessions (id, created_at, updated_at, state)
                     VALUES (?, ?, ?, 'IDLE')",
                    params![synthetic_for_move, now, now],
                )?;
                Ok::<(), rusqlite::Error>(())
            })
            .await?;
        Ok(synthetic)
    }
}

struct HandoffOutcomeInner {
    task_id: String,
    from_user_id: String,
    to_user_id: String,
    previous_status: String,
    new_status: String,
    updated_at: i64,
}
