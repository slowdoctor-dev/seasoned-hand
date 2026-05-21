//! User deactivation + mandatory reassignment (Phase 5 story 5.20).
//!
//! Per architecture §12: an active user cannot be deactivated while
//! they still own work. Deactivation requires a `--reassign-to` target
//! who absorbs all the source user's active task ownership and
//! owner-level SOP/playbook shares. Historical audit attribution is
//! preserved (we never rewrite past `audit_log.actor_user_id` rows);
//! the deactivation itself emits its own audit event for the lifecycle
//! transition.
//!
//! Reassignment of active tasks goes through [`TaskHandoffService`] so
//! the state machine in story 5.9 (Drafted/Briefed/Confirmed/Paused →
//! direct, Running → MustPauseFirst, terminal → TerminalState) governs
//! each move. If any task can't be handed off (e.g. it's Running),
//! the whole deactivation aborts and the source user stays active.
//!
//! refs: /specs/phase-5/architecture.md §12
//! refs: /specs/phase-5/stories/story-5.20.md

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::audit::{AuditAction, AuditLogger, AuditRecord, AuditWriteError};
use crate::auth::{Action, AuthContext, AuthError, AuthResource, authorize};
use crate::db::DbPool;
use crate::handoff::{HandoffError, HandoffRequest, TaskHandoffService};
use crate::time::now_micros;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeactivationOutcome {
    pub source_user_id: String,
    pub target_user_id: String,
    pub tasks_reassigned: usize,
    pub sop_shares_transferred: usize,
    pub playbook_shares_transferred: usize,
    pub audit_log_id: String,
}

#[derive(Debug, Error)]
pub enum DeactivationError {
    #[error("auth: {0}")]
    Auth(#[from] AuthError),
    #[error("source user not found for email: {0}")]
    SourceNotFound(String),
    #[error("target user not found for email: {0}")]
    TargetNotFound(String),
    #[error("source and target must differ")]
    SameUser,
    #[error("target user not in same org as source")]
    CrossOrgTarget,
    #[error(
        "source has {active_tasks} active tasks but no --reassign-to was given (or reassignment failed mid-flight; see cause)"
    )]
    ReassignRequired { active_tasks: usize },
    #[error("handoff failed for task {task_id}: {source}")]
    HandoffFailed {
        task_id: String,
        #[source]
        source: HandoffError,
    },
    #[error("source user is already deactivated")]
    AlreadyDeactivated,
    #[error(
        "refusing to deactivate the last active admin of org {organization_id} (would lock the org out of all admin-gated actions)"
    )]
    LastAdminLockout { organization_id: String },
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("audit: {0}")]
    Audit(#[from] AuditWriteError),
}

#[derive(Clone)]
pub struct UserDeactivationService {
    db: DbPool,
    audit: AuditLogger,
    handoff: TaskHandoffService,
}

impl UserDeactivationService {
    pub fn new(db: DbPool, audit: AuditLogger, handoff: TaskHandoffService) -> Self {
        Self { db, audit, handoff }
    }

    /// Deactivate `source_email` reassigning all active assets to
    /// `target_email`. Returns the outcome on success; on any failure
    /// the source user remains `active` (partial reassignments may have
    /// landed — see comment below).
    ///
    /// Partial-failure caveat: per-task hand-offs go through the
    /// existing [`TaskHandoffService`] one at a time. If task N reassigns
    /// fine but task N+1 fails (e.g. it's Running), tasks 1..N stay
    /// reassigned. This is the correct posture: re-running the
    /// deactivation after fixing task N+1 picks up where we left off
    /// rather than re-shuffling tasks that already moved. The audit_log
    /// row is only emitted on full success, so operators see a clean
    /// "deactivated" mark or no mark at all.
    pub async fn deactivate(
        &self,
        auth: &AuthContext,
        source_email: &str,
        target_email: &str,
        reason: Option<&str>,
    ) -> Result<DeactivationOutcome, DeactivationError> {
        authorize(
            Action::MembershipManage,
            &AuthResource {
                is_same_org: true,
                actor_can_share: true,
            },
            auth,
        )?;
        if source_email == target_email {
            return Err(DeactivationError::SameUser);
        }

        let tenant = auth.tenant_id.clone();
        let source_email_owned = source_email.to_string();
        let target_email_owned = target_email.to_string();
        let (source_user_id, source_status, target_user_id, target_org_id, source_org_id) = self
            .db
            .with_conn(move |conn| {
                let source: Option<(String, String, String)> = conn
                    .query_row(
                        "SELECT u.id, u.status, m.organization_id
                         FROM users u
                         JOIN organization_memberships m
                           ON m.user_id = u.id AND m.is_primary = 1
                         WHERE u.email = ? AND u.tenant_id = ?",
                        params![source_email_owned, tenant],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()?;
                let target: Option<(String, String)> = conn
                    .query_row(
                        "SELECT u.id, m.organization_id
                         FROM users u
                         JOIN organization_memberships m
                           ON m.user_id = u.id AND m.is_primary = 1
                         WHERE u.email = ? AND u.tenant_id = ?",
                        params![target_email_owned, tenant],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?;
                Ok::<_, rusqlite::Error>((source, target))
            })
            .await
            .map(|(source, target)| {
                let (sid, sstatus, sorg) = source
                    .ok_or_else(|| DeactivationError::SourceNotFound(source_email.to_string()))?;
                let (tid, torg) = target
                    .ok_or_else(|| DeactivationError::TargetNotFound(target_email.to_string()))?;
                Ok::<_, DeactivationError>((sid, sstatus, tid, torg, sorg))
            })??;

        if source_status == "deactivated" {
            return Err(DeactivationError::AlreadyDeactivated);
        }
        if source_org_id != target_org_id {
            return Err(DeactivationError::CrossOrgTarget);
        }

        // 0.5. Last-admin lockout guard (P5-HARD-IT1-M1). If the source
        // is an admin and removing them would leave the org with zero
        // active admins, refuse — otherwise the org loses access to
        // every admin-gated action (MembershipManage / EventRawRead /
        // audit-admin) with no recovery path.
        let lockout_org = source_org_id.clone();
        let lockout_source = source_user_id.clone();
        let would_lock_out = self
            .db
            .with_conn(move |conn| {
                let source_role: Option<String> = conn
                    .query_row(
                        "SELECT role FROM organization_memberships
                         WHERE organization_id = ? AND user_id = ?",
                        params![lockout_org, lockout_source],
                        |r| r.get(0),
                    )
                    .optional()?;
                if source_role.as_deref() != Some("admin") {
                    return Ok::<bool, rusqlite::Error>(false);
                }
                let other_active_admins: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM organization_memberships m
                     JOIN users u ON u.id = m.user_id
                     WHERE m.organization_id = ? AND m.role = 'admin'
                       AND u.status = 'active' AND m.user_id != ?",
                    params![lockout_org, lockout_source],
                    |r| r.get(0),
                )?;
                Ok(other_active_admins == 0)
            })
            .await?;
        if would_lock_out {
            return Err(DeactivationError::LastAdminLockout {
                organization_id: source_org_id.clone(),
            });
        }

        // 1. Find every active task owned by source — these need hand-off.
        let source_id_for_query = source_user_id.clone();
        let active_tasks: Vec<(String, i64)> = self
            .db
            .with_conn(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, updated_at FROM tasks
                     WHERE owner_user_id = ?
                       AND status NOT IN ('completed','failed','cancelled',
                                           'Completed','Failed','Cancelled')",
                )?;
                let rows = stmt
                    .query_map(params![source_id_for_query], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok::<Vec<(String, i64)>, rusqlite::Error>(rows)
            })
            .await?;

        // 2. Hand off each active task to the target via the 5.9
        //    service so the state machine + audit emission run as
        //    designed. Each handoff() call is its own transaction +
        //    audit row — partial failures leave tasks already reassigned
        //    in-place (see method-level comment above).
        let mut tasks_reassigned = 0usize;
        for (task_id, expected) in &active_tasks {
            self.handoff
                .handoff(
                    auth,
                    HandoffRequest {
                        task_id: task_id.clone(),
                        to_user_email: target_email.to_string(),
                        reason: Some(format!(
                            "user_deactivation: {} -> {}",
                            source_email, target_email
                        )),
                        expected_updated_at: Some(*expected),
                    },
                )
                .await
                .map_err(|source| DeactivationError::HandoffFailed {
                    task_id: task_id.clone(),
                    source,
                })?;
            tasks_reassigned += 1;
        }

        // 3. Transfer owner-level shares (sop_shares + playbook_shares).
        //    Shares are governed by `granted_by_user_id`; we rewrite
        //    that pointer rather than re-issuing the share so the
        //    `created_at` timestamp + visibility_state survive.
        let source_id_for_shares = source_user_id.clone();
        let target_id_for_shares = target_user_id.clone();
        // P5-HARD-IT1-H2: scope the rewrite to the caller's tenant. Every
        // mutating statement in Phase 5 carries a tenant predicate; this
        // one must too, so the rewrite can never cross a tenant boundary
        // even if a user-id namespace ever collided across tenants.
        let shares_tenant = auth.tenant_id.clone();
        let (sop_shares_transferred, playbook_shares_transferred): (usize, usize) = self
            .db
            .with_conn(move |conn| {
                let tx = conn.transaction()?;
                let sop_n = tx.execute(
                    "UPDATE sop_shares SET granted_by_user_id = ?
                     WHERE granted_by_user_id = ? AND tenant_id = ?",
                    params![target_id_for_shares, source_id_for_shares, shares_tenant],
                )?;
                let pb_n = tx.execute(
                    "UPDATE playbook_shares SET granted_by_user_id = ?
                     WHERE granted_by_user_id = ? AND tenant_id = ?",
                    params![target_id_for_shares, source_id_for_shares, shares_tenant],
                )?;
                tx.commit()?;
                Ok::<(usize, usize), rusqlite::Error>((sop_n, pb_n))
            })
            .await?;

        // 4. Flip users.status -> 'deactivated'.
        let source_id_for_flip = source_user_id.clone();
        let now = now_micros();
        self.db
            .with_conn(move |conn| {
                conn.execute(
                    "UPDATE users SET status = 'deactivated', updated_at = ? WHERE id = ?",
                    params![now, source_id_for_flip],
                )?;
                Ok::<(), rusqlite::Error>(())
            })
            .await?;

        // 5. Emit the lifecycle audit_log row. AuditLogger's dual-write
        //    Misc{kind:"audit_logged"} event lands automatically.
        let audit_log_id = self
            .audit
            .record(
                auth,
                AuditRecord {
                    action: AuditAction::UserDeactivate,
                    resource_type: "user",
                    resource_id: &source_user_id,
                    target_user_id: Some(&target_user_id),
                    decision: Some("deactivate"),
                    reason,
                    metadata: serde_json::json!({
                        "source_email": source_email,
                        "target_email": target_email,
                        "tasks_reassigned": tasks_reassigned,
                        "sop_shares_transferred": sop_shares_transferred,
                        "playbook_shares_transferred": playbook_shares_transferred,
                    }),
                },
            )
            .await?;

        Ok(DeactivationOutcome {
            source_user_id,
            target_user_id,
            tasks_reassigned,
            sop_shares_transferred,
            playbook_shares_transferred,
            audit_log_id,
        })
    }
}

#[cfg(test)]
mod tests;
