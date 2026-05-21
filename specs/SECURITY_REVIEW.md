# Security Review Log

Cross-phase security audit trail. Findings are recorded chronologically with
severity (M/L/H), reproduction, fix commit, and saturation notes.

This log lives outside `/specs/phase-N/` because security findings rarely
align to phase boundaries — they touch Phase 0 sandbox plumbing, Phase 2
intake adapters, Phase 3 PII redactor, etc. simultaneously.

---

## Audit cycle — 2026-05-20 (Claude solo)

> Reviewer: Claude solo (Codex on 5-day rate-limit recovery)
> Scope: post-Phase-4-close-out cross-phase security sweep
> Method: grep-based attack-surface map → targeted code reads → exploit-probe
> tests → fix commits → saturation re-sweep

### Attack-surface map probed

| Surface | Verdict |
|---|---|
| `format!`-built SQL queries (injection) | clean (no production hits; one `PRAGMA` in tests is parametrized via `{}` over the table name, harmless) |
| Loopback / auth guards on admin routes | clean (`require_loopback` applied to all admin + WS routes) |
| Shell/process invocation (CLI subcommands) | clean (all use arg-vec mode, not shell-interpreted) |
| Log emissions containing API keys / tokens | clean (no `tracing::*` calls leak secrets) |
| Cargo dep CVEs (chrono, hyper, ring, rustix, time) | clean (all on current versions; rustls posture, no openssl) |
| Email header injection (CRLF in subject/body) | clean — lettre's `allowed_char` predicate rejects bytes 0/10/13 and routes the entire word through `rfc2047::encode` (base64-encoded inside `=?utf-8?b?...?=`), so CRLF cannot break out of header values |
| Regex compilation from external input (ReDoS) | clean — only `AllowList::parse` compiles regex from env config, which is operator-trusted |
| Tool-hook payload size | clean (`write_large_or_inline` spills oversized payloads to workspace file_ref) |
| Body / WS size caps | acceptable (Axum default 2 MB / ~16 MB; operator-local server) |

### F1 (L) — `deliverable::renderer::ensure_dir` joined arbitrary `&str` into host path

**Status**: FIXED at commit `79d4c9e` (security hardening iter-1)

**Threat model**: `ensure_dir(sandbox, session_id, relative)` joined `relative`
directly into `workspace_host_path` without `..`-rejection. Today all call
sites pass hardcoded `.deliverables` / `.deliverables/.source` constants, so
no current exploit — but the function shape invited a regression. A future
caller passing user-supplied `..` would escape the workspace bind-mount
host-side.

**Fix**: route through `sandbox::normalize_workspace_relative_path` (the
same per-component `ParentDir` rejector that `read_workspace_file` /
`write_workspace_file` use). Helper visibility bumped `fn` → `pub(crate) fn`.

**Severity rationale**: L because no live exploit existed; defense-in-depth
adds a safe-primitive guarantee for future callers.

### F2 (M) — PII redactor blind to PEM private-key blocks

**Status**: FIXED at commit `79d4c9e`

**Threat model**: `redact_pii` powers Phase 3 F-3.14 layer-2 redaction applied
to ALL fields of an extracted playbook (title, overview, steps,
trigger_keywords). Before the fix, PEM blocks survived in two ways:
1. BEGIN/END markers preserved verbatim (no pattern caught them).
2. Inner base64 chunks only partially matched `TOKEN_SHAPE_RE` (32+ char
   `[A-Za-z0-9_-]` alphabet — missed `+/=` standard-base64 chars; missed
   short chunks).

A successful task whose log contained a copy-pasted SSH key (e.g. operator
debugging) would have produced a playbook with a partially-redacted
private key embedded in `overview` or a `step`. That playbook would then
be eligible for FTS indexing, future LLM context injection, and Phase 5
multi-tenant exposure.

**Fix**: new `PRIVATE_KEY_PEM_RE` with `(?s)` multi-line semantics:
`-----BEGIN [A-Z0-9 ]*PRIVATE KEY[A-Z0-9 ]*-----.*?-----END [A-Z0-9 ]*PRIVATE KEY[A-Z0-9 ]*-----`
replaces the whole BEGIN..END block with `[REDACTED_PRIVATE_KEY]`. Ordered
first in `redact_pii` so the long base64 inside isn't partially erased by
`TOKEN_SHAPE_RE` before recognition. New `pii_redaction_catches_pem_private_keys`
test asserts OPENSSH / RSA / EC / DSA / ENCRYPTED marker variants.

**Severity rationale**: M because PEM private keys are catastrophic on
leak and the redaction surface is the *exact* boundary Phase 3 F-3.14
designed to defend.

### F3 (L) — IPv6 redactor blind to common short forms

**Status**: FIXED at commit `79d4c9e`

**Threat model**: the existing IPv6 regex required ≥2 `xxxx:` groups
before the `::` elision. Common short forms slipped through:
- `::1` (IPv6 localhost)
- `fe80::1` (link-local)
- `::ffff:192.0.2.1` (IPv4-mapped)

**Fix**: extended-mode alternation catches `::`-prefixed, `xxxx::yyy`,
and `xxxx::ffff:vvv.www.xxx.yyy` forms while preserving the original
full-form match. New `pii_redaction_catches_ipv6_short_forms` test pins
coverage.

**Severity rationale**: L — PII, not a credential, but high-likelihood.

### F4 (M) — sandbox layer didn't re-validate `session_id` at sinks

**Status**: FIXED at commit `f9d2a57` (security hardening iter-2)

**Threat model**: the intake-router already filters `session_id_hint`
against `[A-Za-z0-9-]{1..=64}` (Phase 2 REVIEW §1/G, DEBT #35). But the
sandbox layer trusted the validated string blindly — three sinks joined
it into host paths or Docker container names without re-validating:
- `SandboxClient::create` → `workspace_root.join(session_id)`
- `SandboxClient::register_existing` → `container_name(session_id)`
- `WorkspaceTtlCron::clean_one` fallback (used after cross-process restart
  loses the in-memory handle cache) → `workspace_root.join(&session_id)` +
  `remove_dir_all`

A future caller bypassing intake (CLI subcommand, direct integration test,
external API connector) could land a `..` segment in `sessions.id` and
escape the bind-mount.

**Fix**: canonical `pub fn sandbox::is_safe_session_id` is now the single
source of truth. `intake::router::is_safe_session_id` is a thin wrapper
over it. `require_safe_session_id` is invoked at every sandbox sink and
the TTL cron's fallback join branch — on rejection, the TTL cron logs and
skips (leaks the directory rather than risk an out-of-bounds rmdir).

**Severity rationale**: M — defense-in-depth against a real
host-side-rmdir threat class, even though no current bypass existed.

### Iter-3 — saturation sweep

Probed: tool-hook event payload redaction, regex-from-external-input,
HTTP/WS body size caps, subprocess argv mode, Cargo CVE posture, email
header injection via lettre, Bifrost upstream URL SSRF.

**Findings**: zero new code fixes.

**Observation (not a finding, documented for Phase 5 attention)**: Phase 4
Action events stored in the canonical events table contain raw tool
args + outputs without applying `redact_pii`. For single-user local
deployments this is the operator's audit log and the operator already
has filesystem-level access to secrets. When Phase 5 flips `tenant_id`
to NOT NULL and the events table starts carrying multi-tenant rows,
this becomes a real cross-tenant leak surface. Phase 5 BMAD Architect
should design tenant-scoped event redaction (or accept that operators
can read all tenants on their server — also valid).

### Saturation verdict

Three iterations: iter-1 closed 1 M-severity + 2 L-severity findings;
iter-2 closed 1 M-severity defense-in-depth gap; iter-3 found zero
load-bearing items. Security hardening loop saturates here.

Codex review of this audit trail can land once the 5-day rate-limit
recovers; the loop is closed from Claude's side.

---

## Codex re-audit addendum — 2026-05-20

Performed commit-truth review against `79d4c9e` and `f9d2a57` plus current
`sandbox/mod.rs`.

- `F1` ACK
- `F2` ACK
- `F3` ACK
- `F4` EXPAND

`F4` correctly added defense-in-depth at `create`, `register_existing`, and
TTL fallback cleanup. One residual sink class remained: container-name paths
in `is_paused`, `pause`, `resume`, and `destroy` still accepted unchecked
`session_id`.

**Codex iter-1 finding (L, fixed inline)**  
Added `require_safe_session_id(session_id)?` to those four methods so the
"all sandbox sinks validate session_id" claim now holds.

Evidence:
- code: `crates/seasoned-hand-core/src/sandbox/mod.rs`
- tests: `sandbox::tests::is_safe_session_id_accepts_and_rejects`,
  `sandbox::tests::require_safe_session_id_returns_invalid_workspace`

---

## Audit cycle — 2026-05-21 (Claude + Codex) — dedicated Security track

> Reviewers: Claude (iter-1), Codex (iter-2, independent re-audit — pending
> capacity; Codex currently at-capacity throttle).
> Scope: a dedicated **Security** hardening track, broader than the just-
> completed Phase-5 cross-tenant saturation pass (10 H + 6 M, see
> `specs/phase-5/REVIEW.md`). Categories: request authn / network exposure,
> SQL injection, path traversal, token compare / CSPRNG, secrets-in-logs,
> DoS / resource caps, deserialization panic-safety, crypto, production
> `unwrap`/`panic`, integer overflow.
> Saturation rule: a full Claude+Codex round with zero new H/M findings.

### iter-1 (Claude) — attack-surface map

| Surface | Verdict |
|---|---|
| Request authentication model (`x-seasoned-hand-*` header trust) | **H-1 (mitigated)** — see below |
| `require_loopback` coverage on sensitive handlers | **H-2 (fixed)** — 3 SOP-share handlers were the only gap |
| SQL injection (all crates) | clean — every value bound via `params!` / `params_from_iter` / `ToSql`; no string-interpolated values |
| Path traversal (workspace / sandbox) | clean — `require_safe_session_id` + workspace-root containment; minor Windows-backslash note (loopback-only, non-blocking) |
| Token compare / invitation tokens | clean — `subtle::ConstantTimeEq::ct_eq` for compare, `uuid` v4 (getrandom CSPRNG) for generation, SHA-256-hashed at rest |
| Secrets / PII in logs | clean — no `tracing::*` call emits API keys, tokens, or raw header identity |
| DoS / resource caps | clean — list endpoints `LIMIT`-capped; axum `DefaultBodyLimit` 2 MB on the router |
| Deserialization panic-safety | clean — no `unwrap`/`expect` on `serde` paths in production |
| Crypto | clean — `sha2` + `subtle` + `getrandom`-backed `uuid`; no hand-rolled primitives |
| Production `unwrap`/`panic` | clean — all hits are in `#[cfg(test)]` modules |

### H-1 (HIGH, mitigated) — header-trust authentication model

`crates/seasoned-hand-server/src/auth.rs` middleware derives the full caller
identity (`tenant_id`, `organization_id`, `actor_user_id`, role) from
self-asserted plaintext `x-seasoned-hand-*` request headers, with no
credential binding. If the control plane is bound to a non-loopback address
without a trusted authenticating gateway in front, any client that can reach
the socket can assert `x-seasoned-hand-org-role: admin` for any tenant — a
complete auth bypass.

This is **by design** for the current single-operator, self-hosted posture:
verifiable end-user authentication (API key / OAuth) is the BASELINE §8
Open Decision slated for Phase 6. The header-trust model is the documented
gateway-trust contract.

Mitigation applied this iteration (defense-in-depth, not a replacement for
real authn):
1. Default bind is loopback (`127.0.0.1`); `HOST` env override.
2. `main.rs` now logs a startup `SECURITY:` warning whenever the resolved
   bind address is non-loopback, naming the gateway requirement.
3. Because **H-2 closes the last `require_loopback` gap**, every sensitive
   handler now enforces loopback — so on the default bind the header-trust
   surface is reachable only from the same host.
4. `SECURITY.md` gains a "Request authentication & network exposure" section
   stating the gateway requirement explicitly.

Residual risk is carried forward to Phase 6 as the real-authn work item.

### H-2 (HIGH, fixed) — missing `require_loopback` on SOP-share handlers

`post_sop_share_handler`, `delete_sop_share_handler`, and
`list_sop_shares_handler` (`crates/seasoned-hand-server/src/lib.rs`) were the
only sensitive Phase 5 handlers that did **not** call `require_loopback`.
They were `with_auth`-gated and tenant-scoped, but lacked the loopback
defense-in-depth every other sensitive handler applies — so on a
mis-configured non-loopback bind they were directly reachable.

Fix: added `ConnectInfo` + `require_loopback(remote)?` as the first body line
of all three handlers (comment tag `SEC-IT1-H2`), and added three regression
sweeps to the existing `assert_handler_refuses_non_loopback` battery
(`{post,delete}_sop_share_refuses_non_loopback_remote`,
`list_sop_shares_refuses_non_loopback_remote`).

### Low findings (logged, not blocking)

- `delta_pct` cost-drift math does `(expected - observed).abs()` on `i64`;
  theoretically overflowable but the inputs are internally-generated cost
  cents, never request-controlled. Not fixed.
- WS outbound channel is unbounded; loopback-only and bounded in practice by
  the agent loop cadence. Not fixed.

### iter-1 verdict

2 HIGH (H-1 mitigated + carried to Phase 6; H-2 fixed), 2 Low logged. All
other categories clean. iter-1 was server-Rust-handler-centric, so it does
not seal the loop — a deeper re-sweep on the lightly-covered surfaces
follows as iter-2.

### iter-2 (Claude) — deep re-sweep of lightly-covered surfaces

iter-1 concentrated on the Rust control-plane HTTP handlers. iter-2 audited
the surfaces it only skimmed: the Next.js frontend, the channel **intake**
adapters (untrusted *inbound* payloads), the CLI, and sandbox / tool
dispatch. (Codex remained at-capacity throttle, so this is a second
independent Claude pass per the rate-limit-handoff workflow, not the
bilateral seal.)

| Surface | Verdict |
|---|---|
| Frontend (Next.js 16 / React 19) | clean — no `dangerouslySetInnerHTML`/`eval`; no API routes or server actions; `NEXT_PUBLIC_*` hold only non-secret base URLs; `next.config.ts` empty; workspace/screenshot URLs are `encodeURIComponent`-escaped; the one `iframe` is the operator-trusted sandbox `novnc_url` with a `sandbox=` attribute |
| Email intake parse (`channel/email/`) | clean — `parse_mail` errors map to `Decode` (no panic), header access is `.get_first_value()`/`unwrap_or_default()`, per-message failures isolated, default-deny allow-list + subject-prefix gate |
| CLI (`seasoned-hand-cli`) | clean — all `Command::new` sites use arg-vec mode (editor spawn, server exec, `xdg-open`/`open`); no `sh -c`/`bash -c`, no `format!`-built command strings |
| Sandbox / tool dispatch | clean — Docker via bollard typed API (no shell); `require_safe_session_id` confirmed on every host-path/container-name sink (`create`/`is_paused`/`pause`/`resume`/`register_existing`/`destroy`); `normalize_workspace_relative_path` rejects null bytes + `..` via `Path::components()` |
| Webhook **delivery** client redirect handling | **M-1 (fixed)** — see below |

### M-1 (MEDIUM, fixed) — SSRF redirect bypass on the webhook delivery client

`WebhookChannel::post_json` (`crates/seasoned-hand-core/src/channel/webhook/mod.rs`)
runs the SSRF guard `ssrf::assert_public_address` against the **initial**
target URL only, then issues the request with the default reqwest client.
reqwest 0.12 follows up to 10 redirects by default, so a delivery/notify
target that passes the guard (a public host, or an allow-listed one) and then
responds `30x Location: http://169.254.169.254/...` (cloud metadata) or
`http://127.0.0.1/...` would have the client follow the hop to the
**unvalidated** internal address — a metadata-endpoint / internal-service
SSRF. The delivery `target_ref` is reachable from the inbound intake path
(`POST /v1/intake/webhook`), and Phase 5 DEBT #1 already flags per-request
URL trust; the redirect bypass is a distinct, currently-live hole on top of
that.

Fix: `with_default_client` now sets `.redirect(reqwest::redirect::Policy::none())`
(comment tag `SEC-IT2-M1`). A 3xx is returned to `post_json` unfollowed and
surfaces as `RemoteRejected { status: 3xx }`, so the only address ever
fetched is the one the guard validated. Regression test
`webhook::tests::webhook_delivery_does_not_follow_redirects` mounts a
`.expect(1)` 302 entry point + an `.expect(0)` redirect target and asserts
the target is never hit.

### iter-2 verdict

1 MEDIUM (M-1, fixed). All other surfaces clean. Two independent Claude
passes (iter-1, iter-2) now stand. Saturation still requires a clean Claude
pass plus Codex's independent confirm — the bilateral seal (cf. Phase-5
cross-tenant pass, where it caught H10).

### iter-3 (Claude) — escalation / secrets / billing / concurrency — CLEAN

iter-3 audited four angles deliberately distinct from iter-1/iter-2 and from
the exhausted cross-tenant pass, assuming an *honest gateway* but a caller who
tries to escalate or evade *within* a trusted identity.

| Angle | Verdict |
|---|---|
| RBAC privilege escalation | clean — role parsing fails closed (unknown/blank role → 401); `(Role::Admin, _)` bypass is reached only after a non-empty `tenant_id` check, so admin is per-tenant; `invite_user` is `MembershipManage`-gated and only inserts when no membership exists, so re-invite can't bump a role; the role/override *write* paths are internal/CLI, not self-service routes |
| Secret-at-rest / in-response | clean — invitation tokens are sha256 `token_hash` PRIMARY KEY (V020), plaintext disclosed exactly once in `InviteOutcome.login_token`; intake/admin/webhook tokens live as `Arc<String>` on a non-`Serialize` `AppState` and are never logged; all token compares use `subtle::ConstantTimeEq` |
| Cost / billing integrity | clean — the cost cap is a per-task in-loop guard recomputed from recorded step cost (no shared cross-task tenant quota exists to race); ledger flush is idempotent recompute-from-source UPSERT keyed by `(tenant,user,month)`; negative/overflow costs surface as drift findings, not corruption; reconcile output is tenant-scoped (iter-1 `.retain`) and the cron sibling keys audit events per `finding.tenant_id` |
| TOCTOU / optimistic concurrency | clean — `DbPool` is `Arc<Mutex<Connection>>`; every `expected_updated_at` read-check-write runs inside a single `with_conn` closure (mutex held throughout) so check+mutate is atomic; handoff additionally uses an explicit `conn.transaction()`. The future multi-connection-pool risk is already documented as a hard paydown prerequisite in `sharing/concurrency.rs` |

**Observation carried forward (NOT a live finding):** `effective_role() =
project_override_role.unwrap_or(org_role)` does not clamp the override to be
≤ the org role. Not exploitable today (override arrives only via the trusted
gateway header; persistence paths are internal/CLI), but it becomes a real
escalation primitive once Phase 6 adds self-service auth/role management.
Logged as DEBT `#S-2` (Phase 6 owner).

### iter-3 verdict

0 new H/M/L across these four angles. iter-1/iter-2 fixes hold. But iter-3 was
not the seal — iter-4 (below) probed yet-different surfaces and found a real
isolation-breaking bug, which is exactly why a single clean pass is never the
seal (cf. Phase-5 cross-tenant pass, where the bilateral confirm caught H10).

### iter-4 (Claude) — WS internals / untrusted parsing / overflow / DoS / TLS / sandbox FS

iter-4 probed the surfaces iter-1..3 skimmed: WebSocket frame handling,
deserialization of sandbox/tool *outputs*, integer overflow on external
values, DoS caps, TLS/cert verification on every outbound client, and the
sandbox↔host filesystem boundary.

| Angle | Verdict |
|---|---|
| WebSocket internals | clean — every command arm is tenant-guarded; malformed JSON → `bad_envelope` Error (no close, no panic); no indexing/`unwrap` on frame content. The unbounded outbound `mpsc` + uncapped inbound text frame are loopback-only (local DoS, out of scope under the honest-gateway model) |
| Untrusted deserialization / panics | clean — sandbox/tool/bootstrap response parsing maps errors; `tools::builtin` uses `.unwrap_or(Value::Null)`; bootstrap `truncate` is char-boundary-safe via `.chars().take()`; all `unwrap`/`expect`/`panic!` in sandbox/tools/agent/playbook are test-only |
| Integer / arithmetic overflow on external values | clean — the only `as`/arithmetic on the agent path runs on internally-capped i64 cost + monotonic event ids; no payload/sandbox-controlled size/count feeds unchecked arithmetic |
| DoS / resource caps | clean(-ish) — HTTP routes inherit axum's 2 MB body limit via `Json`/`Bytes`; workspace file serving capped at 1 MB (`WORKSPACE_FILE_CAP_BYTES`); the only uncapped reads (WS frame, bootstrap response) are loopback/operator-bounded |
| TLS / cert verification | clean — every reqwest client is `rustls-tls` with default-features off; no `danger_accept_invalid_certs`/`_hostnames`/custom verifier anywhere; IMAP uses matching rustls; the sandbox API is `http://127.0.0.1:<port>` (loopback, no secrets) |
| Sandbox ↔ host filesystem boundary | **M-2 (fixed)** — see below |

### M-2 (MEDIUM, fixed) — symlink escape from sandbox to arbitrary host file read/write

The control plane reads and writes workspace files on the **host** side of the
sandbox bind mount: `SandboxClient::read_workspace_file` /
`write_workspace_file` (`crates/seasoned-hand-core/src/sandbox/mod.rs`) and the
HTTP workspace proxy `workspace_proxy_inner`
(`crates/seasoned-hand-server/src/lib.rs`). Both join a relative path onto the
workspace root and then call symlink-following ops (`tokio::fs::read` /
`metadata` / `read_dir` / `write`). The only guard,
`normalize_workspace_relative_path`, rejects `..` and null bytes in the
*request path* — it does **not** inspect on-disk symlinks.

Untrusted agent-generated code runs inside the sandbox with write access to
the bind-mounted workspace. It can plant `ln -s /etc/passwd /workspace/leak`
(or `-> ../../host-secret`), after which:
- the owning tenant reading `GET /v1/workspace/<sid>/leak` (loopback + RBAC +
  tenant-scoped, but that's the legitimate owner) gets an **arbitrary host
  file**; and
- any control-plane `write_workspace_file` whose relative path the sandbox
  pre-symlinked becomes an **arbitrary host file write**.

This escalates confined-sandbox file access into host-file read/write,
breaking the ADR-004 isolation boundary.

Fix (comment tag `SEC-IT4-M2`): both read sinks now resolve the real path with
`tokio::fs::canonicalize` and require it to `starts_with` the canonicalized
workspace root (`canonical_within_workspace`); the write sink refuses to write
through an existing symlink and requires the resolved parent to stay inside
the root (`reject_workspace_write_escape`); the HTTP proxy canonicalizes the
target and checks containment before any FS access. Regression tests
`sandbox::tests::read_workspace_file_rejects_symlink_escape` and
`write_workspace_file_refuses_to_write_through_symlink` plant a real symlink to
an out-of-workspace secret and assert it is rejected (and the host file is left
untouched), while legitimate in-workspace files still read/write.

### iter-4 verdict

1 MEDIUM (M-2, fixed). All other angles clean. Because iter-4 found a real
bug, the "clean pass" counter resets — saturation now requires a FRESH clean
Claude pass (iter-5) **and** Codex's independent confirm, both returning zero
new H/M. Codex remains at-capacity throttled (dispatch queued in its input).
Track status: **not sealed** — iterating.

### Security-track iter-4 (Codex, 2026-05-21) — independent confirm pass

Independent re-audit focus (angles under-covered in iter-1..3): WebSocket command
surface beyond pause/resume/cancel, request-body parsing failure paths, panic/unwrap
reachability from untrusted payloads, resource caps around long-lived command channels,
TLS verification posture on outbound HTTP, and sandbox file-boundary hardening.

#### Grade of prior fixes

- **H-2 (`require_loopback` on SOP-share handlers)**: **ACK**. Verified all three handlers
  (`list/post/delete /v1/sops/:id/shares`) call `require_loopback(remote)?` before work,
  and regression guard tests exist in server test suite.
- **M-1 (webhook redirect SSRF bypass)**: **ACK**. Verified webhook channel client uses
  `reqwest::redirect::Policy::none()` and test
  `webhook_delivery_does_not_follow_redirects` proves non-follow behavior.

#### Reported "new finding" H-3 — RETRACTED (false positive)

> **Claude correction (commit-truth verification).** Codex's iter-4 pass reported
> H-3: "WS `briefing_confirm` lacked a tenant/task scope check" and claimed to fix
> it (export `require_task_tenant`, add a guard in `ws.rs`, add test
> `ws_tenant_a_cannot_confirm_tenant_b_briefing`). **None of this is a new finding
> or a new change.** That exact guard and that exact test already exist in
> committed code from the cross-tenant pass's H10 fix (`1d1b63c`):
> - guard: `crates/seasoned-hand-server/src/ws.rs:427` —
>   `require_task_tenant(state, &task_id, auth_ctx)` returns `forbidden_task_scope`
>   on the `BriefingConfirm` arm before `forward_briefing_confirm`;
> - test: `crates/seasoned-hand-server/tests/ws.rs:615`
>   `ws_tenant_a_cannot_confirm_tenant_b_briefing` (tenant-a confirming tenant-b's
>   briefing → `forbidden_task_scope`).
>
> `git diff` confirms Codex modified **neither** `ws.rs` nor `tests/ws.rs` — its
> claimed edits were no-ops against already-present code. H-3 is therefore a
> **re-discovery of already-mitigated H10, not a new vulnerability**. Disposition:
> **no code change; already fixed**. (Lesson reinforced: verify an agent's claimed
> edits against `git diff` before recording them — cf. the iter-9 premature-
> saturation lesson in `specs/phase-5/REVIEW.md`.)

#### Non-findings (iter-4 sweep)

- No new H/M in webhook/email intake parser paths, sandbox bootstrap HTTP/TLS handling, or
  panic/unwrap exposure from network payload decoding.
- Existing M-2 symlink-escape fix holds (workspace canonicalization + write-symlink refusal).

#### iter-4 (Codex) verdict — corrected

Codex correctly **ACK'd** the H-2, M-1, and M-2 fixes (verified against the tree).
Its sole "new finding" H-3 is **retracted as a false positive** (already-mitigated
H10; see correction above) — so Codex's independent pass found **0 real new H/M**.

The only real new finding this round was **M-2 (Claude iter-4, sandbox symlink
escape)**, now fixed. Because a real new finding landed this round, the track is
**not yet saturated**: per the bilateral rule we need one more round where BOTH
Claude and Codex find 0 new H/M. Next: iter-5 — Claude solo re-sweep + Codex
independent confirm, each verifying any claimed finding against committed code
before recording it.

### iter-5 (Codex) — independent confirm pass — CLEAN

Independent confirm pass against committed tree `466c75f`, with "diff-truth
first" discipline: every claim verified against committed code before recording.

- **Prior fixes graded**: H-2 **ACK** (loopback gate on all three SOP-share
  handlers + non-loopback regression tests green); M-1 **ACK**
  (`redirect::Policy::none()` + `webhook_delivery_does_not_follow_redirects`
  present); M-2 **ACK** (read-path canonicalization/containment + write-path
  symlink refusal in `sandbox/mod.rs` + regression tests present).
- **Re-audit angles, all CLEAN**: WS command arms (`Subscribe`,
  `TaskPause/Resume/Cancel`, `UserResponse`, `BriefingConfirm`) all enforce
  tenant/session/task scope before mutate/read; untrusted payload parsing
  returns structured errors with no externally-reachable panic/unwrap; DoS cap
  posture intact; outbound TLS has no insecure-cert overrides and webhook stays
  non-following; sandbox file boundary holds.
- **New H/M: 0. Verdict: concur saturation** (pending the matching clean Claude
  iter-5 pass below).

### iter-5 (Claude) — SQL/FTS5 / git-command / replay / audit-log integrity

Re-audited the angles still thin after iter-1..4. **0 H, 0 M, 1 LOW.**

| Angle | Verdict |
|---|---|
| SQL / FTS5 query construction (all stores) | clean except L-1 below — all store SQL is parameterized (no `format!`-built SQL; dynamic `push_str` only appends static fragments with bound `?`); audit/session-search/visibility/events all bound; curator FTS `MATCH` sites are `#[cfg(test)]`-only |
| git / checkpoint / sandbox command construction | clean — `git commit -F <file>` (phase title written to a workspace file, never the shell); `git revert --no-commit <sha>` gated by `is_valid_git_sha` (hex, len 40/64) and the tool is masked from every LLM mode; bootstrap git commands are constant strings; no task value reaches a shell-interpreted string |
| replay / checkpoint / artifact deserialization | clean — stored JSON read via `serde_json::from_value` inside `if let Ok` + `.get().and_then(as_str)`, no panic on malformed data; workspace write paths are constants; the one non-test `unwrap` in `checkpoint/persistence.rs` is provably `Some` (guarded) |
| audit-log integrity | clean — `tenant_id`/`organization_id`/`actor_user_id` come solely from the trusted `AuthContext`; the record body cannot override identity columns; INSERT + query filters fully parameterized; query forces `tenant_id = ?` and clamps a User-role caller to its own `actor_user_id` |

#### L-1 (LOW, fixed) — `playbook_search` FTS5 query not metachar-safe

`matcher::production_match` (`crates/seasoned-hand-core/src/matcher/mod.rs`) built
the FTS5 `MATCH` expression by appending `*` to each whitespace token of a brief,
keeping FTS5 syntax characters (`"`, `:`, `^`, `*`, `(`, `-`). The value is *bound*
(no SQL/FTS injection), but a brief containing those characters — ordinary in real
briefs like `fix the "login" bug:`, and reachable via the LLM `playbook_search`
tool (which is not masked) — produced a malformed expression and a SQLite error,
surfaced as a handled `ToolError::Backend`. No panic, no 500, no cross-tenant
impact: a self-inflicted, recoverable tool error that also degraded the matcher
for benign quoted/colonized briefs.

Fix (`SEC-IT5-L1`): `sanitize_fts_token` strips each token to alphanumerics before
the `*` suffix, so the expression is always well-formed and matching degrades
gracefully. Regression test `matcher::tests::production_match_tolerates_fts5_metacharacters`
feeds quotes/`NEAR`/`^`/unbalanced parens and asserts `Ok`, not an FTS5 error.

**Not changed (verified, by design):** `events::search_session_events` also binds a
raw query to `MATCH`, but it is reached only from the operator **CLI**
(`seasoned-hand-cli session-search`), where FTS5 operators (phrase quotes, `NEAR`)
are an intended feature and erroring on malformed operator-typed syntax is correct —
sanitizing there would remove a capability, not close a hole.

#### iter-5 (Claude) verdict

0 H, 0 M; 1 LOW found and fixed (L-1). Combined with Codex's clean iter-5
confirm, **this round is bilaterally clean on H/M** with the sole LOW resolved.

## Security track — SATURATION SEALED (2026-05-21)

The dedicated Security hardening track is **saturated**. Tally across the
Claude + Codex bilateral loop:

- **Fixed**: H-2 (SOP-share loopback), M-1 (webhook redirect SSRF), M-2 (sandbox
  symlink escape), L-1 (FTS5 metachar query). H-1 (header-trust auth) mitigated
  (loopback default + warning + docs) and carried to Phase 6 as the real-authn
  decision; project-override clamp logged as DEBT #S-2.
- **Retracted**: 1 Codex false positive (H-3, already-fixed H10).
- **Seal**: iter-5 is bilaterally clean — Claude (0 H / 0 M / 1 L-fixed) **and**
  Codex (0 new H/M, concur) in the same round, each verifying claims against the
  committed tree. No new H/M from either party with all prior findings resolved.

Carry-forward to Phase 6: H-1 real end-user authentication (API key / OAuth,
BASELINE §8) and DEBT #S-2 (clamp `effective_role()` project override ≤ org role
once self-service auth lands).
