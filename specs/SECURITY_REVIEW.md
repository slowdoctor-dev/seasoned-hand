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
