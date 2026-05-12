# Seasoned Hand

> **Every task makes the hand wiser.**

An open-source autonomous agent platform.
Deep task execution + learning that persists across sessions.
Self-hosted, model-agnostic, MIT-licensed.

---

## What this is

A digital employee, not a chatbot.

- **Executes deep tasks** — 50+ tool calls per task (Manus-style)
- **Learns from work** — auto-extracts playbooks from verified outputs (Hermes-style)
- **Runs anywhere** — $5 VPS, NAS, laptop, or cloud
- **Any LLM** — 12-slot model routing through Bifrost gateway

It's a hand that gets seasoned by the work you give it.

## Status

**Phase -1** — Planning complete. Phase 0 (foundation) starting.

See [`/specs/01-architecture/ARCHITECTURE.md`](specs/01-architecture/ARCHITECTURE.md) for the full design.

## Architecture (one paragraph)

Think of it as an operating system. The **kernel** is a Manus-style agent runtime (Rust + Rig + Tokio). **Syscalls** are 32+ tools (file, shell, browser, search, deploy). **User programs** are playbooks — procedures the agent extracts from verified work and reuses next time. A **curator** runs in the background, consolidating and cleaning learning artifacts. **Memory** persists across sessions in SQLite (FTS5 searchable). Tasks run in isolated Docker sandboxes.

## Quick start (not yet — Phase 0 in progress)

```bash
git clone https://github.com/slowdoctor-dev/seasoned-hand
cd seasoned-hand
cp .env.example .env  # fill in API keys
docker compose up
open http://localhost:3000
```

## Development methodology

**Spec-Driven Development**. See [`/docs/methodology.md`](docs/methodology.md).

- BMAD personas at phase boundaries
- GSD workflow daily (Discuss → Plan → Execute → Verify)
- Stories = 1-3 hours of work
- Fresh context per story
- All design lives in `/specs/` markdown files

## Repository structure

```
/BASELINE.md           ← single-entry-point (read first)
/AGENTS.md             ← source of truth for AI agents
/CLAUDE.md             ← imports AGENTS.md + Claude-specific
/CHANGELOG.md          ← version history
/GLOSSARY.md           ← project terminology
/specs/                ← living specifications
  /00-philosophy/      ← VISION, PRINCIPLES, NON_GOALS
  /01-architecture/
    ARCHITECTURE.md    ← overall (immutable)
    /decisions/        ← ADR-001 to ADR-008
  /06-roadmap/
    ROADMAP.md
  /07-research/        ← external interviews (e.g., Manus direct Q&A)
  /phase-N/            ← current and future phases
    requirements.md
    stories/
/docs/                 ← human docs (methodology, brand, manifesto, kickoff)
/prompts/              ← BMAD/GSD session prompts
/src/                  ← Rust backend (Phase 0+)
/frontend/             ← Next.js frontend (Phase 0+)
/bifrost/              ← Bifrost gateway config
/scripts/              ← dev scripts
/justfile              ← task runner
/docker-compose.yml
```

## Inspiration

- **Manus** (Butterfly Effect) — autonomous task completion, depth
- **Hermes Agent** (Nous Research) — persistent learning, model-agnostic
- **OpenManus** (FoundationAgents) — reference ReAct implementation
- **BMAD-METHOD** — spec-driven development methodology
- **GSD (Get Shit Done)** — AI coding agent workflow

## Phase roadmap

| Phase | Weeks | Deliverable |
|---|---|---|
| 0 | 3 | Foundation: working skeleton, one-line task → result |
| 1 | 4 | Manus 5-layer: deep task completion with verification |
| 2 | 3 | Employee interface: briefing, deliverables, accountability |
| 3 | 4 | Learning system: playbook extraction, FTS5 search |
| 4 | 3 | Curator: auto-maintenance of learning artifacts |
| 5 | 3 | Multi-user: organization, shared SOPs |
| 6 | 2 | Open source release |

## AI tool compatibility

This project is **LLM-agnostic**. Source of truth is `AGENTS.md`. Use any AI coding tool you prefer:

| Tool | Reads AGENTS.md? | Notes |
|---|---|---|
| Claude Code | via CLAUDE.md import | Plan mode + subagents work well for multi-file stories |
| Codex CLI | yes, automatically | Built-in sandbox; use `--profile story` for implementation |
| Cursor 0.50+ | yes, natively | GUI editing experience |
| Cline (VS Code) | yes | Lightweight alternative |
| Aider | partial | Point it at AGENTS.md explicitly |

Switching tools mid-project is fine. Each story runs in a fresh session anyway. Some users keep multiple tools for different tasks (e.g., Claude Code for implementation, Codex for sandboxed experiments).

See [`docs/using-claude-and-codex.md`](docs/using-claude-and-codex.md) for patterns when using multiple tools.

## How to contribute

When public, pick a story from `/specs/phase-N/stories/`, follow the GSD workflow in `/prompts/gsd-execute-story.md`, one PR per story.

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for details.

## License

[MIT](LICENSE) — same as Hermes Agent. No strings.

---

## 한국어

**Every task makes the hand wiser** — 매 작업이 손을 더 영리하게.

오픈소스 자율 에이전트 플랫폼입니다. 한 작업을 끝까지 해내는 깊이(Manus 결)와 시간이 지날수록 자라는 학습(Hermes 결)을 합성. 자체 호스팅 가능, 모델 무관, MIT 라이선스.

자세한 설계는 [`/specs/01-architecture/ARCHITECTURE.md`](specs/01-architecture/ARCHITECTURE.md)에서.
