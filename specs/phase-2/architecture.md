# Phase 2 — Architecture (Employee Interface, OS-shape)

> **Status**: v2.0 (BMAD Architect persona output, 2026-05-13, post-OS-shape commit)
> **Duration**: 5 weeks (extended from ROADMAP's 3 weeks — see §0)
> **Base commit**: `bfaa56e` (Phase 1 hardened, Phase 2 v1.0 in place)
> **Goal**: A digital employee team that lives behind an OS-shaped
> control plane — work comes in over any channel, gets executed by a
> fixed team of specialized roles, and leaves as the artifact form the
> work demands, routed back to the channel that asked for it.
> **Acceptance**: "Do this overnight" works end-to-end via at least
> two distinct channels (e.g., chat + email), produces a real-employee
> deliverable (e.g., a rendered `.docx` / `.pptx` / `.xlsx`), and the
> deliverable carries a complete provenance manifest.

---

## 0. Inputs and methodology note

This v2.0 supersedes the v1.0 drafted earlier in the same architect
session. v1.0 was a faithful translation of ROADMAP §Phase 2 + Phase 1
retrospective into a single-track "outbound notification + markdown
deliverable" design. v2.0 absorbs four substantive corrections raised
during user review of v1.0:

1. **"Agent" → "agent team"**: the system is already a team of
   specialized roles (Initializer / Worker / Verifier / Narrator /
   Plan Manager / Checkpoint Manager / Notify Worker / Curator).
   v2.0 names this explicitly without changing the underlying
   architecture. The marketing surface stays "digital employee team";
   the OS-layer language stays internal-facing (BASELINE / contributor
   docs).

2. **Deliverables must be real-employee artifacts**: `.docx` /
   `.pptx` / `.xlsx`, code, deployed URLs — not just markdown. v2.0
   adds a rendering pipeline (Pandoc / python-pptx / openpyxl) in the
   sandbox image.

3. **Channels are symmetric**: inbound (intake) and outbound
   (delivery + notify) must be the same pluggable abstraction. A task
   that came in via Slack returns via Slack by default. v2.0 introduces
   the **Adapter trait family** as the architectural keystone.

4. **OS-layer commitment** (Option A, 5 weeks): adapter abstraction
   + intake symmetry + provenance manifest + tenancy columns +
   skill/workflow reservation + CLI all land in Phase 2 as a single
   coherent foundation. The user explicitly chose this over a phased
   rollout (Phase 2 + Phase 2.5) because OS-shape is a coherence
   property — partial OS is not an OS.

Forks resolved before drafting v2.0:

- **A1**: separate `projects` + `tasks` tables; `sessions` becomes a
  child of `tasks`. (v1.0 — unchanged.)
- **B1**: structured `Briefing` event + WS confirmation round-trip.
  (v1.0 — unchanged.)
- **C1**: Rust-native `notify` worker + adapters. (v1.0 — but now
  generalized: NotifyAdapter is one instance of the Adapter trait
  family in v2.0.)
- **24h durability**: container-GC-survivable resume via event-stream
  replay. (v1.0 — unchanged.)
- **5 weeks confirmed**: Option A.
- **CLI confirmed**: minimal `seasoned-hand` binary in Phase 2.
- **OS-layer framing**: BASELINE.md §4 gets one line; README stays
  digital-employee-team-led. Public OS-layer category claim deferred
  to Phase 4+ retrospective.

---

## 1. Summary diagram

```
            ╔═══════════════════════════ Layers above ═══════════════════════════╗
            ║                                                                     ║
        Humans                       Other systems                     User scripts
        (chat /                      (webhooks, cron,                  (CLI, SDK,
         email /                      ticket systems,                   future MCP)
         Slack-                       calendar)
         later /
         voice-P6+)
            │                              │                                │
            │                              │                                │
┌───────────▼──────────────────────────────▼────────────────────────────────▼────────┐
│                      Intake adapters (IntakeAdapter trait)                          │
│  ┌────────────────┐  ┌────────────────┐  ┌────────────────┐  ┌─────────────────┐  │
│  │ ChatIntake     │  │ WebhookIntake  │  │ EmailIntake    │  │ CliIntake       │  │
│  │ (existing WS)  │  │ (POST /v1/...) │  │ (IMAP poller)  │  │ (CLI commands)  │  │
│  └────────────────┘  └────────────────┘  └────────────────┘  └─────────────────┘  │
│        │                    │                    │                    │            │
│        └────────────────────┴────────────────────┴────────────────────┘            │
│                                       │                                            │
│                  uniform IntakeEvent { brief_input, reply_target, ... }            │
│                                       ▼                                            │
│  ┌──────────────────────────────────────────────────────────────────────────┐     │
│  │                  KERNEL — agent team + event stream                      │     │
│  │  ┌────────────┐ ┌──────────┐ ┌───────────┐ ┌──────────┐ ┌────────────┐  │     │
│  │  │Initializer │ │ Worker   │ │ Verifier  │ │ Narrator │ │ Curator    │  │     │
│  │  │ (1.4 +     │ │ (Phase 0 │ │ (1.9b+1.10│ │ (1.15)   │ │ (Phase 4+) │  │     │
│  │  │  briefing) │ │  ReAct)  │ │ +DEBT #15)│ │          │ │            │  │     │
│  │  └────────────┘ └──────────┘ └───────────┘ └──────────┘ └────────────┘  │     │
│  │                                                                          │     │
│  │  Plan Manager (1.1) │ Checkpoint Mgr (1.13+1.13b+DEBT #14) │ Breakers   │     │
│  │                                                                          │     │
│  │  Persistence (SQLite WAL): projects · tasks · sessions · plans ·         │     │
│  │       verifications · checkpoints · deliverables · intake_events ·       │     │
│  │       delivery_events · notifications_sent · provenance manifests        │     │
│  │       (V006 → V009 migrations; all rows carry `tenant_id` nullable)      │     │
│  │                                                                          │     │
│  │  Sandbox per session (Phase 0) + renderer toolchain (Pandoc /            │     │
│  │      python-pptx / openpyxl) + git working tree (1.3)                    │     │
│  └──────────────────────────────────────────────────────────────────────────┘     │
│                                       │                                            │
│         uniform Deliverable + Provenance / NotifyEvent / ServerEvent               │
│                                       ▼                                            │
│  ┌────────────────────────────────────────────────────────────────────────────┐   │
│  │           Outbound adapter family (DeliveryAdapter + NotifyAdapter)        │   │
│  │  ┌────────────────┐  ┌────────────────┐  ┌────────────────┐                │   │
│  │  │ WebhookDeliver │  │ EmailDeliver   │  │ ChatDeliver    │  (default:     │   │
│  │  │ (POST callback)│  │ (reply to MID) │  │ (WS event)     │   route to     │   │
│  │  └────────────────┘  └────────────────┘  └────────────────┘   intake.adapter│  │
│  │  ┌────────────────┐  ┌────────────────┐  ┌────────────────┐                │   │
│  │  │ NtfyNotify     │  │ WebhookNotify  │  │ EmailNotify    │                │   │
│  │  └────────────────┘  └────────────────┘  └────────────────┘                │   │
│  └────────────────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────────────────┘

Phase 4+ (not in this spec, but reserved): SlackIntake/Delivery,
NotionDelivery, GoogleDriveDelivery, GitHubDeliveryAdapter (PR creation),
VoiceIntake (Whisper), CalendarIntake. All slot in as new Adapter impls.
```

---

## 2. New components introduced

### 2.1 Project / Task hierarchy

**Module**: `seasoned-hand-core::project` (new).

Same as v1.0:

- `Project { id, title, description?, status, tenant_id, created_at, updated_at }`
- `Task { id, project_id, title, brief?, status, expected_due_at?, completed_at?, failure_reason?, parent_task_id?, schedule?, skill_attached_event_id?, tenant_id, created_at, updated_at }`
- `Session { …Phase 1 fields, + task_id FK }`

Task status state machine:
`drafted → briefed → confirmed → running ⇄ paused → completed | failed | cancelled`

The four new optional fields on `Task` (`parent_task_id`, `schedule`,
`skill_attached_event_id`, `tenant_id`) are **slot reservations** —
unused by Phase 2 logic but persisted as nullable for Phase 3-5 to
populate without a schema migration. This is the OS-shape principle:
"reserve the columns now, ship the feature later".

### 2.2 Briefing protocol

Same as v1.0 — `Initializer` extended with confirm gate, `briefing_call_id`
round-trip via WS `user_response`, 5-minute auto-confirm timeout (configurable). The Brief shape is unchanged:

```typescript
type Brief = {
  goal: string,
  phases: Array<{ id: number; title: string; capabilities?: string[] }>,
  success_criteria: string[],
  expected_deliverables: Array<DeliverableSpec>,   // see §2.3 — NEW shape
}

type DeliverableSpec = {
  filename: string,    // "Q4-summary.docx", "report.pdf", "data.xlsx"
  format: "docx" | "pdf" | "html" | "pptx" | "xlsx" | "csv" | "md" | "json" | "code" | "url",
  description?: string,
}
```

`expected_deliverables` in v2.0 carries **typed format hints** so the
renderer dispatcher (§2.3) knows what to produce. `code` and `url` are
Phase 2 placeholders — see §2.3 footnote.

### 2.3 Deliverable standards (multi-format pipeline)

**Module**: `seasoned-hand-core::deliverable` (new).

Phase 2 supports 8 deliverable formats, with the renderer dispatched
server-side based on the target filename's extension:

| Format | Renderer | Source content | Phase |
|---|---|---|---|
| `.md`, `.txt` | raw write | LLM-produced markdown | 2 ✅ |
| `.json` | raw write (validated) | structured JSON | 2 ✅ |
| `.docx`, `.pdf`, `.html`, `.odt` | **Pandoc CLI** | LLM-produced markdown | 2 ✅ |
| `.csv` | raw write | LLM-produced CSV text | 2 ✅ |
| `.xlsx` | **openpyxl** (Python script) | LLM-produced JSON `{sheets: [{name, rows: [[...]], formats?}]}` | 2 ✅ |
| `.pptx` | **python-pptx** (Python script) | LLM-produced JSON `{slides: [{title, body, layout?}]}` | 2 ✅ |
| `.png`, `.svg`, `.mmd` (diagrams) | Graphviz / Mermaid CLI | LLM-produced graph DSL | 2 🟡 stretch |
| code (git repo + optional URL) | (no rendering — the sandbox git tree itself is the deliverable; optional `deploy_expose_port` for live URL) | Worker writes files via existing tools | 2 ✅ (basic — git repo only); Phase 4 (full deploy + PR) |
| Slack post / Notion page / Drive upload | corresponding DeliveryAdapter | rendered file from above | Phase 4 (when those adapters land) |

**LLM-facing tool**: `task_deliver(content, target_filename, citations)`.

- `content` is **markdown** for prose/document formats, **JSON** for
  structured formats (xlsx, pptx, json).
- `target_filename` carries the extension; the dispatcher infers the
  renderer.
- `citations: Vec<i64>` is an array of `event_id`s proving where the
  content's claims came from. This is the deliverable-level provenance;
  the full provenance manifest (§2.7) wraps this plus runtime metadata.

**Persistence**: `Deliverable { id, task_id, format, source_content_path,
rendered_path, content_sha256, content_size, citations, provenance_manifest,
created_at }`. Both the LLM source (markdown/JSON) and the rendered
artifact (docx/pptx/xlsx) are saved — source for verifier re-reads, rendered
for delivery.

**Renderer toolchain** lives in the sandbox image. Two options:

- **(a) startup-time install** in each new sandbox: `apt install -y
  pandoc && pip install python-pptx openpyxl` (~30-60 s one-time per
  session). Phase 2 default.
- **(b) pre-baked image** `seasoned-hand-sandbox:phase-2`: same
  packages baked in. Faster + reproducible, but introduces a custom
  image-publish step.

Phase 2 ships (a). `phase-2/DEBT #2` tracks the migration to (b) when
the renderer set stabilizes (probably Phase 4).

### 2.4 Status reporting dashboard (backend)

Same as v1.0:

- `GET /v1/projects?limit&cursor` → `ListResponse<Project>`
- `GET /v1/projects/:id` → `Project + task counts`
- `POST /v1/projects` → create
- `PATCH /v1/projects/:id` → rename / archive
- `GET /v1/projects/:id/tasks?status&limit&cursor` → `ListResponse<Task>`
- `GET /v1/tasks/:id` → full Task incl. sessions
- `GET /v1/tasks/:id/deliverables` → `ListResponse<Deliverable>`
- `GET /v1/tasks/:id/notifications` → `ListResponse<NotificationSent>`

v2.0 adds:

- `GET /v1/tasks/:id/intake` → originating IntakeEvent
- `GET /v1/tasks/:id/deliveries` → `ListResponse<DeliveryEvent>` (where each
  deliverable was routed and the result)
- `GET /v1/tasks/:id/provenance` → the full provenance manifest as
  one consolidated JSON
- `GET /v1/adapters` → registered adapter names + health
- `GET /v1/adapters/:name/health` → individual adapter health

### 2.5 Accountability trail

Same as v1.0 — `Misc{kind:"decision"}` events emitted by Initializer,
Verifier, Checkpoint Manager. The provenance manifest §2.7 references
these via `decision_event_ids`.

### 2.6 Long-running task durability (24h+)

Same as v1.0 — soft pause + durable freeze tiers, sandbox-gone resume
rebuilds from event-stream replay. The provenance manifest carries the
full session list, so a 24h task with 3 pause/resume cycles produces a
single Deliverable with `sessions: [S1, S2, S3, S4]` in its manifest.

### 2.7 Adapter framework (NEW — the OS-shape keystone)

**Module**: `seasoned-hand-core::adapter` (new).

Three trait families, all `Send + Sync`, all `async_trait`:

```rust
// Intake — receive briefs from external channels
#[async_trait]
pub trait IntakeAdapter {
    fn name(&self) -> &'static str;
    /// Run the adapter's lifecycle: poll, push events through `sink`,
    /// stop cleanly on `shutdown`. Each adapter owns its own loop
    /// (HTTP server, IMAP poller, websocket subscriber, etc.).
    async fn run(
        &self,
        sink: mpsc::Sender<IntakeEvent>,
        shutdown: CancellationToken,
    ) -> Result<(), AdapterError>;
}

// Delivery — send completed deliverables back to the originating
//            channel (or a different one if the task overrode).
#[async_trait]
pub trait DeliveryAdapter {
    fn name(&self) -> &'static str;
    async fn deliver(
        &self,
        target: &DeliveryTarget,
        deliverable: &Deliverable,
    ) -> Result<DeliveryReceipt, AdapterError>;
}

// Notify — send status/event signals (not artifacts) to external channels.
//          A subset of DeliveryAdapter in spirit but with different cardinality
//          (many notifies per task; usually one deliverable per task).
#[async_trait]
pub trait NotifyAdapter {
    fn name(&self) -> &'static str;
    async fn notify(
        &self,
        target: &NotifyTarget,
        event: &NotifyEvent,
    ) -> Result<NotifyReceipt, AdapterError>;
}
```

**Shared types**:

```rust
pub struct IntakeEvent {
    pub adapter: String,                  // "webhook", "email", "chat", "cli"
    pub intake_id: String,                // unique within adapter (e.g., HTTP req id, IMAP UID, Message-ID)
    pub brief_input: String,              // natural-language brief
    pub reply_target: Option<DeliveryTarget>,  // for symmetric routing
    pub received_at: i64,
    pub metadata: serde_json::Value,      // adapter-specific (sender, subject, signatures, etc.)
    pub tenant_id: Option<String>,        // multi-tenant ready
}

pub struct DeliveryTarget {
    pub adapter: String,                  // matches DeliveryAdapter::name()
    pub target_ref: String,               // adapter-specific (e.g., "thread:T01/C01/1234567890.123", "msgid:<...>", "url:https://example.com/cb")
    pub metadata: serde_json::Value,
}

pub struct DeliveryReceipt {
    pub adapter: String,
    pub external_id: String,              // adapter-specific (e.g., posted msg id)
    pub delivered_at: i64,
    pub raw_response: serde_json::Value,  // for audit
}
```

**Registration**: at boot, `AppState::register_intake_adapter(impl
IntakeAdapter)`, `register_delivery_adapter(impl DeliveryAdapter)`,
`register_notify_adapter(impl NotifyAdapter)`. Adapters are spawned
in dedicated tokio tasks (intake adapters loop; delivery + notify are
called on-demand). One adapter can implement multiple traits (e.g.,
the email adapter is both Intake and Delivery).

**Phase 2 ships these concrete adapters**:

| Adapter | Intake | Delivery | Notify | Notes |
|---|---|---|---|---|
| Webhook | ✅ | ✅ | ✅ | The minimum-viable OS surface — any system can speak to us via HTTP. |
| Email (lettre + IMAP) | ✅ | ✅ | ✅ | The most natural non-technical inbound channel. IMAP poll every 30 s default. |
| Chat (WebSocket) | ✅ (existing) | ✅ | — | Wraps Phase 0's WS as an adapter for uniformity. Deliveries surface as a Chat-pane card. |
| CLI | ✅ | ✅ | — | `seasoned-hand task new ...` lands as an IntakeEvent. Output goes to stdout. |
| Ntfy | — | — | ✅ | Existing v1.0 design — generalized to a NotifyAdapter impl. |

**Phase 4+ adapters** (reserved slots, not in Phase 2):
- Slack (auth-dependent — multi-user)
- Notion / Google Drive / SharePoint (auth-dependent)
- GitHub PR / GitLab MR (auth-dependent)
- Voice (Whisper + TTS — needs Phase 6 audio stack)
- Calendar (Google Calendar / iCal — recurring tasks need Phase 5 scheduler)

### 2.8 Intake protocol (NEW)

Inbound symmetry to the outbound NotifyWorker from v1.0.

```
IntakeAdapter (running in its own tokio task) →
  emits IntakeEvent through the registered mpsc::Sender →
IntakeRouter (one per AppState) →
  validates + persists to intake_events table →
  creates Task via TaskStore (status: drafted) →
  spawns Initializer with the brief_input →
  records intake_id ↔ task_id mapping →
  Briefing protocol takes over (§2.2)
```

**Webhook intake** (`POST /v1/intake/webhook`):
```json
{
  "brief": "Summarize Q4 board deck and email it back",
  "project_id": "default-or-inbox",      // optional
  "reply_target": {                       // optional; defaults to "callback to webhook URL with task_id + deliverable"
    "adapter": "email",
    "target_ref": "msgid:<original-message-id@example.com>",
    "metadata": { "to": "user@example.com" }
  },
  "metadata": { "from_system": "zapier", "auth_token_hint": "..." }
}
```

Response: `202 Accepted` with `{task_id, briefing_call_id}`. The
callback URL receives a POST when the task completes with the
deliverable payload.

**Email intake** (IMAP poller, default 30 s interval):
- Connects to configured IMAP server (env: `IMAP_HOST`, `IMAP_USERNAME`,
  `IMAP_PASSWORD`)
- Filters by `seasoned-hand` label or `+sh@user-mailbox` sub-address
  (configurable)
- Each new email becomes one IntakeEvent with `reply_target = {adapter:
  "email", target_ref: "msgid:<...>"}` so the deliverable goes back as
  a reply to the same thread
- Attachments are placed in `/workspace/.intake/<intake_id>/` for the
  Worker to access

**CLI intake** (handled by §2.10):
`seasoned-hand task new "<brief>" [--project ID]` becomes an
IntakeEvent with `reply_target = {adapter: "cli", target_ref:
"stdout:<pid>"}`. The CLI process blocks until the deliverable is
ready (or returns the task_id with `--detach`).

**Chat intake** (existing WS, now wrapped):
The existing `cmd: "task_create"` WS verb is the chat adapter's
intake mechanism. Phase 2 doesn't change the wire; it just labels
this stream as `IntakeEvent { adapter: "chat", ... }` internally.

### 2.9 Delivery protocol (NEW)

When a Task transitions to `completed`, the Deliverable's
`provenance_manifest.delivered_to` is populated by routing through
the DeliveryAdapter matching the task's `reply_target`:

```
Task.status → completed
  → Deliverable persisted (rendered file in /workspace/.deliverables/)
  → DeliveryRouter:
      target = task.reply_target (or default = task.intake.reply_target)
      lookup adapter by target.adapter name
      → adapter.deliver(target, deliverable)
      → on success: append DeliveryEvent { ok: true, external_id, delivered_at }
      → on failure: 1 retry after 30 s (transient case); then mark failed
                   in DeliveryEvent and emit Misc{kind:"delivery_failed"}
  → NotifyWorker fires task_finished notification (separate from delivery)
```

**Webhook delivery**: POSTs `{task_id, deliverable_id, content_url,
provenance_manifest, status}` to the callback URL. The callback URL
itself can use a presigned-URL pattern to fetch the actual artifact
bytes from `GET /v1/tasks/:id/deliverables/:did/content`.

**Email delivery**: replies to the Message-ID in `reply_target` with
the rendered artifact as an attachment. Subject prefixed with `[Re:
Original Subject]`.

**Chat delivery**: emits a `Deliverable` WS event into the session
that originated the task. Frontend renders as a downloadable card.

**CLI delivery**: writes the deliverable path to stdout and (with
`--open` flag) shells out to the OS open command.

### 2.10 CLI (NEW)

**Crate**: `seasoned-hand-cli` (new binary).

Surface (Phase 2 minimum):

```
seasoned-hand --version
seasoned-hand init                              # bootstrap ~/.seasoned-hand/

seasoned-hand server                            # alias for `cargo run -p seasoned-hand-server`
                                                # (loads config from ~/.seasoned-hand/)

seasoned-hand project list                      # GET /v1/projects
seasoned-hand project create TITLE [--description ...]
seasoned-hand project archive ID

seasoned-hand task new "<brief>" [--project ID] [--detach] [--no-auto-confirm]
                                                # Default: blocks until completion
                                                # --detach: returns task_id immediately
seasoned-hand task list [--project ID] [--status STATUS] [--limit N]
seasoned-hand task show ID                      # full task + sessions + deliverables
seasoned-hand task pause ID [--durable]         # durable defaults true
seasoned-hand task resume ID
seasoned-hand task cancel ID
seasoned-hand task brief ID                     # show the parsed Brief JSON
seasoned-hand task deliverable ID [--open] [--save PATH]
seasoned-hand task provenance ID                # show the provenance manifest

seasoned-hand inbox                             # list briefings awaiting confirmation
seasoned-hand brief confirm BRIEFING_ID
seasoned-hand brief edit BRIEFING_ID [--editor]
seasoned-hand brief cancel BRIEFING_ID

seasoned-hand adapter list                      # registered adapters + health
seasoned-hand adapter test NAME                 # synthetic round-trip test
seasoned-hand adapter logs NAME [--tail]        # adapter-specific log stream
```

The CLI talks to the running server over HTTP (default
`http://127.0.0.1:3000`). It is a **thin HTTP client** plus the
inline CliIntakeAdapter for `task new`. No background daemon.

This is what makes Seasoned Hand an OS-layer rather than a web app:
**every UI action has a clean CLI equivalent**. No UI-only features.
The web frontend becomes one of several frontends, not THE frontend.

### 2.11 Provenance manifest (NEW — mandatory)

Every Deliverable carries a complete manifest as a JSON blob:

```typescript
type ProvenanceManifest = {
  schema_version: 1,
  task_id: string,
  project_id: string,
  tenant_id?: string,                    // null in single-tenant Phase 2

  intake: {
    adapter: string,
    intake_id: string,
    received_at: number,
    metadata: object,                    // redacted at audit time
  },

  brief: {
    brief_event_id: number,              // points to the Briefing event
    confirmed: boolean,                  // false = auto-confirmed via timeout
    confirmed_at?: number,
    edits_applied: number,               // how many edit cycles before confirm
  },

  sessions: Array<{
    id: string,
    started_at: number,
    ended_at?: number,
    end_reason?: "completed" | "paused" | "cancelled" | "failed",
  }>,

  decisions: number[],                   // event_ids of Misc{kind:"decision"}
  verifier_verdicts: string[],           // verification row ids
  checkpoints: Array<{
    checkpoint_id: string,
    git_sha: string,
    rolled_back: boolean,
  }>,

  metrics: {
    tool_calls: number,
    cost_cents: number,
    wall_seconds: number,
    sessions_count: number,
    pause_resume_cycles: number,
    verifier_runs: number,
  },

  delivered_to: Array<{
    adapter: string,
    delivery_id: string,                 // DeliveryEvent.id
    delivered_at: number,
    ok: boolean,
    external_id?: string,                // adapter-side id
  }>,

  // Optional / format-specific
  source_content_sha256?: string,        // for documents
  rendered_content_sha256: string,
  citations: number[],                   // event_ids inline in the deliverable content
}
```

The manifest is computed at deliverable persist time and stored on the
`deliverables.provenance_manifest` column (JSON TEXT). It's queryable
via `GET /v1/tasks/:id/provenance`.

This is the OS-level guarantee: **"I can always answer 'where did this
come from?'"** Every claim in a deliverable points to evidence in the
event stream; the manifest is the index.

### 2.12 DEBT close-outs landing IN Phase 2

Unchanged from v1.0:

- **DEBT #15** — real Verifier `XREADGROUP` loop in
  `verifier::worker::Worker::run`. Required for "Do this overnight".
- **DEBT #14** — `SandboxGitShell::commit_phase` shell-quoting fix.
  Required BEFORE Plan{op:"advance"} fanout broadcaster activates.
- **DEBT #9** — Frontend Playwright bootstrap + coverage for new UI
  surfaces.
- **DEBT #3** — Verifier rollback default flip decision (data-driven,
  Phase 2 closeout).
- **NarratorHook classifier-slot wiring** through `AppState::new`.
- **Phase 0 DEBT #16** — workspace TTL + cleanup cron (was already
  carried in v1.0 §2.6 durable pause).

---

## 3. Data model changes

### V006 — Project / Task baseline + tenancy + skill slots

```sql
CREATE TABLE projects (
    id          TEXT    PRIMARY KEY,
    tenant_id   TEXT,                          -- NEW: nullable in Phase 2, NOT NULL in Phase 5
    title       TEXT    NOT NULL,
    description TEXT,
    status      TEXT    NOT NULL,              -- 'active' | 'archived'
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE TABLE tasks (
    id                        TEXT    PRIMARY KEY,
    project_id                TEXT    NOT NULL REFERENCES projects(id),
    tenant_id                 TEXT,            -- mirrors project.tenant_id
    title                     TEXT    NOT NULL,
    brief                     TEXT,            -- JSON Brief; NULL until briefed
    status                    TEXT    NOT NULL,
    expected_due_at           INTEGER,
    completed_at              INTEGER,
    failure_reason            TEXT,
    parent_task_id            TEXT REFERENCES tasks(id),    -- RESERVED: Phase 5 workflow composition
    schedule                  TEXT,                          -- RESERVED: Phase 5 cron-like recurring
    skill_attached_event_id   INTEGER,                       -- RESERVED: Phase 3 learning
    created_at                INTEGER NOT NULL,
    updated_at                INTEGER NOT NULL
);

ALTER TABLE sessions ADD COLUMN task_id TEXT REFERENCES tasks(id);

CREATE INDEX idx_projects_tenant_status     ON projects(tenant_id, status);
CREATE INDEX idx_tasks_project_status       ON tasks(project_id, status);
CREATE INDEX idx_tasks_parent               ON tasks(parent_task_id);
CREATE INDEX idx_tasks_schedule             ON tasks(schedule) WHERE schedule IS NOT NULL;
CREATE INDEX idx_sessions_task_id           ON sessions(task_id);
```

### V007 — Deliverables + provenance

```sql
CREATE TABLE deliverables (
    id                       TEXT    PRIMARY KEY,
    task_id                  TEXT    NOT NULL REFERENCES tasks(id),
    tenant_id                TEXT,
    format                   TEXT    NOT NULL,    -- 'docx' | 'pdf' | 'html' | 'pptx' | 'xlsx' | 'csv' | 'md' | 'json' | 'code' | 'url'
    source_content_path      TEXT,                -- workspace path of LLM source (markdown/JSON), nullable for raw
    source_content_sha256    TEXT,
    rendered_content_path    TEXT    NOT NULL,    -- workspace path of rendered artifact
    rendered_content_sha256  TEXT    NOT NULL,
    content_size             INTEGER NOT NULL,
    citations                TEXT,                -- JSON array of event_ids
    provenance_manifest      TEXT    NOT NULL,    -- JSON ProvenanceManifest (§2.11)
    created_at               INTEGER NOT NULL
);

CREATE INDEX idx_deliverables_task   ON deliverables(task_id);
CREATE INDEX idx_deliverables_tenant ON deliverables(tenant_id);
```

### V008 — Intake / Delivery / Notify event log

```sql
CREATE TABLE intake_events (
    id              TEXT    PRIMARY KEY,
    tenant_id       TEXT,
    adapter         TEXT    NOT NULL,
    intake_id       TEXT    NOT NULL,         -- unique-per-adapter external id
    brief_input     TEXT    NOT NULL,
    reply_target    TEXT,                     -- JSON DeliveryTarget
    metadata        TEXT,                     -- JSON
    task_id         TEXT REFERENCES tasks(id),  -- populated once Task is created
    received_at     INTEGER NOT NULL,
    UNIQUE (adapter, intake_id)
);

CREATE TABLE delivery_events (
    id              TEXT    PRIMARY KEY,
    tenant_id       TEXT,
    task_id         TEXT    NOT NULL REFERENCES tasks(id),
    deliverable_id  TEXT    NOT NULL REFERENCES deliverables(id),
    adapter         TEXT    NOT NULL,
    target          TEXT    NOT NULL,         -- JSON DeliveryTarget
    ok              INTEGER NOT NULL,
    external_id     TEXT,                     -- adapter-side id (e.g., posted msg id)
    error           TEXT,
    delivered_at    INTEGER NOT NULL
);

CREATE TABLE notifications_sent (
    id              TEXT    PRIMARY KEY,
    tenant_id       TEXT,
    task_id         TEXT,                     -- nullable for pre-task notifies
    trigger_kind    TEXT    NOT NULL,
    adapter         TEXT    NOT NULL,
    target          TEXT,                     -- JSON NotifyTarget
    payload         TEXT,                     -- JSON (redacted)
    ok              INTEGER NOT NULL,
    error           TEXT,
    sent_at         INTEGER NOT NULL
);

CREATE INDEX idx_intake_task         ON intake_events(task_id);
CREATE INDEX idx_intake_adapter      ON intake_events(adapter, received_at);
CREATE INDEX idx_delivery_task       ON delivery_events(task_id);
CREATE INDEX idx_delivery_deliv      ON delivery_events(deliverable_id);
CREATE INDEX idx_notifs_task         ON notifications_sent(task_id);
```

### V009 — Skill / playbook reservation (empty in Phase 2)

```sql
-- Phase 3 will populate. Phase 2 creates tables for forward-compat
-- and to make `phase-2/architecture.md`'s "reservation" claim concrete.

CREATE TABLE skills (
    id              TEXT    PRIMARY KEY,
    tenant_id       TEXT,
    title           TEXT    NOT NULL,
    summary         TEXT,
    schema_version  INTEGER NOT NULL,
    source_task_id  TEXT REFERENCES tasks(id),     -- Phase 3: skill learned from this task
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE TABLE playbooks (
    id              TEXT    PRIMARY KEY,
    tenant_id       TEXT,
    title           TEXT    NOT NULL,
    content_path    TEXT    NOT NULL,                -- markdown playbook on disk
    schema_version  INTEGER NOT NULL,
    source_task_id  TEXT REFERENCES tasks(id),     -- Phase 3: playbook extracted from this task
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE INDEX idx_skills_tenant     ON skills(tenant_id);
CREATE INDEX idx_playbooks_tenant  ON playbooks(tenant_id);
```

---

## 4. API surface

### New HTTP routes

```
# Projects + Tasks (read)
GET    /v1/projects?limit&cursor
GET    /v1/projects/:id
POST   /v1/projects { title, description? }
PATCH  /v1/projects/:id { title?, status? }
GET    /v1/projects/:id/tasks?status&limit&cursor

GET    /v1/tasks/:id
GET    /v1/tasks/:id/deliverables
GET    /v1/tasks/:id/deliverables/:did/content    # raw bytes of rendered artifact
GET    /v1/tasks/:id/notifications
GET    /v1/tasks/:id/intake                       # the originating IntakeEvent
GET    /v1/tasks/:id/deliveries                   # where the deliverable was sent
GET    /v1/tasks/:id/provenance                   # full ProvenanceManifest

# Briefing
POST   /v1/briefings/:briefing_call_id/confirm    { action, edits? }   # also doable via WS user_response

# Intake
POST   /v1/intake/webhook                         # generic webhook intake
                                                  # body: { brief, project_id?, reply_target?, metadata? }
                                                  # 202: { task_id, briefing_call_id }

# Adapters
GET    /v1/adapters                               # list registered adapters + health
GET    /v1/adapters/:name/health
POST   /v1/adapters/:name/test                    # synthetic round-trip test

# Notify config
GET    /v1/notify/config
```

All read routes use the shared `RouteOutcome<T>` from the Phase 1
simplicity pass.

### WS envelope additions

```typescript
// ClientCommand additions
| { cmd: "task_create"; project_id?: string; title?: string; input: string;
    durable?: boolean; max_steps?: number; cost_cap_cents?: number }
| { cmd: "briefing_confirm"; task_id: string;
    in_reply_to_call_id: string;
    action: "confirm" | "edit" | "cancel";
    edits?: PartialBrief }
| { cmd: "task_pause";  task_id: string; durable?: boolean }
| { cmd: "task_resume"; task_id: string }
| { cmd: "task_cancel"; task_id: string }

// Phase 1 legacy { cmd:"task_create", input } still works — auto-creates
// "Inbox" project + a task.

// ServerEvent payload additions
| { kind: "Briefing"; briefing_call_id: string; goal: string;
    phases: Phase[]; success_criteria: string[];
    expected_deliverables: DeliverableSpec[] }
| { kind: "Deliverable"; deliverable_id: string; format: string;
    file_ref: FileRef; citations: number[] }

// `decision`, `briefing_pending`, `briefing_auto_confirmed`, `task_state`,
// `intake_received`, `delivery_attempted`, `delivery_failed`,
// `task_resume_rebuild_required`, `skill_invoked` (reserved Phase 3)
// are all Misc.kind_tag strings — no new EventType variants. Matches
// the Phase 1 pattern for browser_track_*, narration_skipped.
```

### Internal Rust APIs

- `seasoned_hand_core::project::{ProjectStore, TaskStore, Brief, DeliverableSpec}`
- `seasoned_hand_core::deliverable::{DeliverableStore, Deliverable, RendererDispatcher}`
- `seasoned_hand_core::adapter::{IntakeAdapter, DeliveryAdapter,
   NotifyAdapter, IntakeEvent, DeliveryTarget, NotifyTarget,
   AdapterError}`
- `seasoned_hand_core::intake::{IntakeRouter, IntakeEventStore,
   WebhookIntakeAdapter, EmailIntakeAdapter, ChatIntakeAdapter,
   CliIntakeAdapter}`
- `seasoned_hand_core::delivery::{DeliveryRouter, DeliveryEventStore,
   WebhookDeliveryAdapter, EmailDeliveryAdapter, ChatDeliveryAdapter,
   CliDeliveryAdapter}`
- `seasoned_hand_core::notify::{NotifyWorker, NotificationsSentStore,
   NtfyNotifyAdapter, WebhookNotifyAdapter, EmailNotifyAdapter}`
- `seasoned_hand_core::provenance::{ProvenanceManifest, build_manifest}`
- `seasoned_hand_core::skill::{SkillStore, PlaybookStore}` (empty
   Phase 2 — schemas only)
- `seasoned_hand_core::agent::init::Initializer::run_with_confirmation(...)`

### CLI surface

See §2.10 — `seasoned-hand` binary, thin HTTP client.

---

## 5. External dependencies

### New Rust crates

| Crate | Version | Used by | Justification |
|---|---|---|---|
| `lettre` | 0.11 | `notify::EmailNotifyAdapter`, `delivery::EmailDeliveryAdapter` | SMTP send. Pure Rust, tokio-rustls feature. |
| `mailparse` | 0.15 | `intake::EmailIntakeAdapter` | Parse incoming RFC 5322 emails (subject, body, attachments). |
| `async-imap` | 0.10 | `intake::EmailIntakeAdapter` | IMAP poller. Tokio-compatible. |
| `clap` | 4.x | `seasoned-hand-cli` | CLI argument parsing. |
| `colored` | 2.x | `seasoned-hand-cli` | Terminal output. |

### Reused (Phase 0/1)

- `reqwest` — Webhook adapters (intake POST, delivery POST, ntfy POST)
- `redis` — `notify_request` stream (and any future adapter that uses Redis for buffering)
- `serde` + `serde_json` — payload serialization
- `rusqlite` — Project / Task / Deliverable / Intake / Delivery /
  Notify / Skill / Playbook persistence
- `refinery` — V006 → V009 migrations
- `dashmap` — concurrency primitives
- `tokio` + `tokio-util` — async runtime + cancellation tokens

### Sandbox-side renderer toolchain (NEW)

Installed in the sandbox container at session-create time (Phase 2)
or pre-baked into a custom image (Phase 4, see §2.3 DEBT note):

| Tool | Use | Install |
|---|---|---|
| **Pandoc** | markdown → docx / pdf / html / odt | `apt install -y pandoc texlive-xetex` |
| **python-pptx** (Python) | structured JSON → pptx | `pip install python-pptx` |
| **openpyxl** (Python) | structured JSON → xlsx | `pip install openpyxl` |
| (Phase 2 stretch) **Graphviz**, **Mermaid CLI** | dot/mmd → svg/png | apt + npm |

### Frontend

| Package | Version | Justification |
|---|---|---|
| `@playwright/test` | ~1.x | Closes DEBT #9 — FE automated tests. Dev-only dep. |

### ARCHITECTURE.md addendum

The new `lettre`, `mailparse`, `async-imap`, `clap`, `colored` Rust
deps + Pandoc / python-pptx / openpyxl sandbox-side deps trigger an
`/specs/01-architecture/ARCHITECTURE.md` §1.1 one-line addendum per
the AGENTS.md rule. Add it under "Component layers" as: "Phase 2
adapter framework (lettre + mailparse + async-imap + clap + colored;
sandbox-side renderer toolchain: Pandoc + python-pptx + openpyxl)."
One-line edit; not a re-architect.

---

## 6. Interactions with existing components

### Plan Manager (Phase 1 1.1)

Plans become task-scoped: V006 adds `plans.task_id` (nullable; populated
at runtime, no SQL backfill).

### Verifier Worker (Phase 1 1.9b)

Polling stub replaced with real `XREADGROUP` loop. Closes DEBT #15.
Verdicts roll up to Task; the Task's "verifier status" badge surfaces
in the new ProjectList UI.

### Checkpoint Manager (Phase 1 1.13)

Real consumer of `Plan{op:"advance"}` events. Triggers DEBT #14 fix as
prereq.

### Initializer (Phase 1 1.4)

Extended to emit `Briefing` event + confirm gate (§2.2). Original
`Initializer::run` signature preserved for non-confirm-gate callers.

### NarratorHook (Phase 1 1.15)

Classifier-slot wiring through `AppState::new` lands as an early
Phase 2 story. Templates gain entries for `task_deliver`,
`briefing_confirm`.

### `AppState` (Phase 1 1.17 + 1.18)

Gains:
- `Arc<ProjectStore>`, `Arc<TaskStore>`, `Arc<DeliverableStore>`,
  `Arc<NotificationsSentStore>`, `Arc<IntakeEventStore>`,
  `Arc<DeliveryEventStore>`, `Arc<SkillStore>`, `Arc<PlaybookStore>`
- Adapter registries: `Arc<IntakeAdapterRegistry>`,
  `Arc<DeliveryAdapterRegistry>`, `Arc<NotifyAdapterRegistry>`
- IntakeRouter handle, DeliveryRouter handle, NotifyWorker handle

### Frontend (Phase 0/1)

- **HomeShell**: ProjectList (NEW left panel) above TaskList.
- **Chat**: Briefing-confirm card renderer for `Briefing` events.
- **AgentComputer**: gains "Deliverables", "Decisions" tabs.
- **TaskList** → **TaskListInProject**: takes `active_project_id`.

### Sandbox lifecycle (Phase 0 0.8 / Phase 1 1.2 / 1.3)

Session-create path gains the renderer-install step (60 s one-time per
session). Phase 0 DEBT #16 (workspace TTL cleanup) lands here:
- Active task: never GC
- Paused task: 7-day TTL
- Completed task: 30-day TTL (configurable)
- Failed/cancelled: 7-day TTL

---

## 7. Performance budget

| Component | Target |
|---|---|
| Briefing event emit (Initializer parse → emit) | < 500 ms p95 |
| User briefing-confirm round-trip | unbounded (waits for human); 5-min auto-confirm |
| `GET /v1/projects/:id/tasks` (50 rows) | < 100 ms p95 |
| **Intake**: webhook POST → IntakeEvent persisted | < 200 ms p95 |
| **Intake**: IMAP poll cycle (no new mail) | < 1 s p50 |
| **Intake**: IMAP message → IntakeEvent persisted | < 3 s p95 (including attachment download) |
| **Delivery**: ntfy/webhook POST (success) | < 500 ms p95 |
| **Delivery**: email reply (SMTP send) | < 5 s p95 |
| **Renderer**: markdown → docx (Pandoc) | < 2 s p95 (typical 5-page doc) |
| **Renderer**: JSON → pptx (python-pptx) | < 5 s p95 (typical 10-slide deck) |
| **Renderer**: JSON → xlsx (openpyxl) | < 2 s p95 (typical 100-row sheet) |
| **CLI**: `task new "..."` blocking call | unbounded (= task duration) |
| **CLI**: `task list` (50 rows) | < 200 ms p95 (including HTTP round-trip) |
| Sandbox durable freeze | < 3 s |
| Sandbox resume via existing container | < 10 s |
| Sandbox resume via event-stream replay | < 60 s |
| Verifier Worker XREADGROUP poll | `BLOCK 5000 COUNT 16` |
| 24h-continuous task wall budget | 24 h + 30 min slack |
| Phase 1 budgets (verifier latency, plan render, cost) | unchanged |

---

## 8. Failure modes

| Mode | Detection | Handling |
|---|---|---|
| Briefing confirmation timeout | watchdog in Initializer | Misc `briefing_auto_confirmed` + proceed. `briefing_require_confirm: true` overrides. |
| Sandbox container GC'd while paused | `task_resume` finds no handle | Replay rebuild from event stream into fresh sandbox. New Session row, same Task. |
| Event-stream replay corruption | replay step returns error | Task → `failed{reason:"replay_failed"}`. No silent recovery. |
| **Intake adapter offline** (e.g., IMAP server down) | adapter's run() returns error | Adapter re-tries with exponential backoff. Other adapters keep running. `GET /v1/adapters/:name/health` reports the failure. |
| **Intake brief parse failure** | Initializer LLM call returns malformed Brief | Reject with `intake_parse_failed` event; for webhook intake, respond 422 with error detail; for email intake, send a "couldn't understand your request" reply. |
| **Renderer toolchain missing** (sandbox install failed) | renderer dispatcher returns error | Task → `failed{reason:"renderer_missing"}`. Operator alerted via NotifyAdapter. |
| **Renderer rendering failure** (Pandoc / pptx broke on weird input) | renderer process non-zero exit | Re-try once with a "simplify content" LLM call to reduce content complexity. If still fails, deliverable persists as `format: "md"` fallback + warning Misc. |
| **Delivery adapter failure** (5xx, SMTP outage) | adapter returns Err | DeliveryEvent persisted with ok=0 + error. Webhook gets 1 retry after 30 s. Email + ntfy are best-effort. Misc `delivery_failed` emitted. The deliverable itself is still in DB; user can fetch via `GET /v1/tasks/:id/deliverables/:did/content`. |
| **Notification delivery failure** | adapter returns Err | notifications_sent row with ok=0. No retry (notifies are best-effort). |
| Concurrent task_pause + task_resume race | DB state machine | Existing 1.17 cancel-token serialization. |
| Verifier Worker crash between consume + XACK | Redis PEL retains message | Next consumer picks up. `handle_request` is idempotent on `triggered_at_event_id`. |
| Deliverable write during cancelled task | state check before write | Reject with Misc `task_deliver_after_cancel`. |
| Brief over-large (100 phases, etc.) | post-parse validation | Reject with `briefing_invalid{reason:"too_many_phases"}`. Caps: 20 phases, 50 success criteria, 20 deliverables. |
| 24h task hits memory ceiling | runtime telemetry | `task_memory_ceiling` Misc + soft pause. User resumes manually. |
| Notify worker can't reach Redis | producer XADD fails | Log + skip. Task state unaffected. |
| **CLI server unreachable** | HTTP error | Print `seasoned-hand: server at http://127.0.0.1:3000 not reachable. Start it with `seasoned-hand server`.` Exit code 2. |
| **CLI auth missing in multi-user (Phase 5)** | server returns 401 | Print `seasoned-hand: not authenticated. Run `seasoned-hand auth login` first.` (Phase 2: not applicable; reserved.) |

---

## 9. Security considerations

### DEBT #14 — `SandboxGitShell::commit_phase` shell injection

Lands BEFORE Plan{op:"advance"} broadcaster activates. Replace
`format!("git commit ... \"{title}\"")` with stdin-fed `git commit -F -`
via the sandbox `/v1/shell/exec` `stdin` field. Regression test feeds
backtick / dollar / newline.

### Webhook intake authentication

Phase 2 single-user: webhook intake is **token-authenticated**.
`POST /v1/intake/webhook` requires `X-Seasoned-Hand-Intake-Token`
header. Token loaded from env `SEASONED_HAND_INTAKE_TOKEN` (separate
from the admin-rollback token). Empty env disables the endpoint
(returns 503 `intake_token_not_configured`).

Same pattern as Phase 1 1.13b admin-rollback endpoint. Loopback guard
is **NOT** required for webhook intake (the whole point is remote
systems can call us), so the bind-host configuration matters: Phase 2
default still binds 127.0.0.1, but operator can flip to 0.0.0.0 +
firewall rules + token + (future Phase 5) TLS.

### Email intake authentication

The agent's IMAP account credentials are operator-supplied (env). The
INBOX inherits trust from the email account itself — anyone who can
send to that address can submit a brief. Mitigations:

- **Allow-list**: `INTAKE_EMAIL_ALLOWED_SENDERS` env (comma-separated
  email regex). Default: deny all. Operator must whitelist their own
  address.
- **Subject prefix gate**: only messages whose subject starts with a
  configurable prefix (default `[sh]`) are considered intakes. Mail
  without the prefix is left in inbox untouched.

### Webhook delivery URL

Phase 2: operator-configured per-task (in the IntakeEvent payload).
**SSRF risk**: untrusted webhook intake might supply a `reply_target.url`
pointing at internal IPs (`http://10.0.0.1/admin`). Mitigation: the
webhook DELIVERY adapter rejects `target.target_ref` URLs that resolve
to private / link-local / loopback addresses **unless** an operator
allow-list bypasses the check.

Add to `phase-2/DEBT.md #1`: SSRF protection lives in the
WebhookDeliveryAdapter and is permissive by default in Phase 2; tighten
when multi-user lands in Phase 5.

### Email reply spoofing

Email delivery uses lettre to reply to the originating Message-ID.
**Spoofing risk**: an attacker injects a forged email with a Message-ID
matching a legitimate task → next deliverable goes to attacker. Mitigation:

- IMAP intake verifies SPF / DKIM where available. Failed signatures
  are logged + intake rejected.
- Reply addresses are NEVER inferred from email content; only from
  Message-ID + envelope sender, which IMAP records.
- Per-message HMAC: each intake_id includes an HMAC of `(intake_id ||
  task_id || tenant_id)` used as a thread-secret on outbound replies.

### CLI security

The CLI is a thin HTTP client. Phase 2 single-user: no auth, talks to
local `http://127.0.0.1:3000`. Phase 5 multi-user: CLI gains OAuth/JWT
via `seasoned-hand auth login` (reserved).

### Sandbox renderer toolchain

Pandoc / python-pptx / openpyxl run inside the existing sandboxed
container (Phase 0 isolation — `seccomp=unconfined` is documented
DEBT #15 of Phase 0, no new exposure).

### Untrusted JSON payloads to renderers

LLM-produced JSON for pptx/xlsx is **schema-validated** before being
handed to python-pptx/openpyxl. Schema lives in
`seasoned-hand-core::deliverable::schemas`. Reject deeply-nested or
oversized payloads (config: max depth 10, max cells 100k).

### Provenance manifest content

Manifests may include PII (sender email addresses, task content).
Phase 2 single-user: same trust boundary as Phase 1 events table.
Phase 5: encrypt-at-rest for the `provenance_manifest` column.

### Admin rollback (Phase 1 1.13b)

Unchanged. Loopback + token guards apply. The Phase 1 inline unit
tests cover both guards.

---

## 10. Migration plan

### V006 → V009 — schema

Forward-only via `refinery`. Tenant columns are nullable everywhere in
Phase 2; Phase 5 flips to NOT NULL with backfill.

### Backfill strategy

- Phase 0/1 sessions: `task_id = NULL`. Rendered under synthetic
  "Phase 0/1 Archive" project. No SQL backfill.
- Phase 0/1 plans: `plans.task_id = NULL`. Same.

### Breaking-change audit

| Surface | Phase 1 behavior | Phase 2 behavior | Break? |
|---|---|---|---|
| WS `task_create { input }` | session per cmd | task + session per cmd; "Inbox" auto-project | NO (additive) |
| WS `task_pause { session_id }` | session-scoped soft pause | session OR task-scoped durable | NO |
| HTTP `/v1/sessions/...` | unchanged | unchanged + new `/v1/tasks/...` parallel surface | NO |
| `Initializer::run` signature | sync run | sync OR `run_with_confirmation` | NO |
| `WorkerDeps::from_router` signature | 8 args (Phase 1 hardening added cancel_tokens) | 8 args (no change in Phase 2) | NO |

Net: zero wire-level breaks. Existing test fixtures keep working.

---

## 11. Testing strategy

### Unit (Rust)

- `project::{ProjectStore, TaskStore}` — CRUD + pagination + state machine
- `briefing::confirm_round_trip` — confirm / edit / cancel / timeout paths
- `briefing::brief_validation` — rejects over-large briefs
- `deliverable::renderer::{pandoc, pptx, xlsx}` — round-trip render tests with shell-out to actual binaries (sandbox tests)
- `adapter::registry` — register / lookup / dispatch
- `intake::WebhookIntakeAdapter` — POST → IntakeEvent
- `intake::EmailIntakeAdapter` — IMAP fixture + parse
- `delivery::WebhookDeliveryAdapter` — wiremock'd POST callback
- `delivery::EmailDeliveryAdapter` — lettre stub transport
- `notify::*Adapter` — same shape as delivery
- `provenance::build_manifest` — golden-file tests over a known task
- `task::pause_durable + resume_via_replay`
- `cli::*` — argparse + HTTP mock
- DEBT #14 regression: `commit_phase_does_not_shell_inject`
- DEBT #15 regression: `worker_xreadgroup_drives_handle_request` (live-Redis `#[ignore]`)

### Integration (server)

- `tests/phase2_briefing.rs` — end-to-end task_create → Briefing → confirm
- `tests/phase2_briefing_edit.rs` — confirm → edit → re-emitted Briefing
- `tests/phase2_overnight_scaled.rs` — 24h durability scaled via `tokio::time::pause`
- `tests/phase2_webhook_intake_to_email_delivery.rs` — webhook brief → task runs → email reply with attachment
- `tests/phase2_renderer_pipeline.rs` — task_deliver call → docx + pptx + xlsx produced
- `tests/phase2_resume_from_replay.rs` — pause → kill container → resume
- `tests/phase2_provenance_complete.rs` — manifest contains every required field
- `tests/phase2_adapter_health.rs` — `/v1/adapters` reports correct state

### Frontend (Playwright — closes DEBT #9)

- `briefing_card.spec.ts` — render, confirm/edit/cancel
- `projects.spec.ts` — ProjectList nav, task summary cards
- `deliverables.spec.ts` — Deliverables tab render + citation chip
- `decisions.spec.ts` — Decisions pane filter
- `regression_smoke.spec.ts` — Chat / Verifier / 3-track Browser still render

### CLI

- `tests/cli_smoke.rs` — spawns server in test process, runs each
  CLI subcommand end-to-end against it

### E2E (live-LLM workflow_dispatch)

- `phase2-live-overnight`: real briefing flow + durable pause/resume +
  rendered .docx + email delivery + verifier pass. Gated on
  `ANTHROPIC_API_KEY` + `OPENAI_API_KEY` + `SEASONED_HAND_PHASE2_SMOKE=1`.
- `phase2-live-webhook-roundtrip`: webhook intake → email delivery,
  full round-trip on a real Bifrost.

### Acceptance gate (per ROADMAP §Phase 2 + v2.0 OS-shape)

"Do this overnight" works end-to-end:
1. User submits brief via webhook intake (one channel)
2. Briefing event fires; user confirms (or auto-confirms)
3. Task runs ≥ 8h wall (extrapolated from 5-min scaled test)
4. At least one durable pause + resume cycle
5. Deliverable rendered (e.g., `.docx`)
6. Deliverable delivered via the second channel (e.g., email reply)
7. Verifier verdict pass
8. Provenance manifest is complete and queryable
9. CLI can replay the entire flow with `seasoned-hand task new ... --detach && seasoned-hand task show $TASK_ID`

---

## 12. Open technical questions

1. **Briefing auto-confirm default**: 5-min timeout → auto-run is the
   "digital employee" UX. Should this be 0 (always wait for human)
   by default with users opting INTO auto-confirm? Decision affects
   the "Do this overnight" UX — user submits at 11 PM, sleeps, needs
   auto-confirm at 11:05 PM for overnight to work. Recommend
   auto-confirm-on by default.

2. **Email intake allow-list default**: deny-by-default (operator
   whitelists own email) vs allow-all-with-subject-prefix. Security vs
   UX trade-off. Recommend deny-by-default with one-command setup.

3. **Webhook intake bind address**: 127.0.0.1 (Phase 1 default) blocks
   inbound webhooks from external systems. Phase 2 operator may want
   0.0.0.0. Recommend: keep 127.0.0.1 default + document a `--bind`
   flag + a ngrok / Cloudflare Tunnel recommendation in README.

4. **Renderer toolchain failure recovery**: if the sandbox install of
   Pandoc/pptx fails at session create, the session is unusable. Hard
   fail vs degrade-to-md-only? Recommend hard fail with operator
   alert — silent degradation is worse than visible failure.

5. **Provenance manifest size budget**: a long task can accumulate
   thousands of decision events. Truncate the manifest? Or store in a
   separate file? Recommend store inline in DB up to 100 KB; spill to
   `/workspace/.provenance/<task_id>.json` above that.

6. **Skill/playbook reservation in V009**: empty tables in Phase 2.
   Phase 3 may discover the schema needs columns we haven't predicted.
   OK trade-off — the table itself is the contract, columns can be
   added via Phase 3 migrations.

7. **Code-as-deliverable** in Phase 2 vs Phase 4: Phase 2 ships
   "the sandbox git repo as a deliverable" (operator can `git clone`
   the workspace post-completion). Phase 4 adds GitHub PR creation
   via `GitHubDeliveryAdapter`. Confirm this split is acceptable —
   alternative is to defer all code-deliverable to Phase 4.

8. **Configuration source-of-truth location**: `~/.seasoned-hand/` for
   per-user (CLI binary lives here) vs `/etc/seasoned-hand/` for
   system-wide. Phase 2 ships per-user only (`~/.seasoned-hand/`);
   Phase 5 multi-user adds system-wide. Confirm.

---

## 13. Phase 2 story-count estimate

The PM session will carve this into stories. Rough count for the
5-week timebox (~3 h/day = ~75 h total budget):

| Story cluster | Stories | Hours |
|---|---|---|
| Project/Task/Deliverable persistence + V006/V007 | 2 | 6 |
| Briefing protocol (Initializer extension + confirm) | 2 | 6 |
| Renderer toolchain (Pandoc + pptx + xlsx) | 2 | 6 |
| Adapter framework + traits + registry | 1 | 4 |
| Intake adapters (webhook + email + CLI + chat-wrap) | 3 | 9 |
| Delivery adapters (webhook + email + chat + CLI) | 2 | 6 |
| NotifyWorker generalization + V008 + 3 adapters | 1 | 4 |
| Durable pause/resume + workspace TTL (DEBT #16) | 2 | 6 |
| Provenance manifest + V009 reservation | 1 | 3 |
| CLI binary + clap surface | 2 | 6 |
| DEBT #15 (Worker XREADGROUP) | 1 | 4 |
| DEBT #14 (shell-injection fix) | 1 | 2 |
| DEBT #9 (Playwright bootstrap + coverage) | 2 | 6 |
| NarratorHook classifier-slot wiring | 1 | 2 |
| Frontend: ProjectList + Briefing card + Deliverables tab | 3 | 9 |
| Phase 2 E2E + retrospective | 2 | 6 |
| **Total** | **~28** | **~85 h** |

5 weeks × 3 h/day × 5 days = 75 h — tight. Either extend slightly to
5.5 weeks OR trim the stretch goals (diagrams renderer, second-channel
intake test) and hold at 5 weeks.

PM session will rebalance.

---

Architecture is at `/specs/phase-2/architecture.md`. When approved,
start a fresh session with the PM persona to break this into stories.
