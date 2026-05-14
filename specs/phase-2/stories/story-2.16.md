# Story 2.16 — Durable pause/resume + event-stream replay rebuild

> **Status**: complete (2026-05-14)
> **Estimated**: 3 hours
> **Dependencies**: 2.2
> **Phase**: 2
> **Type**: backend
> **Reads first**: `/specs/phase-2/architecture.md` §2.6 (durability), §8 ("Sandbox container GC'd while paused")

---

## Goal

Make `task_pause` durable across container garbage collection. When
`task_resume` finds no sandbox container (because it was orphan-cleaned
during the pause window), rebuild a fresh sandbox + replay the event
stream into a coherent Plan / feature-list / progress / cost state.

> **Risk flag** (per PM proposal): 3h budget. The replay test setup is
> non-trivial. Split into 2.16a (durable pause) + 2.16b (replay rebuild)
> if it exceeds 3.5h.

## Acceptance criteria

- [ ] WS `task_pause` cmd gains `durable: bool` field (defaults `true`).
      Non-durable pause is the Phase 1 1.17 behavior unchanged.
- [ ] Durable pause flow:
      1. Existing soft pause (docker pause + DB state → SUSPENDED)
      2. Persist Misc `task_paused_durable { sandbox_id,
         workspace_path, event_cursor, paused_at }`
      3. Existing `Misc{kind:"task_paused"}` event still emits
- [ ] `task_resume` flow:
      1. Look up the Task's most recent Session
      2. Check sandbox container exists (existing SandboxClient::get)
         AND inspect docker container status
      3. **Container alive**: existing unpause path (Phase 1 1.17)
      4. **Container gone**: rebuild path:
         a. Emit `Misc{kind:"task_resume_rebuild_required"}`
         b. Create a fresh sandbox via `SandboxClient::create`
            (renderer install runs again per story 2.6)
         c. Replay events into Plan Manager:
            - Find latest Plan event (kind="Plan", op="create" or
              "update" with greatest event_id)
            - Re-load Plan into PlanManager via existing
              `PlanManager::create` (idempotent on session_id)
         d. Replay events into feature-list.json:
            - Find latest `feature_done` Misc events
            - Reconstruct feature-list state, write to
              `/workspace/feature-list.json`
         e. Replay events into progress.txt:
            - Aggregate `progress_update` + `progress_recite` Misc
              events in event-id order
            - Write to `/workspace/progress.txt`
         f. Set CostClient baseline to most recent cost snapshot
            event
         g. Create new Session row (linked to same task_id) — DON'T
            reuse the old session_id (each container == one session)
         h. Set Task status → RUNNING; resume runner against new
            session
- [ ] Replay failure handling:
      - Any of the replay steps return error → Task status →
        `failed{reason:"replay_failed", details:...}`
      - No silent recovery; Misc `task_resume_rebuild_failed`
        carries the failing step name
- [ ] Unit tests:
      - `durable_pause_emits_task_paused_durable_misc`
      - `resume_with_live_container_uses_existing_path` (regression)
      - `resume_with_dead_container_rebuilds`
      - `replay_reconstructs_plan_from_events`
      - `replay_reconstructs_feature_list_from_misc`
      - `replay_failure_transitions_task_to_failed`

## Non-goals

- Workspace TTL cleanup (story 2.17). 2.16 only HANDLES container-gone;
  the cron that ACTUALLY garbage-collects sandboxes is story 2.17.
- New WS protocol — only the existing `task_pause` cmd grows the
  `durable` field (additive, backward compatible).

---

## Implementation steps

### 1. WS cmd shape

`crates/seasoned-hand-server/src/ws.rs` — the existing `task_pause`
cmd parser accepts `durable: Option<bool>`, default `Some(true)`.

### 2. Pause path

`crates/seasoned-hand-server/src/ws.rs`'s `task_pause` handler emits
the additional `task_paused_durable` Misc when `durable == true`.

### 3. Resume path

`crates/seasoned-hand-server/src/lib.rs` (or a new module
`crates/seasoned-hand-core/src/task/resume.rs`):

```rust
async fn resume_task(
    task_id: &str,
    deps: ResumeDeps,
) -> Result<(), ResumeError> {
    let session = deps.task_store.latest_session(task_id).await?;
    match deps.sandbox.get(&session.id).await {
        Some(_) => existing_unpause(...).await,
        None => rebuild_and_replay(task_id, &session, deps).await,
    }
}
```

### 4. Replay helpers

```
crates/seasoned-hand-core/src/task/replay.rs
  - replay_plan
  - replay_feature_list
  - replay_progress
  - replay_cost_baseline
```

Each reads from `SqliteEventStore` filtered by `session_id`.

### 5. Tests

Integration-test the rebuild path by:
1. Create task + session
2. Plant a known Plan event + feature_done event + progress_update event
3. Simulate sandbox-gone (mock SandboxClient returns None for `get`)
4. Call `resume_task`
5. Assert new session row created + workspace files reconstructed
   (the test SandboxClient with `insert_handle_for_test` from Phase 1)

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core task::resume task::replay
cargo test -p seasoned-hand-server ws::
./scripts/spec-check.sh
```

---

## Files changed

- `crates/seasoned-hand-core/src/task/mod.rs` (new)
- `crates/seasoned-hand-core/src/task/resume.rs` (new)
- `crates/seasoned-hand-core/src/task/replay.rs` (new)
- `crates/seasoned-hand-core/src/task/tests.rs` (new)
- `crates/seasoned-hand-core/src/lib.rs` (modify — `pub mod task;`)
- `crates/seasoned-hand-server/src/ws.rs` (modify — `durable` field on
  `task_pause`)
- `crates/seasoned-hand-server/src/lib.rs` (modify — resume route uses
  `resume_task`)

---

## Spec references

- `/specs/phase-2/architecture.md` §2.6, §8

---

## Commit message

```
feat(phase-2): story 2.16 - Durable pause/resume + event-stream replay rebuild

- WS task_pause gains durable: bool (defaults true). Durable pause
  emits task_paused_durable Misc with sandbox_id + workspace_path +
  event_cursor metadata.
- task_resume detects container-gone via SandboxClient::get; falls
  back to rebuild + replay path:
  - fresh sandbox (renderer install repeats per story 2.6)
  - replay Plan from latest Plan event into PlanManager
  - replay feature-list.json from feature_done Misc events
  - replay progress.txt from progress_update / progress_recite
  - reset cost baseline from latest snapshot
  - create new session linked to same task_id; resume runner
- Replay failures transition task to failed{reason:"replay_failed"};
  task_resume_rebuild_failed Misc carries the step.
- 6 unit tests including integration-style rebuild path.

refs: /specs/phase-2/stories/story-2.16.md
```

---

## Notes for next story (2.17)

Resume-from-replay works. 2.17 pays down Phase 0 DEBT #16: a workspace
TTL cleanup cron that honors Task status (never GC active tasks, 7d
for paused, 30d for completed, 7d for failed/cancelled).
