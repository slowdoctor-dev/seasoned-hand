# Story 0.8 — AIO Sandbox bollard integration

> **Status**: done
> **Image pin**: `ghcr.io/agent-infra/sandbox:1.0.0.152` (verified against agent-infra/sandbox GitHub releases as of 2026-05-12)
> **Security**: requires `--security-opt seccomp=unconfined` for Chromium per upstream README
> **Estimated**: 3 hours
> **Dependencies**: story 0.3 (DB), 0.4 (events)
> **Phase**: 0
> **Type**: backend
> **Reads first**: `/specs/phase-0/architecture.md` §1 (Sandbox Client), §4.3 (Sandbox backend), §5.1 (bollard 0.17), §5.2 (AIO Sandbox image pinning), `/specs/01-architecture/decisions/ADR-004-aio-sandbox-per-session.md`

---

## Goal

Stand up the `seasoned-hand-core::sandbox` module: bollard 0.17 client that
creates, pauses, resumes, and destroys an AIO Sandbox container per session.
Workspace mount, host port allocation, and the container lifecycle API are
the deliverables here. Story 0.9 then wires this client into the tool
dispatcher's Sandbox backend.

Phase 0 acceptance for Story 0.8 is **lifecycle correctness, not tool
forwarding** — the HTTP-tool calls into the running sandbox are a story
0.9 concern.

## Acceptance criteria

- [ ] `seasoned-hand-core::sandbox` module exposing:
      - `SandboxClient::new(image: String, workspace_root: PathBuf) -> Result<Self, SandboxError>`
      - `SandboxClient::create(&self, session_id: &str) -> Result<SandboxHandle, SandboxError>`
      - `SandboxClient::pause(&self, session_id: &str) -> Result<(), SandboxError>`
      - `SandboxClient::resume(&self, session_id: &str) -> Result<(), SandboxError>`
      - `SandboxClient::destroy(&self, session_id: &str) -> Result<(), SandboxError>`
      - `SandboxClient::get(&self, session_id: &str) -> Option<SandboxHandle>`
- [ ] `SandboxHandle` carries: `container_id`, `session_id`,
      `api_url: String` (e.g. `http://127.0.0.1:<host_port>`),
      `novnc_url`, `ttyd_url`, `workspace_host_path`
- [ ] Container naming: `seasoned-hand-sandbox-<session_id>`
- [ ] Workspace mount: `{workspace_root}/{session_id}` →
      `/workspace` (rw), parent dirs auto-created
- [ ] Host port allocation: pass `0` to Docker, read back the assigned
      host port from `inspect`; never collide between sessions
- [ ] Container started with `extra_hosts: host.docker.internal:host-gateway`
      for parity with the Bifrost service
- [ ] AIO Sandbox image **pinned**: implementer verifies the image name
      (try `ghcr.io/agent-infra/sandbox` then `agentinfra/aio-sandbox`)
      and a stable tag; commits both to architecture.md §5.2 and the
      `AIO_SANDBOX_IMAGE` default in `.env.example`. Never `:latest`.
- [ ] `destroy` is idempotent: calling it twice in a row returns Ok
- [ ] `pause` on an already-paused container surfaces a clean
      `SandboxError::AlreadyPaused` rather than a generic bollard error
- [ ] In-process handle cache: `SandboxClient` holds
      `tokio::sync::RwLock<HashMap<String, SandboxHandle>>` so callers
      don't need to re-`inspect` between operations
- [ ] Unit tests with **no Docker pull**: exercise the URL-building, the
      name pattern, the error mapping, and the in-process cache
- [ ] One `#[ignore]`d live test (gated on Docker daemon) that actually
      pulls the pinned image, creates+inspects+destroys a real container.
      CI will run this with `cargo test -- --ignored` after Docker is up.
- [ ] `cargo clippy --all-targets -- -D warnings` passes
- [ ] `cargo fmt --check` passes
- [ ] `cargo test --workspace` passes (with ignored tests skipped)
- [ ] `./scripts/spec-check.sh` passes

## Non-goals

- Wiring sandbox into the Tool dispatcher (story 0.9)
- Sandbox HTTP-tool forwarding from the agent to the container's
  internal `/v1/...` (story 0.9 forwards via reqwest using the
  `SandboxHandle.api_url`)
- noVNC iframe / xterm frontend wiring (stories 0.24, 0.25)
- AIO Sandbox image rebuild / customization — use upstream as-is
- Cleanup job for orphan workspaces (Phase 1 hardening)
- Network egress allowlisting (Phase 1+, architecture §9 note)

---

## Implementation steps

### 1. Dependencies

`crates/seasoned-hand-core/Cargo.toml`:

```toml
bollard = "0.17"
# bollard 0.17 already brings tokio, serde; nothing new
```

### 2. Module skeleton — `sandbox/mod.rs`

```rust
//! Per-session AIO Sandbox container lifecycle.
//! refs: /specs/phase-0/architecture.md §1, §4.3, §5.2
//! refs: /specs/01-architecture/decisions/ADR-004-aio-sandbox-per-session.md

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bollard::Docker;
use bollard::container::{
    Config, CreateContainerOptions, RemoveContainerOptions, StartContainerOptions,
};
use bollard::secret::{HostConfig, PortBinding};
use thiserror::Error;
use tokio::sync::RwLock;

#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("docker connect error: {0}")]
    Docker(#[from] bollard::errors::Error),
    #[error("invalid path: {0}")]
    Path(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("sandbox not found for session {0}")]
    NotFound(String),
    #[error("sandbox already paused for session {0}")]
    AlreadyPaused(String),
    #[error("port {0} not exposed by container")]
    PortMissing(u16),
}

#[derive(Clone, Debug)]
pub struct SandboxHandle {
    pub session_id: String,
    pub container_id: String,
    pub api_url: String,        // e.g. http://127.0.0.1:<port>
    pub novnc_url: String,
    pub ttyd_url: String,
    pub workspace_host_path: PathBuf,
}

#[derive(Clone)]
pub struct SandboxClient {
    docker: Docker,
    image: String,
    workspace_root: PathBuf,
    handles: Arc<RwLock<HashMap<String, SandboxHandle>>>,
}

impl SandboxClient {
    pub fn new(image: impl Into<String>, workspace_root: impl Into<PathBuf>) -> Result<Self, SandboxError> {
        let docker = Docker::connect_with_local_defaults()?;
        Ok(Self {
            docker,
            image: image.into(),
            workspace_root: workspace_root.into(),
            handles: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub async fn get(&self, session_id: &str) -> Option<SandboxHandle> {
        self.handles.read().await.get(session_id).cloned()
    }

    pub async fn create(&self, session_id: &str) -> Result<SandboxHandle, SandboxError> {
        // 1. ensure workspace_root/session_id exists
        let workspace = self.workspace_root.join(session_id);
        std::fs::create_dir_all(&workspace)?;
        let workspace_abs = workspace.canonicalize()?;

        // 2. build container config: image, mounts, exposed ports, extra hosts
        // 3. create + start
        // 4. inspect to get assigned host ports
        // 5. build handle, cache, return
        todo!("see implementation for full body")
    }

    pub async fn pause(&self, session_id: &str) -> Result<(), SandboxError> { todo!() }
    pub async fn resume(&self, session_id: &str) -> Result<(), SandboxError> { todo!() }
    pub async fn destroy(&self, session_id: &str) -> Result<(), SandboxError> { todo!() }
}

pub fn container_name(session_id: &str) -> String {
    format!("seasoned-hand-sandbox-{session_id}")
}

#[cfg(test)]
mod tests;
```

### 3. `create` body

- Build `PortBinding` map for the three container ports (6080, 7681, 8080)
  with `host_port: None` so Docker assigns dynamic ones
- `HostConfig`: binds (workspace mount), port_bindings, extra_hosts
- Set `attach_stdin/out/err: false` (Phase 0 doesn't follow logs)
- Run `create_container` → `start_container` → `inspect_container`
- From inspect's `NetworkSettings.Ports`, read the assigned host port for
  each container port; build `api_url`, `novnc_url`, `ttyd_url`
- Insert into the handle cache

### 4. `destroy` body

- Try `remove_container(container_name, force: true)`
- If error is "no such container", treat as success (idempotent)
- Remove from the in-process handle cache (whether or not removal succeeded
  — the cache should be consistent with "is this container still ours")

### 5. `pause` / `resume`

- `docker.pause_container(name, None)` / `unpause_container`
- Detect AlreadyPaused via inspect first (state.Paused) for a clean error

### 6. Tests

**Unit (no Docker pull):**
- `container_name` formatting
- `SandboxHandle` URL fields shape
- `SandboxError` mapping from `bollard::errors::Error`

**Live (`#[ignore]`, gated on local Docker):**
- `create_then_inspect_then_destroy` — pulls the pinned image once, runs
  the lifecycle. Asserts: workspace dir created, container appears in
  `docker ps`, ports are reachable on the host (TCP connect, not HTTP),
  destroy removes the container.
- `destroy_is_idempotent`

If the image pull fails (network), tests skip with a clear message rather
than fail.

### 7. Architecture + env updates

- `specs/phase-0/architecture.md` §5.2: replace `<verify-image-name>:<pin-tag>`
  row with the chosen tag.
- `.env.example`: replace `AIO_SANDBOX_IMAGE=ghcr.io/agent-infra/sandbox:latest`
  with the pinned tag.
- Add `data/workspaces/` to `.gitignore` if missing.

---

## Verification

```bash
docker info >/dev/null 2>&1 && DOCKER_OK=1 || DOCKER_OK=0
source $HOME/.cargo/env
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
./scripts/spec-check.sh

# Live sandbox lifecycle (gated):
if [ "$DOCKER_OK" = 1 ]; then
  cargo test --workspace -- --ignored sandbox::
fi
```

---

## Files changed

- `crates/seasoned-hand-core/Cargo.toml` (add `bollard = "0.17"`)
- `crates/seasoned-hand-core/src/lib.rs` (`pub mod sandbox`)
- `crates/seasoned-hand-core/src/sandbox/mod.rs` (new)
- `crates/seasoned-hand-core/src/sandbox/tests.rs` (new)
- `specs/phase-0/architecture.md` (pin AIO Sandbox image)
- `.env.example` (pin AIO_SANDBOX_IMAGE)
- `.gitignore` (`data/workspaces/` if missing)
- `specs/phase-0/DEBT.md` (any new debt incurred — likely a port-allocation
  note and an orphan-workspace-cleanup note)

---

## Spec references

- `/specs/phase-0/architecture.md` §1, §4.3, §5.2
- `/specs/01-architecture/decisions/ADR-004-aio-sandbox-per-session.md`
- `/specs/phase-0/requirements.md` §3.7

---

## Commit message

```
feat(phase-0): story 0.8 - AIO Sandbox bollard client (lifecycle)

- seasoned-hand-core::sandbox module with SandboxClient over
  bollard 0.17
- Per-session container: name seasoned-hand-sandbox-<session_id>,
  workspace mount {root}/{session_id} -> /workspace, dynamic host
  port assignment via "0" + inspect-back
- SandboxClient API: new / create / pause / resume / destroy / get
- SandboxHandle exposes api_url, novnc_url, ttyd_url,
  workspace_host_path; cached in-process (RwLock<HashMap>) so
  callers don't re-inspect between ops
- destroy is idempotent; pause/resume map AlreadyPaused cleanly
- Image pinned (see architecture §5.2 + .env.example)
- N unit tests (no Docker pull) + 1 #[ignore]'d live lifecycle test
- cargo clippy / fmt / test / spec-check all pass

refs: /specs/phase-0/stories/story-0.8.md
```

---

## Notes for next story (0.9)

- `SandboxHandle.api_url` is the base URL for the 22 sandbox-backed
  tools (5 file + 5 shell + 12 browser). Story 0.9's dispatcher passes
  this URL into each tool's invoke.
- `ToolContext` gains a `sandbox: Arc<SandboxClient>` field.
- For tools that need a Sandbox but the session's container isn't up
  yet: dispatcher returns a clean `ToolError::Backend("sandbox not
  ready")` rather than auto-creating (auto-create lives at session
  creation time, not tool dispatch).
