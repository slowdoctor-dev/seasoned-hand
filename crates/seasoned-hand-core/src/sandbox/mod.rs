//! Per-session AIO Sandbox container lifecycle via bollard.
//! refs: /specs/phase-0/architecture.md §1, §4.3, §5.2
//! refs: /specs/01-architecture/decisions/ADR-004-aio-sandbox-per-session.md

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use bollard::Docker;
use bollard::container::{
    Config, CreateContainerOptions, ListContainersOptions, RemoveContainerOptions,
    StartContainerOptions,
};
use bollard::secret::{HostConfig, PortBinding};
use thiserror::Error;
use tokio::sync::RwLock;

use crate::db::DbPool;

pub mod bootstrap;
pub mod cache;
pub mod identity;
pub use cache::RehydrateReport;

/// AIO Sandbox internal ports (per upstream README).
pub const PORT_API: u16 = 8080;
pub const PORT_NOVNC: u16 = 6080;
pub const PORT_TTYD: u16 = 7681;

#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("docker error: {0}")]
    Docker(#[from] bollard::errors::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("db error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("sandbox not found for session {0}")]
    NotFound(String),
    #[error("sandbox already paused for session {0}")]
    AlreadyPaused(String),
    #[error("sandbox not paused for session {0}")]
    NotPaused(String),
    #[error("port {0} not exposed by container")]
    PortMissing(u16),
    #[error("invalid workspace path: {0}")]
    InvalidWorkspace(String),
    #[error("workspace bootstrap: {0}")]
    WorkspaceBootstrap(String),
    #[error("sandbox http: {0}")]
    Http(String),
    #[error("sandbox http status {status} from {path}")]
    HttpStatus { status: u16, path: String },
}

#[derive(Clone, Debug)]
pub struct SandboxHandle {
    pub session_id: String,
    pub container_id: String,
    pub api_url: String,
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
    /// Construct a client connected to the local Docker daemon.
    pub fn new(
        image: impl Into<String>,
        workspace_root: impl Into<PathBuf>,
    ) -> Result<Self, SandboxError> {
        let docker = Docker::connect_with_local_defaults()?;
        Ok(Self {
            docker,
            image: image.into(),
            workspace_root: workspace_root.into(),
            handles: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub fn image(&self) -> &str {
        &self.image
    }

    pub fn workspace_root(&self) -> &PathBuf {
        &self.workspace_root
    }

    pub async fn get(&self, session_id: &str) -> Option<SandboxHandle> {
        self.handles.read().await.get(session_id).cloned()
    }

    pub async fn read_workspace_file(
        &self,
        session_id: &str,
        relative_path: &str,
    ) -> Result<Vec<u8>, SandboxError> {
        let handle = self
            .get(session_id)
            .await
            .ok_or_else(|| SandboxError::NotFound(session_id.to_string()))?;
        let joined = handle
            .workspace_host_path
            .join(normalize_workspace_relative_path(relative_path)?);
        // SEC-IT4-M2: untrusted code in the sandbox can plant a symlink in
        // the bind-mounted workspace (e.g. `ln -s /etc/passwd leak`). The
        // `..`/null-byte rejector only inspects the request path, not on-disk
        // symlinks, so a host-side read here would follow the link and leak
        // an arbitrary host file. Resolve the real path and require it to
        // stay inside the workspace root.
        let path = canonical_within_workspace(&handle.workspace_host_path, &joined).await?;
        Ok(tokio::fs::read(path).await?)
    }

    pub async fn write_workspace_file(
        &self,
        session_id: &str,
        relative_path: &str,
        contents: &[u8],
    ) -> Result<(), SandboxError> {
        let handle = self
            .get(session_id)
            .await
            .ok_or_else(|| SandboxError::NotFound(session_id.to_string()))?;
        let path = handle
            .workspace_host_path
            .join(normalize_workspace_relative_path(relative_path)?);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        // SEC-IT4-M2: refuse to write THROUGH a symlink. A planted symlink at
        // the target (or a parent component) would turn a workspace write
        // into an arbitrary host-file write, escaping the bind mount.
        reject_workspace_write_escape(&handle.workspace_host_path, &path).await?;
        tokio::fs::write(path, contents).await?;
        Ok(())
    }

    pub async fn read_workspace_file_json<T: serde::de::DeserializeOwned>(
        &self,
        session_id: &str,
        relative_path: &str,
    ) -> Result<T, SandboxError> {
        let bytes = self.read_workspace_file(session_id, relative_path).await?;
        serde_json::from_slice(&bytes)
            .map_err(|e| SandboxError::WorkspaceBootstrap(format!("invalid json: {e}")))
    }

    pub async fn write_workspace_file_json<T: serde::Serialize>(
        &self,
        session_id: &str,
        relative_path: &str,
        value: &T,
    ) -> Result<(), SandboxError> {
        let bytes = serde_json::to_vec_pretty(value)
            .map_err(|e| SandboxError::WorkspaceBootstrap(format!("serialize json: {e}")))?;
        self.write_workspace_file(session_id, relative_path, &bytes)
            .await
    }

    /// Canonical accessor for the sandbox `browser_view` payload — the
    /// `{browser_info, elements}` JSON that the Phase 0 `browser_view`
    /// tool surfaces. Both that tool and the Phase 1
    /// `PostBrowserActionHook` (story 1.16, Track B) call this so there
    /// is exactly one HTTP path per logical operation.
    ///
    /// refs: /specs/phase-1/stories/story-1.16.md
    pub async fn browser_view(&self, session_id: &str) -> Result<serde_json::Value, SandboxError> {
        let handle = self
            .get(session_id)
            .await
            .ok_or_else(|| SandboxError::NotFound(session_id.to_string()))?;
        let client = reqwest::Client::new();
        let info = sandbox_get_json(&client, &handle.api_url, "/v1/browser/info").await?;
        let elements =
            sandbox_get_json(&client, &handle.api_url, "/v1/browser/page/elements").await?;
        Ok(serde_json::json!({
            "browser_info": info,
            "elements": elements,
        }))
    }

    /// New in Phase 1 (story 1.16, Track C): capture a PNG screenshot
    /// of the sandbox browser viewport. Returns raw PNG bytes.
    ///
    /// refs: /specs/phase-1/stories/story-1.16.md
    pub async fn browser_screenshot(&self, session_id: &str) -> Result<Vec<u8>, SandboxError> {
        let handle = self
            .get(session_id)
            .await
            .ok_or_else(|| SandboxError::NotFound(session_id.to_string()))?;
        let url = format!("{}/v1/browser/screenshot", handle.api_url);
        let client = reqwest::Client::new();
        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| SandboxError::Http(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(SandboxError::HttpStatus {
                status: status.as_u16(),
                path: "/v1/browser/screenshot".into(),
            });
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| SandboxError::Http(e.to_string()))?;
        Ok(bytes.to_vec())
    }

    pub async fn create(&self, session_id: &str) -> Result<SandboxHandle, SandboxError> {
        require_safe_session_id(session_id)?;
        let workspace = self.workspace_root.join(session_id);
        std::fs::create_dir_all(&workspace)?;
        let workspace_abs = workspace.canonicalize()?;
        let workspace_str = workspace_abs
            .to_str()
            .ok_or_else(|| SandboxError::InvalidWorkspace(workspace_abs.display().to_string()))?
            .to_string();

        let name = container_name(session_id);

        // Dynamic host-port allocation: empty host_port string asks Docker
        // for an ephemeral port.
        let mut port_bindings: HashMap<String, Option<Vec<PortBinding>>> = HashMap::new();
        for p in [PORT_API, PORT_NOVNC, PORT_TTYD] {
            port_bindings.insert(
                format!("{p}/tcp"),
                Some(vec![PortBinding {
                    host_ip: Some("127.0.0.1".into()),
                    host_port: Some(String::new()),
                }]),
            );
        }

        let host_config = HostConfig {
            binds: Some(vec![format!("{workspace_str}:/workspace:rw")]),
            port_bindings: Some(port_bindings),
            extra_hosts: Some(vec!["host.docker.internal:host-gateway".into()]),
            // Browser inside the sandbox needs unconfined seccomp (upstream README).
            security_opt: Some(vec!["seccomp=unconfined".into()]),
            ..Default::default()
        };

        let mut exposed_ports: HashMap<String, HashMap<(), ()>> = HashMap::new();
        for p in [PORT_API, PORT_NOVNC, PORT_TTYD] {
            exposed_ports.insert(format!("{p}/tcp"), HashMap::new());
        }

        let config = Config {
            image: Some(self.image.clone()),
            host_config: Some(host_config),
            exposed_ports: Some(exposed_ports),
            ..Default::default()
        };

        self.docker
            .create_container(
                Some(CreateContainerOptions {
                    name: name.clone(),
                    platform: None,
                }),
                config,
            )
            .await?;
        self.docker
            .start_container::<String>(&name, None::<StartContainerOptions<String>>)
            .await?;

        let inspect = self.docker.inspect_container(&name, None).await?;
        let container_id = inspect.id.clone().unwrap_or_default();

        let host_ports = extract_host_ports(&inspect)?;
        let api_host = host_ports
            .get(&PORT_API)
            .ok_or(SandboxError::PortMissing(PORT_API))?;
        let novnc_host = host_ports
            .get(&PORT_NOVNC)
            .ok_or(SandboxError::PortMissing(PORT_NOVNC))?;
        let ttyd_host = host_ports
            .get(&PORT_TTYD)
            .ok_or(SandboxError::PortMissing(PORT_TTYD))?;

        let handle = SandboxHandle {
            session_id: session_id.to_string(),
            container_id,
            api_url: format!("http://127.0.0.1:{api_host}"),
            novnc_url: format!("http://127.0.0.1:{novnc_host}"),
            ttyd_url: format!("ws://127.0.0.1:{ttyd_host}"),
            workspace_host_path: workspace_abs,
        };

        // Story 1.3: every new workspace is a git working tree from session
        // start (architecture §2.6). Wait briefly for the sandbox HTTP API
        // to come up, then run the bootstrap sequence. On failure, tear the
        // container down so no half-bootstrapped sandbox leaks.
        if let Err(err) = wait_for_api_ready(&handle.api_url).await {
            let _ = self
                .docker
                .remove_container(
                    &name,
                    Some(RemoveContainerOptions {
                        force: true,
                        v: false,
                        link: false,
                    }),
                )
                .await;
            return Err(err);
        }
        let bootstrap_client = reqwest::Client::new();
        if let Err(err) = bootstrap::run_bootstrap(&bootstrap_client, &handle.api_url).await {
            let _ = self
                .docker
                .remove_container(
                    &name,
                    Some(RemoveContainerOptions {
                        force: true,
                        v: false,
                        link: false,
                    }),
                )
                .await;
            return Err(err);
        }
        // Story 2.6: install the renderer toolchain (Pandoc + python-pptx
        // + openpyxl). Skipped by `SANDBOX_SKIP_RENDERER_INSTALL=1` so
        // integration tests don't pay the ~30-60 s install cost. Failure
        // surfaces as `WorkspaceBootstrap` per Phase 1 conventions —
        // session_create returns the same shape the bootstrap step
        // would emit.
        if let Err(err) =
            bootstrap::install_renderer_toolchain(&bootstrap_client, &handle.api_url).await
        {
            let _ = self
                .docker
                .remove_container(
                    &name,
                    Some(RemoveContainerOptions {
                        force: true,
                        v: false,
                        link: false,
                    }),
                )
                .await;
            return Err(err);
        }

        self.handles
            .write()
            .await
            .insert(session_id.to_string(), handle.clone());
        Ok(handle)
    }

    /// Story 2.6: invoke `/v1/shell/exec` on the sandbox for
    /// `session_id`. Used by the renderer dispatcher
    /// ([`crate::deliverable::renderer`]) to run Pandoc / Python
    /// scripts. Returns [`bootstrap::ShellExecOutcome`] verbatim so
    /// callers can branch on `exit_code` + read `stdout` / `stderr`.
    pub async fn shell_exec(
        &self,
        session_id: &str,
        command: &str,
    ) -> Result<bootstrap::ShellExecOutcome, SandboxError> {
        let handle = self
            .get(session_id)
            .await
            .ok_or_else(|| SandboxError::NotFound(session_id.to_string()))?;
        let client = reqwest::Client::new();
        bootstrap::shell_exec(&client, &handle.api_url, command).await
    }

    /// Story 1.13b: cheap pause-state probe used by the admin rollback
    /// endpoint to reject 409 sandbox_paused early. Returns `Ok(false)`
    /// when the container is missing or the docker daemon is
    /// unreachable — those aren't the "paused" failure mode the gate
    /// exists to catch, and treating them as "not paused" lets the
    /// rollback attempt proceed (a real revert failure will surface
    /// downstream as `revert_failed`).
    pub async fn is_paused(&self, session_id: &str) -> Result<bool, SandboxError> {
        require_safe_session_id(session_id)?;
        let name = container_name(session_id);
        match self.docker.inspect_container(&name, None).await {
            Ok(inspect) => Ok(inspect
                .state
                .as_ref()
                .and_then(|s| s.paused)
                .unwrap_or(false)),
            Err(_) => Ok(false),
        }
    }

    pub async fn pause(&self, session_id: &str) -> Result<(), SandboxError> {
        require_safe_session_id(session_id)?;
        let name = container_name(session_id);
        let inspect = self.docker.inspect_container(&name, None).await?;
        if let Some(state) = &inspect.state
            && state.paused == Some(true)
        {
            return Err(SandboxError::AlreadyPaused(session_id.into()));
        }
        self.docker.pause_container(&name).await?;
        Ok(())
    }

    pub async fn resume(&self, session_id: &str) -> Result<(), SandboxError> {
        require_safe_session_id(session_id)?;
        let name = container_name(session_id);
        let inspect = self.docker.inspect_container(&name, None).await?;
        if let Some(state) = &inspect.state
            && state.paused != Some(true)
        {
            return Err(SandboxError::NotPaused(session_id.into()));
        }
        self.docker.unpause_container(&name).await?;
        Ok(())
    }

    /// Scan Docker for `seasoned-hand-sandbox-*` containers and rehydrate
    /// the in-process handle cache from disk reality.
    ///
    /// Containers whose `sessions` row is in a live state ({IDLE, RUNNING,
    /// SUSPENDED, VERIFYING}) are re-registered. Containers whose session
    /// is missing or in a terminal state ({FINISHED, ERROR}) are logged as
    /// orphans and left running — Phase 0 DEBT #16 owns cleanup. Already
    /// cached sessions are skipped, which is what makes this idempotent.
    ///
    /// refs: /specs/phase-1/stories/story-1.2.md
    /// refs: /specs/phase-0/DEBT.md #18
    pub async fn rehydrate_from_docker(
        &self,
        sessions: &DbPool,
    ) -> Result<RehydrateReport, SandboxError> {
        let mut filters: HashMap<&str, Vec<&str>> = HashMap::new();
        filters.insert("name", vec![cache::SANDBOX_CONTAINER_PREFIX]);
        let containers = self
            .docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters,
                ..Default::default()
            }))
            .await?;

        let mut report = RehydrateReport::default();
        for summary in containers {
            let name = match summary
                .names
                .as_ref()
                .and_then(|n| n.first().cloned())
                .filter(|s| !s.is_empty())
            {
                Some(n) => n,
                None => continue,
            };
            let Some(session_id) = cache::extract_session_id_from_name(&name) else {
                continue;
            };
            let session_id = session_id.to_string();

            if self.handles.read().await.contains_key(&session_id) {
                continue;
            }

            let state = match lookup_session_state(sessions, &session_id).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(%session_id, error = %e, "rehydrate: db lookup failed");
                    report.errors.push(format!("{session_id}: db lookup: {e}"));
                    continue;
                }
            };

            if cache::is_live_state(state.as_deref()) {
                // Story 1.3 note: rehydration does NOT re-run
                // `workspace_bootstrap_commands` — the container's
                // /workspace volume is persistent across restarts, so its
                // git tree + identity are still intact from the original
                // create().
                match self.register_existing(&session_id).await {
                    Ok(()) => report.restored += 1,
                    Err(e) => {
                        tracing::warn!(%session_id, error = %e, "rehydrate: register_existing failed");
                        report
                            .errors
                            .push(format!("{session_id}: register_existing: {e}"));
                    }
                }
            } else {
                tracing::warn!(
                    orphan_container = %name,
                    state = ?state,
                    "orphan sandbox container left running (cleanup is DEBT #16)"
                );
                report.orphans += 1;
            }
        }

        tracing::info!(
            restored = report.restored,
            orphans = report.orphans,
            errors = report.errors.len(),
            "sandbox cache rehydrated"
        );
        Ok(report)
    }

    /// Reconstruct a `SandboxHandle` for an existing container without going
    /// through the `create()` path. Used by `rehydrate_from_docker`.
    async fn register_existing(&self, session_id: &str) -> Result<(), SandboxError> {
        require_safe_session_id(session_id)?;
        let name = container_name(session_id);
        let inspect = self.docker.inspect_container(&name, None).await?;
        let container_id = inspect.id.clone().unwrap_or_default();
        let host_ports = extract_host_ports(&inspect)?;
        let api_host = host_ports
            .get(&PORT_API)
            .ok_or(SandboxError::PortMissing(PORT_API))?;
        let novnc_host = host_ports
            .get(&PORT_NOVNC)
            .ok_or(SandboxError::PortMissing(PORT_NOVNC))?;
        let ttyd_host = host_ports
            .get(&PORT_TTYD)
            .ok_or(SandboxError::PortMissing(PORT_TTYD))?;

        let workspace = self.workspace_root.join(session_id);
        let workspace_abs = workspace.canonicalize().unwrap_or(workspace);

        let handle = SandboxHandle {
            session_id: session_id.to_string(),
            container_id,
            api_url: format!("http://127.0.0.1:{api_host}"),
            novnc_url: format!("http://127.0.0.1:{novnc_host}"),
            ttyd_url: format!("ws://127.0.0.1:{ttyd_host}"),
            workspace_host_path: workspace_abs,
        };
        self.handles
            .write()
            .await
            .insert(session_id.to_string(), handle);
        Ok(())
    }

    /// Idempotent: a missing container is treated as success.
    pub async fn destroy(&self, session_id: &str) -> Result<(), SandboxError> {
        require_safe_session_id(session_id)?;
        let name = container_name(session_id);
        let res = self
            .docker
            .remove_container(
                &name,
                Some(RemoveContainerOptions {
                    force: true,
                    v: false,
                    link: false,
                }),
            )
            .await;
        // Always clear from cache, success or not — caller's mental model is
        // "after destroy, this session has no container."
        self.handles.write().await.remove(session_id);

        match res {
            Ok(()) => Ok(()),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => Ok(()),
            Err(e) => Err(SandboxError::Docker(e)),
        }
    }

    /// Test/support utility: manually register a handle in the in-process cache.
    /// Used by non-Docker unit tests that exercise workspace-file helpers.
    pub async fn insert_handle_for_test(&self, handle: SandboxHandle) {
        self.handles
            .write()
            .await
            .insert(handle.session_id.clone(), handle);
    }

    /// Test/support utility: drop the cached handle for a session id so the
    /// next `get(...)` returns `None`. Mirrors what would happen if the
    /// docker container died and `rehydrate_from_docker` failed to find it.
    /// Used by story 2.26's live overnight test to force the durable-resume
    /// rebuild branch after `docker rm -f` hard-killed the container.
    pub async fn remove_handle_for_test(&self, session_id: &str) {
        self.handles.write().await.remove(session_id);
    }
}

/// Normalize an LLM- or operator-supplied workspace-relative path:
/// strip the `/workspace/` prefix and any leading slashes, then reject
/// any `..` segment. Without the `..` reject, a path like
/// `../../etc/passwd` would resolve outside the workspace bind-mount
/// when joined to `handle.workspace_host_path`, enabling host
/// path-traversal from any caller of `task_deliver` / `read_workspace_file` /
/// `write_workspace_file*` (REVIEW §1/B + §1/G, proposed DEBT #38).
///
/// We also reject null bytes (Rust `Path` would silently accept them
/// but the underlying OS calls truncate at them).
/// Strict alphabet for any string that flows into a host workspace path
/// or a Docker container name. Mirrors the intake-router check
/// (`is_safe_session_id` in `intake::router`) so the two layers carry the
/// same definition; the sandbox layer enforces it as defense-in-depth in
/// case a future caller bypasses the intake validator.
///
/// Threat model: an unfiltered `session_id` flows into
/// `workspace_root.join(...)` (host-side directory creation +
/// `remove_dir_all` during TTL cleanup) and `container_name(...)` (Docker
/// container name). A `..` segment escapes the workspace bind-mount;
/// shell metacharacters / slashes break the container name.
pub fn is_safe_session_id(s: &str) -> bool {
    if s.is_empty() || s.len() > 64 {
        return false;
    }
    s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Reject the call with an `InvalidWorkspace` error if `session_id` is
/// not strictly safe per [`is_safe_session_id`]. Used by every sandbox
/// entry point that joins `session_id` into a host path or container
/// name.
fn require_safe_session_id(session_id: &str) -> Result<(), SandboxError> {
    if !is_safe_session_id(session_id) {
        return Err(SandboxError::InvalidWorkspace(format!(
            "unsafe session_id rejected by sandbox layer: {session_id:?}"
        )));
    }
    Ok(())
}

pub(crate) fn normalize_workspace_relative_path(path: &str) -> Result<&str, SandboxError> {
    if path.contains('\0') {
        return Err(SandboxError::InvalidWorkspace(format!(
            "null byte in path: {path:?}"
        )));
    }
    let stripped = path
        .strip_prefix("/workspace/")
        .or_else(|| path.strip_prefix("workspace/"))
        .unwrap_or_else(|| path.trim_start_matches('/'));
    // Use Path::Component to catch `..` regardless of how the segment
    // was spelled (e.g. `foo/../bar`, `../bar`, `bar/..`). Plain
    // `contains("..")` would miss `..` between other segments.
    for component in std::path::Path::new(stripped).components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(SandboxError::InvalidWorkspace(format!(
                "parent-dir segment rejected: {path:?}"
            )));
        }
    }
    Ok(stripped)
}

/// SEC-IT4-M2: resolve `joined` (an already-`..`/null-byte-checked path under
/// the workspace root) to its real, symlink-followed host path and require it
/// to stay inside `root`. The target must exist (this is the READ path). This
/// defeats a sandbox that plants `ln -s /etc/passwd leak` inside the
/// bind-mounted workspace and then reads it back through a host-side helper.
pub(crate) async fn canonical_within_workspace(
    root: &std::path::Path,
    joined: &std::path::Path,
) -> Result<std::path::PathBuf, SandboxError> {
    let root = tokio::fs::canonicalize(root).await?;
    let real = tokio::fs::canonicalize(joined).await?;
    if !real.starts_with(&root) {
        return Err(SandboxError::InvalidWorkspace(format!(
            "workspace path escapes root via symlink: {}",
            joined.display()
        )));
    }
    Ok(real)
}

/// SEC-IT4-M2: write-path counterpart of [`canonical_within_workspace`]. The
/// final file may not exist yet, so we (a) reject if the target itself is an
/// existing symlink (writing through it would escape) and (b) require the
/// already-created parent directory to resolve inside `root` (catching a
/// symlinked ancestor component).
pub(crate) async fn reject_workspace_write_escape(
    root: &std::path::Path,
    target: &std::path::Path,
) -> Result<(), SandboxError> {
    if let Ok(md) = tokio::fs::symlink_metadata(target).await
        && md.file_type().is_symlink()
    {
        return Err(SandboxError::InvalidWorkspace(format!(
            "refusing to write through symlink: {}",
            target.display()
        )));
    }
    let root = tokio::fs::canonicalize(root).await?;
    let parent = target.parent().unwrap_or(target);
    let real_parent = tokio::fs::canonicalize(parent).await?;
    if !real_parent.starts_with(&root) {
        return Err(SandboxError::InvalidWorkspace(format!(
            "workspace write parent escapes root: {}",
            parent.display()
        )));
    }
    Ok(())
}

pub fn container_name(session_id: &str) -> String {
    format!("seasoned-hand-sandbox-{session_id}")
}

/// Block until the sandbox HTTP API accepts a TCP connection or the budget
/// runs out. Used by `SandboxClient::create` before running the workspace
/// bootstrap. ~15 s budget matches Phase 0's live-test wait (story 0.8).
async fn wait_for_api_ready(api_url: &str) -> Result<(), SandboxError> {
    let addr = api_url.trim_start_matches("http://").to_string();
    for _ in 0..30 {
        if tokio::net::TcpStream::connect(&addr).await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Err(SandboxError::WorkspaceBootstrap(format!(
        "sandbox API never became reachable at {addr}"
    )))
}

async fn lookup_session_state(
    pool: &DbPool,
    session_id: &str,
) -> Result<Option<String>, rusqlite::Error> {
    let id = session_id.to_string();
    pool.with_conn(move |conn| {
        match conn.query_row(
            "SELECT state FROM sessions WHERE id = ?",
            rusqlite::params![id],
            |row| row.get::<_, String>(0),
        ) {
            Ok(state) => Ok(Some(state)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    })
    .await
}

fn extract_host_ports(
    inspect: &bollard::secret::ContainerInspectResponse,
) -> Result<HashMap<u16, u16>, SandboxError> {
    let mut out = HashMap::new();
    let ports = inspect
        .network_settings
        .as_ref()
        .and_then(|n| n.ports.as_ref());
    if let Some(ports) = ports {
        for (k, v) in ports.iter() {
            // k is "8080/tcp" etc.; v is Option<Vec<PortBinding>>
            let Some(slash) = k.find('/') else { continue };
            let Ok(container_port) = k[..slash].parse::<u16>() else {
                continue;
            };
            let Some(bindings) = v else { continue };
            // Prefer 127.0.0.1 bindings.
            let host_port = bindings
                .iter()
                .filter(|b| b.host_ip.as_deref() == Some("127.0.0.1"))
                .find_map(|b| b.host_port.as_deref())
                .or_else(|| bindings.iter().find_map(|b| b.host_port.as_deref()))
                .and_then(|s| s.parse::<u16>().ok());
            if let Some(p) = host_port {
                out.insert(container_port, p);
            }
        }
    }
    Ok(out)
}

/// Shared GET-as-JSON helper for the Phase 1 `browser_view` accessor.
/// Kept private to the sandbox module — tool-side HTTP goes through
/// `tools::builtin::sandbox_get` which carries its own error mapping.
async fn sandbox_get_json(
    client: &reqwest::Client,
    api_url: &str,
    path: &str,
) -> Result<serde_json::Value, SandboxError> {
    let url = format!("{api_url}{path}");
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| SandboxError::Http(e.to_string()))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(SandboxError::HttpStatus {
            status: status.as_u16(),
            path: path.to_string(),
        });
    }
    resp.json::<serde_json::Value>()
        .await
        .map_err(|e| SandboxError::Http(e.to_string()))
}

#[cfg(test)]
mod tests;
