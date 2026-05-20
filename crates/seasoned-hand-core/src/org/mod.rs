//! Org / user / membership / project-role-override persistence (story 5.4).
//!
//! V013 (story 5.2) created the 4 tables; this module wraps them in
//! rusqlite-backed stores. Every read and write requires a `tenant_id` —
//! callers route through [`crate::auth::AuthContext`] in HTTP middleware
//! (story 5.5) and CLI/worker boundaries (story 5.6).
//!
//! refs: /specs/phase-5/architecture.md §3.2, §4.1
//! refs: /specs/phase-5/stories/story-5.4.md

use rusqlite::params;
use thiserror::Error;
use uuid::Uuid;

use crate::auth::Role;
use crate::db::DbPool;
use crate::time::now_micros;

#[derive(Debug, Error)]
pub enum OrgStoreError {
    #[error("db error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid role string: {0}")]
    InvalidRole(String),
    #[error("invalid status: {0}")]
    InvalidStatus(String),
}

fn role_to_db(role: Role) -> &'static str {
    match role {
        Role::Admin => "admin",
        Role::User => "user",
        Role::Viewer => "viewer",
    }
}

fn role_from_db(s: &str) -> Result<Role, OrgStoreError> {
    match s {
        "admin" => Ok(Role::Admin),
        "user" => Ok(Role::User),
        "viewer" => Ok(Role::Viewer),
        other => Err(OrgStoreError::InvalidRole(other.to_string())),
    }
}

// ============================================================================
// Organization
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Organization {
    pub id: String,
    pub tenant_id: String,
    pub slug: String,
    pub display_name: String,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewOrganization {
    pub tenant_id: String,
    pub slug: String,
    pub display_name: String,
}

#[derive(Clone)]
pub struct OrganizationStore {
    pool: DbPool,
}

impl OrganizationStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, new: NewOrganization) -> Result<String, OrgStoreError> {
        let id = format!("org-{}", Uuid::new_v4());
        let id_for_move = id.clone();
        let now = now_micros();
        self.pool
            .with_conn(move |conn| {
                conn.execute(
                    "INSERT INTO organizations
                     (id, tenant_id, slug, display_name, status, created_at, updated_at)
                     VALUES (?, ?, ?, ?, 'active', ?, ?)",
                    params![
                        id_for_move,
                        new.tenant_id,
                        new.slug,
                        new.display_name,
                        now,
                        now,
                    ],
                )?;
                Ok::<_, OrgStoreError>(())
            })
            .await?;
        Ok(id)
    }

    pub async fn get(&self, id: &str) -> Result<Organization, OrgStoreError> {
        let id = id.to_string();
        let id_for_err = id.clone();
        self.pool
            .with_conn(move |conn| {
                conn.query_row(
                    "SELECT id, tenant_id, slug, display_name, status, created_at, updated_at
                     FROM organizations WHERE id = ?",
                    params![id],
                    |r| {
                        Ok(Organization {
                            id: r.get(0)?,
                            tenant_id: r.get(1)?,
                            slug: r.get(2)?,
                            display_name: r.get(3)?,
                            status: r.get(4)?,
                            created_at: r.get(5)?,
                            updated_at: r.get(6)?,
                        })
                    },
                )
                .map_err(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => OrgStoreError::NotFound(id_for_err),
                    other => OrgStoreError::Sqlite(other),
                })
            })
            .await
    }

    pub async fn list_by_tenant(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<Organization>, OrgStoreError> {
        let tenant_id = tenant_id.to_string();
        self.pool
            .with_conn(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, tenant_id, slug, display_name, status, created_at, updated_at
                     FROM organizations WHERE tenant_id = ? ORDER BY created_at ASC",
                )?;
                let rows = stmt.query_map(params![tenant_id], |r| {
                    Ok(Organization {
                        id: r.get(0)?,
                        tenant_id: r.get(1)?,
                        slug: r.get(2)?,
                        display_name: r.get(3)?,
                        status: r.get(4)?,
                        created_at: r.get(5)?,
                        updated_at: r.get(6)?,
                    })
                })?;
                rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
            })
            .await
    }
}

// ============================================================================
// User
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: String,
    pub tenant_id: String,
    pub email: String,
    pub display_name: String,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewUser {
    pub tenant_id: String,
    pub email: String,
    pub display_name: String,
}

#[derive(Clone)]
pub struct UserStore {
    pool: DbPool,
}

impl UserStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, new: NewUser) -> Result<String, OrgStoreError> {
        let id = format!("user-{}", Uuid::new_v4());
        let id_for_move = id.clone();
        let now = now_micros();
        self.pool
            .with_conn(move |conn| {
                conn.execute(
                    "INSERT INTO users
                     (id, tenant_id, email, display_name, status, created_at, updated_at)
                     VALUES (?, ?, ?, ?, 'active', ?, ?)",
                    params![
                        id_for_move,
                        new.tenant_id,
                        new.email,
                        new.display_name,
                        now,
                        now,
                    ],
                )?;
                Ok::<_, OrgStoreError>(())
            })
            .await?;
        Ok(id)
    }

    pub async fn get(&self, id: &str) -> Result<User, OrgStoreError> {
        let id = id.to_string();
        let id_for_err = id.clone();
        self.pool
            .with_conn(move |conn| {
                conn.query_row(
                    "SELECT id, tenant_id, email, display_name, status, created_at, updated_at
                     FROM users WHERE id = ?",
                    params![id],
                    |r| {
                        Ok(User {
                            id: r.get(0)?,
                            tenant_id: r.get(1)?,
                            email: r.get(2)?,
                            display_name: r.get(3)?,
                            status: r.get(4)?,
                            created_at: r.get(5)?,
                            updated_at: r.get(6)?,
                        })
                    },
                )
                .map_err(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => OrgStoreError::NotFound(id_for_err),
                    other => OrgStoreError::Sqlite(other),
                })
            })
            .await
    }

    pub async fn soft_deactivate(&self, id: &str) -> Result<(), OrgStoreError> {
        let id = id.to_string();
        let now = now_micros();
        self.pool
            .with_conn(move |conn| {
                let n = conn.execute(
                    "UPDATE users SET status = 'deactivated', updated_at = ? WHERE id = ?",
                    params![now, id.clone()],
                )?;
                if n == 0 {
                    Err(OrgStoreError::NotFound(id))
                } else {
                    Ok(())
                }
            })
            .await
    }
}

// ============================================================================
// OrganizationMembership
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Membership {
    pub id: String,
    pub tenant_id: String,
    pub organization_id: String,
    pub user_id: String,
    pub role: Role,
    pub is_primary: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewMembership {
    pub tenant_id: String,
    pub organization_id: String,
    pub user_id: String,
    pub role: Role,
    pub is_primary: bool,
}

#[derive(Clone)]
pub struct MembershipStore {
    pool: DbPool,
}

impl MembershipStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, new: NewMembership) -> Result<String, OrgStoreError> {
        let id = format!("membership-{}", Uuid::new_v4());
        let id_for_move = id.clone();
        let now = now_micros();
        let role_str = role_to_db(new.role);
        let is_primary_int = if new.is_primary { 1 } else { 0 };
        self.pool
            .with_conn(move |conn| {
                conn.execute(
                    "INSERT INTO organization_memberships
                     (id, tenant_id, organization_id, user_id, role, is_primary,
                      created_at, updated_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        id_for_move,
                        new.tenant_id,
                        new.organization_id,
                        new.user_id,
                        role_str,
                        is_primary_int,
                        now,
                        now,
                    ],
                )?;
                Ok::<_, OrgStoreError>(())
            })
            .await?;
        Ok(id)
    }

    pub async fn list_for_user(&self, user_id: &str) -> Result<Vec<Membership>, OrgStoreError> {
        let user_id = user_id.to_string();
        self.pool
            .with_conn(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, tenant_id, organization_id, user_id, role, is_primary,
                            created_at, updated_at
                     FROM organization_memberships
                     WHERE user_id = ?
                     ORDER BY is_primary DESC, created_at ASC",
                )?;
                let rows = stmt
                    .query_map(params![user_id], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                            r.get::<_, String>(4)?,
                            r.get::<_, i64>(5)?,
                            r.get::<_, i64>(6)?,
                            r.get::<_, i64>(7)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                let mut out = Vec::with_capacity(rows.len());
                for (
                    id,
                    tenant_id,
                    org_id,
                    user_id,
                    role_str,
                    is_primary,
                    created_at,
                    updated_at,
                ) in rows
                {
                    out.push(Membership {
                        id,
                        tenant_id,
                        organization_id: org_id,
                        user_id,
                        role: role_from_db(&role_str)?,
                        is_primary: is_primary != 0,
                        created_at,
                        updated_at,
                    });
                }
                Ok(out)
            })
            .await
    }

    pub async fn update_role(
        &self,
        membership_id: &str,
        new_role: Role,
    ) -> Result<(), OrgStoreError> {
        let membership_id = membership_id.to_string();
        let now = now_micros();
        let role_str = role_to_db(new_role);
        self.pool
            .with_conn(move |conn| {
                let n = conn.execute(
                    "UPDATE organization_memberships
                     SET role = ?, updated_at = ?
                     WHERE id = ?",
                    params![role_str, now, membership_id.clone()],
                )?;
                if n == 0 {
                    Err(OrgStoreError::NotFound(membership_id))
                } else {
                    Ok(())
                }
            })
            .await
    }
}

// ============================================================================
// ProjectRoleOverride
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRoleOverride {
    pub id: String,
    pub tenant_id: String,
    pub project_id: String,
    pub user_id: String,
    pub role: Role,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewProjectRoleOverride {
    pub tenant_id: String,
    pub project_id: String,
    pub user_id: String,
    pub role: Role,
}

#[derive(Clone)]
pub struct ProjectRoleOverrideStore {
    pool: DbPool,
}

impl ProjectRoleOverrideStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, new: NewProjectRoleOverride) -> Result<String, OrgStoreError> {
        let id = format!("pro-{}", Uuid::new_v4());
        let id_for_move = id.clone();
        let now = now_micros();
        let role_str = role_to_db(new.role);
        self.pool
            .with_conn(move |conn| {
                conn.execute(
                    "INSERT INTO project_role_overrides
                     (id, tenant_id, project_id, user_id, role, created_at, updated_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?)",
                    params![
                        id_for_move,
                        new.tenant_id,
                        new.project_id,
                        new.user_id,
                        role_str,
                        now,
                        now,
                    ],
                )?;
                Ok::<_, OrgStoreError>(())
            })
            .await?;
        Ok(id)
    }

    pub async fn for_user_project(
        &self,
        user_id: &str,
        project_id: &str,
    ) -> Result<Option<Role>, OrgStoreError> {
        let user_id = user_id.to_string();
        let project_id = project_id.to_string();
        let row: Option<String> = self
            .pool
            .with_conn(move |conn| {
                conn.query_row(
                    "SELECT role FROM project_role_overrides
                     WHERE user_id = ? AND project_id = ?",
                    params![user_id, project_id],
                    |r| r.get::<_, String>(0),
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(OrgStoreError::Sqlite(other)),
                })
            })
            .await?;
        match row {
            Some(s) => Ok(Some(role_from_db(&s)?)),
            None => Ok(None),
        }
    }
}

pub mod deactivation;
pub use deactivation::{DeactivationError, DeactivationOutcome, UserDeactivationService};
pub mod invitation;
pub use invitation::{InvitationError, InvitationService, InviteOutcome, MembershipRow};

#[cfg(test)]
mod invitation_tests;
#[cfg(test)]
mod tests;
