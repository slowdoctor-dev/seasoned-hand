use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::auth::{Action, AuthContext, AuthError, AuthResource, authorize};
use crate::db::DbPool;
use crate::time::now_micros;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SopPermission {
    Viewer,
    Editor,
    Owner,
}

impl SopPermission {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SopShareRow {
    pub id: String,
    pub tenant_id: String,
    pub sop_id: String,
    pub subject_type: String,
    pub subject_id: String,
    pub subject_email: Option<String>,
    pub permission: SopPermission,
    pub granted_by_user_id: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Error)]
pub enum SopShareError {
    #[error("{0}")]
    Auth(#[from] AuthError),
    #[error("sop not found: {0}")]
    SopNotFound(String),
    #[error("user not found for email: {0}")]
    UserNotFound(String),
    #[error("invalid permission in database: {0}")]
    InvalidPermission(String),
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
}

#[derive(Clone)]
pub struct SopShareService {
    db: DbPool,
}

impl SopShareService {
    pub fn new(db: DbPool) -> Self {
        Self { db }
    }

    pub async fn ensure_default_owner(
        &self,
        tenant_id: &str,
        sop_id: &str,
        owner_user_id: &str,
    ) -> Result<(), SopShareError> {
        let tenant_id = tenant_id.to_string();
        let sop_id = sop_id.to_string();
        let owner_user_id = owner_user_id.to_string();
        self.db
            .with_conn(move |conn| {
                let now = now_micros();
                conn.execute(
                    "INSERT INTO sop_shares (
                       id, tenant_id, sop_id, subject_type, subject_id, permission, granted_by_user_id, created_at, updated_at
                     ) VALUES (?, ?, ?, 'user', ?, 'owner', ?, ?, ?)
                     ON CONFLICT(sop_id, subject_type, subject_id)
                     DO UPDATE SET permission='owner', granted_by_user_id=excluded.granted_by_user_id, updated_at=excluded.updated_at",
                    params![
                        Uuid::new_v4().to_string(),
                        tenant_id,
                        sop_id,
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
        sop_id: &str,
        user_email: &str,
        permission: SopPermission,
    ) -> Result<SopShareRow, SopShareError> {
        self.authorize_share(auth, sop_id).await?;
        let tenant = auth.tenant_id.clone();
        let actor = auth.actor_user_id.clone();
        let sop_id = sop_id.to_string();
        let user_email = user_email.to_string();
        let perm = permission.as_db_str().to_string();
        self.db
            .with_conn(move |conn| {
                let sop_exists = conn
                    .query_row("SELECT 1 FROM sops WHERE id = ?", params![sop_id], |row| {
                        row.get::<_, i64>(0)
                    })
                    .optional()?
                    .is_some();
                if !sop_exists {
                    return Err(SopShareError::SopNotFound(sop_id.clone()));
                }
                let subject_id: String = conn
                    .query_row(
                        "SELECT id FROM users WHERE tenant_id = ? AND email = ?",
                        params![tenant, user_email],
                        |row| row.get(0),
                    )
                    .optional()?
                    .ok_or_else(|| SopShareError::UserNotFound(user_email.clone()))?;
                let now = now_micros();
                let id = Uuid::new_v4().to_string();
                conn.execute(
                    "INSERT INTO sop_shares (
                       id, tenant_id, sop_id, subject_type, subject_id, permission, granted_by_user_id, created_at, updated_at
                     ) VALUES (?, ?, ?, 'user', ?, ?, ?, ?, ?)
                     ON CONFLICT(sop_id, subject_type, subject_id)
                     DO UPDATE SET permission=excluded.permission, granted_by_user_id=excluded.granted_by_user_id, updated_at=excluded.updated_at",
                    params![id, tenant, sop_id, subject_id, perm, actor, now, now],
                )?;
                let row = conn.query_row(
                    "SELECT ss.id, ss.tenant_id, ss.sop_id, ss.subject_type, ss.subject_id, u.email, ss.permission, ss.granted_by_user_id, ss.created_at, ss.updated_at
                     FROM sop_shares ss
                     LEFT JOIN users u ON u.id = ss.subject_id
                     WHERE ss.sop_id = ? AND ss.subject_type = 'user' AND ss.subject_id = ?",
                    params![sop_id, subject_id],
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
                        ))
                    },
                )?;
                let permission = SopPermission::from_db(&row.6)
                    .ok_or_else(|| SopShareError::InvalidPermission(row.6.clone()))?;
                Ok::<SopShareRow, SopShareError>(SopShareRow {
                    id: row.0,
                    tenant_id: row.1,
                    sop_id: row.2,
                    subject_type: row.3,
                    subject_id: row.4,
                    subject_email: row.5,
                    permission,
                    granted_by_user_id: row.7,
                    created_at: row.8,
                    updated_at: row.9,
                })
            })
            .await
    }

    pub async fn unshare(
        &self,
        auth: &AuthContext,
        sop_id: &str,
        user_email: &str,
    ) -> Result<bool, SopShareError> {
        self.authorize_share(auth, sop_id).await?;
        let tenant = auth.tenant_id.clone();
        let sop_id = sop_id.to_string();
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
                    "DELETE FROM sop_shares
                     WHERE sop_id = ? AND subject_type = 'user' AND subject_id = ?",
                    params![sop_id, subject_id],
                )?;
                Ok::<bool, SopShareError>(n > 0)
            })
            .await
    }

    pub async fn list_for_sop(
        &self,
        auth: &AuthContext,
        sop_id: &str,
    ) -> Result<Vec<SopShareRow>, SopShareError> {
        self.authorize_share(auth, sop_id).await?;
        let sop_id = sop_id.to_string();
        self.db
            .with_conn(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT ss.id, ss.tenant_id, ss.sop_id, ss.subject_type, ss.subject_id, u.email, ss.permission, ss.granted_by_user_id, ss.created_at, ss.updated_at
                     FROM sop_shares ss
                     LEFT JOIN users u ON u.id = ss.subject_id
                     WHERE ss.sop_id = ?
                     ORDER BY ss.subject_type ASC, ss.subject_id ASC",
                )?;
                let mapped = stmt.query_map(params![sop_id], |r| {
                    let permission_raw: String = r.get(6)?;
                    let permission = SopPermission::from_db(&permission_raw)
                        .ok_or_else(|| rusqlite::Error::InvalidColumnType(6, "permission".into(), rusqlite::types::Type::Text))?;
                    Ok(SopShareRow {
                        id: r.get(0)?,
                        tenant_id: r.get(1)?,
                        sop_id: r.get(2)?,
                        subject_type: r.get(3)?,
                        subject_id: r.get(4)?,
                        subject_email: r.get(5)?,
                        permission,
                        granted_by_user_id: r.get(7)?,
                        created_at: r.get(8)?,
                        updated_at: r.get(9)?,
                    })
                })?;
                let mut out = Vec::new();
                for row in mapped {
                    out.push(row?);
                }
                Ok::<Vec<SopShareRow>, rusqlite::Error>(out)
            })
            .await
            .map_err(SopShareError::from)
    }

    pub async fn list_for_user(
        &self,
        auth: &AuthContext,
        user_id: &str,
    ) -> Result<Vec<SopShareRow>, SopShareError> {
        authorize(
            Action::SopShare,
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
                    "SELECT ss.id, ss.tenant_id, ss.sop_id, ss.subject_type, ss.subject_id, u.email, ss.permission, ss.granted_by_user_id, ss.created_at, ss.updated_at
                     FROM sop_shares ss
                     LEFT JOIN users u ON u.id = ss.subject_id
                     WHERE ss.tenant_id = ? AND ss.subject_type = 'user' AND ss.subject_id = ?
                     ORDER BY ss.updated_at DESC, ss.id DESC",
                )?;
                let mapped = stmt.query_map(params![tenant, user_id], |r| {
                    let permission_raw: String = r.get(6)?;
                    let permission = SopPermission::from_db(&permission_raw)
                        .ok_or_else(|| rusqlite::Error::InvalidColumnType(6, "permission".into(), rusqlite::types::Type::Text))?;
                    Ok(SopShareRow {
                        id: r.get(0)?,
                        tenant_id: r.get(1)?,
                        sop_id: r.get(2)?,
                        subject_type: r.get(3)?,
                        subject_id: r.get(4)?,
                        subject_email: r.get(5)?,
                        permission,
                        granted_by_user_id: r.get(7)?,
                        created_at: r.get(8)?,
                        updated_at: r.get(9)?,
                    })
                })?;
                let mut out = Vec::new();
                for row in mapped {
                    out.push(row?);
                }
                Ok::<Vec<SopShareRow>, rusqlite::Error>(out)
            })
            .await
            .map_err(SopShareError::from)
    }

    async fn authorize_share(&self, auth: &AuthContext, sop_id: &str) -> Result<(), SopShareError> {
        let tenant = auth.tenant_id.clone();
        let org_id = auth.organization_id.clone();
        let actor_user_id = auth.actor_user_id.clone();
        let sop_id = sop_id.to_string();
        let can_share = self
            .db
            .with_conn(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT permission
                     FROM sop_shares
                     WHERE tenant_id = ? AND sop_id = ?
                       AND (
                         (subject_type = 'user' AND subject_id = ?)
                         OR (subject_type = 'org' AND subject_id = ?)
                       )",
                )?;
                let rows = stmt.query_map(params![tenant, sop_id, actor_user_id, org_id], |r| {
                    r.get::<_, String>(0)
                })?;
                let mut can_share = false;
                for row in rows {
                    let perm_raw = row?;
                    if let Some(perm) = SopPermission::from_db(&perm_raw)
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
            Action::SopShare,
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
                "INSERT INTO sops (id, title, content, version, enforced, created_at, updated_at)
                 VALUES ('sop-1', 'Deploy', 'Checklist', 1, 1, 1, 1)",
                [],
            )?;
            conn.execute(
                "INSERT INTO sop_shares (id, tenant_id, sop_id, subject_type, subject_id, permission, granted_by_user_id, created_at, updated_at)
                 VALUES ('seed-owner', 'tenant-a', 'sop-1', 'user', 'u-owner', 'owner', 'u-owner', 1, 1)",
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
        let service = SopShareService::new(pool);
        let err = service
            .share(
                &ctx(Role::Viewer, "u-viewer"),
                "sop-1",
                "viewer@acme.dev",
                SopPermission::Owner,
            )
            .await
            .expect_err("viewer should not escalate");
        assert!(matches!(
            err,
            SopShareError::Auth(AuthError::Unauthorized { .. })
        ));
    }

    #[tokio::test]
    async fn admin_can_override_any_grant() {
        let pool = open_pool().await;
        seed(&pool).await;
        let service = SopShareService::new(pool.clone());
        service
            .share(
                &ctx(Role::Admin, "u-admin"),
                "sop-1",
                "viewer@acme.dev",
                SopPermission::Editor,
            )
            .await
            .expect("admin share");
        let rows = service
            .list_for_sop(&ctx(Role::Admin, "u-admin"), "sop-1")
            .await
            .expect("list");
        let row = rows
            .into_iter()
            .find(|r| r.subject_email.as_deref() == Some("viewer@acme.dev"))
            .expect("share row");
        assert_eq!(row.permission, SopPermission::Editor);
    }

    #[tokio::test]
    async fn default_owner_policy_inserts_owner_share_row() {
        let pool = open_pool().await;
        seed(&pool).await;
        pool.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sops (id, title, content, version, enforced, created_at, updated_at)
                 VALUES ('sop-2', 'Incident', 'Steps', 1, 1, 1, 1)",
                [],
            )?;
            Ok::<(), rusqlite::Error>(())
        })
        .await
        .expect("seed sop-2");
        let service = SopShareService::new(pool.clone());
        service
            .ensure_default_owner("tenant-a", "sop-2", "u-owner")
            .await
            .expect("owner row");
        let rows = service
            .list_for_sop(&ctx(Role::Admin, "u-admin"), "sop-2")
            .await
            .expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].subject_id, "u-owner");
        assert_eq!(rows[0].permission, SopPermission::Owner);
    }

    #[tokio::test]
    async fn share_visibility_propagates_within_five_seconds_budget() {
        let pool = open_pool().await;
        seed(&pool).await;
        let service = SopShareService::new(pool.clone());
        let start = std::time::Instant::now();
        service
            .share(
                &ctx(Role::Admin, "u-admin"),
                "sop-1",
                "viewer@acme.dev",
                SopPermission::Viewer,
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
        assert_eq!(rows[0].sop_id, "sop-1");
    }
}
