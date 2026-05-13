# Changelog

All notable changes to Seasoned Hand will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

### Pending decisions
- Multi-tenant DB strategy
- Auth method (API key vs OAuth)
- Default cloud sandbox provider
- Telemetry opt-in approach

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
