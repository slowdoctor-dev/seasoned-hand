# Changelog

All notable changes to Seasoned Hand will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

### Added
- **Self-host the UI from the control plane** (issue #33, deferred from #5). When
  `SH_UI_DIST` points at a built Dioxus bundle, the Axum server serves it as the
  router **fallback** (`tower-http` `ServeDir` + `ServeFile` SPA fallback) so a
  single binary serves `/v1` + `/ws` + the UI. API routes always win; the static
  serve is public (the shell calls the auth-gated API itself); a missing/invalid
  `SH_UI_DIST` fails fast at boot, and unset → API-only (unchanged). New
  `AppState::with_ui_dist` builder + `serve_ui` integration test. Net-new dep:
  `tower-http 0.6` (`fs`), server crate only (ARCHITECTURE.md §1.1 addendum).
- **Dioxus UI functional progress** (Phase 6 story 6.4, partial): WS ack handling
  captures the `task_create` session_id into the shared selection; an interactive
  **BriefingCard** (confirm / **edit** / cancel via `briefing_confirm`);
  AgentComputer **Deliverables**, **Files** (workspace root), and **Verifier**
  (pass/fail verdicts) tabs. Added `Verification` / `WorkspaceListing` DTOs to
  `-dto`. The server now shares its session/deliverable DTOs via `-dto` (story
  6.3b). Real Monaco/xterm/noVNC interop shims drafted in `index.html` (story
  6.2 — written but not yet verified against a live session).
- **Shared `ServerEnvelope`** (Phase 6 story 6.3c): the server now serializes
  `seasoned_hand_dto::ServerEnvelope` (its private copy removed), so the
  server→client WS envelope is shared end-to-end. `-dto`'s `Ack`/`Error`
  optional fields gained `skip_serializing_if` for byte-identical output. The
  inbound `CommandPayload`/`ClientEnvelope` stay server-local by design (they
  carry deserialize-time dispatch the UI's send-only mirror doesn't need).
- **`seasoned-hand-dto` crate** (Phase 6 story 6.3) — wasm-safe, pure-serde
  shared wire DTOs. `seasoned-hand-core` now re-exports the domain entities
  (Project, Task, Deliverable + status enums + `legal_transitions`) from it, and
  `seasoned-hand-ui` consumes it directly (deleting its hand-mirrored `dto.rs`),
  so the backend and the wasm frontend share one definition. The DB-string
  mapping moved into `-dto` with `From<EnumParseError>` lifting into the core
  errors. Server-side adoption of the session/WS types is story 6.3b.

### Changed
- **`server/lib.rs` decomposition started — issue #22 batch F (part 2):** extracted
  the HTTP error machinery (`error.rs`: `ApiError`/`ApiResult`, `api_err`, the
  `map_*_error` helpers) and the route classifiers + simple request guards
  (`guards.rs`: `with_auth`/`public`/`self_gated`, `authorize_in_handler`,
  `require_loopback`) out of the 4.1k-line god-file. Pure code moves (behavior
  pinned by the integration suite). The larger, security-critical remainder
  (`state.rs`, the AppState-coupled tenant guards, and per-domain `routes/`
  modules) is tracked in #43.
- **Dioxus Tailwind pipeline** (issue #33, deferred from #5): replaced the Tailwind
  CDN `<script>` with a pinned Tailwind **v4.3.1 standalone-CLI** build (no Node) —
  `just build-css` emits the gitignored `crates/seasoned-hand-ui/assets/tailwind.css`
  (purged via `@source` content detection) wired through Dioxus `[web.resource]`;
  `dev-ui`/`build-ui` build the CSS first, while `check-ui` stays a Rust/wasm-only
  gate. CI's `ui` job validates `build-css`; a `workflow_dispatch`-only `ui-bundle`
  job runs a full `dx build` and asserts the bundle ships exactly one stylesheet.
- **`SandboxClient` connects to Docker lazily** — `SandboxClient::new` no longer
  opens the Docker socket at construction; the daemon is connected on first
  actual container operation (cached via `OnceCell`). This lets the control
  plane **boot without Docker** (REST API / dev / UI work) instead of aborting at
  startup with `Docker(SocketNotFoundError)`. Behaviour with Docker present is
  unchanged; operations that need a container still surface a clear error when it
  is absent. Verified: the server now runs migrations + serves with no Docker and
  no Redis (both already degrade gracefully).
- **Phase 6 started — Dioxus frontend migration begun** (2026-06-05). Phase 6
  (open-source release) is now active, opening with the Next.js → Dioxus migration
  (ADR-016). `crates/seasoned-hand-ui` scaffolded (unified-Rust UI; web target
  first). The earlier same-day "deferred until stabilization" decision was
  reversed; the stabilization items are retained as a **release-readiness
  checklist** in `/specs/06-roadmap/ROADMAP.md` that gates the public-release tag
  (not Phase 6 start). ROADMAP phase markers (3/4/5) reconciled to ✅ Complete.
- **Frontend: Next.js → Dioxus (unified Rust)** (ADR-016, amends the frontend
  clause of ADR-002). Adopts a single Rust/Dioxus UI crate targeting Web (WASM) /
  Desktop / Mobile from one codebase, sharing `seasoned-hand-core` DTOs (no
  TS↔Rust type codegen). The `/v1` REST + WebSocket boundary and the entire
  control plane are unchanged — frontend-layer swap only. Monaco / xterm / noVNC
  retained on web+desktop via JS interop; mobile `AgentComputer` degrades to
  read-only. BASELINE §4 + §7 #5 amended; ARCHITECTURE.md → v1.5. Decision
  recorded; implementation staged across Phase 6 (gated on a step-1 interop spike).
- **Relicensed MIT → Apache License 2.0** (ADR-015, supersedes the license clause
  of ADR-008). Adds an explicit patent grant + patent-retaliation clause for the
  public release. `LICENSE` now carries the canonical Apache 2.0 text; new top-level
  `NOTICE` file added; `Cargo.toml` SPDX id set to `Apache-2.0`; README / BASELINE /
  GLOSSARY / CONTRIBUTING references updated.

### Removed
- **Next.js frontend removed — Dioxus cutover** (issue #5, ADR-016 migration plan
  step 5). Deleted the entire `frontend/` Next.js 15 + React 19 + TypeScript app;
  the unified-Rust `crates/seasoned-hand-ui` (Dioxus) is now the only UI. Retired
  the pnpm/Next CI jobs (`frontend`, `frontend-e2e`) and the `typecheck` /
  `test-frontend` / `dev-frontend` `just` recipes, replacing them with a
  `cargo check --target wasm32-unknown-unknown` UI gate wired into `just verify`
  and CI. Dropped the commented Next.js service from `docker-compose.yml` and the
  `frontend/` ignore rules from `.gitignore`. Live docs (README, AGENTS,
  BASELINE, ARCHITECTURE v1.5, getting-started) reconciled to Dioxus-only.
  Deferred to a follow-up: a compiled Tailwind v4 pipeline (UI still loads Tailwind
  via CDN) and serving the `dx` bundle from the control plane / compose. Historical
  phase-0..4 specs are left untouched (they record what shipped at the time).
- Pre-Phase-0 internal bootstrap docs that leaked private development context and
  had no public value now that the project is built and public:
  `docs/github-setup-guide.md`, `docs/first-week-plan.md`, `docs/setup-checklist.md`.
  (Historical content remains in git history.)

### Security
- **Invitation-token login: org-binding + TTL enforcement (issue #6 post-hoc
  review of batch E #41).** The live login path
  (`auth::AuthSessionStore::login`) resolved identity from the user's **primary**
  membership and never enforced the invitation TTL — so (a) redeeming an
  invitation could mint a session scoped to the wrong organization/tenant for a
  user with multiple memberships (a stale primary would yield a cross-tenant
  session, since `organizations.tenant_id` is 1:1), and (b) an unconsumed
  invitation token never expired. Fixes: migration **V027** adds
  `organization_id` to `user_invitation_tokens`; `invite_user` persists it at
  mint; `login` now resolves the membership the token was minted for (fail-closed
  `NoMembership` if absent) and rejects tokens past `LOGIN_TOKEN_TTL_MICROS`
  (legacy NULL-org tokens fall back to primary resolution; they are single-use
  and short-TTL). Removed the never-wired, org-blind
  `InvitationService::verify_and_consume_login_token` (the divergent second
  consume path that masked the gap); its single-use/TTL coverage moved onto the
  live `login` path. *(Codex-found, Claude-fixed)*
- **Audit integrity, store-layer access control, invitation tokens — issue #22
  batch E:**
  - **Audit log is now tamper-evident + append-only at the DB layer**
    (`audit/ledger.rs`, migration **V026**). Each row carries a SHA-256
    `row_hash = H(prev_hash || row fields)` chained from the previous row (global
    chain; genesis = 64 zeros; legacy rows keep NULL hashes and the chain starts
    from the first hashed row). `BEFORE UPDATE`/`BEFORE DELETE` triggers
    `RAISE(ABORT)` so the table can't be mutated or deleted from — append-only is
    enforced by SQLite, not just convention. Added a `verify_chain` check. *(Codex)*
  - **Store-layer IDOR / privilege-escalation closed** (`org/mod.rs`).
    `Organization::get` / `User::get` / `for_user_project` are now tenant-scoped
    (a foreign-tenant PK reads as `NotFound`); `soft_deactivate` and the
    escalation-primitive `update_role` require an `AuthContext`, enforce
    `authorize(MembershipManage)` (admin-only), and scope the mutation to the
    caller's tenant. (Latent — these stores have no HTTP route yet; defense-in-depth.)
  - **Invitation login tokens are now verified, single-use, and expiring**
    (`org/invitation.rs`). `verify_and_consume_login_token` hash-checks the token,
    rejects unknown/expired (>7d from `created_at`)/already-consumed, and consumes
    it atomically (`UPDATE … WHERE consumed_at IS NULL`) so concurrent logins can't
    both succeed. Previously the minted tokens were stored but never read.
- **Request hardening — issue #22 batch D:**
  - **Explicit request body cap** (`DefaultBodyLimit::max(1 MiB)`, server `app()`)
    replacing axum's silent 2 MB default, applied to every route incl. the
    internet-facing intake handlers; `serde_json`'s own recursion limit bounds
    nesting depth, so size was the remaining vector.
  - **Global request timeout** (`TimeoutLayer`, 60s → `408`) so a hung sandbox/DB
    handler can't hold a connection open forever. The long-lived `/ws` and the
    `/v1/intake/cli` long-poll are registered after the layer and excluded.
  - **Notify `target_override` now SSRF-validated** (`notify/worker.rs`) — an
    operator-supplied override URL is checked against the same
    `ssrf::assert_public_address` guard the webhook channel uses (was returned
    verbatim), and the per-stream-entry fan-out is bounded by a `Semaphore`
    (`DEFAULT_MAX_IN_FLIGHT = 16`) instead of spawning one unbounded task each.
    *(Codex)*
  - **Verifier per-session maps now evict on session end** — the worker's session
    locks (`verifier/worker.rs`, race-safe `remove_if`) and the invalidation
    `sessions` map (`evict_session`) no longer grow unbounded. *(Codex)*
- **Medium-severity hardening — issue #22 batch A** (latent findings from the
  2026-06-14 repo review):
  - **No more plaintext credentials in `Debug`.** `ImapConfig`/`SmtpConfig`
    (`channel/email/imap.rs`, `smtp.rs`) now hand-write `Debug` to redact the
    password as `***` (host/port/username stay visible), so a stray `{:?}`/tracing
    at boot can't leak it.
  - **ntfy topic validated + percent-encoded.** `channel/ntfy.rs` rejects topics
    containing a scheme/`/`/`\`/`@`/`..`/`:`/control chars and percent-encodes the
    remainder, so a crafted `target_ref` can't rewrite the target URL.
  - **Inbound email Message-ID / attachment filename sanitized.**
    `channel/email/mod.rs` strips CR/LF (header-injection) and `/`/`\` (path
    traversal), caps length, trims leading/trailing dots, and falls back to
    `attachment` — applied to both inbound parse and outbound `filename_for`.
  - **Dioxus interop no longer builds JS via `Debug`.** `ui/src/interop.rs` encodes
    every value passed to the Monaco/xterm/noVNC `document::eval` shims with
    `serde_json` (plus U+2028/U+2029 escaping) instead of `{:?}` — attacker-
    influenced workspace content (a Monaco `value`) can no longer break out of the
    string literal and inject script.

### Fixed
- **Agent context window + UI reconnect — issue #22 batch F (part 1):**
  - **Per-iteration context now replays the RECENT window + anchors the seed
    brief** (`agent/prompt.rs`). `build_messages` previously replayed the *oldest*
    100 events (`EventStore::query` is `ORDER BY id ASC LIMIT N`), so on a 50+
    tool-call task the agent stopped seeing its own recent activity. It now fetches
    the most-recent N (`SqliteEventStore::recent_events`) and anchors the session's
    first event (the original brief) so the goal never falls out of the window;
    `pair_messages` folds any window-boundary orphan to plain text (no provider 400).
  - **Dioxus reconnect no longer replays history from 0** (`ui/src/ws.rs`). The
    per-session resume watermark is now kept at its highest value, so an
    app-initiated `Subscribe{from:0}` (initial load) can't lower a watermark already
    advanced by received events — a reconnect resumes from the last seen id.
- **Cost & curator correctness — issue #22 batch C:**
  - **Final-step cost is now recorded** (`agent/mod.rs`) — `record_step_cost` moved
    above the verifier-completion and breaker early-returns, so a task can no longer
    blow `cost_cap_cents` on its last step without the spend being accounted for.
  - **Cost delta re-baselines on a Bifrost counter reset** (`cost/mod.rs`) — a
    `current < baseline` snapshot (gateway counter restarted at 0) now bills the
    post-reset value instead of `.max(0)`-masking it as a 0 delta (silent
    under-billing); `saturating_sub` guards the subtraction. Drift `delta_pct` uses
    `abs_diff` (no overflow/`i64::MIN.abs()` panic); the per-user `SUM(cost_cents)`
    is documented fail-loud (SQLite raises on i64 overflow).
  - **Curator `Quarantine` now maps to its own `decision_type = "quarantine"`**
    (was `"keep"`), so it no longer pollutes the `WorkPatternExtractor`
    self-improvement stats; migration **V025** expands the `decision_type` CHECK
    (also splitting archive-recommend/apply/restore). *(Codex)*
  - **Embedding-budget breaker re-checked inside the candidate loop** — one cycle
    can no longer issue ~100 embedding calls after the budget tripped mid-cycle.
    *(Codex)*
  - **Curator LRU is now O(1)-ish** (`HashMap` + `VecDeque`) instead of
    `Vec::remove(0)` + linear scan (quadratic per cycle). *(Codex)*
  - **`review_required` sampling is now an explicit policy field** — the
    undocumented revision-id-hash-mod-10 (~30%) is replaced by a configurable
    `CuratorConfig.review_sample_rate` (env-wired, default 0.30). *(Codex)*
- **Tenant-isolation correctness — issue #22 batch B:**
  - **`list_events` now routes through the canonical `require_session_tenant`
    guard** (`server/src/lib.rs`) instead of an inline `JOIN projects … p.tenant_id`.
    The old inner join excluded chat-spawned sessions (project_id NULL, tenancy
    from `task_id`), so their legitimate owner got a spurious 404.
  - **`list_sessions` tenant filter fixed.** It previously matched
    `sessions.project_id IN (SELECT id FROM tasks …)` — overloading a project id
    against task ids, returning the wrong set and dropping task-spawned sessions.
  - **Fail-closed session-tenancy predicate** (review hardening): the shared
    `SESSION_TENANT_PREDICATE` now requires *every* present direct parent (project
    via `project_id`, task via `task_id`) to match the tenant and excludes orphans,
    so a corrupt row whose project and task resolve to *different* tenants belongs
    to **neither** — closing a `COALESCE(p, t)` vs task-first-projection disagreement
    that could have let one tenant read a mismatched session's raw events. Applied
    uniformly to `require_session_tenant`, `list_sessions`, and
    `require_verification_tenant`.
  - **Added a cross-tenant regression test for the redacted feed**
    (`GET /v1/events/:session_id`): a tenant-A caller reads none of a tenant-B
    session's rows; the tenant-B caller sees its own. Plus task-spawned
    reachability + isolation tests for `list_events`/`list_sessions`.
  - **Documented the accepted residual risk** that `events` has no `tenant_id`
    column (tenancy derived via the session→task→project chain, sentinel fallback);
    `tenant_event_view.tenant_id` is materialized at write time so reads are stable
    (`events/visibility.rs`).
- **Medium-severity correctness — issue #22 batch A:**
  - **Cost client now has a 15s HTTP timeout** (`cost/mod.rs`) — a hung Bifrost
    can no longer stall cost snapshots (and the cost-cap polling that depends on
    them) indefinitely.
  - **SQLite `busy_timeout = 5000` + `synchronous = NORMAL`** set on file-backed
    connections (`db/mod.rs`) — a writer waits for the lock instead of failing
    immediately with `SQLITE_BUSY` once the multi-connection pay-down lands.
  - **Removed a latent `unwrap()`** in `checkpoint/persistence.rs` — the cursor is
    now bound directly via the query `match` (no `has_cursor`/`unwrap()` desync).
- Public-hygiene cleanup before open-sourcing: genericized a personal dev-machine
  path in `specs/phase-0/stories/story-0.18.md`; blanked the placeholder Bifrost
  dev keys in `.env.example`; refreshed the ADR index (`specs/01-architecture/
  decisions/README.md`) to list ADR-011 through ADR-015.

### Pending decisions
- Default cloud sandbox provider
- Telemetry opt-in approach
- Phase 6 scope finalization (open-source release polish + marketplace)

---

## [0.5.0] — 2026-05-21

Phase 5 release: Multi-User + Organization. 33 stories shipped (5.1–5.33).
Spec references: `/specs/phase-5/requirements.md`, `/specs/phase-5/architecture.md`,
ADR-014 (V013 tenant-RBAC reconciliation).

### Added
- Multi-user core domain: `crate::org` exposes `OrganizationStore`,
  `UserStore`, `MembershipStore`, `ProjectRoleOverrideStore`, and
  `UserDeactivationService` for the mandatory-reassignment lifecycle.
  V013 bootstraps the `org-legacy-default` / `user-legacy-admin`
  sentinel triple so audit + cost + share FKs always resolve.
- RBAC policy engine: `crate::auth::policy::authorize` enforces the
  §4.3 matrix (admin/user/viewer × 8 actions) with project-role
  override precedence. `SystemAuth::for_worker` + `for_cli_operator`
  give every worker spawn a tenant-pinned admin identity.
- Audit log surface: `crate::audit::AuditLogger` writes the
  immutable `audit_log` table + dual-writes a Misc
  `audit_logged` event per OQ §8 Option C. HTTP `GET /v1/audit` +
  CLI `seasoned-hand audit list` admin-only.
- Task hand-off lifecycle: `crate::handoff::TaskHandoffService`
  enforces the Drafted/Briefed/Confirmed/Paused → direct,
  Running → MustPauseFirst, terminal → TerminalState state machine
  with optimistic concurrency (`expected_updated_at`) and per-handoff
  audit + Misc `task_paused_for_handoff` event emission.
- Per-user cost ledger: `crate::billing::user_cost::NearlineWriter`
  (1h cron) + `ReconciliationJob` (24h cron with current+previous
  month) keep `user_cost_ledger` per `(tenant, user, month_yyyymm)`
  reconciling within NFR-5.4's ±0.5% drift budget.
- Tenant-aware event redaction (closes Phase 4 SECURITY_REVIEW
  iter-3 carry-in / DEBT #S-1): `crate::events::visibility::apply`
  runs as a write-time hook on every `SqliteEventStore::append`,
  emits a redacted `tenant_event_view` row keyed by `(event_id,
  tenant_id, visibility_level)`. `visibility::query` (any role) +
  `visibility::query_raw` (admin + Action::EventRawRead + audit_log
  row) gate read access.
- Session-search RBAC: `session_search_index` gains `tenant_id` +
  `visibility_level` columns + index; queries enforce
  `(tenant_id = caller, visibility_level IN allowed_set)` at the DB
  layer per arch §10.
- Sharing services: `crate::sharing::sop` + `crate::sharing::playbook`
  with optimistic concurrency on `share` / `unshare` /
  `update_visibility_state`. `playbook_shares.visibility_state`
  ∈ (review, shared, suspended); curator auto-shares high-confidence
  revisions and the matcher only surfaces `shared` rows.
- Curator tenant boundaries + failure taxonomy (F-5.14): every curator
  SQL query gains `tenant_id = :tenant`; F-5.14 emits the three
  deterministic categories (`curator_cycle_refused/tenant_unresolved`,
  `curator_decision_quarantined/tenant_unresolved`,
  `curator_decision_quarantined/cross_tenant_ref`).
- Curator rationale schema versioning (closes DEBT #96):
  `SchemaVersion::wrap_v2` envelopes new payloads as
  `{"schema_version": 2, "data": {...}}`; readers tolerate V1
  (Phase 4 flat) + V2 + future versions via fallback-to-V1 detection.
- Global strict-config harmonization (closes DEBT #91):
  `crate::config::strict` hosts `parse_{bool,u32,u64,f32}_strict` +
  `env_*_or_default` helpers shared across server + CLI + workers.
  SH_LEARNING_ENABLED + SEASONED_HAND_ROLLBACK_ON_VERIFIER_FAIL now
  fail-fast on invalid values.
- FTS5 named weight constants (partial close DEBT #76):
  `crate::search::fts_weights` exposes per-table column-weight
  structs; uniform-1.0 today, full retune deferred to Phase 6 per
  documented dogfood procedure (`specs/phase-5/dogfood_fts_retune.md`).
- 7 named NFR acceptance harnesses (story 5.26–5.32) + composite
  5-actor benchmark, all under their per-spec CI budgets.
- 7 schema migrations: V013 (tenant + RBAC + audit + cost + projection
  bootstrap), V014 (projects/tasks/deliverables NOT NULL), V015
  (skills NOT NULL), V016 (playbooks NOT NULL + FTS trigger recreate),
  V017 (tasks.owner_user_id), V018 (session_search_index RBAC columns
  + FTS rebuild), V019 (11 curator tables NOT NULL), V020
  (intake/delivery/notifications NOT NULL).

### Changed
- Architecture document advanced to v1.4 with ADR-014 reconciliation
  notes for §2.5 event schema, §3.2 tenant chain, §4 RBAC matrix,
  §13.1 rationale envelope contract.
- Phase baseline status advanced to `Phase 5 complete → Phase 6
  starting` in `BASELINE.md` and `AGENTS.md` §13.
- `scripts/spec-check.sh` now checks Phase 5 close-out hooks (V013
  schema + auth/audit/visibility/billing/handoff/org modules +
  ARCHITECTURE v1.4) as Check #10, plus the Phase 5 dependency
  addendum (Check #9, DEBT #97).
- AuthContext now mandatory at every service boundary;
  `MissingTenantContext` is the fail-closed error.

### Closed (DEBT close-out matrix)
- `#S-1` (tenant-scoped event redaction) — CLOSED via story 5.14
- `#91` (global strict-config harmonization) — CLOSED via story 5.22
- `#93` (optional fork-promotion governance) — CLOSED via story 5.8
- `#96` (curator rationale schema evolution tooling) — CLOSED via story 5.25
- `#97` (per-crate dependency justification) — CLOSED via story 5.23
- `#76` (FTS5 weight retune) — PARTIAL CLOSED via story 5.24
  (named-constants surface landed; full retune deferred to Phase 6
  per documented dogfood eval procedure)
- `#92` (adaptive auto-archive thresholds) — DEFERRED TO PHASE 6 via
  story 5.25 (needs production telemetry; ±5pp gate criterion documented)
- `#94` (retrospective tiered model-by-size) — DEFERRED TO PHASE 6
  via story 5.25 (needs cost/quality measurement from named
  retrospective harness; ≥15% cost reduction gate)

### Fixed
- 27 fixture files updated to set explicit `tenant_id` so the
  V014-V020 NOT NULL flips don't break the test corpus.

---

## [0.4.0] — 2026-05-19

Phase 4 release: Curator + self-improvement loop. 22 stories shipped (4.2-4.23).
Spec references: `/specs/phase-4/requirements.md`, `/specs/phase-4/architecture.md`,
ADR-013 (V011 schema reconciliation).

### Added
- Curator runtime: `CuratorWorker`, `SqliteCandidateBuilder`, `LlmSemanticAdjudicator`,
  `SqliteConsolidationEngine`, `SqliteConflictDetector`, `SqliteRetrospectiveGenerator`,
  `SqliteWorkPatternExtractor`, `SqliteOperatorReviewQueue`,
  `SqliteKnowledgeDatasourceWriter`, `EmbeddingBudget`, and
  `CuratorRetentionJob` (story 4.23 / NFR-4.4 close).
- Phase 4 schema migrations: V011 lands the curator ledger
  (`playbook_revisions`, `playbook_revision_outcomes`, `curator_decisions`,
  `curator_review_queue`, `sop_conflicts`, `knowledge_items`, `datasource_items`,
  `weekly_retrospectives`, `retrospective_citations`, `curator_search_index` +
  FTS5 virtual table + AI/AD/AU maintenance triggers); V012 adds
  `curator_decisions_summary` for the retention/compaction tail.
- Event taxonomy v1.3: `Skill{kind:"curation_decision"}` + `Misc` curator
  families (`curator_cycle_*`, `curator_decision_quarantined`,
  `curator_budget_circuit_open`, `curator_retrospective_refused`,
  `curator_storage_cap_warning`, `curator_retention_cycle_completed`,
  `playbook_extraction_*`).
- Phase 4 acceptance benchmark `phase4_warm_full_loop_benchmark` (story 4.21):
  cold→curate→warm replay over 270 verified artifacts; precision@3 = 1.0,
  78% stale-playbook ratio reduction, elapsed ~184 ms.

### Changed
- Architecture document advanced to v1.3 with ADR-012 + ADR-013 reconciliation
  notes for §2.5 event schema and §3.2 curator surface.
- Phase baseline status advanced to `Phase 4 complete → Phase 5 starting` in
  `BASELINE.md` and `AGENTS.md` §13.
- `scripts/spec-check.sh` now checks Phase 4 curator + retention spec hooks
  (V011/V012, `CuratorRetentionJob`, ARCHITECTURE.md v1.3 reconciliation).

### Fixed
- Closed Phase 3 DEBT inherited per F-4.26 closure matrix (see
  `/specs/phase-4/DEBT.md`). Residual partial: #76 (full FTS weight retune
  deferred per §6), #91 (global strict-parse harmonization deferred to
  Phase 5 per §6).

---

## [0.3.0] — 2026-05-18

Phase 3 release: learning loop foundations and production extraction wiring.
Spec references: `/specs/phase-3/requirements.md`, `/specs/phase-3/architecture.md`.

### Added
- Production `PlannerSlotExtractionHandler` implementing verifier-gated synchronous
  extraction through planner-slot LLM, deterministic redaction/adversarial checks,
  quality-floor enforcement, output cap handling, and `playbooks` writes.
- Server wiring for `.with_extraction(...)` on `VerifierGate`, gated by
  `SH_LEARNING_ENABLED` (default `true`).
- Extraction handler integration tests for success path, adversarial rejection, and PII
  redaction.

### Changed
- Phase baseline status advanced to `Phase 3 complete → Phase 4 starting` in
  `BASELINE.md` and `AGENTS.md` section 13.
- Story/requirements tracking updated: story 3.17 marked done, requirements table updated.

### Fixed
- Closed Phase 3 DEBT #84 by shipping and wiring the production extraction handler
  (previously every PASS emitted `extraction_handler_not_configured` and wrote no playbook).

---

## [0.2.0] — 2026-05-16

Phase 2 release: Employee interface (OS-shape). 27 stories shipped.
Spec reference: `/specs/phase-2/RETROSPECTIVE.md`.

### Added
- **Project / Task / Subtask schema** (story 2.2, `d30674e`): V006
  migration adds `projects` + `tasks` (status machine
  `drafted → briefed → confirmed → running ⇄ paused → completed |
  failed | cancelled`), nullable `tenant_id` on every new row.
  `ProjectStore` + `TaskStore` with `find_or_create_inbox`.
- **Deliverable / Intake / Delivery / Notify / Skill stores** (story
  2.3, `2c36eae`): V007 `deliverables`, V008 `intake_events` +
  `delivery_events` + `notify_events`, V009 reserves `skills` +
  `playbooks` (empty; Phase 3 populates).
- **Channel framework** (story 2.4, `93fff98`): `IntakeProvider` +
  `DeliverySink` + `NotifySink` traits, `ChannelRegistration` builder,
  `ChannelRegistry`. One `*Channel` struct per integration implements
  1-3 role traits.
- **IntakeRouter + DeliveryRouter + `GET /v1/channels`** (story 2.5,
  `030ffcd`): fan-in `mpsc::Sender<IntakeEvent>` drained by
  `IntakeRouter::run`; `DeliveryRouter::deliver_task` resolves a
  registered `DeliverySink` per task reply-target.
- **Sandbox-side renderer toolchain** (story 2.6, `bb752f7`):
  startup-time `apt install pandoc texlive-xetex` + `pip install
  python-pptx openpyxl` (~30-60 s per session; pre-baked image is
  DEBT #2).
- **`Brief` shape + `DeliverableSpec` typed schema** (story 2.7,
  `e89cb17`): `Brief { goal, phases[], success_criteria[],
  expected_deliverables: DeliverableSpec[] }`; `DeliverableFormat`
  enum (markdown / json / csv / docx / pdf / html / pptx / xlsx /
  code / url).
- **Initializer briefing-confirm gate** (story 2.8, `86a8893` + 2.8b
  `d1006ff`): `Initializer::run_with_confirmation` emits Misc
  `briefing_pending` + ServerEvent `Briefing{briefing_call_id}`, waits
  for `briefing_confirm | edit | cancel` on a per-task mpsc, falls
  back to 5-min auto-confirm. WS `briefing_confirm` verb routes
  through `forward_briefing_confirm` keyed by `briefing_senders`.
- **ChatChannel** (story 2.9, `5e32d74`): wraps existing WS as a
  `Channel`; `AppState::new` registers it as the always-on baseline.
- **WebhookChannel** (story 2.10, `b19ec7f`): `POST /v1/intake/webhook`
  (intake, token-gated) + `DeliverySink` (HTTP POST to
  `reply_target.url`) + `NotifySink` (HTTP POST notify). Default-deny
  SSRF posture in `assert_public_address` (loopback / private /
  link-local rejected; `WEBHOOK_DELIVERY_ALLOWLIST` env bypasses
  per-CIDR). Replaces `with_channels` with
  `AppState::register_channel(ChannelRegistration)`.
- **EmailChannel** (story 2.11, `e4894e6`): IMAP poll intake + SMTP
  delivery + SMTP notify. `INTAKE_EMAIL_ALLOWED_SENDERS` regex allow-list.
- **NtfyChannel + NotifyWorker** (story 2.12, `ef54b2a`):
  Redis-Stream `notify_request` consumer + dispatch loop;
  `NtfyChannel` ships notify-only.
- **CliChannel** (story 2.13, `f8fe092`): in-process `IntakeProvider`
  + `DeliverySink` (stdout); registered via
  `AppState::register_cli_channel` (closes DEBT #23 in story 2.21a).
- **`task_deliver` LLM tool + `RendererDispatcher`** (story 2.14,
  `f663501`): Worker-mode-only tool, server-side renderer dispatch by
  filename extension (md / txt / json / csv raw; docx / pdf / html
  via Pandoc; pptx via python-pptx; xlsx via openpyxl).
- **Provenance manifest** (story 2.15, `f37b6fa`): mandatory exit
  gate; `build_manifest(...)` produces an `intake → brief →
  decisions → verifier_verdicts → checkpoints → delivered_to` trail;
  `GET /v1/tasks/:id/provenance` returns the manifest.
- **Durable pause / resume + event-stream replay** (story 2.16,
  `2b9734c`): WS `task_pause` gains `durable: bool` (default `true`);
  resume rebuilds sandbox from event-stream replay when the container
  is gone, reconstructs Plan + feature-list + progress.
- **Workspace TTL + cleanup cron** (story 2.17, `ce7de1d`):
  `WorkspaceTtlCron` honors task state (active: never GC, paused:
  none / configurable, completed: 30 d, failed/cancelled: 7 d,
  drafted/briefed: 1 d) + admin `POST /v1/admin/sandbox/cleanup`.
  Closes Phase 0 DEBT #16.
- **Verifier Worker XREADGROUP loop** (story 2.18, `8eda594`):
  `Worker::run` bootstraps consumer group via `XGROUP CREATE` +
  `XREADGROUP GROUP verifier-workers <worker-{host}-{pid}> BLOCK 5000
  COUNT 16`; per-session FIFO via `DashMap<SessionId, Mutex>`; global
  cap via `Semaphore`. Closes Phase 1 DEBT #15.
- **NarratorHook classifier-slot wiring** (story 2.20, `ed6aa83`):
  `AppState::new` wires the classifier-slot LLM path through the
  Dispatcher build sequence; closes story-1.15 exec-note.
- **`seasoned-hand` CLI binary** (story 2.21a / 2.21b, `527ff75` /
  `6adabe0`): `init`, `server`, `project list/create/archive`, `task
  new/list/show/pause/resume/cancel/brief/deliverable/provenance`,
  `inbox`, `brief confirm/edit/cancel`, `channel list/test/logs`
  (logs is stub — DEBT #30).
- **Frontend ProjectList + DeliverablesTab + DecisionsTab** (story
  2.22, `b8a86b9`): left-panel ProjectList above TaskList; right-panel
  Deliverables tab + Decisions tab in AgentComputer.
- **Frontend BriefingCard** (story 2.23, `e820488`): Chat-panel
  inline renderer intercepts `Briefing` events; confirm / edit /
  cancel buttons emit WS `briefing_confirm` cmd.
- **Frontend Playwright bootstrap + smoke coverage** (story 2.24,
  `aff80fd`): Playwright 1.60 + 7 chromium specs; `pnpm test:e2e`
  wired; `frontend-e2e` CI job (workflow_dispatch). Closes Phase 1
  DEBT #9.
- **Phase 2 deterministic E2E** (story 2.25, `18e3d0d`):
  `phase2_overnight_default_path` on the default `cargo test
  --workspace` path (wiremocked); `phase2_webhook_roundtrip`
  ignored-by-default.
- **Phase 2 live-LLM workflow_dispatch jobs** (story 2.26, `27d3770`):
  `phase2-live-overnight` and `phase2-webhook-roundtrip` CI jobs gated
  on operator trigger + provider keys.

### Changed
- **Task state machine widened** (story 2.8 / DEBT #19):
  `Drafted / Briefed / Confirmed → Cancelled` added to
  `legal_transitions` so `BriefingAction::Cancel` can fire from the
  briefing gate.
- **`AppState::register_channel(ChannelRegistration)`** replaces
  `with_channels(ChannelRegistry)` (story 2.10 / DEBT #17):
  per-channel merge; chat baseline survives every subsequent
  registration.
- **WS `task_create` flow inverted** (story 2.9 → 2.8b): handler now
  pushes an `IntakeEvent` through the `IntakeRouter` (closes DEBT
  #15); session row + AgentRunner spawn move into
  `WsInitializerSpawner` post-briefing-confirm.
- **`task_deliver` persists absolute `rendered_content_path`** (story
  2.26 / DEBT #32): resolves workspace-relative path against
  `SandboxClient::workspace_host_path` before writing the deliverables
  row; `EmailChannel::deliver` can `tokio::fs::read(...)` directly.

### Fixed
- **SandboxGitShell shell-injection** (story 2.19, `43a06d8`):
  `commit_phase` switched from `format!("git commit -m \"{phase_title}\"")`
  to `git commit -F /workspace/.commit-msg/<phase_id>.txt`; 6 malicious
  payloads now contained. Closes Phase 1 DEBT #14.
- **Frontend automated tests** (story 2.24, `aff80fd`): Playwright
  bootstrap + 7 chromium specs covering the new Phase 1 and Phase 2
  surfaces. Closes Phase 1 DEBT #9.
- **Verifier Worker XREADGROUP loop** (story 2.18, `8eda594`):
  replaces the polling shim; verdicts now flow end-to-end through
  Redis Streams. Closes Phase 1 DEBT #15.
- **Workspace cleanup cron** (story 2.17, `ce7de1d`): paid down Phase
  0 DEBT #16 — orphan sandbox workspaces no longer accumulate
  indefinitely.

### Deferred (phase-2/DEBT.md)
- SSRF allow-list bypass still operator-trusted (#1) — Phase 5 tightens
- Pre-baked sandbox renderer image (#2) — Phase 3+
- Code-as-deliverable git-tree-only (#3) — Phase 4
- Email allow-list operator-curated (#4) — Phase 5
- Provenance manifest size budget 100 KB inline (#5) — Phase 3+
- Skill / playbook tables empty (#6) — Phase 3 fills
- **Verifier rollback default still opt-in** (#7) — carries Phase 1
  DEBT #3 into Phase 3 (no precision data accumulated yet from
  `phase2-live-overnight` jobs)
- CLI auth (#8) — Phase 5
- `ProjectStore::find_or_create_inbox` UNIQUE backstop (#14) — Phase 5
- EmailChannel discards attachment bytes (#18) — Phase 3
- Initializer loose `in_reply_to_call_id` match (#20)
- Non-chat channels don't forward briefing events (#21)
- e2e + phase1_gaia tests don't send `briefing_confirm` (#22)
- Provenance `brief.confirmed_at` / `IntakeProvenance` synthesis
  stubs (#24, #25)
- `resume_task` in-memory handle proxy (#27)
- Replay cost baseline resets to zero on rebuild (#28)
- `task new --no-auto-confirm` metadata flag not honored by spawner
  (#29)
- `seasoned-hand channel logs` is a stub (#30)
- BriefingCard eviction / reload / server-error UX (#31)

---

## [0.1.0] — 2026-05-13

Phase 1 release: Manus 5-layer deep execution. 23 stories shipped.
Spec reference: `/specs/phase-1/RETROSPECTIVE.md`.

### Added
- **Plan Manager** (story 1.1, `5c5dec9`): real `plan_create` /
  `plan_advance` / `plan_update` tools backed by a `plans` SQLite
  table; structured `== PLAN ==` sticky-context render replaces the
  Phase 0 raw-event rendering. Closes Phase 0 DEBT #25.
- **Sandbox workspace as a git working tree** (story 1.3, `1192162`):
  every session bootstraps `git init` + hardcoded identity + empty
  initial commit, so checkpoints land on a real tree from step one.
- **Initializer** (story 1.4, `bde7921`): briefing → plan creation →
  workspace bootstrap → `feature-list.json` + `progress.txt`; two
  new LLM tools `feature_mark_done` + `progress_update`.
- **Tool-mask layer** (story 1.5, `992e6e6`, PRINCIPLE #2):
  `AgentMode` enum (Initializer / Worker / Verifier / Internal) +
  `DefaultMaskPolicy` keeps masked tools in the catalog with
  `available:false` instead of hot-swapping the schema.
- **Context Recitation** (story 1.6, `fd3d090`, PRINCIPLE #4): every
  10 iterations the runtime injects the tail of `progress.txt` as a
  `Misc{kind:"progress_recite"}` event.
- **Bifrost alias resolver + capability table** (story 1.7,
  `fa3dacd`): `router::capability::Resolver` queries Bifrost's
  `/v1/models/<alias>` at startup, learns the upstream provider model
  id, and looks up tool-calling / json-mode / vision flags. Closes
  Phase 0 DEBT #22.
- **Verifier hard-fail on identical main/verifier model id** (story
  1.8, `e0a8a04`): server refuses to start when the verifier slot
  resolves to the same provider model as main (architecture §2.4.3 —
  prevents L4 meta-cognition from collapsing into self-consistency).
- **Verifier persistence + read routes** (story 1.9, `720f8a0`):
  V004 migration adds the `verifications` table; new
  `GET /v1/sessions/:id/verifications` (paginated) and
  `GET /v1/verifications/:id` routes.
- **Verifier Worker runtime** (story 1.9b, `fe8086e`): Redis-Streams
  consumer + per-session concurrency + watchdog + graceful shutdown.
  Fresh-context build per request, FAIL-biased system prompt.
- **TaskComplete trigger** (story 1.10, `a4514b8` + follow-ups
  `798c388`, `5019cb9`, `76b7a0e`, `4c09465`): RUNNING → VERIFYING
  transition on `idle` / `final-message`; gate applies the §2.4.5
  transition table (pass → FINISHED, fail+suggestion → resume with
  suggested plan update, fail-no-suggestion → SUSPENDED).
- **Invalidation Detector + Invalidation trigger** (story 1.11,
  `142aeec`): SHA-256-mismatch heuristic on a closed allow-list
  (`file_read` / `file_write` / `file_str_replace`); emits
  `verifier_request` with `trigger:"Invalidation"`.
- **Circuit Breaker (4 conditions) + CircuitBreaker trigger +
  Diversity Injector** (story 1.12, `cab77d1`): unified
  Stuck / Cost / MaxSteps / ErrorRate breakers route through the
  Verifier before terminating; Diversity Injector rotates 4 phrasing
  variants for stuck-recovery prompts.
- **Checkpoint Manager** (story 1.13, `e5150b6`): V005 migration +
  in-sandbox `git commit` on every `plan_advance`; `checkpoint_label`
  LLM tool sets the label for the next checkpoint.
- **Checkpoint rollback** (story 1.13b, `fc0ad84`):
  `AgentMode::Internal` `checkpoint_rollback` tool (LLM-masked) +
  admin POST endpoint with loopback / token / state / sandbox-pause
  guards; opt-in `RollbackHandler` trait wires the verifier-fail
  rollback path (default OFF — see DEBT #3).
- **Hook output-truncation file-ref path** (story 1.14, `7132f16`):
  `events::truncation::write_large_or_inline` writes >16 KB hook
  outputs to `/workspace/.eventfiles/<event_id>.<ext>` and records
  `EventPayloadBody::FileRef{path, sha256, size, content_type}`.
  Closes Phase 0 DEBT #21.
- **NarratorHook** (story 1.15, `a1c3de8`): templated path for ~13
  cheap tools + classifier-slot LLM path (2-second timeout,
  `tool_choice:none`, max_tokens=50); emits
  `Message{role:"assistant", ui:"narrate", call_id}` before every
  tool dispatch. Sticky-context builder filters `ui:"narrate"`
  messages so narration never re-enters agent context.
- **3-track Browser representation — backend** (story 1.16,
  `7cbbf88`): `PostBrowserActionHook` emits side-channel
  `Misc{kind:"browser_track_b", call_id, dom_text_ref}` for DOM text
  and `Misc{kind:"browser_track_c", call_id, file_ref}` for PNG
  screenshots; failure-tolerant `*_skipped` variants.
  `SandboxClient::browser_view` / `browser_screenshot` are the
  canonical accessors shared by the Phase 0 tool and the hook.
- **WS task control — real** (story 1.17, `d95e264`):
  `task_pause` → sandbox pause + SUSPENDED + `Misc{kind:"task_paused"}`;
  `task_resume` → unpause + RUNNING + runner resume + `task_resumed`;
  `task_cancel` → cancellation token + sandbox destroy + FINISHED +
  `task_cancelled`. Closes Phase 0 DEBT #27.
- **Frontend: narration lane + Verifier verdict pane** (story 1.18,
  `096415b`): Chat renders `Message{ui:"narrate"}` events as inline
  em-dash italic notes; AgentComputer gains a "Verifier" tab with
  pass/fail badges, lazy `/v1/verifications/:id` detail fetch on
  expand, and evidence chips resolved against a client-side event
  index built in `HomeShell`.
- **Phase 1 E2E runtime verification** (story 1.20, `fd11caf`):
  10-task GAIA-Level-1 fixture corpus (`#[ignore]` behind
  `SEASONED_HAND_PHASE1_SMOKE=1`); `phase1_stable_50step` test on
  the default `cargo test` path with a deterministic wiremocked
  ≥50-step task; `phase1-live-smoke` `workflow_dispatch` CI job.

### Changed
- **Sticky context** filters out `Message{ui:"narrate"}` events so
  narration never re-enters agent context (architecture §12 q2).
- **Phase 0 `browser_view` tool** rewired to call the new shared
  `SandboxClient::browser_view` accessor (no parallel HTTP path
  between the tool and the PostBrowserActionHook).
- **Hook ordering**: NarratorHook → EventEmittingHook → InvalidationHook
  → PostBrowserActionHook (registered in this order so narration lands
  before the Action event for clean UI ordering).
- **WS hook lifted to `HomeShell`** so Chat and AgentComputer share
  one WebSocket and one `Map<event_id, ServerEvent>` index — gives
  the Verifier verdict pane synchronous evidence-chip lookup.

### Fixed
- **VerifierGate cursor persistence** (`4c09465`): historical
  `verifier_verdict` rows no longer re-replay on every restart;
  `verifier_gate_ack` Misc markers seed the cursor.
- **WS pong-echo flake** (`4c09465`): server no longer echoes
  `{type:"pong"}` envelopes on client pong replies.
- **Stuck-detector test instability** (`5019cb9`): 3 pre-existing
  test failures patched so the default `cargo test --workspace`
  path is green from `5019cb9` forward.

### Deferred (phase-1/DEBT.md)
- Verifier automatic rollback (#3) — mechanism shipped, default opt-in
- Single invalidation heuristic (#4) — file SHA mismatch only
- Single verifier slot for all 3 triggers (#5)
- Egress allowlist deny-default (#6) — Phase 5
- Diversity Injector variants in a Rust constant (#7)
- Track C screenshots full-resolution / no cleanup (#8)
- Frontend automated tests (#9) — Phase 2 brings Playwright/RTL
- Lazy evidence-event resolution (#10)
- Sandbox `git` identity hardcoded (#11) — Phase 5
- Verifier fail-closed default on Bifrost 5xx (#12)
- ARCHITECTURE.md text drift on tool count / Next.js version (#1, #2)
- Classifier-slot wiring through `AppState::new` (story 1.15 exec
  notes)

---

## [0.0.1] — 2026-05-12 (Phase 0)

Phase 0 release: Working skeleton. 27 stories shipped.
Spec reference: `/specs/phase-0/RETROSPECTIVE.md`.

### Added
- Initial repository scaffold
- `AGENTS.md` as universal source of truth for AI coding agents
- `CLAUDE.md` import wrapper for Claude Code
- `.codex/config.toml.example` for Codex CLI
- `BASELINE.md` as single-entry-point session starter
- `/specs/00-philosophy/` — VISION, PRINCIPLES, NON_GOALS
- `/specs/01-architecture/ARCHITECTURE.md` — overall (immutable v1.0)
- `/specs/01-architecture/decisions/` — ADR-001 through ADR-008
- `/specs/06-roadmap/ROADMAP.md` — 6-phase plan (22 weeks)
- `/specs/phase-0/requirements.md` — Phase 0 scope (27 stories)
- `/specs/phase-0/stories/story-0.1.md` — Bifrost Docker setup
- `/specs/phase-0/stories/_template.md` — story format
- `/docs/manifesto.md` — why this project exists
- `/docs/brand.md` — visual and verbal identity
- `/docs/methodology.md` — SDD + BMAD + GSD details
- `/docs/getting-started.md` — human onboarding
- `/docs/first-week-plan.md` — first 7 days action plan
- `/docs/setup-checklist.md` — domain and account acquisition
- `/docs/using-claude-and-codex.md` — multi-tool patterns
- `GLOSSARY.md` — project terminology
- `/prompts/` — BMAD personas (analyst, architect, pm) + GSD execute-story
- `/scripts/spec-check.sh` and `status.sh`
- `LICENSE` (MIT)
- `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`
- `.github/ISSUE_TEMPLATE/` and `PULL_REQUEST_TEMPLATE.md`
- `.github/workflows/ci.yml`
- `docker-compose.yml` (Bifrost + Redis skeleton)
- `justfile`, `.env.example`, `.gitignore`

### Added (post-Manus interview, 2026)
- ADR-009: Map tool (embarrassingly parallel) — deferred to Phase 4+ with full spec
- ADR-010: Plan as Process Control Block (PCB)
- PRINCIPLES.md #16: Context is RAM, sandbox filesystem is disk
- PRINCIPLES.md #17: Plans are sticky context anchors, never free text
- ARCHITECTURE.md § 6: 4-layer verification framework (L1 Deterministic, L2 Cross-source, L3 Observation, L4 Meta-cognition)
- ARCHITECTURE.md § 2.3: plans SQLite table for Plan Manager
- ARCHITECTURE.md OS metaphor expanded: Plan = PCB, current_phase_id = Program Counter
- BASELINE.md § 11.5: external validation section (Manus direct Q&A)
- GLOSSARY.md: PCB, Plan, plan_advance/update/create, sticky context, 4-layer verification, map tool, goal drift, cumulative state

### Changed (post-Manus interview)
- ARCHITECTURE.md § 4 agent loop: explicit Briefing + Plan create steps, plan-aware iteration
- ARCHITECTURE.md OS metaphor mapping: Kernel = LLM (not agent runtime), Scheduler = agent runtime
- BASELINE.md stack table: added Plan Manager and Verification (4-layer) rows
- BASELINE.md hard decisions: added #9 (RAM/disk) and #10 (Plan as sticky PCB)

---

## How to update this file

### When adding entries to [Unreleased]

Group changes under sections:

- **Added** — new features
- **Changed** — changes to existing functionality
- **Deprecated** — features marked for removal
- **Removed** — features actually removed
- **Fixed** — bug fixes
- **Security** — security fixes (note CVE if applicable)
- **Pending decisions** — open architectural questions (our addition to
  Keep a Changelog, useful pre-1.0)

Each entry should be a single line, written in past tense for completed
changes:

> Added 12-slot model router with capability detection

Reference the relevant ADR, story, or PR if non-obvious:

> Changed sandbox cleanup policy to TTL-based (ADR-009, story 4.7)

### When releasing a version

1. Create a new section above [Unreleased]:
   ```
   ## [0.1.0] — YYYY-MM-DD
   ```
2. Move all Unreleased entries into it
3. Reset [Unreleased] to empty section structure
4. Commit with `chore: release v0.1.0`
5. Tag: `git tag -a v0.1.0 -m "release v0.1.0"`
6. Push tags: `git push --tags`

### Version numbering

Pre-1.0 (we're here):
- 0.x.y — breaking changes allowed in any release
- Use minor bumps (0.1 → 0.2) for phase completions
- Use patch bumps (0.1.0 → 0.1.1) for fixes within a phase

Post-1.0 (after Phase 6):
- Major (1.x → 2.x): breaking changes
- Minor (1.0 → 1.1): backward-compatible features
- Patch (1.0.0 → 1.0.1): backward-compatible fixes
