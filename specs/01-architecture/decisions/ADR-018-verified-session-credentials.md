# ADR-018: Verified session credentials (opaque token) — replace client-asserted identity

Status: Accepted
Date: 2026-06-14
Relates to: [ADR-017](ADR-017-browser-auth-transport-subprotocol-token.md) (browser
            transport), [ADR-014](ADR-014-phase5-v013-tenant-rbac.md) (tenant + RBAC)
Closes: issue #7 (server-side). Client login UX is the follow-up (issue #26).

## Context

Through Phase 5 the control plane derived identity (`tenant_id`, `organization_id`,
`actor_user_id`, `org_role`) from plaintext `x-seasoned-hand-*` request headers, and
ADR-017 added a browser-shaped *transport* (Bearer / WS subprotocol) carrying an
**unsigned** identity token. Both are **client-asserted**: the server trusted
whatever identity the caller supplied, with only the loopback bind as a backstop
(issue #7, #8, #21). That is acceptable for single-host dev but unsafe for any
multi-user or networked deployment.

The pieces for real authentication already existed but were unused: `invite_user`
mints a single-use login token, stores `sha256(token)` in `user_invitation_tokens`,
and creates the user's `organization_memberships` row — but nothing ever verified or
consumed that token.

This ADR decides the **credential**: what the server verifies. ADR-017's transport
(how a browser carries it) is unchanged.

## Decision

**Identity is verified against a server-stored opaque session token; client-asserted
headers are no longer trusted by default.**

### Credential — opaque session token

A random 256-bit token issued at login. Only its `sha256` hash is persisted, in a
new `auth_sessions` table (migration **V022**) together with the resolved identity
(`tenant_id`, `organization_id`, `user_id`, `org_role`), `created_at`, `expires_at`
(7 days), and a nullable `revoked_at`. Verification is an O(1) primary-key probe on
the hash; the token value is never stored, so a DB read cannot leak credentials.

Chosen over a signed JWT to avoid a new crypto dependency (which would require an
ARCHITECTURE.md change), to get cheap revocation (`revoked_at`), and because the
per-request DB read is already the norm in this single-writer SQLite design.

### Login — exchange an invitation for a session

`POST /v1/auth/login { invitation_token }` (public): in one transaction, validate
the unconsumed `user_invitation_tokens` row, resolve identity from the user's primary
`organization_memberships` row, consume the invitation (`consumed_at`), insert the
session, and return `{ access_token, expires_at, tenant_id, organization_id,
actor_user_id, org_role }` once. Owned by `core::auth::AuthSessionStore`.

### Verification — middleware resolves identity in priority order

1. **Verified session token** — `Authorization: Bearer` (REST) or the non-sentinel
   `Sec-WebSocket-Protocol` entry (browser `/ws`, per ADR-017), looked up in
   `auth_sessions` (not revoked, not expired). Checked **first** so a stray
   forwarded header can't override a real credential, and a presented-but-invalid
   token is rejected rather than demoted to the header path.
2. **Legacy `x-seasoned-hand-*` headers** — accepted **only** when
   `SH_INSECURE_AUTH_HEADERS` is set (loopback dev / tests / CLI). Off by default.

The ADR-017 **unsigned identity token is removed** — it had the same wire shape as a
real bearer token, so accepting both invited confusion and fail-open. The middleware
becomes DB-backed: `app()` injects an `AuthDeps { sessions, allow_insecure_headers }`
request extension so the per-route `from_fn` auth layer can verify without threading
state through every `with_auth` call site.

### Dev affordance

`POST /v1/auth/dev-login` (loopback **and** `SH_INSECURE_AUTH_HEADERS` only,
startup-warned) issues a session for a default dev identity so local browser dev
works before the client login UX (#26) lands.

## Consequences

**Positive:**
- The server no longer trusts client-asserted identity by default — closes the core
  of issue #7. Tenant isolation and RBAC now rest on a verified credential.
- Revocable, expiring sessions; invitation tokens become single-use as designed.
- Drops onto ADR-017's transport with no client wire change beyond *which* token is
  sent; verification replaced only the token-decode step.

**Negative / accepted:**
- A per-request DB read for token verification (acceptable on the existing
  single-writer pool; can move to a cache later).
- Browser dev requires `/v1/auth/dev-login` (or a real login) — the unsigned
  fallback is gone. The full client login UX is deferred to #26; until then
  `SH_INSECURE_AUTH_HEADERS` + dev-login cover local dev.
- The `x-seasoned-hand-*` header path still exists (behind the flag) for tests /
  CLI; it is insecure by definition and loudly warned at startup.

**Neutral / out of scope:**
- The RBAC resource-flag fail-open (#8) and the middleware `RouteAction`
  fail-open (#21) are unchanged here — separate issues.
- Token rotation/refresh, session listing, and per-user revocation APIs are future
  work; the schema (`revoked_at`, `expires_at`) already supports them.

## Alternatives considered

- **Signed JWT (HMAC):** stateless verification, no per-request DB read. Rejected:
  new dependency (ARCHITECTURE change), harder revocation, and the DB read is cheap
  here. The schema does not preclude adding JWTs later.
- **Keep both header + unsigned token behind one insecure flag:** simplest for dev,
  but leaves two client-asserted shapes accepted and is easy to misconfigure into
  fail-open. Rejected for secure-by-default.
- **Remove all client-asserted paths immediately:** most rigorous, but would force a
  full session-seeding rewrite of the existing integration suite in one step.
  Deferred — the gated header path is retained for tests/CLI.

## References

- Issue #7 (this ADR, server side), #26 (client login UX follow-up), #8 / #21
  (separate auth fail-opens), `user_invitation_tokens` (V020).
- `migrations/V022__phase6_auth_sessions.sql`,
  `crates/seasoned-hand-core/src/auth/session.rs`,
  `crates/seasoned-hand-server/src/auth.rs`.
- [ADR-017](ADR-017-browser-auth-transport-subprotocol-token.md) — transport this
  credential rides on.
