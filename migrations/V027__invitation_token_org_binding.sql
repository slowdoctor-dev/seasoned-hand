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
-- ON DELETE CASCADE (matching organization_memberships' FK to organizations): if
-- the bound org is hard-deleted, its pending invitation tokens are removed too —
-- an invite to a deleted org is void. Crucially this is NOT `SET NULL`: SET NULL
-- would silently rebind a still-valid token to the user's *primary* membership
-- via the NULL legacy path (re-introducing the wrong-org bug this migration
-- fixes). With CASCADE, a NULL organization_id can only mean a genuine pre-V027
-- legacy row, never a post-deletion artifact.

PRAGMA foreign_keys = OFF;

ALTER TABLE user_invitation_tokens
    ADD COLUMN organization_id TEXT REFERENCES organizations(id) ON DELETE CASCADE;

PRAGMA foreign_keys = ON;
PRAGMA foreign_key_check;
