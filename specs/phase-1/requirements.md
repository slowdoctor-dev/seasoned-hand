# Phase 1 — Manus 5-Layer (Deep Execution)

> **Status**: v1.0 (BMAD PM persona output, 2026-05-12)
> **Duration**: 4 weeks
> **Goal**: 50+ tool-call sessions stable. Verifier wired. Plan Manager real.
> Initializer/Worker. Circuit Breaker hardened. Git checkpoint + rollback.
> 3-track browser. Live narration.
> **Bridges**: `/specs/01-architecture/ARCHITECTURE.md` (immutable) +
> `/specs/phase-1/architecture.md` (Architect output, `3f2fa6c`) →
> `/specs/phase-1/stories/story-1.X.md` (this PM output).

---

## 1. Goals

By end of Phase 1:

1. The agent runtime executes 50+ tool-call sessions without stuck/cost/error
   termination on representative tasks (50-step synthetic task + GAIA Level 1
   fixture).
2. Every task starts with a structured Plan (multi-phase), a workspace
   `feature-list.json`, and an append-only `progress.txt`.
3. A separate **Verifier Worker** (different model, FAIL-biased prompt, fresh
   context) fires on three triggers: TaskComplete, Invalidation, CircuitBreaker.
4. The Plan Manager renders a structured sticky-context block on every
   iteration (replacing Phase 0's raw-event rendering).
5. Six context-engineering principles are enforced — three of them
   (`tool-mask`, `recitation`, `diversity-injection`) ship as new components
   in this phase.
6. A unified Circuit Breaker (four conditions) consults the Verifier before
   ending a session.
7. The sandbox workspace is a git working tree from session start; phase
   advances create checkpoints; admin endpoint can roll back.
8. The frontend surfaces three new lanes: live narration, Verifier verdict,
   and a 3-track BrowserTab (live noVNC + DOM text + screenshot strip).
9. Five Phase 0 DEBT items (#18, #21, #22, #25, #27) are closed.

**Not in scope** (per ROADMAP): briefing protocol (Phase 2), learning system
(Phase 3), Curator (Phase 4), multi-user (Phase 5), polished docs (Phase 6).

---

## 2. Non-functional requirements

Phase 0 budgets carry forward. New budgets:

| Requirement | Target |
|---|---|
| Verifier run latency (1 LLM call) | < 6 s p95 |
| Verifier cost per task (≤3 trigger fires) | < 5 ¢ |
| Narration overhead per LLM-narrated tool call | < 200 ms |
| Narration cost per 50-step task | < 2 ¢ |
| Plan render token budget | ≤ 1000 tokens (unchanged from Phase 0) |
| Checkpoint create (in-sandbox `git commit`) | < 300 ms |
| Invalidation hash bookkeeping heap | < 1 MB / session @ 10k files |
| Cold start with Initializer | < 12 s |
| End-to-end 50-step task wall time | < 4 min p50 (main = Claude-class) |
| Session cost cap default | $2 (raised from Phase 0's $1) |

Full table: `/specs/phase-1/architecture.md` §7.

---

## 3. Functional requirements

### 3.1 Plan Manager (real)

Phase 0 stub closure (DEBT #25). `plan_create`, `plan_advance`, `plan_update`
mutate the existing `plans` table per ADR-010. Sticky context renders the
structured `Goal / Phase 1 [done] / Phase 2 [active] / …` block (max
1000 tokens). Plan{op} event emitted on every mutation.

### 3.2 Initializer / Worker

Initializer runs once per task, pre-loop:

1. Calls `planner` slot → structured plan (goal + 3-8 phases).
2. Calls `plan_create(goal, phases)`.
3. Writes `/workspace/feature-list.json` and `/workspace/progress.txt`.
4. Hands off to the Worker (a single agent loop instance per task).

The Initializer **does not** consume an iteration cap slot.

### 3.3 Workspace bootstrap

SandboxClient creates the workspace as a git working tree with identity
`Seasoned Hand <seasoned-hand@local>` and an initial empty commit on session
create. The LLM never sees git tools.

### 3.4 Context-engineering principles

| # | Principle | Phase 1 enforcement |
|---|---|---|
| 1 | KV-cache friendly prefix | spec-check asserts sticky context never reorders earlier turns |
| 2 | No mid-iteration tool catalog changes | **Tool-mask layer** — masked tools stay in the catalog with `available:false` |
| 3 | Filesystem as memory | Verifier always reads `feature-list.json` instead of replaying events |
| 4 | Todo recitation | **Context Recitation** — every 10 iterations, runtime injects `progress.txt` tail |
| 5 | Errors preserved | Verifier `fail` verdicts persisted as Misc events |
| 6 | Diversity injection | **Diversity Injector** — strategy-change prompt rotates through 4 phrasings + one referenced observation |

### 3.5 Verifier Worker

- Separate Tokio task per control-plane process.
- Subscribes to Redis Streams `verify_request` (consumer group `verifier`).
- Concurrency limit: 1 per session (FIFO), 2 across sessions (configurable).
- Single LLM call per run, no tools.
- FAIL-biased system prompt loaded from `/config/prompts/verifier.system.txt`.
- Output: strict JSON `{ verdict, reason, evidence_event_ids, suggested_plan_update }`.
- Every run persisted to new `verifications` table.

Three trigger sources:

- **TaskComplete** — fires when agent calls `idle` or `message_notify_user`
  with `final:true`; session enters new `VERIFYING` state until verdict.
- **Invalidation** — fires from PostToolUse hook when file SHA-256 mismatches
  the last-known content for a path **and** the change came via a
  non-allow-listed tool (anything other than `file_write` / `file_str_replace`).
- **CircuitBreaker** — fires when the breaker trips; asks Verifier whether
  partial work is salvageable.

Startup gate: `verifier` slot **must** resolve to a different provider+model
ID than `main`. Server fails to start on equality.

### 3.6 Circuit Breaker (unified, 4 conditions)

Single Tokio actor per session, four conditions: **Stuck** (≥4 duplicate
assistant outputs), **Cost** (≥ cap), **MaxSteps** (≥ max), **ErrorRate**
(≥5 of last 10 observations `ok:false`). Each trip enqueues a Verifier
CircuitBreaker request; verdict drives recovery (plan_update, SUSPEND, or
ERROR). Cost remains a hard cap.

### 3.7 Checkpoint Manager

- On `Plan{op:"advance"}` → `git add -A && git commit -m "phase N: <title>"`
  inside the sandbox. SHA + phase_id stored in new `checkpoints` table.
- Internal-only `checkpoint_rollback` invoked by Verifier verdict (opt-in,
  default off) or admin HTTP endpoint.
- Admin endpoint: `POST /v1/sessions/:id/checkpoints/:checkpoint_id/rollback`
  with `127.0.0.1` bind + `SEASONED_HAND_ADMIN_TOKEN` header. Refuses to
  rollback while session is `RUNNING`.

### 3.8 New LLM-visible tools

| Tool | Purpose |
|---|---|
| `feature_mark_done(feature_id)` | Flip a feature's status to `done` in `feature-list.json`; emit Misc `feature_done` |
| `progress_update(line)` | Append one line to `progress.txt` |
| `checkpoint_label(label)` | Set the label of the next phase-advance checkpoint |

Plus **internal-only**: `checkpoint_rollback` (never exposed via Bifrost
tool schema; invoked by Verifier or admin endpoint only).

Net LLM-visible catalog after Phase 1: **36** (33 Phase 0 + 3 new). The
ARCHITECTURE.md "32-tools" text drift is tracked in `phase-1/DEBT.md` #1.

### 3.9 Narrator Hook

PreToolUse hook that emits `Message{role:"assistant", ui:"narrate", content}`:

- **Templated path** (0 LLM tokens) for: `plan_*`, `idle`, `file_read`,
  `file_find_*`, `glossary_lookup`, `playbook_search`, `sop_read`,
  `feature_mark_done`, `progress_update`, `checkpoint_label`.
- **Classifier-slot LLM path** (~50 tokens, cheap model) for:
  `file_write`, `file_str_replace`, `shell_*`, `browser_*`, `info_search_web`,
  `deploy_*`, `message_*`.

Narration events are **never** included in subsequent agent context (UI signal
only). Failure mode: 2s timeout → skip narration, emit Misc `narration_skipped`.

### 3.10 3-track Browser representation

Every `browser_*` invocation produces three tracks via PostBrowserAction hook:

| Track | Source | Storage |
|---|---|---|
| A. Live noVNC | sandbox port 6080 (Phase 0) | not stored (live only) |
| B. DOM text | `browser_view` (or hook calls it post-action) | event payload inline if < 16 KB, else file_ref |
| C. Screenshot | sandbox screenshot endpoint → `/workspace/.tracks/<call_id>.png` | file_ref in Misc `browser_track_c` |

### 3.11 New HTTP routes

| Method | Path | Purpose |
|---|---|---|
| GET  | `/v1/sessions/:id/verifications` | List Verifier runs |
| GET  | `/v1/verifications/:id` | One Verifier run (full) |
| GET  | `/v1/sessions/:id/checkpoints` | List checkpoints |
| POST | `/v1/sessions/:id/checkpoints/:checkpoint_id/rollback` | Admin-only rollback |
| GET  | `/v1/sessions/:id/feature-list` | Proxy read of workspace `feature-list.json` |
| GET  | `/v1/sessions/:id/progress` | Proxy read of `progress.txt` (last 200 lines default) |

WebSocket: same envelope; new `Message.ui:"narrate"` plus new `Misc.kind`
values (see architecture.md §3.4).

### 3.12 Frontend additions

- **Narration lane** — Chat panel renders `Message{ui:"narrate"}` in a
  lighter weight than user/assistant turns.
- **Verifier verdict pane** — AgentComputer gains a tab/strip filtering
  `Misc{kind:"verifier_*"}` events; click → opens evidence list.
- **3-track BrowserTab** — replaces the Phase 0 noVNC-only BrowserTab.
  Track A iframe (existing), Track B scrollable DOM-text pane, Track C
  horizontal screenshot strip (clickable for fullsize).

### 3.13 Phase 0 DEBT items closed in Phase 1

| Phase 0 DEBT # | What | Closed by story |
|---|---|---|
| #18 | SandboxClient handle-cache single-process assumption | 1.2 |
| #21 | Hook output-truncation inline-preview fallback | 1.14 |
| #22 | Capability table assumes Bifrost cloud aliases support tool calling | 1.7 |
| #25 | Plan tools remain callable stubs | 1.1 |
| #27 | WS `task_pause` / `task_resume` / `task_cancel` are protocol stubs | 1.17 |

Phase 0 DEBT items NOT closed in Phase 1: #1, #7, #8, #15, #16, #26 — see
phase-1/DEBT.md item 13.

---

## 4. Story breakdown

Each story is 1-3 hours, one PR, one commit, story-relative acceptance
criteria, executable by any AGENTS.md-compatible agent from a fresh session.
Detailed specs at `/specs/phase-1/stories/story-1.X.md`.

| ID | Story title | Est | Deps | Closes |
|---|---|---|---|---|
| 1.1  | Real Plan Manager (plan_create/advance/update wired; structured sticky render) | 3h | — | Phase 0 DEBT #25 |
| 1.2  | SandboxClient handle-cache rehydration | 1.5h | — | Phase 0 DEBT #18 |
| 1.3  | Sandbox workspace bootstrap (`git init` + identity + initial empty commit) | 2h | 1.2 | — |
| 1.4  | Initializer + feature-list.json + progress.txt + 2 new LLM tools | 3h | 1.1, 1.3 | — |
| 1.5  | Tool-mask layer (PRINCIPLE #2) | 2h | 1.4 | — |
| 1.6  | Context Recitation (PRINCIPLE #4) | 2h | 1.4 | — |
| 1.7  | Bifrost alias → provider model-ID resolution + capability fallback | 2h | — | Phase 0 DEBT #22 |
| 1.8  | Verifier slot startup gate (verifier ≠ main resolved-model-ID) | 1h | 1.7 | — |
| 1.9  | Verifier DB layer + V004 migration + `verifications` table + read routes | 2h | 1.8 | — |
| 1.9b | Verifier Worker runtime — Redis Streams + concurrency + watchdog | 3h | 1.9 | — |
| 1.10 | TaskComplete trigger + VERIFYING state + verdict handling | 3h | 1.9b, 1.1 | — |
| 1.11 | Invalidation Detector + Invalidation trigger | 2h | 1.9b | — |
| 1.12 | Circuit Breaker unification (4 conditions) + CircuitBreaker trigger + Diversity Injector | 3h | 1.10, 1.11 | — |
| 1.13 | Checkpoint Manager — V005 + commit-on-advance + `checkpoint_label` | 2h | 1.10, 1.3 | — |
| 1.13b | Checkpoint rollback — internal tool + admin endpoint + opt-in Verifier path | 2.5h | 1.13, 1.5 | — |
| 1.14 | Hook output-truncation → sandbox file-ref path | 1.5h | 1.3 | Phase 0 DEBT #21 |
| 1.15 | Narrator Hook (templated + classifier-slot LLM path) | 2h | 1.5, 1.14 | — |
| 1.16 | 3-track Browser representation — backend (PostBrowserAction + DOM/screenshot/file_ref) | 2h | 1.14 | — |
| 1.17 | WS `task_pause` / `task_resume` / `task_cancel` real | 2h | 1.10, 1.9b | Phase 0 DEBT #27 |
| 1.18 | Frontend: narration lane + Verifier verdict pane | 2h | 1.9, 1.10, 1.15 | — |
| 1.19 | Frontend: 3-track BrowserTab (A/B/C) | 2.5h | 1.16 | — |
| 1.20 | Phase 1 E2E + acceptance fixture (corpus only, gated live-LLM smoke) + retrospective + DEBT audit | 3-4h | all | — |

**Total**: 22 stories, ~50 h, fits 4-week timebox at ~3 h/day with Codex
pair workflow. Parallelisable seams: {1.5, 1.6, 1.14, 1.17} after 1.4 lands;
{1.11, 1.13} after 1.10; {1.13b} after 1.13 + 1.5.

---

## 5. Acceptance criteria

Phase 1 is done when:

```
✓ A 50-step synthetic task runs to completion without Stuck/MaxSteps/Cost termination
✓ The Verifier fires exactly once on each successful task (TaskComplete trigger)
  with verdict `pass`
✓ A GAIA Level 1-style 10-task fixture set has ≥8 passes
✓ feature-list.json + progress.txt are present and consistent at end of every task
✓ Plan sticky-context render stays under 1000 tokens
✓ Server refuses to start when verifier slot resolves to the same model ID as main
✓ Frontend renders narration / verdict / 3-track tabs without console errors
✓ `just verify` passes all gates
✓ Phase 0 DEBT items #18, #21, #22, #25, #27 closed
✓ phase-1/DEBT.md keeps an accurate ledger of new shortcuts
```

---

## 6. Deferred (NOT in Phase 1)

- ❌ Briefing protocol, deliverable templates (Phase 2)
- ❌ Pause/resume across days, async notifications (Phase 2)
- ❌ Project/Task/Subtask data model (Phase 2)
- ❌ Learning system, playbook auto-extraction (Phase 3)
- ❌ Curator, self-improvement (Phase 4)
- ❌ Multi-user, organization, real auth (Phase 5)
- ❌ Polished docs, one-command deploy (Phase 6)
- ❌ Automatic rollback default-on (`Phase 1` ships mechanism only, opt-in)
- ❌ Frontend automated tests (Phase 2 — flagged in phase-1/DEBT.md #9)
- ❌ Egress allowlist deny-default (Phase 5 — config surface only in Phase 1)

---

## 7. Risks

| Risk | Mitigation |
|---|---|
| Verifier provider outage blocks completion | Default `verifier.fail_open=false` per PRINCIPLE #10; visible block beats silent pass. `phase-1/DEBT.md` #12 tracks. |
| Verifier false positives drift the plan | Verifier-driven mutations tagged `source:"verifier"` in Plan events; audit trail differentiates from agent-driven. |
| Invalidation false-positives from out-of-band edits | One heuristic (file SHA mismatch) + allow-list; verdict-as-noise acceptable if rare. `phase-1/DEBT.md` #4 tracks. |
| Sandbox git commits fail (disk full) | Misc `checkpoint_create{ok:false}`; session continues; rollback unavailable for that phase. |
| Cost cap reached during deep tasks | Default cap raised $1 → $2; still configurable per task; Cost breaker is hard. |
| Test debt grows on new UI surfaces | `phase-1/DEBT.md` #9 explicitly flags frontend automated test deferral to Phase 2. |
| Phase 1 introduces new tools, widening the architecture.md tool-count drift | `phase-1/DEBT.md` #1 schedules a doc-only ADR-011 / ARCHITECTURE.md v1.1 — not in this phase. |

---

## 8. Dependencies

- Phase 0 stack (Bifrost, Redis, SQLite, sandbox).
- AIO Sandbox image must include `git` (already present in pinned
  `ghcr.io/agent-infra/sandbox:1.0.0.152`).
- Two Rust crate additions: `sha2 = "0.10"`. No `git2` (git invoked via
  sandbox shell — architecture.md §5.1).
- No new frontend dependencies (all UI built from existing primitives).
- No new external services / containers.

---

## 9. Open questions (carried from architecture.md §12)

These are story-level details, resolved as each story lands:

1. Verifier evidence_event_ids — pre-fetch vs lazy. **Decision: lazy** (story 1.18).
2. Narration filter at sticky-context boundary — story 1.4 / 1.15 detail.
3. Diversity injector variant source — Rust const array in Phase 1 (story 1.12); DB-promotable in Phase 4.
4. Multi-verifier-slot-per-trigger — deferred to Phase 4 (`phase-1/DEBT.md` #5).
5. Sandbox git identity — `seasoned-hand@local` / `Seasoned Hand` (story 1.3).
6. feature_mark_done across phases — allowed; emit Misc `feature_done_out_of_phase` (story 1.4).
7. Screenshot retention — full-resolution, cleanup tied to Phase 0 DEBT #16 (deferred).
8. Verifier evidence anchoring after plan_update — event IDs stable, anchoring valid (story 1.10).

---

## 10. Spec references

- `/specs/01-architecture/ARCHITECTURE.md` — immutable overall (§4 agent
  loop, §5 context principles, §6 verification layers L1-L4, §7 tool catalog).
- `/specs/phase-1/architecture.md` — Phase 1 specifics (this document's
  source of truth).
- `/specs/phase-1/DEBT.md` — Phase 1 debt seed (12 items at architecture
  boundary).
- `/specs/phase-0/RETROSPECTIVE.md` — what worked / what to fix from Phase 0.
- `/specs/06-roadmap/ROADMAP.md` §"Phase 1" — original deliverable list.
- ADR-007 (conservative learning), ADR-010 (Plan as PCB), ADR-003 (12-slot routing).
- PRINCIPLES.md #2 (one tool per iteration), #4 (recitation), #10
  (failure-tolerant), #16 (RAM/disk), #17 (plan stickiness).
