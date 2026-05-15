# BASELINE — Seasoned Hand

> **Single source of truth.** Read this first when starting a new session.
> All decisions accumulated through planning. Everything else derives from this.
>
> *Every task makes the hand wiser.*

---

## 1. Project Identity

| Field | Value |
|---|---|
| **Name** | Seasoned Hand |
| **Repo** | `github.com/slowdoctor-dev/seasoned-hand` (개인 계정, Public, MIT) |
| **License** | MIT |
| **Tagline** | Every task makes the hand wiser. |
| **Status** | Phase 2 complete → Phase 3 starting |
| **Domain** | General-purpose autonomous AI agent platform (no domain assumptions) |
| **Audience** | Developers + business users (balanced) |
| **Philosophy** | Digital Employee (not assistant) |

## 2. Vision (One Sentence)

> An open-source autonomous AI agent platform that combines Manus-grade
> execution depth with Hermes-grade time-axis learning, self-hosted and
> model-agnostic, so users can hire an AI employee that gets seasoned by
> the work they delegate.

## 3. Core Insight (Why This Project Exists)

Two breakthroughs of 2025-26 sit on different axes:

- **Manus** proved AI can finish hard tasks (50+ tool calls per task)
- **Hermes** proved AI can remember work (skills persist across sessions)

No system yet combines both — depth AND time. That's the empty quadrant
we're filling. See `/specs/00-philosophy/VISION.md` for full reasoning.

## 4. Architecture Summary

OS metaphor: kernel = agent runtime, user-space = learning artifacts.

**Architecturally an OS-layer for work**: pluggable channel adapters
(intake / delivery / notify, all symmetric — anything that delivers
work in is matched by something that delivers results back out),
structured work representation (Project → Task → Session →
Deliverable → Decision as first-class persisted entities), mandatory
provenance trail (every deliverable traces back to evidence event IDs
+ decisions + verifier verdicts + checkpoints), persistent skills /
playbooks (Phase 3+), multi-tenant-ready schema (every row carries a
nullable `tenant_id`; Phase 5 flips to NOT NULL). Phase 2 lands the
OS-shape foundation (see `/specs/phase-2/architecture.md`).

| Layer | Choice | Rationale (ADR) |
|---|---|---|
| LLM Gateway | Bifrost (Go) | ADR-001: 50x faster than LiteLLM, single binary |
| Control plane | Rust + Axum + Tokio + Rig | ADR-002: memory predictability, true concurrency |
| Frontend | Next.js 15 + Tailwind v4 + React 19 | (no dedicated ADR — frontend half of ADR-002 hybrid; UI stack in ARCHITECTURE.md §1) |
| Sandbox | AIO Sandbox (Docker per session) | ADR-004: isolation per task |
| Persistence | SQLite WAL + Redis | ADR-005: SQLite for events, Redis for pub/sub |
| Model routing | 12-slot (3 main + 9 auxiliary) | ADR-003: Hermes-inspired |
| Agent tool source-of-truth | AGENTS.md universal | ADR-006: LLM-agnostic via AGENTS.md |
| Conservative learning | Verified work only | ADR-007: only verified work feeds playbooks |
| License | MIT, public from day 0 | ADR-008: open from start |
| Map / fan-out tool | Deferred to Phase 4+ | ADR-009: depth + learning first |
| Tool catalog | 32+ (29 Manus + 3 learning) | (no dedicated ADR — catalogue in ARCHITECTURE.md §7) |
| Event stream | 8 types append-only | (no dedicated ADR — schema in ARCHITECTURE.md §2.1, append-only in PRINCIPLES #3) |
| Plan Manager | Structured PCB-style plan | ADR-010: prevents goal drift, sticky context |
| Verification | 4-layer framework | (deterministic / cross-source / observation / meta-cognition) |

Full architecture: `/specs/01-architecture/ARCHITECTURE.md`
Decision records: `/specs/01-architecture/decisions/`

## 5. Methodology

**Spec-Driven Development** (BMAD + GSD hybrid).

Workflow:
1. **Phase boundary** — BMAD personas (Analyst → Architect → PM) write specs
2. **Daily** — GSD workflow (Discuss → Plan → Execute → Verify)
3. **Per story** — fresh AI session, 1-3 hours, 1 PR, 1 commit
4. **Spec compliance** — code matches spec; if diverged, update spec in same PR

Tools (LLM-agnostic): Claude Code, Codex CLI, Cursor, or any AGENTS.md-compatible tool.

Full methodology: `/docs/methodology.md`

## 6. 6-Phase Roadmap

| Phase | Weeks | Deliverable | Learning |
|---|---|---|---|
| 0 | 3 | Foundation: working skeleton | (none) |
| 1 | 4 | Manus 5-layer (deep execution) | (none) |
| 2 | 5 (actual: 3 days under parallel-mode compression) | Employee interface (briefing, deliverables) | (none) |
| 3 | 4 | 4-layer learning system | **starts** |
| 4 | 3 | Curator + self-improvement | (matures) |
| 5 | 3 | Multi-user + organization | (scales) |
| 6 | 2 | Open source release | (ships) |

Total: **22 weeks ≈ 5 months** (planned). Phase 0/1/2 actuals to date:
3 days + ~2 days + 3 days under Claude + Codex parallel-mode.

Full roadmap: `/specs/06-roadmap/ROADMAP.md`

## 7. Hard Decisions (Immutable Without ADR Update)

These are set. Changing requires new ADR + version bump in `/specs/01-architecture/ARCHITECTURE.md`:

1. **Open source from day one** (MIT, public repo)
2. **Model-agnostic** (12-slot routing via Bifrost)
3. **Self-hostable** (no SaaS dependencies in core)
4. **Domain-neutral core** (no medical/legal/specific verticals in MVP)
5. **Rust backend + TypeScript frontend** (not unified language)
6. **One tool per agent loop iteration** (HARD constraint, Manus principle)
7. **Conservative learning** (only verified work, not all interactions)
8. **English code/comments, Korean docs welcome** (bilingual project)
9. **Context = RAM, sandbox files = disk** (long-task accuracy depends on this)
10. **Plan is sticky PCB** (structured, in every iteration context, never free text)

## 8. Open Decisions (TBD before Phase 0 ends)

- [ ] Multi-tenant DB strategy: separate vs shared with user_id
- [ ] Auth: API key, OAuth, or both
- [ ] Default cloud sandbox provider for users without local Docker
- [ ] Opt-in telemetry: format, scope, privacy policy
- [ ] GitHub repo: stay personal vs move to org (later)

## 9. Repository Map

```
/BASELINE.md           ← THIS FILE (read first)
/AGENTS.md             ← AI agent source of truth
/CLAUDE.md             ← Claude Code import wrapper
/README.md             ← external entry point
/LICENSE               ← MIT
/CHANGELOG.md          ← version history (KeepAChangelog)
/GLOSSARY.md           ← project terminology
/CONTRIBUTING.md
/CODE_OF_CONDUCT.md
/SECURITY.md
/docker-compose.yml
/justfile
/.env.example, /.gitignore, /.codex/, /.github/

/specs/
  /00-philosophy/      ← why and what we believe
    VISION.md, PRINCIPLES.md, NON_GOALS.md
  /01-architecture/    ← how it's built
    ARCHITECTURE.md
    /decisions/        ← ADR-001 to ADR-NNN
  /06-roadmap/         ← when and what's next
    ROADMAP.md
  /07-research/        ← external interviews, market analysis
    manus-direct-qa.md
    manus-plan-tool-spec.md
    manus-map-tool-spec.md
  /phase-0/            ← current phase
    requirements.md
    /stories/          ← story-0.1.md to story-0.27.md
  /phase-N/            ← future phases added here

/docs/
  manifesto.md, brand.md, methodology.md,
  getting-started.md, using-claude-and-codex.md,
  kickoff.md, setup-checklist.md

/prompts/              ← BMAD/GSD session prompts
/scripts/              ← spec-check.sh, status.sh
```

## 10. Where to Start

**Brand new AI session?** Read these in order:
1. This file (`BASELINE.md`)
2. `/AGENTS.md`
3. `/specs/01-architecture/ARCHITECTURE.md`
4. Whatever specific story or task you're working on

**Brand new contributor (human)?** Read these in order:
1. `README.md`
2. `BASELINE.md` (this file)
3. `/docs/manifesto.md` (why)
4. `/docs/getting-started.md` (how)
5. `/docs/first-week-plan.md` (first 7 days)

**Starting Phase N work?**
1. `/specs/06-roadmap/ROADMAP.md` (where we are)
2. `/specs/phase-N/requirements.md` (what to build)
3. BMAD Architect persona → `/specs/phase-N/architecture.md` (how)
4. BMAD PM persona → `/specs/phase-N/stories/*.md` (units of work)
5. GSD workflow → implement story by story

## 11. Key Inspirations (Credit Where Due)

- **Manus** (Butterfly Effect) — autonomous task completion, depth
- **Hermes Agent** (Nous Research) — persistent learning, model-agnostic
- **OpenManus** (FoundationAgents) — reference ReAct implementation
- **BMAD-METHOD** — spec-driven development methodology
- **GSD (Get Shit Done)** — Claude Code workflow patterns
- **GitHub Spec Kit** — spec-driven dev standardization

## 11.5. External validation (Manus direct Q&A)

We interviewed Manus directly about its operation and limits. Key findings:

- **OS metaphor is precise** — Manus self-describes using the same 5 OS concepts we use
- **Sequential-by-default is correct** — "AI agents are prone to cascading errors"
- **Filesystem as memory is critical** — "sandbox is my hard drive, context window is RAM"
- **Cross-session learning is missing in Manus** — confirms our differentiation
- **Manus's own wishlist (global knowledge graph, proactive clarification, faster
  inner loop) matches 3 of our designed features**

Direct Manus quote on its key limitation:
> "I am like a brilliant consultant who walks into your office every morning
> with total amnesia of yesterday's meeting."

This is what Seasoned Hand fills. Validated externally.

Full interview transcripts and analysis: `/specs/07-research/manus-*.md`

---

## 12. This File's Purpose

`BASELINE.md` exists to defeat **context rot** — the gradual loss of project
context across many AI sessions, summarizations, and compactions. When in
doubt, when starting fresh, when an AI seems confused, point to this file.

Everything important is here in compressed form. Detailed files exist
elsewhere; this is the index and the agreement.

If something here conflicts with another file in this repo, **this file
wins** until reconciled. Either update this file, or update the other.
Never let them silently diverge.

---

*An operating system for autonomous work that learns from experience.*
