# GLOSSARY

> Project terminology. When you encounter a word that means something
> specific in this project, look here.
>
> Alphabetical within each section.

---

## Core concepts

**Agent loop**
The ReAct (Reason + Act) cycle: think → choose ONE tool → execute → observe
→ repeat. One iteration = one tool call. See ADR principles in
`/specs/00-philosophy/PRINCIPLES.md` § 2.

**Auxiliary slot**
One of 9 model routing slots for sub-tasks (vision, web_extract,
compression, etc.) Distinct from main slots. See ADR-003.

**Bifrost**
The LLM gateway (Go, single binary) sitting between our control plane and
LLM providers. See ADR-001.

**BMAD**
Methodology pattern (Business Analyst → Architect → PM → Dev personas)
applied at phase boundaries. See `/docs/methodology.md`.

**Brief**
The structured data shape (Phase 2 `crates/seasoned-hand-core/src/agent/init`
`Brief` struct) the Initializer authors from a one-line user task: goal,
phases, success criteria, deliverable specs, capabilities. Distinct from
"Briefing", which is the flow that produces it.

**Briefing**
The phase between user delegation and execution. The Initializer interprets
the one-line task, authors a `Brief`, and waits at the confirm gate. Agent
starts when the user confirms / edits / cancels (or 5-minute auto-confirm
fires). See `/specs/phase-2/architecture.md` §2.7.

**Capability detection**
Startup-time check that each configured model supports its slot's required
features (vision, tool calling, JSON mode, context size).

**Conservative learning**
Policy: extract playbooks only from verified, complex, repeat-pattern work.
See ADR-007.

**Curator**
Background process (Phase 4+) that maintains learning artifacts —
consolidating duplicates, archiving stale, flagging conflicts.

**Cumulative state**
The agent's working state at iteration N is the sum of all prior
iterations (events + file changes + plan state). Not just "the last
message." Validated against Manus's own self-description.

**Deliverable**
Concrete output from a task (file, report, data, deployment). What the
user actually receives. Not the same as "the agent's response message."

**Digital employee**
The product framing. Not assistant, not chatbot, not copilot. Employee:
hired, briefed, trusted to deliver.

**Discuss → Plan → Execute → Verify**
Daily story workflow. See GSD.

**Event stream**
Append-only SQLite table that records every Message, Action, Observation,
Plan, Knowledge, Datasource, Skill, and Misc event. Single source of truth
for runtime state. 8 types total (Knowledge / Datasource / Skill are
reserved for Phase 3+ Curator emission; the V002 CHECK constraint and the
Rust `EventType` enum carry them today).

**Glossary**
4th layer of learning artifacts: organizational terms, people, systems,
project-specific vocabulary. Distinct from this `GLOSSARY.md` file (which
is about *project* terms, not user-domain terms).

**Goal drift**
The phenomenon where an agent gets so focused on a sub-problem that it
forgets the original task. Plan-as-PCB (ADR-010) prevents this.

**GSD (Get Shit Done)**
Lightweight daily workflow. See `/docs/methodology.md`.

**Hermes Agent**
Open-source autonomous agent framework by Nous Research. One of our two
primary inspirations. Time-axis learning is its breakthrough.

**Initializer / Worker pattern**
Anthropic-popularized harness: an initializer agent sets up the task
context; a worker agent executes; control passes back when stuck.
Phase 1 deliverable.

**Main slot**
One of 3 primary model routing slots: `main`, `planner`, `verifier`.

**Manus**
Autonomous agent SaaS by Butterfly Effect. One of our two primary
inspirations. Execution depth is its breakthrough.

**Map tool**
Manus's parallel sub-agent fan-out mechanism for embarrassingly-parallel
tasks (e.g., "find contact info for 100 companies"). Deferred in Seasoned
Hand to Phase 4+. See ADR-009.

**Model router**
Component that resolves a slot name to a concrete `(provider, model,
base_url, api_key)` tuple and dispatches the call (through Bifrost).

**PCB (Process Control Block)**
OS concept for the structured data tracking one process's state. In
Seasoned Hand, the Plan plays this role: it holds goal + phases +
current_phase_id and is injected into every iteration's context. See
ADR-010.

**Plan**
Structured artifact (goal + phases + current_phase_id) that serves as
the agent's PCB. Created at task start, advanced explicitly via
`plan_advance`, restructured via `plan_update`. Always at top of
context (sticky). See ADR-010.

**Provenance manifest**
The full evidence trail attached to a Phase 2 deliverable: intake row +
brief confirm/edit lineage + plan phases + tool-call events +
verifier verdict + checkpoint refs. JSON in `deliverables.provenance_manifest`
up to 100 KB inline; spills to `/workspace/.provenance/<task_id>.json`
beyond that. Closes the "where did this come from?" question for every
deliverable. See `/specs/phase-2/architecture.md` §6 + Phase 2 DEBT #5.

**plan_advance, plan_update, plan_create**
Three plan-related tool actions:
- `plan_create(goal, phases)` — initial plan at task start
- `plan_advance()` — move current_phase_id to next phase
- `plan_update(phases)` — replace phases array (course correction)

**Playbook**
3rd layer of learning artifacts: an auto-extracted procedure from verified
successful work. Used to accelerate similar future tasks. Distinct from
SOPs.

**Project history**
2nd layer of learning artifacts: full event-stream traces of completed
projects, searchable via FTS5.

**Sandbox**
Isolated execution environment (Docker container) where the agent runs
shell, browser, and code operations. One per session.

**SDD (Spec-Driven Development)**
Methodology where specs are the source of truth and code is derived from
them. See `/docs/methodology.md`.

**Seasoned**
Adjective. State of being shaped by sustained use. The English word
captures what time + work produces (compare Korean 길든).

**Slot**
A routing target with a name (e.g., `vision`, `main`) bound to a concrete
model. 12 total (3 main + 9 auxiliary).

**Sticky context**
Content that is always at the top of every iteration's prompt, never
compressed away. The Plan is sticky. SOPs may be sticky. Tool catalog
is sticky. Recent events are not sticky (subject to compression).

**SOP (Standard Operating Procedure)**
1st layer of learning artifacts: explicit organizational rules, written
by humans, version-controlled, enforced by the agent. Distinct from
playbooks.

**Story**
Unit of implementation work. 1-3 hours, one PR, one commit. Has acceptance
criteria. Located in `/specs/phase-N/stories/`.

**Verifier**
Service (different model than main) that reviews completed work for
correctness. Biased toward FAIL to catch confabulation. Operates as
Layer 4 of the 4-layer verification framework.

**Verification (4-layer)**
The framework with 4 distinct layers:
- L1 Deterministic — tool output re-read (PostToolUse hook)
- L2 Cross-source — ≥2 sources for info, conflicts reported
- L3 Observation — Analyze Context step at iteration start
- L4 Meta-cognition — Verifier slot re-evaluation

See ARCHITECTURE.md § 6.

---

## Architecture pieces

**AIO Sandbox**
The specific Docker image we use for per-session sandboxes. Ships with
Ubuntu + Chromium + tmux + VNC + ttyd.

**Axum**
The Rust HTTP framework we use for the control plane. Built on Tokio.

**Bollard**
The Rust Docker SDK we use to manage AIO Sandbox containers.

**ChannelRegistration**
The Phase 2 builder shape that registers one channel (chat, webhook,
email, ntfy, cli) into the `ChannelRegistry` under any combination of
the three roles: intake, delivery, notify. Symmetric — every channel
that delivers work in can also deliver results back out. See
`crates/seasoned-hand-core/src/channel/mod.rs` +
`/specs/phase-2/architecture.md` §2.7.

**DeliveryRouter**
In-process Tokio coordinator that routes a completed deliverable to the
correct `DeliverySink` impl based on the task's `reply_target` (chat,
webhook, email, ntfy, cli). Append-only audit row to `delivery_events`.
Phase 2 story 2.5. See `crates/seasoned-hand-core/src/delivery/`.

**FTS5**
SQLite's full-text search extension. Used for playbook matching and
session history search.

**IntakeRouter**
In-process Tokio coordinator that drains a single fan-in
`mpsc::Sender<IntakeEvent>` from every long-lived `IntakeProvider`
(webhook, email, cli) plus the WS chat handler, persists the intake row,
creates the drafted Task, and spawns the Initializer via an
`InitializerSpawner`. Phase 2 story 2.5 + 2.8b. See
`crates/seasoned-hand-core/src/intake/router.rs`.

**NotifyWorker**
Redis XREADGROUP consumer that drains the `notify_request` stream and
dispatches to `NotifySink` impls per the `config/notify.toml` per-trigger
routing table. Worker (not router) because it lives across process
restarts via Redis PEL; pairs with `verifier::Worker` as the project's
two Redis-consumer-group surfaces. Phase 2 story 2.12.

**Rig**
The Rust LLM agent framework we use for the agent loop. Provides
abstractions over OpenAI-compatible providers.

**Tokio**
Rust's async runtime. Powers all concurrent work in the control plane.

**WAL (Write-Ahead Logging)**
SQLite mode we use for the event stream. Allows concurrent readers during
writes. See ADR-005.

**WorkspaceTtlCron**
Phase 2 sandbox cleanup cron that walks every workspace directory and
removes those whose session has exceeded a per-status TTL (default:
30 d completed, 7 d failed/cancelled, 1 d drafted/briefed; running +
paused never GC). Closes Phase 0 DEBT #16. Admin manual trigger at
`POST /v1/admin/sandbox/cleanup`. See
`crates/seasoned-hand-core/src/task/ttl.rs`.

---

## Process & methodology

**ADR (Architecture Decision Record)**
A document capturing one architectural decision: context, decision,
consequences, alternatives. Lives in
`/specs/01-architecture/decisions/`.

**BASELINE**
The single-entry-point document (`/BASELINE.md`). Read first when starting
a new AI session.

**Living spec**
A spec file that updates alongside code changes. The opposite of "set
once and forget."

**Spec compliance**
Property: code matches its spec. Verified by `./scripts/spec-check.sh`.
Drift is a bug.

**Story status**
States a story can be in: `ready` → `in-progress` → `done` (or `blocked`).
Tracked in the story file itself.

---

## Korean ↔ English

A few terms where bilingual clarity helps:

| Korean | English (in this project) | Notes |
|---|---|---|
| 길든 | seasoned | Tagline equivalent. *Broken in by use.* |
| 스펙 | spec, specification | Use English in code, Korean ok in prose |
| 자율 에이전트 | autonomous agent | English form preferred in product naming |
| 작업 | task | Use "task" in code; "작업" ok in user-facing docs |
| 결과물 | deliverable | "Output" is too generic |
| 검증 | verification (the act); verifier (the component) | |
| 위임 | delegation | The act of giving the agent a task |

---

## Acronyms

| Acronym | Meaning |
|---|---|
| ADR | Architecture Decision Record |
| AIO | All-In-One (as in AIO Sandbox) |
| Apache-2.0 | Apache License 2.0 — the project's license (permissive + explicit patent grant) |
| BMAD | Business analyst / Architect / Manager / Dev (methodology) |
| FTS | Full-Text Search (SQLite extension FTS5) |
| GSD | Get Shit Done (daily workflow) |
| PCB | Process Control Block (OS concept; our Plan plays this role) |
| PR | Pull Request |
| SDD | Spec-Driven Development |
| SDLC | Software Development Lifecycle |
| SOP | Standard Operating Procedure |
| SoT | Source of Truth |
| WAL | Write-Ahead Log |
| WIP | Work in Progress |

---

## How to add a term

When you find yourself defining a project-specific word in conversation
or docs more than once, add it here. The test: "Would a new contributor
need this defined?"

Don't add: general programming terms (everyone knows), one-off codenames
that won't outlive a week, words with the same meaning as common English.
