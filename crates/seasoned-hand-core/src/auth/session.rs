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
use crate::org::invitation::LOGIN_TOKEN_TTL_MICROS;
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
    /// invitation (must exist, be unconsumed, and be within its TTL), resolves
    /// identity from the membership the invitation was minted for, consumes the
    /// invitation, and inserts a session — all in one transaction.
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

                    // Issue #6 review: read the token's bound organization +
                    // mint time. `organization_id` is nullable only for legacy
                    // (pre-V027) tokens.
                    let invite: Option<(String, Option<String>, i64)> = tx
                        .query_row(
                            "SELECT user_id, organization_id, created_at \
                             FROM user_invitation_tokens \
                             WHERE token_hash = ?1 AND consumed_at IS NULL",
                            [&invite_hash],
                            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                        )
                        .optional()?;
                    let (user_id, invite_org_id, created_at) =
                        invite.ok_or(AuthLoginError::InvalidInvitation)?;

                    // Enforce the 7-day invitation TTL. Previously unchecked on
                    // this (live) path — an unconsumed token never expired. An
                    // expired token reports as `InvalidInvitation` (same as
                    // unknown/used) so the boundary leaks no expiry oracle.
                    if now.saturating_sub(created_at) > LOGIN_TOKEN_TTL_MICROS {
                        return Err(AuthLoginError::InvalidInvitation);
                    }

                    // Resolve the membership the invitation was minted for — of
                    // an ACTIVE user in an ACTIVE org. A deactivated user or
                    // suspended/archived org cannot exchange an invitation.
                    //
                    // Issue #6 review: bind to the token's `organization_id` so a
                    // user with multiple memberships gets a session for the org
                    // the invite was issued for, NOT whichever happens to be
                    // primary. Legacy (NULL-org) tokens fall back to primary
                    // resolution; they are single-use and short-TTL, so they
                    // expire rather than need a backfill.
                    let membership: Option<(String, String, String)> = match &invite_org_id {
                        Some(org_id) => tx
                            .query_row(
                                "SELECT m.tenant_id, m.organization_id, m.role \
                                 FROM organization_memberships m \
                                 JOIN users u ON u.id = m.user_id AND u.status = 'active' \
                                 JOIN organizations o \
                                   ON o.id = m.organization_id AND o.status = 'active' \
                                 WHERE m.user_id = ?1 AND m.organization_id = ?2",
                                rusqlite::params![&user_id, org_id],
                                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                            )
                            .optional()?,
                        None => tx
                            .query_row(
                                "SELECT m.tenant_id, m.organization_id, m.role \
                                 FROM organization_memberships m \
                                 JOIN users u ON u.id = m.user_id AND u.status = 'active' \
                                 JOIN organizations o \
                                   ON o.id = m.organization_id AND o.status = 'active' \
                                 WHERE m.user_id = ?1 \
                                 ORDER BY m.is_primary DESC, m.created_at ASC LIMIT 1",
                                [&user_id],
                                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                            )
                            .optional()?,
                    };
                    let (tenant_id, organization_id, role_db) =
                        membership.ok_or(AuthLoginError::NoMembership)?;

                    // Conditional consume: re-assert unconsumed in the write
                    // predicate and require exactly one affected row, so a
                    // concurrent exchange (future multi-connection pool / second
                    // process) cannot double-spend one invitation.
                    let consumed = tx.execute(
                        "UPDATE user_invitation_tokens SET consumed_at = ?1 \
                         WHERE token_hash = ?2 AND consumed_at IS NULL",
                        rusqlite::params![now, invite_hash],
                    )?;
                    if consumed != 1 {
                        return Err(AuthLoginError::InvalidInvitation);
                    }
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
                // Re-resolve live identity on every verify: a session authenticates
                // only while the user is active, the org is active, and the
                // membership still exists — and authorizes with the CURRENT
                // membership role, not the role snapshotted at login. So user
                // deactivation, org suspension, membership removal, and role
                // changes take effect immediately, not at token expiry.
                conn.query_row(
                    "SELECT m.tenant_id, m.organization_id, s.user_id, m.role \
                     FROM auth_sessions s \
                     JOIN users u ON u.id = s.user_id AND u.status = 'active' \
                     JOIN organizations o \
                       ON o.id = s.organization_id AND o.status = 'active' \
                     JOIN organization_memberships m \
                       ON m.user_id = s.user_id AND m.organization_id = s.organization_id \
                     WHERE s.token_hash = ?1 AND s.revoked_at IS NULL AND s.expires_at > ?2",
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

    /// Idempotently seed a default dev identity (active org + user + admin
    /// membership) and issue a session for it. Dev-only — gated by loopback +
    /// `SH_INSECURE_AUTH_HEADERS` at the caller. Seeding real rows keeps the
    /// session valid under `verify`'s live-identity re-resolution and prevents FK
    /// breakage when dev requests write audit / membership rows.
    pub async fn issue_dev_session(&self) -> Result<LoginResult, rusqlite::Error> {
        const DEV_TENANT: &str = "default";
        const DEV_ORG: &str = "default";
        const DEV_USER: &str = "dev-user";
        let now = now_micros();
        let expires_at = now + SESSION_TTL_MICROS;
        let token = generate_token();
        let hash = sha256_hex(token.as_bytes());
        self.db
            .with_conn(move |conn| -> rusqlite::Result<()> {
                let tx = conn.transaction()?;
                tx.execute(
                    "INSERT OR IGNORE INTO organizations \
                     (id, tenant_id, slug, display_name, status, created_at, updated_at) \
                     VALUES (?1, ?1, 'dev', 'Dev Org', 'active', ?2, ?2)",
                    rusqlite::params![DEV_ORG, now],
                )?;
                tx.execute(
                    "INSERT OR IGNORE INTO users \
                     (id, tenant_id, email, display_name, status, created_at, updated_at) \
                     VALUES (?1, ?2, 'dev@localhost', 'Dev User', 'active', ?3, ?3)",
                    rusqlite::params![DEV_USER, DEV_TENANT, now],
                )?;
                tx.execute(
                    "INSERT OR IGNORE INTO organization_memberships \
                     (id, tenant_id, organization_id, user_id, role, is_primary, \
                      created_at, updated_at) \
                     VALUES ('m-dev-user', ?1, ?2, ?3, 'admin', 1, ?4, ?4)",
                    rusqlite::params![DEV_TENANT, DEV_ORG, DEV_USER, now],
                )?;
                tx.execute(
                    "INSERT INTO auth_sessions \
                     (token_hash, user_id, tenant_id, organization_id, org_role, \
                      created_at, expires_at, revoked_at) \
                     VALUES (?1, ?2, ?3, ?4, 'admin', ?5, ?6, NULL)",
                    rusqlite::params![hash, DEV_USER, DEV_TENANT, DEV_ORG, now, expires_at],
                )?;
                tx.commit()
            })
            .await?;
        Ok(LoginResult {
            token,
            context: AuthContext {
                tenant_id: DEV_TENANT.to_string(),
                organization_id: DEV_ORG.to_string(),
                actor_user_id: DEV_USER.to_string(),
                org_role: Role::Admin,
                project_override_role: None,
            },
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
                 VALUES (?1, ?2, ?3, NULL)",
                rusqlite::params![ih, uid, now_micros()],
            )
            .unwrap();
        })
        .await;
    }

    /// Seed a user with two active memberships across distinct tenants —
    /// primary `o1`/`t1`/admin and secondary `o2`/`t2`/viewer — plus a third
    /// org `o3`/`t3` the user is NOT a member of, and an invitation token bound
    /// to `bound_org`. (`organizations.tenant_id` is UNIQUE, so distinct orgs
    /// are distinct tenants — which is exactly why resolving the *token's* org
    /// rather than the primary membership matters: a stale primary would mint a
    /// cross-tenant session.) Used to prove login binds to the invitation's org.
    async fn seed_multi_membership_invite(
        pool: &DbPool,
        user_id: &str,
        invite_token: &str,
        bound_org: &str,
    ) {
        let invite_hash = sha256_hex(invite_token.as_bytes());
        let (uid, ih, org) = (user_id.to_string(), invite_hash, bound_org.to_string());
        pool.with_conn(move |conn| {
            for (oid, tenant, slug) in [
                ("o1", "t1", "o1-slug"),
                ("o2", "t2", "o2-slug"),
                ("o3", "t3", "o3-slug"),
            ] {
                conn.execute(
                    "INSERT OR IGNORE INTO organizations \
                     (id, tenant_id, slug, display_name, status, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, 'Org', 'active', 1, 1)",
                    rusqlite::params![oid, tenant, slug],
                )
                .unwrap();
            }
            conn.execute(
                "INSERT INTO users (id, tenant_id, email, display_name, status, created_at, updated_at) \
                 VALUES (?1, 't1', ?2, 'Dev', 'active', 1, 1)",
                rusqlite::params![uid, format!("{uid}@example.test")],
            )
            .unwrap();
            // Primary membership: o1 / t1 / admin.
            conn.execute(
                "INSERT INTO organization_memberships \
                 (id, tenant_id, organization_id, user_id, role, is_primary, created_at, updated_at) \
                 VALUES (?1, 't1', 'o1', ?2, 'admin', 1, 1, 1)",
                rusqlite::params![format!("m1-{uid}"), uid],
            )
            .unwrap();
            // Secondary membership: o2 / t2 / viewer. (No membership in o3.)
            conn.execute(
                "INSERT INTO organization_memberships \
                 (id, tenant_id, organization_id, user_id, role, is_primary, created_at, updated_at) \
                 VALUES (?1, 't2', 'o2', ?2, 'viewer', 0, 2, 2)",
                rusqlite::params![format!("m2-{uid}"), uid],
            )
            .unwrap();
            // Invitation token bound to the chosen org.
            conn.execute(
                "INSERT INTO user_invitation_tokens \
                 (token_hash, user_id, organization_id, created_at, consumed_at) \
                 VALUES (?1, ?2, ?3, ?4, NULL)",
                rusqlite::params![ih, uid, org, now_micros()],
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
    async fn login_binds_session_to_invitation_org_not_primary() {
        // Issue #6 review: token bound to o2 (viewer); the user's PRIMARY
        // membership is o1 (admin). The session must be scoped to o2/viewer,
        // not the primary o1/admin.
        let pool = open(":memory:").await.unwrap();
        seed_multi_membership_invite(&pool, "u1", "invite-o2", "o2").await;
        let store = AuthSessionStore::new(pool);

        let result = store.login("invite-o2").await.expect("login ok");
        assert_eq!(result.context.organization_id, "o2");
        assert_eq!(result.context.tenant_id, "t2");
        assert_eq!(result.context.org_role, Role::Viewer);

        let verified = store.verify(&result.token).await.expect("verify ok");
        assert_eq!(verified.organization_id, "o2");
        assert_eq!(verified.tenant_id, "t2");
        assert_eq!(verified.org_role, Role::Viewer);
    }

    #[tokio::test]
    async fn login_to_org_with_no_membership_is_rejected() {
        // A token bound to an org the user is NOT a member of must fail closed
        // rather than silently fall back to another membership.
        let pool = open(":memory:").await.unwrap();
        seed_multi_membership_invite(&pool, "u1", "invite-o3", "o3").await;
        let store = AuthSessionStore::new(pool);
        assert!(matches!(
            store.login("invite-o3").await,
            Err(AuthLoginError::NoMembership)
        ));
    }

    #[tokio::test]
    async fn expired_invitation_is_rejected() {
        // Issue #6 review: the live login path now enforces the 7-day TTL.
        let pool = open(":memory:").await.unwrap();
        seed_user_with_invite(&pool, "u1", "invite-old", "user").await;
        pool.with_conn(|conn| {
            let stale = now_micros() - LOGIN_TOKEN_TTL_MICROS - 1;
            conn.execute(
                "UPDATE user_invitation_tokens SET created_at = ?1 WHERE user_id = 'u1'",
                [stale],
            )
            .unwrap();
        })
        .await;
        let store = AuthSessionStore::new(pool);
        assert!(matches!(
            store.login("invite-old").await,
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
    async fn issue_dev_session_creates_a_verifiable_session() {
        let pool = open(":memory:").await.unwrap();
        let store = AuthSessionStore::new(pool);
        // No pre-seeding: issue_dev_session seeds its own active dev identity.
        let result = store.issue_dev_session().await.unwrap();
        let verified = store.verify(&result.token).await.expect("verify ok");
        assert_eq!(verified.actor_user_id, "dev-user");
        assert_eq!(verified.org_role, Role::Admin);
    }

    async fn set_user_status(pool: &DbPool, user_id: &str, status: &str) {
        let (uid, st) = (user_id.to_string(), status.to_string());
        pool.with_conn(move |conn| {
            conn.execute(
                "UPDATE users SET status = ?1 WHERE id = ?2",
                rusqlite::params![st, uid],
            )
            .unwrap();
        })
        .await;
    }

    async fn set_membership_role(pool: &DbPool, user_id: &str, role: &str) {
        let (uid, r) = (user_id.to_string(), role.to_string());
        pool.with_conn(move |conn| {
            conn.execute(
                "UPDATE organization_memberships SET role = ?1 WHERE user_id = ?2",
                rusqlite::params![r, uid],
            )
            .unwrap();
        })
        .await;
    }

    #[tokio::test]
    async fn deactivated_user_cannot_login() {
        let pool = open(":memory:").await.unwrap();
        seed_user_with_invite(&pool, "u1", "invite-ddd", "admin").await;
        set_user_status(&pool, "u1", "deactivated").await;
        let store = AuthSessionStore::new(pool);
        assert!(matches!(
            store.login("invite-ddd").await,
            Err(AuthLoginError::NoMembership)
        ));
    }

    #[tokio::test]
    async fn session_stops_verifying_after_user_deactivation() {
        let pool = open(":memory:").await.unwrap();
        seed_user_with_invite(&pool, "u1", "invite-eee", "admin").await;
        let store = AuthSessionStore::new(pool.clone());

        let result = store.login("invite-eee").await.unwrap();
        assert!(store.verify(&result.token).await.is_some());

        // Deactivating the user invalidates the already-issued session.
        set_user_status(&pool, "u1", "deactivated").await;
        assert!(store.verify(&result.token).await.is_none());
    }

    #[tokio::test]
    async fn role_downgrade_takes_effect_on_next_verify() {
        let pool = open(":memory:").await.unwrap();
        seed_user_with_invite(&pool, "u1", "invite-fff", "admin").await;
        let store = AuthSessionStore::new(pool.clone());

        let result = store.login("invite-fff").await.unwrap();
        assert_eq!(
            store.verify(&result.token).await.unwrap().org_role,
            Role::Admin
        );

        // Downgrade to viewer — verify authorizes with the CURRENT role.
        set_membership_role(&pool, "u1", "viewer").await;
        assert_eq!(
            store.verify(&result.token).await.unwrap().org_role,
            Role::Viewer
        );
    }
}
