//! Shell-out helpers that commit a phase boundary inside the per-session
//! sandbox container via its `/v1/shell/exec` endpoint.
//!
//! The commit message is passed via `git commit -F <path>` reading from
//! a workspace file (NOT interpolated into the shell command line).
//! `phase_title` is LLM-controlled and may contain backticks / `$()` /
//! newlines; the file-based pattern guarantees none of it ever enters
//! the shell context. Story 2.19 + Phase 1 DEBT #14.
//!
//! No `git2` dep (architecture §5.1 — git is always invoked through the
//! sandbox shell so the LLM sees no git surface).
//!
//! refs: /specs/phase-1/stories/story-1.13.md
//! refs: /specs/phase-2/stories/story-2.19.md
//! refs: /specs/phase-1/architecture.md §2.6, §5.1

use serde::Deserialize;
use thiserror::Error;

use crate::sandbox::SandboxClient;

/// Outcome of one /v1/shell/exec call.
#[derive(Debug, Deserialize, Default)]
pub struct ShellOutcome {
    #[serde(default)]
    pub exit_code: i32,
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
}

#[derive(Debug, Error)]
pub enum CheckpointGitError {
    #[error("no sandbox handle for session {0}")]
    NoSandbox(String),
    #[error("http: {0}")]
    Http(String),
    #[error("shell command {cmd:?} exited {exit_code} stderr={stderr}")]
    NonZeroExit {
        cmd: String,
        exit_code: i32,
        stderr: String,
    },
    #[error("decode: {0}")]
    Decode(String),
}

/// Trait abstraction over the three-command commit sequence. The
/// production impl runs against the real sandbox HTTP API; tests inject
/// a mock that returns canned SHAs.
#[async_trait::async_trait]
pub trait GitShell: Send + Sync {
    async fn commit_phase(
        &self,
        session_id: &str,
        phase_id: i64,
        phase_title: &str,
    ) -> Result<String, CheckpointGitError>;
}

/// Production implementation backed by the per-session SandboxClient.
pub struct SandboxGitShell {
    sandbox: std::sync::Arc<SandboxClient>,
    http: reqwest::Client,
}

impl SandboxGitShell {
    pub fn new(sandbox: std::sync::Arc<SandboxClient>) -> Self {
        Self {
            sandbox,
            http: reqwest::Client::new(),
        }
    }

    async fn exec(&self, api_url: &str, cmd: &str) -> Result<ShellOutcome, CheckpointGitError> {
        let url = format!("{api_url}/v1/shell/exec");
        let resp = self
            .http
            .post(&url)
            .json(&serde_json::json!({ "command": cmd }))
            .send()
            .await
            .map_err(|e| CheckpointGitError::Http(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(CheckpointGitError::Http(format!(
                "HTTP {}: {body}",
                status.as_u16()
            )));
        }
        resp.json::<ShellOutcome>()
            .await
            .map_err(|e| CheckpointGitError::Decode(e.to_string()))
    }
}

#[async_trait::async_trait]
impl GitShell for SandboxGitShell {
    async fn commit_phase(
        &self,
        session_id: &str,
        phase_id: i64,
        phase_title: &str,
    ) -> Result<String, CheckpointGitError> {
        let handle = self
            .sandbox
            .get(session_id)
            .await
            .ok_or_else(|| CheckpointGitError::NoSandbox(session_id.to_string()))?;
        let api_url = &handle.api_url;

        // Write the commit message to a workspace file via the host-fs
        // mount (no shell). The path uses a server-generated numeric
        // `phase_id` (i64) so the filename itself can't carry injection.
        // The `phase_title` content lives in the file, never on the
        // shell command line — that's what makes the injection vector
        // from Phase 1 DEBT #14 unreachable.
        let msg_path = format!("/workspace/.commit-msg/{phase_id}.txt");
        let msg_body = format!("phase {phase_id}: {phase_title}");
        self.sandbox
            .write_workspace_file(session_id, &msg_path, msg_body.as_bytes())
            .await
            .map_err(|e| CheckpointGitError::Http(e.to_string()))?;

        // Three shell commands, each a constant string with no
        // user-controlled interpolation. `msg_path` is built from
        // `phase_id: i64` and a hardcoded prefix.
        let commit_cmd = format!("git -C /workspace commit -q --allow-empty -F {msg_path}");
        let cleanup_cmd = format!("rm -f {msg_path}");
        let cmds: [&str; 3] = [
            "git -C /workspace add -A",
            commit_cmd.as_str(),
            cleanup_cmd.as_str(),
        ];
        for cmd in cmds {
            let out = self.exec(api_url, cmd).await?;
            if out.exit_code != 0 {
                return Err(CheckpointGitError::NonZeroExit {
                    cmd: cmd.to_string(),
                    exit_code: out.exit_code,
                    stderr: out.stderr,
                });
            }
        }
        // Capture HEAD sha.
        let head = self
            .exec(api_url, "git -C /workspace rev-parse HEAD")
            .await?;
        if head.exit_code != 0 {
            return Err(CheckpointGitError::NonZeroExit {
                cmd: "git -C /workspace rev-parse HEAD".to_string(),
                exit_code: head.exit_code,
                stderr: head.stderr,
            });
        }
        Ok(head.stdout.trim().to_string())
    }
}
