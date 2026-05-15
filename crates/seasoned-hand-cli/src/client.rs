//! HTTP client wrapper for the `seasoned-hand` server.
//!
//! Maps `RouteOutcome`-style responses (200/201/202/4xx/5xx) into a
//! typed [`ApiError`] enum so subcommand handlers can pattern-match on
//! the failure shape (e.g. `404 task_not_found`, `409 wrong_state:running`).

use anyhow::{Context, Result};
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

    pub async fn list_projects(&self) -> Result<Vec<Project>, ApiError> {
        let resp = self.inner.get(self.url("/v1/projects")).send().await?;
        decode(resp).await
    }

    pub async fn create_project(
        &self,
        title: &str,
        description: Option<&str>,
    ) -> Result<Project, ApiError> {
        let resp = self
            .inner
            .post(self.url("/v1/projects"))
            .json(&CreateProjectBody { title, description })
            .send()
            .await?;
        decode(resp).await
    }

    pub async fn archive_project(&self, id: &str) -> Result<(), ApiError> {
        let resp = self
            .inner
            .post(self.url(&format!("/v1/projects/{id}/archive")))
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
        let mut req = self
            .inner
            .get(self.url(&format!("/v1/projects/{project_id}/tasks")));
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
            .inner
            .get(self.url(&format!("/v1/tasks/{id}")))
            .send()
            .await?;
        decode(resp).await
    }

    pub async fn pause_task(&self, id: &str, durable: bool) -> Result<(), ApiError> {
        let resp = self
            .inner
            .post(self.url(&format!("/v1/tasks/{id}/pause")))
            .json(&PauseBody {
                durable: Some(durable),
            })
            .send()
            .await?;
        decode_unit(resp).await
    }

    pub async fn resume_task(&self, id: &str) -> Result<(), ApiError> {
        let resp = self
            .inner
            .post(self.url(&format!("/v1/tasks/{id}/resume")))
            .send()
            .await?;
        decode_unit(resp).await
    }

    pub async fn cancel_task(&self, id: &str) -> Result<(), ApiError> {
        let resp = self
            .inner
            .post(self.url(&format!("/v1/tasks/{id}/cancel")))
            .send()
            .await?;
        decode_unit(resp).await
    }

    pub async fn task_provenance(&self, id: &str) -> Result<serde_json::Value, ApiError> {
        let resp = self
            .inner
            .get(self.url(&format!("/v1/tasks/{id}/provenance")))
            .send()
            .await?;
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
