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
other categories clean. Awaiting Codex's independent iter-2 re-audit before
declaring saturation — a single sweep is not a seal (cf. the Phase-5
cross-tenant pass, where the bilateral confirm caught H10).
