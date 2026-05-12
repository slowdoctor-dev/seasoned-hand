# Story 1.3 — Sandbox workspace bootstrap (`git init` + identity)

> **Status**: ready
> **Estimated**: 2 hours
> **Dependencies**: 1.2 (handle-cache rehydration in place; rehydrate path
> assumes `git init` already happened on first create)
> **Phase**: 1
> **Type**: backend
> **Reads first**: `/specs/phase-1/architecture.md` §2.6 (Git Checkpoint),
> §5.1 (no `git2`; we use sandbox shell), §12 question 5 (git identity),
> `/specs/phase-0/architecture.md` §"Sandbox Client" (current `create`
> contract).

---

## Goal

Extend `SandboxClient::create` so every new sandbox workspace is a git
working tree from session start, with a known identity and an initial empty
commit. The LLM is never given git tools. This story does *not* yet
checkpoint on phase advances (story 1.13) — it only establishes the
working tree so Checkpoint Manager can `git commit -m "phase N: ..."`
against a sane HEAD.

## Acceptance criteria

- [ ] `SandboxClient::create(...)` extended with a post-container-start
      bootstrap that runs, in order, against the new container's
      `/v1/shell` endpoint:
      1. `git init -q /workspace`
      2. `cd /workspace && git config user.email "seasoned-hand@local"`
      3. `cd /workspace && git config user.name  "Seasoned Hand"`
      4. `cd /workspace && git commit --allow-empty -q -m "init"`
- [ ] Bootstrap failure (any of the four shell calls returning non-zero)
      surfaces as `SandboxError::WorkspaceBootstrap(stderr)`. `create()`
      tears down the container before returning the error — no half-
      bootstrapped sandbox leaks.
- [ ] Smoke check at end of bootstrap: `git --version` runs and reports a
      version. If `git` is missing in the sandbox image, the error message
      is `"sandbox image missing git binary"`.
- [ ] Constants live in `crates/seasoned-hand-core/src/sandbox/identity.rs`:
      `GIT_USER_EMAIL`, `GIT_USER_NAME`. Comments cite phase-1/DEBT.md #11
      (per-user identity is Phase 5).
- [ ] Tests:
      - `bootstrap_command_strings_are_correct` — unit test on the helper
        that builds the four shell commands, asserts exact string values.
      - `bootstrap_returns_workspace_bootstrap_error_on_missing_git`
        (integration, `#[ignore]` gated by `RUN_DOCKER_TESTS=1`) — runs
        `create()` against a stripped image where `git` is missing;
        asserts the documented error message + container cleanup.
      - `bootstrap_happy_path` (integration, `#[ignore]`) — creates a real
        sandbox, asserts `cd /workspace && git log --oneline | wc -l` ≥ 1.
- [ ] No new entries in `specs/phase-1/DEBT.md` (the existing #11
      hardcoded-identity item already covers this).

## Non-goals

- Per-user git identity (Phase 5; phase-1/DEBT.md #11 tracks).
- Checkpoint on phase advance — story 1.13.
- Exposing any git tool to the LLM — explicitly forbidden by architecture
  §2.6 ("LLM **never sees** git").
- Repointing rehydrated sandboxes to re-init git — workspace is persistent;
  rehydration assumes the workspace is already initialised.
- Writing `feature-list.json` / `progress.txt` — that's story 1.4
  (Initializer).

---

## Implementation steps

### 1. Identity module

```rust
// crates/seasoned-hand-core/src/sandbox/identity.rs
// Phase 1 hardcoded identity. Per-user attribution lands in Phase 5
// (phase-1/DEBT.md #11). Do not change without an ADR update.
pub const GIT_USER_EMAIL: &str = "seasoned-hand@local";
pub const GIT_USER_NAME: &str  = "Seasoned Hand";
```

### 2. Bootstrap helper

```rust
// crates/seasoned-hand-core/src/sandbox/bootstrap.rs
pub(crate) fn workspace_bootstrap_commands() -> [&'static str; 5] {
    [
        "git init -q /workspace",
        "git -C /workspace config user.email \"seasoned-hand@local\"",
        "git -C /workspace config user.name \"Seasoned Hand\"",
        "git -C /workspace commit --allow-empty -q -m \"init\"",
        "git --version",
    ]
}
```

(Strings are constant for clarity; the unit test asserts them.)

### 3. Wire into `SandboxClient::create`

```rust
impl SandboxClient {
    pub async fn create(&self, ...) -> Result<SandboxHandle, SandboxError> {
        let handle = self.start_container(...).await?;
        for cmd in workspace_bootstrap_commands() {
            let out = self.shell_exec(&handle, cmd).await?;
            if out.exit_code != 0 {
                // Tear down before bubbling the error
                let _ = self.destroy(&handle).await;
                let msg = if cmd == "git --version" && out.stderr.contains("not found") {
                    "sandbox image missing git binary".to_string()
                } else {
                    format!("workspace bootstrap failed: cmd={cmd:?}, stderr={}", out.stderr)
                };
                return Err(SandboxError::WorkspaceBootstrap(msg));
            }
        }
        Ok(handle)
    }
}
```

`shell_exec` already exists from Phase 0 (story 0.8 / 0.9). If it doesn't
return exit code, extend it minimally — Phase 0 wired the AIO Sandbox
`/v1/shell` endpoint which returns `{exit_code, stdout, stderr}`.

### 4. Error variant

```rust
#[derive(thiserror::Error, Debug)]
pub enum SandboxError {
    // ... existing variants ...
    #[error("workspace bootstrap: {0}")]
    WorkspaceBootstrap(String),
}
```

### 5. Rehydration interaction

`SandboxClient::rehydrate_from_docker` (story 1.2) does **not** call
`workspace_bootstrap_commands` — existing containers already have an
initialised workspace. Add a one-line comment in `rehydrate_from_docker`
near the `register_existing` call documenting this assumption.

### 6. Tests

Pure-unit test asserts the five command strings exactly (catches typos in
quoting / `-C` flag / etc. via golden-string comparison).

Integration test (`#[ignore]`):

- Pull the pinned AIO Sandbox image.
- `SandboxClient::create("test-session-bootstrap-${PID}")`.
- Via `shell_exec`: `git -C /workspace rev-parse HEAD` returns a SHA.
- Via `shell_exec`: `git -C /workspace config user.email` returns
  `seasoned-hand@local`.
- Tear down in test fixture.

---

## Verification

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test -p seasoned-hand-core sandbox::bootstrap::tests
cargo test -p seasoned-hand-core sandbox::bootstrap::integration -- --ignored  # if RUN_DOCKER_TESTS=1
./scripts/spec-check.sh
```

Manual smoke: start the server, create a session via the existing Phase 0
WS `task_create` flow, then `docker exec seasoned-hand-sandbox-<id> sh -c
'cd /workspace && git log --oneline'`. One commit (`init`) should be visible.

---

## Files changed

- `crates/seasoned-hand-core/src/sandbox/identity.rs` (new)
- `crates/seasoned-hand-core/src/sandbox/bootstrap.rs` (new)
- `crates/seasoned-hand-core/src/sandbox/client.rs` (modify — call bootstrap
  in `create`)
- `crates/seasoned-hand-core/src/sandbox/error.rs` (modify — `WorkspaceBootstrap`
  variant)
- `crates/seasoned-hand-core/src/sandbox/mod.rs` (modify — `pub mod bootstrap; pub mod identity;`)
- `crates/seasoned-hand-core/src/sandbox/tests.rs` (modify — 1 unit + 2 ignored integration tests)

---

## Spec references

- `/specs/phase-1/architecture.md` §2.6 (workspace as git tree from
  Initializer step 4), §5.1 (no `git2`; sandbox shell only), §5.2 (AIO
  Sandbox image `1.0.0.152` already ships `git`), §12 question 5
  (identity decision).

---

## Commit message

```
feat(phase-1): story 1.3 - sandbox workspace git bootstrap

- SandboxClient::create runs git init + identity config + initial empty
  commit + `git --version` smoke check against the new container's
  /v1/shell on every create
- Bootstrap failure tears down the container and surfaces
  SandboxError::WorkspaceBootstrap(stderr); missing-git case has a
  dedicated message
- Identity hardcoded to "seasoned-hand@local" / "Seasoned Hand"; Phase 5
  multi-user replaces this (phase-1/DEBT.md #11 unchanged)
- Rehydration path documented to NOT re-init: workspace persists with
  container
- 1 unit + 2 ignored integration tests

refs: /specs/phase-1/stories/story-1.3.md
```

---

## Notes for next story (1.4)

The sandbox workspace is now a git working tree at creation time. Story 1.4
(Initializer) layers `feature-list.json` + `progress.txt` writes on top of
this bootstrap and runs *after* the bootstrap finishes. Story 1.13
(Checkpoint Manager) will commit phase advances on top of the `init` commit
this story leaves on HEAD.
