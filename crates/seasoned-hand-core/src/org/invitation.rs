use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::audit::{AuditAction, AuditLogger, AuditRecord};
use crate::auth::{Action, AuthContext, AuthError, AuthResource, Role, authorize};
use crate::db::DbPool;
use crate::hash::sha256_hex;
use crate::time::now_micros;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InviteOutcome {
    pub user_id: String,
    pub display_name: String,
    pub login_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MembershipRow {
    pub user_id: String,
    pub email: String,
    pub display_name: String,
    pub role: String,
    pub status: String,
}

#[derive(Debug, Error)]
pub enum InvitationError {
    #[error("auth: {0}")]
    Auth(#[from] AuthError),
    #[error("organization not found for slug: {0}")]
    OrganizationNotFound(String),
    #[error("cross-tenant organization access denied")]
    CrossTenantDenied,
    #[error("invalid role: {0}")]
    InvalidRole(String),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("audit write: {0}")]
    AuditWrite(String),
}

#[derive(Clone)]
pub struct InvitationService {
    db: DbPool,
    audit: AuditLogger,
}

impl InvitationService {
    pub fn new(db: DbPool, audit: AuditLogger) -> Self {
        Self { db, audit }
    }

    pub async fn invite_user(
        &self,
        auth: &AuthContext,
        organization_slug: &str,
        email: &str,
        role: &str,
    ) -> Result<InviteOutcome, InvitationError> {
        authorize(
            Action::MembershipManage,
            &AuthResource {
                is_same_org: true,
                actor_can_share: true,
            },
            auth,
        )?;

        let role = parse_role(role)?;
        let slug = organization_slug.to_string();
        let email = email.trim().to_ascii_lowercase();
        let email_for_tx = email.clone();
        let email_for_audit = email.clone();
        let tenant = auth.tenant_id.clone();
        let now = now_micros();

        let (user_id, display_name, token_plain, org_id) = self
            .db
            .with_conn(move |conn| -> Result<(String, String, String, String), InvitationError> {
                let org_row: Option<(String, String)> = conn
                    .query_row(
                        "SELECT id, tenant_id FROM organizations WHERE slug = ?1",
                        [slug.clone()],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;
                let Some((organization_id, org_tenant)) = org_row else {
                    return Err(InvitationError::OrganizationNotFound(slug));
                };
                if org_tenant != tenant {
                    return Err(InvitationError::CrossTenantDenied);
                }

                let tx = conn.transaction()?;

                let existing_user: Option<(String, String)> = tx
                    .query_row(
                        "SELECT id, display_name FROM users WHERE tenant_id = ?1 AND email = ?2",
                        params![org_tenant, email_for_tx.clone()],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;

                let (user_id, display_name) = if let Some((id, name)) = existing_user {
                    (id, name)
                } else {
                    let id = format!("user-{}", Uuid::new_v4());
                    let display_name = email_for_tx
                        .split('@')
                        .next()
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or("user")
                        .to_string();
                    tx.execute(
                        "INSERT INTO users (id, tenant_id, email, display_name, status, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?5)",
                        params![id, org_tenant, email_for_tx.clone(), display_name, now],
                    )?;
                    (id, display_name)
                };

                let has_membership: Option<String> = tx
                    .query_row(
                        "SELECT id FROM organization_memberships
                         WHERE organization_id = ?1 AND user_id = ?2",
                        params![organization_id, user_id],
                        |row| row.get(0),
                    )
                    .optional()?;

                if has_membership.is_none() {
                    let existing_count: i64 = tx.query_row(
                        "SELECT COUNT(*) FROM organization_memberships WHERE user_id = ?1",
                        [user_id.clone()],
                        |row| row.get(0),
                    )?;
                    let membership_id = format!("membership-{}", Uuid::new_v4());
                    tx.execute(
                        "INSERT INTO organization_memberships
                         (id, tenant_id, organization_id, user_id, role, is_primary, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                        params![
                            membership_id,
                            org_tenant,
                            organization_id,
                            user_id,
                            role_to_db(role),
                            if existing_count == 0 { 1 } else { 0 },
                            now
                        ],
                    )?;
                }

                let token_plain = generate_login_token();
                let token_hash = sha256_hex(token_plain.as_bytes());
                tx.execute(
                    "INSERT INTO user_invitation_tokens (token_hash, user_id, created_at, consumed_at)
                     VALUES (?1, ?2, ?3, NULL)",
                    params![token_hash, user_id, now],
                )?;

                tx.commit()?;
                Ok((user_id, display_name, token_plain, organization_id))
            })
            .await?;

        self.audit
            .record(
                auth,
                AuditRecord {
                    action: AuditAction::UserInvite,
                    resource_type: "organization",
                    resource_id: &org_id,
                    target_user_id: Some(&user_id),
                    decision: Some("allow"),
                    reason: None,
                    metadata: serde_json::json!({
                        "email": email_for_audit,
                        "role": role_to_db(role),
                        "organization_slug": organization_slug,
                    }),
                },
            )
            .await
            .map_err(|e| InvitationError::AuditWrite(e.to_string()))?;

        Ok(InviteOutcome {
            user_id,
            display_name,
            login_token: token_plain,
        })
    }

    pub async fn list_org_users(
        &self,
        auth: &AuthContext,
        organization_slug: &str,
    ) -> Result<Vec<MembershipRow>, InvitationError> {
        authorize(
            Action::MembershipManage,
            &AuthResource {
                is_same_org: true,
                actor_can_share: true,
            },
            auth,
        )?;
        let slug = organization_slug.to_string();
        let slug_for_err = slug.clone();
        let tenant = auth.tenant_id.clone();
        self.db
            .with_conn(move |conn| {
                let org_row: Option<(String, String)> = conn
                    .query_row(
                        "SELECT id, tenant_id FROM organizations WHERE slug = ?1",
                        [slug],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;
                let Some((org_id, org_tenant)) = org_row else {
                    return Err(InvitationError::OrganizationNotFound(slug_for_err));
                };
                if org_tenant != tenant {
                    return Err(InvitationError::CrossTenantDenied);
                }
                let mut stmt = conn.prepare(
                    "SELECT m.user_id, u.email, u.display_name, m.role, u.status
                     FROM organization_memberships m
                     JOIN users u ON u.id = m.user_id
                     WHERE m.organization_id = ?1
                     ORDER BY m.role ASC, u.email ASC",
                )?;
                let rows = stmt.query_map([org_id], |row| {
                    Ok(MembershipRow {
                        user_id: row.get(0)?,
                        email: row.get(1)?,
                        display_name: row.get(2)?,
                        role: row.get(3)?,
                        status: row.get(4)?,
                    })
                })?;
                rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
            })
            .await
    }
}

fn parse_role(role: &str) -> Result<Role, InvitationError> {
    match role {
        "admin" => Ok(Role::Admin),
        "user" => Ok(Role::User),
        "viewer" => Ok(Role::Viewer),
        other => Err(InvitationError::InvalidRole(other.to_string())),
    }
}

fn role_to_db(role: Role) -> &'static str {
    match role {
        Role::Admin => "admin",
        Role::User => "user",
        Role::Viewer => "viewer",
    }
}

fn generate_login_token() -> String {
    let mut bytes = [0_u8; 32];
    bytes[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(Uuid::new_v4().as_bytes());
    base64url_no_pad(&bytes)
}

fn base64url_no_pad(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity((input.len() * 4).div_ceil(3));
    let mut i = 0;
    while i + 3 <= input.len() {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8) | input[i + 2] as u32;
        out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 6) & 0x3f) as usize] as char);
        out.push(TABLE[(n & 0x3f) as usize] as char);
        i += 3;
    }
    let rem = input.len() - i;
    if rem == 1 {
        let n = (input[i] as u32) << 16;
        out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
    } else if rem == 2 {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8);
        out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 6) & 0x3f) as usize] as char);
    }
    out
}
