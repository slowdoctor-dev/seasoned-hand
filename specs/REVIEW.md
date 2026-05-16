# Cross-Phase Pre-Phase-3 Hardening Review

> **Status**: pre-Phase-3 audit (2026-05-16)
> **Scope**: whole codebase — Phase 0/1/2 complete (closed 2026-05-12,
> 2026-05-13, 2026-05-15); Phase 2 close-out hardening pass
> (`fc75ae4` + `0714dbf` + `cbb7b77` + `709f404` + `ce2bfca`) already
> shipped.
> **HEAD**: `ce2bfca` (branch `main`)
> **Method**: 5 parallel sub-agent audits (security / simplicity /
> stickiness arch+ws+http / stickiness methodology+docs / readability),
> spot-verified inline (Agent E's claim that the frontend timestamp
> drift is still open was checked and **rejected** — DEBT #33 is in
> fact closed; the code already divides μs by 1000 with WHY comments
> citing DEBT #33).
>
> This file lives at `/specs/REVIEW.md` (NOT under any phase-N subdir)
> because the findings span phases. New DEBT entries are numbered from
> **#48** to continue the sequence from `/specs/phase-2/REVIEW.md`,
> which reached #47.

---

## Executive summary

The codebase enters Phase 3 in healthy shape. The Phase 2 review's
H-severity findings (path-traversal via webhook `session_id_hint` and
LLM `target_filename` shell-inject) are closed. The cross-phase pass
surfaces **one new M-severity security finding** (`/v1/workspace/:session_id/*`
not loopback-gated — same shape as the closed DEBT #34), a small
cluster of **doc-staleness drifts** (`AGENTS.md` §13/§14 + `README.md`
still announce "Phase -1 → Phase 0 starting"), and a longer tail of
**low-severity simplification and readability candidates** carried
forward from Phase 0/1 that the Phase 2 review intentionally didn't
audit.

Headline findings:

1. **M — Workspace HTTP proxy lacks loopback gate** (Phase 0 / story
   0.17 era surface — not touched by Phase 2 review). `GET /v1/workspace/:session_id`
   and `GET /v1/workspace/:session_id/*sub_path` at
   `crates/seasoned-hand-server/src/lib.rs:1042-1054` serve sandbox
   workspace contents (including deliverables, prompts, intermediate
   code) without `require_loopback(remote)?`. The handler does block
   `..` segments at `lib.rs:1064` but is reachable from any caller on
   `HOST=0.0.0.0` binds who can guess (or scrape from `GET /v1/sessions`,
   which is also not loopback-gated) a session UUID. Same shape as the
   closed DEBT #34 provenance route fix; identical one-line remedy.

2. **M — Top-of-repo phase status is stale across three docs**.
   `AGENTS.md:185` ("Phase: -1 (planning) → Phase 0 starting"),
   `AGENTS.md:198` ("ADR-001 to ADR-008" — ADR-010 exists),
   `README.md:24` ("Phase -1 — Planning complete. Phase 0 (foundation)
   starting"), and `README.md:32` ("Quick start (not yet — Phase 0 in
   progress)") were not updated after Phase 0/1/2 closed. These are
   the first files a fresh AI session reads per AGENTS.md §5; the
   project's "context rot" defense from BASELINE §12 cannot work if the
   load-bearing entry-point files lie about the current phase. Note
   that AGENTS.md is in the AGENTS.md §9 NEVER list — these touch-ups
   need explicit human approval before applying, even though they're
   doc-only.

3. **M — ARCHITECTURE.md §2.2 sessions states are 5; code+schema now have 6.**
   `migrations/V004__verifications.sql:25-47` correctly widens the
   `sessions.state` CHECK constraint via table-recreate to include
   `VERIFYING`; production code writes it at `agent/mod.rs:614`. The
   `/specs/01-architecture/ARCHITECTURE.md` v1.0 text at lines 165 +
   `state TEXT NOT NULL — IDLE, RUNNING, FINISHED, ERROR, SUSPENDED`
   still lists 5. This is a known subset of Phase 1 DEBT #1/#2 (general
   ARCHITECTURE.md text drift); calling it out here so the eventual
   ADR-011 + v1.1 bump covers it explicitly along with `task_count`
   and Next.js version. **Do not edit ARCHITECTURE.md unilaterally** —
   AGENTS.md §9 NEVER.

4. **M — `TaskStatus` Phase 2 state machine has no entry in ARCHITECTURE.md.**
   `crates/seasoned-hand-cli/src/format.rs:21-29` reflects 8 task
   states (`Drafted`, `Briefed`, `Confirmed`, `Running`, `Paused`,
   `Completed`, `Failed`, `Cancelled`); the immutable architecture doc
   only describes `sessions.state`. The Phase 2 architecture doc
   `/specs/phase-2/architecture.md` describes this in detail, but a
   future reader who only loads the v1.0 immutable spec sees a system
   that's silent on tasks-vs-sessions. Again Phase 1 DEBT #1/#2
   umbrella.

5. **M — GLOSSARY.md is missing 5-7 load-bearing Phase 2 terms**.
   `ChannelRegistration`, `IntakeRouter`, `DeliveryRouter`,
   `NotifyWorker`, `WorkspaceTtlCron`, `Provenance Manifest`, and
   the `Brief` (data shape) vs `Briefing` (event/gate) distinction
   are each referenced 5-15× in specs + code but absent from
   `/GLOSSARY.md`. PRINCIPLES #13 ("Build for the next session") and
   AGENTS.md §11.1 both lean on the glossary as the
   intergenerational-memory layer; the gap costs Phase 3.

6. **M — Module organization debt continues to grow**. Phase 2 review
   filed task_deliver.rs (1082 L), notify/worker.rs (621 L),
   channel/email/mod.rs (621 L) as split candidates. Cross-phase
   sweep adds `crates/seasoned-hand-server/src/lib.rs` at **2879
   lines** — the largest prod file in the repo and the inbound HTTP
   surface for ~40 routes. Splitting handlers into per-resource
   modules (`lib/{tasks,projects,channels,admin,workspace}.rs`)
   would let Phase 3 add new routes without the file growing
   unboundedly.

7. **Quantitatively**: 17 new DEBT-worthy findings (1 M security,
   3 M doc/spec, 1 M GLOSSARY, 1 M file-split, 11 L cosmetic /
   simplicity / WHY-comment). No new **H**-severity findings emerged —
   the Phase 2 review's two H items were the only ones extant; both
   closed.

8. **Architecturally healthy** (re-affirmed cross-phase): SQL
   parameterization is uniform across all 11 store crates audited;
   verifier worker XREADGROUP + per-session FIFO + XACK-on-malformed
   is correctly implemented (`verifier/worker.rs:551-595`); zero
   TODO/FIXME/XXX/HACK across all crates + frontend + migrations;
   acronym casing is consistent (`Sqlite*` not `SQLite*`, `Ws*` not
   `WS*`); test naming discipline is exceptional (zero `test_*` /
   `_works` / `_basic` anti-patterns in any phase); module-level
   `//!` doc-blocks cite specs in 27 of 28 modules; bilingual policy
   (PRINCIPLES #14) honored — code is English, Korean docs first-class.

Proposed new DEBT entries: **#48** workspace HTTP proxy loopback,
**#49** doc-staleness sweep (AGENTS.md + README.md phase status),
**#50** GLOSSARY Phase 2 terms, **#51** ARCHITECTURE.md v1.0 text
drift consolidation (subsumes Phase 1 DEBT #1/#2), **#52**
`crates/seasoned-hand-server/src/lib.rs` 2879-line split, **#53**
`plan/mod.rs` missing module doc-block, **#54** `SimplifyLlm` trait
collapse, **#55** `ToolMaskPolicy` collapse (or data-driven), **#56**
WHAT-comments in `tools/builtin.rs` section dividers, **#57**
WHY-comments missing on agent constants, **#58** `pub` shrinkage
audit for Phase 0/1 surfaces, **#59** `/v1/sessions` not loopback-gated,
**#60** Phase 1 large-file follow-up split set, **#61** event-types
reserved-but-unwired note, **#62** spec-check.sh phase-version gate,
**#63** frontend `pnpm test` is a passing stub, **#64** `tenant_id:
None` Phase 5 conversion meta-DEBT.

Items #48 + #49 + #59 should land in a single "pre-Phase-3 hardening
pass" commit; the rest are independent and could be batched as Phase 3
warm-up.

---

## (1) Security findings

The whole-codebase pass agrees with the Phase 2 review's catalogue:
SSRF posture is comprehensive, SQL is uniformly parameterized,
constant-time compares are in place for tokens, the verifier worker's
PEL handling is correct, the checkpoint shell-injection fix from
DEBT #14 is solid file-based, the LLM filename allowlist from
DEBT #36 reaches every Pandoc / python-pptx / openpyxl call site,
and `normalize_workspace_relative_path` rejects `..` after DEBT #38.

**One new M-severity finding** plus a small set of belt-and-suspenders
notes follow.

### Section A — Loopback gate gaps (Phase 0/1 surfaces)

- **[M]** **`/v1/workspace/:session_id` + `/v1/workspace/:session_id/*sub_path` are not loopback-gated.**
  `crates/seasoned-hand-server/src/lib.rs:1042-1054, 1234-1236`. The
  handler does block `..` segments at `lib.rs:1064`, but the route is
  reachable from any caller on `HOST=0.0.0.0` binds. Workspace
  contents include user prompts, intermediate code, deliverables, and
  any file the agent created. Session UUIDs are discoverable via
  `GET /v1/sessions` (also not loopback-gated — see below). This is
  the same shape as the closed DEBT #34 provenance route fix; remedy
  is one `require_loopback(remote)?` per handler + the `ConnectInfo`
  extractor pattern already used at `:1628`, `:1798`, etc. **New DEBT #48.**

- **[M]** **`GET /v1/sessions`, `GET /v1/sessions/:id`, `GET /v1/sessions/:id/events`,
  `GET /v1/sessions/:id/feature-list`, `GET /v1/sessions/:id/progress`
  are not loopback-gated**. `crates/seasoned-hand-server/src/lib.rs:1229-1233`.
  Each leaks Phase 0/1 era session inventory + event payloads (which
  carry tool inputs / outputs, potentially including LLM prompts,
  shell command stdout, file content excerpts) on non-loopback binds.
  Session listing is the index that makes the workspace-proxy gap
  exploitable without prior session-UUID knowledge. **New DEBT #59.**

- **[L]** ✅ `/v1/cost` (`lib.rs:1228`) discloses only aggregate
  cost_cents — single integer, no PII. Acceptable to leave un-gated;
  belt-and-suspenders: gate it in Phase 5 when cost data is
  per-tenant.

- **[L]** ✅ `/healthz`, `/ws`, `/v1/verifications/:id`,
  `/v1/tasks/:id/provenance` (after DEBT #34 close) are correctly
  gated where required and intentionally open where required.

### Section B — Verifier worker (Phase 1)

- **[L]** ✅ XREADGROUP consumer loop is correct: malformed payloads
  are XACKed (preventing PEL retention of un-parseable garbage),
  terminal handler errors XACK after emitting `verifier_verdict_error`
  Misc, watchdog timeouts XACK + return Ok(None) to caller. The only
  PEL-retention path is a crash strictly between consume and XACK —
  next consumer (or restart) picks up via `triggered_at_event_id`
  dedupe. `crates/seasoned-hand-core/src/verifier/worker.rs:551-624`.
  Phase 1 DEBT #15 close is solid.

- **[L]** Phase 1 DEBT #12 (verifier 5xx fail-closed by default) is
  unchanged. Verified default = `false`, env override `verifier.fail_open=true`
  exists. Documented + intentional. No change.

### Section C — Plan Manager (Phase 1)

- **[L]** `plan_create` / `plan_update` accept arbitrary phase
  payloads. `crates/seasoned-hand-core/src/plan/mod.rs:95-96, 161-162`
  rejects empty phases but does not cap maximum count, recursive depth
  (phases are flat — no nesting in the schema, so depth is fixed at
  1), or per-title length. An LLM that paginates `plan_update` with a
  100_000-phase array would inflate `plans.phases` column to MB scale
  and slow `sticky_render` linearly. Single-operator + LLM trust
  bound makes this theoretical; Phase 5 multi-user should add a cap.
  Belt-and-suspenders — no new DEBT, but worth a comment.

- **[L]** `plan/render.rs:1-23` `sticky_render` uses
  `token_cap / phases.len().max(1) * 3` as initial per-title budget
  without a comment explaining the `* 3` constant (presumably a
  chars-per-token heuristic). One-line WHY would help future readers.
  Bundle under #57.

### Section D — Browser tracks (Phase 1)

- **[L]** Per Agent A: Track C screenshot filenames are
  `call_id` (server-generated UUID), not LLM-controlled — no
  path-traversal vector. Phase 1 DEBT #8 (no per-track retention)
  remains open as a disk-fill concern at scale; Phase 2 DEBT #16
  (workspace TTL) closes it indirectly via per-status TTLs.

### Section E — Cost client (Phase 0)

- **[L]** Phase 0 DEBT #26 (single-session cost attribution) remains
  open and load-bearing — single-operator deployment is the threat
  bound. No new finding.

### Section F — Hook output truncation (Phase 0+1)

- **[L]** ✅ `events/truncation.rs:30-50` writes `<event_id>.<ext>`
  where event_id is server-allocated `i64` and ext is content-type
  mapped via static `extension_for(...)`. SHA256 verified. No
  user-controlled component reaches the filename. Clean.

### Section G — LLM-facing tool path arguments

- **[L]** ✅ Every `file_*` tool routes through the sandbox HTTP API
  (`/v1/file/{read,write,...}`); the AIO Sandbox endpoint runs its
  own normalization, and the host-side `task_deliver` path (which
  bypasses the sandbox HTTP API and writes directly via
  `workspace_host_path.join(...)`) goes through the DEBT #36
  filename allowlist + DEBT #38 `..` reject. Phase 2 hardening
  pass 3 was thorough on this surface.

- **[L]** ✅ `shell_exec` is always called via sandbox HTTP API
  (no host-side shell). `browser_console_exec` JS runs inside the
  sandbox's Chromium; XSS-into-our-own-DOM is bounded by the
  CSP-isolated noVNC iframe (verified in `frontend/components/agent-computer/*`).

- **[L]** ✅ `info_search_web` uses `reqwest` with `BRAVE_API_KEY` /
  `TAVILY_API_KEY` set as header, not query param — no env-var
  smuggling vector.

### Section H — Secrets handling cross-phase

- **[L]** ✅ Sweep of `std::env::var(...)` across all crates: no
  call result flows into `tracing` fields, into LLM context, or
  into events. `BIFROST_MASTER_KEY` (Phase 0 DEBT #8) is still
  read at boot but never sent on outbound Bifrost calls (Phase 0
  localhost-only binds). Phase 5 will wire enforcement.

- **[L]** `ImapConfig` + `SmtpConfig` still derive `Debug` (Phase 2
  REVIEW Section E observation, not promoted to DEBT). A defense
  before Phase 5 multi-user is the `Secret<String>` newtype; no
  current `format!("{:?}", config)` site exists.

### Section I — Egress allowlist (Phase 1 DEBT #6)

- **[L]** Default remains `["*"]` (permissive). The flag itself is
  carried in `SandboxConfig` but no production path consults it for
  outbound HTTP from `info_search_web` / browser. Known open;
  Phase 5 territory.

### Section J — Database-tamper / tenant_id

- **[L]** ✅ Zero string-interpolated SQL in Phase 0/1 stores —
  `events/sqlite.rs`, `db/mod.rs`, `verifier/persistence.rs`,
  `checkpoint/persistence.rs`. ORDER BY values are constants; LIMIT
  values are placeholder-bound.

- **[L]** **`tenant_id: None` is hardcoded at 100% of production
  construction sites** (Agent B sweep). The field is forward-compat
  for Phase 5 NOT-NULL flip; documenting that the Phase 5 migration
  must do all of (struct types + construction sites + DB load-paths
  + auth-layer fill-in) in a single atomic commit. **New DEBT #64.**

### Section K — Sandbox seccomp / bind-mount

- **[L]** Phase 0 DEBT #15 (`seccomp=unconfined`) unchanged. Bind-mount
  source `workspace_root.join(session_id)` is now safe because
  `is_safe_session_id` (DEBT #35 close) gates `session_id` at intake
  ingress — bind-mount input is `^[A-Za-z0-9-]+$`. Solid.

### Section L — Server bind / loopback (cross-cutting)

See Section A — two new gaps on workspace + sessions GET endpoints
elevate this from a tidy "all gated" state to "Phase 0/1 GET surface
needs a sweep before Phase 5 multi-user". A consolidated
`require_loopback` audit at lib.rs:1226+ where the router is built
is the right shape for the fix.

---

## (2) Simplicity / anti-overengineering findings

### Trait surfaces (continuing where Phase 2 REVIEW left off)

- **KEEP — `Hook`**. 5 prod impls (`NoopHook`, `EventEmittingHook`,
  `InvalidationHook`, `NarratorHook`, `PostBrowserActionHook`).
  Genuinely polymorphic — each runs distinct side-effects (event
  writes, verifier triggers, screenshot capture, narration). Defends
  itself.

- **KEEP — `EventStore`**. 1 prod impl (`SqliteEventStore`), 0 test
  impls. Phase 0 DEBT #6 documents the bare-async-fn-in-trait
  choice. Trait exists as a type-level boundary; collapsing would
  couple `ToolContext` to `Arc<SqliteEventStore>` and cost more than
  it saves.

- **SIMPLIFY — `SimplifyLlm`** (`deliverable/task_deliver.rs:107-117`).
  1 prod impl (`PlannerSimplifyLlm`, `:139-160`) + 1 test impl
  (`RecordingSimplify`, `:699-713`). Pure test-seam. Replace with a
  concrete struct + `#[cfg(test)]` mock injected via a field. ~60
  LOC removed. **New DEBT #54.**

- **SIMPLIFY — `ToolMaskPolicy`** (`dispatch/mask.rs:17, 23`). 1
  prod impl (`DefaultMaskPolicy`), 0 test impls. The 20-line match
  inside `is_available` is static config that would live more
  honestly as a `const MASK_RULES: &[(&str, AgentMode, bool)] = &[...]`
  + a 4-line lookup function. Removes the trait + one indirection.
  ~25 LOC removed. **New DEBT #55.**

- **KEEP — `Tool` family** (38 tools in `tools/builtin.rs`).
  Genuine multiplicity — each tool has distinct schema + behavior.
  Load-bearing.

- **KEEP — `NotifyDispatch`, `TargetResolver`, `GitShell`,
  `RollbackHandler`**. Each has 1 prod + 1 test impl that defends a
  real test-double need (Redis pub/sub mock, fixture-driven config,
  in-memory git fake, rollback recorder). Justified.

### Dep duplication

- ✅ Single-version pinning is clean across the workspace. No
  duplicate `reqwest`, `tokio`, `serde_json`, `uuid`, `regex`,
  `chrono` (chrono isn't even used — std `SystemTime`). No
  `once_cell` + `lazy_static` mix. No `sqlx` / `diesel`
  alongside `rusqlite`.

- ✅ Phase 2's 8 new deps (`lettre`, `mailparse`, `async-imap`,
  `clap`, `colored`, `ipnet`, `subtle`, `toml`) are single-version,
  workspace-pinned, no dev/prod bleed.

### Configuration sprawl

- ✅ `.env.example` (post DEBT #42 close) is the single canonical
  env-var manifest. ~40 vars, all documented. No shadow
  configuration discovered.

- ✅ `config/notify.toml` is the only TOML config file. JSON
  schemas embedded in `tools/builtin.rs` are necessary (LLM-visible
  tool registry). Markdown prompts (`narrator.system.txt`,
  `verifier.system.txt`) are lazy-loaded with fallback paths.

- **[L]** **`CLI_INTAKE_MAX_WAIT_SECS` read at two places** —
  `crates/seasoned-hand-cli/src/commands/task.rs:133` (client poll
  loop) and `crates/seasoned-hand-server/src/lib.rs:2489` (server
  query-param fallback). Phase 2 REVIEW raised this without filing
  DEBT; restated here. Drop client-side override and let the server
  own the timeout via the existing `max_wait_ms` query param.
  Bundle under #55 housekeeping.

### Module charters (missing/weak doc blocks)

- **[L]** `crates/seasoned-hand-core/src/plan/mod.rs:1` starts with
  `use std::sync::Arc;` — no `//!` preamble. Every other module in
  the crate has a 3-20 line WHY-block citing spec + DEBT entries.
  `plan` is conspicuously bare. **New DEBT #53.**

- ✅ All other 27 `mod.rs` files audited carry adequate-or-better
  module charters.

### Phase 0/1 features still load-bearing?

- ✅ Stuck detection (`agent/stuck.rs`) — wired at `agent/mod.rs:64`,
  used.
- ✅ Diversity injector (`agent/diversity.rs`) — wired at
  `agent/mod.rs:66`, used.
- ✅ Track C screenshots (Phase 1 DEBT #8) — wired in
  `PostBrowserActionHook`, used.
- ✅ Plan tools (`plan_create`, `plan_advance`, `plan_update`) —
  wired, used. Phase 0 DEBT #25 closed.
- ✅ Feature/progress tools (`feature_mark_done`, `progress_update`)
  — wired in Initializer prompt + agent loop.
- ✅ Checkpoint label/rollback — wired, masked correctly (rollback
  Internal-only per `dispatch/mask.rs:32-33`).

**Concern**: **No grep hits for "recite" / "todo recitation"** in
the agent loop. ARCHITECTURE.md §5 principle #4 ("Todo recitation:
re-read todo.md every ~10 turns") is spec'd as one of the 6
context-engineering principles, but Phase 1 1.12 ("Recite") may not
have been wired into the live loop. Either it landed under a
different name (search candidates: `progress_recite`, `progress_update`,
`recite`) or it was deferred. **Worth verifying before Phase 3** —
not promoting to DEBT pending the human's read of Phase 1 1.12
acceptance criteria.

### Tool catalog usage

- ✅ All 38 tools are either invoked in tests or registered as
  intentional Phase-N stubs with `[UNAVAILABLE...]` or schema-noted
  Phase gating. No silent dead tools.

- **[L]** Phase 0 stub tools `deploy_expose_port`, `deploy_apply_deployment`
  are NOT masked from the LLM — they're registered in the catalog
  with stub responses (`{ok:false, error:"not_implemented"}`).
  Per Phase 0 DEBT #3 this is intentional during the build-out
  window; today both stubs have been load-bearing for ~3 phases.
  Consider masking them via the existing `ToolMaskPolicy` machinery
  until they have real implementations. Bundle under #55.

### Single-caller helpers

- **[L]** `derive_title` at `crates/seasoned-hand-core/src/intake/router.rs:288`
  — 6-line helper, one caller (`:208`). Phase 2 REVIEW flagged for
  inlining; still extant. Bundle under #56 housekeeping.

- ✅ Other Phase 0/1 helpers reviewed (`replay_cost_baseline`,
  `walk_misc_events`, `count_actions`, `is_unique_violation`) earn
  their keep as semantic / pipeline helpers.

### Back-compat shims

- ✅ Phase 2 REVIEW caught `Initializer::run` + `AgentRunner::run`
  as test-only after 2.8b; sweep of other Phase 1 components
  (Worker, CheckpointManager, PlanManager, SandboxClient) shows no
  similar test-only-but-still-pub functions. `checkpoint/mod.rs:134`
  `pub async fn run()` is a documented placeholder pending story
  1.20 event-driven wiring (file header explains); ok to keep, but
  worth `#[doc(hidden)]` until wired. Bundle under #58.

### `tenant_id` ceremony

- **[L]** 100% of production construction sites hardcode `None`
  (verified across 55+ sites in core + server + cli). Field is
  pure forward-compat. Document a single Phase 5 conversion task
  per Agent B. **New DEBT #64.**

---

## (3) Stickiness — spec ↔ implementation drifts

Phase 2 REVIEW catalogued the WS verb session_id-vs-task_id drift
(DEBT #39), the 5 missing HTTP routes (DEBT #40), and the
`RouteOutcome<T>` unmet (DEBT #44). Cross-phase sweep adds:

### A — ARCHITECTURE.md text drift (consolidated)

Phase 1 DEBT #1 (tool count "32" vs shipped 38) and #2 (Next.js 15
vs shipped 16) already track the doc-only drift. The cross-phase
sweep adds three more drift items that belong in the same
ADR-011 + v1.1 bump:

- **[M]** **§2.2 sessions states list 5; V004 + code have 6**.
  `IDLE, RUNNING, FINISHED, ERROR, SUSPENDED` in the immutable doc;
  `+ VERIFYING` in `migrations/V004__verifications.sql:25-47` (correct
  table-recreate widening) and at `crates/seasoned-hand-core/src/agent/mod.rs:614`.
  Phase 1 architecture doc §3.2 documents the widening; the
  immutable doc text never updated. Bundle under **new DEBT #51**.

- **[M]** **`TaskStatus` 8-variant Phase 2 state machine is not in
  the immutable doc**. Architecture v1.0 talks only about
  `sessions.state`; the Phase 2 introduction of `tasks` (V006) with
  the `Drafted/Briefed/Confirmed/Running/Paused/Completed/Failed/Cancelled`
  machine is documented in `/specs/phase-2/architecture.md` only.
  A fresh AI session loading only ARCHITECTURE.md cannot reason
  about Phase 2 task semantics. Bundle under **new DEBT #51**.

- **[L]** **§7 tool-catalog block** still says 32 (29 Manus + 3
  learning); shipped 38 includes Phase 1 (`feature_mark_done`,
  `progress_update`, `checkpoint_label`, `checkpoint_rollback`) and
  Phase 2 (`task_deliver`) net adds. Phase 1 DEBT #1; restate under
  #51 only for the consolidated ADR fix.

### B — Event types reserved but never emitted

- **[informational]** `EventType::Knowledge`, `EventType::Datasource`,
  `EventType::Skill` are present in the enum
  (`crates/seasoned-hand-core/src/events/mod.rs:33, 55`) and accepted
  by the V002 CHECK constraint, but no production code path emits
  them. They are Phase 3+ reservations (per BASELINE §4
  "schema is forward-compat for Phase 5"). Worth a one-line "Phase 3
  Curator will populate; reserved" doc comment on the enum.
  **New DEBT #61** (informational, not a blocker).

### C — WS protocol shape (Phase 2 REVIEW already covers verbs)

- **[L]** **`ServerEnvelope::Event` carries `session_id` at both
  envelope level and payload level**. `ws.rs:22-27` plus the JSON
  `data` blob from `events.data` — clients see the same value twice.
  No collision risk (both come from the same DB row), but it's an
  un-documented redundancy on the WS wire. Bundle into the eventual
  ADR-011 + v1.1 ARCHITECTURE.md §4 cleanup.

- **[L]** **`ClientEnvelope::Pong` does not echo `id`**, and
  `ServerEnvelope::Ping` carries only `ts`. `ws.rs:41, 133-136` —
  Pong has no round-trip token. Phase 2 review caught the unit
  drift (μs vs unix s); this is a separate symmetry gap. With
  one in-flight Ping at a time it's fine; if Phase 4 keep-alive
  scales to multi-task UIs, add `id` to both. Belt-and-suspenders;
  no new DEBT.

### D — HTTP route shape

Phase 2 REVIEW DEBT #40 covers 5 missing spec'd routes. Two
additional drifts surface cross-phase:

- **[L]** **Status-code mapping for task transitions is not
  pinned**. `task_pause` / `task_resume` / `task_cancel` all return
  bare `StatusCode::OK` with empty JSON. Architecture §4 doesn't
  spec when 200 vs 202 vs 409 applies. Bundle into the
  DEBT #40 / ADR-011 doc fix.

- **[L]** **WS error envelope shape vs HTTP**. Phase 2 REVIEW
  caught `intake_rejected:duplicate_intake_id` (HTTP) vs
  `error: "duplicate_intake_id"` (WS) — unify on `<code>:<subcode>`.
  Sweep of other WS errors in `ws.rs:339, 397, 411` and HTTP errors
  in `lib.rs:892, 962, 1003+` shows the same inconsistency for
  other reason codes. Single pass to unify; bundle into a "Phase 3
  error-envelope unification" mini-DEBT. Recommended as part of
  #58 polish.

### E — `spec-check.sh` and CI

- **[L]** **`scripts/spec-check.sh` hard-codes expected tool count
  at `39`** (38 unique tools + the `task_deliver` registration
  override). The number is correct at HEAD but is detached from the
  spec — when Phase 3 adds learning tools, the gate will fail
  silently until manually updated. Add a comment block or a
  version variable. **New DEBT #62.**

- **[L]** **Frontend `pnpm test` is a passing stub**.
  `frontend/package.json` test script returns 0 without running
  anything; Playwright is `pnpm test:e2e` and only runs under
  `workflow_dispatch`. `just verify` calls `test-frontend` ->
  `pnpm test` and always passes. The brief mentions "pnpm test"
  as a gate; in reality the gate is a no-op. **New DEBT #63.**

- **[L]** **CI jobs run in parallel without ordering on spec-check**.
  `.github/workflows/ci.yml` (verified by inspection during
  orientation) doesn't `needs: [spec-check]` on the rust / frontend
  jobs. If spec-check fails, rust may still pass with new tools that
  break the count assertion. Low risk; documentation-level fix.

### F — Methodology docs (Agent D)

- ✅ Prompts under `/prompts/` (BMAD analyst/architect/pm + GSD
  execute-story) still match `/docs/methodology.md`. No drift
  from BASELINE §5 4-phase workflow.

- ✅ `.codex/config.toml.example` correctly loads AGENTS.md via
  Codex CLI's standard cascade.

- ✅ No Claude-only or Codex-only features in production code.
  LLM-agnostic per ADR-006 is honored.

### G — GLOSSARY coverage

- **[M]** GLOSSARY.md missing 5-7 load-bearing Phase 2 terms (Agent D):
  - `ChannelRegistration` (trait builder; appears 15+ times in
    CHANGELOG + Phase 2 architecture)
  - `IntakeRouter` / `DeliveryRouter` / `NotifyWorker` (Phase 2
    coordinators; appear in 8+ test files and the channel framework)
  - `WorkspaceTtlCron` (Phase 2 story 2.17 component)
  - `Provenance Manifest` (Phase 2 acceptance criterion)
  - `Brief` (data shape) vs `Briefing` (event/gate) — distinct
    concepts collapsed in current GLOSSARY entry
  Cost is felt in Phase 3 onboarding. **New DEBT #50.**

### H — Top-of-repo phase staleness

- **[M]** **`AGENTS.md:185-187`** still says
  `Phase: -1 (planning) → Phase 0 starting` /
  `Next milestone: complete Phase 0 (27 stories)`. Repo is at
  Phase 2 complete; CHANGELOG [0.2.0] (2026-05-16) is the
  authoritative state. Note `AGENTS.md` is in the §9 NEVER list —
  **propose; do not edit without explicit human approval**.

- **[M]** **`AGENTS.md:198`** lists `ADR-001 to ADR-008` —
  ADR-009 + ADR-010 exist on disk. Same NEVER constraint.

- **[M]** **`README.md:24`** says
  `**Phase -1** — Planning complete. Phase 0 (foundation) starting.`
  README is editable freely. Should read
  `**Phase 2 complete** → Phase 3 starting.` per BASELINE §6.

- **[M]** **`README.md:32`** says
  `## Quick start (not yet — Phase 0 in progress)`. Phase 0/1/2
  shipped; a real Quick Start is now possible (`docker compose up
  -d`, `just verify`, etc.). Either populate the section or update
  the disclaimer to "Phase 2 complete; quick-start docs at
  `/docs/getting-started.md`".

All four bundled as **new DEBT #49**.

### I — Phase 2 docs

- **[L]** `/specs/phase-2/requirements.md:1-3` status line says
  "v1.0 (BMAD PM persona output, 2026-05-13)" — never bumped to v1.1
  post-close-out. Same pattern as Phase 2 architecture v2.1 which
  did get the bump. Low priority.

---

## (4) Readability findings

Phase 2 REVIEW already audited Phase 2 comment / naming / module
organization surfaces. Cross-phase sweep adds:

### A — Comment policy (Phase 0/1 modules)

**WHAT-comments to delete** (selected; ~10 worst offenders):

- `crates/seasoned-hand-core/src/tools/builtin.rs:126, 170, 215, 246, 273`
  — section-divider comments
  (`// ===== message_notify_user =====`, etc.) restate the function
  name 2-3 lines below. Either convert to `/// # message_notify_user`
  block doc-comments or remove. ~5-10 lines. **New DEBT #56.**

- ✅ Phase 0/1 modules surveyed (`agent/`, `cost/`, `events/`,
  `verifier/`, `checkpoint/`, `plan/`, `dispatch/`, `capability/`,
  `router/`) show the same WHAT-comment discipline Phase 2 review
  praised. No further bulk cleanup needed.

**WHY-comments missing** (selected):

- `crates/seasoned-hand-core/src/agent/stuck.rs:8-9` —
  `STUCK_WARN_AT = 2; STUCK_HARD_AT = 4` constants without WHY.
  Bundle under **new DEBT #57**.

- `crates/seasoned-hand-core/src/agent/diversity.rs` — 4-variant
  array without a comment explaining "empirically sufficient for
  Phase 1; promote to DB at Phase 4 Curator per DEBT #7". Bundle
  under #57.

- `crates/seasoned-hand-core/src/llm/mod.rs:41-42` —
  `BIFROST_MASTER_KEY` placeholder without WHY ("Phase 0 DEBT #8;
  Phase 5 wires enforcement"). Bundle under #57.

- `crates/seasoned-hand-core/src/db/mod.rs` (or wherever DbPool is)
  — Phase 0 `Arc<Mutex<Connection>>` single-writer choice (Phase 0
  DEBT #1). Worth a one-line WHY. Bundle under #57.

- `crates/seasoned-hand-core/src/plan/render.rs:1-23` — `* 3`
  constant in token-budget heuristic without WHY (chars-per-token
  rough estimate). Bundle under #57.

**TODO/FIXME/XXX/HACK**:

- ✅ **Zero hits across all crates, all migrations, all
  `.github/workflows/*.yml`, all `frontend/**/*.{ts,tsx}` (non-node_modules),
  all `docs/`, all `prompts/`.** Discipline holds cross-phase.

### B — Public API surface (Phase 0/1)

- **[L]** **`pub async fn run()` on `CheckpointManager`** at
  `crates/seasoned-hand-core/src/checkpoint/mod.rs:134` is a
  documented placeholder pending story 1.20 wiring. Either mark
  `#[doc(hidden)]` until then or note explicitly in the function
  doc-comment that production callers should use
  `handle_plan_advance` directly. **New DEBT #58.**

- **[L]** **`pub async fn handle_request_with_watchdog` on
  `verifier::Worker`** at `verifier/worker.rs:657` is exported in
  `mod.rs:27` but called only from test fixtures + internal
  worker spawn. `pub(crate)` would suffice. Bundle under #58.

- **[L]** Re-exports in `crates/seasoned-hand-core/src/agent/mod.rs:38`
  (`pub use prompt::build_messages`) have no external callers. Audit
  + downgrade. Bundle under #58.

### C — Naming consistency cross-phase

- **[L]** **`*Router` vs `*Worker` asymmetry** — `IntakeRouter`,
  `DeliveryRouter` (in-process Tokio coordinators) vs `NotifyWorker`,
  `verifier::Worker` (Redis XREADGROUP consumers). Phase 2 REVIEW
  noted this; the rationale (in-process vs out-of-process) is
  defensible but undocumented. One-line module-doc cite in
  `notify/worker.rs` + `verifier/worker.rs`. Bundle under #57.

- ✅ Module-suffix consistency (`*Store`, `*Channel`, `*Sink`,
  `*Provider`) is uniform.

- ✅ Acronym casing (`Sqlite`, `Ws`, `Http`) is uniform.

- ✅ Misc kind strings — Phase 0/1 all `snake_case`; Phase 2's lone
  `"Deliverable"` (capitalized) is already filed as a `kind` casing
  drift in DEBT #41 close (renamed). Verified clean post-DEBT #41.

### D — Test naming

- ✅ Zero `test_*`, `it_works`, `_basic`, `_happy_path`,
  `_roundtrip` anti-patterns across all crates. Phase 2 REVIEW's
  `*_crud` finding (`intake_event_store_crud`, etc.) is the only
  outlier and is low priority. No new DEBT.

### E — Error messages

- **[L]** **HTTP `internal_error` / `db_error` opaque codes**
  (Phase 2 REVIEW recommendation): extend the
  `<code>:<subcode>` pattern to Phase 0/1 routes too.
  `crates/seasoned-hand-server/src/lib.rs:892, 962, 1003, 1101,
  1146, 1557, 1670, 1700, 2463` plus the Phase 0 routes at
  `:1229-1233`. Single pass; bundle under #58 polish.

- **[L]** **WS/HTTP error envelope unification** (carried from
  Stickiness/D). Phase 3 polish.

### F — Module organization (file size)

Top 10 largest prod Rust files (verified `find ... | wc -l`):

| File | Lines | Status |
|---|---|---|
| `crates/seasoned-hand-server/src/lib.rs` | **2879** | **NEW DEBT #52** — split per resource into `lib/{tasks,projects,channels,admin,workspace,intake,delivery}.rs` |
| `crates/seasoned-hand-core/src/tools/builtin.rs` | 1494 | Acceptable as flat tool registry; sectioning per #56 |
| `crates/seasoned-hand-core/src/deliverable/task_deliver.rs` | 1082 | Phase 2 REVIEW recommendation; bundle into **#60** |
| `crates/seasoned-hand-server/src/ws.rs` | 1045 | Acceptable — WS protocol cohesive |
| `crates/seasoned-hand-core/src/verifier/worker/tests.rs` | 979 | Pure test file; ok |
| `crates/seasoned-hand-core/src/agent/tests.rs` | 827 | Pure test file; ok |
| `crates/seasoned-hand-core/src/agent/mod.rs` | 725 | Borderline; **bundle into #60** for Phase 3 split candidates |
| `crates/seasoned-hand-core/src/sandbox/mod.rs` | 715 | Borderline; bundle into #60 |
| `crates/seasoned-hand-core/src/verifier/gate.rs` | 693 | Acceptable |
| `crates/seasoned-hand-core/src/verifier/worker.rs` | 677 | Phase 2 review tolerated; bundle into #60 |
| `crates/seasoned-hand-core/src/agent/init/mod.rs` | 657 | Borderline; bundle into #60 |
| `crates/seasoned-hand-core/src/notify/worker.rs` | 621 | Phase 2 REVIEW recommendation; bundle into #60 |
| `crates/seasoned-hand-core/src/channel/email/mod.rs` | 621 | Phase 2 REVIEW recommendation; bundle into #60 |

**New DEBT #52** (`lib.rs` split) is the biggest single readability
win — 2879 lines hosting ~40 HTTP handlers is the file most likely
to suffer merge-conflict + diff-review pain in Phase 3. Estimated
~8-12 hours to split cleanly; defers well.

**New DEBT #60** bundles the rest of the Phase 2 large-file
recommendations (still open) plus the cross-phase additions
(`agent/mod.rs`, `sandbox/mod.rs`, `verifier/worker.rs`,
`agent/init/mod.rs`).

### G — Frontend cohesion (Phase 0/1 components)

- ✅ All Phase 0/1 components are under 350 lines.
- ✅ State management pattern (props-down + local `useState`/`useCallback`/`useMemo`)
  is consistent with Phase 2 additions. No drift toward
  Context/Redux/Zustand.
- ✅ Timestamp handling — DEBT #33 close fixed both
  `decisions-tab.tsx:90` and `verifier-tab.tsx` to divide μs by 1000
  with WHY-comment citing DEBT #33. Verified inline. Agent E's
  report that this was still open is **rejected**.

### H — Doc comments on public types (Phase 0/1)

- **[L]** `crates/seasoned-hand-core/src/agent/mod.rs` `pub struct
  RunRequest`, `pub enum AgentError`, `pub struct AgentRunner`,
  `pub struct AgentRunnerDeps` — no `///` summaries. Bundle under
  **#58**.

- **[L]** `crates/seasoned-hand-core/src/checkpoint/mod.rs:31-49`
  `pub struct PlanAdvanceEvent`, `pub struct CheckpointManagerDeps` —
  no `///` summaries. Bundle under #58.

- ✅ Phase 1 verifier types (`VerifyTrigger`, `InvalidationReason`,
  etc.) are well-documented.
- ✅ Phase 2 deliverable / project / brief types are well-documented.

---

## (5) Convergent cross-cutting issues

Three patterns surface from multiple audit dimensions:

1. **Loopback-gate coverage on Phase 0/1 GET endpoints** — flagged
   by Security/A (workspace + sessions routes) and confirmed by
   inspection. Pattern: every Phase 2 task/project route was gated;
   the Phase 0/1 `/v1/sessions`, `/v1/sessions/:id/events`,
   `/v1/workspace/*` routes were not. Single sweep would close
   both DEBT #48 + DEBT #59.

2. **ARCHITECTURE.md v1.0 text drift** — flagged by Stickiness/A
   (sessions states 5→6, TaskStatus not present, tool count 32→38,
   Next.js 15→16). Phase 1 DEBT #1 + #2 are the umbrella; this
   review proposes consolidating all known drifts into a single
   ADR-011 + v1.1 bump.

3. **Module charters + WHY-comments cluster** — flagged by
   Simplicity (plan/mod.rs missing block) and Readability (~5 WHY
   sites missing on Phase 0/1 constants). A single polish pass
   covering #53 + #57 closes both threads.

---

## (6) Recommended new DEBT entries

Proposed for human approval. Numbered from **#48** to continue the
sequence from Phase 2 REVIEW (which reached #47). Severity column:
**H** = exploitable today; **M** = exploitable at Phase 5 scale or
will silently confuse contributors; **L** = belt-and-suspenders /
cosmetic / Phase 5 deferred. Ledger column indicates which DEBT.md
the entry belongs in (cross-phase items below propose a new
`/specs/REVIEW-DEBT.md` or appending to whichever phase's ledger is
the most natural home).

| # | Title | Severity | Ledger | Origin |
|---|---|---|---|---|
| 48 | `/v1/workspace/:session_id/*` not loopback-gated | **M** | cross-phase | Security/A |
| 49 | AGENTS.md §13/§14 + README.md phase status stale (3 doc sites) | **M** | cross-phase | Stickiness/H |
| 50 | GLOSSARY missing 5-7 Phase 2 terms | M | phase-2 | Stickiness/G |
| 51 | Consolidated ARCHITECTURE.md v1.0 text drift (subsumes phase-1 #1, #2 + sessions state + TaskStatus) | M | phase-1 (extend) | Stickiness/A |
| 52 | `crates/seasoned-hand-server/src/lib.rs` 2879-line split | M | cross-phase | Readability/F |
| 53 | `plan/mod.rs` missing module doc-block | L | phase-1 | Simplicity/Module charters |
| 54 | `SimplifyLlm` trait collapse to concrete + #[cfg(test)] mock | L | phase-2 | Simplicity/Trait surfaces |
| 55 | `ToolMaskPolicy` collapse or data-driven | L | phase-0 | Simplicity/Trait surfaces |
| 56 | WHAT-comments + section dividers in `tools/builtin.rs` | L | phase-0 | Readability/A |
| 57 | WHY-comments missing on Phase 0/1 constants (stuck/diversity/Bifrost/DbPool/plan-render) | L | cross-phase | Readability/A |
| 58 | `pub` surface shrinkage audit + missing doc-comments on Phase 0/1 types | L | cross-phase | Readability/B + H |
| 59 | `GET /v1/sessions`, `:id`, `:id/events` + feature-list + progress not loopback-gated | **M** | cross-phase | Security/A |
| 60 | Phase 1 large-file split set (`agent/mod.rs`, `sandbox/mod.rs`, `verifier/worker.rs`, `agent/init/mod.rs`) | L | phase-1 | Readability/F |
| 61 | EventType `Knowledge` / `Datasource` / `Skill` reserved but unwired (Phase 3 territory; doc-comment marker) | L | phase-2 | Stickiness/B |
| 62 | `spec-check.sh` hard-coded tool count `39` lacks phase-version gate | L | cross-phase | Stickiness/E |
| 63 | Frontend `pnpm test` is a passing stub | L | phase-2 | Stickiness/E |
| 64 | `tenant_id: None` 100% hardcoded — Phase 5 conversion meta-DEBT | L | phase-2 | Security/J |

Items #48 + #59 are the only **M-severity security additions**;
both close with the same one-line `require_loopback(remote)?` +
`ConnectInfo<SocketAddr>` extractor pattern already used at 16+
sites in `lib.rs`. #49 + #51 are M-severity doc-staleness drifts
that **require explicit human approval before any edit** because
`AGENTS.md` is on the AGENTS.md §9 NEVER list and ARCHITECTURE.md
is too.

---

## (7) Suggested follow-up commits

Tightest cuts, ordered by impact-per-LOC:

1. **~30-line fix** — add `require_loopback(remote)?` to
   `workspace_root` (`lib.rs:1042`), `workspace_proxy` (`:1049`),
   `list_sessions` (`:1229`), `get_session` (`:1230`),
   `list_events` (`:1231`), `get_feature_list` (`:1232`),
   `get_progress` (`:1233`). Update `ConnectInfo<SocketAddr>`
   extractor on each. Closes DEBT #48 + #59. **One commit, M+M security.**

2. **~15-line fix** — `README.md:24, 32` phase status update; add a
   one-line "Phase 2 complete — see CHANGELOG [0.2.0] and
   BASELINE.md §6" pointer. Does not touch AGENTS.md (still
   NEVER-listed). Half of DEBT #49 closes.

3. **~5-line fix** — request human approval to edit AGENTS.md §13
   + §14 phase status + ADR list. Without approval, this half of
   DEBT #49 stays open — file the entry, attach the rationale, and
   stop. Per AGENTS.md §9 NEVER, the agent (this one or any future)
   cannot proceed unilaterally.

4. **~50-line addition** — `/GLOSSARY.md` 7 new term entries
   (ChannelRegistration, IntakeRouter, DeliveryRouter, NotifyWorker,
   WorkspaceTtlCron, Provenance Manifest, Brief vs Briefing).
   Closes DEBT #50.

5. **~10-line fix** — `plan/mod.rs` 3-line module preamble citing
   V003 + ADR-010 + Phase 1 story 1.1. Closes DEBT #53.

6. **~30-line polish** — WHY-comments cluster: `agent/stuck.rs`
   constants, `agent/diversity.rs` 4-variant note, `llm/mod.rs`
   BIFROST placeholder, `db/mod.rs` DbPool Arc<Mutex> note,
   `plan/render.rs` `* 3` constant. Closes DEBT #57. Each cite
   should reference the relevant phase DEBT number for
   searchability.

7. **~10-line cleanup** — `tools/builtin.rs` section dividers
   converted from `// ===== name =====` to `/// # name` block
   comments (or removed). Closes DEBT #56.

8. **~60-line trait collapse** — `SimplifyLlm` → concrete struct
   + `#[cfg(test)]` mock. Closes DEBT #54. One commit, no behavior
   change.

9. **~25-line trait collapse** — `ToolMaskPolicy` → static
   `const MASK_RULES: &[...] = &[...]` + lookup function. Closes
   DEBT #55.

10. **~30-line polish** — `pub` shrinkage on Phase 0/1
    `handle_request_with_watchdog`, `build_messages`,
    `CheckpointManager::run` doc-hide, plus missing `///` summaries
    on Phase 0/1 types. Closes DEBT #58.

11. **DEFER — ARCHITECTURE.md v1.1 / ADR-011** — requires explicit
    human approval and a coordinated PR. Closes DEBT #51 +
    Phase 1 DEBT #1, #2. Bundle for the Phase 3 architecture
    boundary.

12. **DEFER — `lib.rs` 2879-line split** (DEBT #52) — substantial
    refactor; ~8-12 hours. Phase 3 warm-up if Phase 3 adds new
    handlers to the file.

13. **DEFER — Phase 1 large-file splits** (DEBT #60) — bundle into
    a single Phase 3 polish PR.

Items 1-10 are revertable independently; total estimated cost is
~250 added + ~150 removed across ~12 files. None touches the
NEVER-listed files unilaterally — items 3 (AGENTS.md) and 11
(ARCHITECTURE.md) stop and request human approval rather than
edit.

---

## (8) Positive findings worth recording

The cross-phase pass reaffirms and extends Phase 2 REVIEW §8:

- **Zero TODO/FIXME/XXX/HACK** across ALL three phases, all
  crates, all migrations, all CI workflows, all `frontend/`, all
  `docs/`, all `prompts/`. Discipline is unbroken from story 0.1
  through story 2.27 + hardening pass 4. AGENTS.md §10 ALWAYS rule
  is honored.

- **SQL parameterization is uniform across all 11 stores cross-phase**
  (Phase 0 `events/sqlite.rs`, `db/mod.rs`; Phase 1
  `verifier/persistence.rs`, `checkpoint/persistence.rs`; Phase 2 all
  Phase-2-introduced stores). Zero `format!`-built queries; ORDER BY
  constants; LIMIT placeholder-bound.

- **Verifier worker XREADGROUP loop is correct cross-restart** —
  malformed payloads XACK (no PEL retention of garbage), terminal
  errors XACK after emitting `verifier_verdict_error` Misc, watchdog
  timeouts XACK, only PEL-retention path is a crash strictly between
  consume and XACK (caught by next consumer via `triggered_at_event_id`
  dedupe). Phase 1 DEBT #15 close is solid.

- **Dependency hygiene is pristine** — single-version pinning
  across the workspace; no `reqwest` / `chrono` / `regex` / `tokio`
  duplication; no `once_cell` + `lazy_static` mix; Phase 2's 8 new
  deps (lettre, mailparse, async-imap, clap, colored, ipnet, subtle,
  toml) added cleanly.

- **`.env.example` is the single canonical config manifest**
  (post-DEBT #42 close). Operators can boot from a fresh checkout
  by copy-and-fill; no hunting through architecture docs for env
  names.

- **Loopback-gate coverage is at 16 routes** (`require_loopback`
  appears at `lib.rs:1628, 1798, 1914, 1957, 1994, 2037, 2075, 2112,
  2148, 2198, 2235, 2269, 2409, 2587, 2673` + the closed DEBT #34
  provenance route). The remaining gaps at `/v1/sessions*` and
  `/v1/workspace/*` are localized and trivially closable.

- **Module-level `//!` doc-blocks** cite specs + phase + DEBT in 27
  of 28 modules. The single outlier (`plan/mod.rs`) is filed as
  DEBT #53.

- **Acronym + naming consistency** (`Sqlite*`, `Ws*`, `Http*` —
  always lowercase-after-first; `*Store`, `*Channel`, `*Sink`,
  `*Provider` — always last-position suffix). Phase 2 REVIEW
  praised this; cross-phase sweep confirms it holds back to
  Phase 0.

- **Test naming discipline** — zero `test_*`, `it_works`, `_basic`,
  `_happy_path`, `_smoke` anti-patterns across 22 cargo test
  binaries + 7 frontend Playwright specs + 13 `#[ignore]` live
  tests. Names describe behavior in ≥95% of cases.

- **Verifier rollback default `false`** (Phase 1 DEBT #3 carryover)
  remains data-driven punt — Phase 2 workflow_dispatch jobs haven't
  accumulated precision data yet. Honest deferral; not silently
  flipped.

- **DEBT discipline cross-phase**: every closed item carries the
  closing SHA; every open item carries a named pay-down phase. The
  three DEBT.md files act as the project's coherent
  "what-shipped-vs-what-was-cut" memory layer. Phase 2's 47 entries
  + Phase 0's 28 + Phase 1's 13 give a complete picture of the
  trade-offs taken across 5 months of (very compressed) work.

- **Phase 0/1 features all still load-bearing**: stuck detection,
  diversity injector, Track C screenshots, plan tools, feature /
  progress tools, checkpoint label / rollback — none have decayed
  into dead code under Phase 2's growth. The single concern
  ("recite" / todo-recitation not located by grep) is flagged for
  pre-Phase-3 human verification, not promoted to DEBT.

- **Frontend coherence**: state management pattern (props-down +
  local hooks) is uniform Phase 0 → Phase 2; component file sizes
  stay under 400 lines without splitting; timestamp unit handling
  (post-DEBT #33) carries the WHY-comment cross-referencing the
  DEBT entry — exemplary instance of the project's
  build-for-next-session principle.

---

*Reviewer's overall assessment: the codebase enters Phase 3 in
strong shape. The single new M-severity security finding (workspace
HTTP proxy loopback gap) is one line per handler away from closure;
the doc-staleness drifts are surface-level and 90% closable without
touching NEVER-listed files. The deeper architectural debt
(ARCHITECTURE.md v1.0 text vs reality, `lib.rs` 2879-line monolith,
Phase 1 large-file split set) is well-bounded and not in Phase 3's
critical path — Phase 3's learning-system surface compounds none of
it. The discipline that produced Phase 2 in 3 days under
parallel-mode visibly held cross-phase: zero TODOs, uniform SQL
hygiene, honest DEBT ledgers, exemplary spec-driven story flow. The
project is ready for the learning system.*
