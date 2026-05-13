//! Shell-out helpers that run `git add -A && git commit -q --allow-empty
//! -m "phase N: <title>" && git rev-parse HEAD` inside the per-session
//! sandbox container via its `/v1/shell/exec` endpoint.
//!
//! No `git2` dep (architecture §5.1 — git is always invoked through the
//! sandbox shell so the LLM sees no git surface).
//!
//! refs: /specs/phase-1/stories/story-1.13.md
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

        let title_esc = phase_title.replace('"', "\\\"");
        let commit_msg = format!("phase {phase_id}: {title_esc}");
        let cmds = [
            "git -C /workspace add -A".to_string(),
            format!("git -C /workspace commit -q --allow-empty -m \"{commit_msg}\""),
        ];
        for cmd in &cmds {
            let out = self.exec(api_url, cmd).await?;
            if out.exit_code != 0 {
                return Err(CheckpointGitError::NonZeroExit {
                    cmd: cmd.clone(),
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
