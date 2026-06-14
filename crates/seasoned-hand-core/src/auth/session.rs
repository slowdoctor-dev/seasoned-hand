//! Verified session credentials (issue #7 / ADR-018).
//!
//! Replaces the unsigned, client-asserted identity token of ADR-017 with an
//! **opaque server-stored session token**: a random token is issued at login,
//! its SHA-256 hash is persisted in `auth_sessions` together with the resolved
//! identity + expiry, and the auth middleware looks a presented token up by hash
//! on every request. The token value is never stored, so a DB read does not leak
//! credentials; lookup is an O(1) primary-key probe on the hash.
//!
//! This module owns the *credential*; the browser *transport* (Bearer + WS
//! subprotocol) is ADR-017 and lives in the server's auth middleware.

use crate::auth::{AuthContext, Role};
use crate::db::DbPool;
use crate::hash::sha256_hex;
use crate::time::now_micros;
use rusqlite::OptionalExtension;
use uuid::Uuid;

/// Session lifetime: 7 days (microseconds, matching the project time unit).
const SESSION_TTL_MICROS: i64 = 7 * 24 * 60 * 60 * 1_000_000;

#[derive(Debug, thiserror::Error)]
pub enum AuthLoginError {
    #[error("invalid or already-used invitation token")]
    InvalidInvitation,
    #[error("user has no organization membership")]
    NoMembership,
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
}

/// A freshly issued session: the plaintext token (returned to the caller once)
/// plus the identity it authenticates and its absolute expiry (micros).
pub struct LoginResult {
    pub token: String,
    pub context: AuthContext,
    pub expires_at: i64,
}

/// Persistence + verification for opaque session tokens.
#[derive(Clone)]
pub struct AuthSessionStore {
    db: DbPool,
}

impl AuthSessionStore {
    pub fn new(db: DbPool) -> Self {
        Self { db }
    }

    /// Exchange a single-use invitation token for a session token. Validates the
    /// invitation (must exist and be unconsumed), resolves identity from the
    /// user's primary membership, consumes the invitation, and inserts a session
    /// — all in one transaction.
    pub async fn login(&self, invitation_token: &str) -> Result<LoginResult, AuthLoginError> {
        let now = now_micros();
        let expires_at = now + SESSION_TTL_MICROS;
        let invite_hash = sha256_hex(invitation_token.as_bytes());
        let token = generate_token();
        let session_hash = sha256_hex(token.as_bytes());

        let (user_id, tenant_id, organization_id, role_db) = self
            .db
            .with_conn(
                move |conn| -> Result<(String, String, String, String), AuthLoginError> {
                    let tx = conn.transaction()?;

                    let user_id: Option<String> = tx
                        .query_row(
                            "SELECT user_id FROM user_invitation_tokens \
                             WHERE token_hash = ?1 AND consumed_at IS NULL",
                            [&invite_hash],
                            |r| r.get(0),
                        )
                        .optional()?;
                    let user_id = user_id.ok_or(AuthLoginError::InvalidInvitation)?;

                    let membership: Option<(String, String, String)> = tx
                        .query_row(
                            "SELECT tenant_id, organization_id, role \
                             FROM organization_memberships \
                             WHERE user_id = ?1 \
                             ORDER BY is_primary DESC, created_at ASC LIMIT 1",
                            [&user_id],
                            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                        )
                        .optional()?;
                    let (tenant_id, organization_id, role_db) =
                        membership.ok_or(AuthLoginError::NoMembership)?;

                    tx.execute(
                        "UPDATE user_invitation_tokens SET consumed_at = ?1 WHERE token_hash = ?2",
                        rusqlite::params![now, invite_hash],
                    )?;
                    tx.execute(
                        "INSERT INTO auth_sessions \
                         (token_hash, user_id, tenant_id, organization_id, org_role, \
                          created_at, expires_at, revoked_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
                        rusqlite::params![
                            session_hash,
                            user_id,
                            tenant_id,
                            organization_id,
                            role_db,
                            now,
                            expires_at
                        ],
                    )?;
                    tx.commit()?;
                    Ok((user_id, tenant_id, organization_id, role_db))
                },
            )
            .await?;

        Ok(LoginResult {
            token,
            context: AuthContext {
                tenant_id,
                organization_id,
                actor_user_id: user_id,
                org_role: role_from_db(&role_db),
                project_override_role: None,
            },
            expires_at,
        })
    }

    /// Verify a presented session token. Returns the stored identity iff a
    /// matching session exists, is not revoked, and has not expired.
    pub async fn verify(&self, token: &str) -> Option<AuthContext> {
        let now = now_micros();
        let hash = sha256_hex(token.as_bytes());
        self.db
            .with_conn(move |conn| {
                conn.query_row(
                    "SELECT tenant_id, organization_id, user_id, org_role FROM auth_sessions \
                     WHERE token_hash = ?1 AND revoked_at IS NULL AND expires_at > ?2",
                    rusqlite::params![hash, now],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                        ))
                    },
                )
                .optional()
                .ok()
                .flatten()
            })
            .await
            .map(
                |(tenant_id, organization_id, actor_user_id, role_db)| AuthContext {
                    tenant_id,
                    organization_id,
                    actor_user_id,
                    org_role: role_from_db(&role_db),
                    project_override_role: None,
                },
            )
    }

    /// Issue a session for an already-known identity, bypassing the invitation
    /// flow. Used only by the loopback-gated dev-login affordance; never reachable
    /// without `SH_INSECURE_AUTH_HEADERS` + loopback at the caller.
    pub async fn issue_for(&self, context: AuthContext) -> Result<LoginResult, rusqlite::Error> {
        let now = now_micros();
        let expires_at = now + SESSION_TTL_MICROS;
        let token = generate_token();
        let hash = sha256_hex(token.as_bytes());
        let user_id = context.actor_user_id.clone();
        let tenant_id = context.tenant_id.clone();
        let organization_id = context.organization_id.clone();
        let role_db = role_to_db(context.org_role).to_string();
        self.db
            .with_conn(move |conn| {
                conn.execute(
                    "INSERT INTO auth_sessions \
                     (token_hash, user_id, tenant_id, organization_id, org_role, \
                      created_at, expires_at, revoked_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
                    rusqlite::params![
                        hash,
                        user_id,
                        tenant_id,
                        organization_id,
                        role_db,
                        now,
                        expires_at
                    ],
                )
            })
            .await?;
        Ok(LoginResult {
            token,
            context,
            expires_at,
        })
    }

    /// Revoke a session by its token. Returns whether a live session was revoked.
    pub async fn revoke(&self, token: &str) -> Result<bool, rusqlite::Error> {
        let now = now_micros();
        let hash = sha256_hex(token.as_bytes());
        let n = self
            .db
            .with_conn(move |conn| {
                conn.execute(
                    "UPDATE auth_sessions SET revoked_at = ?1 \
                     WHERE token_hash = ?2 AND revoked_at IS NULL",
                    rusqlite::params![now, hash],
                )
            })
            .await?;
        Ok(n > 0)
    }
}

/// 256-bit random token, lowercase hex (charset-safe for `Authorization` and WS
/// subprotocol headers). Mirrors `org::invitation::generate_login_token`'s
/// two-UUIDv4 construction.
fn generate_token() -> String {
    let mut bytes = [0_u8; 32];
    bytes[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(Uuid::new_v4().as_bytes());
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn role_from_db(s: &str) -> Role {
    match s {
        "admin" => Role::Admin,
        "viewer" => Role::Viewer,
        _ => Role::User,
    }
}

fn role_to_db(role: Role) -> &'static str {
    match role {
        Role::Admin => "admin",
        Role::User => "user",
        Role::Viewer => "viewer",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open;

    async fn seed_user_with_invite(pool: &DbPool, user_id: &str, invite_token: &str, role: &str) {
        let invite_hash = sha256_hex(invite_token.as_bytes());
        let (uid, ih, r) = (user_id.to_string(), invite_hash, role.to_string());
        pool.with_conn(move |conn| {
            conn.execute(
                "INSERT OR IGNORE INTO organizations \
                 (id, tenant_id, slug, display_name, status, created_at, updated_at) \
                 VALUES ('o1', 't1', 'o1-slug', 'Org', 'active', 1, 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO users (id, tenant_id, email, display_name, status, created_at, updated_at) \
                 VALUES (?1, 't1', ?2, 'Dev', 'active', 1, 1)",
                rusqlite::params![uid, format!("{uid}@example.test")],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO organization_memberships \
                 (id, tenant_id, organization_id, user_id, role, is_primary, created_at, updated_at) \
                 VALUES (?1, 't1', 'o1', ?2, ?3, 1, 1, 1)",
                rusqlite::params![format!("m-{uid}"), uid, r],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO user_invitation_tokens (token_hash, user_id, created_at, consumed_at) \
                 VALUES (?1, ?2, 1, NULL)",
                rusqlite::params![ih, uid],
            )
            .unwrap();
        })
        .await;
    }

    #[tokio::test]
    async fn login_then_verify_roundtrips_identity() {
        let pool = open(":memory:").await.unwrap();
        seed_user_with_invite(&pool, "u1", "invite-aaa", "admin").await;
        let store = AuthSessionStore::new(pool);

        let result = store.login("invite-aaa").await.expect("login ok");
        assert_eq!(result.context.actor_user_id, "u1");
        assert_eq!(result.context.org_role, Role::Admin);

        let verified = store.verify(&result.token).await.expect("verify ok");
        assert_eq!(verified.tenant_id, "t1");
        assert_eq!(verified.organization_id, "o1");
        assert_eq!(verified.actor_user_id, "u1");
        assert_eq!(verified.org_role, Role::Admin);
    }

    #[tokio::test]
    async fn invitation_is_single_use() {
        let pool = open(":memory:").await.unwrap();
        seed_user_with_invite(&pool, "u1", "invite-bbb", "user").await;
        let store = AuthSessionStore::new(pool);

        assert!(store.login("invite-bbb").await.is_ok());
        // Second login with the same (now consumed) invitation must fail.
        assert!(matches!(
            store.login("invite-bbb").await,
            Err(AuthLoginError::InvalidInvitation)
        ));
    }

    #[tokio::test]
    async fn unknown_invitation_is_rejected() {
        let pool = open(":memory:").await.unwrap();
        let store = AuthSessionStore::new(pool);
        assert!(matches!(
            store.login("nope").await,
            Err(AuthLoginError::InvalidInvitation)
        ));
    }

    #[tokio::test]
    async fn unknown_or_revoked_token_does_not_verify() {
        let pool = open(":memory:").await.unwrap();
        seed_user_with_invite(&pool, "u1", "invite-ccc", "viewer").await;
        let store = AuthSessionStore::new(pool);

        assert!(store.verify("not-a-real-token").await.is_none());

        let result = store.login("invite-ccc").await.unwrap();
        assert!(store.verify(&result.token).await.is_some());
        assert!(store.revoke(&result.token).await.unwrap());
        assert!(store.verify(&result.token).await.is_none());
    }

    #[tokio::test]
    async fn issue_for_creates_a_verifiable_session() {
        let pool = open(":memory:").await.unwrap();
        let store = AuthSessionStore::new(pool);
        let ctx = AuthContext {
            tenant_id: "default".into(),
            organization_id: "default".into(),
            actor_user_id: "dev-user".into(),
            org_role: Role::Admin,
            project_override_role: None,
        };
        let result = store.issue_for(ctx).await.unwrap();
        let verified = store.verify(&result.token).await.expect("verify ok");
        assert_eq!(verified.actor_user_id, "dev-user");
        assert_eq!(verified.org_role, Role::Admin);
    }
}
