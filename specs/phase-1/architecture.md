# Phase 1 — Architecture

> **Status**: v1.0 (BMAD Architect output, 2026-05-12)
> **Phase**: 1 (Manus 5-Layer — Deep Execution)
> **Bridges**: `/specs/01-architecture/ARCHITECTURE.md` (immutable overall) +
> `/specs/phase-0/architecture.md` (closed) → `/specs/phase-1/stories/`
> (to be broken out by BMAD PM persona after this doc is approved).
> **Scope**: 50+ tool-call sessions stable. Verifier slot wired. Plan Manager
> made real. Initializer/Worker. Circuit breaker hardened. Git checkpoint +
> rollback. 3-track browser. Live narration. **No** learning, **no**
> employee/briefing UI, **no** multi-user.

This document specifies *how* Phase 1 extends the Phase 0 skeleton into a
deep-execution runtime. It does not re-derive overall architecture; it lists
**only what changes**. When a spec here conflicts with ARCHITECTURE.md,
ARCHITECTURE.md wins — unless the conflict is listed in §0 below, in which
case the immutable spec already drifted from reality during Phase 0 and is
scheduled for a doc-only ADR fix.

---

## 0. Conflicts to reconcile with `/specs/01-architecture/ARCHITECTURE.md`

Phase 0 shipped two divergences from the immutable spec that this document
inherits as-is. Tracked in `specs/phase-1/DEBT.md`; resolved by a separate
doc-only ADR commit, never silently in this file.

| # | ARCHITECTURE.md says | Phase 0 shipped | Severity | Pay-down |
|---|---|---|---|---|
| 1 | §2.4 / §7: 32 tools (29 Manus + 3 learning) | 33 tools (`plan_advance` exposed as a dispatchable tool, per ADR-010) | Low (text-only) | Doc-only ADR-011 or `ARCHITECTURE.md` v1.1 |
| 2 | §1.1 / BASELINE §4: Next.js 15 | Next.js 16 / React 19.2 / Tailwind 4.3 | Low (text-only) | Same doc-only fix |

Phase 1 also introduces **new tools** (see §2 and §4.3) that further widen
this gap from 33 to a target near 37. The same ADR is the right place to
land the reconciliation.

---

## 1. Summary diagram

Phase 1 surface. **Bold** = new or substantively changed in Phase 1.
`[brackets]` = built in Phase 0, referenced for context.

```
┌──────────────────────────────────────────────────────────────────────────┐
│  Frontend (Next.js 16, App Router, Tailwind v4, React 19)                 │
│  [TaskList | Chat | AgentComputer] panels from Phase 0                    │
│  + NEW: Live-narration lane in Chat (`ui:"narrate"` Messages)             │
│  + NEW: 3-track BrowserTab inside AgentComputer                           │
│    (Track A: noVNC live │ Track B: DOM text │ Track C: screenshot strip)  │
│  + NEW: Verifier verdict pane (Misc{kind:"verifier_*"} events)            │
└──────────────────────────┬───────────────────────────────────────────────┘
                           │ WebSocket + HTTP
                           ↓
┌──────────────────────────────────────────────────────────────────────────┐
│  Control Plane (Rust workspace: seasoned-hand-core + -server)              │
│                                                                            │
│  ┌──────────────────────────────────────────────────────────────────┐    │
│  │ API layer (axum 0.7) — Phase 0 surface + new endpoints below      │    │
│  │ + GET  /v1/sessions/:id/verifications                              │    │
│  │ + GET  /v1/sessions/:id/checkpoints                                │    │
│  │ + POST /v1/sessions/:id/checkpoints/:id/rollback (admin only)      │    │
│  └──────────────────────────────────────────────────────────────────┘    │
│  ┌──────────────────────────────────────────────────────────────────┐    │
│  │ Agent runtime (ReAct loop, Phase 0)                                │    │
│  │ + NEW: Initializer phase (pre-loop) ─ feature-list.json,           │    │
│  │        progress.txt, plan_create call                              │    │
│  │ + NEW: Worker = single agent loop instance per task in Phase 1     │    │
│  │ + NEW: 6 context principles fully enforced (see §2.3)              │    │
│  │ + NEW: Circuit breaker (4 conditions, see §2.5)                    │    │
│  │ + Stuck detection (Phase 0) extended with diversity injection       │    │
│  └──────────────────────────────────────────────────────────────────┘    │
│  ┌──────────────────────────────────────────────────────────────────┐    │
│  │ Plan Manager (ADR-010) — Phase 0 stubs → REAL in Phase 1          │    │
│  │ + plan_create / plan_advance / plan_update all wired to `plans`    │    │
│  │ + Sticky context renders structured Plan, not raw event payload    │    │
│  │ + Emits Plan{op,…} event on every mutation (Phase 0 schema reused) │    │
│  └──────────────────────────────────────────────────────────────────┘    │
│  ┌──────────────────────────────────────────────────────────────────┐    │
│  │ NEW: Verifier Worker (Tokio task, separate from agent runtime)    │    │
│  │  - Subscribes to internal `verify_request` channel (Redis pub/sub) │    │
│  │  - 3 trigger sources: TaskComplete │ Invalidation │ CircuitBreaker │    │
│  │  - Runs `verifier` slot with FAIL-biased system prompt              │    │
│  │  - Constructs fresh context (NEVER reuses agent context)           │    │
│  │  - Emits Misc{kind:"verifier_verdict", verdict, reason, ...}        │    │
│  │  - On fail → enqueues `plan_update_required` for the agent loop    │    │
│  │  - Persists every run to new `verifications` table                  │    │
│  └──────────────────────────────────────────────────────────────────┘    │
│  ┌──────────────────────────────────────────────────────────────────┐    │
│  │ NEW: Checkpoint Manager                                            │    │
│  │  - PostPhaseAdvance hook → `git commit` in sandbox workspace       │    │
│  │  - Rollback on verifier failure → `git revert` last checkpoint     │    │
│  │  - Persists SHA + phase mapping in new `checkpoints` table         │    │
│  │  - LLM does NOT see git — invisible plumbing                       │    │
│  └──────────────────────────────────────────────────────────────────┘    │
│  ┌──────────────────────────────────────────────────────────────────┐    │
│  │ NEW: Narrator Hook (PreToolUse)                                    │    │
│  │  - Templated narration for cheap tools (file_*, plan_*, idle)      │    │
│  │  - LLM narration via `classifier` slot for complex tools           │    │
│  │  - Emits Message{role:"assistant", ui:"narrate", content}          │    │
│  └──────────────────────────────────────────────────────────────────┘    │
│  ┌──────────────────────────────────────────────────────────────────┐    │
│  │ NEW: Invalidation Detector (PostToolUse hook)                      │    │
│  │  - Tracks file path → SHA-256 of last read/write content           │    │
│  │  - When a later observation contradicts a prior assertion,         │    │
│  │    emits `Verifier.Invalidation` request                           │    │
│  │  - Allow-list of expected-rewrite tools (file_write,               │    │
│  │    file_str_replace) so legitimate edits don't trigger             │    │
│  └──────────────────────────────────────────────────────────────────┘    │
│  ┌──────────────────────────────────────────────────────────────────┐    │
│  │ Tool Dispatcher (Phase 0) — same shape, expanded catalog           │    │
│  │ + 3 new LLM-visible tools: `checkpoint_label`, `feature_mark_done`,│    │
│  │   `progress_update` (see §4.3)                                     │    │
│  │ + 1 internal-only hook tool: `checkpoint_rollback` (no LLM exposure)│    │
│  │ + Tool-masking: catalog stays stable; masked tools surfaced via    │    │
│  │   `description` field, never removed from catalog (PRINCIPLE #2)   │    │
│  └──────────────────────────────────────────────────────────────────┘    │
│  ┌──────────────────────────────────────────────────────────────────┐    │
│  │ Event Stream (Phase 0) — schema unchanged; new Misc.kind values    │    │
│  │   verifier_verdict │ verifier_request │ checkpoint_create │        │
│  │   checkpoint_rollback │ feature_done │ progress_recite             │    │
│  └──────────────────────────────────────────────────────────────────┘    │
│  ┌──────────────────────────────────────────────────────────────────┐    │
│  │ 12-slot Model Router (Phase 0) — same; `verifier` slot now used   │    │
│  │ + Startup hard-fail if `verifier` slot resolves to the same model  │    │
│  │   id as `main` (FAIL-biased = different model, see ARCH §6 L4)     │    │
│  └──────────────────────────────────────────────────────────────────┘    │
│  ┌──────────────────────────────────────────────────────────────────┐    │
│  │ Sandbox Client (Phase 0) — extensions                              │    │
│  │ + Workspace bootstrap writes feature-list.json + progress.txt      │    │
│  │ + `git init` + initial commit at sandbox create                    │    │
│  │ + Screenshot capture endpoint wired for 3-track Track C            │    │
│  └──────────────────────────────────────────────────────────────────┘    │
│                                                                            │
│  Persistence: SQLite WAL (Phase 0) + Redis pub/sub (Phase 0)              │
│  + Redis Streams used for `verify_request` channel (NEW use; same dep)    │
└──────────────────────────┬───────────────────────────────────────────────┘
                           │ OpenAI-compatible HTTP (unchanged)
                           ↓
                Bifrost (Phase 0) — http://localhost:4000/v1
```

**Explicitly out of Phase 1** (deferred to later phases per ROADMAP):

- Employee-style briefing protocol, deliverable templates (Phase 2)
- Pause/resume across days, async notifications (Phase 2)
- SOPs / playbooks / glossary / FTS5 (Phase 3)
- Curator background worker, self-improvement (Phase 4)
- Multi-user, organization, per-user cost, audit log (Phase 5)
- Polished docs, one-command deploy (Phase 6)

---

## 2. New components introduced

| # | Component | Crate / module | Tech | Integrates with |
|---|---|---|---|---|
| 1 | Initializer | `seasoned-hand-core::agent::init` | tokio task (pre-loop) | Plan Manager, Sandbox client, Event stream |
| 2 | Plan Manager (real) | `seasoned-hand-core::plan` | rusqlite + serde_json | Agent runtime, Event stream, Redis pub/sub |
| 3 | Verifier Worker | `seasoned-hand-core::verifier` | tokio task + Redis Streams + LLM client | Module worker pool, Plan Manager |
| 4 | Verifier Trigger Sources | `seasoned-hand-core::verifier::triggers` | 3 sub-modules | Hook chain, Agent runtime, Circuit breaker |
| 5 | Invalidation Detector | `seasoned-hand-core::verifier::invalidation` | PostToolUse hook + content-hash map | Hook chain, Event stream |
| 6 | Checkpoint Manager | `seasoned-hand-core::checkpoint` | reqwest → sandbox `/v1/shell` git | Hook chain (PostPhaseAdvance), Sandbox client |
| 7 | Narrator Hook | `seasoned-hand-core::agent::narrate` | PreToolUse hook + classifier slot | LLM client, Event stream, Frontend |
| 8 | Circuit Breaker (extended) | `seasoned-hand-core::agent::breaker` | 4-condition state machine | Stuck tracker, Cost client, Error counter, Verifier |
| 9 | Diversity Injector | `seasoned-hand-core::agent::diversity` | small Rust function over recent assistant messages | Stuck tracker, Sticky context builder |
| 10 | Context Recitation | `seasoned-hand-core::agent::recite` | timer + file_read | Agent runtime, Sandbox client |
| 11 | 3-track Browser Capture | `seasoned-hand-core::browser::tracks` | PostBrowserAction hook | Sandbox client, Event stream, Frontend |
| 12 | Tool-mask Layer | `seasoned-hand-core::dispatch::mask` | trait extension over `Tool` | Tool dispatcher |

**Reused from Phase 0 without change**: API layer, Tool dispatcher core,
Event stream writer + subscribe, LLM client, 12-slot router (config only —
`verifier` is now selected), Sandbox client lifecycle, Session store,
Cost client (`/cost` polling). The stuck detector grows a diversity-injection
output but its interface is unchanged.

### 2.1 Initializer + Worker pattern

The Initializer runs **once per task**, before the first agent loop iteration:

1. Reads the user's one-line briefing from the `task_create` WS command.
2. Calls **planner slot** to produce a structured plan (goal + 3-8 phases,
   each with `capabilities[]`). Validates JSON via serde, falls back to a
   single-phase plan if malformed (Phase 0 §8 already specifies the
   fallback; reused).
3. Calls `plan_create(goal, phases)` — persists to `plans`, emits
   `Plan{op:"create"}`.
4. Writes two workspace files at the sandbox root:
   - `feature-list.json` — `[{ id, title, status:"todo"|"doing"|"done", depends_on?:[id] }, …]` derived 1:1 from the plan phases.
   - `progress.txt` — append-only plain-text log; Initializer writes the goal
     and the initial feature list as the first lines.
5. Hands off to the **Worker** = a single agent loop instance running in a
   Tokio task. Phase 1 ships **one Worker per task**. ADR-009 (map tool
   deferred) means we are not building parallel Workers in this phase.

The Initializer **does not** count toward the iteration cap; its plan-create
call uses the `planner` slot independently of `main`. The Worker's first
sticky context already includes the structured plan rendered by the Plan
Manager.

### 2.2 Plan Manager — Phase 0 stubs paid down

Phase 0 DEBT #25: `plan_create`, `plan_advance`, `plan_update` return
`not_implemented`. Phase 1 wires them to the existing `plans` table per
ADR-010 §"Storage" and §"Actions". No schema change.

Behavioral spec:

| Tool | Caller | Effect |
|---|---|---|
| `plan_create(goal, phases)` | Initializer only (NOT exposed to LLM mid-loop; uses `tool_choice` mask) | Insert row, set `current_phase_id=phases[0].id`, emit `Plan{op:"create"}` |
| `plan_advance()` | LLM **or** runtime auto-advance when all features in the active phase are `done` in `feature-list.json` | Atomic UPDATE of `current_phase_id` to the next pending phase, emit `Plan{op:"advance"}`, fire PostPhaseAdvance hook |
| `plan_update(phases)` | LLM **or** Verifier (when verdict = fail with `suggested_phases`) | Replace `phases` JSON, recompute `current_phase_id` to the lowest pending id, emit `Plan{op:"update"}` |

Sticky context render (replaces Phase 0 raw-event rendering):

```
== PLAN ==
Goal: <goal>
Phase 1 [done]: <title>
Phase 2 [active]: <title>   ← current
Phase 3 [pending]: <title>
== END PLAN ==
```

Hard cap on render: 1000 tokens (Phase 0 §7 budget unchanged). If exceeded,
truncate phase titles individually; never drop the structure.

### 2.3 Six context-engineering principles — enforcement table

PRINCIPLES.md §16+§17 plus the 6 from `/specs/01-architecture/ARCHITECTURE.md`
§5. Phase 0 already enforced #1, #2, #5 implicitly. Phase 1 makes the
remaining four explicit, tested, and gate-enforced.

| # | Principle | Phase 0 status | Phase 1 enforcement |
|---|---|---|---|
| 1 | KV-cache friendly prefix | append-only events; sticky plan added | unchanged; add gate in `spec-check.sh` asserting sticky context never re-orders earlier turns |
| 2 | No mid-iteration tool catalog changes | dispatcher exposes all 33 tools every iter | NEW: tool-masking layer — mask sets `available:false` in the schema description, the tool stays in the catalog so KV-cache is preserved |
| 3 | Filesystem as memory | 16 KB → file_ref in event payload | NEW: `feature-list.json` + `progress.txt` + Verifier always reads `feature-list.json` instead of replaying events |
| 4 | Todo recitation | not implemented | NEW: Context Recitation — every 10 iterations, runtime injects a synthetic `progress_recite` Misc event whose content is the current `progress.txt`; the agent's next iteration must consume it before tool selection |
| 5 | Errors preserved | failed observations stay in stream | unchanged; add Verifier behavior: a verdict of `fail` is also preserved as a Misc event so the user can audit |
| 6 | Diversity injection | not implemented | NEW: when stuck-tracker hits 2 duplicates, the strategy-change prompt is itself rotated through a small set of phrasings (4 variants) and includes one specific recent observation by reference, to break few-shot lock-in |

PRINCIPLE #16 (RAM/disk) is implicit in the `feature-list.json` design; the
`progress.txt` recitation closes PRINCIPLE #4.

### 2.4 Verifier Worker — L4 meta-cognition (Option B literal)

This is the marquee Phase 1 component. ARCHITECTURE.md §6 L4 literally:
three triggers, FAIL-biased prompt, separate model, fresh context, output
either drives `plan_update` or returns to user.

#### 2.4.1 Worker shape

Separate Tokio task per control-plane process. Subscribes via Redis Streams
(consumer group `verifier`) to the `verify_request` stream. Concurrency
limit = 1 active verification per session (FIFO within session); across
sessions, up to `verifier_max_concurrency` (default 2) run in parallel.

A verification run is a single LLM call (no tools — Verifier reads the
event stream and the sandbox files it needs, does not act on the world).

#### 2.4.2 Three trigger sources (literal §6)

All three emit `verify_request` envelopes onto the Redis Stream with the
same shape:

```rust
struct VerifyRequest {
    session_id: SessionId,
    trigger: VerifyTrigger,        // TaskComplete | Invalidation | CircuitBreaker
    triggered_at_event_id: u64,    // anchor in event stream
    context_hint: VerifyContextHint, // see §2.4.4
}

enum VerifyTrigger {
    TaskComplete { final_message_call_id: String },
    Invalidation { reason: InvalidationReason },
    CircuitBreaker { kind: BreakerKind /* Stuck | Cost | MaxSteps | ErrorRate */ },
}
```

**A. TaskComplete trigger** — fires when the agent calls `idle` or
`message_notify_user` with a `final:true` argument flag. The Worker
intercepts BEFORE the session transitions to `FINISHED`; the session
sits in a new transient state `VERIFYING` (see §3.2) until verdict.

**B. Invalidation trigger** — fires from the Invalidation Detector hook
(§2.5) when a later observation contradicts an earlier assertion.
Phase 1 ships **one** heuristic: file content hash mismatch.

Algorithm (in Rust pseudocode):

```rust
// PostToolUse hook tracks every file_read / file_write observation:
let path = obs.normalized_path()?;
let new_sha = sha256(obs.body);
match file_hashes.get(&path) {
    Some(old_sha) if old_sha != &new_sha && !expected_rewrite(&obs.via_tool) => {
        emit_verify_request(VerifyTrigger::Invalidation {
            reason: InvalidationReason::FileMismatch { path, old_sha, new_sha },
        });
    }
    _ => {}
}
file_hashes.insert(path, new_sha);

fn expected_rewrite(tool: &str) -> bool {
    matches!(tool, "file_write" | "file_str_replace")
}
```

**Allow-list semantics**: `file_write` and `file_str_replace` are
deliberate edits and never trigger invalidation. Any other path through
which file content changes (a shell command, a browser-triggered
download) **does** trigger, because the agent's mental model of the file
was bypassed.

**C. CircuitBreaker trigger** — fires when the circuit breaker (§2.5)
trips. In Phase 0, stuck/cost terminations transitioned straight to
ERROR/SUSPENDED; in Phase 1 the breaker first asks the Verifier
*"is the partial work salvageable via `plan_update`, or genuinely done?"*

#### 2.4.3 FAIL-biased system prompt (slot policy)

The verifier slot's system prompt template (loaded from
`/config/prompts/verifier.system.txt`, NOT inlined in Rust):

```
You are an independent reviewer of work performed by an autonomous agent.
Your job is to find ways the work is wrong, incomplete, or unverifiable.
You do NOT collaborate. You do NOT execute tools. You read the recorded
work and decide.

Bias toward FAIL. If you cannot independently confirm a claim, that is a
fail with reason "unverified". Surface-level success signals (a tool
returned ok=true) are NOT confirmation of work correctness.

Output exactly one JSON object:
{
  "verdict": "pass" | "fail",
  "reason": "<one sentence>",
  "evidence_event_ids": [<u64>, ...],
  "suggested_plan_update": { "phases": [...] } | null
}
```

Startup check: `verifier` slot **must** resolve to a different model ID
than `main`. If equal, the server fails to start with a clear error.
Rationale: a model verifying its own output suffers self-consistency
bias (Manus validation Q&A). Implementing this as a startup gate
prevents misconfiguration silently degrading L4.

#### 2.4.4 Fresh context construction

The Verifier Worker constructs its own context per run:

1. **Plan snapshot** at `triggered_at_event_id` (from `plans` table +
   event replay if mutated since).
2. **Feature list** — current contents of `feature-list.json` (read via
   sandbox file API).
3. **Anchored window** of events: ±N events around `triggered_at_event_id`
   (default N=50). Older events not included.
4. **Trigger description** — one paragraph describing why this Verifier
   run was started (filled by the trigger source).
5. **System prompt** as in §2.4.3.

The agent's running context is **never** passed to the Verifier. Fresh
context is structurally enforced by constructing the prompt in the
Worker, not by passing a snapshot from the agent runner.

#### 2.4.5 Verdict handling

| Verdict | Effect |
|---|---|
| `pass` (TaskComplete trigger) | Session transitions `VERIFYING → FINISHED`; agent's final message delivered to user |
| `pass` (Invalidation / CircuitBreaker) | Misc event emitted; agent loop continues unchanged |
| `fail` with `suggested_plan_update` | Verifier calls `plan_update(suggested_phases)` directly (Verifier IS authorized to mutate the plan); agent loop resumes; if session was `VERIFYING`, returns to `RUNNING`; if it was already in a breaker-trigger state, the breaker resets one of its counters (see §2.5) |
| `fail` without suggestion | Session transitions to `SUSPENDED` with a Misc event containing the verdict; user sees the failure and must intervene (via `task_resume` after editing the plan, or `task_cancel`) |

Every Verifier run is persisted (§3.1) regardless of verdict for audit
and Phase 4 Curator analysis.

### 2.5 Circuit Breaker — 4 conditions

Phase 0 had two breakers (stuck, cost). Phase 1 adds two more and
unifies them behind one structure.

| Breaker | Trigger condition | Phase 0 status | Phase 1 behavior |
|---|---|---|---|
| Stuck (loop) | ≥4 consecutive duplicate assistant outputs | story 0.15 — terminates ERROR | enqueue `Verifier{CircuitBreaker(Stuck)}`; if Verifier says `fail` without suggestion, ERROR; if `pass`, reset counter to 0; if `fail` with suggestion, continue |
| Cost | `session.cost_cents ≥ cap_cents` | story 0.16 — SUSPENDED | enqueue `Verifier{CircuitBreaker(Cost)}` before SUSPEND; if Verifier says "salvageable", SUSPEND anyway (cost is a hard cap) but verdict tells user what was achieved |
| MaxSteps | `iteration_count ≥ max_steps` | iteration cap exists in runner | NEW: same behavior as Cost — Verifier consulted for "report what was done", session ends |
| ErrorRate | ≥5 of last 10 Observation events have `ok:false` | NOT in Phase 0 | NEW: enqueue `Verifier{CircuitBreaker(ErrorRate)}`; Verifier verdict drives plan_update OR SUSPEND |

The breaker state machine is a single Tokio actor per session subscribed
to the event stream; counters reset per-breaker on verdict-driven
recoveries.

### 2.6 Git Checkpoint + Rollback

Sandbox workspace is a git working tree from the moment the Initializer
finishes step 4. Checkpoint Manager:

- On `Plan{op:"advance"}` event → `git add -A && git commit -m "phase N: <title>"` in the workspace, store SHA + phase_id + event_id in `checkpoints` (§3.3).
- On Verifier verdict `fail` with `rollback:true` (a future-compat field; Phase 1 default = false, so rollback is opt-in via config) → `git revert --no-commit <sha>` of the last checkpoint, emit Misc `checkpoint_rollback`, send a system message to the agent context describing the revert.
- The LLM **never sees** git: no git-related tools are exposed to it. Checkpoint actions are hook-driven plumbing.

Rationale for opt-in rollback in Phase 1: silent rollback of agent work
without the agent knowing risks worse drift than no rollback at all. Phase 1
delivers the *mechanism* and an admin endpoint to invoke it manually
(§4.1); Phase 2+ may turn on automatic rollback once the Verifier's
precision is validated.

### 2.7 3-track Browser Representation

Every `browser_*` tool invocation produces all three tracks synchronously:

| Track | Source | Storage | Frontend rendering |
|---|---|---|---|
| A. Live noVNC | AIO Sandbox built-in noVNC at port 6080 (Phase 0 story 0.24) | not stored, live only | iframe in BrowserTab |
| B. DOM text snapshot | `browser_view` already returns text; PostBrowserAction hook captures it for all browser_* tools (not just `_view`) | event payload `output.dom_text` if < 16 KB, else file_ref | scrollable text pane |
| C. Screenshot | NEW: PostBrowserAction hook calls sandbox screenshot endpoint, saves PNG to `/workspace/.tracks/<call_id>.png` | file_ref in Misc event `kind:"browser_track_c"` | horizontal strip; click → fullsize |

Cost: one extra sandbox HTTP call per browser action. AIO Sandbox already
exposes screenshot capture; we use it instead of evaluating JS in-page.

### 2.8 Narrator Hook

PreToolUse hook, runs **before** every tool dispatch. Decision matrix:

| Tool category | Source | Cost |
|---|---|---|
| `plan_*`, `idle`, `feature_mark_done`, `progress_update`, `checkpoint_label` | templated (Rust `format!`) | 0 tokens |
| `file_read`, `file_find_by_name`, `file_find_in_content`, `glossary_lookup`, `playbook_search`, `sop_read` | templated | 0 tokens |
| `file_write`, `file_str_replace`, `shell_*`, `browser_*`, `info_search_web`, `deploy_*`, `message_*` | LLM via `classifier` slot (cheap-model) | ~50 tokens per call |

Narration is emitted as `Message{role:"assistant", content, ui:"narrate"}`.
The frontend renders these in a distinct lane (lighter weight than the
agent's actual reply text). They are **not** included in subsequent agent
context — they are pure UI signal.

Configurable per-deployment via `narrator.enabled = bool`,
`narrator.llm_path = list<tool_glob>`. Defaults are conservative: LLM
narration enabled for the action-changing tools only.

---

## 3. Data model changes

Phase 0 tables (`sessions`, `events`, `plans`) are unchanged at the
schema level. The `sessions.state` CHECK constraint gains one value.

### 3.1 New table — `verifications`

```sql
CREATE TABLE verifications (
  id                        TEXT PRIMARY KEY,                  -- UUID v4
  session_id                TEXT NOT NULL REFERENCES sessions(id),
  triggered_at_event_id     INTEGER NOT NULL,                  -- anchor in events
  trigger_kind              TEXT NOT NULL CHECK(trigger_kind IN
                              ('TaskComplete','Invalidation','CircuitBreaker')),
  trigger_detail            TEXT NOT NULL,                     -- JSON
  verdict                   TEXT NOT NULL CHECK(verdict IN ('pass','fail')),
  reason                    TEXT NOT NULL,
  evidence_event_ids        TEXT NOT NULL,                     -- JSON array<u64>
  suggested_plan_update     TEXT,                              -- JSON or NULL
  model_id                  TEXT NOT NULL,                     -- which model returned the verdict
  cost_cents                INTEGER NOT NULL DEFAULT 0,        -- attributed via /cost delta
  created_at                INTEGER NOT NULL                   -- unix epoch µs
);
CREATE INDEX idx_verifications_session ON verifications(session_id, created_at);
CREATE INDEX idx_verifications_verdict ON verifications(verdict);
```

Persisted for every Verifier run regardless of verdict. Powers the
`GET /v1/sessions/:id/verifications` endpoint and Phase 4 Curator.

### 3.2 `sessions.state` CHECK widened

Phase 0:
```sql
state IN ('IDLE','RUNNING','FINISHED','ERROR','SUSPENDED')
```

Phase 1 adds `VERIFYING`:
```sql
state IN ('IDLE','RUNNING','FINISHED','ERROR','SUSPENDED','VERIFYING')
```

Transitions:
- `RUNNING → VERIFYING` on TaskComplete trigger
- `VERIFYING → FINISHED` on pass
- `VERIFYING → RUNNING` on fail-with-suggestion
- `VERIFYING → SUSPENDED` on fail-without-suggestion

Migration: `V004__verifications.sql` adds the table and an idempotent
`ALTER` (via DROP/CREATE of the CHECK via a temp table — refinery
convention) for `sessions.state`.

### 3.3 New table — `checkpoints`

```sql
CREATE TABLE checkpoints (
  id                TEXT PRIMARY KEY,                          -- UUID v4
  session_id        TEXT NOT NULL REFERENCES sessions(id),
  plan_phase_id     INTEGER NOT NULL,                          -- phases[].id at time of commit
  git_sha           TEXT NOT NULL,                             -- 40-char hex
  label             TEXT,                                      -- optional human label (from `checkpoint_label` tool)
  triggered_by_event_id INTEGER NOT NULL,
  rolled_back_at    INTEGER,                                   -- unix epoch µs or NULL
  rolled_back_by    TEXT,                                      -- "verifier" | "admin:<user>"
  created_at        INTEGER NOT NULL
);
CREATE INDEX idx_checkpoints_session ON checkpoints(session_id, created_at);
```

Migration: `V005__checkpoints.sql`.

### 3.4 Event `data` payloads — new `Misc.kind` values

No schema migration. New documented values:

| `Misc.kind` | Producer | Payload extras |
|---|---|---|
| `verifier_request` | Trigger sources | `{ trigger, triggered_at_event_id, request_id }` |
| `verifier_verdict` | Verifier Worker | `{ verdict, reason, evidence_event_ids, suggested_plan_update?, verification_id }` |
| `checkpoint_create` | Checkpoint Manager | `{ checkpoint_id, plan_phase_id, git_sha, label? }` |
| `checkpoint_rollback` | Checkpoint Manager | `{ checkpoint_id, git_sha, reason, rolled_back_by }` |
| `feature_done` | `feature_mark_done` tool | `{ feature_id, title }` |
| `progress_recite` | Context Recitation | `{ progress_path, content_preview }` |
| `browser_track_b` | 3-track hook | `{ call_id, dom_text_ref }` (story 1.16; side-channel for Observation's DOM text — see story-1.16.md execution notes) |
| `browser_track_b_skipped` | 3-track hook | `{ call_id, reason }` |
| `browser_track_c` | 3-track hook | `{ call_id, file_ref }` |
| `browser_track_c_skipped` | 3-track hook | `{ call_id, reason }` |
| `narration` (NOT a `Misc.kind`) | — | Emitted as a Message event with `ui:"narrate"`, not as Misc |

### 3.5 `feature-list.json` schema (workspace file, not DB)

Stored at `/workspace/feature-list.json` in the sandbox. JSON shape:

```typescript
type FeatureList = {
  version: 1,
  goal: string,
  features: Array<{
    id: string,                          // stable identifier (e.g. "f-1")
    title: string,
    status: "todo" | "doing" | "done",
    depends_on?: Array<string>,
    plan_phase_id: number,               // which plan phase covers this
    completed_at?: number,               // unix epoch µs, when status flipped to done
    notes?: string                       // optional one-line agent note
  }>
};
```

Reads/writes go through the sandbox file API. The Plan Manager keeps
`feature-list.json` in sync with the structured plan on every
`plan_update` (one-shot rewrite). The `feature_mark_done(feature_id)`
tool updates only the `status` field of a single feature.

### 3.6 `progress.txt` format (workspace file, not DB)

Append-only plain text. One line per significant event:

```
2026-05-12T13:42:01Z  init           goal: <goal>
2026-05-12T13:42:03Z  plan           phases: 1) ... 2) ... 3) ...
2026-05-12T13:42:30Z  feature-done   f-1  <title>
2026-05-12T13:45:11Z  phase-advance  2/3
2026-05-12T13:50:42Z  recite         (snapshot included below)
...
```

The Context Recitation timer reads the whole file every 10 iterations
and injects its tail (last 80 lines) into the next agent context as a
`Misc{kind:"progress_recite"}` event so the agent must consume it
before acting.

---

## 4. API surface

### 4.1 New HTTP routes

| Method | Path | Purpose |
|---|---|---|
| GET  | `/v1/sessions/:id/verifications` | List Verifier runs for a session (newest first, paginated) |
| GET  | `/v1/verifications/:id` | One Verifier run (full evidence + suggested_plan_update) |
| GET  | `/v1/sessions/:id/checkpoints` | List checkpoints (newest first) |
| POST | `/v1/sessions/:id/checkpoints/:checkpoint_id/rollback` | **Admin-only**, body `{ reason: string }`. Returns 202 if accepted; emits Misc `checkpoint_rollback`; session must be in `SUSPENDED` state (rollback while RUNNING is rejected). |
| GET  | `/v1/sessions/:id/feature-list` | Proxy read of `/workspace/feature-list.json` |
| GET  | `/v1/sessions/:id/progress` | Proxy read of `/workspace/progress.txt` (paginated by line, default last 200 lines) |

Phase 1 stays **localhost-only single-user** (Phase 0 §9 unchanged). The
"admin-only" rollback endpoint is gated by a `127.0.0.1` bind check plus
an env var `SEASONED_HAND_ADMIN_TOKEN` that must be supplied as a header.
Phase 5 replaces this with proper auth.

### 4.2 WebSocket protocol — additions

Envelope unchanged. New `EventPayload` variants surface via the existing
`event` envelope:

```typescript
type EventPayload =
  | { kind: "Message";     role: "user"|"assistant"; content: string;
                           ui?: "notify"|"ask"|"narrate" }              // <-- "narrate" added
  | { kind: "Action";      tool: string; args: object; call_id: string }
  | { kind: "Observation"; ... }                                         // unchanged
  | { kind: "Plan";        op: "create"|"advance"|"update"; ... }        // unchanged
  | { kind: "Misc";        kind_tag: string };                           // unchanged shape, new kind_tags (see §3.4)
```

New CommandPayloads: **none in Phase 1**. Rollback is HTTP-only because
it's destructive; we want the explicit body and the admin token.

### 4.3 Tool catalog additions

Inherits the Phase 0 catalog of 33 tools (including `plan_advance`).
Adds:

| Tool | LLM-visible? | Backend | Notes |
|---|---|---|---|
| `feature_mark_done(feature_id)` | Yes | Internal | Mutates `feature-list.json`; emits Misc `feature_done` |
| `progress_update(line)` | Yes | Internal | Appends one line to `progress.txt`; no event (the file IS the audit) |
| `checkpoint_label(label)` | Yes | Internal | Adds a human-readable label to the **next** PostPhaseAdvance checkpoint; consumed once |
| `checkpoint_rollback` (internal-only) | **No** | Internal | Invoked by Verifier verdict or admin endpoint; never exposed via Bifrost tool schema |

Net catalog size visible to LLM after Phase 1: **36** (33 from Phase 0
+ 3 new). `checkpoint_rollback` is internal-only; `plan_create` remains
internal-only as well. Phase 0 DEBT #4 reconciliation must update
ARCHITECTURE.md to describe the catalog count formula rather than a
fixed number, since Phase 1 grows it and Phase 3+ will too
(`sop_*`, `playbook_*` writers).

### 4.4 Bifrost interface

Unchanged at the network level. Phase 1 simply exercises slots that
Phase 0 left at `auto`:

- `verifier` slot — required, validated at startup against `main`
- `planner` slot — already used in Phase 0; Phase 1 changes nothing
- `classifier` slot — newly used by the Narrator hook
- All other auxiliary slots — unchanged

`bifrost/config.yaml`: no edits needed if user supplies different model
IDs for `agent-primary` (main), a different alias for `verifier`, and a
cheap alias for `classifier`. Stories will document the recommended
defaults. Suggested in this spec:

```yaml
# Suggested Phase 1 slot defaults (not a hard contract)
slots:
  main:       agent-primary       # e.g. claude-sonnet-4-6
  planner:    agent-primary
  verifier:   agent-fallback      # e.g. gpt-5.x — different vendor on purpose
  classifier: local-fast          # e.g. llama3.2:3b
```

The startup check enforces `verifier ≠ main` at the resolved-model-ID
level, not at the slot-alias level.

---

## 5. External dependencies

### 5.1 Rust crates — additions

```toml
sha2            = "0.10"             # invalidation hash; no new transitive surface
git2            = { version = "0.18", default-features = false, features = ["vendored-libgit2"] }
                                     # control-plane is NOT writing the workspace; see below
```

**Decision: do not use `git2` from the control plane.** Workspace lives
inside the sandbox; the control plane drives git via the existing sandbox
shell tool path (`shell_exec git commit …`). This keeps the security
boundary clean (host process never executes user-content code) and
avoids a heavyweight C dependency. **`git2` is therefore NOT added.**
Only `sha2` is new.

### 5.2 External services

Unchanged from Phase 0. No new containers.

AIO Sandbox image must include `git` (already present in the pinned
`ghcr.io/agent-infra/sandbox:1.0.0.152` per upstream README). Story-level
verification: a smoke test runs `shell_exec git --version` against a
fresh container and asserts the binary exists.

### 5.3 Frontend dependencies

No new dependencies. New UI surface (3-track BrowserTab, narration lane,
verdict pane) is built from React + existing primitives. The screenshot
strip in Track C uses `<img src={workspace_proxy_url}>` over the existing
`/v1/workspace/:session_id/*path` route — no new image library.

---

## 6. Interactions with existing components

| Phase 0 component | Change in Phase 1 |
|---|---|
| Event stream writer | Add new `Misc.kind` values to the documented set (no schema change). |
| Event stream subscribe | No change. |
| Tool dispatcher | Add tool-masking layer (§2.3 principle #2). Existing 33 tools unchanged. |
| Tool registry | Register 3 new LLM-visible tools + 1 internal tool. |
| Agent runtime ReAct loop | Initializer wraps the loop; loop itself unchanged except for sticky-context render (now uses Plan Manager). |
| Stuck tracker | Add diversity injection to the strategy-change prompt path; counters and termination thresholds unchanged. |
| Cost client | Unchanged. Verifier cost is captured as a per-run delta from the same `/cost` poll. |
| Plan Manager | Phase 0 stubs → real impl (DEBT #25 closed). Schema unchanged. |
| Hooks (PreToolUse / PostToolUse / PostToolUseFailure) | Phase 1 adds **PostPhaseAdvance** and **PostBrowserAction** hook points. Phase 0 hook surface stays. Narrator runs as PreToolUse. Invalidation Detector runs as PostToolUse. Hook ordering documented per-point. |
| Sandbox client | Workspace bootstrap writes `feature-list.json` + `progress.txt` and runs `git init` + initial empty commit. Cleanup unchanged. |
| Sandbox handle cache (DEBT #18) | **Pay-down**: on startup, scan Docker for `seasoned-hand-sandbox-*` containers and rehydrate (deferred from Phase 0 retro). Lives in this phase. |
| 12-slot router | No code change. Adds startup validation: `verifier ≠ main` (resolved-model-ID). |
| WebSocket server task control (`task_pause`/`task_resume`/`task_cancel`) | Phase 0 stubs → real (DEBT #27 closed): wire per-session cancel tokens and sandbox `bollard pause/unpause`; runner checks cancel token between iterations and at every `await` checkpoint. |
| Capability table fallback (DEBT #22) | Resolve Bifrost aliases to provider model IDs at startup, then probe capability via static table; remove the agent-primary/agent-fallback hardcoded assumption. |
| Hook output-truncation (DEBT #21) | Replace inline preview fallback with the sandbox-file-write path now that broader sandbox-tool wiring is in place. |
| Frontend | Three new UI elements: narration lane (filter on `ui:"narrate"`), Verifier verdict pane (filter on `Misc{kind:"verifier_*"}`), 3-track BrowserTab in AgentComputer. No automated test debt is paid down in this phase — flagged in DEBT.md. |

The set of Phase 0 DEBT items that this phase **does** pay down:
**#18, #21, #22, #25, #27** (plus the new ones added to phase-1/DEBT.md).
Items still deferred to later phases: #1 (DbPool), #7/#8 (auth), #15
(seccomp tightening — Phase 1 considers but does not require), #16
(workspace cleanup retention policy), #27-frontend-tests.

---

## 7. Performance budget

Phase 0 budgets carry forward unchanged. New budgets for Phase 1
components:

| Budget | Target | Owner | Verification |
|---|---|---|---|
| Verifier run latency | < 6 s p95 (1 LLM call, < 5K tokens) | Verifier Worker | tracing span; CI smoke test against fake OpenAI |
| Verifier cost per task | < 5 ¢ per task with ≤3 trigger fires | Verifier Worker | unit test on token estimator |
| Narration overhead per LLM-narrated tool call | < 200 ms additional latency | Narrator Hook | tracing span |
| Narration cost per task | < 2 ¢ per 50-step task at default policy | Narrator Hook | criterion bench |
| Plan render (post-real Plan Manager) | unchanged 1000-token cap | Plan Manager | unit test on serializer |
| Checkpoint create latency | < 300 ms (in-sandbox git commit) | Checkpoint Manager | sandbox integration test |
| Invalidation hash bookkeeping | O(1) per file observation; < 1 MB heap per session even with 10k files | Invalidation Detector | unit test on hash map growth bound |
| Cold start with Initializer | < 12 s (adds plan_create + workspace bootstrap on top of Phase 0's 10 s budget) | Initializer | E2E timer |
| End-to-end 50-step task wall time | < 4 min p50 with main=Claude-class model | full stack | E2E test in story-1.x suite |

Cost cap per session: default raised from $1 (Phase 0) to $2 to
accommodate the additional Verifier + Narrator spend. Still
configurable per task via `task_create.cost_cap_cents`. Rationale: 50-step
deep tasks at Phase 0's $1 ceiling routinely tripped the cap before the
agent could complete; doubling buys headroom for verification overhead
on top of real work without removing the safety rail.

---

## 8. Failure modes

Phase 0 failure modes (§8) carry forward unchanged. New failure modes
introduced by Phase 1 components:

| Failure | Detection | Handling |
|---|---|---|
| Verifier slot resolves to same model as `main` | startup check | Server fails to start with an actionable error pointing at `slots.yaml` |
| Verifier LLM returns malformed JSON | serde parse error | Retry once with stricter prompt; on second failure, treat as `fail{reason:"verifier_unparseable"}` and SUSPEND. NEVER infer pass from a parse failure (PRINCIPLE #10) |
| Verifier slot returns 5xx via Bifrost | reqwest status | Treat as `fail{reason:"verifier_unavailable"}` — but allow override config `verifier.fail_open=false` (default) to instead pause the session until verifier comes back |
| Invalidation false positive (user/script edits a file out of band) | impossible to detect from observations alone | Mitigation: invalidation reason carries the contradicted observation event_id; the agent's next iteration receives a Misc `verifier_request` and can dismiss via `progress_update` rather than `plan_update`. The verdict still fires; verdict-as-noise is acceptable if rare |
| Checkpoint git commit fails (sandbox disk full, etc.) | sandbox shell exit code | Emit Misc `checkpoint_create` with `ok:false`; session continues. Rollback for that phase is then unavailable; recorded as DEBT entry in `checkpoints.rolled_back_by` field as `"unavailable"` |
| Rollback attempted while sandbox container is paused | bollard inspect | Reject with 409 from the admin endpoint; require `task_resume` first |
| Worker / verifier scheduling deadlock (Redis Streams consumer drops session) | watchdog: VERIFYING > 60 s with no verdict | Emit Misc `verifier_watchdog`; transition session to SUSPENDED with reason `"verifier_timeout"` |
| Initializer plan_create returns 0-phase plan | post-parse validation | Reject; Initializer falls back to single-phase plan (Phase 0 §8 mechanism reused) |
| `feature-list.json` missing or corrupt mid-task | file_read returns error or invalid JSON | Recreate from current Plan Manager state; emit Misc `feature_list_recovered`; warn user |
| Diversity injector exhausts variant set | counter on injector | After all 4 variants used, fall back to stuck-tracker's existing termination path (4 dupes → ERROR) — diversity does not buy infinite retries |
| Narrator LLM is slow / down | reqwest timeout (2 s) | Skip narration silently for this call; emit Misc `narration_skipped`. Tool dispatch proceeds — narration is best-effort UI, never blocks |

---

## 9. Security considerations

Phase 1 inherits Phase 0's posture (localhost-only, single-user) and
adds:

- **Verifier-as-actuator**: the Verifier Worker can call `plan_update`
  directly. This is an in-process trusted call, not exposed via HTTP/WS.
  No new external attack surface; the verifier prompt is loaded from
  disk and the model output is constrained to a JSON schema before any
  plan mutation is accepted.
- **Plan-mutation chain integrity**: every plan mutation emits a Plan
  event with `op` and `snapshot`. Verifier-driven mutations are tagged
  in the event source as `source:"verifier"` so the audit trail
  distinguishes agent-initiated from verifier-initiated updates.
- **Admin rollback endpoint**: `127.0.0.1` bind + `SEASONED_HAND_ADMIN_TOKEN`
  header. Phase 5 replaces both with real auth. The endpoint refuses
  rollback while session state is `RUNNING` to prevent racing the agent.
- **Sandbox seccomp** (Phase 0 DEBT #15): unchanged in Phase 1. Phase 1
  considers a tailored profile but does not deliver one — flagged in
  `phase-1/DEBT.md`.
- **Egress allowlisting**: Phase 0 deferred this to Phase 1. Phase 1
  introduces a config flag `sandbox.egress_allowlist = ["*"|...]` and
  ships the `*` (permissive) default to preserve current behavior. The
  enforcement path is plumbed but the deny default ships in Phase 5.
  This is explicit half-step: the *config surface* exists in Phase 1 so
  Phase 5 deny-default doesn't require a schema change.
- **Workspace path traversal**: unchanged from Phase 0.
- **Prompt injection from tool outputs** (Phase 0 §9 deferral): partial
  mitigation arrives via L2 cross-source validation as the Verifier
  reads two independent sources for any factual claim. Phase 1 does not
  build a dedicated prompt-injection sanitizer; Phase 4+ may.
- **Secrets in event stream**: unchanged. Note that the Verifier reads
  the event stream including any secrets the agent may have surfaced;
  this is fine in single-user Phase 1 but is a Phase 5 redaction
  requirement.

---

## 10. Migration plan

Two new migrations land in this phase:

```
/migrations/
  V001__sessions.sql        (Phase 0)
  V002__events.sql          (Phase 0)
  V003__plans.sql           (Phase 0)
  V004__verifications.sql   (NEW: table + sessions.state CHECK widening)
  V005__checkpoints.sql     (NEW)
```

`V004` widens `sessions.state` CHECK to include `'VERIFYING'`. SQLite
does not support `ALTER TABLE ... ALTER CONSTRAINT`, so refinery's
recommended pattern is: create new table with the wider CHECK, copy
rows, drop old, rename. Migration is idempotent (re-running on a
populated database succeeds without data loss). A migration test runs
in CI.

The control plane upgrade is **backwards-compatible** at the API level:

- Frontend that doesn't know about `ui:"narrate"` falls back to rendering
  as regular assistant messages.
- Frontend that doesn't know about `Misc.kind:"verifier_*"` ignores
  those events (Phase 0 frontend behavior is to render Misc as muted
  audit lines).
- Existing sessions from Phase 0 (state = IDLE/RUNNING/FINISHED/ERROR
  /SUSPENDED) are unaffected; the new VERIFYING state is reachable only
  via Phase 1 code paths.

No data backfill required. Verifier coverage applies only to new tasks
created on Phase 1 code; existing FINISHED sessions are not retroactively
verified.

---

## 11. Testing strategy

### 11.1 Unit (per story)

- Plan Manager: create / advance / update / serialization round-trip.
- Verifier system prompt loader (config-driven, file-backed).
- Verifier verdict JSON schema enforcement (parse failures handled).
- Invalidation Detector: hash bookkeeping; allow-list correctness; false-positive shape on out-of-band edits.
- Circuit Breaker state machine: each breaker's transitions, including verdict-driven recoveries.
- Narrator: templated path returns deterministic string per (tool, args).
- Tool-mask: masked tools still appear in the schema with `available:false`; KV-prefix stability verified by a property test.
- Diversity Injector: each of 4 variants emitted at least once across 4 stuck cycles.

### 11.2 Integration

- **Initializer → Worker happy path**: synthetic `task_create` produces feature-list.json + progress.txt + plan + first iteration entered. Real SQLite, real Redis.
- **Verifier Worker round-trip**: mock LLM client returns canned JSON; assert verdict landed in `verifications` + Misc emitted + plan mutated for fail-with-suggestion.
- **Invalidation hook end-to-end**: write a file via `file_write`, then have a `shell_exec` overwrite it, assert a `verifier_request` is enqueued with reason `FileMismatch`.
- **Checkpoint Manager**: PostPhaseAdvance hook against a live sandbox container, assert `git log` shows the commit; rollback path triggered manually and verified by `git status`.
- **Circuit Breaker**: synthesize 5/10 failing observations and assert the ErrorRate breaker fires once and only once.
- **Startup gate**: configure `slots.yaml` with verifier=main; server boot must fail with the documented error message.

### 11.3 E2E (Phase 1 closing story)

Acceptance test from ROADMAP §"Phase 1":

- "GAIA Level 1-style tasks succeed ≥80%" — use a curated 10-task fixture
  set (mix of browse + extract + summarize + multi-step coding) and
  require ≥8 passes. Failing tasks are recorded but do not fail CI; they
  are escalated to story spec for analysis.
- "50+ tool call sessions stable" — one synthetic task designed to
  exceed 50 iterations (multi-page browsing + summarize). E2E timer
  must show no Stuck termination, no MaxSteps termination, no cost
  overrun beyond budget.
- "Verifier fires on completion" — assert every successful E2E task
  produces exactly one `TaskComplete` Verifier run with verdict `pass`.

### 11.4 Live-LLM "smoke" job

CI gains a `workflow_dispatch` job (mirror of Phase 0 retro item 14)
that runs ONE 50-step task against real Bifrost + real Claude/GPT
models when `ANTHROPIC_API_KEY` and `OPENAI_API_KEY` are present. Gated
behind a cost cap of $0.50 per run and a 10-minute timeout. NOT on the
default `cargo test` path.

---

## 12. Open technical questions

These remain for the BMAD PM persona to either resolve during story
breakdown or carry forward as story-scoped open questions.

1. **Verifier evidence_event_ids resolution** — when the verdict cites
   `evidence_event_ids` that the frontend doesn't have cached, how
   should the verdict pane render? Decision needed: pre-fetch on
   verdict arrival vs lazy-load on click. Recommendation: lazy.
2. **Narration storage cost** — narration emits Message events. Are
   those events included in the agent's own context? Decision: **no**
   (per §2.8), but how exactly do we filter them out at the sticky-context
   builder boundary? Probably a new `Message.ui:"narrate"` filter in
   `build_iteration_context()`. Story-level detail.
3. **Diversity Injector variant source** — variants stored where? In a
   Rust constant array, a YAML file, or a database table? Phase 1
   default: Rust constant array. Phase 4 Curator may promote to DB.
4. **Multi-Verifier strategy** — Phase 1 spec ships one verifier slot.
   ARCHITECTURE.md §3 leaves room for distinct verifier instances per
   trigger type. Defer: single verifier slot for all 3 triggers in
   Phase 1; phase-1/DEBT.md tracks the question for Phase 4.
5. **Sandbox `git` config user identity** — `git commit` requires a
   user.name/user.email. Decision: Initializer runs
   `git config user.email "seasoned-hand@local"` and
   `git config user.name "Seasoned Hand"` at workspace bootstrap. Story-
   level detail.
6. **Feature-list ↔ Plan synchronization edge case** — what if the
   agent calls `feature_mark_done` for a feature whose `plan_phase_id`
   is not currently active? Decision: allow it (the agent may finish
   work across phases) but emit a Misc `feature_done_out_of_phase` for
   audit.
7. **PostBrowserAction screenshot size** — full-resolution PNGs of a
   1280×800 noVNC view are ~200-400 KB. At 50 browser steps per task,
   workspace files grow ~15 MB. Acceptable; Phase 1 does not paginate
   or thumbnail. Cleanup tied to existing workspace TTL (Phase 0
   DEBT #16, still pending).
8. **Verifier evidence anchoring after `plan_update`** — verdicts cite
   event_ids that may pre-date a plan update. Are those still valid
   evidence? Decision: **yes** — event stream is append-only and event
   IDs are stable. Verdicts remain valid audit artifacts even after
   the plan they critique has been replaced.

---

## 13. Story breakdown stub (for PM persona)

This is **not** the PM phase output; just a sketch of natural seams in
this architecture, intended to make the PM's job easier without
prescribing it:

- 1.1 — Real Plan Manager (DEBT #25 close, 3 plan tools wired)
- 1.2 — Initializer + feature-list.json + progress.txt bootstrap
- 1.3 — Context principles enforcement (tool-mask, recitation, diversity injector)
- 1.4 — `verifier` slot startup gate + Verifier Worker scaffolding (no triggers yet)
- 1.5 — TaskComplete trigger + verdict handling
- 1.6 — Invalidation Detector + Invalidation trigger
- 1.7 — Circuit Breaker unification (4 conditions) + CircuitBreaker trigger
- 1.8 — Checkpoint Manager (commit on phase advance; admin rollback endpoint)
- 1.9 — Narrator Hook (templated + classifier-slot LLM path)
- 1.10 — 3-track Browser representation (capture + frontend strip)
- 1.11 — DEBT pay-downs (#18 sandbox cache, #21 hook truncation, #22 capability, #27 task_pause/resume/cancel)
- 1.12 — E2E + Phase 1 acceptance + retrospective

Approximate scope: 12 stories, ~3-5 hours each = within the 4-week
Phase 1 timebox at solo-with-Codex velocity (Phase 0 closed 27 stories
in roughly that window).

---

## 14. Architecture is at `/specs/phase-1/architecture.md`

When approved, start a fresh session with the **BMAD PM persona** at
`/prompts/bmad-pm.md` to break this document into `/specs/phase-1/stories/story-1.X.md`
files.
