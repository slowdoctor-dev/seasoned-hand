# Story 1.2 — SandboxClient handle-cache rehydration (close DEBT #18)

> **Status**: ready
> **Estimated**: 1.5 hours
> **Dependencies**: none
> **Phase**: 1
> **Type**: backend
> **Reads first**: `/specs/phase-0/DEBT.md` #18 (what this closes),
> `/specs/phase-0/architecture.md` §"Sandbox Client" (Phase 0 lifecycle
> contract), `/specs/phase-1/architecture.md` §6 row "Sandbox handle cache"
> (pay-down statement).

---

## Goal

On control-plane startup, scan Docker for existing
`seasoned-hand-sandbox-*` containers and rehydrate the in-process
`SandboxClient` handle cache. After this story, a control-plane restart in
the middle of a running task does not orphan its sandbox container or leave
the cache out of sync with Docker reality. This unblocks the Initializer
(story 1.4), Checkpoint Manager (story 1.13), and the WS task-control story
(1.17), all of which assume the cache reflects ground truth.

## Acceptance criteria

- [ ] `SandboxClient::rehydrate_from_docker(&self) -> Result<RehydrateReport, SandboxError>`
      enumerates containers via bollard with filter `name = "seasoned-hand-sandbox-*"`
      and re-registers each container into the in-process handle cache keyed
      by `session_id` (parsed from the container name suffix).
- [ ] Rehydration is idempotent: calling it twice in a row yields the same
      `RehydrateReport` with `restored: 0` on the second call.
- [ ] Stale containers (where the corresponding `sessions` row is absent or
      its `state IN ('FINISHED','ERROR')`) are **not** re-registered; they
      are logged with `tracing::warn!(orphan_container = ...)` and left
      running (cleanup is Phase 0 DEBT #16's job).
- [ ] `RehydrateReport { restored: usize, orphans: usize, errors: Vec<String> }`
      is logged at INFO on server startup.
- [ ] Called once at server bootstrap, before the HTTP listener binds, and
      after the DB pool + Redis pool are established.
- [ ] Tests:
      - `rehydrate_with_no_containers_reports_zero` — unit test against a
        bollard mock or `#[ignore]` integration with a real Docker daemon
        that has no matching containers.
      - `rehydrate_with_two_containers_one_with_session_one_without`
        (integration, `#[ignore]` by default; gated behind
        `RUN_DOCKER_TESTS=1` env var like Phase 0 sandbox tests) — creates
        two containers manually, only one has a matching `sessions` row,
        asserts `restored == 1 && orphans == 1`.
      - `rehydrate_is_idempotent` — calling twice in succession against
        the same state yields identical reports.
- [ ] DEBT updates: Phase 0 DEBT #18 entry struck through with date + commit
      ref. No new entries in `specs/phase-1/DEBT.md`.

## Non-goals

- Cleanup of orphan containers (Phase 0 DEBT #16 — workspace TTL).
- Multi-process / multi-host coordination (Phase 5 — multi-user).
- Re-attaching to a sandbox whose container is paused: just register the
  handle; the WS task_resume path (story 1.17) handles unpause.

---

## Implementation steps

### 1. Cache type

In `crates/seasoned-hand-core/src/sandbox/client.rs` (or whichever Phase 0
module owns the cache), expose:

```rust
pub(crate) struct SandboxHandleCache {
    inner: tokio::sync::RwLock<HashMap<SessionId, SandboxHandle>>,
}
```

If Phase 0 stored handles in a different shape (e.g. `DashMap`), keep the
shape — this story does not refactor it.

### 2. Rehydrate entry point

```rust
impl SandboxClient {
    pub async fn rehydrate_from_docker(
        &self,
        sessions: Arc<DbPool>,
    ) -> Result<RehydrateReport, SandboxError> {
        let docker = self.docker.clone();
        let mut filters = HashMap::new();
        filters.insert("name", vec!["seasoned-hand-sandbox-"]);
        let containers = docker.list_containers(Some(ListContainersOptions {
            all: true,
            filters,
            ..Default::default()
        })).await?;

        let mut report = RehydrateReport::default();
        for c in containers {
            let name = c.names.as_ref().and_then(|n| n.first().cloned())
                .unwrap_or_default();
            let session_id = match name.strip_prefix("/seasoned-hand-sandbox-") {
                Some(s) => s.to_string(),
                None => continue,
            };
            let state = sessions_state(&sessions, &session_id).await?;
            match state.as_deref() {
                Some("IDLE") | Some("RUNNING") | Some("SUSPENDED") | Some("VERIFYING") => {
                    self.register_existing(&session_id, &c).await?;
                    report.restored += 1;
                }
                _ => {
                    tracing::warn!(orphan_container = %name, state = ?state, "orphan sandbox container");
                    report.orphans += 1;
                }
            }
        }
        tracing::info!(?report, "sandbox cache rehydrated");
        Ok(report)
    }
}
```

Notes:

- `VERIFYING` is the new Phase 1 state (story 1.10). It is fine to include
  it in this story — the migration lands later but the match arm pattern
  is value-checked, not type-checked, against the DB.
- `register_existing(session_id, container)` reconstructs a `SandboxHandle`
  from the container's existing port bindings and stores it in the cache.
  The internal port discovery already exists in Phase 0; this method is a
  small refactor that exposes the path without going through `create()`.

### 3. Server startup wiring

`crates/seasoned-hand-server/src/main.rs` (or `lib.rs`):

```rust
let sandbox = AppState::sandbox(&state).clone();
let sessions_pool = AppState::db_pool(&state).clone();
match sandbox.rehydrate_from_docker(sessions_pool).await {
    Ok(r) => tracing::info!(?r, "sandbox cache rehydrated"),
    Err(e) => tracing::error!(error = %e, "sandbox rehydration failed; continuing with empty cache"),
}
let listener = ...;
```

Rehydration failure is **non-fatal** — Docker may be unavailable in a test
harness. Server starts with an empty cache; new tasks succeed; existing
containers remain orphaned (logged).

### 4. Tests

Pure-unit test: feed a mock `bollard::Docker` (or a thin trait wrapper)
returning a synthetic container list; assert `RehydrateReport` matches.

Integration test (`#[ignore]` by default, gated by `RUN_DOCKER_TESTS=1`):

- Create `docker run -d --rm --name seasoned-hand-sandbox-test-rehydrate-${PID}`
  using `bollard` directly from the test setup.
- Insert a matching `sessions` row in a temp SQLite (or use the existing
  test fixture).
- Call `rehydrate_from_docker(...)`, assert `restored == 1`.
- Tear down the container in test fixture `drop`.

### 5. DEBT.md update

`specs/phase-0/DEBT.md` item #18: strike-through with date + commit ref.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core sandbox::cache::tests
cargo test -p seasoned-hand-core sandbox::cache::integration -- --ignored  # only if RUN_DOCKER_TESTS=1
./scripts/spec-check.sh
```

Boot the server (`cargo run -p seasoned-hand-server`) with one pre-existing
container and observe the startup log line containing
`sandbox cache rehydrated restored=1 orphans=0`.

---

## Files changed

- `crates/seasoned-hand-core/src/sandbox/client.rs` (modify — add
  `rehydrate_from_docker` + `register_existing`)
- `crates/seasoned-hand-core/src/sandbox/cache.rs` (modify — public read
  helper for tests if not already)
- `crates/seasoned-hand-server/src/main.rs` (modify — call rehydrate at
  startup, log report)
- `crates/seasoned-hand-core/src/sandbox/tests.rs` (modify — 3 new tests)
- `specs/phase-0/DEBT.md` (close #18)

---

## Spec references

- `/specs/phase-1/architecture.md` §6 ("Sandbox handle cache — Pay-down").
- `/specs/phase-0/DEBT.md` #18 (origin of the gap).

---

## Commit message

```
fix(phase-1): story 1.2 - SandboxClient handle-cache rehydration

- SandboxClient::rehydrate_from_docker enumerates
  seasoned-hand-sandbox-* containers via bollard at server startup,
  re-registers handles for sessions whose state ∈ {IDLE, RUNNING,
  SUSPENDED, VERIFYING}, logs orphans (sessions FINISHED/ERROR or
  missing), and is idempotent
- Rehydration failure is non-fatal: log + continue with empty cache
- 3 tests: unit (no containers / mocked list), idempotency, ignored
  Docker integration (RUN_DOCKER_TESTS=1)

Closes Phase 0 DEBT #18.

refs: /specs/phase-1/stories/story-1.2.md
```

---

## Notes for next story (1.3)

The cache now survives restarts. Story 1.3 (workspace bootstrap) adds
`git init` + identity + initial commit to the existing `SandboxClient::create`
path. Rehydration **does not** re-run `git init` — it assumes the workspace
was already initialised when the container was first created.
