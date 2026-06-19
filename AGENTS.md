# Agent Instructions

> **Source of truth** for all AI coding agents.
> This project is **LLM-agnostic**: Claude Code, Codex CLI, Cursor, Cline,
> Windsurf, Aider, and any AGENTS.md-compatible tool should all produce
> equivalent results when working from this file.
>
> File loading:
> - **Codex CLI**: reads AGENTS.md automatically
> - **Cursor 0.50+**: reads AGENTS.md natively
> - **Claude Code**: imports AGENTS.md via CLAUDE.md (one-line import)
> - **Others**: most agents recognize AGENTS.md; if not, point them here
>
> Keep under 300 lines — focused signal beats sprawling noise.
> Codex silently truncates beyond `project_doc_max_bytes`.

---

## 1. Project context

**Seasoned Hand** — open-source autonomous agent platform.

Tagline: *Every task makes the hand wiser.*

Combines:
- **Manus-grade execution** — deep task completion (50+ tool calls per task)
- **Hermes-grade learning** — skills/memory persisting across sessions

OS metaphor: kernel = agent runtime (Rust + Rig), syscalls = 38 tools,
filesystem = event stream, user programs = playbooks (learned from verified work).

## 2. Stack (immutable)

| Layer | Choice |
|---|---|
| LLM Gateway | Bifrost (Go) — 50x faster than LiteLLM |
| Control plane | Rust + Axum + Tokio + Rig |
| Frontend | Dioxus (unified Rust → Web/Desktop/Mobile) — ADR-016 amends ADR-002; the Next.js 15 + React 19 stack was removed in the Phase 6 cutover (#5) |
| Sandbox | AIO Sandbox (Docker) per session |
| Persistence | SQLite WAL + Redis |
| Model routing | 12-slot (3 main + 9 auxiliary, Hermes-inspired) |

Full architecture: `/specs/01-architecture/ARCHITECTURE.md`

## 3. Methodology

**Spec-Driven Development**. Full doc: `/docs/methodology.md`.

- BMAD personas at phase boundaries (Analyst → Architect → PM)
- GSD workflow daily (Discuss → Plan → Execute → Verify)
- Stories = 1-3 hours of work
- Fresh context per story (no carryover)
- All state in `/specs/` markdown files (git-versioned)

## 4. File map

```
/BASELINE.md           ← single-entry-point session starter (READ FIRST)
/AGENTS.md             ← THIS FILE (source of truth)
/CLAUDE.md             ← imports AGENTS.md + Claude-specific
/CHANGELOG.md          ← version history
/GLOSSARY.md           ← project terminology
/specs/
  /00-philosophy/      ← VISION, PRINCIPLES, NON_GOALS
  /01-architecture/
    ARCHITECTURE.md    ← overall (immutable)
    /decisions/        ← ADR-001 to ADR-NNN
  /06-roadmap/
    ROADMAP.md
  /07-research/        ← external interviews
  /phase-N/
    requirements.md
    /stories/story-N.X.md
/crates/               ← Rust workspace: core, server, cli, dto + ui (Dioxus, ADR-016)
/docs/                 ← human docs
/prompts/              ← BMAD/GSD session prompts
/scripts/, /justfile
```

## 5. Per-story workflow (mandatory)

### Startup (every session)
1. Read this file (`/AGENTS.md`)
2. Read `/specs/01-architecture/ARCHITECTURE.md`
3. Read the specific story file
4. Read files referenced by the story

### 4-phase workflow
1. **Discuss** — surface unclear points
2. **Plan** — output structured plan, wait for OK
3. **Execute** — implement matching spec exactly
4. **Verify** — `just verify`, all gates pass
5. **Commit** — one story = one commit

## 6. Verification gates (must pass before done)

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
just check-ui   # UI crate (excluded from workspace): fmt + clippy + wasm check
./scripts/spec-check.sh
```

Or: `just verify`

## 7. Code style

**Rust**: edition 2024, zero clippy warnings, `thiserror` for errors, no `unwrap()` in production, no `unsafe` without justification comment.

**Dioxus UI** (`crates/seasoned-hand-ui`, ADR-016): edition 2021 (no let-chains), wasm32 target, components are `#[component]` fns, share wire types via `seasoned-hand-dto` (never hand-mirror them).

**Markdown specs**: one H1 per file, ATX headers, code blocks with language tags, wrap at 100 chars.

**Commits**:
```
feat(phase-N): story X.Y - brief description

- what changed
- why

refs: /specs/phase-N/stories/story-X.Y.md
```

## 8. Spec compliance

If implementation must diverge from spec:
1. STOP implementation
2. Update the spec in same PR
3. Note divergence in commit message
4. Continue matching new spec

Never let code and spec drift silently.

---

## 9. NEVER

- Carry context between stories
- Invent features not in the spec
- Skip verification gates
- Depend on a specific AI tool's features in production code (must work across Claude Code, Codex, Cursor, etc.)
- Modify `/AGENTS.md`, `/CLAUDE.md`, or `/specs/01-architecture/ARCHITECTURE.md` without approval
- Add dependencies without updating `/specs/01-architecture/ARCHITECTURE.md`
- Use `git push --force` on shared branches
- Disable a test to pass CI
- Add a TODO without filing an issue
- Make architectural decisions silently

## 10. ALWAYS

- Ask if spec is unclear (don't guess)
- State 2-3 options when ambiguity exists
- Run tests after each meaningful change
- Quote exact spec sections when justifying choices
- Update specs when discovering missing requirements
- Roll back when a hack would be needed to pass tests

---

## 11. When stuck

State:
1. Which spec section you're implementing
2. Exact conflict or unclear point
3. 2-3 options for resolution

Then wait. Don't decide silently.

## 12. Sub-agent delegation

For stories spanning 5+ files:
1. Spawn isolated sub-agent per sub-task
2. Each sub-agent: load its spec, do its thing, return result
3. Don't carry full conversation across sub-tasks

This mirrors how Seasoned Hand itself works (recursive principle).

---

## 13. Current state

- **Phase**: 6 complete — **v1 shipped as `v0.6.0` (2026-06-18)**. Open-source
  release + Dioxus cutover (ADR-016) done (Next.js removed in #5); one-command
  Docker deploy + CI/CD auto-release (GitHub Release + GHCR image) live; the
  performance track is sealed and the #22/#23 hardening buckets are closed.
- **Branch**: main
- **Next (Beyond v1)**: non-blocking follow-ups — demo media; running
  `just test-docker-host` on a Docker host to exercise the `#[ignore]`d sandbox
  suites; the post-v1 deferrals (default cloud sandbox provider, telemetry
  opt-in, community channel — BASELINE §8); marketplace-style artifact exchange;
  and the dogfood-driven retunes for FTS5 weights (DEBT #76 successor) and
  curator adaptive policies (DEBT #92 / #94 successors).

Check status: `just status` (or `git log --oneline` + `CHANGELOG.md`)

## 14. References

- `/BASELINE.md` — single entry point (read first)
- `/CLAUDE.md` — Claude-specific additions
- `/specs/00-philosophy/` — VISION, PRINCIPLES, NON_GOALS
- `/specs/01-architecture/ARCHITECTURE.md` — immutable system architecture (v1.5)
- `/specs/01-architecture/decisions/` — ADR-001 to ADR-018
- `/specs/06-roadmap/ROADMAP.md` — 6-phase plan
- `/specs/REVIEW.md` — pre-Phase-3 cross-phase hardening review
- `/docs/methodology.md` — full methodology
- `/docs/getting-started.md` — human onboarding
- `/docs/manifesto.md` — why this project exists
- `/docs/brand.md` — visual identity
- `/GLOSSARY.md` — project terminology
