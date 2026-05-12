# Phase 0 — Retrospective

> Phase 0 shipped 2026-05-12. 27 stories, ~hours of wall time, two
> agents (Claude Code + Codex CLI) in parallel tmux panes.

## What shipped

All 27 stories committed and pushed to `origin/main`:

| Story | Title | Commit |
|---|---|---|
| 0.1  | Bifrost Docker setup | `6cdac9d` |
| 0.2  | Rust workspace + Axum healthz | `4f5633d` |
| 0.3  | SQLite schema + WAL migrations | `c1c236b` |
| 0.4  | Event stream API (append + query) | `467e2f9` |
| 0.5  | Redis pub/sub for subscribe | `885e67f` |
| 0.6  | Tool trait + 5 simplest tools | `433fe41` |
| 0.7  | Register all 33 tools (5 real + 28 stubs) | `7a10d9b` |
| 0.8  | AIO Sandbox bollard lifecycle client | `3fb5ba7` |
| 0.9  | Tool dispatcher + 4 representative real tools | `23e01d5` |
| 0.10 | EventEmittingHook (Action + Observation) | `4fe69ff` |
| 0.11 | OpenAI-compatible LLM client over Bifrost | `95f1742` |
| 0.12 | 12-slot model router + YAML config | `6d3d054` |
| 0.13 | Capability auto-detection at startup | `f7eb06b` |
| 0.14 | Agent runner ReAct loop | `eca90c4` |
| 0.15 | Stuck detection (real pump) | `4cb6524` |
| 0.16 | Cost cap via Bifrost /cost polling | `2458fad` |
| 0.17 | WebSocket server + envelope protocol | `58a10f4` |
| 0.18 | Next.js 16 + Tailwind v4 + TS strict | `52795c1` |
| 0.19 | 3-panel resizable layout | `c1999ec` |
| 0.20 | WebSocket client + reconnection | `1533048` |
| 0.21 | Chat component | `02cab2b` |
| 0.22 | TaskList component | `395afe5` |
| 0.23 | AgentComputer tabs scaffold | `1c97071` |
| 0.24 | noVNC iframe + session detail route | `765ff65` |
| 0.25 | xterm.js terminal (read-only) | `a28239c` |
| 0.26 | Monaco editor + file tree (read-only) | `6147dbb` |
| 0.27 | Phase 0 E2E + cleanup + retrospective | (this commit) |

**Goal achieved**: `requirements.md` §5 — the WebSocket-driven flow exists,
backed by a working agent loop, dispatcher, slot router, sandbox client,
hooks, and a 3-panel Next.js UI.

## Deferred to Phase 1+

`specs/phase-0/DEBT.md` lists the full set. Headlines by severity:

- **High @ Phase 5**: WebSocket auth, Bifrost auth (Phase 0 = localhost-only)
- **Medium**:
  - 18 of 22 sandbox tools still stubs (DEBT #19) — biggest single item
  - DbPool single-writer (Arc<Mutex<Connection>>) — Phase 1 swap
  - CI workflow not validated against new structure (DEBT #14)
  - Sandbox seccomp=unconfined security tradeoff
  - SandboxClient in-process handle cache (single-process assumption)
  - No frontend automated tests (manual smoke only)
- **Low**: ~15 items including Next 16 vs spec's Next 15, cost-poll
  polling instead of push, etc.

## What worked

1. **Spec-driven development held up.** Every story had a written spec
   *before* implementation. When implementation drifted (e.g. story 0.5's
   `EventStore::subscribe` moved to `RedisPool::subscribe`), the spec
   was updated in the same PR, and a DEBT entry tracked the conscious
   deviation. The spec, not the conversation, was the source of truth.
2. **LLM-agnostic via AGENTS.md**: Codex CLI and Claude Code both
   contributed without any tool-specific extension in production code.
3. **Two-agent parallelism**: Codex implementing backend stories while
   Claude wrote subsequent specs (and handled git ops where Codex's
   sandbox lacked DNS). Roughly halved sequential wall time on the
   middle backend stories.
4. **Solo fallback**: When Codex hit rate limits / model capacity, Claude
   continued solo against the existing specs without coordination
   overhead. Codex reviewed completed work later.
5. **Append-only DEBT.md ledger**: every new shortcut documented in the
   same commit that introduced it. By Phase 0 close: 27 entries, several
   already resolved with strike-through + date. No silent debt.
6. **`tool_choice="required"` + first-tool-only enforcement** (PRINCIPLE
   #2): caught no real runtime violations during dev because the
   discipline lived at the dispatcher level, not at the prompt level.
7. **Bifrost as a thin gateway**: switching from gpt-5.5 (rate-limited)
   to gpt-5.3-codex mid-stream required zero control-plane changes.

## What to fix in Phase 1 (top 3)

1. **DEBT #19 — wire the remaining 18 sandbox tools.** The agent runner
   can't actually browse or run shell commands end-to-end until the
   file_str_replace / shell_view / browser_* set is real. The
   `sandbox_post` helper in 0.9 already shows the pattern.
2. **DEBT #14 — CI workflow against the real workspace.** `.github/workflows/ci.yml`
   exists from the initial scaffold but has never been validated against
   the now-real Rust + Next.js setup. Add a workflow that runs
   `cargo clippy/fmt/test --workspace`, `pnpm typecheck/lint/build`,
   `scripts/spec-check.sh`, and the ignored Redis + sandbox lifecycle
   tests.
3. **DEBT #18 — SandboxClient handle-cache rehydration.** On startup,
   scan Docker for `seasoned-hand-sandbox-*` containers and re-populate
   the in-process cache. Currently a control-plane restart leaves
   orphan containers + a confused cache.

## Phase 1 starting point

- Branch off `main` at the story-0.27 commit
- Read `specs/06-roadmap/ROADMAP.md` §"Phase 1" for scope
- Pick up the 3 items above first; the rest of DEBT.md is naturally
  scheduled under the Phase 1 architecture work (Verifier, deep
  execution, the Manus 5-layer)

## Phase 0 by the numbers

- 27 stories shipped
- ~80 unit tests + several integration tests passing
- 27 DEBT items logged (a few resolved along the way)
- ~Rust crates: 2 (`seasoned-hand-core` lib + `seasoned-hand-server` bin)
- ~Frontend deps: Next.js 16, React 19, Tailwind v4, react-resizable-panels,
  @monaco-editor/react, @xterm/xterm
- 33-tool catalog (5 real + 28 stubs)
- 4 verification gates (clippy / fmt / test / spec-check)
