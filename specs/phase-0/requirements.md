# Phase 0 — Foundation Infrastructure

> **Status**: v1.0 (planning)
> **Duration**: 3 weeks
> **Goal**: Working skeleton. One-line task delegation → result. Learning not yet.

---

## 1. Goals

By end of Phase 0:

1. User can submit a task via web UI
2. Agent loop executes, calls tools, produces output
3. User sees live progress in 3-panel UI
4. Session and events persisted in SQLite
5. At least one cloud LLM and one local LLM working through Bifrost
6. AIO Sandbox creates fresh container per session
7. WebSocket streams events to frontend in real-time

**Not in scope**: Verifier (Phase 1), learning (Phase 3), multi-user (Phase 5)

---

## 2. Non-functional requirements

| Requirement | Target |
|---|---|
| Cold start (first task) | <10 seconds |
| Subsequent task start | <2 seconds |
| Memory per session | <200MB |
| Idle memory (no sessions) | <100MB |
| WebSocket latency | <50ms |
| Tool call dispatch overhead | <20ms |
| One agent loop iteration | <1s + LLM time |

---

## 3. Functional requirements

### 3.1 LLM Gateway (Bifrost)

- [ ] Bifrost Docker container running on port 4000
- [ ] At least 3 model aliases configured:
  - `agent-primary` → cloud (Anthropic or OpenAI)
  - `agent-fallback` → different cloud
  - `local-fast` → Ollama on localhost
- [ ] OpenAI-compatible `/v1/chat/completions` endpoint reachable
- [ ] Cost tracking enabled (callback to local DB)
- [ ] Fallback chain works (kill primary → fallback used)

### 3.2 Control plane (Rust)

- [ ] Axum HTTP server on port 3000
- [ ] WebSocket endpoint at `/ws`
- [ ] SQLite database with migrations
- [ ] Redis connection pool
- [ ] Module loading and graceful shutdown

### 3.3 Event Stream

- [ ] Schema matches `/specs/01-architecture/ARCHITECTURE.md` section 2.1
- [ ] API: `append(event)`, `query(session_id, filter)`, `subscribe(session_id)`
- [ ] Subscribe via Redis pub/sub
- [ ] Append-only enforced (no UPDATE/DELETE)

### 3.4 Tool Catalog

- [ ] All 32 tools defined as Rust structs implementing `Tool` trait
- [ ] Each tool has: name, description, JSON Schema, dispatch target
- [ ] Tool registry exposes OpenAI-format schema for LLM

### 3.5 Tool Dispatcher

- [ ] Routes 32 tools to 5 backends:
  - File/Shell/Browser → AIO Sandbox HTTP API
  - Message → Frontend WebSocket
  - Search → external (Brave or Tavily)
  - Deploy → Tailscale Funnel (or skip in Phase 0)
  - Internal (sop/playbook/glossary) → SQLite
- [ ] Hooks: PreToolUse, PostToolUse, PostToolUseFailure
- [ ] Errors preserved in event stream

### 3.6 Agent Runner

- [ ] Loop: think → act → observe
- [ ] `tool_choice: required` enforced
- [ ] One tool per iteration enforced
- [ ] max_steps configurable per task
- [ ] Stuck detection (duplicate response counting)
- [ ] Cost cap per session ($1 default)

### 3.7 Sandbox (AIO Sandbox)

- [ ] One container per session via `bollard`
- [ ] Mount `/workspace` to persistent volume
- [ ] Container lifecycle: create on session start, pause on idle, destroy on session end
- [ ] HTTP API for all 32 tool calls
- [ ] noVNC available on port 6080
- [ ] xterm via ttyd on port 7681

### 3.8 Model Router (12-slot)

- [ ] Config file parsing (YAML)
- [ ] Each slot resolves to (provider, model, base_url, api_key)
- [ ] `auto` provider defaults to main
- [ ] `main` provider reuses main config
- [ ] `base_url` takes precedence over provider
- [ ] Capability auto-detection (call `/v1/models`)
- [ ] Reject startup if main slot lacks tool calling

### 3.9 Frontend (Next.js)

- [ ] 3-panel layout (react-resizable-panels)
- [ ] Left: TaskList (sessions list)
- [ ] Center: Chat (Message + ask UI)
- [ ] Right: AgentComputer tabs (Browser, Terminal, Editor, Files)
- [ ] WebSocket client connects to `/ws`
- [ ] Renders Message events
- [ ] Distinguishes notify vs ask UI

### 3.10 Browser tab (noVNC iframe)

- [ ] iframe loads sandbox's noVNC URL
- [ ] Auto-resize
- [ ] Takeover toggle (currently just info, full takeover in Phase 1)

### 3.11 Terminal tab

- [ ] xterm.js connected to sandbox ttyd
- [ ] Read-only by default (full interactive in Phase 1)

### 3.12 Editor tab

- [ ] Monaco editor
- [ ] Read-only view of workspace files
- [ ] File tree

---

## 4. Story breakdown

Each story is 1-3 hours. See `/specs/phase-0/stories/` for individual story specs.

| ID | Story title | Est. hours |
|---|---|---|
| 0.1 | Bifrost Docker setup + smoke test | 2 |
| 0.2 | Rust workspace initialization (Cargo + Axum hello) | 1 |
| 0.3 | SQLite schema + migrations (events, sessions) | 2 |
| 0.4 | Event Stream API (append/query) | 2 |
| 0.5 | Redis pub/sub for event subscribe | 2 |
| 0.6 | Tool trait + 5 simplest tools | 3 |
| 0.7 | Remaining 27 tools (in batches of ~5) | 8 |
| 0.8 | AIO Sandbox bollard integration | 3 |
| 0.9 | Tool dispatcher 5-way routing | 3 |
| 0.10 | Hooks (Pre/Post/Failure) | 2 |
| 0.11 | LLM client (OpenAI-compat over Bifrost) | 2 |
| 0.12 | Model router (slot resolution) | 3 |
| 0.13 | Capability auto-detection | 2 |
| 0.14 | Agent runner ReAct loop | 4 |
| 0.15 | Stuck detection | 1 |
| 0.16 | Cost cap | 1 |
| 0.17 | WebSocket server (Axum) | 2 |
| 0.18 | Next.js project init + Tailwind | 1 |
| 0.19 | 3-panel resizable layout | 2 |
| 0.20 | WebSocket client + reconnection | 2 |
| 0.21 | Chat component (notify rendering) | 3 |
| 0.22 | TaskList component | 2 |
| 0.23 | AgentComputer tabs scaffold | 2 |
| 0.24 | noVNC iframe integration | 2 |
| 0.25 | xterm.js + ttyd terminal | 3 |
| 0.26 | Monaco editor + file tree | 3 |
| 0.27 | Phase 0 integration test (E2E) | 4 |

**Total**: 27 stories, ~65 hours, ~3 weeks at 4 hrs/day.

---

## 5. Acceptance criteria

Phase 0 is done when:

```
✓ I can run `just up` and the whole stack starts (Bifrost, control plane, frontend)
✓ I can open http://localhost:3000 and see the 3-panel UI
✓ I can type "Find the GitHub stars of FoundationAgents/OpenManus" and submit
✓ Agent calls browser tool, navigates to GitHub, reads the number
✓ I see live narration in chat ("Opening browser...", "Navigating to GitHub...")
✓ I see the browser in the right panel via noVNC
✓ Final result appears as a message in chat
✓ Session is persisted; I can refresh and see history
✓ Cost is tracked and shown
✓ `just verify` passes all gates
```

---

## 6. Deferred (NOT in Phase 0)

- ❌ Verifier (Phase 1)
- ❌ Initializer + Worker pattern (Phase 1)
- ❌ Context engineering 6 principles full enforcement (Phase 1)
- ❌ Project/Task/Subtask data model (Phase 2)
- ❌ Briefing protocol (Phase 2)
- ❌ Learning system (Phase 3)
- ❌ Curator (Phase 4)
- ❌ Multi-tenant (Phase 5)
- ❌ Polished docs (Phase 6)

---

## 7. Risks

| Risk | Mitigation |
|---|---|
| AIO Sandbox setup more complex than expected | Time-box to 1 day; fallback to custom Docker image |
| Bollard API churn | Pin specific version |
| Bifrost local model latency | Verify Mac/Linux Ollama performance early |
| Rust learning curve | Story 0.2 includes Rust setup; allocate buffer |
| WebSocket reconnection complexity | Use proven library (socketio is JS-only; use raw + heartbeat) |

---

## 8. Dependencies

- Docker Desktop or compatible (for Bifrost, AIO Sandbox)
- Rust 1.78+ (2024 edition)
- Node 20+ + pnpm
- Redis (Docker)
- SQLite (bundled with rusqlite)

---

## 9. Open questions

- [ ] Use bollard 0.17+ or wait for 0.18?
- [ ] Pin AIO Sandbox to specific version or use `latest`?
- [ ] SSE or WebSocket for chat token streaming? (Decision: WebSocket — already in use)
- [ ] How to handle very large tool outputs (>100KB)? Spec: file ref in event stream.
