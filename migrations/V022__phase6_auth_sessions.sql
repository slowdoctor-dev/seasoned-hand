-- Phase 6 / issue #7 (ADR-018): verified session credentials.
--
-- Replaces the unsigned, client-asserted identity token of ADR-017 with an
-- opaque server-stored session token. A random token is issued at login; only
-- its SHA-256 hash is stored here, together with the identity resolved from the
-- consumed invitation + primary membership and an expiry. The auth middleware
-- looks a presented token up by hash on every request.
--
-- user_id is a plain column (no FK to users): sessions are ephemeral and the
-- loopback dev-login affordance issues sessions for a synthetic dev identity
-- that need not correspond to a seeded users row. Real logins always carry a
-- real user_id resolved from user_invitation_tokens.

CREATE TABLE IF NOT EXISTS auth_sessions (
    token_hash       TEXT PRIMARY KEY,
    user_id          TEXT NOT NULL,
    tenant_id        TEXT NOT NULL,
    organization_id  TEXT NOT NULL,
    org_role         TEXT NOT NULL CHECK(org_role IN ('admin','user','viewer')),
    created_at       INTEGER NOT NULL,
    expires_at       INTEGER NOT NULL CHECK(expires_at > created_at),
    revoked_at       INTEGER
);

CREATE INDEX IF NOT EXISTS idx_auth_sessions_user ON auth_sessions(user_id);
CREATE INDEX IF NOT EXISTS idx_auth_sessions_expires ON auth_sessions(expires_at);
