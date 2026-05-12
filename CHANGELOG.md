# Changelog

All notable changes to Seasoned Hand will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

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

### Pending decisions
- Multi-tenant DB strategy
- Auth method (API key vs OAuth)
- Default cloud sandbox provider
- Telemetry opt-in approach
- `capabilities` flag integration with 12-slot routing (Phase 1 decision)

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
