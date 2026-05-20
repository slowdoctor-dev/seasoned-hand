use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::auth::{Action, AuthContext, AuthError, AuthResource, authorize};
use crate::db::DbPool;
use crate::time::now_micros;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaybookPermission {
    Viewer,
    Editor,
    Owner,
}

impl PlaybookPermission {
    fn from_db(value: &str) -> Option<Self> {
        match value {
            "viewer" => Some(Self::Viewer),
            "editor" => Some(Self::Editor),
            "owner" => Some(Self::Owner),
            _ => None,
        }
    }

    fn as_db_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Editor => "editor",
            Self::Owner => "owner",
        }
    }

    fn can_share(self) -> bool {
        matches!(self, Self::Editor | Self::Owner)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VisibilityState {
    Review,
    Shared,
    Suspended,
}

impl VisibilityState {
    pub(crate) fn from_db(value: &str) -> Option<Self> {
        match value {
            "review" => Some(Self::Review),
            "shared" => Some(Self::Shared),
            "suspended" => Some(Self::Suspended),
            _ => None,
        }
    }

    pub(crate) fn as_db_str(self) -> &'static str {
        match self {
            Self::Review => "review",
            Self::Shared => "shared",
            Self::Suspended => "suspended",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaybookShareRow {
    pub id: String,
    pub tenant_id: String,
    pub playbook_id: String,
    pub subject_type: String,
    pub subject_id: String,
    pub subject_email: Option<String>,
    pub permission: PlaybookPermission,
    pub visibility_state: VisibilityState,
    pub granted_by_user_id: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Error)]
pub enum PlaybookShareError {
    #[error("{0}")]
    Auth(#[from] AuthError),
    #[error("playbook not found: {0}")]
    PlaybookNotFound(String),
    #[error("user not found for email: {0}")]
    UserNotFound(String),
    #[error("invalid permission in database: {0}")]
    InvalidPermission(String),
    #[error("invalid visibility_state in database: {0}")]
    InvalidVisibilityState(String),
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
}

#[derive(Clone)]
pub struct PlaybookShareService {
    db: DbPool,
}

impl PlaybookShareService {
    pub fn new(db: DbPool) -> Self {
        Self { db }
    }

    pub async fn ensure_default_owner(
        &self,
        tenant_id: &str,
        playbook_id: &str,
        owner_user_id: &str,
    ) -> Result<(), PlaybookShareError> {
        let tenant_id = tenant_id.to_string();
        let playbook_id = playbook_id.to_string();
        let owner_user_id = owner_user_id.to_string();
        self.db
            .with_conn(move |conn| {
                let now = now_micros();
                conn.execute(
                    "INSERT INTO playbook_shares (
                       id, tenant_id, playbook_id, subject_type, subject_id, permission, visibility_state, granted_by_user_id, created_at, updated_at
                     ) VALUES (?, ?, ?, 'user', ?, 'owner', 'shared', ?, ?, ?)
                     ON CONFLICT(playbook_id, subject_type, subject_id)
                     DO UPDATE SET permission='owner', granted_by_user_id=excluded.granted_by_user_id, updated_at=excluded.updated_at",
                    params![
                        Uuid::new_v4().to_string(),
                        tenant_id,
                        playbook_id,
                        owner_user_id,
                        owner_user_id,
                        now,
                        now
                    ],
                )?;
                Ok::<(), rusqlite::Error>(())
            })
            .await?;
        Ok(())
    }

    pub async fn share(
        &self,
        auth: &AuthContext,
        playbook_id: &str,
        user_email: &str,
        permission: PlaybookPermission,
    ) -> Result<PlaybookShareRow, PlaybookShareError> {
        self.authorize_share(auth, playbook_id).await?;
        let tenant = auth.tenant_id.clone();
        let actor = auth.actor_user_id.clone();
        let playbook_id = playbook_id.to_string();
        let user_email = user_email.to_string();
        let perm = permission.as_db_str().to_string();
        self.db
            .with_conn(move |conn| {
                let sop_exists = conn
                    .query_row("SELECT 1 FROM playbooks WHERE id = ?", params![playbook_id], |row| {
                        row.get::<_, i64>(0)
                    })
                    .optional()?
                    .is_some();
                if !sop_exists {
                    return Err(PlaybookShareError::PlaybookNotFound(playbook_id.clone()));
                }
                let subject_id: String = conn
                    .query_row(
                        "SELECT id FROM users WHERE tenant_id = ? AND email = ?",
                        params![tenant, user_email],
                        |row| row.get(0),
                    )
                    .optional()?
                    .ok_or_else(|| PlaybookShareError::UserNotFound(user_email.clone()))?;
                let now = now_micros();
                let id = Uuid::new_v4().to_string();
                conn.execute(
                    "INSERT INTO playbook_shares (
                       id, tenant_id, playbook_id, subject_type, subject_id, permission, visibility_state, granted_by_user_id, created_at, updated_at
                     ) VALUES (?, ?, ?, 'user', ?, ?, 'review', ?, ?, ?)
                     ON CONFLICT(playbook_id, subject_type, subject_id)
                     DO UPDATE SET permission=excluded.permission, granted_by_user_id=excluded.granted_by_user_id, updated_at=excluded.updated_at",
                    params![id, tenant, playbook_id, subject_id, perm, actor, now, now],
                )?;
                let row = conn.query_row(
                    "SELECT ss.id, ss.tenant_id, ss.playbook_id, ss.subject_type, ss.subject_id, u.email, ss.permission, ss.visibility_state, ss.granted_by_user_id, ss.created_at, ss.updated_at
                     FROM playbook_shares ss
                     LEFT JOIN users u ON u.id = ss.subject_id
                     WHERE ss.playbook_id = ? AND ss.subject_type = 'user' AND ss.subject_id = ?",
                    params![playbook_id, subject_id],
                    |r| {
                        Ok::<
                            (
                                String,
                                String,
                                String,
                                String,
                                String,
                                Option<String>,
                                String,
                                String,
                                String,
                                i64,
                                i64,
                            ),
                            rusqlite::Error,
                        >((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                            r.get(6)?,
                            r.get(7)?,
                            r.get(8)?,
                            r.get(9)?,
                            r.get(10)?,
                        ))
                    },
                )?;
                let permission = PlaybookPermission::from_db(&row.6)
                    .ok_or_else(|| PlaybookShareError::InvalidPermission(row.6.clone()))?;
                let visibility_state = VisibilityState::from_db(&row.7)
                    .ok_or_else(|| PlaybookShareError::InvalidVisibilityState(row.7.clone()))?;
                Ok::<PlaybookShareRow, PlaybookShareError>(PlaybookShareRow {
                    id: row.0,
                    tenant_id: row.1,
                    playbook_id: row.2,
                    subject_type: row.3,
                    subject_id: row.4,
                    subject_email: row.5,
                    permission,
                    visibility_state,
                    granted_by_user_id: row.8,
                    created_at: row.9,
                    updated_at: row.10,
                })
            })
            .await
    }

    pub async fn unshare(
        &self,
        auth: &AuthContext,
        playbook_id: &str,
        user_email: &str,
    ) -> Result<bool, PlaybookShareError> {
        self.authorize_share(auth, playbook_id).await?;
        let tenant = auth.tenant_id.clone();
        let playbook_id = playbook_id.to_string();
        let user_email = user_email.to_string();
        self.db
            .with_conn(move |conn| {
                let subject_id: Option<String> = conn
                    .query_row(
                        "SELECT id FROM users WHERE tenant_id = ? AND email = ?",
                        params![tenant, user_email],
                        |row| row.get(0),
                    )
                    .optional()?;
                let Some(subject_id) = subject_id else {
                    return Ok(false);
                };
                let n = conn.execute(
                    "DELETE FROM playbook_shares
                     WHERE playbook_id = ? AND subject_type = 'user' AND subject_id = ?",
                    params![playbook_id, subject_id],
                )?;
                Ok::<bool, PlaybookShareError>(n > 0)
            })
            .await
    }

    pub async fn list_for_playbook(
        &self,
        auth: &AuthContext,
        playbook_id: &str,
    ) -> Result<Vec<PlaybookShareRow>, PlaybookShareError> {
        self.authorize_share(auth, playbook_id).await?;
        let playbook_id = playbook_id.to_string();
        self.db
            .with_conn(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT ss.id, ss.tenant_id, ss.playbook_id, ss.subject_type, ss.subject_id, u.email, ss.permission, ss.visibility_state, ss.granted_by_user_id, ss.created_at, ss.updated_at
                     FROM playbook_shares ss
                     LEFT JOIN users u ON u.id = ss.subject_id
                     WHERE ss.playbook_id = ?
                     ORDER BY ss.subject_type ASC, ss.subject_id ASC",
                )?;
                let mapped = stmt.query_map(params![playbook_id], |r| {
                    let permission_raw: String = r.get(6)?;
                    let permission = PlaybookPermission::from_db(&permission_raw)
                        .ok_or_else(|| rusqlite::Error::InvalidColumnType(6, "permission".into(), rusqlite::types::Type::Text))?;
                    let visibility_raw: String = r.get(7)?;
                    let visibility_state = VisibilityState::from_db(&visibility_raw)
                        .ok_or_else(|| rusqlite::Error::InvalidColumnType(7, "visibility_state".into(), rusqlite::types::Type::Text))?;
                    Ok(PlaybookShareRow {
                        id: r.get(0)?,
                        tenant_id: r.get(1)?,
                        playbook_id: r.get(2)?,
                        subject_type: r.get(3)?,
                        subject_id: r.get(4)?,
                        subject_email: r.get(5)?,
                        permission,
                        visibility_state,
                        granted_by_user_id: r.get(8)?,
                        created_at: r.get(9)?,
                        updated_at: r.get(10)?,
                    })
                })?;
                let mut out = Vec::new();
                for row in mapped {
                    out.push(row?);
                }
                Ok::<Vec<PlaybookShareRow>, rusqlite::Error>(out)
            })
            .await
            .map_err(PlaybookShareError::from)
    }

    pub async fn list_for_user(
        &self,
        auth: &AuthContext,
        user_id: &str,
    ) -> Result<Vec<PlaybookShareRow>, PlaybookShareError> {
        authorize(
            Action::PlaybookShare,
            &AuthResource {
                is_same_org: true,
                actor_can_share: true,
            },
            auth,
        )?;
        let tenant = auth.tenant_id.clone();
        let user_id = user_id.to_string();
        self.db
            .with_conn(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT ss.id, ss.tenant_id, ss.playbook_id, ss.subject_type, ss.subject_id, u.email, ss.permission, ss.visibility_state, ss.granted_by_user_id, ss.created_at, ss.updated_at
                     FROM playbook_shares ss
                     LEFT JOIN users u ON u.id = ss.subject_id
                     WHERE ss.tenant_id = ? AND ss.subject_type = 'user' AND ss.subject_id = ?
                     ORDER BY ss.updated_at DESC, ss.id DESC",
                )?;
                let mapped = stmt.query_map(params![tenant, user_id], |r| {
                    let permission_raw: String = r.get(6)?;
                    let permission = PlaybookPermission::from_db(&permission_raw)
                        .ok_or_else(|| rusqlite::Error::InvalidColumnType(6, "permission".into(), rusqlite::types::Type::Text))?;
                    let visibility_raw: String = r.get(7)?;
                    let visibility_state = VisibilityState::from_db(&visibility_raw)
                        .ok_or_else(|| rusqlite::Error::InvalidColumnType(7, "visibility_state".into(), rusqlite::types::Type::Text))?;
                    Ok(PlaybookShareRow {
                        id: r.get(0)?,
                        tenant_id: r.get(1)?,
                        playbook_id: r.get(2)?,
                        subject_type: r.get(3)?,
                        subject_id: r.get(4)?,
                        subject_email: r.get(5)?,
                        permission,
                        visibility_state,
                        granted_by_user_id: r.get(8)?,
                        created_at: r.get(9)?,
                        updated_at: r.get(10)?,
                    })
                })?;
                let mut out = Vec::new();
                for row in mapped {
                    out.push(row?);
                }
                Ok::<Vec<PlaybookShareRow>, rusqlite::Error>(out)
            })
            .await
            .map_err(PlaybookShareError::from)
    }

    /// Transition a single share row's `visibility_state`. Used by curator
    /// auto-share (review → shared on high-confidence) and admin approval
    /// (review → shared on operator action). Caller must have admin role
    /// per the §4.3 matrix — `PlaybookShare` action is the gate.
    pub async fn update_visibility_state(
        &self,
        auth: &AuthContext,
        playbook_id: &str,
        user_email: &str,
        new_state: VisibilityState,
    ) -> Result<bool, PlaybookShareError> {
        self.authorize_share(auth, playbook_id).await?;
        let tenant = auth.tenant_id.clone();
        let playbook_id = playbook_id.to_string();
        let user_email = user_email.to_string();
        let new_state_str = new_state.as_db_str();
        let now = now_micros();
        self.db
            .with_conn(move |conn| {
                let subject_id: Option<String> = conn
                    .query_row(
                        "SELECT id FROM users WHERE tenant_id = ? AND email = ?",
                        params![tenant, user_email],
                        |row| row.get(0),
                    )
                    .optional()?;
                let Some(subject_id) = subject_id else {
                    return Ok(false);
                };
                let n = conn.execute(
                    "UPDATE playbook_shares
                     SET visibility_state = ?, updated_at = ?
                     WHERE playbook_id = ? AND subject_type = 'user' AND subject_id = ?",
                    params![new_state_str, now, playbook_id, subject_id],
                )?;
                Ok::<bool, PlaybookShareError>(n > 0)
            })
            .await
    }

    /// Curator-facing hook: when ConsolidationEngine writes a new playbook
    /// (or revision) with high enough confidence, this writes a `playbook_shares`
    /// row directly in `visibility_state='shared'` so the matcher picks it up
    /// immediately. Low-confidence revisions stay in `review` (the default for
    /// any operator-initiated share).
    ///
    /// Bypasses the `authorize_share` check because the caller is a system
    /// worker holding a `SystemAuth::for_worker` context; the worker context
    /// is already admin-role for its tenant. The org membership lookup that
    /// `authorize_share` would do isn't meaningful for a worker actor.
    ///
    /// `granted_by_user_id` uses the V013-bootstrapped `user-legacy-admin`
    /// sentinel — the audit attribution is meant to point at "the operator
    /// who configured the system to auto-share this", and the legacy admin
    /// is the canonical operator identity for system-driven writes. The
    /// `actor_user_id` from `worker_auth` (e.g. `system-worker-curator`)
    /// isn't a `users.id` row so it can't satisfy the FK.
    pub async fn curator_auto_share(
        &self,
        worker_auth: &AuthContext,
        playbook_id: &str,
        confidence: f32,
        archive_apply_min_confidence: f32,
    ) -> Result<VisibilityState, PlaybookShareError> {
        let state = if confidence >= archive_apply_min_confidence {
            VisibilityState::Shared
        } else {
            VisibilityState::Review
        };
        let state_str = state.as_db_str();
        let tenant = worker_auth.tenant_id.clone();
        let org_id = worker_auth.organization_id.clone();
        let playbook_id = playbook_id.to_string();
        let now = now_micros();
        self.db
            .with_conn(move |conn| {
                conn.execute(
                    "INSERT INTO playbook_shares (
                       id, tenant_id, playbook_id, subject_type, subject_id, permission, visibility_state, granted_by_user_id, created_at, updated_at
                     ) VALUES (?, ?, ?, 'org', ?, 'viewer', ?, 'user-legacy-admin', ?, ?)
                     ON CONFLICT(playbook_id, subject_type, subject_id)
                     DO UPDATE SET visibility_state = excluded.visibility_state, updated_at = excluded.updated_at",
                    params![
                        Uuid::new_v4().to_string(),
                        tenant,
                        playbook_id,
                        org_id,
                        state_str,
                        now,
                        now
                    ],
                )?;
                Ok::<(), rusqlite::Error>(())
            })
            .await?;
        Ok(state)
    }

    async fn authorize_share(
        &self,
        auth: &AuthContext,
        playbook_id: &str,
    ) -> Result<(), PlaybookShareError> {
        let tenant = auth.tenant_id.clone();
        let org_id = auth.organization_id.clone();
        let actor_user_id = auth.actor_user_id.clone();
        let playbook_id = playbook_id.to_string();
        let can_share = self
            .db
            .with_conn(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT permission
                     FROM playbook_shares
                     WHERE tenant_id = ? AND playbook_id = ?
                       AND (
                         (subject_type = 'user' AND subject_id = ?)
                         OR (subject_type = 'org' AND subject_id = ?)
                       )",
                )?;
                let rows = stmt
                    .query_map(params![tenant, playbook_id, actor_user_id, org_id], |r| {
                        r.get::<_, String>(0)
                    })?;
                let mut can_share = false;
                for row in rows {
                    let perm_raw = row?;
                    if let Some(perm) = PlaybookPermission::from_db(&perm_raw)
                        && perm.can_share()
                    {
                        can_share = true;
                        break;
                    }
                }
                Ok::<bool, rusqlite::Error>(can_share)
            })
            .await?;
        authorize(
            Action::PlaybookShare,
            &AuthResource {
                is_same_org: true,
                actor_can_share: can_share,
            },
            auth,
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{AuthContext, Role};
    use crate::db;

    async fn open_pool() -> DbPool {
        db::open(":memory:").await.expect("db")
    }

    fn ctx(role: Role, user_id: &str) -> AuthContext {
        AuthContext {
            tenant_id: "tenant-a".into(),
            organization_id: "org-a".into(),
            actor_user_id: user_id.into(),
            org_role: role,
            project_override_role: None,
        }
    }

    async fn seed(pool: &DbPool) {
        pool.with_conn(|conn| {
            conn.execute(
                "INSERT INTO organizations (id, tenant_id, slug, display_name, status, created_at, updated_at)
                 VALUES ('org-a', 'tenant-a', 'org-a', 'Org A', 'active', 1, 1)",
                [],
            )?;
            conn.execute(
                "INSERT INTO users (id, tenant_id, email, display_name, status, created_at, updated_at)
                 VALUES ('u-admin', 'tenant-a', 'admin@acme.dev', 'Admin', 'active', 1, 1)",
                [],
            )?;
            conn.execute(
                "INSERT INTO users (id, tenant_id, email, display_name, status, created_at, updated_at)
                 VALUES ('u-owner', 'tenant-a', 'owner@acme.dev', 'Owner', 'active', 1, 1)",
                [],
            )?;
            conn.execute(
                "INSERT INTO users (id, tenant_id, email, display_name, status, created_at, updated_at)
                 VALUES ('u-viewer', 'tenant-a', 'viewer@acme.dev', 'Viewer', 'active', 1, 1)",
                [],
            )?;
            conn.execute(
                "INSERT INTO playbooks (id, tenant_id, title, content_path, schema_version, source_task_id, created_at, updated_at)
                 VALUES ('pb-1', 'tenant-a', 'Deploy', 'pb/deploy.md', 1, NULL, 1, 1)",
                [],
            )?;
            conn.execute(
                "INSERT INTO playbook_shares (id, tenant_id, playbook_id, subject_type, subject_id, permission, visibility_state, granted_by_user_id, created_at, updated_at)
                 VALUES ('seed-owner', 'tenant-a', 'pb-1', 'user', 'u-owner', 'owner', 'shared', 'u-owner', 1, 1)",
                [],
            )?;
            Ok::<(), rusqlite::Error>(())
        })
        .await
        .expect("seed");
    }

    #[tokio::test]
    async fn viewer_cannot_escalate_own_permission() {
        let pool = open_pool().await;
        seed(&pool).await;
        let service = PlaybookShareService::new(pool);
        let err = service
            .share(
                &ctx(Role::Viewer, "u-viewer"),
                "pb-1",
                "viewer@acme.dev",
                PlaybookPermission::Owner,
            )
            .await
            .expect_err("viewer should not escalate");
        assert!(matches!(
            err,
            PlaybookShareError::Auth(AuthError::Unauthorized { .. })
        ));
    }

    #[tokio::test]
    async fn admin_can_override_any_grant() {
        let pool = open_pool().await;
        seed(&pool).await;
        let service = PlaybookShareService::new(pool.clone());
        service
            .share(
                &ctx(Role::Admin, "u-admin"),
                "pb-1",
                "viewer@acme.dev",
                PlaybookPermission::Editor,
            )
            .await
            .expect("admin share");
        let rows = service
            .list_for_playbook(&ctx(Role::Admin, "u-admin"), "pb-1")
            .await
            .expect("list");
        let row = rows
            .into_iter()
            .find(|r| r.subject_email.as_deref() == Some("viewer@acme.dev"))
            .expect("share row");
        assert_eq!(row.permission, PlaybookPermission::Editor);
    }

    #[tokio::test]
    async fn default_owner_policy_inserts_owner_share_row() {
        let pool = open_pool().await;
        seed(&pool).await;
        pool.with_conn(|conn| {
            conn.execute(
                "INSERT INTO playbooks (id, tenant_id, title, content_path, schema_version, source_task_id, created_at, updated_at)
                 VALUES ('pb-2', 'tenant-a', 'Incident', 'pb/inc.md', 1, NULL, 1, 1)",
                [],
            )?;
            Ok::<(), rusqlite::Error>(())
        })
        .await
        .expect("seed pb-2");
        let service = PlaybookShareService::new(pool.clone());
        service
            .ensure_default_owner("tenant-a", "pb-2", "u-owner")
            .await
            .expect("owner row");
        let rows = service
            .list_for_playbook(&ctx(Role::Admin, "u-admin"), "pb-2")
            .await
            .expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].subject_id, "u-owner");
        assert_eq!(rows[0].permission, PlaybookPermission::Owner);
        assert_eq!(rows[0].visibility_state, VisibilityState::Shared);
    }

    #[tokio::test]
    async fn share_visibility_propagates_within_five_seconds_budget() {
        let pool = open_pool().await;
        seed(&pool).await;
        let service = PlaybookShareService::new(pool.clone());
        let start = std::time::Instant::now();
        service
            .share(
                &ctx(Role::Admin, "u-admin"),
                "pb-1",
                "viewer@acme.dev",
                PlaybookPermission::Viewer,
            )
            .await
            .expect("share");
        let rows = service
            .list_for_user(&ctx(Role::Admin, "u-admin"), "u-viewer")
            .await
            .expect("list for user");
        assert!(
            start.elapsed() <= std::time::Duration::from_secs(5),
            "share visibility exceeded 5s p95 budget",
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].playbook_id, "pb-1");
    }

    #[tokio::test]
    async fn curator_auto_share_picks_state_by_confidence() {
        // F-5.7 + architecture §6.2: high-confidence → 'shared' immediately;
        // low-confidence → 'review' until admin approves.
        let pool = open_pool().await;
        seed(&pool).await;
        // V013 sentinel users.id 'user-legacy-admin' is the granted_by target
        // for curator-driven shares (see curator_auto_share doc comment).
        let service = PlaybookShareService::new(pool.clone());
        let worker = crate::auth::SystemAuth::for_worker("org-a", "tenant-a", "curator");

        // Confidence above threshold → shared.
        let state = service
            .curator_auto_share(&worker, "pb-1", 0.92, 0.55)
            .await
            .expect("auto-share high-confidence");
        assert_eq!(state, VisibilityState::Shared);

        // Re-run with low confidence on the same row should DOWNGRADE to review
        // (UPSERT semantics — last writer wins on visibility_state).
        let state = service
            .curator_auto_share(&worker, "pb-1", 0.30, 0.55)
            .await
            .expect("auto-share low-confidence");
        assert_eq!(state, VisibilityState::Review);
    }
}
