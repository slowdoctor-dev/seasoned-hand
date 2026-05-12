# Story 1.1 — Real Plan Manager (close DEBT #25)

> **Status**: done
> **Estimated**: 3 hours
> **Dependencies**: none (front-loaded per Phase 1 plan)
> **Phase**: 1
> **Type**: backend
> **Reads first**: `/specs/phase-1/architecture.md` §2.2 (Plan Manager) and
> §3 (data model), `/specs/01-architecture/decisions/ADR-010-plan-as-process-control-block.md`,
> `/specs/phase-0/stories/story-0.14.md` §3 (current baseline plan
> creation), `/specs/phase-0/DEBT.md` #25 (what this closes).

---

## Goal

Replace Phase 0's `not_implemented` stubs in `plan_create`, `plan_advance`,
`plan_update` with real bodies wired to the existing `plans` table. Replace
the agent runtime's raw-event plan rendering with a structured sticky-context
block (`Goal / Phase 1 [done] / Phase 2 [active] / Phase 3 [pending]`) capped
at 1000 tokens. Every mutation emits a `Plan{op,...}` event. This is the
foundation for every other Phase 1 story that touches plans (Initializer,
Verifier suggested updates, Checkpoint Manager phase advances).

## Acceptance criteria

- [ ] `seasoned-hand-core::plan::PlanManager` struct exposes
      `create(session_id, goal, phases) -> Plan`,
      `advance(session_id) -> Plan` (auto-pick next pending phase),
      `update(session_id, phases, source: PlanMutationSource) -> Plan`.
      `PlanMutationSource = Agent | Verifier | Runtime`.
- [ ] `plan_create`, `plan_advance`, `plan_update` tools delegate to
      `PlanManager` and return real JSON (`{ok:true, plan: <snapshot>}`
      instead of `not_implemented`). `plan_create` remains LLM-masked
      (see story 1.5 — for now use a feature-flag on the tool registry).
- [ ] Sticky-context render replaces the Phase 0 `format!("PLAN: {}", plan_event.data)`
      one-liner with the block shown in architecture.md §2.2:
      ```
      == PLAN ==
      Goal: <goal>
      Phase 1 [done]: <title>
      Phase 2 [active]: <title>
      Phase 3 [pending]: <title>
      == END PLAN ==
      ```
      Truncate individual titles before dropping structure; never exceed
      1000 tokens (estimated via `tiktoken_rs::p50k_base` or the existing
      Phase 0 token estimator — pick whichever is already a workspace dep).
- [ ] Phase 0 DEBT #25 entry struck through with date + commit ref.
- [ ] Tests:
      - `plan_create_inserts_row_emits_event` — round-trip via rusqlite memory DB.
      - `plan_advance_auto_picks_next_pending` — 3 phases, advance twice, third remains pending after first call.
      - `plan_update_replaces_phases_and_resets_current` — `current_phase_id` becomes lowest pending after update.
      - `plan_update_tags_source_in_event_data` — `Verifier` source visible in emitted Plan event payload.
      - `sticky_render_under_1000_tokens_long_titles` — synthesize 20 phases with long titles; render result is ≤ 1000 tokens; structure preserved.
      - `agent_runner_uses_structured_render` — wiremock'd Bifrost asserts the system message body contains the `== PLAN ==` block.

## Non-goals

- Initializer-driven first plan (story 1.4 — this story leaves baseline
  single-phase plan creation from story 0.14 in place; story 1.4 swaps
  the caller).
- `plan_create` being masked from the LLM mid-loop (story 1.5 — tool-mask
  layer; this story can ship with `plan_create` still LLM-visible).
- Verifier calling `plan_update` directly (story 1.10 — this story only
  *accepts* a `Verifier` source enum value).
- Migration of existing `plans` rows — none exist in production yet
  (Phase 0 closed with stubs that wrote nothing).

---

## Implementation steps

### 1. Module layout

```
crates/seasoned-hand-core/src/plan/
  mod.rs       — PlanManager, Plan, Phase, PlanMutationSource, errors
  render.rs    — sticky_render(plan) -> String + token-cap helper
  tools.rs     — wires plan_create/advance/update tool bodies
  tests.rs
```

`plan_advance` / `plan_update` tool structs already exist (story 0.7);
this story replaces their `dispatch` bodies. `plan_create` is internal-only
(used by Initializer — story 1.4 — and once, baseline from story 0.14).

### 2. Types

```rust
// crates/seasoned-hand-core/src/plan/mod.rs
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Phase {
    pub id: u32,                        // 1-based, stable within a plan
    pub title: String,
    pub status: PhaseStatus,            // Pending | Active | Done
    pub capabilities: Vec<String>,      // for diversity injection later
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PhaseStatus { Pending, Active, Done }

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Plan {
    pub session_id: String,
    pub goal: String,
    pub phases: Vec<Phase>,
    pub current_phase_id: Option<u32>,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlanMutationSource { Agent, Verifier, Runtime }

pub struct PlanManager { pool: Arc<DbPool>, events: Arc<dyn EventStore> }
```

### 3. Operations

`PlanManager::create` — SELECT existing row (UNIQUE on `session_id`); reject
with `PlanError::AlreadyExists` if a non-empty plan is already present. INSERT
otherwise. Set `current_phase_id = phases[0].id` and mark `phases[0].status = Active`.

`PlanManager::advance` — load plan; flip current Active → Done; find lowest
`id` with `Pending` status, mark it Active, set `current_phase_id`. If no
pending remains, leave `current_phase_id = None` and emit
`Plan{op:"advance", terminal:true}`.

`PlanManager::update(phases, source)` — replace `phases` JSON wholesale.
Recompute `current_phase_id` to lowest `Pending` id (or first `Active` if
one was supplied). Emit `Plan{op:"update", source}`.

All three operations: single transaction over the `plans` table; event emit
happens after commit. `Plan` event payload schema is unchanged from Phase 0
but gains a `source: "agent"|"verifier"|"runtime"` field for `update`.

### 4. Sticky render

```rust
// crates/seasoned-hand-core/src/plan/render.rs
pub fn sticky_render(plan: &Plan, token_cap: usize) -> String {
    let header = "== PLAN ==\n";
    let goal = format!("Goal: {}\n", plan.goal);
    let body = plan.phases.iter().map(|p| {
        let marker = match p.status {
            PhaseStatus::Done    => "[done]",
            PhaseStatus::Active  => "[active]",
            PhaseStatus::Pending => "[pending]",
        };
        format!("Phase {} {}: {}\n", p.id, marker, p.title)
    }).collect::<String>();
    let footer = "== END PLAN ==\n";
    let mut out = format!("{header}{goal}{body}{footer}");
    if estimate_tokens(&out) > token_cap {
        out = truncate_to_token_cap(out, token_cap);
    }
    out
}
```

`truncate_to_token_cap` trims **phase titles** first (longest first, ellipsis
suffix). It never drops a phase entirely. If after all titles are trimmed to
`"…"` the cap is still exceeded, drop oldest `Done` phases (preserve all
Active/Pending). Property test ensures the structure (`== PLAN ==` header,
`== END PLAN ==` footer, one line per phase) is always preserved.

Reuse the existing Phase 0 token estimator if present; otherwise add
`tiktoken-rs = "0.5"` to `seasoned-hand-core/Cargo.toml` and use
`tiktoken_rs::p50k_base()`. Check `cargo tree` before adding — if a
heuristic estimator already exists (e.g. `chars / 4`), reuse it; the cap
exists to bound runaway plans, not for byte-perfect accounting.

### 5. Agent runner integration

In `agent::prompt::build_messages` (story 0.14), replace:

```rust
out.push(Message {
    role: Role::System,
    content: Some(format!("PLAN: {}", plan_event.data)),
    ..Default::default()
});
```

with:

```rust
let plan = self.plan_manager.snapshot(session_id).await?;
out.push(Message {
    role: Role::System,
    content: Some(sticky_render(&plan, 1000)),
    ..Default::default()
});
```

`AppState` builds `plan_manager: Arc<PlanManager>`; `AgentRunner` holds a clone.

### 6. Tool dispatch wiring

`plan_advance` / `plan_update` tools (story 0.7 stubs) get real bodies
inside `crates/seasoned-hand-core/src/plan/tools.rs`. The dispatcher
already routes by tool name; no dispatcher change beyond passing the
`PlanManager` handle through `ToolContext`.

`plan_create` tool: keep registered, but only invoked internally
(`PlanManager::create` is a Rust call; the tool body is in place for
story 1.4 to call via `ToolDispatcher::dispatch` directly without LLM
involvement). Story 1.5 will hide it from LLM via the tool-mask layer;
for now it stays LLM-visible — that's fine because the runtime calls it
once at task start (story 0.14 baseline) and the LLM has no reason to
call it itself.

### 7. DEBT.md update

In `specs/phase-0/DEBT.md`, strike through item #25 with the date and the
new commit ref. In `specs/phase-1/DEBT.md`, no new debt expected (this
story is purely a pay-down).

---

## Verification

```bash
cd <repo-root>
cargo clippy -p seasoned-hand-core -p seasoned-hand-server --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core --lib plan::
cargo test -p seasoned-hand-core agent::tests::agent_runner_uses_structured_render
./scripts/spec-check.sh
```

Expected: 6 new `plan::` tests + 1 modified runner test green; full
workspace test suite green.

---

## Files changed

- `crates/seasoned-hand-core/src/lib.rs` — `pub mod plan;`
- `crates/seasoned-hand-core/src/plan/mod.rs` (new)
- `crates/seasoned-hand-core/src/plan/render.rs` (new)
- `crates/seasoned-hand-core/src/plan/tools.rs` (new)
- `crates/seasoned-hand-core/src/plan/tests.rs` (new)
- `crates/seasoned-hand-core/src/agent/prompt.rs` (modify — call `sticky_render`)
- `crates/seasoned-hand-core/src/tools/plan_advance.rs` (modify — delegate)
- `crates/seasoned-hand-core/src/tools/plan_update.rs` (modify — delegate)
- `crates/seasoned-hand-core/src/tools/plan_create.rs` (modify — delegate)
- `crates/seasoned-hand-core/src/dispatch/context.rs` (modify — add `plan_manager`)
- `crates/seasoned-hand-server/src/state.rs` (modify — build `Arc<PlanManager>`)
- `Cargo.toml` (workspace deps if `tiktoken-rs` is new — only if needed)
- `specs/phase-0/DEBT.md` (close #25)

---

## Spec references

- `/specs/phase-1/architecture.md` §2.2 (Plan Manager spec table), §3 (Plan
  event data carries `source`).
- `/specs/01-architecture/decisions/ADR-010-plan-as-process-control-block.md`
  §"Storage", §"Actions", §"Render".
- `/specs/00-philosophy/PRINCIPLES.md` #17 (plan stickiness), #11 (audit trail).

---

## Commit message

```
feat(phase-1): story 1.1 - real Plan Manager (DEBT #25 close)

- seasoned-hand-core::plan::PlanManager wires create/advance/update to
  the plans table per ADR-010; PlanMutationSource enum tags origin
  (Agent/Verifier/Runtime) into the Plan event data payload
- plan_advance / plan_update tool bodies now real (no more
  not_implemented); plan_create stays internal-call-only (Initializer
  in story 1.4 will be its caller)
- sticky_render() returns the structured == PLAN == block with 1000-
  token cap; truncates titles before dropping structure; agent runner
  build_messages() now uses it instead of raw event JSON
- 6 unit + integration tests
- cargo clippy / fmt / test / spec-check all pass

Closes Phase 0 DEBT #25.

refs: /specs/phase-1/stories/story-1.1.md
```

---

## Notes for next story (1.2)

`PlanManager` now exists as a service available via `AppState`. Story 1.4
(Initializer) will call `PlanManager::create()` directly from its pre-loop
bootstrap. Story 1.10 (Verifier verdict handling) will pass `Source::Verifier`
when applying a suggested update. Story 1.13 (Checkpoint Manager) hooks the
PostPhaseAdvance event emitted from `PlanManager::advance`.

Story 1.2 is independent (sandbox handle-cache rehydration) and can be
worked in parallel by the Codex pair.

## Notes from execution

- Implemented `seasoned-hand-core::plan` with persisted `create/advance/update`
  operations wired to `plans` + `Plan` event emission on each mutation.
- Replaced raw `PLAN: {json}` sticky context with structured `== PLAN ==`
  rendering, token-capped using `tiktoken-rs`.
- Wired plan tools (`plan_advance`, `plan_update`) to real `PlanManager`
  mutations and added tests for plan mutation behavior + structured render.
