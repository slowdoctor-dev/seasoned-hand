# Phase 2 — Architecture (Employee Interface)

> **Status**: v1.0 (BMAD Architect persona output, 2026-05-13)
> **Duration**: 3 weeks
> **Base commit**: `4ad4256` (post-Phase-1 hardening complete)
> **Goal**: "Do this overnight" workflow works end-to-end. The system
> stops feeling like a chatbot and starts feeling like a digital
> employee — one that takes a brief, executes for 24h, and reports back.

## 0. Inputs and methodology note

This architecture spec was drafted directly from the BMAD Architect
persona (`/prompts/bmad-architect.md`) without a preceding Analyst
session — Phase 2 scope is well-bounded by:

- `/specs/06-roadmap/ROADMAP.md` §Phase 2 (the 9-deliverable list)
- `/specs/phase-1/RETROSPECTIVE.md` (top-3 Phase 2 carry-overs +
  hardening DEBT items #14 and #15)
- `/specs/01-architecture/ARCHITECTURE.md` (immutable — referenced,
  not modified)

The forks resolved with the user before drafting:

- **A1**: separate `projects` + `tasks` tables; `sessions` becomes a
  child of `tasks` (a single Task may span multiple Session executions
  across pause/resume).
- **B1**: structured `Briefing` event + WS confirmation round-trip
  using the existing `user_response` verb.
- **C1**: Rust-native `notify` Redis-Streams worker + adapters for
  ntfy / webhook / email (`lettre`). ROADMAP's "BullMQ" mention
  conflicts with ADR-002 (Rust control plane) so the stack stays
  Rust.
- **24h durability target**: paused tasks must survive at least 24h
  (container GC + event-stream replay rebuild path).
- **3 weeks** confirmed.

---

## 1. Summary diagram

```
┌─────────────────────────────────────────────────────────────────────────┐
│                            Next.js 16 Frontend                           │
│  ┌─────────────┐ ┌──────────────────────┐ ┌─────────────────────────┐   │
│  │ ProjectList │ │ Chat + Briefing Card │ │ AgentComputer + tabs:   │   │
│  │ (NEW left)  │ │ (MOD: confirm UI)    │ │  Browser / Terminal /   │   │
│  │             │ │                      │ │  Editor / Verifier /    │   │
│  │             │ │                      │ │  Deliverables (NEW) /   │   │
│  │             │ │                      │ │  Decisions (NEW)        │   │
│  └─────────────┘ └──────────────────────┘ └─────────────────────────┘   │
│                              │ (one shared WS in HomeShell — Phase 1)   │
└──────────────────────────────┼───────────────────────────────────────────┘
                               │
┌──────────────────────────────┼───────────────────────────────────────────┐
│                   Rust control plane (Axum, ADR-002)                     │
│                              │                                           │
│   ┌──────────────────────────▼──────────────────────────┐                │
│   │  HTTP routes: + /v1/projects, /v1/tasks/:id/...     │                │
│   │  WS envelopes: + Briefing, Deliverable, Decision    │                │
│   └────────┬────────────────────┬─────────────────────┬─┘                │
│            │                    │                     │                  │
│   ┌────────▼─────────┐ ┌────────▼──────────┐ ┌────────▼──────────┐       │
│   │ ProjectStore     │ │ Initializer (1.4) │ │ NotifyWorker      │       │
│   │ TaskStore (NEW)  │ │ + Briefing emit   │ │ (NEW Redis-       │       │
│   │ DeliverableStore │ │ + confirm wait    │ │ Streams consumer) │       │
│   │ (NEW)            │ │ (NEW)             │ │                   │       │
│   └────────┬─────────┘ └────────┬──────────┘ └────────┬──────────┘       │
│            │                    │                     │                  │
│            │              ┌─────▼─────────────────────▼─────┐            │
│            │              │ Existing Phase 1 runtime:        │            │
│            └──────────────┤  AgentRunner (1.14+1.17),        │            │
│                           │  PlanManager (1.1),              │            │
│                           │  Verifier Worker (1.9b + DEBT    │            │
│                           │   #15 close: real XREADGROUP),   │            │
│                           │  Checkpoint Manager (1.13)       │            │
│                           │   + DEBT #14 fix (1.13b shell    │            │
│                           │   injection) + advance fanout    │            │
│                           └──────────────────────────────────┘            │
│                                                                          │
│   Persistence (SQLite WAL):                                              │
│     V006: projects, tasks, sessions.task_id                              │
│     V007: deliverables                                                   │
│     V008: notifications_sent (audit log; Redis Stream is the queue)      │
│                                                                          │
│   Adapters (out-of-stack): ntfy / webhook / SMTP                         │
└──────────────────────────────────────────────────────────────────────────┘
```

---

## 2. New components introduced

### 2.1 Project / Task / Subtask hierarchy

**Module**: `seasoned-hand-core::project` (new).

- `Project { id, title, description?, status, created_at, updated_at }` —
  a long-lived bucket ("Q2 launch", "OSS docs revamp"). Status:
  `active | archived`.
- `Task { id, project_id, title, brief: Option<Brief>, status,
  expected_due_at?, completed_at?, failure_reason? }` — one "do this"
  unit. Status: `drafted | briefed | confirmed | running | paused |
  completed | failed | cancelled`.
- `Subtask` is **NOT** a separate table — subtasks are the Plan's
  phases (Phase 1's `plans.phases[*]`). The ROADMAP's "Project / Task
  / Subtask" hierarchy resolves to Project → Task → Plan-phase
  semantically, with `tasks.id` joined to `plans.session_id` via
  `sessions.task_id`.
- `Session { …existing 1.x fields, + task_id (FK, nullable for Phase 0/1
  legacy rows) }` — sessions become **executions of a task**. A single
  Task with 3 pause/resume cycles produces 3 Session rows.

Stores: `ProjectStore` + `TaskStore` (rusqlite-backed, mirror the Phase
1 `CheckpointStore` / `VerificationStore` shape; queries use
parameterized SQL per the simplicity audit's invariant).

### 2.2 Briefing protocol

**Module**: `seasoned-hand-core::agent::init` (Phase 1 1.4) extended.

The existing Initializer is extended with a confirm gate:

```
task_create →
  Initializer parses input into a Brief →
  Initializer emits Misc{kind:"briefing_pending", briefing_call_id, brief} →
  Server emits ServerEvent{ Briefing { briefing_call_id, goal,
                                       phases, success_criteria,
                                       expected_deliverables } } →
  Frontend renders confirmation card →
  User clicks Confirm / Edit / Cancel →
  WS user_response{ in_reply_to_call_id: briefing_call_id,
                    content: "confirm" | "edit" | "cancel",
                    edits?: PartialBrief } →
  Initializer:
    - on confirm  → seed Plan from brief, transition task to "running"
    - on edit     → re-parse with edits, re-emit Briefing
    - on cancel   → task → "cancelled", emit task_state
    - on timeout (default 5 min, configurable; --no-auto-confirm flag
                  disables it) → log briefing_auto_confirmed Misc and
                  proceed with the brief as-is.
```

`Brief` shape (JSON, stored on `tasks.brief`):

```typescript
type Brief = {
  goal: string,                         // 1-3 sentences
  phases: Array<{ id: number; title: string; capabilities?: string[] }>,
  success_criteria: string[],           // each ≤ 200 chars
  expected_deliverables: string[],      // human-readable, e.g. "summary.md", "github.csv"
}
```

The Plan Manager (Phase 1 1.1) seeds from `brief.phases` on confirm.

### 2.3 Deliverable standards

**Module**: `seasoned-hand-core::deliverable` (new).

Phase 2 ships exactly two formats:

- **markdown** — primary; written by the LLM via a new tool
  `task_deliver(content, format, citations)`. Persisted to
  `/workspace/.deliverables/<deliverable_id>.md` via the Phase 1
  story-1.14 file-ref helper. Citations are an array of `event_id`s
  proving provenance (the Phase 1 `evidence_event_ids` pattern from
  the verifier verdicts).
- **json** — structured payloads (used when the brief's
  `expected_deliverables` includes a `.json` file).

`Deliverable { id, task_id, format, content_ref: FileRef, citations:
Vec<i64>, created_at }`. New `Deliverable` event kind. Inline content
in the event for ≤16 KB; file_ref above (story 1.14 path).

New LLM tool `task_deliver` is **Worker-mode only** (masked from
Initializer + Verifier + Internal modes — Phase 1 1.5 mask layer
handles this once the tool is added to `DefaultMaskPolicy`).

### 2.4 Status reporting dashboard (backend)

Read-only HTTP routes (frontend is a story-level deliverable, not in
this spec):

- `GET /v1/projects?limit=&cursor=` → `ListResponse<Project>`
- `GET /v1/projects/:id` → `Project + task counts by status`
- `POST /v1/projects` → create
- `PATCH /v1/projects/:id` → rename / archive
- `GET /v1/projects/:id/tasks?status=&limit=&cursor=` → `ListResponse<Task>`
- `GET /v1/tasks/:id` → full Task incl. session-ids list
- `GET /v1/tasks/:id/deliverables` → `ListResponse<Deliverable>`
- `GET /v1/tasks/:id/notifications` → `ListResponse<NotificationSent>`

All use the shared `seasoned_hand_core::routes::RouteOutcome` from the
Phase 1 simplicity pass (commit `dc87177`).

### 2.5 Accountability trail

**No new module** — piggybacks on the existing event stream.

New Misc kind `decision` emitted by:

- **Initializer**: briefing decisions (which phases were chosen and
  why)
- **Verifier**: each verdict already carries `reason` + `evidence_event_ids`;
  Phase 2 promotes this into the `decision` Misc kind for uniform
  rendering
- **Checkpoint Manager**: rollback decisions (`reason`,
  `evidence_event_ids`)

Shape: `Misc { kind:"decision", source: String, reason: String,
alternatives_rejected: Vec<String>, evidence_event_ids: Vec<i64> }`.

Frontend reads `Misc{kind:"decision"}` events and renders chronologically.
No new HTTP route needed (read via the existing event-query path).

### 2.6 Long-running task durability (24h+)

Two pause tiers:

- **Soft pause** (existing Phase 1 1.17): `docker pause` + WS
  `task_paused` event + DB state SUSPENDED. Survives across runner
  restarts AS LONG AS the docker container exists.
- **Durable freeze** (NEW Phase 2): soft pause + persist
  `task_paused_durable` Misc with `{ event_cursor, sandbox_id,
  workspace_path, paused_at }`. On resume, if the container is gone
  (sandbox orphan-cleaned per Phase 0 DEBT #16), reconstruct from
  event-stream replay into a fresh sandbox.

WS `task_pause` gets a `durable: bool` field. Phase 2 defaults to
`true` (always durable).

**Resume flow**:

```
task_resume(task_id) →
  load TaskStore → look up last session_id and last event cursor →
  check sandbox handle: exists?
    YES → unpause + RUNNING + agent runner resume (Phase 1 1.17)
    NO  → spawn fresh sandbox + replay events:
            - Plan Manager state from Plan events
            - feature-list.json from initial create + feature_done events
            - progress.txt from progress_update / progress_recite events
            - cost from CostClient last snapshot
          → start new Session row (linked to same task_id) → RUNNING
```

The Phase 0 DEBT #16 (workspace TTL + cleanup) finally pays down here:
paused tasks extend the workspace TTL by 7 days; tasks in `running` or
`paused` status are NEVER garbage-collected.

### 2.7 Async notification system

**Module**: `seasoned-hand-core::notify` (new).

```
Event triggers (event-stream listener):
  - task_state{ to: "completed" }  → notify(task_finished)
  - task_state{ to: "failed" }     → notify(task_failed)
  - briefing_pending               → notify(briefing_pending)  (opt-in)
  - verifier_verdict{ verdict: "fail" } → notify(verifier_fail)  (opt-in)

Producer: hook on EventStore::append filters event kinds; XADD to
          Redis Stream "notify_request".

Consumer: notify::Worker (per-process Tokio task, like Phase 1's
          Verifier Worker shape, but real XREADGROUP from day one
          — no polling stub).

Adapters (Adapter trait, plural impls):
  - NtfyAdapter      — POST to https://ntfy.sh/<topic> (configurable host)
  - WebhookAdapter   — POST {url, json: payload}
  - EmailAdapter     — lettre 0.11 SMTP, async-tokio feature

Per-trigger routing config (config/notify.toml):
  [trigger.task_finished]
  adapters = ["ntfy", "email"]
  [trigger.task_failed]
  adapters = ["ntfy", "email", "webhook"]

Audit log: every dispatch writes a notifications_sent row regardless
           of adapter success/failure. ok=0 + error message on failure.
```

**No retries** for ntfy / email (best-effort). **Webhook** gets 1
retry after 30 s on 5xx (the common transient case).

### 2.8 DEBT close-outs landing IN Phase 2

These are not new components, but they're prerequisites for Phase 2
features:

- **DEBT #15 close** — real Verifier `XREADGROUP` loop in
  `verifier::worker::Worker::run` (per-session FIFO via DashMap, global
  Semaphore from config, XACK + dead-letter on parse failure). Required
  before "Do this overnight" because background-fired triggers must
  produce verdicts without a host caller.
- **DEBT #14 close** — `SandboxGitShell::commit_phase` shell-quoting
  replaced with `git commit -F -` reading from stdin. Required BEFORE
  Plan{op:"advance"} fanout broadcaster lands (which is also a Phase 2
  surface — Checkpoint Manager goes from stub to wired).
- **NarratorHook classifier-slot wiring** — `AppState::new` learns to
  accept an `Option<ClassifierWiring>` so the LLM-path narration
  actually fires for `file_write` / `shell_*` / `browser_*` (story 1.15
  exec notes flagged this).
- **DEBT #9 close** — Playwright bootstrap + baseline coverage for the
  three new Phase 2 UI surfaces (Briefing card, Deliverables tab,
  Decisions pane).
- **DEBT #3 decision** — Verifier rollback default flip. Phase 2
  collects production verdict precision from real "overnight" tasks;
  if ≥90% precision, flip default to `true` in the Phase 2 closeout.
  Otherwise carry into Phase 3.

---

## 3. Data model changes

### V006 — Project / Task baseline

```sql
CREATE TABLE projects (
    id          TEXT    PRIMARY KEY,
    title       TEXT    NOT NULL,
    description TEXT,
    status      TEXT    NOT NULL,  -- 'active' | 'archived'
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE TABLE tasks (
    id              TEXT    PRIMARY KEY,
    project_id      TEXT    NOT NULL REFERENCES projects(id),
    title           TEXT    NOT NULL,
    brief           TEXT,             -- JSON Brief (§2.2); NULL until briefed
    status          TEXT    NOT NULL, -- see §2.1
    expected_due_at INTEGER,
    completed_at    INTEGER,
    failure_reason  TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

ALTER TABLE sessions ADD COLUMN task_id TEXT REFERENCES tasks(id);

CREATE INDEX idx_tasks_project_status ON tasks(project_id, status);
CREATE INDEX idx_sessions_task_id     ON sessions(task_id);
```

**Backfill**: Phase 0 / Phase 1 sessions are left with `task_id =
NULL`. They render in the frontend under a synthetic "Phase 0/1
archive" project (read-only, no new tasks attachable). A one-time
migration script (NOT a SQL migration) can backfill them at the
operator's choice; not Phase 2's responsibility.

### V007 — Deliverables

```sql
CREATE TABLE deliverables (
    id              TEXT    PRIMARY KEY,
    task_id         TEXT    NOT NULL REFERENCES tasks(id),
    format          TEXT    NOT NULL, -- 'markdown' | 'json'
    content_path    TEXT    NOT NULL, -- workspace file path
    content_sha256  TEXT    NOT NULL,
    content_size    INTEGER NOT NULL,
    citations       TEXT,             -- JSON array of event_ids
    created_at      INTEGER NOT NULL
);

CREATE INDEX idx_deliverables_task ON deliverables(task_id);
```

### V008 — Notification audit log

```sql
CREATE TABLE notifications_sent (
    id           TEXT    PRIMARY KEY,
    task_id      TEXT,                  -- nullable: some notifies (e.g. briefing_pending) precede task creation
    trigger_kind TEXT    NOT NULL,
    adapter      TEXT    NOT NULL,      -- 'ntfy' | 'webhook' | 'email'
    payload      TEXT,                  -- redacted JSON
    ok           INTEGER NOT NULL,      -- 1 | 0
    error        TEXT,                  -- non-null when ok=0
    sent_at      INTEGER NOT NULL
);

CREATE INDEX idx_notifs_task ON notifications_sent(task_id);
```

The queue itself is the Redis Stream `notify_request`; this table is
audit, not state.

---

## 4. API surface

### New HTTP routes

All read-only routes use the shared `RouteOutcome<T>` from the Phase 1
simplicity pass.

```
GET    /v1/projects?limit&cursor                 → ListResponse<Project>
POST   /v1/projects { title, description? }      → Project
PATCH  /v1/projects/:id { title?, status? }      → Project
GET    /v1/projects/:id                          → Project + task counts
GET    /v1/projects/:id/tasks?status&limit&cursor → ListResponse<Task>

GET    /v1/tasks/:id                             → Task (with session list)
GET    /v1/tasks/:id/deliverables                → ListResponse<Deliverable>
GET    /v1/tasks/:id/notifications               → ListResponse<NotificationSent>

GET    /v1/notify/config                         → notify adapter config (read-only)
```

Existing routes (`/v1/sessions/...`) stay; they remain top-level for
backward compatibility with Phase 1 callers and test fixtures.

### WS envelope additions

```typescript
// ClientCommand additions
| { cmd: "task_create"; project_id?: string; title?: string; input: string;
    durable?: boolean; max_steps?: number; cost_cap_cents?: number }
| { cmd: "briefing_confirm"; task_id: string;
    in_reply_to_call_id: string;       // the briefing_call_id from the Briefing event
    action: "confirm" | "edit" | "cancel";
    edits?: PartialBrief }
| { cmd: "task_pause";  task_id: string; durable?: boolean }   // durable defaults true in Phase 2
| { cmd: "task_resume"; task_id: string }
| { cmd: "task_cancel"; task_id: string }
```

Phase 1's `cmd: "task_create" { input }` keeps working — it auto-creates a default project named `Inbox` and a task. The legacy `cmd: "task_pause" { session_id }` shape from Phase 1 1.17 still works (session-scoped); the new task-scoped form is preferred.

```typescript
// ServerEvent payload additions
| { kind: "Briefing"; briefing_call_id: string; goal: string;
    phases: Phase[]; success_criteria: string[];
    expected_deliverables: string[] }
| { kind: "Deliverable"; deliverable_id: string; format: "markdown" | "json";
    file_ref: FileRef; citations: number[] }
// `decision` is a Misc.kind_tag, not a kind: avoids growing the EventType enum.
// Same pattern Phase 1 used for browser_track_*, narration_skipped.
```

### Internal Rust APIs

- `seasoned_hand_core::project::{ProjectStore, TaskStore, Brief}` (rusqlite-backed)
- `seasoned_hand_core::deliverable::{DeliverableStore, Deliverable}`
- `seasoned_hand_core::notify::{Worker, Adapter, NtfyAdapter,
   WebhookAdapter, EmailAdapter, NotificationsSentStore}`
- `seasoned_hand_core::agent::init::Initializer` extended:
   `Initializer::run_with_confirmation(&self, task_id, brief_input,
   confirm_timeout)`

---

## 5. External dependencies

### New crates

| Crate | Version | Used by | Justification |
|---|---|---|---|
| `lettre` | 0.11 | `notify::EmailAdapter` | SMTP client. Pure Rust, async via `tokio-rustls` feature. Cheaper than wiring an external SMTP relay client. |

### Reused (existing in Phase 0/1)

- `reqwest` — webhook + ntfy adapters (HTTP POST)
- `redis` (via `pubsub::RedisPool`) — `notify_request` stream + the Phase 1 `verify_request` consumer-group bootstrap reused as a template
- `serde` + `serde_json` — payload (de)serialization
- `rusqlite` — Project / Task / Deliverable persistence
- `refinery` — V006 → V008 migrations
- `dashmap` — concurrency primitives (already used by 1.17 cancel_tokens)
- `tokio` + `tokio-util` — async runtime + cancellation tokens

### Frontend (story-level, listed for completeness)

| Package | Version | Justification |
|---|---|---|
| `@playwright/test` | ~1.x | Closes Phase 1 DEBT #9 — FE automated tests. Dev-only dep. |

No new runtime frontend deps.

### Documentation update required

The new `lettre` dep triggers an `/specs/01-architecture/ARCHITECTURE.md`
addendum per the AGENTS.md rule "Add dependencies without updating
`/specs/01-architecture/ARCHITECTURE.md`" being forbidden. Add a
one-line entry to §1.1 Component layers ("notify module — lettre
0.11 + ntfy/webhook"). One-line edit; not a re-architect.

---

## 6. Interactions with existing components

### Plan Manager (Phase 1 1.1)

Plans become **task-scoped** rather than session-scoped. The
`plans.session_id` column gets re-interpreted as "the first session
that activated this plan"; the Plan persists across pause/resume.

**Migration**: V006 adds `plans.task_id` (nullable, derived from
`sessions.task_id` on rehydrate; backfill via the runtime, not SQL).

### Verifier Worker (Phase 1 1.9b)

The polling stub at `Worker::run` is replaced with a real
`XREADGROUP GROUP verifier <consumer> BLOCK 5000 COUNT 16 STREAMS
verify_request >` loop. Closes DEBT #15.

Verdicts roll up to the Task: a Task's "verifier status" is the most
recent verdict from its most recent Session. Frontend shows this as a
badge on the Task summary card.

### Checkpoint Manager (Phase 1 1.13)

`CheckpointManager::run` becomes a real consumer of
`Plan{op:"advance"}` events (the broadcaster from story 1.20's
deferred E2E work lands here). Triggers DEBT #14 fix as a prereq.

Checkpoints become task-scoped: the `checkpoints.session_id` field
stays for traceability, but the rollback target lookup goes via
`task_id` → "latest checkpoint of any session of this task".

### Initializer (Phase 1 1.4)

Extended to emit `Briefing` events + wait for confirmation (§2.2).
`Initializer::run` keeps its existing signature for callers that
opt-out of the confirm gate (story-level decision: tests + the legacy
`task_create { input }` path).

### EventEmittingHook (Phase 0 0.10)

Unchanged. Phase 2's new event kinds flow through the same append
path. New `EventType` variants: none — Briefing + Deliverable become
new Action / Misc tags inside the existing `Message` / `Misc`
variants.

### NarratorHook (Phase 1 1.15)

Templated path gains 3 new entries: `task_deliver` → "Drafting the
deliverable", `briefing_confirm` → "Confirming the brief",
`task_create` (already templated as the generic).

The classifier-slot AppState wiring lands as the first Phase 2 housekeeping story (story-1.15 exec-note close-out).

### WS / `AppState` (Phase 1 1.17 + 1.18)

`AppState` gets:
- `Arc<ProjectStore>`, `Arc<TaskStore>`, `Arc<DeliverableStore>`,
  `Arc<NotificationsSentStore>`
- Notify Worker handle (`Option<NotifyWorkerHandle>` — `None` when no
  adapters configured)

The lifted `useAgentSocket` hook in `HomeShell` (1.18) is reused by
the new ProjectList panel + Deliverables tab.

### Frontend (Phase 0/1)

- **HomeShell** (Phase 0): adds ProjectList as a top-of-left panel
  above the existing TaskList. ProjectList → Tasks-of-active-project
  navigation.
- **Chat** (1.18): gains a "Briefing confirm" card renderer that
  intercepts `Briefing` events before the regular `EventRow` branch.
- **AgentComputer** (1.18 / 1.19): gains "Deliverables" + "Decisions"
  tabs. Verifier / Browser / Editor / Terminal stay unchanged.
- **TaskList** (Phase 0): renamed to **TaskListInProject**; takes
  `active_project_id` instead of "all sessions".

---

## 7. Performance budget

| Component | Target |
|---|---|
| Briefing event emit (Initializer parse → emit) | < 500 ms p95 (one LLM call to planner slot) |
| User briefing-confirm round-trip | unbounded (waits for human); 5-min auto-confirm timeout |
| `GET /v1/projects/:id/tasks` (50 rows) | < 100 ms p95 |
| Notification dispatch (queue → adapter call) | < 500 ms p95 ntfy/webhook; < 5 s p95 email |
| Sandbox durable freeze (pause + persist cursor) | < 3 s |
| Sandbox resume via existing container | < 10 s |
| Sandbox resume via event-stream replay (rebuild) | < 60 s |
| Verifier Worker XREADGROUP poll | `BLOCK 5000 COUNT 16` — burst-capable |
| Deliverable write to workspace | < 1 s (workspace fs write, ≤ 16 KB inline / file_ref above) |
| 24h-continuous task wall budget | 24 h + 30 min slack (memory growth ceiling) |
| Phase 1 budgets (verifier latency, plan render, cost, etc.) | unchanged |

---

## 8. Failure modes

| Mode | Detection | Handling |
|---|---|---|
| Briefing confirmation timeout (default 5 min) | watchdog in Initializer | Misc `briefing_auto_confirmed` + proceed with the brief as-is. Operator can set `briefing_require_confirm: true` in TOML. |
| Sandbox container GC'd while paused | `task_resume` finds no handle | Emit Misc `task_resume_rebuild_required`, spawn fresh sandbox, replay event stream into Plan + feature-list + progress + cost baseline. New Session row, same Task. |
| Event-stream replay corruption (schema drift, etc.) | replay step returns error | Task → `failed{reason:"replay_failed"}`. No silent recovery. Surface via Decisions pane. |
| Notification adapter failure (5xx, SMTP outage) | adapter returns Err | Append `notifications_sent { ok:0, error }`. Webhook gets 1 retry after 30 s. ntfy + email are best-effort (no retry — fire-and-forget). |
| Concurrent task_pause + task_resume race | DB state machine | Existing 1.17 cancel-token serialization. Resume waits for pause to finish via state check (`SUSPENDED` is the only resumable state). |
| Verifier Worker crash between consume + XACK | Redis Streams PEL retains the message | Next consumer picks it up. `handle_request` deduplicates on `triggered_at_event_id` via the insert path. |
| Deliverable write during cancelled task | state check before write | Reject with Misc `task_deliver_after_cancel`. Tool returns `ok:false`. |
| Brief field too long (e.g. 100 phases) | Initializer post-parse validation | Reject with `briefing_invalid{reason:"too_many_phases"}`. Hard cap: 20 phases, 50 success criteria, 50 deliverables. Configurable. |
| 24h task hits memory ceiling | runtime telemetry | `task_memory_ceiling` Misc + soft pause. No automatic recovery in Phase 2; user resumes manually. |
| Notify worker can't reach Redis | producer XADD fails | Log + skip — notification is best-effort. Task state unaffected. |

---

## 9. Security considerations

### DEBT #14 — `SandboxGitShell::commit_phase` shell injection

Story-level: lands BEFORE the Plan{op:"advance"} broadcaster activates
in Phase 2. Replace `format!("git commit ... \"{title}\"")` with
stdin-fed `git commit -F -` via the sandbox `/v1/shell/exec` `stdin`
field. Regression test feeds `` "`whoami`" ``, `$(id)`, newlines —
asserts no command substitution executes.

### Briefing free-text input

Untrusted user input flows through Initializer's LLM call. No escaping
needed (the Initializer prompt is system-controlled; user input lands
as a user-role message). The LLM's parsed output is JSON-validated
against the `Brief` schema before being written to `tasks.brief`.

### Webhook adapter URL

Phase 2 single-user: webhook URLs come from operator's
`config/notify.toml`. Trusted source — no SSRF protection in Phase 2.

**Phase 2 DEBT entry** (track as `phase-2/DEBT.md #1`): when multi-user
arrives in Phase 5, webhook URLs become user-supplied; need
SSRF-protection (no internal-IP, no loopback, no metadata-service URLs).

### Email adapter SMTP credentials

Loaded from env (`SMTP_HOST`, `SMTP_USERNAME`, `SMTP_PASSWORD`,
`SMTP_PORT`). Never logged. `lettre`'s standard auth flow.

### Notification payload PII

Default payloads are redacted: include only `task_id`, `task_title`,
`trigger_kind`, `timestamp`. Full content stays in DB. Operator can
opt into "verbose" mode per-adapter in `config/notify.toml` — that's
a security trade-off the operator owns.

### Task brief stored in plain text

`tasks.brief` JSON column may contain sensitive task descriptions.
Phase 2 single-user: same trust boundary as Phase 1 events table.
Encryption at rest is a Phase 5 concern.

### Admin rollback endpoint (Phase 1 1.13b)

Unchanged. The loopback + token guards from Phase 1 still apply. The
new test seam from B1 (`tests::admin_rollback_refuses_non_loopback_remote`)
keeps the regression covered.

---

## 10. Migration plan

### V006 → V008 — schema

All three migrations are forward-only (`refinery` standard). No
rollback path provided.

- V006: projects, tasks, sessions.task_id
- V007: deliverables
- V008: notifications_sent

### Backfill strategy

- **Phase 0 / Phase 1 sessions**: left with `task_id = NULL`. Frontend
  renders them under a synthetic "Phase 0/1 Archive" project (read-only
  view). No SQL backfill required.
- **Phase 0 / Phase 1 plans**: same — `plans.task_id` stays NULL for
  legacy rows.

### Breaking-change audit

| Surface | Phase 1 behavior | Phase 2 behavior | Break? |
|---|---|---|---|
| WS `task_create { input }` | session per cmd | task + session per cmd; "Inbox" auto-project | NO (additive) |
| WS `task_pause { session_id }` | session-scoped soft pause | session-scoped soft pause (legacy) OR task-scoped durable (new) | NO |
| HTTP `/v1/sessions/...` | unchanged | unchanged + new `/v1/tasks/...` parallel surface | NO |
| Event stream schema | EventType variants fixed | unchanged (new tags inside Message/Misc) | NO |
| `Initializer::run` signature | sync run | sync run OR `run_with_confirmation` | NO (additive) |

Net: zero wire-level breaks. Frontend gains new surfaces but existing
Chat / AgentComputer keep working unchanged.

---

## 11. Testing strategy

### Unit (Rust)

- `project::{ProjectStore, TaskStore}` — CRUD + pagination + state-machine tests (`drafted → briefed → confirmed → ...`)
- `briefing::confirm_round_trip` — confirm + edit + cancel + timeout paths
- `briefing::brief_validation` — rejects too-many-phases / over-long success_criteria
- `notify::{NtfyAdapter, WebhookAdapter, EmailAdapter}` — wiremock for ntfy/webhook; lettre's `StubTransport` for email
- `notify::Worker` — Redis-Streams consume + adapter dispatch (live-Redis `#[ignore]`)
- `task::pause_durable + resume_via_replay` — sandbox-gone path
- `deliverable::DeliverableStore` — CRUD + file-ref write/read
- DEBT #14 regression: `commit_phase_does_not_shell_inject` — feeds backtick / dollar / newline phase_title
- DEBT #15 regression: `worker_xreadgroup_drives_handle_request` (live-Redis `#[ignore]`)

### Integration (server)

- `tests/phase2_briefing.rs` — full task_create → Briefing event → confirm → first iteration
- `tests/phase2_briefing_edit.rs` — confirm → edit → re-emitted Briefing
- `tests/phase2_overnight_scaled.rs` — 24 h durability test scaled to ~5 min via `tokio::time::pause`
- `tests/phase2_notify_chain.rs` — task_finished → notify_request → ntfy adapter wiremock'd
- `tests/phase2_resume_from_replay.rs` — pause → kill container → resume rebuilds from event stream
- `tests/phase2_deliverable_workflow.rs` — task → task_deliver tool → Deliverable event + file_ref + citations

### Frontend (Playwright — closes DEBT #9)

- `briefing_card.spec.ts` — render, confirm/edit/cancel
- `projects.spec.ts` — ProjectList nav, task summary cards
- `deliverables.spec.ts` — Deliverables tab render + citation chip
- `decisions.spec.ts` — Decisions pane filter
- Regression smoke: existing Chat / Verifier / 3-track Browser still render

### E2E (live-LLM workflow_dispatch — extends Phase 1's `phase1-live-smoke`)

- `phase2-live-overnight`: real briefing flow → resume after 30 min mock-clock advance → verifier passes → notification fires → deliverable persisted. Gated on `ANTHROPIC_API_KEY` + `OPENAI_API_KEY` + `SEASONED_HAND_PHASE2_SMOKE=1`.

### Acceptance gate (per ROADMAP)

"Do this overnight" workflow runs end-to-end against a real Bifrost
config:
1. user submits `task_create` with a multi-phase brief
2. Briefing event fires + user confirms
3. Task runs for ≥ 8h wall (extrapolation from the 5-min scaled test)
4. At least one durable pause + resume cycle
5. Deliverable written
6. Notification fires
7. Verifier verdict pass

---

## 12. Open technical questions

1. **Email adapter trust model**: env-loaded SMTP creds for Phase 2 single-user. Phase 5 multi-user may need per-user SMTP relay tokens (SES, Postmark API key). My read: defer to Phase 5; Phase 2 ships env+lettre.

2. **Briefing auto-confirm default**: 5-min timeout → auto-run is the "digital employee" UX. Should this be 0 (always wait for human) by default with users opting INTO auto-confirm? Decision affects the "Do this overnight" UX — if the user submits a task at 11 PM and goes to sleep, auto-confirm at 11:05 PM is required for the overnight flow to work.

3. **Project-level cost cap**: Phase 1 has per-session cost cap. Should projects get aggregate caps? My read: no for Phase 2 (per-task cap is the right granularity); Phase 5 multi-user may add per-org caps.

4. **Deliverable formats beyond markdown + json**: HTML (sandboxed iframe render)? PDF (server-side render)? My read: defer to Phase 5. Markdown covers the "overnight summary" UX.

5. **Status dashboard refresh mechanism**: Polling vs WS-event-driven? Phase 1 lifted the WS hook to HomeShell for VerifierTab. ProjectList + status dashboard reuse the same pattern. Confirmed implicit in Fork B1.

6. **Notification trigger config granularity**: per-user (env/TOML) for Phase 2; per-org in Phase 5. Confirm before drafting stories.

7. **Workspace TTL value**: Phase 2 pays down DEBT #16 (workspace cleanup cron). Default TTL for an active task: never. For a `paused` task: 7 days. For a `completed` task: 30 days. For `failed`/`cancelled`: 7 days. Open question: should `completed` retention be operator-configurable from day one?

---

Architecture is at `/specs/phase-2/architecture.md`. When approved,
start a fresh session with the PM persona to break this into stories.
