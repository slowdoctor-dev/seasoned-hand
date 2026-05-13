# Story 1.4 — Initializer + feature-list.json + progress.txt + 2 LLM tools

> **Status**: done
> **Estimated**: 3 hours
> **Dependencies**: 1.1 (Plan Manager), 1.3 (sandbox workspace git bootstrap)
> **Phase**: 1
> **Type**: backend
> **Reads first**: `/specs/phase-1/architecture.md` §2.1 (Initializer +
> Worker pattern), §3.5 (`feature-list.json` schema), §3.6 (`progress.txt`
> format), §4.3 (new tools), §12 question 6 (`feature_done_out_of_phase`),
> `/specs/phase-1/stories/story-1.1.md` (PlanManager API).

---

## Goal

Add an Initializer phase that runs once per task before the first agent loop
iteration. It calls the `planner` slot to produce a multi-phase plan,
persists the plan via `PlanManager::create`, and writes
`/workspace/feature-list.json` + `/workspace/progress.txt` into the
sandbox. Two new LLM-visible tools (`feature_mark_done`, `progress_update`)
let the agent maintain these files mid-task. Two new HTTP routes proxy
them for the frontend.

## Acceptance criteria

- [ ] `seasoned-hand-core::agent::init::Initializer::run(session_id, briefing)`
      executes (a) `planner`-slot LLM call → structured `{goal, phases[]}`
      JSON, (b) `PlanManager::create(...)`, (c) writes
      `/workspace/feature-list.json` derived from phases, (d) writes
      `/workspace/progress.txt` with the goal + initial feature list, (e)
      returns a `InitReport { plan: Plan, feature_count: usize }`.
- [ ] Initializer does **not** count toward `max_steps`. The Worker
      (existing `AgentRunner::run` from story 0.14) starts iteration 0
      *after* `Initializer::run` returns.
- [ ] On planner-slot failure (network, malformed JSON, 0-phase plan):
      fall back to a single-phase plan with the briefing as both goal and
      phase title (mechanism reused from Phase 0 §8). Emit Misc
      `init_planner_fallback{reason}`.
- [ ] New LLM-visible tools registered in the tool catalog:
      - `feature_mark_done(feature_id: String)` → flips `status` of one
        feature in `feature-list.json` to `"done"` + sets `completed_at`;
        emits `Misc{kind:"feature_done", feature_id, title}`. If the
        feature's `plan_phase_id` is not the currently active phase, also
        emit `Misc{kind:"feature_done_out_of_phase", ...}` (architecture
        §12 q6 decision: allow, but audit).
      - `progress_update(line: String)` → appends one timestamped line to
        `progress.txt`. No event (the file IS the audit). Rate-limit:
        max 200 chars/line; lines beyond that are truncated with `…`.
- [ ] New HTTP routes:
      - `GET /v1/sessions/:id/feature-list` — proxy read of
        `/workspace/feature-list.json` from the sandbox.
      - `GET /v1/sessions/:id/progress?lines=<N>` (default `N=200`) —
        proxy tail of `/workspace/progress.txt`.
- [ ] `feature-list.json` schema validates against the architecture §3.5
      TypeScript-style type definition. JSON Schema (`schemars`-derived
      or hand-written) stored at
      `crates/seasoned-hand-core/src/agent/init/feature_list.schema.json`.
- [ ] Tests:
      - `initializer_writes_feature_list_and_progress` — wiremock'd Bifrost
        returns a valid 3-phase plan; assert both workspace files exist
        with expected contents.
      - `initializer_falls_back_on_zero_phase_plan` — wiremock returns
        `{phases: []}`; assert single-phase fallback + Misc event.
      - `feature_mark_done_flips_status_and_emits_event` — pure-unit on
        the tool body against a temp workspace fixture.
      - `feature_mark_done_out_of_phase_emits_extra_misc` — feature
        belongs to phase 2 but plan is on phase 1.
      - `progress_update_truncates_long_lines` — 500-char input becomes a
        200-char (with `…`) line in the file.
      - `http_feature_list_route_returns_json` — axum test client against
        a synthetic sandbox HTTP mock.
      - `http_progress_route_returns_tail_with_default_lines` — same.

## Non-goals

- Hiding `plan_create` from the LLM tool catalog — that's story 1.5
  (tool-mask). The Initializer calls `PlanManager::create` directly, so
  `plan_create` being LLM-visible meanwhile is harmless.
- The `checkpoint_label` tool — story 1.13.
- Verifier reading `feature-list.json` — story 1.10 / 1.11 wire that.
- Context recitation injecting `progress.txt` tail into the agent context
  — story 1.6.
- WebSocket push of feature-list updates — frontend polls the HTTP routes
  on a 2s timer (story 1.18+ if needed). HTTP proxy is sufficient for
  Phase 1.

---

## Implementation steps

### 1. Module layout

```
crates/seasoned-hand-core/src/agent/init/
  mod.rs               — Initializer, InitReport, InitError
  feature_list.rs      — FeatureList, Feature, read/write helpers
  progress.rs          — write_line, read_tail
  feature_list.schema.json
  tests.rs
crates/seasoned-hand-core/src/tools/
  feature_mark_done.rs — new
  progress_update.rs   — new
crates/seasoned-hand-server/src/routes/
  sessions_feature_list.rs — GET /v1/sessions/:id/feature-list
  sessions_progress.rs     — GET /v1/sessions/:id/progress
```

### 2. Types

```rust
// agent/init/feature_list.rs
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FeatureList {
    pub version: u32,                 // always 1 in Phase 1
    pub goal: String,
    pub features: Vec<Feature>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Feature {
    pub id: String,                   // "f-1", "f-2", ...
    pub title: String,
    pub status: FeatureStatus,        // todo | doing | done
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    pub plan_phase_id: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}
```

### 3. Initializer body

```rust
// agent/init/mod.rs
impl Initializer {
    pub async fn run(&self, session_id: &str, briefing: &str)
        -> Result<InitReport, InitError>
    {
        let plan_json = self.call_planner_slot(briefing).await
            .unwrap_or_else(|e| {
                self.emit_misc(session_id, "init_planner_fallback",
                    json!({"reason": e.to_string()}));
                fallback_single_phase_plan(briefing)
            });
        let parsed = parse_plan_or_fallback(&plan_json, briefing, |reason| {
            self.emit_misc(session_id, "init_planner_fallback",
                json!({"reason": reason}));
        });
        let plan = self.plan_manager.create(session_id, parsed.goal, parsed.phases).await?;
        let feature_list = derive_feature_list(&plan);
        self.sandbox.write_workspace_file(session_id,
            "/workspace/feature-list.json",
            &serde_json::to_vec_pretty(&feature_list)?).await?;
        self.sandbox.write_workspace_file(session_id,
            "/workspace/progress.txt",
            &initial_progress_lines(&plan).into_bytes()).await?;
        Ok(InitReport { plan, feature_count: feature_list.features.len() })
    }
}
```

`call_planner_slot` is a thin wrapper around the existing `LlmClient` that
selects `SlotName::Planner` (Phase 0 router already wires planner). The
system prompt for the planner slot lives at
`/config/prompts/planner.system.txt` (Phase 0 already loads similar prompts;
follow the same pattern — file-on-disk, not inlined).

`derive_feature_list(plan)` walks phases and produces one feature per
phase **for Phase 1**. (Multi-feature-per-phase would require the planner
to emit them; out of Phase 1 scope.) Each feature's `plan_phase_id`
matches the phase id; ids are `f-1`, `f-2`, ... numbered by appearance.

### 4. Wire into AgentRunner

`AgentRunner::run` (story 0.14) gains an early step:

```rust
// before the iteration loop:
let init = self.initializer.run(&req.session_id, &req.input).await?;
// ... existing loop below now does NOT call create_baseline_plan
```

Replace the Phase 0 `create_baseline_plan` call with the Initializer.
The Phase 0 single-phase fallback is preserved via `fallback_single_phase_plan`.

### 5. Tools

```rust
// crates/seasoned-hand-core/src/tools/feature_mark_done.rs
pub struct FeatureMarkDoneTool;

#[async_trait::async_trait]
impl Tool for FeatureMarkDoneTool {
    fn name(&self) -> &'static str { "feature_mark_done" }
    fn schema(&self) -> Value { /* feature_id: string, required */ }
    async fn dispatch(&self, ctx: &ToolContext, args: Value) -> ToolOutput {
        let feature_id: String = parse_arg(&args, "feature_id")?;
        let mut fl = ctx.sandbox.read_workspace_file_json::<FeatureList>(
            &ctx.session_id, "/workspace/feature-list.json").await?;
        let f = fl.features.iter_mut().find(|f| f.id == feature_id)
            .ok_or_else(|| ToolError::not_found("feature_id"))?;
        f.status = FeatureStatus::Done;
        f.completed_at = Some(now_micros());
        let title = f.title.clone();
        let plan_phase_id = f.plan_phase_id;
        ctx.sandbox.write_workspace_file_json(&ctx.session_id,
            "/workspace/feature-list.json", &fl).await?;
        ctx.events.emit_misc(&ctx.session_id, "feature_done",
            json!({"feature_id": feature_id, "title": title})).await?;
        let active = ctx.plan_manager.current_phase_id(&ctx.session_id).await?;
        if Some(plan_phase_id) != active {
            ctx.events.emit_misc(&ctx.session_id, "feature_done_out_of_phase",
                json!({"feature_id": feature_id, "plan_phase_id": plan_phase_id, "active": active})).await?;
        }
        ToolOutput::ok(json!({"feature_id": feature_id, "status": "done"}))
    }
}
```

`progress_update` is simpler: append `<iso8601>  user           <line>` to
`/workspace/progress.txt`. (Six-space gutter to align with Initializer-
written lines.) Truncate to 200 chars + `…` if longer.

### 6. HTTP routes

```rust
// crates/seasoned-hand-server/src/routes/sessions_feature_list.rs
pub async fn get_feature_list(
    State(s): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<FeatureList>, ApiError> {
    let bytes = s.sandbox.read_workspace_file(&session_id, "/workspace/feature-list.json").await?;
    let fl: FeatureList = serde_json::from_slice(&bytes).map_err(ApiError::bad_state)?;
    Ok(Json(fl))
}

// sessions_progress.rs
pub async fn get_progress(
    State(s): State<AppState>,
    Path(session_id): Path<String>,
    Query(q): Query<ProgressQuery>,
) -> Result<String, ApiError> { /* read tail of last `lines` (default 200) */ }
```

Register both in the router builder. Add 404 mapping when the workspace file
is missing (sandbox returns `NotFound`).

### 7. Misc-kind documentation

In `crates/seasoned-hand-core/src/events/misc.rs` (or wherever Phase 0
documents `Misc.kind` values), append:

```
feature_done, feature_done_out_of_phase, init_planner_fallback
```

### 8. Tests

See acceptance criteria. Tests at `agent::init::tests` and
`tools::feature_mark_done::tests` / `tools::progress_update::tests`.
HTTP route tests in `crates/seasoned-hand-server/tests/feature_list.rs`.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core agent::init::
cargo test -p seasoned-hand-core tools::feature_mark_done
cargo test -p seasoned-hand-core tools::progress_update
cargo test -p seasoned-hand-server feature_list progress
./scripts/spec-check.sh
```

---

## Files changed

- `crates/seasoned-hand-core/src/agent/mod.rs` (modify — `pub mod init;`,
  call Initializer in `run()`)
- `crates/seasoned-hand-core/src/agent/init/mod.rs` (new)
- `crates/seasoned-hand-core/src/agent/init/feature_list.rs` (new)
- `crates/seasoned-hand-core/src/agent/init/feature_list.schema.json` (new)
- `crates/seasoned-hand-core/src/agent/init/progress.rs` (new)
- `crates/seasoned-hand-core/src/agent/init/tests.rs` (new)
- `crates/seasoned-hand-core/src/tools/feature_mark_done.rs` (new)
- `crates/seasoned-hand-core/src/tools/progress_update.rs` (new)
- `crates/seasoned-hand-core/src/tools/registry.rs` (modify — register both)
- `crates/seasoned-hand-core/src/sandbox/client.rs` (modify — add
  `read_workspace_file`, `write_workspace_file`, `read_workspace_file_json`,
  `write_workspace_file_json` helpers if missing)
- `crates/seasoned-hand-server/src/routes/sessions_feature_list.rs` (new)
- `crates/seasoned-hand-server/src/routes/sessions_progress.rs` (new)
- `crates/seasoned-hand-server/src/routes/mod.rs` (modify — register)
- `config/prompts/planner.system.txt` (new — planner-slot system prompt)
- `specs/phase-1/DEBT.md` (no expected entries; verify before commit)

---

## Spec references

- `/specs/phase-1/architecture.md` §2.1 (Initializer steps 1-5), §3.5
  (FeatureList schema), §3.6 (progress.txt format), §4.1 (HTTP routes),
  §4.3 (tool catalog additions), §12 q5/q6.
- `/specs/00-philosophy/PRINCIPLES.md` #3 (filesystem as memory).

---

## Commit message

```
feat(phase-1): story 1.4 - Initializer + feature-list.json + progress.txt

- agent::init::Initializer runs once per task pre-loop: planner-slot
  LLM call → structured plan → PlanManager::create → workspace bootstrap
  files (/workspace/feature-list.json + /workspace/progress.txt)
- Initializer doesn't count toward max_steps
- Two new LLM-visible tools: feature_mark_done(feature_id) flips status
  + emits feature_done Misc (plus feature_done_out_of_phase if not in
  active phase); progress_update(line) appends one timestamped line,
  truncates >200 chars
- Two new HTTP routes proxy workspace files:
  GET /v1/sessions/:id/feature-list, GET /v1/sessions/:id/progress
- 0-phase / malformed planner responses fall back to single-phase plan
  (mechanism reused from Phase 0) with Misc init_planner_fallback
- 7 unit + 2 axum route tests

refs: /specs/phase-1/stories/story-1.4.md
```

---

## Notes for next story (1.5)

`plan_create` is now called by the Initializer programmatically, **not** by
the LLM. Story 1.5 (tool-mask layer) will hide `plan_create` from the
LLM-facing tool schema while keeping it in the catalog (PRINCIPLE #2). The
LLM still sees `plan_advance`, `plan_update`, `feature_mark_done`,
`progress_update`.

The `feature-list.json` Verifier-read path is ready: story 1.10 (Verifier
fresh context construction) will read this file as one of its inputs.

Story 1.6 (Context Recitation) will tail `/workspace/progress.txt` every
10 iterations and inject it as a Misc event.

## Notes from execution

- Added `agent::init::Initializer` pre-loop bootstrap with planner parse/fallback,
  `PlanManager::create`, and workspace initialization for
  `feature-list.json` + `progress.txt`.
- Replaced baseline-plan seeding in `AgentRunner::run` with Initializer wiring.
- Added LLM-visible tools `feature_mark_done` and `progress_update`.
- Added HTTP routes:
  - `GET /v1/sessions/:id/feature-list`
  - `GET /v1/sessions/:id/progress`
- Added feature/progress tool tests and server route tests in
  `crates/seasoned-hand-server/tests/feature_list.rs`.

## Execution notes (post-Phase-1 consistency audit)

**Naming drift — workspace paths are passed without the `/workspace/`
prefix.** The story body calls out `/workspace/feature-list.json` and
`/workspace/progress.txt` as the on-disk paths. The implementation
passes the workspace-relative form
(`SandboxClient::write_workspace_file(session_id, "feature-list.json",
...)`). `SandboxClient::normalize_workspace_relative_path` joins
against `handle.workspace_host_path` so the host-fs result is correct
and identical to what the spec describes. The literal `/workspace/...`
in the spec body is the **logical** sandbox path; the convention
everywhere in the Phase 1 code is the workspace-relative form. The
Verifier context builder + the HTTP routes both follow this
convention, so the on-disk reality stays self-consistent.
