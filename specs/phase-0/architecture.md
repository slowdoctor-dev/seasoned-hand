# Phase 0 — Architecture

> **Status**: v1.1 (Codex review issues 1–11 closed)
> **Phase**: 0 (Foundation Infrastructure)
> **Bridges**: `/specs/01-architecture/ARCHITECTURE.md` (immutable overall) → `/specs/phase-0/stories/` (27 concrete stories)
> **Scope**: working skeleton. No Verifier, no learning, no multi-user, no Curator.

> **Changelog**:
> - v1.1 (this rev): clarified Bifrost contract (§4.4), Ollama optionality (§5.2),
>   Bifrost auth posture (§9), and Phase 0 vs Phase 1+ cost-tracking ownership.
>   Resolves all 11 spec gaps surfaced in Codex pre-flight review of story 0.1.
> - v1.0: initial.

This document specifies *how* Phase 0 implements the overall architecture.
It does not re-derive decisions captured in ADR-001..010. When a spec here
conflicts with ARCHITECTURE.md, ARCHITECTURE.md wins — file a divergence
note in the same PR.

---

## 1. Summary diagram

Phase 0 surface only. Components in [brackets] are designed-but-deferred —
referenced because the schema/wiring must stay forward-compatible.

```
┌──────────────────────────────────────────────────────────────────────┐
│  Frontend (Next.js 15, App Router, Tailwind v4, React 19)             │
│  - 3-panel layout (TaskList | Chat | AgentComputer)                   │
│  - WebSocket client at ws://localhost:3000/ws                         │
│  - noVNC <iframe> → http://localhost:6080                             │
│  - xterm.js → ttyd ws at ws://localhost:7681                          │
│  - Monaco (read-only) over workspace REST                             │
└────────────────────────┬─────────────────────────────────────────────┘
                         │ WebSocket + HTTP
                         ↓
┌──────────────────────────────────────────────────────────────────────┐
│  Control Plane (Rust workspace: seasoned-hand-core + -server)         │
│                                                                       │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │ API layer (axum 0.7)                                          │    │
│  │ HTTP: /healthz, /v1/sessions, /v1/sessions/:id/events         │    │
│  │ WS:   /ws  (envelope protocol — §4.2)                         │    │
│  └─────────────────────────────────────────────────────────────┘    │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │ Agent runtime                                                 │    │
│  │ - ReAct loop (rig 0.5 + custom)                               │    │
│  │ - tool_choice="required", one tool per iteration              │    │
│  │ - Stuck detection (2 duplicate outputs → strategy prompt)     │    │
│  │ - Hooks: PreToolUse / PostToolUse / PostToolUseFailure        │    │
│  │ - Cost cap per session ($1 default)                           │    │
│  └─────────────────────────────────────────────────────────────┘    │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │ Plan Manager (ADR-010)                                        │    │
│  │ - plan_create runs at task start (planner slot, pre-loop)     │    │
│  │ - plan_advance / plan_update exposed as tools                 │    │
│  │ - Sticky context: plan rendered at top of every iteration     │    │
│  │ - Persisted in `plans` table; pub/sub on update               │    │
│  └─────────────────────────────────────────────────────────────┘    │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │ Tool Dispatcher                                                │    │
│  │ - Registry (HashMap<&'static str, Arc<dyn Tool>>)             │    │
│  │ - 32 tools wired to 5 backends via ToolContext (§4.3)         │    │
│  │ - Emits Action + Observation events on every call             │    │
│  └─────────────────────────────────────────────────────────────┘    │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │ Event Stream                                                   │    │
│  │ - append-only writer over rusqlite (WAL mode)                  │    │
│  │ - query(session_id, type?, after_id?) → Vec<Event>            │    │
│  │ - subscribe(session_id) → Redis pub/sub channel                │    │
│  └─────────────────────────────────────────────────────────────┘    │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │ LLM Client                                                     │    │
│  │ - OpenAI-compatible client → Bifrost at /v1                    │    │
│  │ - 12-slot resolver (loads bifrost-mirror config from YAML)     │    │
│  │ - Capability detection at startup (GET /v1/models)             │    │
│  └─────────────────────────────────────────────────────────────┘    │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │ Sandbox Client (bollard 0.17)                                  │    │
│  │ - One AIO Sandbox container per session                        │    │
│  │ - Lifecycle: create on session start, pause on idle,           │    │
│  │   destroy on session end                                       │    │
│  │ - Talks to sandbox HTTP API for all sandbox-backed tools       │    │
│  └─────────────────────────────────────────────────────────────┘    │
│                                                                       │
│  Persistence: SQLite (rusqlite 0.31, WAL) + Redis (deadpool-redis     │
│  0.15, pub/sub only — Streams deferred)                                │
└────────────────────────┬─────────────────────────────────────────────┘
                         │ OpenAI-compatible HTTP
                         ↓
┌──────────────────────────────────────────────────────────────────────┐
│  Bifrost (Go, Docker) — http://localhost:4000/v1                       │
│  - Phase 0: 3 model aliases: agent-primary, agent-fallback, local-fast│
│  - Cost tracking enabled (read back via Bifrost API)                  │
└────────────────────────┬─────────────────────────────────────────────┘
                         │
       ┌─────────────────┼─────────────────┐
       ↓                 ↓                 ↓
   Anthropic         OpenAI            Ollama (local)
```

**Explicitly out of Phase 0** (designed in ARCHITECTURE.md, not built here):
Verifier slot enforcement, Curator, Initializer/Worker module pattern,
playbook/SOP/glossary tables, Redis Streams queue, the `map` tool (ADR-009),
multi-tenant DB strategy, auth, briefing protocol.

---

## 2. New components introduced

| # | Component | Crate | Tech | Integrates with |
|---|---|---|---|---|
| 1 | API layer | `seasoned-hand-server` | axum 0.7 + tokio-tungstenite | Frontend, Agent runtime |
| 2 | Agent runtime | `seasoned-hand-core::agent` | rig 0.5 + tokio 1.40 | LLM client, Tool dispatcher, Plan Manager, Event stream |
| 3 | Plan Manager | `seasoned-hand-core::plan` | rusqlite + serde_json | Agent runtime, Event stream, Redis pub/sub |
| 4 | Tool catalog (32 tools) | `seasoned-hand-core::tools` | static registry | Tool dispatcher |
| 5 | Tool dispatcher | `seasoned-hand-core::dispatch` | trait-object registry | Agent runtime, Sandbox client, Frontend WS, Search client, SQLite |
| 6 | Event stream | `seasoned-hand-core::events` | rusqlite WAL + Redis pub/sub | Everything that emits events; WS subscribers |
| 7 | LLM client | `seasoned-hand-core::llm` | reqwest + async-openai-compatible | Agent runtime, Plan Manager |
| 8 | 12-slot model router | `seasoned-hand-core::router` | serde_yaml config | LLM client |
| 9 | Sandbox client | `seasoned-hand-core::sandbox` | bollard 0.17 | Tool dispatcher (Sandbox backend) |
| 10 | Search client | `seasoned-hand-core::search` | reqwest (Brave or Tavily) | Tool dispatcher (Search backend) |
| 11 | Session store | `seasoned-hand-core::session` | rusqlite | API layer, Agent runtime |
| 12 | Config loader | `seasoned-hand-server::config` | serde + figment | Server bootstrap |
| 13 | Bifrost (external) | — | Go binary in Docker | LLM client |
| 14 | AIO Sandbox (external) | — | Docker image | Sandbox client |
| 15 | Frontend app | `frontend/` (Next.js 15) | React 19 + Tailwind v4 | API layer (WS + REST) |

**Workspace layout** (decision: 2-crate workspace):

```
/Cargo.toml                # workspace root
/crates/
  /seasoned-hand-core/     # library (all subsystems, no main())
    /src/
      lib.rs
      agent/               # ReAct loop, stuck detection, hooks
      plan/                # plan manager (ADR-010)
      events/              # append-only writer, query, subscribe
      tools/               # 32 tool impls + registry
      dispatch/            # dispatcher + ToolContext
      sandbox/             # bollard client + lifecycle
      search/              # Brave/Tavily client
      llm/                 # OpenAI-compatible client
      router/              # 12-slot resolver + capability detection
      session/             # session CRUD
      db/                  # migrations + connection pool
      error.rs             # thiserror-based crate-wide error type
  /seasoned-hand-server/   # binary
    /src/
      main.rs
      config.rs            # YAML config loading
      http.rs              # axum routes + middleware
      ws.rs                # WebSocket handler (envelope protocol)
/migrations/               # SQLite migrations (refinery or sqlx-migrate)
  001_sessions.sql
  002_events.sql
  003_plans.sql
/bifrost/
  config.yaml
  data/                    # gitignored
  README.md
/frontend/                 # Next.js 15 app
  /app/                    # App Router
  /components/
  /lib/                    # WS client, types (mirrored from Rust via ts-rs in Phase 1)
  package.json
  tsconfig.json
  tailwind.config.ts
/docker-compose.yml        # Bifrost + Redis only in Phase 0 (sandbox launched per-session via bollard)
/.env.example
/justfile                  # already present
/scripts/test-bifrost.sh   # story 0.1
```

`ts-rs` type sharing is **deferred to Phase 1**; Phase 0 frontend hand-writes
TypeScript types matching the WS envelope. Acceptable because the protocol
surface is small (8 event types + 5 commands).

---

## 3. Data model changes

Phase 0 introduces the **three tables required for the agent loop**.
Tables for learning artifacts (`sops`, `playbooks`, `playbooks_fts`,
`glossary`) are specified in ARCHITECTURE.md §2.5 but **deferred to Phase 3**.

### 3.1 `sessions`

Verbatim from ARCHITECTURE.md §2.2:

```sql
CREATE TABLE sessions (
  id            TEXT PRIMARY KEY,           -- UUID v4
  created_at    INTEGER NOT NULL,           -- unix epoch µs
  updated_at    INTEGER NOT NULL,
  state         TEXT NOT NULL CHECK(state IN
                  ('IDLE','RUNNING','FINISHED','ERROR','SUSPENDED')),
  project_id    TEXT,
  user_id       TEXT,                       -- NULL in Phase 0 (single-user)
  title         TEXT,
  cost_cents    INTEGER NOT NULL DEFAULT 0,
  tool_calls    INTEGER NOT NULL DEFAULT 0,
  metadata      TEXT                        -- JSON
);
CREATE INDEX idx_sessions_state ON sessions(state);
```

### 3.2 `events`

Verbatim from ARCHITECTURE.md §2.1:

```sql
CREATE TABLE events (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id    TEXT NOT NULL REFERENCES sessions(id),
  timestamp     INTEGER NOT NULL,           -- unix epoch µs
  type          TEXT NOT NULL CHECK(type IN
                  ('Message','Action','Observation','Plan',
                   'Knowledge','Datasource','Skill','Misc')),
  source        TEXT NOT NULL,              -- user|agent|planner|tool:<name>|sub_agent_N (forward-compat ADR-009)
  data          TEXT NOT NULL               -- JSON payload (see §3.4)
);
CREATE INDEX idx_events_session_time ON events(session_id, timestamp);
CREATE INDEX idx_events_type ON events(type);
```

**Append-only invariant** enforced in code (no SQL trigger needed):
`events::append` is the only public writer; no `update` or `delete` methods
on the `EventStore` trait. Test gate: clippy lint via custom `# [deny]` on
the module + a unit test that asserts the trait has no mutating methods.

### 3.3 `plans` (ADR-010)

Verbatim from ARCHITECTURE.md §2.3:

```sql
CREATE TABLE plans (
  id                TEXT PRIMARY KEY,        -- UUID v4
  session_id        TEXT NOT NULL REFERENCES sessions(id),
  goal              TEXT NOT NULL,
  phases            TEXT NOT NULL,           -- JSON array<Phase>
  current_phase_id  INTEGER NOT NULL,
  created_at        INTEGER NOT NULL,
  updated_at        INTEGER NOT NULL
);
CREATE INDEX idx_plans_session ON plans(session_id);
```

`phases` JSON shape (per ADR-010):

```json
[
  {
    "id": 1,
    "title": "Open GitHub repo page",
    "capabilities": ["browse"],
    "status": "active"
  },
  {
    "id": 2,
    "title": "Read star count",
    "capabilities": ["browse", "extract"],
    "status": "pending"
  }
]
```

Status values: `"pending" | "active" | "done" | "skipped"`.

### 3.4 Event `data` payloads (per type)

Compact reference; full schemas live in `seasoned-hand-core::events::types`.

| `type` | `data` shape (TypeScript-style) |
|---|---|
| `Message` | `{ role: "user"\|"assistant", content: string, ui: "notify"\|"ask"\|null }` |
| `Action` | `{ tool: string, args: object, call_id: string }` |
| `Observation` | `{ call_id: string, ok: boolean, output?: any, error?: { kind: string, message: string }, file_ref?: string }` |
| `Plan` | `{ op: "create"\|"advance"\|"update", plan_id: string, snapshot: Plan }` |
| `Knowledge` | reserved (Phase 3) |
| `Datasource` | reserved (Phase 3) |
| `Skill` | reserved (Phase 3) |
| `Misc` | `{ kind: string, ...any }` |

**Large output handling** (PRINCIPLE #16, resolves req §9 open question):
if `output` would exceed **16 KB serialized**, the tool writes the full
content to `/workspace/.observations/<call_id>.txt` inside the sandbox,
and the event stores only `file_ref: "/workspace/.observations/<call_id>.txt"`
plus a 1 KB preview in `output.preview`. The agent then reads via `file_read`
when needed. This keeps the event stream KV-cache-friendly (PRINCIPLE #3).

### 3.5 Migrations

Use `refinery` (embedded SQL migrations, Rust-native, no extra runtime).
Migrations run on every server start; SQLite WAL set via `PRAGMA journal_mode=WAL;`
on connection open. Migration files committed to `/migrations/` and numbered
sequentially. Phase 0 ships 3 files (sessions, events, plans).

---

## 4. API surface

### 4.1 HTTP routes (axum)

| Method | Path | Purpose |
|---|---|---|
| GET  | `/healthz` | Liveness probe (200 if SQLite, Redis, Bifrost reachable) |
| GET  | `/v1/sessions` | List sessions (paginated, newest first) |
| POST | `/v1/sessions` | Create session (body: `{ title?, project_id? }` → returns `{ id }`) |
| GET  | `/v1/sessions/:id` | Session detail |
| GET  | `/v1/sessions/:id/events` | Query events (`?after_id=&type=&limit=`) |
| GET  | `/v1/sessions/:id/plan` | Current plan snapshot |
| GET  | `/v1/workspace/:session_id/*path` | Proxy read of sandbox workspace files (read-only, for Monaco) |
| GET  | `/v1/cost` | Aggregated cost (proxies Bifrost `/cost`) |

**No** task-control HTTP endpoints — task lifecycle is driven via WebSocket
commands (§4.2). Rationale: a single channel avoids race conditions between
"agent producing events" and "user pausing." HTTP is for query and listing
only.

### 4.2 WebSocket protocol — `/ws`

Single envelope, two channels (server↔client). All messages are JSON.

```typescript
// shared envelope
type Envelope =
  | { type: "event";   id: string; session_id: string; ts: number; payload: EventPayload }
  | { type: "command"; id: string; session_id: string; ts: number; payload: CommandPayload }
  | { type: "ack";     id: string; ref: string; ok: boolean; error?: string }
  | { type: "ping";    ts: number }
  | { type: "pong";    ts: number }
  | { type: "error";   id?: string; kind: string; message: string };

// server → client
type EventPayload =
  | { kind: "Message";     role: "user"|"assistant"; content: string; ui?: "notify"|"ask" }
  | { kind: "Action";      tool: string; args: object; call_id: string }
  | { kind: "Observation"; call_id: string; ok: boolean; output?: any; error?: { kind: string; message: string }; file_ref?: string }
  | { kind: "Plan";        op: "create"|"advance"|"update"; plan_id: string; snapshot: Plan }
  | { kind: "Misc";        kind_tag: string };

// client → server
type CommandPayload =
  | { cmd: "subscribe";     session_id: string; from_event_id?: number }
  | { cmd: "task_create";   input: string; max_steps?: number; cost_cap_cents?: number }
  | { cmd: "task_pause";    session_id: string }
  | { cmd: "task_resume";   session_id: string }
  | { cmd: "task_cancel";   session_id: string }
  | { cmd: "user_response"; session_id: string; in_reply_to_call_id: string; content: string };
```

**Connection flow**:

1. Client connects to `/ws`.
2. Client sends `subscribe` for one or more sessions.
3. Server replays missed events (if `from_event_id` provided), then streams
   new events in real time.
4. Client may send `task_create` (no `session_id` — server allocates and
   returns `ack { ok: true }` carrying the new id).
5. Heartbeat: server sends `ping` every 30 s; client must respond with `pong`
   within 10 s or connection is closed. Frontend reconnects with backoff
   (story 0.20).

**Event delivery guarantee**: at-least-once. Each event has a monotonic
`session_id`-scoped `id` (the SQLite rowid). Frontend deduplicates by
`(session_id, id)`. The `from_event_id` mechanism makes reconnect-and-catch-up
trivial.

### 4.3 Internal dispatcher API — 32 tools → 5 backends

`ToolContext` is constructed once per session and shared (Arc-cloned) for
every tool invocation. Each `Tool::invoke` uses only the backend it needs.

```rust
pub struct ToolContext {
    pub session_id: SessionId,
    pub sandbox:    Arc<SandboxClient>,    // backend: Sandbox
    pub frontend:   Arc<FrontendBus>,      // backend: Frontend (WS sender)
    pub search:     Arc<SearchClient>,     // backend: Search
    pub deploy:     Arc<DeployClient>,     // backend: Deploy (stub in Phase 0)
    pub store:      Arc<dyn EventStore>,   // backend: Internal (SQLite)
    pub plan:       Arc<PlanManager>,      // for plan_* tools
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn schema(&self) -> serde_json::Value;
    async fn invoke(&self, args: Value, ctx: &ToolContext)
        -> Result<ToolOutput, ToolError>;
}
```

**Routing table** (32 tools — Phase 0 deferral noted):

| Tool | Backend | Notes |
|---|---|---|
| `message_notify_user`, `message_ask_user` | **Frontend** | Emits Message event; `ask` blocks loop until `user_response` command received |
| `file_read`, `file_write`, `file_str_replace`, `file_find_in_content`, `file_find_by_name` | **Sandbox** | HTTP to AIO Sandbox file API |
| `shell_exec`, `shell_view`, `shell_wait`, `shell_write_to_process`, `shell_kill_process` | **Sandbox** | HTTP to AIO Sandbox shell API (per-process IDs) |
| `browser_view`, `browser_navigate`, `browser_restart`, `browser_click`, `browser_input`, `browser_move_mouse`, `browser_press_key`, `browser_select_option`, `browser_scroll_up`, `browser_scroll_down`, `browser_console_exec`, `browser_console_view` | **Sandbox** | HTTP to AIO Sandbox Chromium API |
| `info_search_web` | **Search** | Brave Search API in Phase 0 (Tavily as fallback config) |
| `deploy_expose_port`, `deploy_apply_deployment` | **Deploy** | **STUB** in Phase 0 — returns "deferred" error; full impl Phase 1+ |
| `idle` | **Internal** | Signals task completion; agent loop terminates |
| `sop_read`, `playbook_search`, `glossary_lookup` | **Internal** | **STUB** in Phase 0 — returns empty; tables added Phase 3 |

**Plan tools** are dispatched in-band but not part of the 32-tool catalog
exposed to the LLM as separate functions — they ARE exposed to the LLM
(so the agent can call `plan_advance`/`plan_update`) but `plan_create`
is called by the runtime pre-loop, not by the LLM mid-loop.

| Tool | Backend | Notes |
|---|---|---|
| `plan_create` (internal, pre-loop only) | Plan Manager | Called by agent runtime at task start; uses `planner` slot |
| `plan_advance` | Plan Manager | Atomic increment of `current_phase_id` |
| `plan_update` | Plan Manager | Replace phases array; emit Plan event with `op:"update"` |

**Hooks** (story 0.10): three async hook points, each receives `&ToolContext`
and a mutable `HookData`. Phase 0 hooks are minimal: PreToolUse emits the
Action event; PostToolUse emits the Observation event; PostToolUseFailure
preserves the error in the event stream (PRINCIPLE #10).

### 4.4 Bifrost interface

**Network contract (Phase 0):**

| Item | Value | Source of truth |
|---|---|---|
| Container internal port | `8080` | Bifrost image default |
| Host port (mapped) | `4000` | `docker-compose.yml` |
| Host bind | `127.0.0.1` (localhost-only) | §9 |
| Base URL (from control plane) | `http://localhost:4000` | `BIFROST_BASE_URL` env |
| OpenAI-compatible prefix | `/v1` | OpenAI convention |
| Health endpoint | `GET /health` → 200 | verified against pinned tag in story 0.1 |
| Auth (Phase 0) | **none** | §9 (BIFROST_MASTER_KEY env is forward-compat scaffolding for Phase 5) |

**Endpoints the control plane consumes:**

- `POST /v1/chat/completions` — agent loop, planner slot (story 0.11+)
- `GET  /v1/models` — capability detection at startup (story 0.13)
- `GET  /cost` — aggregated cost (story 0.16 reads this; **NOT** used in story 0.1)

**Cost-tracking ownership across Phase 0 stories (resolves Codex issue #2):**

| Story | Cost-tracking behavior |
|---|---|
| 0.1 | Bifrost-native cost: enabled in `bifrost/config.yaml` (`observability.cost_tracking: true`). Visibility = Bifrost's own surface (logs / JSON output of `/cost` endpoint). **No local DB callback** — DB doesn't exist yet (story 0.3). |
| 0.3 | SQLite schema (`sessions.cost_cents`) lands. |
| 0.16 | Control plane polls `GET /cost` after each tool call, increments `sessions.cost_cents`, enforces cap. This is the "callback to local DB" requirement from req §3.1 — fulfilled here, not in 0.1. |

**Slot ownership across Phase 0 stories (resolves Codex issue #1):**

- **Story 0.1**: creates **Bifrost-side aliases only** — `agent-primary`,
  `agent-fallback`, `local-fast`. Three flat names, no slot semantics.
- **Story 0.12**: introduces the **Rust-side 12-slot resolver**. The slot
  config (`/config/slots.yaml`, server-side, distinct from
  `bifrost/config.yaml`) maps slot names (`main`, `planner`, `verifier`,
  the 9 auxiliary slots) → Bifrost aliases.
- **Story 0.13**: capability auto-detection via `GET /v1/models`.

In Phase 0, Bifrost owns the **alias→provider→model** routing.
The Rust control plane owns the **slot→alias** routing. These two layers
are independent and shipped in different stories.

**Model IDs are env-overridable (resolves Codex issue #5):**

The defaults in `bifrost/config.yaml` are *examples that work today*, not
forever-pinned values. Use the Bifrost templating syntax:

```yaml
models:
  - name: agent-primary
    provider: anthropic
    model: ${BIFROST_MODEL_PRIMARY:-claude-sonnet-4-6}
  - name: agent-fallback
    provider: openai
    model: ${BIFROST_MODEL_FALLBACK:-gpt-4o}
  - name: local-fast
    provider: ollama
    model: ${BIFROST_MODEL_LOCAL_FAST:-llama3.2:3b}
```

Defaults chosen for Phase 0:
- `claude-sonnet-4-6` — current best price/perf for tool-calling agents
- `gpt-4o` — different vendor for true fallback
- `llama3.2:3b` — **light** (~2 GB), runs on a laptop; replaces the
  earlier `qwen2.5:32b` which is impractical for first-run smoke tests

**Fallback verification (resolves Codex issue #3):**

Story 0.1's smoke test proves `agent-primary` and `local-fast` work
individually. To prove the fallback chain, the test includes a
**dedicated sub-test** that sets `ANTHROPIC_API_KEY=sk-deliberately-invalid`
for the duration of one HTTP call and asserts Bifrost transparently
serves the response from `agent-fallback` (OpenAI). The override is
applied via `env -u` / inline env, NEVER by mutating the developer's
real `.env` file.

---

## 5. External dependencies

### 5.1 Rust crates (versions pinned to minor)

```toml
# crates/seasoned-hand-core/Cargo.toml — partial
axum            = "0.7"
tokio           = { version = "1.40", features = ["full"] }
tokio-tungstenite = "0.23"
rusqlite        = { version = "0.31", features = ["bundled", "serde_json"] }
refinery        = { version = "0.8", features = ["rusqlite"] }
deadpool-redis  = "0.15"
bollard         = "0.17"             # pin — req §9 open question resolved
reqwest         = { version = "0.12", features = ["json", "stream"] }
serde           = { version = "1", features = ["derive"] }
serde_json      = "1"
serde_yaml      = "0.9"
figment         = { version = "0.10", features = ["yaml", "env"] }
thiserror       = "1"
tracing         = "0.1"
tracing-subscriber = "0.3"
uuid            = { version = "1", features = ["v4", "serde"] }
rig-core        = "0.5"              # ReAct loop, tool schemas
async-trait     = "0.1"
```

**Rejected**: any crate not in this list during Phase 0 work requires an
update to ARCHITECTURE.md §11 and approval (per AGENTS.md §9 NEVER).

### 5.2 External services

| Service | Image / Version | Port | Phase 0 use |
|---|---|---|---|
| Bifrost | `maximhq/bifrost:<pin-tag>` — Story 0.1 implementer picks the latest stable tag from `https://hub.docker.com/r/maximhq/bifrost/tags`, commits the chosen tag here AND in `docker-compose.yml`. Never use `latest`. | 4000 (host) → 8080 (container) | LLM gateway |
| Redis | `redis:7-alpine` | 6379 | Pub/sub fanout |
| AIO Sandbox | `<verify-image-name>:<pin-tag>` — Story 0.8 implementer verifies the actual image name (`agentinfra/aio-sandbox` or `ghcr.io/agent-infra/sandbox`) and pins a tag. `.env.example` currently says `ghcr.io/agent-infra/sandbox:latest`; pin it during story 0.8. | 6080 (noVNC), 7681 (ttyd), 8080 (sandbox API) | Per-session container, launched via bollard, NOT in docker-compose |
| Ollama | **Optional** — host machine, not Docker | 11434 | Provides `local-fast` alias. If absent, story 0.1's `local-fast` smoke sub-test is **skipped, not failed**. Default model `llama3.2:3b` (light, ~2 GB). User installs Ollama and pulls the model per their OS — documented in `docs/getting-started.md`, not enforced by Phase 0 acceptance. |

`docker-compose.yml` defines only Bifrost + Redis. Sandbox containers are
managed by the control plane via bollard (not Compose) because lifecycle is
per-session.

### 5.3 Frontend dependencies

```json
{
  "next": "15.x",
  "react": "19.x",
  "react-dom": "19.x",
  "tailwindcss": "4.x",
  "react-resizable-panels": "2.x",
  "monaco-editor": "0.50.x",
  "@monaco-editor/react": "4.x",
  "@xterm/xterm": "5.x",
  "@xterm/addon-fit": "0.10.x"
}
```

Package manager: **pnpm only** (AGENTS.md §7).

---

## 6. Interactions with existing components

There is no prior phase. This section is N/A. Phase 0 is greenfield.

Forward-compatibility notes (so Phase 1+ doesn't need to refactor):

- **Verifier slot (Phase 1)**: slot router already has a `verifier` slot
  entry that resolves to `auto` in Phase 0. Phase 1 wires the L4
  meta-cognition trigger (ARCHITECTURE.md §6) without schema changes.
- **Initializer/Worker pattern (Phase 1)**: events table already supports
  `source: "sub_agent_N"` (ADR-009 §"Phase 0 implications").
- **Learning artifacts (Phase 3)**: migration files 004+ add
  `sops`/`playbooks`/`playbooks_fts`/`glossary` without touching Phase 0 tables.
- **Multi-tenant (Phase 5)**: `sessions.user_id` is nullable in Phase 0,
  becomes required in Phase 5.
- **`map` tool (Phase 4+)**: tool registry is a `HashMap<&'static str, Arc<dyn Tool>>`
  — dynamic registration possible without core refactor (ADR-009).

---

## 7. Performance budget

From requirements.md §2 plus Phase 0 component allocations:

| Budget | Target | Owner component | How verified |
|---|---|---|---|
| Cold start (first task) | < 10 s | Sandbox client (container create) | story 0.27 E2E timer |
| Subsequent task start | < 2 s | Session store + agent runtime | story 0.27 |
| Memory per active session | < 200 MB | Agent runtime + sandbox connection | `cargo-bloat` + RSS monitor |
| Idle memory (no sessions) | < 100 MB | Server baseline | RSS monitor at boot |
| WebSocket fanout latency | < 50 ms p95 | API layer + Redis pub/sub | story 0.20 ad-hoc bench |
| Tool dispatch overhead | < 20 ms | Dispatcher (excluding tool I/O) | criterion bench in story 0.9 |
| One agent loop iteration | < 1 s + LLM time | Agent runtime | tracing spans |
| Event append (single) | < 5 ms p95 | Event stream | criterion bench in story 0.4 |
| Plan render (sticky context) | < 200 tokens typical, hard cap 1000 tokens | Plan Manager | unit test on serializer |

Cost budget per session: **$1 default** (configurable via `task_create
cost_cap_cents`). Bifrost reports cost; control plane halts the loop when
session `cost_cents` ≥ cap and emits a Misc event with `kind: "cost_cap"`.

---

## 8. Failure modes

| Failure | Detection | Handling |
|---|---|---|
| Bifrost unreachable | reqwest timeout (5 s) on first call | Mark session ERROR, emit Observation with `error.kind = "llm_gateway_down"`; do NOT silently retry (PRINCIPLE #10) |
| Bifrost returns 5xx on primary alias | HTTP status | Bifrost's own fallback chain (config.yaml `routing.fallbacks`) kicks in; if all fail, surface to event stream |
| Sandbox container fails to start | bollard error | Retry once with `--rm`; on second failure, session ERROR with clear message |
| Sandbox crashes mid-session | bollard event stream | Pause session, surface Observation, allow user to resume (which restarts container; workspace volume persists) |
| Tool call exceeds 5 min | `tokio::time::timeout` per dispatch | Emit Observation `{ ok: false, error.kind: "timeout" }`; loop continues (agent decides next action) |
| Tool output > 16 KB | output size check in dispatcher | Write to `/workspace/.observations/<call_id>.txt`, store `file_ref` only (§3.4) |
| Stuck agent (duplicate output ≥ 2) | Hash of last assistant message | Inject strategy-change prompt; if 4 consecutive duplicates, terminate session with ERROR |
| Cost cap reached | `session.cost_cents ≥ cap_cents` after Bifrost reports | Emit `Misc { kind: "cost_cap" }`; transition session to SUSPENDED; require user resume |
| Redis disconnect | deadpool-redis reconnect | Buffer events in-memory (cap 1000); on reconnect, drain. If buffer overflows, drop oldest and emit Misc warning |
| SQLite write failure (disk full, lock contention) | rusqlite Result | Surface as 500 on HTTP, retry with backoff on WS; if persistent, session ERROR |
| WebSocket disconnect mid-task | tokio-tungstenite event | Task continues; events accumulate in DB. Frontend reconnects + replays via `from_event_id` |
| Plan creation returns malformed JSON | serde parse error | Retry once with stricter prompt; on second failure, fall back to single-phase plan `[{ id:1, title: input, capabilities: [], status: "active" }]` and log Misc warning |
| User sends `task_cancel` mid-loop | WS command | Set session SUSPENDED; agent loop checks cancellation token between iterations; sandbox paused |

**Hard rule** (PRINCIPLE #10): no failure is silently swallowed. Every
failure either produces an event the user can see or transitions the
session to a visible state (ERROR / SUSPENDED).

---

## 9. Security considerations

Phase 0 is single-user, localhost-only. Hardening for multi-user lives in
Phase 5.

- **Sandbox isolation**: AIO Sandbox is the security boundary (ADR-004).
  All shell/file/browser tool calls execute inside the container; the
  control plane never executes them on the host.
- **API keys**: read from `.env`, passed to Bifrost via environment, never
  logged. `.env` is gitignored (verify in story 0.1).
- **WebSocket auth**: Phase 0 binds `127.0.0.1` only. No CORS open, no
  auth headers. Phase 5 adds bearer-token auth.
- **Bifrost auth (resolves Codex issue #9)**: Phase 0 runs Bifrost
  unauthenticated, bound to `127.0.0.1:4000`. The `BIFROST_MASTER_KEY`
  env var in `.env.example` is forward-compat scaffolding for Phase 5+
  (when the control plane authenticates outbound requests to a
  publicly-exposed Bifrost) and is **unused** by the control plane and
  by the story 0.1 smoke test in Phase 0. Setting it has no effect in
  Phase 0; clearing it does not break anything.
- **Sandbox network egress**: AIO Sandbox default network mode = bridged
  with internet access. Acceptable Phase 0 risk (single-user, opt-in
  deployment). Phase 1 considers egress allowlisting.
- **Workspace path traversal**: `/v1/workspace/:session_id/*path` resolves
  `path` inside the sandbox volume and rejects `..` segments. Test required.
- **SQL injection**: rusqlite with bound parameters only; no `format!`
  into SQL anywhere. Clippy lint enforces.
- **Prompt injection from tool outputs** (e.g., a browsed webpage tries to
  hijack the agent): out of scope Phase 0. Tracked for Phase 1 hardening
  alongside Verifier (Layer 2 cross-source validation, ARCHITECTURE.md §6).
- **Secrets in event stream**: tool outputs may contain secrets (e.g.,
  `cat .env`). Acceptable Phase 0 (single-user, local DB). Phase 5
  redaction layer is future work.

---

## 10. Migration plan

N/A — Phase 0 is greenfield, no prior deployments.

Migration files exist (`/migrations/001..003`) as forward-compatible
infrastructure for Phase 3+ additions.

---

## 11. Testing strategy

### 11.1 Unit (per story)

Every Rust module has `#[cfg(test)]` tests for pure functions. Coverage
target: not enforced as a number in Phase 0, but each public function on
public traits must have at least one test.

### 11.2 Integration (per subsystem)

- **Event stream** (story 0.4): tests round-trip append → query →
  subscribe with a real SQLite file + real Redis (testcontainers).
- **Tool dispatcher** (story 0.9): test routing for one tool per backend
  with mocked backend clients; one full real-sandbox integration test.
- **Plan Manager** (folded into stories 0.6 + 0.14): tests
  create/advance/update flows; serialization stability.
- **LLM client + slot router** (stories 0.11–0.13): tests against a local
  fake OpenAI server (`mockito`); capability detection round-trip.

### 11.3 E2E (story 0.27)

Headless test driven by a small Rust test binary:

1. `docker compose up -d` (Bifrost + Redis).
2. Start control plane.
3. POST `/v1/sessions` → get id.
4. Open WS, send `task_create { input: "Find the GitHub stars of FoundationAgents/OpenManus" }`.
5. Drain events until `Message { ui: "notify" }` with a numeric answer
   appears, OR session transitions to ERROR.
6. Assert: cost_cents > 0, tool_calls > 0, at least one Action with
   `tool: "browser_navigate"`, final Message contains a digit-only token.

Acceptance criterion from requirements.md §5 is satisfied iff this test
passes deterministically (with `agent-fallback` masked off, so primary
LLM is exercised).

### 11.4 Verification gates (mandatory pre-commit, per AGENTS.md §6)

```
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
pnpm typecheck
pnpm test
./scripts/spec-check.sh
```

`spec-check.sh` is enhanced in story 0.27 to verify:
- `crates/seasoned-hand-core/src/tools/mod.rs` declares exactly 32 tools
  (current heuristic in `scripts/spec-check.sh` already counts `pub mod`).
- Migration files are sequentially numbered with no gaps.

---

## 12. Open technical questions

These remain open at the time of writing; resolve before the affected story.

- [ ] **Brave vs Tavily** for `info_search_web` in Phase 0. Default Brave
  (API exists, free tier sufficient); Tavily as YAML-configurable
  alternative. Resolve in story 0.7.
- [ ] **Refinery vs sqlx-migrate** — both fit. Refinery is lighter
  (recommended); decide in story 0.3.
- [ ] **rig 0.5 tool-call surface** — confirm rig exposes `tool_choice:
  required` directly; if not, hand-roll the OpenAI request in story 0.14.
- [ ] **AIO Sandbox API stability** — verify the HTTP API shape against the
  pinned version in story 0.8 before story 0.7 wires the browser tools.
- [ ] **Cost cap granularity** — current design is per-session. Open: should
  cap also enforce per-day? Defer to Phase 1 (no Phase 0 user requirement).
- [ ] **Plan rebuild on context compression** — Phase 0 has no compression
  pass (small contexts). When `compression` slot is wired (Phase 1), the
  plan-verbatim-preservation guarantee in ADR-010 must be tested.
- [ ] **Frontend type sharing** — hand-write WS types in Phase 0 vs adopt
  `ts-rs` codegen now. Decision: hand-write (small surface). Revisit
  Phase 1 if drift becomes a real problem.
- [ ] **Bifrost dashboard URL (Codex issue #7)** — Bifrost may or may not
  expose a built-in admin dashboard. Decision: drop dashboard verification
  from any automated acceptance criteria. Cost visibility in Phase 0 =
  `curl http://localhost:4000/cost` output, captured by story 0.1's smoke
  test and asserted as a non-empty JSON object. Any dashboard surfaced by
  the chosen Bifrost tag is treated as a manual-inspection nice-to-have.
- [ ] **Bifrost config schema fidelity (Codex issue #4)** — the example
  YAML in story 0.1 reflects ADR-001's intent, not a verified Bifrost
  config file. Story 0.1 implementer cross-checks the schema against the
  pinned tag's documentation/README and adjusts both files if Bifrost's
  actual schema differs (e.g., key names, nesting). The `Models`/`Routing`/
  `Observability` section names here are the *intent*, not the wire format.

---

## Appendix A — Mapping stories to this architecture

| Story | Hits these sections |
|---|---|
| 0.1  | §5.2 (Bifrost) |
| 0.2  | §2 (workspace layout) |
| 0.3  | §3.1–3.3, §3.5 |
| 0.4  | §3.2, §3.4 |
| 0.5  | §4.2 (subscribe channel side) |
| 0.6  | §4.3 (Tool trait), 5 simplest tools = message_notify_user, message_ask_user, file_read, idle, glossary_lookup (stub) |
| 0.7  | rest of §4.3 routing table |
| 0.8  | §2 (Sandbox client) + §5.2 |
| 0.9  | §4.3 dispatcher |
| 0.10 | §4.3 hooks |
| 0.11 | §2 (LLM client) + §4.4 |
| 0.12 | §2 (router) + §4.4 slots.yaml |
| 0.13 | §4.4 capability detection |
| 0.14 | §1 agent runtime + §4.3 plan_create pre-loop |
| 0.15 | §8 stuck row |
| 0.16 | §7 cost budget + §8 cost row |
| 0.17 | §4.2 WS server side |
| 0.18 | §5.3 frontend init |
| 0.19 | §1 frontend 3-panel |
| 0.20 | §4.2 client + reconnect |
| 0.21 | §4.2 Message rendering |
| 0.22 | §4.1 GET /v1/sessions |
| 0.23 | §1 AgentComputer tabs scaffold |
| 0.24 | §1 noVNC iframe |
| 0.25 | §1 xterm.js + ttyd |
| 0.26 | §1 Monaco + §4.1 workspace proxy |
| 0.27 | §11.3 E2E |

---

## Appendix B — Phase 0 simplifications vs ARCHITECTURE.md

These are intentional; not divergences requiring an ADR update because
ARCHITECTURE.md describes the *target*, not Phase 0's subset.

| ARCHITECTURE.md feature | Phase 0 behavior |
|---|---|
| Verifier (slot, L4 meta-cognition) | Slot reserved; no enforcement |
| Curator | Not present |
| Module Workers via Redis Streams | Redis pub/sub only; modules run inline |
| 4-layer verification framework | Only L1 (deterministic re-read of tool output) and L3 (per-iteration observation analysis) operate; L2/L4 deferred |
| `make_manus_page` replacement (`deploy_apply_deployment`) | Stub; returns "deferred" error |
| `sop_read`, `playbook_search`, `glossary_lookup` | Stubs; tables absent |
| Diversity injection (context engineering #6) | Not enforced (small contexts) |
| Compression slot | Not used (small contexts) |
| Multi-tenant (`user_id`) | Always NULL |
| Auth | None (localhost only) |

---

*End of Phase 0 architecture. When approved, BMAD PM persona breaks this
into stories 0.2–0.27 (story 0.1 already drafted).*
