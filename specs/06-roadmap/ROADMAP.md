# ROADMAP

> 22-week build plan. 6 phases. Each phase has clear deliverables and
> acceptance criteria. Update phase status in this file as work completes.

---

## Overview

```
Phase 0 ─────────┐   Foundation skeleton (3w)
Phase 1 ─────────┐   Manus 5-layer execution (4w)
Phase 2 ─────────┐   Employee interface (3w)
Phase 3 ─────────┐   Learning system (4w)  ← LEARNING STARTS
Phase 4 ─────────┐   Curator + self-improvement (3w)
Phase 5 ─────────┐   Multi-user + organization (3w)
Phase 6 ─────────┘   Open source release (2w)

Total: 22 weeks ≈ 5 months
```

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

## Phase 3 — Learning System (4 weeks) — LEARNING STARTS — CURRENT (kickoff pending; BMAD Architect persona on `/specs/phase-3/architecture.md`)

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

## Phase 4 — Curator + Self-Improvement (3 weeks)

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

## Phase 5 — Multi-User + Organization (3 weeks)

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

## Phase 6 — Open Source Release (2 weeks)

**Goal**: External users can adopt without hand-holding.

**Deliverables**:
- Polished docs (English + Korean)
- One-command Docker Compose deploy
- Scenario-specific config examples (cloud, hybrid, fully-local)
- Demo videos / GIFs
- Migration guide (from other agent platforms)
- CI/CD + auto-release pipeline
- Community channel (Discord or GitHub Discussions)
- LICENSE, CONTRIBUTING, CODE_OF_CONDUCT, SECURITY all polished

**Acceptance**: A new developer can install and run in under 30 minutes.

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
