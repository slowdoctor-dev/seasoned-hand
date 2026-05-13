# Phase 2 — Employee Interface (OS-shape)

> **Status**: v1.0 (BMAD PM persona output, 2026-05-13)
> **Duration**: 5 weeks
> **Goal**: A digital employee team that lives behind an OS-shape control
> plane. Work comes in over any channel (chat / webhook / email / CLI),
> gets executed by the fixed Phase-1 role team (Initializer / Worker /
> Verifier / Narrator / Plan / Checkpoint / Notify), and leaves as the
> artifact form the work demands (.docx / .pptx / .xlsx / code / URL),
> routed back to the channel that asked.
> **Bridges**: `/specs/01-architecture/ARCHITECTURE.md` (immutable) +
> `/specs/phase-2/architecture.md` v2.1 (Architect output, `9b8d92a`) →
> `/specs/phase-2/stories/story-2.X.md` (this PM output).

---

## 1. Goals

By end of Phase 2:

1. The system accepts work via at least **four channels** (chat, webhook,
   email, CLI), each implemented as one `*Channel` struct with 1-3 role
   trait impls (IntakeProvider / DeliverySink / NotifySink).
2. Every task has a structured **Brief** (goal + phases +
   success_criteria + typed `DeliverableSpec[]`), confirmed by the user
   (or auto-confirmed via 5-min timeout) before the Worker loop starts.
3. Tasks produce **real-employee deliverables**: markdown, JSON, CSV
   (raw), plus `.docx` / `.pdf` / `.html` (Pandoc), `.pptx` (python-pptx),
   `.xlsx` (openpyxl). Each deliverable carries a complete **provenance
   manifest** tracing back to brief / decisions / verifier verdicts /
   checkpoints / metrics.
4. Tasks survive **24h+ pause-resume cycles** even when the sandbox
   container has been garbage-collected (event-stream replay rebuild
   path).
5. A `seasoned-hand` **CLI** binary mirrors every UI action — projects /
   tasks / briefings / channels / provenance / inbox.
6. Five Phase-1 DEBT carry-overs close: **#15** (Worker XREADGROUP),
   **#14** (SandboxGitShell shell-injection), **#9** (Playwright
   bootstrap), **#3** (verifier rollback default decision — data-driven
   at Phase 2 closeout), **#0** (NarratorHook classifier-slot wiring).
   Plus **Phase 0 DEBT #16** (workspace TTL cron) pays down.
7. Frontend gains three new surfaces: **ProjectList** (left panel),
   **Briefing card** (Chat-panel inline confirmation UX), **Deliverables
   + Decisions tabs** (AgentComputer right panel).
8. **Multi-tenant-ready schema**: every new row carries a nullable
   `tenant_id`. Phase 5 flips to NOT NULL.
9. **Skill / playbook reservation tables** (V009) are empty but exist
   so Phase 3 (learning) adds rows, not tables.

**Not in scope** (per ROADMAP + architecture §0): briefing→learning
pipeline (Phase 3), Curator self-improvement (Phase 4), Slack / Notion /
Drive channels (Phase 4+, multi-user prereq), multi-user auth (Phase 5),
voice / calendar channels (Phase 6+), public "OS for work" category
claim (Phase 4+ retrospective).

---

## 2. Non-functional requirements

Phase 0 + Phase 1 budgets carry forward. New budgets (from architecture
§7):

| Requirement | Target |
|---|---|
| Briefing event emit (Initializer parse → emit) | < 500 ms p95 |
| `GET /v1/projects/:id/tasks` (50 rows) | < 100 ms p95 |
| Channel intake → IntakeEvent persisted (HTTP) | < 200 ms p95 |
| Channel intake (IMAP poll cycle, no new mail) | < 1 s p50 |
| Channel intake (IMAP message → IntakeEvent) | < 3 s p95 |
| Channel delivery (HTTP success) | < 500 ms p95 |
| Channel delivery (SMTP send) | < 5 s p95 |
| Renderer markdown → docx (Pandoc) | < 2 s p95 |
| Renderer JSON → pptx (python-pptx) | < 5 s p95 |
| Renderer JSON → xlsx (openpyxl) | < 2 s p95 |
| Sandbox durable freeze | < 3 s |
| Sandbox resume via existing container | < 10 s |
| Sandbox resume via event-stream replay | < 60 s |
| Verifier XREADGROUP poll | BLOCK 5000 COUNT 16 |
| 24h-continuous task wall budget | 24 h + 30 min slack |
| Phase 1 verifier / plan / cost budgets | unchanged |

---

## 3. Functional requirements

### 3.1 Project / Task / Subtask hierarchy

- `Project { id, title, description?, status, tenant_id?, created_at,
  updated_at }`. Status: `active | archived`.
- `Task { id, project_id, title, brief?, status, expected_due_at?,
  completed_at?, failure_reason?, parent_task_id?, schedule?,
  skill_attached_event_id?, tenant_id?, created_at, updated_at }`. Status:
  `drafted → briefed → confirmed → running ⇄ paused → completed | failed
  | cancelled`.
- `Session` (existing) gains `task_id` FK. One Task can spawn many
  Sessions (pause/resume cycles).
- `Subtask` = Plan phase (existing `plans.phases[*]`). No separate table.

### 3.2 Briefing protocol

`Initializer::run_with_confirmation` extends story-1.4's Initializer:

1. Parse `input` into a typed `Brief { goal, phases[],
   success_criteria[], expected_deliverables: DeliverableSpec[] }`
2. Emit Misc `briefing_pending` + ServerEvent `Briefing{ briefing_call_id, ... }`
3. Wait for WS `user_response { in_reply_to_call_id: briefing_call_id,
   action: "confirm" | "edit" | "cancel", edits? }` OR auto-confirm
   after 5 min (configurable via `briefing_auto_confirm_ms`; setting
   `briefing_require_confirm: true` disables auto)
4. On confirm: seed Plan from `brief.phases`, status → `running`
5. On edit: re-parse with edits, re-emit Briefing
6. On cancel: status → `cancelled`, emit `task_state`

### 3.3 Deliverable standards (multi-format)

LLM tool `task_deliver(content, target_filename, citations)` —
Worker-mode only. Server-side renderer dispatch by filename extension:

| Format | Renderer | Source |
|---|---|---|
| `.md`, `.txt`, `.json`, `.csv` | raw write | LLM-produced content |
| `.docx`, `.pdf`, `.html`, `.odt` | Pandoc CLI | LLM-produced markdown |
| `.pptx` | python-pptx | LLM-produced JSON `{slides: [...]}` |
| `.xlsx` | openpyxl | LLM-produced JSON `{sheets: [...]}` |
| code (git repo + optional URL) | the sandbox git tree itself | Worker writes files via existing tools |

Sandbox image gains Pandoc + python-pptx + openpyxl via startup-time
install (apt + pip). Phase 4 may migrate to a pre-baked image.

### 3.4 Channel framework (OS-shape keystone)

- `IntakeProvider`, `DeliverySink`, `NotifySink` traits.
- One concrete `*Channel` struct per integration implements 1, 2, or 3
  role traits.
- Registration: `state.channels().register(ChannelRegistration::new("name").with_intake(arc).with_delivery(arc).with_notify(arc))`.
- Phase 2 ships 5 channels: `ChatChannel`, `WebhookChannel`,
  `EmailChannel`, `NtfyChannel`, `CliChannel`.

### 3.5 Provenance manifest (mandatory)

Every Deliverable carries a complete JSON manifest:

```typescript
type ProvenanceManifest = {
  schema_version: 1,
  task_id, project_id, tenant_id?,
  intake: { channel, intake_id, received_at, metadata },
  brief: { brief_event_id, confirmed, confirmed_at?, edits_applied },
  sessions: Array<{ id, started_at, ended_at?, end_reason? }>,
  decisions: number[],
  verifier_verdicts: string[],
  checkpoints: Array<{ checkpoint_id, git_sha, rolled_back }>,
  metrics: { tool_calls, cost_cents, wall_seconds, sessions_count,
             pause_resume_cycles, verifier_runs },
  delivered_to: Array<{ channel, delivery_id, delivered_at, ok, external_id? }>,
  source_content_sha256?, rendered_content_sha256, citations: number[],
}
```

### 3.6 Long-running task durability

- Soft pause (Phase 1 1.17) keeps the docker container paused.
- **Durable freeze** (new): pause + persist event-stream cursor + sandbox metadata.
- **Resume rebuild**: if sandbox container is gone, replay events into a fresh container; reconstruct Plan + feature-list + progress + cost baseline.
- WS `task_pause` gains `durable: bool` (defaults `true`).
- Workspace TTL via Phase 0 DEBT #16: active=never GC; paused=7d; completed=30d; failed/cancelled=7d.

### 3.7 Notification system (generalized from v1.0)

`NotifyWorker` consumes Redis Stream `notify_request`, dispatches to
registered `NotifySink` channels. Triggers: `task_finished`,
`task_failed`, `briefing_pending`, `verifier_fail`. Best-effort
delivery (1 retry for webhook 5xx; no retry for ntfy/email).

### 3.8 CLI (`seasoned-hand` binary)

Thin HTTP client + inline `CliChannel` for `task new`. Subcommands:
`init`, `server`, `project list/create/archive`, `task new/list/show/
pause/resume/cancel/brief/deliverable/provenance`, `inbox`, `brief
confirm/edit/cancel`, `channel list/test/logs`.

### 3.9 Frontend

- **ProjectList** panel (left side of HomeShell, above existing TaskList)
- **Briefing card** renderer in Chat panel (intercepts `Briefing` events with confirm/edit/cancel buttons)
- **Deliverables tab** + **Decisions tab** in AgentComputer (right panel)
- **Playwright bootstrap** + smoke coverage for all three new surfaces (closes Phase 1 DEBT #9)

### 3.10 DEBT close-outs

| Phase | Item | Closed in Phase 2 story |
|---|---|---|
| Phase 0 | #16 (workspace TTL + cleanup cron) | 2.17 |
| Phase 1 | #3 (verifier rollback default flip) | 2.27 (data-driven) |
| Phase 1 | #9 (frontend automated tests) | 2.24 |
| Phase 1 | #14 (SandboxGitShell shell injection) | 2.19 |
| Phase 1 | #15 (Worker XREADGROUP loop) | 2.18 |
| Phase 1 | story-1.15 classifier-slot wiring exec-note | 2.20 |

---

## 4. Story breakdown

Each story is 1-3 hours, one PR, one commit, story-relative acceptance
criteria, executable by any AGENTS.md-compatible agent from a fresh
session. Detailed specs at `/specs/phase-2/stories/story-2.X.md`.

| ID | Story title | Est | Deps | Closes | Status |
|---|---|---|---|---|---|
| 2.1  | Phase 2 scaffolds — requirements.md + DEBT.md | 0.5h | — | — | done |
| 2.2  | V006 migration + ProjectStore + TaskStore | 2.5h | 2.1 | — | ready |
| 2.3  | V007 + V008 + V009 migrations + remaining stores | 3h | 2.2 | — | ready |
| 2.4  | Channel framework — traits + ChannelRegistration + ChannelRegistry | 2h | 2.1 | — | ready |
| 2.5  | IntakeRouter + DeliveryRouter | 2h | 2.4, 2.3 | — | ready |
| 2.6  | Sandbox-side renderer toolchain (Pandoc + python-pptx + openpyxl) | 3h | 2.1 | — | ready |
| 2.7  | Brief shape + DeliverableSpec typed schema | 1.5h | 2.2 | — | ready |
| 2.8  | Initializer::run_with_confirmation (Briefing + confirm gate) | 2h | 2.7 | — | ready |
| 2.9  | ChatChannel (wraps existing WS as IntakeProvider + DeliverySink) | 1h | 2.4, 2.5 | — | ready |
| 2.10 | WebhookChannel (intake + delivery + notify, three role impls) | 2.5h | 2.4, 2.5 | — | ready |
| 2.11 | EmailChannel (IMAP intake + SMTP delivery + notify) | 3h | 2.4, 2.5 | — | ready |
| 2.12 | NtfyChannel (notify-only) + NotifyWorker | 2h | 2.4 | — | ready |
| 2.13 | CliChannel (process-local intake + stdout delivery) | 1.5h | 2.4 | — | ready |
| 2.14 | task_deliver LLM tool + RendererDispatcher | 2.5h | 2.6, 2.3 | — | ready |
| 2.15 | Provenance manifest builder + GET /v1/tasks/:id/provenance route | 2h | 2.3, 2.5 | — | ready |
| 2.16 | Durable pause/resume + event-stream replay rebuild | 3h | 2.2 | — | ready |
| 2.17 | Workspace TTL + cleanup cron | 2h | 2.2 | Phase 0 DEBT #16 | ready |
| 2.18 | Verifier Worker real XREADGROUP loop | 3h | — | DEBT #15 | ready |
| 2.19 | SandboxGitShell shell-injection fix | 1h | — | DEBT #14 | ready |
| 2.20 | NarratorHook classifier-slot wiring through AppState::new | 1.5h | — | story-1.15 | ready |
| 2.21 | seasoned-hand CLI binary + subcommand surface | 3h | 2.13 + routes | — | ready |
| 2.22 | Frontend: ProjectList + Deliverables + Decisions tabs | 3h | 2.2, 2.3 | — | ready |
| 2.23 | Frontend: Briefing card + confirm/edit/cancel UI | 2.5h | 2.8 | — | ready |
| 2.24 | Frontend: Playwright bootstrap + smoke coverage | 2.5h | 2.22, 2.23 | DEBT #9 | ready |
| 2.25 | Phase 2 E2E (deterministic 50-step + briefing + email roundtrip) | 3h | 2.2-2.20 | — | ready |
| 2.26 | Phase 2 live-LLM workflow_dispatch jobs | 2h | 2.25 | — | ready |
| 2.27 | Phase 2 closeout (retrospective + DEBT audit + status flips) | 1.5h | 2.26 | — | ready |

**Total**: 27 stories, ~59 h budgeted across 5 weeks at ~3 h/day
(75 h available; 16 h slack).

**Parallelisable seams**:
- After 2.1: `{2.2, 2.6, 2.18, 2.19, 2.20}` — 5 parallel
- After 2.2: `{2.3, 2.7, 2.16, 2.17, 2.22}` — 5 parallel
- After 2.4 + 2.5: `{2.9, 2.10, 2.11, 2.12, 2.13}` — 5 channels in parallel (one struct each)
- After 2.8: + 2.23 frontend
- After 2.22 + 2.23: 2.24 Playwright
- After 2.20 backend stories: serialized close-out 2.25 → 2.26 → 2.27

---

## 5. Acceptance criteria

Phase 2 is done when:

```
✓ Briefing protocol works: task_create → Briefing → user confirms → Worker runs
✓ Briefing auto-confirms after 5 min (default; configurable)
✓ All four Phase-2 channels round-trip (chat, webhook, email, CLI)
✓ Channel registered ONCE per integration via ChannelRegistration builder
✓ Deliverables produced in markdown, json, docx, pdf, html, pptx, xlsx, csv
✓ Code-as-deliverable: sandbox git tree is the artifact (basic; PR creation = Phase 4)
✓ Every Deliverable carries a complete provenance manifest
✓ 24h+ task survives container GC via event-stream replay rebuild
✓ Workspace TTL cron honors task state (Phase 0 DEBT #16)
✓ `seasoned-hand` CLI mirrors every UI action
✓ Frontend has Playwright coverage for new surfaces (Phase 1 DEBT #9)
✓ Phase 1 DEBT items #14, #15, story-1.15 wiring all close
✓ Phase 1 DEBT #3 (rollback default) gets a data-driven decision
✓ Multi-tenant-ready schema (all new tables have nullable tenant_id)
✓ Skill / playbook tables exist (empty — Phase 3 ready)
✓ `just verify` passes all gates
✓ phase-2/DEBT.md keeps an accurate ledger of new shortcuts
✓ "Do this overnight" end-to-end works on at least two channels
```

---

## 6. Deferred (NOT in Phase 2)

- ❌ Briefing → playbook auto-extraction (Phase 3 — learning)
- ❌ Curator, self-improvement (Phase 4)
- ❌ Slack / Notion / Google Drive / GitHub channels (Phase 4 — auth needs Phase 5 multi-user first)
- ❌ Voice / calendar channels (Phase 6+ audio + Phase 5 scheduler prereq)
- ❌ Multi-user, organization, real auth (Phase 5)
- ❌ Multi-employee orchestration (Phase 6+ — multiple Seasoned Hand instances coordinated)
- ❌ Public "OS for work" category claim in README (earn-before-claim; revisit Phase 4 retro)
- ❌ Polished docs, one-command deploy (Phase 6)
- ❌ HTML / PDF deliverable formats beyond Pandoc baseline (Phase 4+)
- ❌ Recurring schedules / pipelines (Phase 5)
- ❌ Encryption-at-rest for provenance manifests (Phase 5)
