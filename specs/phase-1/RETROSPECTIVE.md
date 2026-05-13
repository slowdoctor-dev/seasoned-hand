# Phase 1 — Retrospective

> Phase 1 shipped 2026-05-13. 23 stories, ~12 hours of effective wall
> time, two agents (Claude Code + Codex CLI) in parallel tmux panes.

## What shipped

All 23 stories committed and pushed to `origin/main`:

| Story | Title | Commit |
|---|---|---|
| 1.1   | Real Plan Manager (plan_create/advance/update + structured sticky render) | `5c5dec9` |
| 1.2   | SandboxClient handle-cache rehydration | `c50db97` |
| 1.3   | Sandbox workspace `git init` + identity + initial empty commit | `1192162` |
| 1.4   | Initializer + feature-list.json + progress.txt + 2 new LLM tools | `bde7921` |
| 1.5   | Tool-mask layer (PRINCIPLE #2) | `992e6e6` |
| 1.6   | Context Recitation (PRINCIPLE #4) | `fd3d090` |
| 1.7   | Bifrost alias resolver + capability table | `fa3dacd` |
| 1.8   | Verifier slot startup gate (verifier ≠ main resolved-model-id) | `e0a8a04` |
| 1.9   | Verifier DB layer + V004 migration + `verifications` table + read routes | `720f8a0` |
| 1.9b  | Verifier Worker runtime — Redis Streams + concurrency + watchdog | `fe8086e` |
| 1.10  | TaskComplete trigger + VERIFYING state + verdict handling | `a4514b8` (+ `798c388`, `5019cb9`, `76b7a0e`, `4c09465`) |
| 1.11  | Invalidation Detector + Invalidation trigger | `142aeec` |
| 1.12  | Circuit Breaker unification (4 conditions) + CircuitBreaker trigger + Diversity Injector | `cab77d1` |
| 1.13  | Checkpoint Manager — V005 + commit-on-advance + `checkpoint_label` | `e5150b6` |
| 1.13b | Checkpoint rollback — internal tool + admin endpoint + opt-in Verifier path | `fc0ad84` |
| 1.14  | Hook output-truncation → sandbox file-ref path | `7132f16` |
| 1.15  | Narrator Hook (templated + classifier-slot LLM path) | `a1c3de8` |
| 1.16  | 3-track Browser representation — backend | `7cbbf88` |
| 1.17  | WS `task_pause` / `task_resume` / `task_cancel` real | `d95e264` |
| 1.18  | Frontend: narration lane + Verifier verdict pane | `096415b` |
| 1.19  | Frontend: 3-track BrowserTab (A/B/C) | `75d1c39` |
| 1.20  | Phase 1 E2E runtime verification (GAIA + 50-step + workflow_dispatch) | `fd11caf` |
| 1.20b | Phase 1 closeout (retrospective + DEBT audit + status flips) | (this commit) |

**Goal achieved**: `requirements.md` §5 — the Verifier fires on every
TaskComplete, the unified Circuit Breaker consults it before
terminating, plans are real (PCB-style structured sticky context),
checkpoints commit per phase advance, the 3-track Browser pipeline is
end-to-end, and the WS task controls execute real pause/resume/cancel
instead of acknowledging stubs. The default `cargo test --workspace`
path drives a deterministic ≥50-step scripted task to completion with
exactly one TaskComplete verifier_verdict / pass.

## Deferred to Phase 2+

`specs/phase-1/DEBT.md` is the source of truth. Headlines by severity:

- **Medium**:
  - **#3** Verifier rollback default opt-in (mechanism shipped; flip
    after Phase 1 precision data lands)
  - **#4** Single invalidation heuristic (file content hash mismatch
    only — add more when retro surfaces false negatives)
  - **#6** Egress allowlist defaults to `["*"]` (config surface
    plumbed; deny default lands with multi-user in Phase 5)
  - **#9** No frontend automated tests (Phase 2 brings up Playwright
    or Vitest+RTL)
- **Low**: ~10 items including tool-count drift in `ARCHITECTURE.md`
  text, lazy evidence-event resolution, Diversity Injector variants
  as Rust constants, hardcoded sandbox `git` identity, classifier-slot
  AppState plumbing deferred (1.15 ships templated-only at boot).

## What worked

1. **The Verifier-as-separate-Worker shape held.** Putting the
   verifier on its own Redis Stream + DB row + fresh-context build +
   FAIL-biased system prompt — instead of inlining it into the main
   loop — kept it independent enough that the same code paths drove
   TaskComplete (1.10), Invalidation (1.11), and CircuitBreaker (1.12)
   triggers with one match arm per kind. The §2.4.5 transition table
   stayed declarative.
2. **Parallel-stash pattern scaled.** Stories 1.10 / 1.13b / 1.16
   landed while Codex's 1.11 / 1.12 WIP sat in the working tree. Each
   time the seam was: snapshot Codex's modified files to
   `/tmp/codex-stash-<story>/`, revert just those, verify my isolated
   work, commit, restore Codex's WIP. Three episodes, zero accidental
   cross-commits.
3. **Codex rate-limit handoff.** When Codex hit its 5h limit
   mid-1.10, Claude took over against the existing spec and finished
   1.10 + 1.13 + 1.13b solo; Codex reviewed at recovery and picked up
   1.11 / 1.12 / 1.14 / 1.17 / 1.20 in the same session. No
   coordination overhead.
4. **Spec-divergence-as-execution-notes.** Every story whose
   implementation departed from its spec sketch documented the
   reasoning inline (story-1.10 verifier_active guard, 1.15
   side-channel Track B Misc events, 1.18 lifted WS hook,
   classifier-slot AppState deferral). The spec, not the
   conversation, stayed authoritative; the gap was visible at PR
   time.
5. **Security-review skill caught real things.** The skill found
   VerifierGate's cursor-reset re-replay risk on restart (fixed at
   `4c09465` with `verifier_gate_ack` markers) and a WS pong-echo
   flake. Both shipped before the phase closed.
6. **`Hook::pre`/`post`/`failure` from Phase 0 absorbed everything.**
   1.10 (TaskComplete trigger), 1.11 (Invalidation), 1.15 (Narrator
   pre-emit), 1.16 (PostBrowserAction) — four new behaviors, one
   trait, no signature changes. The "no new hook trait surface"
   discipline mattered.
7. **Sandbox = HTTP gateway, not just Docker.** Story 1.16 promoted
   `SandboxClient::browser_view` / `browser_screenshot` from one-off
   `sandbox_get` calls in `tools/builtin.rs` into typed methods on the
   client. The pattern (canonical accessor used by both the Phase 0
   tool and the Phase 1 hook) eliminated the "parallel HTTP path"
   class of bugs the spec warned against.

## What to fix in Phase 2 (top 3)

1. **DEBT #9 — Frontend automated tests.** Three new UI surfaces
   (narration lane, Verifier verdict pane, 3-track BrowserTab) shipped
   with manual-smoke only. Add Playwright (or Vitest+RTL) and write
   baseline coverage before Phase 2 layers on the briefing protocol
   UI.
2. **DEBT #3 — Verifier rollback default.** Phase 1 collected Verifier
   verdicts in production-shaped tests. Phase 2 should look at the
   real-task precision numbers (from the GAIA workflow_dispatch job +
   any user sessions in this period) and decide whether to flip the
   default from opt-in to opt-out. If precision >90%, flip.
3. **Classifier-slot wiring for NarratorHook through AppState::new.**
   1.15 deferred this — the hook is templated-only at boot because
   the dispatcher is `Arc`d before prompt loading. A small commit
   that either accepts `Option<ClassifierWiring>` in `AppState::new`
   or splits the dispatcher build into tools-only + tools+narrator
   phases unlocks the LLM-path narration the spec called for.

## Phase 1 starting point — Phase 2

- Branch off `main` at the story-1.20b commit.
- Read `specs/06-roadmap/ROADMAP.md` §"Phase 2" for scope (briefing
  protocol, deliverable templates, pause/resume across days, async
  notifications, Project/Task/Subtask model).
- Pick up the 3 items above as the Phase 2 warm-up batch; the rest
  of `phase-1/DEBT.md` is naturally scheduled under Phase 2-5
  architecture work.

## Phase 1 by the numbers

- **23 stories shipped** (20 + 1.9b + 1.13b + 1.20b)
- **234 tests passing** at `cargo test --workspace` (default path —
  no cloud credentials) — 211 core lib + 3 server lib + 20 server
  integration. Includes 6 narrate + 6 browser-tracks + 7 admin
  rollback + the 50-step deterministic E2E.
- **Phase 0 DEBT closed**: 5 items (#18 / #21 / #22 / #25 / #27)
- **Phase 1 DEBT logged**: 13 seed entries + in-phase additions
- **Tool catalog**: 37 (33 Phase-0 + `feature_mark_done` +
  `progress_update` + `checkpoint_label` + internal-only
  `checkpoint_rollback`)
- **Migrations**: V004 (verifications), V005 (checkpoints)
- **New event Misc kinds**: `verifier_request`, `verifier_verdict`,
  `verifier_gate_ack`, `feature_done`, `progress_recite`,
  `checkpoint_create`, `checkpoint_rollback`,
  `narration_skipped`, `browser_track_b`, `browser_track_b_skipped`,
  `browser_track_c`, `browser_track_c_skipped`
- **4 verification gates** still green (clippy / fmt / test /
  spec-check). spec-check tool-catalog count = 37.

## Corrections (post-close review)

_To be filled when Codex completes the post-close review (Phase 0
pattern: review at `fbb562f` surfaced 4 items → fixed in `3426780`)._
