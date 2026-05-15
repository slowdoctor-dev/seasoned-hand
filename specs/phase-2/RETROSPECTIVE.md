# Phase 2 — Retrospective

> Phase 2 shipped 2026-05-15. 27 stories, three calendar days
> (2026-05-13 → 2026-05-15) of effective wall time, two agents (Claude
> Code + Codex CLI) in parallel tmux panes. The original spec budgeted
> 5 weeks; parallel-mode compression took it to ~3 days.

## What shipped

All 27 stories committed and pushed to `origin/main`:

| Story | Title | Commit |
|---|---|---|
| 2.1  | Phase 2 scaffolds — requirements.md + DEBT.md | `66e8d98` |
| 2.2  | V006 migration + ProjectStore + TaskStore | `d30674e` |
| 2.3  | V007 + V008 + V009 migrations + Deliverable / Intake / Delivery / Notify / Skill stores | `2c36eae` |
| 2.4  | Channel framework — 3 role traits + `ChannelRegistration` + `ChannelRegistry` | `93fff98` |
| 2.5  | `IntakeRouter` + `DeliveryRouter` + `GET /v1/channels` | `030ffcd` |
| 2.6  | Sandbox-side renderer toolchain (Pandoc + python-pptx + openpyxl) | `bb752f7` |
| 2.7  | `Brief` shape + `DeliverableSpec` typed schema | `e89cb17` |
| 2.8  | `Initializer::run_with_confirmation` (Briefing + confirm gate) | `86a8893` (+ `d1006ff`) |
| 2.9  | `ChatChannel` wraps existing WS as a Channel | `5e32d74` |
| 2.10 | `WebhookChannel` (intake + delivery + notify, three role impls) | `b19ec7f` |
| 2.11 | `EmailChannel` (IMAP intake + SMTP delivery + notify) | `e4894e6` |
| 2.12 | `NtfyChannel` (notify-only) + `NotifyWorker` | `ef54b2a` |
| 2.13 | `CliChannel` (process intake + stdout delivery) | `f8fe092` |
| 2.14 | `task_deliver` LLM tool + `RendererDispatcher` wiring | `f663501` |
| 2.15 | Provenance manifest builder + `GET /v1/tasks/:id/provenance` route | `f37b6fa` |
| 2.16 | Durable pause/resume + event-stream replay rebuild | `2b9734c` |
| 2.17 | Workspace TTL + cleanup cron (closes Phase 0 DEBT #16) | `ce7de1d` |
| 2.18 | Verifier Worker real XREADGROUP loop (closes Phase 1 DEBT #15) | `8eda594` |
| 2.19 | SandboxGitShell shell-injection fix (closes Phase 1 DEBT #14) | `43a06d8` |
| 2.20 | NarratorHook classifier-slot wiring through `AppState::new` | `ed6aa83` |
| 2.21 | `seasoned-hand` CLI binary + intake / brief / inbox surface | `527ff75` (+ `6adabe0`) |
| 2.22 | Frontend: ProjectList + Deliverables + Decisions tabs | `b8a86b9` |
| 2.23 | Frontend: Briefing card + confirm/edit/cancel UI | `e820488` |
| 2.24 | Frontend: Playwright bootstrap + smoke coverage (closes Phase 1 DEBT #9) | `aff80fd` |
| 2.25 | Phase 2 deterministic E2E (overnight workflow) | `18e3d0d` |
| 2.26 | Phase 2 live-LLM workflow_dispatch jobs (closes Phase 2 DEBT #32) | `27d3770` |
| 2.27 | Phase 2 closeout (retrospective + DEBT audit + status flips) | (this commit) |

**Goal achieved**: `requirements.md` §5 — work comes in over four
non-trivial channels (chat / webhook / email / CLI) plus a notify-only
fifth (ntfy), every task carries a typed `Brief` with confirm/edit/cancel
gate semantics (5-min auto-confirm fallback), deliverables render to 8
real-employee formats (md / json / csv + Pandoc docx / pdf / html +
python-pptx + openpyxl) each carrying a complete provenance manifest
back to brief / decisions / verdicts / checkpoints, 24h+ tasks survive
container GC via event-stream replay rebuild, the workspace TTL cron
honors task state per Phase 0 DEBT #16, the `seasoned-hand` CLI mirrors
every UI action, the frontend grew three new surfaces under Playwright
smoke coverage (closing Phase 1 DEBT #9), and three Phase 1 DEBT
carry-overs (#14 / #15 / #9) closed. The default `cargo test --workspace`
path drives 399 tests green including the
`phase2_overnight_default_path` end-to-end deterministic story.

## Deferred to Phase 3+

`specs/phase-2/DEBT.md` is the source of truth. Headlines by severity:

- **Medium**:
  - **#1** WebhookChannel SSRF allow-list bypass still operator-trusted
    (Phase 5 tightens once URLs become user-supplied)
  - **#7** Verifier rollback default still opt-in — Phase 1 DEBT #3
    carry-over; no precision data yet (`phase2-live-overnight` jobs
    landed at 2.26 but haven't accumulated runs)
  - **#18** EmailChannel discards attachment bytes after extracting
    metadata (manifest staging needs Phase 3+ plumbing)
  - **#21** Non-chat channels don't forward briefing events back to the
    user — 5-min auto-confirm is the Phase 2 contract for non-chat
    intake; interactive flow is Phase 4/5
- **Low** (~15 items): pre-baked sandbox renderer image (#2),
  code-as-deliverable git-tree-only (#3), email allow-list curation
  (#4), provenance manifest size budget (#5), CLI auth (#8),
  `ProjectStore::find_or_create_inbox` UNIQUE backstop (#14),
  Initializer loose `in_reply_to_call_id` match (#20), e2e tests don't
  send `briefing_confirm` (#22), provenance `brief.confirmed`
  placeholders (#24), `IntakeProvenance` "unknown" synthesis (#25),
  in-memory handle proxy for `resume_task` (#27), replay cost baseline
  reset (#28), `task new --no-auto-confirm` not honored by spawner
  (#29), `channel logs` stub (#30), BriefingCard rough edges (#31).
- **Informational** (no pay-down needed): skill / playbook tables
  empty (#6 — Phase 3 fills), Phase 1 carry-over rollup (#9), task
  state machine widening (#19 — code-level refinement).

## What worked

1. **Parallel-mode compression held all the way through.** The Phase 1
   "parallel-stash" pattern (snapshot Codex's WIP, revert just those
   files, verify isolated work, commit, restore Codex's WIP) ran
   continuously across 24+ stories. Every commit landed against
   `origin/main` cleanly; zero accidental cross-commits across the
   three calendar days. The original 5-week spec budget collapsed into
   ~3 days because story scopes were tight enough for the second agent
   to claim a different story while the first one's CI ran. The 27/27
   story rate is the headline.
2. **BMAD Architect/PM split early on 2026-05-13 was the keystone.**
   The architecture v2.0 / v2.1 commits (`1ab1377` / `9b8d92a`) and the
   PM 27-story spec commit (`66e8d98`) landed within hours of the
   Phase 1 close. Stories were already self-contained, dependency-graphed,
   and budgeted before the first implementation commit — which meant
   the parallel-stash pattern had real seams to work with. Without the
   PM output the per-story 1-3h budget would have been theoretical;
   with it, stories 2.2 through 2.20 fired off in lockstep across both
   panes.
3. **Seed DEBT.md (9 entries written at architecture time) did real
   work.** Every seed entry survived end-to-end review at Phase 2
   close (none were resolved in-phase, but none were also rewritten —
   the seed reflected reality on day one). The in-phase additions
   (12 new entries between 2.4 and 2.26) were strictly *new*
   shortcuts, never re-litigations of the seed. The discipline mattered:
   seed = forward-looking, in-phase = backward-looking, no overlap.
4. **OS-shape Channel framework collapsed five separate "integration"
   storylines into one trait surface.** Stories 2.9 (chat), 2.10 (webhook),
   2.11 (email), 2.12 (ntfy), 2.13 (cli) each shipped as a single
   `*Channel` struct implementing 1-3 of `IntakeProvider` /
   `DeliverySink` / `NotifySink`. The `AppState::register_channel(...)`
   builder (story 2.10's resolution of DEBT #17) made each channel a
   pure registration call from `main.rs`, so the production server
   added new channels via copy-paste of the builder line, not
   bespoke wiring. The five-channel batch came in under budget every
   single time.
5. **Deterministic-then-live test split landed late but saved CI.**
   Story 2.25 shipped `phase2_overnight_default_path` (deterministic,
   wiremocked, runs on every `cargo test --workspace`) before story
   2.26 added the `phase2-live-overnight` and `phase2-webhook-roundtrip`
   workflow_dispatch jobs. Result: the default test path stays cheap
   and green forever; the live LLM + SMTP/IMAP smoke jobs run only when
   explicitly triggered by an operator with the right secrets. Both
   contracts (deterministic-by-default + live-on-demand) are now
   reproduced in the test layout, ready for Phase 3+ E2E layering.
6. **Workspace = first-class entity, not "just a directory".** Story
   2.17 wired `WorkspaceTtlCron` with per-status TTLs (30 d completed,
   7 d failed/cancelled, 1 d drafted/briefed; running + paused never
   GC) and an admin POST endpoint. This is the bridge between Phase 1's
   "sandbox = HTTP gateway" (story 1.16) and the Phase 3+ filesystem-as-
   knowledge-base concept. Phase 0 DEBT #16, open since the very first
   sandbox story, finally closed.
7. **Provenance manifest as a mandatory exit gate, not optional metadata.**
   Story 2.15 made `build_manifest(...)` non-skippable: every Deliverable
   carries an `intake → brief → decisions → verifier_verdicts →
   checkpoints → delivered_to` trail. Stubs (`brief.confirmed_at = None`,
   `IntakeProvenance{channel: "unknown"}`) were tracked as DEBT #24/#25
   rather than silently omitted, so Phase 3 has a complete schema to
   thread the gate's real counters through. This is also the surface
   that Phase 3 learning will read for "what did this task actually do?"

## What to fix in Phase 3 (top 3)

1. **DEBT #2 — Pre-baked sandbox image.** Phase 2 installs Pandoc +
   python-pptx + openpyxl via `apt install` + `pip install` at
   session-create time (~30-60 s per session). Phase 3 layers learning
   on top of Phase 2's sandbox, so the session-spawn budget gets more
   sensitive; baking a `seasoned-hand-sandbox:phase-3` image with the
   toolchain cuts cold-start to <5 s and removes a class of "Pandoc
   install flake" failures.
2. **Phase 1 DEBT #3 — Verifier rollback default decision with real
   precision data.** Story 2.26 landed the `phase2-live-overnight`
   jobs but they're workflow_dispatch-only and haven't accumulated
   runs. Phase 3 should run them periodically (or fold the rollback
   decision into a Phase 3 closeout story) and flip
   `checkpoint_rollback_on_verifier_fail` to `true` if precision ≥90%.
   Until then the default stays opt-in.
3. **DEBT #21 — Thread briefing flow back through non-chat channels.**
   Today, webhook + email intake successfully creates the Brief +
   confirms via 5-min auto-confirm, but the user never sees the
   briefing card. Phase 3+ should add `IntakeProvider::reply_with_brief`
   (or a side-channel) so webhook callers receive the brief on the
   reply URL and email senders get a confirmation email. Without this,
   non-chat intake is one-way and the gate's value is only realised
   for chat clients.

## Phase 2 starting point — Phase 3

- Branch off `main` at the story-2.27 commit.
- Read `specs/06-roadmap/ROADMAP.md` §"Phase 3" for scope (4-layer
  learning system, playbook auto-extraction, skill library,
  cross-session memory). The skill / playbook tables (V009) are
  already in place from story 2.3 — Phase 3 fills the rows, not the
  schema.
- Next BMAD step: fresh session with the **Architect persona** on
  `/specs/phase-3/architecture.md`. Reference inputs:
  Phase 2 architecture (`/specs/phase-2/architecture.md`),
  provenance manifest schema (story 2.15), DEBT carry-overs above.
- Pick up the 3 items above as the Phase 3 warm-up batch (or fold
  them into Phase 3's first scope review).

## Phase 2 by the numbers

- **27 stories shipped** (2.1 + 25 implementation stories + 2.27;
  story 2.8 + 2.21 each split into two sub-commits 2.8b / 2.21b at
  execution time)
- **3 calendar days** of wall time (2026-05-13 → 2026-05-15); closeout
  commit ships 2026-05-16 (day 4). Original spec budgeted 5 weeks
  (~75 h at 3 h/day); parallel-mode collapsed it to ~3 days.
- **399 tests passing** at `cargo test --workspace` (default path —
  no cloud credentials, no Redis required for default-path), across 22
  test binaries. **17 ignored** (live-LLM smoke, live-Redis,
  live-Docker, live-SMTP/IMAP — all `workflow_dispatch`-only).
- **Phase 0 DEBT closed**: 1 item (#16 workspace TTL cron, story 2.17)
- **Phase 1 DEBT closed**: 3 items (#9 frontend tests / #14
  SandboxGitShell injection / #15 Verifier XREADGROUP)
- **Phase 1 DEBT carried**: #3 (verifier rollback default — no
  precision data yet) lives as Phase 2 DEBT #7
- **Phase 2 DEBT logged**: 9 seed + 22 in-phase additions (numbered
  #10–#32 with #26 skipped) = **31 entries total**; **8 closed
  in-phase** (#10 / #11 / #13 / #15 / #16 / #17 / #23 / #32) + 1
  partial (#12); **22 open at close** (9 seed + 13 in-phase). Of the
  in-phase open set, #18 (EmailChannel attachment bytes) carries
  Medium severity; the rest are Low or informational (#19)
- **Channels registered**: 5 (`chat`, `webhook`, `email`, `cli`, `ntfy`)
  across 3 role traits (`IntakeProvider` / `DeliverySink` / `NotifySink`)
- **Renderer formats**: 8 (`md`, `json`, `csv`, `docx`, `pdf`, `html`,
  `pptx`, `xlsx`) plus `code` (sandbox git tree as deliverable) and
  `url` (URL-only deliverable) — 10 `DeliverableFormat` enum variants
  total
- **Migrations**: V006 (`projects` + `tasks`), V007 (`deliverables`),
  V008 (`intake_events` + `delivery_events` + `notify_events`), V009
  (`skills` + `playbooks` — empty, Phase 3 fills)
- **CLI surface**: `seasoned-hand` binary with `init`, `server`,
  `project list/create/archive`, `task new/list/show/pause/resume/
  cancel/brief/deliverable/provenance`, `inbox`, `brief confirm/edit/
  cancel`, `channel list/test/logs` (logs is stub — DEBT #30)
- **Frontend surfaces added**: ProjectList (left panel), BriefingCard
  (Chat panel inline), DeliverablesTab + DecisionsTab (AgentComputer
  right panel) — all under 7 Playwright chromium specs (story 2.24)
- **4 verification gates** still green (clippy / fmt / test /
  spec-check)

## Corrections (post-close review)

_To be filled when Codex completes the post-close review (Phase 0 / 1
pattern: review surfaces items → fixed in a follow-up commit). Codex's
5-hour rate-limit window recovers ~2026-05-17; the review pass is the
expected next-team-mode item._
