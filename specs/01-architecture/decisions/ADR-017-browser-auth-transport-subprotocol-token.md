# ADR-017: Browser auth transport — query/subprotocol token (not request headers)

Status: Accepted
Date: 2026-06-14
Relates to: [ADR-016](ADR-016-dioxus-unified-rust-frontend.md) (Dioxus frontend),
            [ADR-014](ADR-014-phase5-v013-tenant-rbac.md) (tenant + RBAC + audit)

## Context

Phase 5 put the control plane's identity in an `AuthContext` (`tenant_id`,
`organization_id`, `actor_user_id`, `org_role`) and the HTTP layer derives it from
`x-seasoned-hand-*` **request headers** (`crates/seasoned-hand-server/src/auth.rs`,
`parse_auth_context`). `/ws` and most `/v1/*` routes are wrapped in `with_auth(...)`
(`lib.rs:836,1462`), so the middleware requires those headers and 401s without them.

This is unreachable from a browser:

- **Browser `WebSocket` cannot set arbitrary request headers.** The `WebSocket`
  constructor takes only a URL and a list of **subprotocols** — there is no header
  API. So `/ws` can never receive `x-seasoned-hand-*` from a browser, by design of
  the platform, not by client oversight.
- Both committed clients in fact send **no** auth at all today: Dioxus
  `Request::get(&url).send()` (`crates/seasoned-hand-ui/src/api.rs:11`) and
  `WebSocket::open(&url)` (`crates/seasoned-hand-ui/src/ws.rs:77`); legacy Next
  `fetch()` (`frontend/lib/api.ts:29`) and `new WebSocket(url)`
  (`frontend/lib/ws.ts:133`). The server's WS tests pass only because they use
  `tokio_tungstenite` with explicit headers.

This is tracked as **issue #24** (transport blocker) and is the browser-facing twin
of **issue #7** (identity is client-asserted) and **#21** (middleware fails open on
missing `RouteAction`). ADR-016 explicitly states the `/v1` + WebSocket boundary is
"the contract and does not change" — but that contract was never satisfiable from a
browser, so the contract itself needs an addition.

**Two concerns, deliberately separated:**
1. **Transport** — *how* a browser conveys a credential to REST and to the WS
   upgrade. This ADR.
2. **Credential** — *what* that credential is and how the server verifies it
   (signing, expiry, a login endpoint validating `user_invitation_tokens`). That is
   issue #7 / a dedicated Phase 6 story, **out of scope here**.

## Decision

**Browser auth uses a bearer token carried over two transports, not request
headers:**

- **REST (`/v1/*`)** — `Authorization: Bearer <token>`. (`fetch` and `gloo-net`
  both can set this; only WebSocket cannot.)
- **WebSocket (`/ws`)** — the **`Sec-WebSocket-Protocol`** handshake. The client
  offers two subprotocols: a fixed sentinel `seasoned-hand-auth.v1` **and** the
  token value:

  ```
  new WebSocket(url, ["seasoned-hand-auth.v1", "<token>"])
  ```

  The server echoes back **only** the sentinel `seasoned-hand-auth.v1` (a constant)
  to complete the handshake, and reads the token from the *other* offered
  subprotocol. This is the standard browser-WS bearer pattern (used by, e.g., the
  Kubernetes apiserver); it keeps the token **out of the URL/query string**, so it
  does not land in access logs, `Referer`, or browser history.

**Server changes** (`auth.rs::parse_auth_context`): identity resolves in priority
order — (1) `x-seasoned-hand-*` headers (unchanged; service-to-service + tests),
(2) `Authorization: Bearer <token>`, (3) the non-sentinel `Sec-WebSocket-Protocol`
entry. The token decodes to the same `AuthContext` fields. `ws_upgrade` selects the
sentinel subprotocol so the handshake succeeds.

**Token shape (this slice):** the lowercase-hex encoding of a JSON object
`{tenant_id, organization_id, actor_user_id, org_role}` — an **unsigned identity
assertion** (hex keeps the token within the RFC 6455 subprotocol token charset
without an added `base64`/`hex` crate dependency). It fixes the *transport* and makes the browser UI functional; it is
**no more trusted than the current headers** (still client-asserted — see #7). The
real credential — signed/expiring token minted by a login endpoint that validates
`user_invitation_tokens` — is issue #7 and replaces only the *decode/verify* step,
not the transport chosen here.

We chose subprotocol over the two rejected transports below primarily to keep the
token out of URLs and to avoid requiring deployment infrastructure.

## Consequences

**Positive:**
- The Dioxus (and legacy Next) UI can authenticate to `with_auth` routes and stream
  `/ws` events — unblocks #24 and the Phase 6 cutover.
- Token never appears in the URL/query, so it is not logged by proxies or stored in
  history (the main weakness of the query-string variant).
- The server's existing header path is untouched, so `tokio_tungstenite` tests and
  any service-to-service caller keep working; this is purely additive.
- Drops cleanly into the #7 credential work — only the token decode/verify changes.

**Negative / accepted:**
- Until #7 lands, the token is unsigned and client-asserted (the security posture is
  unchanged from headers; this ADR is transport-only and must not be mistaken for
  authentication). The dev posture remains loopback-bound (`require_loopback`).
- Subprotocol smuggling of a bearer token is a known-but-unusual idiom; documented
  here and in code comments to avoid surprise.
- A token large enough to exceed header/handshake size limits would fail; the
  identity assertion is small, so this is not a practical concern.

**Neutral:**
- Cookie-based auth (Alternative A) could be revisited if/when the UI is served
  same-origin behind the server and CSRF protections are added; this ADR does not
  preclude it.

## Alternatives considered

### Alternative A — HttpOnly cookie session
Server sets an `HttpOnly` cookie at login; same-origin `fetch` and the WS upgrade
both send it automatically. Most browser-native. **Rejected for now:** requires the
UI to be strictly same-origin with the API, pulls in CSRF handling for the REST
surface, and couples the transport to a login/session story that does not exist yet.
Kept as a future option (see Neutral).

### Alternative B — Token in the query string (`/ws?token=…`)
Simplest WS approach. **Rejected:** tokens in URLs leak into server access logs,
proxy logs, `Referer` headers, and browser history — an unacceptable default for a
credential. The subprotocol variant gives the same reach without the leakage.

### Alternative C — Same-origin gateway injects `x-seasoned-hand-*`
A reverse proxy terminates auth and injects the headers; clients stay header-free
and the server contract is literally unchanged. **Rejected as the default:** pushes
the entire auth concern into deployment infrastructure that a self-hoster must stand
up, which conflicts with the "self-hostable, no SaaS dependencies in core" hard
decision. Remains valid for operators who *want* a gateway.

### Alternative D — Keep header-only auth, dev bypass for the browser
Add a loopback/dev identity fallback so the UI works, defer all real transport.
**Rejected:** leaves the browser permanently unable to authenticate in any non-dev
deployment and bakes in a footgun; the subprotocol transport is barely more work and
is production-shaped.

## References

- Issue #24 (browser clients cannot send auth headers), #7 (identity client-asserted
  — the credential layer), #21 (middleware fails open).
- [ADR-016](ADR-016-dioxus-unified-rust-frontend.md) — states the `/v1` + WS boundary
  is the contract; this ADR adds the browser transport that contract was missing.
- `crates/seasoned-hand-server/src/auth.rs`, `…/ws.rs` — implementation surface.
