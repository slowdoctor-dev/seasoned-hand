//! Sandbox workspace `git init` + identity bootstrap.
//!
//! Runs immediately after the container is created and reachable, so that
//! every sandbox workspace is a git working tree from session start with a
//! known identity and an `init` empty commit on HEAD. Story 1.13's
//! Checkpoint Manager will commit phase advances on top of this `init`.
//!
//! The LLM is never given git tools (architecture §2.6: "LLM **never sees**
//! git"). Identity is hardcoded per Phase 1 (see [`crate::sandbox::identity`]
//! and phase-1/DEBT.md #11).
//!
//! refs: /specs/phase-1/stories/story-1.3.md
//! refs: /specs/phase-1/architecture.md §2.6, §5.1

use serde::Deserialize;

use crate::sandbox::SandboxError;

/// The five shell commands run, in order, against the sandbox's
/// `/v1/shell/exec` endpoint on every create. Strings are constant for
/// clarity; the unit test asserts them as golden values.
pub(crate) fn workspace_bootstrap_commands() -> [&'static str; 5] {
    [
        "git init -q /workspace",
        "git -C /workspace config user.email \"seasoned-hand@local\"",
        "git -C /workspace config user.name \"Seasoned Hand\"",
        "git -C /workspace commit --allow-empty -q -m \"init\"",
        "git --version",
    ]
}

/// Subset of the AIO Sandbox `/v1/shell/exec` response. The endpoint
/// returns `{exit_code, stdout, stderr, ...}` per its OpenAPI; the
/// renderer dispatcher (story 2.6) needs `stdout` too so we keep the
/// full triple here.
///
/// `pub(crate)` so the renderer module can import it; not part of the
/// public surface — keep `ChannelError` / `RenderError` as the
/// surface-facing types.
#[derive(Debug, Deserialize, Default, Clone)]
pub struct ShellExecOutcome {
    #[serde(default)]
    pub exit_code: i32,
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
}

/// Story 2.6: renderer-toolchain install commands. Run after
/// [`run_bootstrap`] inside `SandboxClient::create`. ~30-60 s
/// one-time per session (acceptable per architecture §5 / DEBT #2,
/// which defers the pre-baked image to Phase 4). Skipped entirely
/// when `SANDBOX_SKIP_RENDERER_INSTALL=1` (tests + pre-baked-image
/// future).
pub(crate) fn renderer_install_commands() -> [&'static str; 2] {
    [
        // Pandoc for docx/pdf/html/odt; texlive-xetex for pdf-via-pandoc.
        // python3-pip is installed defensively — Phase 2 sandbox image
        // ships it but pre-baked images may not.
        "apt-get install -y pandoc texlive-xetex python3-pip",
        // python-pptx for pptx; openpyxl for xlsx. --break-system-packages
        // tolerates Debian/Ubuntu's PEP 668 lock.
        "pip3 install --break-system-packages python-pptx openpyxl",
    ]
}

/// Env override that disables the renderer install step. When set to
/// `"1"`, [`install_renderer_toolchain`] returns `Ok(())` immediately
/// without touching the sandbox — matches the pre-baked image future
/// (phase-2/DEBT.md #2) and lets the integration tests skip the
/// install cost.
pub const SKIP_INSTALL_ENV: &str = "SANDBOX_SKIP_RENDERER_INSTALL";

/// Story 2.6: install the renderer toolchain on a freshly-bootstrapped
/// sandbox. Returns the same `WorkspaceBootstrap` error variant as
/// [`run_bootstrap`] on first non-zero exit, so the upstream
/// `session_create_failed` surface stays uniform.
pub(crate) async fn install_renderer_toolchain(
    client: &reqwest::Client,
    api_url: &str,
) -> Result<(), SandboxError> {
    if std::env::var(SKIP_INSTALL_ENV).is_ok_and(|v| v == "1") {
        return Ok(());
    }
    for cmd in renderer_install_commands() {
        let outcome = post_shell_exec(client, api_url, cmd).await?;
        if outcome.exit_code != 0 {
            return Err(SandboxError::WorkspaceBootstrap(format!(
                "renderer install failed: cmd={cmd:?}, exit={}, stderr={:?}",
                outcome.exit_code,
                truncate(&outcome.stderr, 400),
            )));
        }
    }
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect()
    }
}

/// Public wrapper around [`post_shell_exec`] used by the renderer
/// dispatcher (story 2.6). Lets one-off shell invocations land
/// without re-implementing the request shape. `pub(crate)` keeps it
/// out of the SDK surface — the canonical caller is
/// [`crate::deliverable::renderer`].
pub(crate) async fn shell_exec(
    client: &reqwest::Client,
    api_url: &str,
    command: &str,
) -> Result<ShellExecOutcome, SandboxError> {
    post_shell_exec(client, api_url, command).await
}

/// Run the bootstrap sequence against `api_url` (the sandbox HTTP API).
///
/// Returns `Ok(())` on success, or `Err(SandboxError::WorkspaceBootstrap)`
/// the first time any command exits non-zero (or the HTTP call fails).
/// The caller is responsible for tearing down the container on error.
pub(crate) async fn run_bootstrap(
    client: &reqwest::Client,
    api_url: &str,
) -> Result<(), SandboxError> {
    for cmd in workspace_bootstrap_commands() {
        let outcome = post_shell_exec(client, api_url, cmd).await?;
        if outcome.exit_code != 0 {
            let msg = if cmd == "git --version"
                && (outcome.stderr.contains("not found") || outcome.stderr.contains("No such file"))
            {
                "sandbox image missing git binary".to_string()
            } else {
                format!(
                    "workspace bootstrap failed: cmd={cmd:?}, exit={}, stderr={:?}",
                    outcome.exit_code, outcome.stderr,
                )
            };
            return Err(SandboxError::WorkspaceBootstrap(msg));
        }
    }
    Ok(())
}

async fn post_shell_exec(
    client: &reqwest::Client,
    api_url: &str,
    command: &str,
) -> Result<ShellExecOutcome, SandboxError> {
    let url = format!("{api_url}/v1/shell/exec");
    let resp = client
        .post(&url)
        .json(&serde_json::json!({ "command": command }))
        .send()
        .await
        .map_err(|e| SandboxError::WorkspaceBootstrap(format!("http: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(SandboxError::WorkspaceBootstrap(format!(
            "shell/exec HTTP {}: {body}",
            status.as_u16()
        )));
    }
    resp.json::<ShellExecOutcome>()
        .await
        .map_err(|e| SandboxError::WorkspaceBootstrap(format!("decode: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn bootstrap_command_strings_are_correct() {
        let cmds = workspace_bootstrap_commands();
        // Golden-string asserts — catches typos in quoting / `-C` flag /
        // working dir / identity values.
        assert_eq!(cmds[0], "git init -q /workspace");
        assert_eq!(
            cmds[1],
            "git -C /workspace config user.email \"seasoned-hand@local\""
        );
        assert_eq!(
            cmds[2],
            "git -C /workspace config user.name \"Seasoned Hand\""
        );
        assert_eq!(
            cmds[3],
            "git -C /workspace commit --allow-empty -q -m \"init\""
        );
        assert_eq!(cmds[4], "git --version");
        assert_eq!(cmds.len(), 5);
    }

    #[test]
    fn identity_constants_match_bootstrap_commands() {
        // Guards against drift between the constants module and the inlined
        // string literals in the bootstrap command list.
        use crate::sandbox::identity::{GIT_USER_EMAIL, GIT_USER_NAME};
        let cmds = workspace_bootstrap_commands();
        assert!(cmds[1].contains(GIT_USER_EMAIL), "email drifted");
        assert!(cmds[2].contains(GIT_USER_NAME), "name drifted");
    }

    #[tokio::test]
    async fn run_bootstrap_happy_path_against_mock() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/shell/exec"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "exit_code": 0,
                "stdout": "",
                "stderr": ""
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        run_bootstrap(&client, &server.uri())
            .await
            .expect("bootstrap should succeed against a 0-exit mock");
    }

    #[tokio::test]
    async fn run_bootstrap_surfaces_missing_git_error() {
        let server = MockServer::start().await;
        // First four commands succeed; `git --version` returns 127.
        Mock::given(method("POST"))
            .and(path("/v1/shell/exec"))
            .and(body_partial_json(serde_json::json!({
                "command": "git --version"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "exit_code": 127,
                "stdout": "",
                "stderr": "sh: 1: git: not found"
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/shell/exec"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "exit_code": 0,
                "stdout": "",
                "stderr": ""
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let err = run_bootstrap(&client, &server.uri())
            .await
            .expect_err("bootstrap should fail when git is missing");
        match err {
            SandboxError::WorkspaceBootstrap(msg) => {
                assert_eq!(msg, "sandbox image missing git binary");
            }
            other => panic!("expected WorkspaceBootstrap, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_bootstrap_surfaces_generic_nonzero_exit() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/shell/exec"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "exit_code": 1,
                "stdout": "",
                "stderr": "fatal: reinitialized existing Git repository"
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let err = run_bootstrap(&client, &server.uri())
            .await
            .expect_err("bootstrap should fail on first non-zero exit");
        match err {
            SandboxError::WorkspaceBootstrap(msg) => {
                assert!(msg.contains("workspace bootstrap failed"), "msg was: {msg}");
                assert!(msg.contains("git init"), "msg was: {msg}");
                assert!(msg.contains("exit=1"), "msg was: {msg}");
            }
            other => panic!("expected WorkspaceBootstrap, got {other:?}"),
        }
    }
}
