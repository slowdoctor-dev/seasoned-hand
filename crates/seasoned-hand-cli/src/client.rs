//! HTTP client wrapper for the `seasoned-hand` server.
//!
//! Maps `RouteOutcome`-style responses (200/201/202/4xx/5xx) into a
//! typed [`ApiError`] enum so subcommand handlers can pattern-match on
//! the failure shape (e.g. `404 task_not_found`, `409 wrong_state:running`).

use anyhow::{Context, Result};
use seasoned_hand_core::agent::init::briefing::PartialBrief;
use seasoned_hand_core::deliverable::Deliverable;
use seasoned_hand_core::project::{Project, Task};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct ApiClient {
    base_url: String,
    inner: reqwest::Client,
}

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("server returned {status}: {error}")]
    Server { status: u16, error: String },
    #[error("transport: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("decode: {0}")]
    Decode(String),
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    #[serde(default)]
    error: String,
}

#[derive(Debug, Serialize)]
struct CreateProjectBody<'a> {
    title: &'a str,
    description: Option<&'a str>,
}

#[derive(Debug, Serialize, Default)]
struct PauseBody {
    durable: Option<bool>,
}

#[derive(Debug, Serialize)]
struct CliIntakeBody<'a> {
    brief: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    project_id: Option<&'a str>,
    metadata: serde_json::Value,
    wait: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CliIntakeAck {
    pub task_id: String,
    pub intake_id: String,
    #[serde(default)]
    pub deliverable: Option<Deliverable>,
    #[serde(default)]
    pub briefing_call_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InboxEntry {
    pub briefing_id: String,
    pub task_id: String,
    pub project_id: String,
    pub title: String,
    #[serde(default)]
    pub brief: Option<serde_json::Value>,
    pub created_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SopShareEntry {
    pub id: String,
    pub tenant_id: String,
    pub sop_id: String,
    pub subject_type: String,
    pub subject_id: String,
    #[serde(default)]
    pub subject_email: Option<String>,
    pub permission: String,
    pub granted_by_user_id: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize)]
struct SopShareBody<'a> {
    user_email: &'a str,
    permission: &'a str,
}

#[derive(Debug, Serialize)]
struct SopUnshareBody<'a> {
    user_email: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(tag = "action", rename_all = "lowercase")]
pub enum BriefingConfirmRequest {
    Confirm,
    Cancel,
    Edit { edits: PartialBrief },
}

#[derive(Debug, Serialize)]
struct BriefingConfirmBody<'a> {
    action: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    edits: Option<&'a PartialBrief>,
}

impl ApiClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            inner: reqwest::Client::new(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    fn with_auth_headers(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let tenant_id =
            std::env::var("SH_TENANT_ID").unwrap_or_else(|_| "legacy-default".to_string());
        let org_id = std::env::var("SH_ORGANIZATION_ID")
            .unwrap_or_else(|_| "org-legacy-default".to_string());
        let actor_user_id =
            std::env::var("SH_ACTOR_USER_ID").unwrap_or_else(|_| "user-cli-operator".to_string());
        let role = std::env::var("SH_ORG_ROLE").unwrap_or_else(|_| "admin".to_string());
        req.header("x-seasoned-hand-tenant-id", tenant_id)
            .header("x-seasoned-hand-organization-id", org_id)
            .header("x-seasoned-hand-actor-user-id", actor_user_id)
            .header("x-seasoned-hand-org-role", role)
    }

    pub async fn list_projects(&self) -> Result<Vec<Project>, ApiError> {
        let resp = self
            .with_auth_headers(self.inner.get(self.url("/v1/projects")))
            .send()
            .await?;
        decode(resp).await
    }

    pub async fn create_project(
        &self,
        title: &str,
        description: Option<&str>,
    ) -> Result<Project, ApiError> {
        let resp = self
            .with_auth_headers(self.inner.post(self.url("/v1/projects")))
            .json(&CreateProjectBody { title, description })
            .send()
            .await?;
        decode(resp).await
    }

    pub async fn archive_project(&self, id: &str) -> Result<(), ApiError> {
        let resp = self
            .with_auth_headers(
                self.inner
                    .post(self.url(&format!("/v1/projects/{id}/archive"))),
            )
            .send()
            .await?;
        decode_unit(resp).await
    }

    pub async fn list_tasks(
        &self,
        project_id: &str,
        status: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<Task>, ApiError> {
        let mut req = self.with_auth_headers(
            self.inner
                .get(self.url(&format!("/v1/projects/{project_id}/tasks"))),
        );
        let mut query: Vec<(&str, String)> = Vec::new();
        if let Some(s) = status {
            query.push(("status", s.to_string()));
        }
        if let Some(l) = limit {
            query.push(("limit", l.to_string()));
        }
        if !query.is_empty() {
            req = req.query(&query);
        }
        let resp = req.send().await?;
        decode(resp).await
    }

    pub async fn get_task(&self, id: &str) -> Result<Task, ApiError> {
        let resp = self
            .with_auth_headers(self.inner.get(self.url(&format!("/v1/tasks/{id}"))))
            .send()
            .await?;
        decode(resp).await
    }

    pub async fn pause_task(&self, id: &str, durable: bool) -> Result<(), ApiError> {
        let resp = self
            .with_auth_headers(self.inner.post(self.url(&format!("/v1/tasks/{id}/pause"))))
            .json(&PauseBody {
                durable: Some(durable),
            })
            .send()
            .await?;
        decode_unit(resp).await
    }

    pub async fn resume_task(&self, id: &str) -> Result<(), ApiError> {
        let resp = self
            .with_auth_headers(self.inner.post(self.url(&format!("/v1/tasks/{id}/resume"))))
            .send()
            .await?;
        decode_unit(resp).await
    }

    pub async fn cancel_task(&self, id: &str) -> Result<(), ApiError> {
        let resp = self
            .with_auth_headers(self.inner.post(self.url(&format!("/v1/tasks/{id}/cancel"))))
            .send()
            .await?;
        decode_unit(resp).await
    }

    pub async fn task_provenance(&self, id: &str) -> Result<serde_json::Value, ApiError> {
        let resp = self
            .with_auth_headers(
                self.inner
                    .get(self.url(&format!("/v1/tasks/{id}/provenance"))),
            )
            .send()
            .await?;
        decode(resp).await
    }

    /// Block on POST /v1/intake/cli until the deliverable comes back, or
    /// the server's long-poll ceiling fires. `max_wait` is total request
    /// timeout — bumped past `CLI_INTAKE_MAX_WAIT_SECS` so the server
    /// times out first and surfaces its `deliver_timeout` error.
    pub async fn intake_cli_blocking(
        &self,
        brief: &str,
        project_id: Option<&str>,
        metadata: serde_json::Value,
        max_wait: std::time::Duration,
    ) -> Result<CliIntakeAck, ApiError> {
        // +30s padding so the server's tokio::time::timeout fires before
        // reqwest gives up on us.
        let client_timeout = max_wait + std::time::Duration::from_secs(30);
        let inner = reqwest::Client::builder()
            .timeout(client_timeout)
            .build()
            .map_err(ApiError::Transport)?;
        let resp = self
            .with_auth_headers(inner.post(self.url("/v1/intake/cli")))
            .json(&CliIntakeBody {
                brief,
                project_id,
                metadata,
                wait: true,
            })
            .send()
            .await?;
        decode(resp).await
    }

    /// POST /v1/intake/cli with wait=false — ack as soon as the task row
    /// is minted. The deliverable still lands in
    /// `~/.seasoned-hand/deliverables/` (the CliChannel's file fallback).
    pub async fn intake_cli_detach(
        &self,
        brief: &str,
        project_id: Option<&str>,
        metadata: serde_json::Value,
    ) -> Result<CliIntakeAck, ApiError> {
        let resp = self
            .with_auth_headers(self.inner.post(self.url("/v1/intake/cli")))
            .json(&CliIntakeBody {
                brief,
                project_id,
                metadata,
                wait: false,
            })
            .send()
            .await?;
        decode(resp).await
    }

    pub async fn list_inbox(&self, project_id: Option<&str>) -> Result<Vec<InboxEntry>, ApiError> {
        let mut req = self.with_auth_headers(self.inner.get(self.url("/v1/inbox")));
        if let Some(pid) = project_id {
            req = req.query(&[("project_id", pid)]);
        }
        let resp = req.send().await?;
        decode(resp).await
    }

    pub async fn briefing_confirm(
        &self,
        briefing_id: &str,
        action: BriefingConfirmRequest,
    ) -> Result<(), ApiError> {
        let body = match &action {
            BriefingConfirmRequest::Confirm => BriefingConfirmBody {
                action: "confirm",
                edits: None,
            },
            BriefingConfirmRequest::Cancel => BriefingConfirmBody {
                action: "cancel",
                edits: None,
            },
            BriefingConfirmRequest::Edit { edits } => BriefingConfirmBody {
                action: "edit",
                edits: Some(edits),
            },
        };
        let resp = self
            .with_auth_headers(
                self.inner
                    .post(self.url(&format!("/v1/briefings/{briefing_id}/confirm"))),
            )
            .json(&body)
            .send()
            .await?;
        decode_unit(resp).await
    }

    pub async fn list_channels(&self) -> Result<serde_json::Value, ApiError> {
        let resp = self
            .with_auth_headers(self.inner.get(self.url("/v1/channels")))
            .send()
            .await?;
        decode(resp).await
    }

    pub async fn sop_share(
        &self,
        sop_id: &str,
        user_email: &str,
        permission: &str,
    ) -> Result<SopShareEntry, ApiError> {
        let resp = self
            .with_auth_headers(
                self.inner
                    .post(self.url(&format!("/v1/sops/{sop_id}/shares"))),
            )
            .json(&SopShareBody {
                user_email,
                permission,
            })
            .send()
            .await?;
        decode(resp).await
    }

    pub async fn sop_unshare(&self, sop_id: &str, user_email: &str) -> Result<(), ApiError> {
        let resp = self
            .with_auth_headers(
                self.inner
                    .delete(self.url(&format!("/v1/sops/{sop_id}/shares"))),
            )
            .json(&SopUnshareBody { user_email })
            .send()
            .await?;
        decode_unit(resp).await
    }

    pub async fn sop_list_shares(&self, sop_id: &str) -> Result<Vec<SopShareEntry>, ApiError> {
        let resp = self
            .with_auth_headers(
                self.inner
                    .get(self.url(&format!("/v1/sops/{sop_id}/shares"))),
            )
            .send()
            .await?;
        decode(resp).await
    }

    pub async fn channel_test(
        &self,
        name: &str,
        role: Option<&str>,
    ) -> Result<serde_json::Value, ApiError> {
        let mut req = self.with_auth_headers(
            self.inner
                .post(self.url(&format!("/v1/channels/{name}/test"))),
        );
        if let Some(r) = role {
            req = req.query(&[("role", r)]);
        }
        let resp = req.send().await?;
        decode(resp).await
    }
}

async fn decode<T: serde::de::DeserializeOwned>(resp: reqwest::Response) -> Result<T, ApiError> {
    let status = resp.status();
    if status.is_success() {
        let bytes = resp.bytes().await?;
        serde_json::from_slice(&bytes).map_err(|e| ApiError::Decode(e.to_string()))
    } else {
        Err(ApiError::Server {
            status: status.as_u16(),
            error: extract_error(resp).await,
        })
    }
}

async fn decode_unit(resp: reqwest::Response) -> Result<(), ApiError> {
    let status = resp.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(ApiError::Server {
            status: status.as_u16(),
            error: extract_error(resp).await,
        })
    }
}

async fn extract_error(resp: reqwest::Response) -> String {
    let bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => return format!("transport error reading body: {e}"),
    };
    if bytes.is_empty() {
        return "(empty body)".into();
    }
    match serde_json::from_slice::<ErrorBody>(&bytes) {
        Ok(b) if !b.error.is_empty() => b.error,
        _ => String::from_utf8_lossy(&bytes).into_owned(),
    }
}

/// Translate a [`reqwest::Error`] into a user-friendly anyhow error.
/// Differentiates connection-refused (typical when the server isn't
/// running) from generic transport failures.
pub fn anyhow_err(err: ApiError) -> anyhow::Error {
    match &err {
        ApiError::Transport(e) if e.is_connect() => {
            anyhow::Error::msg(format!("could not reach server: {e}"))
                .context("is `seasoned-hand-server` running?")
        }
        _ => anyhow::Error::msg(err.to_string()),
    }
}

/// Resolve subcommand errors into a single anyhow chain.
pub fn into_anyhow<T>(res: Result<T, ApiError>) -> Result<T> {
    res.map_err(anyhow_err)
        .with_context(|| "API call failed".to_string())
}
