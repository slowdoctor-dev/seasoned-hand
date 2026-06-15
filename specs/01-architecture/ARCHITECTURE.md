# Seasoned Hand — Architecture Specification

> **Status**: v1.5 (Dioxus unified-Rust frontend per ADR-016)
> **Last updated**: 2026-06-05
> **Owners**: Project lead
>
> **v1.2 amendments (ADR-012, 2026-05-18)**: §2.5 reconciled with Phase 3 V010:
> V009-compatible playbook carry-forward columns retained, learning/search columns added,
> `session_search_index` + `session_search_fts` added, and FTS5 maintenance triggers
> specified (`playbooks_*`, `session_search_index_*`).
>
> **v1.3 amendments (ADR-013, 2026-05-18)**: §2.5 reconciled with Phase 4 V011:
> playbook denormalization (`source_project_id`, `active_revision_id`, archive metadata),
> revision graph (`playbook_revisions`, `playbook_revision_outcomes`), curator runtime
> persistence (`curator_decisions`, `curator_review_queue`, `sop_conflicts`,
> `knowledge_items`, `datasource_items`, `weekly_retrospectives`,
> `retrospective_citations`), and `curator_search_index` + `curator_search_fts` with
> maintenance triggers (`curator_search_index_*`). §2.1 Skill taxonomy expanded to
> include `curation_decision` events.
>
> **v1.4 amendments (ADR-014, 2026-05-20)**: §2.5 reconciled with Phase 5 V013:
> multi-user org/user domain (`organizations`, `users`, `organization_memberships`,
> `project_role_overrides`), immutable operation ledger (`audit_log`), per-user billing
> rollups (`user_cost_ledger`), sharing ACL surfaces (`sop_shares`, `playbook_shares`),
> and tenant-safe event projection (`tenant_event_view`). Phase 2-4 mutable tables tighten
> `tenant_id` from nullable to NOT NULL with deterministic backfill and validation. §2.1
> notes Phase 5 multi-user/audit event kinds.
>
> **v1.5 amendments (ADR-016, 2026-06-05)**: §1.1 Frontend layer changed from
> Next.js + React + TypeScript to a **unified-Rust Dioxus** frontend
> (`crates/seasoned-hand-ui`) targeting Web (WASM) / Desktop / Mobile from one
> codebase. The `/v1` REST + WebSocket boundary, the control plane, Bifrost, and the
> sandbox are **unchanged** — this is a frontend-layer swap. Monaco / xterm / noVNC are
> retained on web/desktop via JS interop; the mobile `AgentComputer` degrades to
> read-only. Amends the frontend clause of ADR-002; BASELINE §4 + §7 #5 updated in the
> same change. Implementation is staged across Phase 6 stories (see ADR-016 migration plan,
> gated on a step-1 interop spike).

This is the **immutable architectural specification**. Changes require:
1. PR with rationale
2. Update version number
3. Migration plan if breaking

---

## 1. System overview

Seasoned Hand is an autonomous agent platform built on the principle:

> **Manus runtime as kernel + Hermes learning as user-space**

### OS metaphor (precise mapping)

After direct validation with Manus, the OS analogy maps as follows:

| OS concept | Seasoned Hand component |
|---|---|
| Kernel | LLM (reasoning, via 12-slot router) |
| Scheduler | Agent runtime (Rust + Rig + Tokio) |
| Syscalls | 32+ tools |
| Drivers | Tool backends (sandbox, browser, search, deploy) |
| Hardware | Sandbox (AIO Sandbox Docker per session) |
| **Process** | One task (session) |
| **Process Control Block (PCB)** | Plan (goal + phases + current_phase_id) |
| **Program counter** | current_phase_id |
| Working memory (RAM) | Context window |
| Persistent memory | Sandbox filesystem |
| Process history | Event stream (append-only) |
| User-space programs | Playbooks (Phase 3+) |
| Standard library | SOPs + Glossary (Phase 3+) |
| cron daemon | Curator (Phase 4+) |

The **Plan as PCB** mapping is critical: it's the structured artifact that
tracks "where we are in the task" and prevents goal drift. See ADR-010.

### 1.1 Component layers

```
┌──────────────────────────────────────────────────────┐
│  Frontend (Dioxus — unified Rust, per ADR-016)        │
│  - Targets: Web (WASM) | Desktop | Mobile             │
│  - 3-panel UI: TaskList | Chat | AgentComputer        │
│  - WebSocket subscribe to event stream                │
│  - noVNC / Monaco / xterm via JS interop (web+desktop)│
│  - Mobile AgentComputer degrades to read-only         │
│  - Shares wire DTOs via seasoned-hand-dto (no codegen)│
└──────────────────────┬───────────────────────────────┘
                       │ WebSocket + HTTP (/v1 — unchanged)
                       ↓
┌──────────────────────────────────────────────────────┐
│  Control Plane (Rust)                                 │
│  ┌──────────────────────────────────────────────┐    │
│  │ API Gateway (Axum)                            │    │
│  │ HTTP routes + WebSocket (tungstenite)        │    │
│  └──────────────────────────────────────────────┘    │
│  ┌──────────────────────────────────────────────┐    │
│  │ Agent Runner (Rig + custom extensions)        │    │
│  │ - ReAct loop with "1 tool per iteration"     │    │
│  │ - Context engineering 6 principles            │    │
│  │ - Stuck detection (OpenManus pattern)         │    │
│  │ - Hooks (PreToolUse, PostToolUse, etc.)      │    │
│  │ - 4-layer verification (see § 5)              │    │
│  └──────────────────────────────────────────────┘    │
│  ┌──────────────────────────────────────────────┐    │
│  │ Plan Manager (ADR-010)                        │    │
│  │ - Structured plan: goal + phases + phase_id  │    │
│  │ - Sticky context injection (every iteration) │    │
│  │ - plan_create / plan_advance / plan_update   │    │
│  │ - Persistent in `plans` SQLite table         │    │
│  └──────────────────────────────────────────────┘    │
│  ┌──────────────────────────────────────────────┐    │
│  │ Tool Dispatcher                               │    │
│  │ 32+ tools → 5 backends (sandbox, frontend,    │    │
│  │ search, deploy, internal)                     │    │
│  └──────────────────────────────────────────────┘    │
│  ┌──────────────────────────────────────────────┐    │
│  │ Module Workers (Tokio tasks)                  │    │
│  │ - Planner / Verifier / Curator                │    │
│  │ - Redis Streams queue                         │    │
│  └──────────────────────────────────────────────┘    │
│  ┌──────────────────────────────────────────────┐    │
│  │ Persistence                                    │    │
│  │ - SQLite (rusqlite): events, sessions,        │    │
│  │   playbooks, glossary, sops                   │    │
│  │ - Redis (deadpool-redis): pub/sub, streams    │    │
│  └──────────────────────────────────────────────┘    │
│  ┌──────────────────────────────────────────────┐    │
│  │ Sandbox Client (bollard)                      │    │
│  │ - AIO Sandbox Docker per session              │    │
│  └──────────────────────────────────────────────┘    │
└──────────────────────┬───────────────────────────────┘
                       │ OpenAI-compatible HTTP
                       ↓
┌──────────────────────────────────────────────────────┐
│  Bifrost (Go) — LLM Gateway                            │
│  - 12-slot model routing                              │
│  - Credential pools, fallback chains                  │
│  - Cost tracking, prompt caching                      │
└──────────────────────┬───────────────────────────────┘
                       │
       ┌───────────────┼───────────────┐
       ↓               ↓               ↓
   Cloud LLMs    Local LLMs       Self-hosted
   (Anthropic,   (Ollama, MLX,    (vLLM, SGLang)
    OpenAI,       llama.cpp)
    Google AI)
```

> **UI hosting (issue #33).** The Dioxus UI is a static wasm bundle and is
> deployment-independent of the control plane. Two topologies are supported:
> (a) **dev** — `dx serve` / `just dev-ui` runs the UI on its own port against the
> `/v1` + `/ws` API; (b) **single-binary self-host** — `just build-ui` produces a
> static bundle and the control plane serves it as the router **fallback** when
> `SH_UI_DIST` points at the bundle dir. The API routes (`/v1/*`, `/ws`,
> `/healthz`, `/metrics`) always take precedence; the fallback only serves the
> shell + assets and resolves unknown paths to `index.html` (SPA). Static serve is
> public (the shell calls the auth-gated API itself). Net-new dep (per §9):
> `tower-http 0.6` (`fs` feature — `ServeDir`/`ServeFile`), server crate only.
>
> **Phase 2 channel-framework addendum** (per AGENTS.md §9 — net-new
> Rust deps trigger a one-line note):
> `lettre 0.11` (SMTP send, EmailChannel delivery + notify),
> `mailparse 0.15` (RFC 5322 parse, EmailChannel intake),
> `async-imap 0.10` (IMAP poll, EmailChannel intake — `tokio-rustls`
> feature, no openssl),
> `toml 0.8` (`config/notify.toml` parser, story 2.12 NotifyWorker
> per-trigger routing).
> Sandbox-side renderer toolchain: Pandoc + python-pptx + openpyxl
> (installed at session-create time per phase-2/DEBT.md #2).
> Phase 3 learning addendum: `unicode-normalization 0.1`
> (NFD normalization for matcher, F-3.4 / story 3.5).
>
> **Phase 5 dependency addendum** (story 5.23 / F-5.20 / closes Phase 5
> DEBT #97): zero net-new Rust dependencies. Multi-user + organization,
> RBAC, audit log, hand-off lifecycle, per-user cost ledger,
> tenant-aware event redaction, optimistic concurrency, and global
> strict-config harmonization all built on the Phase 0-4 crate set.
> Any future story that adds a workspace dependency must extend this
> addendum with a one-line justification per crate; `scripts/spec-check.sh`
> enforces the existence of this addendum block as a discipline gate.

---

## 2. Data model

### 2.1 Event Stream (single source of truth)

```sql
CREATE TABLE events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id TEXT NOT NULL,
  timestamp INTEGER NOT NULL,    -- unix epoch microseconds
  type TEXT NOT NULL CHECK(type IN (
    'Message',      -- user input
    'Action',       -- tool call by agent
    'Observation',  -- tool result
    'Plan',         -- planner module output
    'Knowledge',    -- knowledge module retrieval
    'Datasource',   -- datasource module catalog
    'Skill',        -- playbook match/injection/outcome + curation_decision (v1.3)
    'Misc'
  )),
  source TEXT NOT NULL,  -- user, agent, planner, knowledge, etc.
  data TEXT NOT NULL,    -- JSON payload
  FOREIGN KEY (session_id) REFERENCES sessions(id)
);

CREATE INDEX idx_events_session_time ON events(session_id, timestamp);
CREATE INDEX idx_events_type ON events(type);
```

Append-only. Never UPDATE or DELETE. KV-cache friendly.

`Skill` event payload sub-kinds are architecture-visible taxonomy:
`match`, `injection`, `outcome`, `curation_decision` (added in v1.3).
Phase 5 adds multi-user audit/ownership `Misc` kinds (for example:
`task_handoff_completed`, `tenant_event_projection_failed`,
`membership_role_changed`) while preserving append-only semantics.

### 2.2 Sessions

```sql
CREATE TABLE sessions (
  id TEXT PRIMARY KEY,       -- UUID
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  state TEXT NOT NULL,       -- IDLE, RUNNING, FINISHED, ERROR, SUSPENDED, VERIFYING
  project_id TEXT,           -- optional grouping
  user_id TEXT,              -- multi-tenant (single user for v0)
  title TEXT,
  cost_cents INTEGER DEFAULT 0,
  tool_calls INTEGER DEFAULT 0,
  metadata TEXT              -- JSON
);
```

`VERIFYING` was added by Phase 1 story 1.9
(`migrations/V004__verifications.sql:25-47` widens the CHECK constraint
via the canonical SQLite new-table → copy → rename pattern).

### 2.2.1 Tasks (Phase 2)

Phase 2 introduces `tasks` as the durable user-facing unit on top of
`sessions`. One task may have multiple sessions over time (e.g. a
durable pause/resume cycle rebuilds the sandbox into a fresh session).
A `sessions` row carries an optional `task_id` foreign key from
`migrations/V006__phase2_projects_tasks.sql`.

```sql
-- abridged; see migrations/V006 for full schema
CREATE TABLE tasks (
  id TEXT PRIMARY KEY,           -- UUID
  project_id TEXT NOT NULL REFERENCES projects(id),
  status TEXT NOT NULL,          -- Drafted, Briefed, Confirmed, Running,
                                 -- Paused, Completed, Failed, Cancelled
  title TEXT NOT NULL,
  ...
);
```

`TaskStatus` is an 8-variant state machine
(`Drafted → Briefed → Confirmed → Running ⇄ Paused → Completed | Failed
| Cancelled`). `Cancelled` is reachable from every non-terminal state —
`Drafted`, `Briefed`, `Confirmed` (per Phase 2 DEBT #19's pre-run cancel
window), AND `Running`/`Paused` (operator cancel mid-execution). The
legal-transitions matrix lives at
`crates/seasoned-hand-core/src/project/task.rs::legal_transitions`; the
full Phase 2 task lifecycle is documented in
`/specs/phase-2/architecture.md` §2.2.

### 2.3 Plans (ADR-010)

```sql
CREATE TABLE plans (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  goal TEXT NOT NULL,
  phases TEXT NOT NULL,        -- JSON array of {id, title, capabilities, status}
  current_phase_id INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  FOREIGN KEY (session_id) REFERENCES sessions(id)
);

CREATE INDEX idx_plans_session ON plans(session_id);
```

The plan is the Process Control Block (PCB) of the task. Injected into
every iteration's sticky context. See ADR-010.

### 2.4 Tool Catalog (static)

In-code constant. Not in DB. **38 tools** = 28 (29 Manus-leaked minus
`make_manus_page`, which we replaced with `deploy_apply_deployment` —
see §7 "Removed") + 3 learning stubs (`sop_read`, `playbook_search`,
`glossary_lookup`) + 2 LLM-callable plan tools (`plan_advance`,
`plan_update`) + 4 Phase 1 additions (`feature_mark_done`,
`progress_update`, `checkpoint_label`, `checkpoint_rollback`) + 1
Phase 2 addition (`task_deliver`). The 38-count is pinned by
`scripts/spec-check.sh`. See §7 for the full enumeration.

Plan management (ADR-010): `plan_advance` and `plan_update` are
exposed through `ToolDispatcher` as LLM-callable tools — the agent's
loop drives plan progression via them. `plan_create(goal, phases)` is
the Initializer's internal entry point used to seed the plan at task
start; it is NOT registered in the tool catalog and is not LLM-callable.

### 2.5 Learning artifacts (Phase 3+)

```sql
-- SOPs: explicit, version-controlled
CREATE TABLE sops (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  content TEXT NOT NULL,
  version INTEGER NOT NULL,
  enforced BOOLEAN DEFAULT 1,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

-- Playbooks: auto-extracted from verified work
CREATE TABLE playbooks (
  id TEXT PRIMARY KEY,
  tenant_id TEXT,              -- nullable in Phase 3 (Phase 5 tightens)
  title TEXT NOT NULL,
  content_path TEXT NOT NULL,  -- V009 compatibility, reserved in Phase 3
  schema_version INTEGER NOT NULL,
  source_task_id TEXT REFERENCES tasks(id),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  trigger_keywords TEXT NOT NULL,  -- JSON array
  content TEXT NOT NULL,
  version INTEGER NOT NULL,
  success_count INTEGER DEFAULT 0,
  failure_count INTEGER DEFAULT 0,
  avg_duration_ms INTEGER,
  avg_tool_calls INTEGER,
  status TEXT NOT NULL  -- active, archived, pinned
);

CREATE VIRTUAL TABLE playbooks_fts USING fts5(
  title, trigger_keywords, content,
  content='playbooks',
  content_rowid='rowid'
);

CREATE TRIGGER playbooks_ai AFTER INSERT ON playbooks BEGIN
  INSERT INTO playbooks_fts(rowid, title, trigger_keywords, content)
  VALUES (new.rowid, new.title, new.trigger_keywords, new.content);
END;
CREATE TRIGGER playbooks_ad AFTER DELETE ON playbooks BEGIN
  INSERT INTO playbooks_fts(playbooks_fts, rowid, title, trigger_keywords, content)
  VALUES ('delete', old.rowid, old.title, old.trigger_keywords, old.content);
END;
CREATE TRIGGER playbooks_au AFTER UPDATE ON playbooks BEGIN
  INSERT INTO playbooks_fts(playbooks_fts, rowid, title, trigger_keywords, content)
  VALUES ('delete', old.rowid, old.title, old.trigger_keywords, old.content);
  INSERT INTO playbooks_fts(rowid, title, trigger_keywords, content)
  VALUES (new.rowid, new.title, new.trigger_keywords, new.content);
END;

-- Glossary: organizational facts
CREATE TABLE glossary (
  id TEXT PRIMARY KEY,
  term TEXT NOT NULL UNIQUE,
  definition TEXT NOT NULL,
  category TEXT NOT NULL,  -- person, system, terminology, context
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

-- Session search index (Phase 3 denormalized FTS surface)
CREATE TABLE session_search_index (
  event_id INTEGER PRIMARY KEY,
  session_id TEXT NOT NULL,
  timestamp INTEGER NOT NULL,
  event_type TEXT NOT NULL CHECK(event_type IN (
    'Message', 'Action', 'Observation', 'Plan',
    'Knowledge', 'Datasource', 'Skill', 'Misc'
  )),
  source TEXT NOT NULL,
  searchable_text TEXT NOT NULL
);

CREATE INDEX idx_session_search_session_time
  ON session_search_index(session_id, timestamp);
CREATE INDEX idx_session_search_type
  ON session_search_index(event_type);

CREATE VIRTUAL TABLE session_search_fts USING fts5(
  searchable_text,
  content='session_search_index',
  content_rowid='event_id'
);

CREATE TRIGGER session_search_index_ai AFTER INSERT ON session_search_index BEGIN
  INSERT INTO session_search_fts(rowid, searchable_text)
  VALUES (new.event_id, new.searchable_text);
END;
CREATE TRIGGER session_search_index_ad AFTER DELETE ON session_search_index BEGIN
  INSERT INTO session_search_fts(session_search_fts, rowid, searchable_text)
  VALUES ('delete', old.event_id, old.searchable_text);
END;
CREATE TRIGGER session_search_index_au AFTER UPDATE ON session_search_index BEGIN
  INSERT INTO session_search_fts(session_search_fts, rowid, searchable_text)
  VALUES ('delete', old.event_id, old.searchable_text);
  INSERT INTO session_search_fts(rowid, searchable_text)
  VALUES (new.event_id, new.searchable_text);
END;

-- Project History uses events table (no separate table)
```

### 2.5.1 Phase 4 (V011) extension surface

V011 extends the learning schema with curator/revision persistence:

- `playbooks` additions: `source_project_id`, `active_revision_id`, `archived_reason`,
  `archived_at` (+ `idx_playbooks_project_status`).
- revision graph:
  - `playbook_revisions` (revision chain, `parent_revision_id` FK)
  - `playbook_revision_outcomes` (revision-scoped counters/decay state)
- curator runtime ledger:
  - `curator_decisions`
  - `curator_review_queue`
  - `sop_conflicts`
- knowledge/datasource persistence:
  - `knowledge_items`
  - `datasource_items`
- retrospective persistence:
  - `weekly_retrospectives`
  - `retrospective_citations`
- curator search surface:
  - `curator_search_index`
  - `curator_search_fts` (FTS5, external-content)
  - triggers: `curator_search_index_ai`, `curator_search_index_ad`,
    `curator_search_index_au`

V011 also performs one-time backfill from V010:
- set `playbooks.source_project_id` from `tasks.project_id` (when source task exists),
- seed revision-1 rows and active revision pointers,
- seed `playbook_revision_outcomes` from existing counters,
- rebuild `playbooks_fts` once post-backfill.

### 2.5.2 Phase 5 (V013) extension surface

V013 extends the learning/runtime schema for multi-user operation:

- org/user identity and role graph:
  - `organizations`
  - `users`
  - `organization_memberships`
  - `project_role_overrides`
- collaboration sharing ACL:
  - `sop_shares`
  - `playbook_shares`
- immutable operator-grade audit ledger:
  - `audit_log`
- per-user cost rollups:
  - `user_cost_ledger`
- tenant-safe event projection:
  - `tenant_event_view`
- tenant tightening:
  - all Phase 2-4 mutable `tenant_id` columns are NOT NULL after backfill and validation.

V013 follows the same atomic-slice reconciliation rule as V010/V011:
migration + successor ADR + ARCH version bump land together.

---

## 3. Model routing — 12 slots

### 3.1 Main slots (3)

| Slot | Default | Purpose |
|---|---|---|
| `main` | (user-configured) | Agent loop, tool decisions |
| `planner` | (user-configured) | Task decomposition |
| `verifier` | (user-configured, different from main) | Result verification |

### 3.2 Auxiliary slots (9)

| Slot | Default behavior |
|---|---|
| `vision` | auto (main if vision-capable, else cheap vision model) |
| `web_extract` | auto |
| `screenshot` | auto |
| `compression` | auto |
| `session_title` | auto (cheap model preferred) |
| `session_search` | auto |
| `classifier` | auto (cheap model preferred) |
| `embedding` | (separate embedding model) |
| `reasoning` | auto (or o3-style if user configures) |

### 3.3 3-tuple per slot

```yaml
slot_name:
  provider: openrouter | anthropic | openai | google | custom | auto | main
  model: <provider-specific model ID>
  base_url: <override URL, takes precedence over provider>
```

---

## 4. Agent loop (Manus pattern, immutable)

```
At task start:
  0a. Briefing — interpret one-line task, propose plan
  0b. Plan create — structured plan stored (goal + phases)
  
Per iteration:
  1. Read sticky context — plan + recent events
  2. Analyze: evaluate current phase progress
  3. Select ONE tool to call (or plan_advance / plan_update)
  4. Execute (sandbox or external)
  5. Observe (result → event stream)
  6. Verify deterministically (L1, see § 5)
  7. Repeat until idle or max_steps
  
At task end:
  8. Final verification (L2-L4)
  9. Submit result via message_notify_user
```

Constraints:
- **One tool call per iteration** (HARD constraint, enforced at API level via `tool_choice="required"`)
- **Plan always at top of context** (sticky, survives compression)
- Stuck detection: 2+ duplicate assistant outputs → strategy change prompt
- Max steps: configurable per task type (10 simple, 50 complex, 100+ coding)
- Cost cap: per session ($) and global daily

---

## 5. Context engineering (6 principles, immutable)

From Manus official blog. All MUST be enforced:

1. **KV-cache friendly**: stable prefix, never modify earlier context
2. **No mid-iteration tool changes**: mask, don't remove
3. **Filesystem as memory**: large content → file path reference (see PRINCIPLE #16)
4. **Todo recitation**: re-read todo.md every ~10 turns
5. **Preserve errors**: failed tool calls stay in context
6. **Diversity injection**: vary phrasing to prevent few-shot lock-in

**Plus our additions** (validated by Manus direct Q&A):

7. **Plan stickiness** (PRINCIPLE #17, ADR-010): plan structure always at top
8. **RAM/disk dichotomy** (PRINCIPLE #16): conversation = RAM (lossy), files = disk (100% accurate)

---

## 6. Verification (4-layer framework)

A single "Verifier service" is insufficient. Manus validation revealed
verification operates at 4 distinct layers:

### Layer 1 — Deterministic verification
**Trigger**: After every tool call (PostToolUse hook).
**Mechanism**: Re-read tool output to confirm it actually happened.
- File write → file_read to verify content
- Browser navigate → screenshot + DOM check
- Shell command → exit code + stdout inspection

**Failure response**: Preserve error in event stream (PRINCIPLE #5);
agent must address before next iteration.

### Layer 2 — Cross-source validation
**Trigger**: During information gathering (info_search_web, web_extract).
**Mechanism**: Require ≥2 independent sources before treating a fact
as established. Conflicts reported rather than silently resolved.

**Failure response**: Report discrepancy as observation; do not pick
one source arbitrarily.

### Layer 3 — Observation analysis
**Trigger**: Every iteration start (built into agent loop step 1-2).
**Mechanism**: Agent must analyze recent observations before deciding
next action. Cannot proceed if last observation is an error.

**Failure response**: Diagnose error and adjust strategy.

### Layer 4 — Meta-cognition (Verifier slot)
**Trigger**: 
  - Task completion (before declaring done)
  - When new data invalidates assumed-good earlier work
  - When circuit breaker triggers (stuck, cost, error count)

**Mechanism**: Separate model (verifier slot) reviews completed work
with FAIL-biased prompting in fresh context.

**Failure response**: Trigger `plan_update` (course correction) or
return to user with explanation.

---

## 7. Tool catalog (38 tools)

### From Manus leaked spec (29)

- **Message (2)**: `message_notify_user`, `message_ask_user`
- **File (5)**: `file_read`, `file_write`, `file_str_replace`, `file_find_in_content`, `file_find_by_name`
- **Shell (5)**: `shell_exec`, `shell_view`, `shell_wait`, `shell_write_to_process`, `shell_kill_process`
- **Browser (12)**: `browser_view`, `browser_navigate`, `browser_restart`, `browser_click`, `browser_input`, `browser_move_mouse`, `browser_press_key`, `browser_select_option`, `browser_scroll_up`, `browser_scroll_down`, `browser_console_exec`, `browser_console_view`
- **Search (1)**: `info_search_web`
- **Deploy (2)**: `deploy_expose_port`, `deploy_apply_deployment`
- **System (2)**: `idle`, `make_manus_page` (we replace this — see below)

### Learning additions (3, Phase 0)

- `sop_read` — read SOP by ID/title
- `playbook_search` — FTS5 search playbooks
- `glossary_lookup` — look up term

### Phase 1 additions (4)

- `feature_mark_done` — Initializer marks a Brief feature complete (story 1.4)
- `progress_update` — Worker writes a one-line progress note (story 1.4)
- `checkpoint_label` — Worker tags the next checkpoint with a phase title (story 1.13)
- `checkpoint_rollback` — Internal-only rollback to a prior checkpoint
  (story 1.13b, masked from every LLM-facing mode per `dispatch/mask.rs`)

### Phase 2 additions (1)

- `task_deliver` — Worker hands a finished real-employee artifact back
  to the operator (story 2.14). Worker-mode only via `ToolMaskPolicy`.

### Plan-related (ADR-010)

`plan_advance` and `plan_update` are LLM-callable through
`ToolDispatcher` and count toward the 38-tool total above. `plan_create`
is the Initializer's internal seed call and is NOT registered in the
catalog — it's documented here only for ADR-010 completeness.

### Removed

- `make_manus_page` (Manus-specific, replaced with `deploy_apply_deployment` generic)

---

## 8. Phase plan summary

| Phase | Weeks | Goal |
|---|---|---|
| 0 | 3 | Foundation infrastructure |
| 1 | 4 | Manus 5-layer capabilities (deep execution) |
| 2 | 3 | Employee interface (briefing, deliverables, accountability) |
| 3 | 4 | Time-axis learning (4-layer system, playbook extraction) |
| 4 | 3 | Curator + self-improvement |
| 5 | 3 | Multi-user + organization |
| 6 | 2 | Open source release |

**Total: 22 weeks**

Each phase has its own spec dir: `/specs/phase-N/`.

---

## 9. Non-goals

What we explicitly do NOT build:

- Messaging platform adapters (Telegram, Slack, etc.) — web UI first
- OAuth provider integrations — API keys sufficient for v1
- Mobile app — web responsive only
- Multi-modal generation (images, audio) — text + tool use only
- Fine-tuning pipeline — use existing models
- Distributed multi-region deployment — single-region for v1
- Marketplace for playbooks — user creates locally first

---

## 10. Quality gates

Every PR must pass:

```
✓ cargo clippy --all-targets -- -D warnings
✓ cargo fmt --check
✓ cargo test --workspace
✓ just check-ui — UI crate fmt + clippy + wasm check (crates/seasoned-hand-ui, Dioxus, ADR-016)
✓ spec-check (custom: verifies code matches /specs)
✓ no TODO without linked issue
✓ commit message follows convention
```

---

## 11. Open questions (TBD before Phase 0)

- [ ] Multi-tenant DB strategy: separate DB per user vs shared with user_id?
- [ ] Auth: API key, OAuth, or both?
- [x] License: MIT vs Apache 2.0? → **Apache-2.0** (ADR-015; was MIT per ADR-008)
- [ ] Repo name: `seasoned-hand`, `seasoned-hand`, `something-else`?
- [ ] Default cloud sandbox provider for users without local Docker?
- [ ] Telemetry: opt-in usage stats? privacy policy?

---

## 12. References to other specs

- `/specs/phase-0/architecture.md` — Phase 0 detailed architecture
- `/specs/phase-0/requirements.md` — Phase 0 requirements
- `/specs/phase-0/stories/` — Phase 0 story breakdown
- `/docs/methodology.md` — development methodology
- `/CLAUDE.md` — project context for AI agents
- `/AGENTS.md` — AI agent instructions

---

## 13. Versioning

- This spec: v1.0 (initial)
- Bumped on breaking architectural changes
- Each phase spec: independent versioning

### 13.1 Curator rationale envelope contract (Phase 5 story 5.25)

`curator_decisions.rationale_json` payloads carry an outer envelope so
readers can dispatch on schema version without parsing the inner data:

```json
{"schema_version": 2, "data": { ...decision-type-specific payload... }}
```

Versioning rules:

- **V1** (Phase 4): flat object, no envelope. The whole JSON IS the
  data. Detected by absence of an integer `schema_version` key. Readers
  MUST tolerate V1 rows forever — no migration.
- **V2** (Phase 5+): wrapped envelope with `schema_version: 2` and an
  inner `data` object. Every new write uses V2.
- **Future versions** (V3+): readers that don't know the version fall
  back to V1 detection, which validates the row as "well-formed object"
  but doesn't pretend to understand the inner shape. This keeps older
  binaries safe in the face of forward-evolved payloads.

Implementation:
[`seasoned_hand_core::curator::rationale::SchemaVersion`](../../crates/seasoned-hand-core/src/curator/rationale.rs).
Both production write sites (`MinerExtractionEngine::run_once` pattern
recommendations + `ProductionConsolidationEngine::propose` duplicate
candidates) wrap with `SchemaVersion::wrap_v2`. Closes Phase 5 DEBT #96.
