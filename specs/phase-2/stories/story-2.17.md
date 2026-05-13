# Story 2.17 — Workspace TTL + cleanup cron (Phase 0 DEBT #16)

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 2.2
> **Phase**: 2
> **Type**: backend
> **Reads first**: `/specs/phase-2/architecture.md` §6 "Sandbox lifecycle" + Phase 0 DEBT #16

---

## Goal

Pay down Phase 0 DEBT #16. Sandbox containers + workspace directories
accumulate without bound today. Land a cleanup cron that honors Task
status — never GC active tasks, configurable TTLs per terminal state.

## Acceptance criteria

- [ ] New module `seasoned-hand-core::task::ttl`. Single Tokio task
      spawned at boot.
- [ ] Cron interval: 1 hour default (env
      `SANDBOX_CLEANUP_INTERVAL_SEC=3600`).
- [ ] Per-status TTLs (env-configurable, defaults from architecture):
      - `running` / `paused` (durable): **never GC**
      - `completed`: 30 days (`SANDBOX_TTL_COMPLETED_DAYS=30`)
      - `failed` / `cancelled`: 7 days
        (`SANDBOX_TTL_FAILED_CANCELLED_DAYS=7`)
      - `drafted` / `briefed`: 1 day (cleanup leftover untouched
        brief drafts) (`SANDBOX_TTL_DRAFT_DAYS=1`)
- [ ] Each cron cycle:
      1. Query `tasks` for candidates: `status IN (completed, failed,
         cancelled, drafted, briefed)` AND `updated_at < now - TTL[status]`
      2. For each candidate's most recent Session:
         a. Stop + remove the docker container (existing
            `bollard` API) if present
         b. Recursively delete `workspace_host_path`
         c. Persist Misc `sandbox_cleaned { session_id, task_id,
            reason: "ttl_<status>" }` event
      3. Update `tasks.updated_at` to `now` so the next cron tick
         doesn't re-process the same row
- [ ] Active tasks (`running`, `paused`) are NEVER touched, even if
      `updated_at` is old.
- [ ] Failures in cleanup (Docker error, fs delete failure) get
      logged but don't crash the cron; emit
      `sandbox_cleanup_failed { session_id, error }` Misc and move on.
- [ ] Operator can trigger a manual cleanup cycle via
      `POST /v1/admin/sandbox/cleanup` (admin-token-gated, like
      Phase 1 1.13b rollback).
- [ ] Unit tests:
      - `ttl_cleans_completed_task_after_30d`
      - `ttl_skips_running_task`
      - `ttl_skips_paused_task_for_durable_pause`
      - `ttl_cleans_failed_task_after_7d`
      - `ttl_handles_missing_container_gracefully`
      - `admin_manual_cleanup_route_runs_one_cycle`

## Non-goals

- TTL-driven event-row pruning (Phase 5 — event stream is
  source-of-truth for provenance).
- Cleanup of deliverable files (Phase 5 — deliverables persist
  separately from session workspaces).
- Cleanup of provenance file-refs (Phase 5 — same reason).

---

## Implementation steps

### 1. Module

```
crates/seasoned-hand-core/src/task/ttl.rs
```

```rust
pub struct WorkspaceTtlCron {
    deps: TtlDeps,
    config: TtlConfig,
}

impl WorkspaceTtlCron {
    pub async fn run(&self, shutdown: CancellationToken) -> Result<(), TtlError> {
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return Ok(()),
                _ = tokio::time::sleep(self.config.interval) => self.cleanup_cycle().await,
            }
        }
    }
    async fn cleanup_cycle(&self) { ... }
}
```

### 2. AppState wiring

`AppState::new` constructs the cron, `main.rs` spawns it under a
shutdown token (mirror Phase 1's checkpoint manager spawn pattern).

### 3. Admin route

`POST /v1/admin/sandbox/cleanup` — same auth guards as Phase 1 1.13b's
admin rollback (loopback + token).

### 4. Tests

In-memory DB fixture with seeded tasks at various
`(status, updated_at)` combinations + mocked SandboxClient. Assert
the cron deletes the right ones and leaves the right ones alone.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core task::ttl
cargo test -p seasoned-hand-server --test admin_sandbox_cleanup
./scripts/spec-check.sh
```

---

## Files changed

- `crates/seasoned-hand-core/src/task/ttl.rs` (new)
- `crates/seasoned-hand-core/src/task/mod.rs` (modify)
- `crates/seasoned-hand-server/src/lib.rs` (modify — admin route +
  AppState construction)
- `crates/seasoned-hand-server/src/main.rs` (modify — spawn cron)
- `crates/seasoned-hand-server/tests/admin_sandbox_cleanup.rs` (new)
- `specs/phase-0/DEBT.md` (modify — strike-through #16 with this
  commit's SHA after merge)

---

## Spec references

- `/specs/phase-0/DEBT.md` #16 (workspace cleanup)
- `/specs/phase-2/architecture.md` §6 ("Sandbox lifecycle")

---

## Commit message

```
fix(phase-2): story 2.17 - Workspace TTL + cleanup cron (Phase 0 DEBT #16 close)

- WorkspaceTtlCron spawns at boot. Cycle interval 1h default. Per-Task-
  status TTLs (env-configurable):
  - running / durable-paused: never GC
  - completed: 30d
  - failed / cancelled: 7d
  - drafted / briefed: 1d
- Each cycle: query candidates by (status, updated_at), stop +
  remove docker container, rm -rf workspace dir, emit sandbox_cleaned
  Misc. Failures log + emit sandbox_cleanup_failed; don't crash cron.
- POST /v1/admin/sandbox/cleanup for manual trigger (admin-token-
  gated, loopback-only — Phase 1 1.13b pattern).
- 6 unit + 1 integration test.

closes: Phase 0 DEBT #16

refs: /specs/phase-2/stories/story-2.17.md
```

---

## Notes for next story (2.18)

Phase 0 DEBT #16 closes. 2.18 closes Phase 1 DEBT #15 (Verifier
XREADGROUP).
