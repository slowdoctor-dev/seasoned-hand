# Phase 1 — Technical Debt Ledger

> Append-only list of shortcuts, stubs, simplifications, and deferred
> work introduced during Phase 1. Same discipline as Phase 0 DEBT.md.
>
> Seeded at architecture phase boundary (2026-05-12). Items added during
> story implementation get appended below the seed block.

## Closeout audit (story 1.20b, 2026-05-13)

All 13 seed items remain **open** at Phase 1 close. Each is scheduled
to a specific later phase or a follow-up commit (see individual
`Pay down` lines below). The closeout commit (story 1.20b) did not
flip any item to `resolved` — every Phase 1 implementation either
held the simplification deliberately (DEBT #3, #4, #5, #7, #8, #11,
#12) or carried a doc-only fix outside the phase-close blast radius
(#1, #2, #10). Phase 0 DEBT items closed in Phase 1 are tracked in
`specs/phase-0/DEBT.md` strike-throughs (#18 / #21 / #22 / #25 / #27).

In-phase additions from stories 1.1–1.20 surface as **execution
notes** inside each story file (story-1.10, 1.15, 1.16, 1.18 carry
divergence + deferred-plumbing notes that have not graduated to
formal DEBT entries because they are either documented behavior
choices or one-commit follow-ups, not load-bearing shortcuts). If
any of these need to be tracked as real debt — most likely
**classifier-slot wiring through `AppState::new`** (story-1.15 exec
notes) — append them below as item 14+ when the decision is made.

---

## Seed (from architecture.md, 2026-05-12)

### 1. ARCHITECTURE.md text drift — tool count "32" vs shipped 33→36
- **Origin**: inherited from Phase 0 (Phase 0 DEBT #4 resolved at the
  spec-check level but not in the immutable doc text); Phase 1
  architecture.md §0 and §4.3 widen the gap.
- **Severity**: **Low** (documentation only)
- **What**: `/specs/01-architecture/ARCHITECTURE.md` §2.4 and §7 say
  "32 tools". Phase 0 shipped 33 (counting `plan_advance` per ADR-010).
  Phase 1 adds `feature_mark_done`, `progress_update`, `checkpoint_label`
  → 36 LLM-visible tools, plus an internal-only `checkpoint_rollback`.
- **Why**: Editing ARCHITECTURE.md is gated (AGENTS.md §9 NEVER). The
  Phase 1 spec instead documents the divergence transparently and
  schedules a doc-only fix.
- **Pay down**: Draft a doc-only ADR-011 ("Tool catalog count is
  derived, not pinned") or bump ARCHITECTURE.md to v1.1 with the
  catalog described as a formula (29 Manus + 3 learning + ADR-010 plan
  exposed + Phase-N additions). One commit, no code change.

### 2. ARCHITECTURE.md text drift — "Next.js 15" vs shipped Next.js 16
- **Origin**: Phase 0 DEBT #27, inherited unchanged.
- **Severity**: **Low** (documentation only)
- **What**: ARCHITECTURE.md §1 / §5.3 and BASELINE.md §4 say
  "Next.js 15". Frontend actually runs Next.js 16 / React 19.2 /
  Tailwind 4.3.
- **Pay down**: Same doc-only commit as item 1 above.

### 3. Verifier rollback is opt-in (default off)
- **Origin**: architecture.md §2.6
- **Severity**: **Medium**
- **What**: Checkpoint Manager has the *mechanism* for `git revert` on
  Verifier fail, but the default config sets automatic rollback OFF.
  Verdict failures emit Misc + SUSPEND rather than rewinding.
- **Why**: Silent rollback of agent work before the Verifier's
  precision is validated risks worse drift than no rollback. Admin
  endpoint covers the deliberate-rollback case.
- **Pay down**: After Phase 1 retrospective collects Verifier
  precision numbers from real tasks, decide whether to flip the
  default in Phase 2. If precision >90%, flip; else keep opt-in.

### 4. Invalidation heuristic = file content hash mismatch only
- **Origin**: architecture.md §2.4.2
- **Severity**: **Medium**
- **What**: The Invalidation Detector ships ONE heuristic: file SHA-256
  mismatch when re-read via a non-allow-listed tool path. Other forms
  of "new data invalidates earlier work" (web-page content drift, shell
  output contradicting an asserted fact, plan-phase status conflict)
  are not detected.
- **Why**: One heuristic with a clean allow-list is testable. Layering
  more detectors before any have been validated against real tasks
  would multiply the false-positive surface.
- **Pay down**: Add additional invalidation heuristics in Phase 2 or
  Phase 4 driven by retrospective false-negative data (tasks where the
  Verifier should have fired mid-task and didn't).

### 5. Single verifier slot for all 3 triggers
- **Origin**: architecture.md §12 open question 4
- **Severity**: **Low**
- **What**: ARCHITECTURE.md §3 leaves room for distinct verifier model
  instances per trigger type (e.g. a cheaper model for Invalidation
  triggers, a stronger one for TaskComplete). Phase 1 uses one
  `verifier` slot for all three.
- **Why**: Cost-versus-precision tradeoff is unmeasured. Premature
  optimization without data.
- **Pay down**: Phase 4 Curator analysis once verification cost +
  precision data exists.

### 6. Egress allowlist: config surface only, deny default deferred
- **Origin**: architecture.md §9
- **Severity**: **Medium**
- **What**: Phase 1 adds the `sandbox.egress_allowlist` config flag and
  the plumbing to enforce it, but the shipped default is `["*"]`
  (permissive — current behavior). The deny default lands in Phase 5
  with multi-user.
- **Why**: Flipping the default to deny before any user has populated
  their allowlist would break every Phase 1 task. We need the config
  surface in place so the Phase 5 flip is a default change, not a
  schema change.
- **Pay down**: Phase 5 flips default; documents migration in
  CHANGELOG.

### 7. Diversity Injector variants are a Rust constant array
- **Origin**: architecture.md §12 open question 3
- **Severity**: **Low**
- **What**: The 4 phrasing variants for stuck-tracker strategy-change
  prompts live in a constant array in
  `seasoned-hand-core::agent::diversity`. No mechanism for users or
  Curator to add variants.
- **Pay down**: Phase 4 — Curator may promote variants to a DB table
  for per-org tuning. Phase 1 deliberately ships the simplest thing.

### 8. PostBrowserAction screenshots are full-resolution, no cleanup
- **Origin**: architecture.md §12 open question 7
- **Severity**: **Low**
- **What**: Track C screenshots are stored at full noVNC resolution
  (~200-400 KB each). 50-step browser-heavy tasks accrue ~15 MB of
  PNGs in the workspace. No thumbnailing, no per-track retention
  policy; cleanup folded into the still-pending Phase 0 DEBT #16
  workspace TTL story.
- **Pay down**: Tied to Phase 0 DEBT #16 (workspace TTL + cleanup
  cron). Whichever phase lands #16 should also add Track C-specific
  retention.

### 9. Frontend has no automated test coverage
- **Origin**: inherited from Phase 0 retrospective; Phase 1 adds
  three new UI surfaces (narration lane, verdict pane, 3-track
  BrowserTab) without test infrastructure.
- **Severity**: **Medium**
- **What**: Phase 0 closed with manual-smoke-only frontend. Phase 1
  adds significantly more UI without addressing this. Risk:
  regressions in the new UI surfaces go undetected until E2E.
- **Pay down**: Phase 2 — bring up Playwright or Vitest+RTL, write
  baseline coverage for narration filter, verdict rendering, and
  3-track strip. Phase 1 itself does NOT pay this down.

### 10. Verifier evidence_event_ids resolution is lazy / story-local
- **Origin**: architecture.md §12 open question 1
- **Severity**: **Low**
- **What**: The verdict pane fetches evidence events on click, not
  proactively. Slow under poor network; acceptable in localhost-only
  Phase 1.
- **Pay down**: Phase 5 when multi-user / remote sessions matter.

### 11. Sandbox `git` identity hardcoded
- **Origin**: architecture.md §12 open question 5
- **Severity**: **Low**
- **What**: All Phase 1 checkpoint commits use `user.email =
  seasoned-hand@local` and `user.name = Seasoned Hand`. No per-user
  attribution.
- **Pay down**: Phase 5 (multi-user) populates from the authenticated
  user's profile.

### 12. Verifier failure on Bifrost 5xx is fail-closed by default
- **Origin**: architecture.md §8
- **Severity**: **Low**
- **What**: If the Verifier slot's Bifrost call returns 5xx, the
  verdict is treated as `fail{reason:"verifier_unavailable"}`. Config
  `verifier.fail_open=true` exists to override but defaults to closed.
  Risk: an outage in the verifier provider blocks task completion
  even when the agent's work is correct.
- **Why**: Per PRINCIPLE #10 (failure-tolerant, never failure-hiding),
  presuming pass on a verifier outage is worse than a visible block.
- **Pay down**: Revisit if Phase 1 retrospective shows verifier
  provider outages are a real availability risk.

### 13. Phase 0 DEBT items NOT paid down by Phase 1
- **Origin**: architecture.md §6 — explicit Phase 1 pay-down list
- **Severity**: **n/a** (informational)
- **What**: Phase 0 DEBT items intentionally NOT addressed in Phase 1:
  - **#1** `DbPool` single-writer — Phase 5 (multi-user)
  - **#7** WebSocket auth — Phase 5
  - **#8** Bifrost auth — Phase 5
  - **#9** Bifrost smoke without keys — Phase 1 hardening (optional)
  - **#10** `$HOME/.cargo` env-sourcing — already handled in justfile
    by story 0.27; informational
  - **#11** Cost polling vs push — depends on Bifrost upstream
  - **#15** Sandbox seccomp tightening — Phase 1 considers but defers
  - **#16** Workspace TTL + cleanup — still pending; merges with item
    8 above
  - **#26** Bifrost per-request cost attribution — depends on Bifrost
    upstream
- **Pay down**: Each linked to its target phase above.

---

## In-phase additions (from stories 1.1–1.20b)

### 15. Verifier Worker `run()` is a polling stub — no `XREADGROUP` consumer
- **Origin**: story 1.9b, surfaced by the post-Phase-1
  story-consistency audit (1.20b close)
- **Severity**: **High** (blocks the Verifier feature in any
  deployment where the producer-side `XADD verify_request` calls go
  through Redis Streams — which is every production deployment)
- **What**: Story 1.9b's acceptance criteria specify
  `XREADGROUP GROUP verifier <consumer> BLOCK 5000 COUNT 16 STREAMS
  verify_request >` + per-session FIFO via `DashMap<SessionId,
  Arc<Mutex<()>>>` + global concurrency cap via `Semaphore`.
  `Worker::run()` ships as a polling shim that calls
  `ensure_consumer_group()` (a no-op) and then loops on a 500 ms
  `tokio::time::sleep`. It does no Redis I/O. The `Semaphore` /
  session-locks plumbing was dropped during the post-Phase-1
  simplicity pass (`6bbb34e`) because nothing was reading from it.
  Net: the production verifier loop reads zero entries from the
  stream; verdicts only flow when host code calls
  `Worker::handle_request(&req)` directly, which only test code
  currently does. Every trigger emission path (idle-final, breaker
  fire, file-mismatch hook) successfully `XADD`s into the stream,
  but those entries accumulate forever.
- **Why it shipped this way**: Story 1.9b's intent was to lock down
  the `handle_request` pipeline (fresh context, FAIL-biased prompt,
  watchdog, parse + persist) so that wiring the consumer loop is a
  pure plumbing job with no algorithm risk. The `phase1_stable_50step`
  E2E (story 1.20) drives `handle_request` directly to prove the
  shape end-to-end. The trade-off was deliberate but never marked as
  DEBT.
- **Pay down**: First post-Phase-1 commit on the verifier-runtime
  surface. Implement the XREADGROUP loop + parse + dispatch +
  XACK + the per-session FIFO + global semaphore exactly per
  story 1.9b's "Concurrency" section. Add the two named tests
  (`worker_respects_per_session_fifo`,
  `worker_respects_global_concurrency_cap`) under a live-Redis
  feature flag (mirroring the existing `#[ignore]`'d Redis pattern
  from Phase 0). The shipped `handle_request` is unchanged.

### 14. `SandboxGitShell::commit_phase` builds a shell string with weak quoting (`phase_title` injection)
- **Origin**: story 1.13, surfaced by the post-Phase-1 security review
  (story 1.20b commit `e5948c2`)
- **Severity**: **Medium** (latent — not currently reachable in shipped
  code; becomes exploitable the moment the Plan{op:"advance"}
  broadcaster lands)
- **What**:
  `crates/seasoned-hand-core/src/checkpoint/git_in_sandbox.rs:110-114`
  builds the sandbox `git commit -m "<phase_title>"` shell command via
  `format!` with only `replace('"', "\\\"")` escaping. The double-quoted
  argument leaves `` ` `` (command substitution), `$` (variable
  expansion), and `\` open. `phase_title` is LLM-controlled via the
  `plan_update` / `plan_advance` tool args → `Phase.title`.
- **Why it isn't a current vuln**: The only caller path
  (`CheckpointManager::handle_plan_advance`) has no live wiring —
  `main.rs:108-132` spawns `CheckpointManager::run`, a polling no-op
  stub. The Plan{op:"advance"} broadcaster is documented as deferred
  ("real fanout lands alongside the global event bus in story 1.20
  E2E") and story 1.20 only added tests, not the broadcaster.
- **Why it stays open**: Whoever wires the broadcaster will reach this
  code path on the first real `plan_advance`. The fix is small but
  must land BEFORE the broadcaster.
- **Pay down**: Replace the manual escape with one of:
  - `git commit -F -` reading the title from stdin via
    `/v1/shell/exec`'s stdin field (preferred; no quoting needed),
  - `shell_escape::escape(...)` on `phase_title`,
  - or move to a real argv path (`/v1/shell/exec` accepting `argv` rather
    than a shell string) and stop building shell strings here altogether.
  Add a regression test that feeds `` "`whoami`" ``, `$(id)`, and a
  newline through `phase_title` and asserts the resulting sandbox
  command does not execute them. Block the broadcaster commit on this
  landing.

---

## Categories quick-reference (same as Phase 0)

| Severity | Meaning |
|---|---|
| **H** | Blocks the next phase's goals if not addressed |
| **M** | Will bite at scale or in a year, manageable today |
| **L** | Documentation / minor friction / one-line fix later |
