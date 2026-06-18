# ROADMAP

> 22-week build plan. 6 phases. Each phase has clear deliverables and
> acceptance criteria. Update phase status in this file as work completes.

---

## Overview

```
Phase 0 ─────────┐   Foundation skeleton (3w)        ✅
Phase 1 ─────────┐   Manus 5-layer execution (4w)     ✅
Phase 2 ─────────┐   Employee interface (3w)          ✅
Phase 3 ─────────┐   Learning system (4w)             ✅  ← LEARNING STARTED
Phase 4 ─────────┐   Curator + self-improvement (3w)  ✅
Phase 5 ─────────┐   Multi-user + organization (3w)   ✅
Phase 6 ─────────┘   Open source release + Dioxus     ✅  ← v1 SHIPPED (v0.6.0)

Total: 22 weeks ≈ 5 months
```

> **v1 shipped — Phase 6 complete (2026-06-18, `v0.6.0`).** The open-source
> release landed: the Next.js→Dioxus cutover (ADR-016), one-command Docker
> deploy, CI/CD auto-release (GitHub Release + GHCR image), config + migration
> docs, the #22/#23 security/hardening buckets, the invitation-token org-binding
> fix, a **sealed performance track**, and full doc reconciliation. The parallel
> release-readiness checklist (below) is green. All six phases are now ✅; further
> work is tracked under "Beyond v1".

---

## Phase 0 — Foundation (3 weeks) — ✅ Complete (2026-05-12, 27 stories, see `/specs/phase-0/RETROSPECTIVE.md`)

**Goal**: One-line task delegation → result. Working skeleton.

**Deliverables**:
- Bifrost gateway running with 3+ model aliases
- Rust control plane (Axum + Tokio + Rig)
- Event Stream (SQLite + Redis pub/sub)
- 32 tool catalog + dispatcher
- AIO Sandbox per session (bollard)
- 12-slot model router with capability detection
- Next.js frontend with 3-panel UI (TaskList / Chat / AgentComputer)
- WebSocket for live event streaming
- noVNC iframe, xterm.js terminal, Monaco editor read-only

**Acceptance**: 
- `just up` brings up full stack
- "Find OpenManus GitHub stars" → agent navigates → returns answer in chat
- Live narration visible in all panels
- Session persists across restart
- Cost tracking works
- `just verify` passes all gates

**Status**: Planning complete. 27 stories identified in
`/specs/phase-0/requirements.md`.

---

## Phase 1 — Manus 5-Layer (4 weeks) — ✅ Complete (2026-05-13, 23 stories, see `/specs/phase-1/RETROSPECTIVE.md`)

**Goal**: Deep task completion. 50+ tool calls per task without falling apart.

**Deliverables**:
- Initializer + Worker pattern (Anthropic-style harness)
- `feature-list.json` + `progress.txt` persistence
- Context engineering 6 principles enforced:
  1. KV-cache friendly stable prefix
  2. No mid-iteration tool catalog changes
  3. Filesystem as memory
  4. Todo recitation
  5. Errors preserved in context
  6. Diversity injection
- Verifier service (different model, bias toward FAIL)
- Circuit breaker (max turns, cost cap, loop detection)
- Git checkpoint + rollback
- 3-track browser representation
- Live narration via PreToolUse-equivalent hook

**Acceptance**: GAIA Level 1-style tasks succeed ≥80%. 50+ tool call
sessions stable.

---

## Phase 2 — Employee Interface (3 weeks → 5 in spec; actual 3 days under parallel-mode) — ✅ Complete (2026-05-15, 27 stories, see `/specs/phase-2/RETROSPECTIVE.md`)

**Goal**: Feels like a digital employee, not a chatbot.

**Deliverables**:
- Project / Task / Subtask data model
- Briefing protocol (delegate → AI interprets → confirms → executes)
- Deliverable standards (output template, citation)
- Status reporting dashboard
- Accountability trail (decision provenance)
- Long-running tasks (24h+ stable)
- Pause/resume (Docker pause + event stream restore)
- Async notifications (BullMQ + ntfy or similar)

**Acceptance**: "Do this overnight" workflow works end-to-end.

---

## Phase 3 — Learning System (4 weeks) — LEARNING STARTED — ✅ Complete (2026-05-18, see CHANGELOG `[0.3.0]`)

**Goal**: Same task type is faster the second time. Time-axis benefit
visible.

**Deliverables**:
- 4-layer learning data model:
  - SOPs (explicit org rules, versioned)
  - Playbooks (auto-extracted from verified work)
  - Project history (FTS5 searchable)
  - Glossary (org terminology)
- Conservative learning trigger (ADR-007)
- Playbook auto-extraction pipeline
- Playbook matching (new task → similar playbooks)
- Playbook injection at task start (Initializer context)
- Session search via FTS5 + LLM summarization

**Acceptance**: A type of task, on the second run, completes with 30%
fewer tool calls.

---

## Phase 4 — Curator + Self-Improvement (3 weeks) — ✅ Complete (2026-05-19, 22 stories, see CHANGELOG `[0.4.0]`)

**Goal**: System manages its own learning artifacts.

**Deliverables**:
- Curator background worker (Tokio task)
- Playbook success rate tracking
- Duplicate consolidation
- Auto-archive of stale/low-success playbooks
- Skill self-improvement (patch during use)
- SOP conflict detection + alerts
- User work-pattern modeling
- Weekly retrospective auto-generation

**Acceptance**: After one month of use, playbook library auto-improves
without manual curation.

---

## Phase 5 — Multi-User + Organization (3 weeks) — ✅ Complete (2026-05-21, 33 stories, see CHANGELOG `[0.5.0]`)

**Goal**: Multiple users share one Seasoned Hand instance.

**Deliverables**:
- Multi-tenant data model (organization → user)
- Role-based access (admin, user, viewer)
- SOPs shared across users within an org
- Playbooks shareable
- Task hand-off (one user → another)
- Audit log (who delegated what)
- Per-user cost tracking

**Acceptance**: 5-person team uses one instance without stepping on each
other.

---

## Release-readiness checklist (gated the `v0.6.0` public-release tag) — ✅ green

**Goal**: Lock the Phase 0–5 product into a known-good, fully-reconciled state
before the project is tagged for public release. Reconciled and green for
`v0.6.0` (2026-06-18).

**Checklist**:

- [x] **Performance track sealed** — iter-3 Codex confirm found one per-event
      hot-path issue (P4, fixed); iter-4 came back clean on both halves → track
      sealed, 4 fixes total (`specs/PERFORMANCE_REVIEW.md`).
- [x] **Test gate runnable on a Docker host** — `cargo test --workspace` is green
      in CI for all non-`#[ignore]`d suites; the Docker/Redis-dependent tiers run
      via `just test-docker-host` (`scripts/test-docker-host.sh`) on a machine
      with a Docker daemon.
- [x] **Doc reconciliation** — no spec drift: ROADMAP / README / BASELINE markers
      + ADR count + bundle paths reconciled (#48, #51, #54); ARCHITECTURE version
      consistent.
- [x] **Hardening tracks all sealed** — security + maintainability + performance
      all sealed; the #22/#23 buckets closed; no new DEBT carry-ins.
- [x] **Open decisions resolved or explicitly deferred** — BASELINE §8: multi-tenant
      DB + auth resolved; cloud sandbox provider, telemetry opt-in, and community
      channel explicitly **deferred post-v1** (#50, #54).

---

## Phase 6 — Open Source Release + Dioxus migration — ✅ Complete (v0.6.0, 2026-06-18)

**Status**: Shipped as `v0.6.0` (GitHub Release + GHCR image
`ghcr.io/slowdoctor-dev/seasoned-hand:v0.6.0`). Started 2026-06-05 with the
Dioxus frontend migration (ADR-016); the Next.js stack was removed in the
cutover and the control plane now self-hosts the unified-Rust wasm UI.

**Goal**: External users can adopt without hand-holding.

**Deliverables**:
- [x] Polished docs (English + Korean) — config reference + migration guide (#49)
- [x] One-command Docker Compose deploy — multi-stage image, self-served UI (#48)
- [x] Scenario config examples (local / hybrid / cloud) — `docs/configuration.md` (#49)
- [ ] Demo videos / GIFs — _follow-up (non-blocking)_
- [x] Migration guide — `docs/migration-guide.md` (#49)
- [x] CI/CD + auto-release pipeline — `release.yml` (tag → release + GHCR) (#48)
- [~] Community channel — **deferred post-v1** (issues-only for now) (#54)
- [x] LICENSE, CONTRIBUTING, CODE_OF_CONDUCT, SECURITY — audited + polished (#49)

**Acceptance**: A new developer can install and run in under 30 minutes
(`git clone … && docker compose up -d`).

**Open follow-ups (non-blocking, tracked beyond this milestone)**: demo media;
running `just test-docker-host` on a Docker host to exercise the `#[ignore]`d
sandbox suites; the post-v1 deferrals (cloud sandbox provider, telemetry,
community channel).

---

## Beyond v1 (post-Phase 6)

**Not yet committed**, but plausible:

- Mobile companion app (read-only, status check, approvals)
- Voice interface (Whisper + TTS auxiliary slots)
- Plugin system (community playbook registry, opt-in)
- Distributed deployment (multi-region)
- Fine-tuned domain models (verified-work as training data)
- Enterprise features (SSO, advanced audit, SLAs)

These are explicitly **not** in scope for v1. See `NON_GOALS.md`.

---

## Time accounting

- Total: 22 weeks
- Solo full-time: 5-6 months
- Solo evenings/weekends: 10-12 months
- Two people pairing: 3-4 months
- Slack/realistic: add 20% buffer

Track real velocity in phase retrospectives:
`/specs/phase-N/retrospective.md`.

---

## How to update this file

When a phase completes:

1. Move `CURRENT` marker to next phase
2. Add completion notes inline ("Completed YYYY-MM-DD, story count N")
3. Link to retrospective
4. Update BASELINE.md § 6 if scope shifted significantly
5. Commit with `docs(roadmap): close Phase N` message
