-- Issue #6 post-hoc security review (batch E #41 follow-up).
--
-- Bind invitation login tokens to the organization they were minted for. The
-- token previously stored only (token_hash, user_id), so the login path
-- (auth::AuthSessionStore::login) resolved identity from the user's PRIMARY
-- membership instead of the org/role the invitation was issued for — minting a
-- session scoped to the wrong organization for any user with multiple
-- memberships.
--
-- Nullable on purpose: pre-existing rows predate org-binding. Invitation tokens
-- are single-use and short-TTL (7 days), so legacy rows expire rather than need
-- a backfill; login falls back to primary-membership resolution only for a NULL
-- organization_id (documented legacy path).
--
-- ON DELETE SET NULL: if the org row is later removed, the token simply reverts
-- to the legacy (primary) resolution rather than dangling a stale FK.

PRAGMA foreign_keys = OFF;

ALTER TABLE user_invitation_tokens
    ADD COLUMN organization_id TEXT REFERENCES organizations(id) ON DELETE SET NULL;

PRAGMA foreign_keys = ON;
PRAGMA foreign_key_check;
