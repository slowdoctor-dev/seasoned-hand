use rusqlite::params;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::auth::{Action, AuthContext, AuthError, AuthResource, Role, authorize};
use crate::db::DbPool;
use crate::events::{EventStore, EventType, NewEvent, sqlite::SqliteEventStore};
use crate::time::now_micros;

/// Canonical action names written to `audit_log.action`. Listed here so
/// new mutating stories pick a strongly-typed variant instead of inventing
/// strings — `to_db_str` is the only conversion site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    TaskHandoff,
    TaskCancel,
    SopShare,
    SopUnshare,
    PlaybookShare,
    PlaybookUnshare,
    PlaybookApprove,
    UserInvite,
    UserDeactivate,
    MembershipUpdate,
    EventRawRead,
}

impl AuditAction {
    fn to_db_str(self) -> &'static str {
        match self {
            Self::TaskHandoff => "task.handoff",
            Self::TaskCancel => "task.cancel",
            Self::SopShare => "sop.share",
            Self::SopUnshare => "sop.unshare",
            Self::PlaybookShare => "playbook.share",
            Self::PlaybookUnshare => "playbook.unshare",
            Self::PlaybookApprove => "playbook.approve",
            Self::UserInvite => "user.invite",
            Self::UserDeactivate => "user.deactivate",
            Self::MembershipUpdate => "membership.update",
            Self::EventRawRead => "event.raw_read",
        }
    }

    fn from_db_str(s: &str) -> Option<Self> {
        Some(match s {
            "task.handoff" => Self::TaskHandoff,
            "task.cancel" => Self::TaskCancel,
            "sop.share" => Self::SopShare,
            "sop.unshare" => Self::SopUnshare,
            "playbook.share" => Self::PlaybookShare,
            "playbook.unshare" => Self::PlaybookUnshare,
            "playbook.approve" => Self::PlaybookApprove,
            "user.invite" => Self::UserInvite,
            "user.deactivate" => Self::UserDeactivate,
            "membership.update" => Self::MembershipUpdate,
            "event.raw_read" => Self::EventRawRead,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditRow {
    pub id: String,
    pub tenant_id: String,
    pub organization_id: String,
    pub actor_user_id: String,
    pub action: String,
    pub resource_type: String,
    pub resource_id: String,
    pub target_user_id: Option<String>,
    pub decision: Option<String>,
    pub reason: Option<String>,
    pub metadata: String,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct AuditRecord<'a> {
    pub action: AuditAction,
    pub resource_type: &'a str,
    pub resource_id: &'a str,
    pub target_user_id: Option<&'a str>,
    pub decision: Option<&'a str>,
    pub reason: Option<&'a str>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub struct AuditQuery {
    pub actor_user_id: Option<String>,
    pub action: Option<AuditAction>,
    pub since_micros: Option<i64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Error)]
pub enum AuditWriteError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("event store: {0}")]
    Event(#[from] crate::events::EventError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Error)]
pub enum AuditQueryError {
    #[error("auth: {0}")]
    Auth(#[from] AuthError),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("invalid action in db: {0}")]
    InvalidAction(String),
}

/// Append-only writer for `audit_log`. Every mutating Phase 5 story funnels
/// its audit emission through here so the column list + dual-write Misc
/// event stay consistent.
#[derive(Clone)]
pub struct AuditLogger {
    db: DbPool,
    events: std::sync::Arc<SqliteEventStore>,
}

impl AuditLogger {
    pub fn new(db: DbPool, events: std::sync::Arc<SqliteEventStore>) -> Self {
        Self { db, events }
    }

    /// Record one audit event. Writes the `audit_log` row first, then
    /// emits a summarized `Misc{kind:"audit_logged"}` event keyed off the
    /// canonical operational session for the actor (synthesized as
    /// `audit:<tenant_id>` so cross-actor audit timelines aggregate per
    /// tenant). Returns the inserted row id.
    pub async fn record(
        &self,
        auth: &AuthContext,
        rec: AuditRecord<'_>,
    ) -> Result<String, AuditWriteError> {
        let now = now_micros();
        let id = format!("audit-{}", Uuid::new_v4());
        let metadata_str = serde_json::to_string(&rec.metadata)?;
        let id_for_move = id.clone();
        let tenant = auth.tenant_id.clone();
        let org = auth.organization_id.clone();
        let actor = auth.actor_user_id.clone();
        let action_str = rec.action.to_db_str().to_string();
        let resource_type = rec.resource_type.to_string();
        let resource_id = rec.resource_id.to_string();
        let target_user_id = rec.target_user_id.map(str::to_string);
        let decision = rec.decision.map(str::to_string);
        let reason = rec.reason.map(str::to_string);
        let metadata_for_move = metadata_str.clone();
        self.db
            .with_conn(move |conn| {
                conn.execute(
                    "INSERT INTO audit_log (
                       id, tenant_id, organization_id, actor_user_id, action,
                       resource_type, resource_id, target_user_id, decision, reason,
                       metadata, created_at
                     ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        id_for_move,
                        tenant,
                        org,
                        actor,
                        action_str,
                        resource_type,
                        resource_id,
                        target_user_id,
                        decision,
                        reason,
                        metadata_for_move,
                        now,
                    ],
                )?;
                Ok::<(), rusqlite::Error>(())
            })
            .await?;

        // Dual-write Misc event (OQ #8 Option C). The session_id is a
        // synthetic per-tenant audit feed so existing timeline consumers
        // can subscribe to `audit:<tenant>` for a structured-reporting
        // mirror of the audit_log table.
        let session_id = format!("audit:{}", auth.tenant_id);
        self.ensure_session(&session_id).await?;
        self.events
            .append(NewEvent {
                session_id,
                event_type: EventType::Misc,
                source: "audit".to_string(),
                data: serde_json::json!({
                    "kind": "audit_logged",
                    "audit_log_id": id,
                    "tenant_id": auth.tenant_id,
                    "organization_id": auth.organization_id,
                    "actor_user_id": auth.actor_user_id,
                    "action": rec.action.to_db_str(),
                    "resource_type": rec.resource_type,
                    "resource_id": rec.resource_id,
                    "target_user_id": rec.target_user_id,
                    "decision": rec.decision,
                }),
            })
            .await?;
        Ok(id)
    }

    /// Read audit rows. Admin sees all org rows; user sees only their own
    /// actions; viewer is denied at the `authorize(Action::AuditRead, ...)`
    /// gate before this method body runs.
    pub async fn query(
        &self,
        auth: &AuthContext,
        q: AuditQuery,
    ) -> Result<Vec<AuditRow>, AuditQueryError> {
        authorize(
            Action::AuditRead,
            &AuditResourceProxy(auth.org_role).into(),
            auth,
        )?;
        // User role: limit results to the actor's own rows regardless of
        // the caller-supplied `actor_user_id` filter (per architecture
        // §4.3 row "View audit log (org) — user → allow (limited)").
        let effective_actor_filter = match auth.org_role {
            Role::User => Some(auth.actor_user_id.clone()),
            _ => q.actor_user_id.clone(),
        };
        let tenant = auth.tenant_id.clone();
        let action_filter = q.action.map(|a| a.to_db_str().to_string());
        let since = q.since_micros;
        let limit = q.limit.unwrap_or(200).min(1000);
        let rows = self
            .db
            .with_conn(move |conn| {
                // Build the query dynamically so optional filters slot in cleanly.
                let mut sql = String::from(
                    "SELECT id, tenant_id, organization_id, actor_user_id, action,
                            resource_type, resource_id, target_user_id, decision, reason,
                            metadata, created_at
                     FROM audit_log
                     WHERE tenant_id = ?",
                );
                let mut params_vec: Vec<rusqlite::types::Value> = vec![tenant.into()];
                if let Some(actor) = effective_actor_filter {
                    sql.push_str(" AND actor_user_id = ?");
                    params_vec.push(actor.into());
                }
                if let Some(action_s) = action_filter {
                    sql.push_str(" AND action = ?");
                    params_vec.push(action_s.into());
                }
                if let Some(since) = since {
                    sql.push_str(" AND created_at >= ?");
                    params_vec.push(since.into());
                }
                sql.push_str(" ORDER BY created_at DESC LIMIT ?");
                params_vec.push((limit as i64).into());

                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params_vec.iter()), |r| {
                        Ok(AuditRow {
                            id: r.get(0)?,
                            tenant_id: r.get(1)?,
                            organization_id: r.get(2)?,
                            actor_user_id: r.get(3)?,
                            action: r.get(4)?,
                            resource_type: r.get(5)?,
                            resource_id: r.get(6)?,
                            target_user_id: r.get(7)?,
                            decision: r.get(8)?,
                            reason: r.get(9)?,
                            metadata: r.get(10)?,
                            created_at: r.get(11)?,
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok::<Vec<AuditRow>, rusqlite::Error>(rows)
            })
            .await?;

        // Validate each row's action string maps to a known variant (the DB
        // could conceivably carry a future Phase 6 action that this build
        // doesn't know yet — we surface that as a hard error here so
        // operators see drift rather than silently returning unknowns).
        for row in &rows {
            if AuditAction::from_db_str(&row.action).is_none() {
                return Err(AuditQueryError::InvalidAction(row.action.clone()));
            }
        }

        Ok(rows)
    }

    async fn ensure_session(&self, session_id: &str) -> Result<(), AuditWriteError> {
        let session_id = session_id.to_string();
        let now = now_micros();
        self.db
            .with_conn(move |conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO sessions (id, created_at, updated_at, state)
                     VALUES (?, ?, ?, 'IDLE')",
                    params![session_id, now, now],
                )?;
                Ok::<(), rusqlite::Error>(())
            })
            .await?;
        Ok(())
    }
}

// Per-role gating uses the same `is_same_org=true` shape as the SOP /
// playbook share gates; defining the proxy keeps the call-site terse.
struct AuditResourceProxy(Role);

impl From<AuditResourceProxy> for AuthResource {
    fn from(_: AuditResourceProxy) -> Self {
        AuthResource {
            is_same_org: true,
            actor_can_share: true,
        }
    }
}
