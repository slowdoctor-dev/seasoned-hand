//! OpenAI-compatible chat-completions client.
//! Talks to Bifrost (or any OpenAI-compatible endpoint) at <base_url>/chat/completions.
//! refs: /specs/phase-0/architecture.md §1, §4.4

pub mod types;

use reqwest::Client;
use thiserror::Error;

pub use types::*;

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("status {code}: {body}")]
    Status { code: u16, body: String },
    #[error("json parse: {0}")]
    JsonParse(#[from] serde_json::Error),
    #[error("response missing choices")]
    MissingChoice,
}

#[derive(Clone)]
pub struct LlmClient {
    http: Client,
    base_url: String,
    api_key: Option<String>,
}

impl LlmClient {
    pub fn new(base_url: impl Into<String>, api_key: Option<String>) -> Self {
        Self {
            http: Client::new(),
            base_url: base_url.into(),
            api_key,
        }
    }

    /// Defaults: `BIFROST_BASE_URL` (or http://localhost:4000/v1)
    /// and optional `BIFROST_MASTER_KEY` (Phase 0 typically unused
    /// per architecture §9 — bound to 127.0.0.1 without auth).
    pub fn from_env() -> Self {
        let base_url =
            std::env::var("BIFROST_BASE_URL").unwrap_or_else(|_| "http://localhost:4000/v1".into());
        let api_key = std::env::var("BIFROST_MASTER_KEY")
            .ok()
            .filter(|s| !s.is_empty());
        Self::new(base_url, api_key)
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub async fn chat_completion(
        &self,
        req: ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, LlmError> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let mut builder = self.http.post(&url).json(&req);
        if let Some(key) = &self.api_key {
            builder = builder.bearer_auth(key);
        }
        let resp = builder.send().await?;
        let status = resp.status();
        let bytes = resp.bytes().await?;
        if !status.is_success() {
            let body = String::from_utf8_lossy(&bytes).into_owned();
            return Err(LlmError::Status {
                code: status.as_u16(),
                body,
            });
        }
        let parsed: ChatCompletionResponse = serde_json::from_slice(&bytes)?;
        if parsed.choices.is_empty() {
            return Err(LlmError::MissingChoice);
        }
        Ok(parsed)
    }

    pub async fn list_models(&self) -> Result<Vec<ModelInfo>, LlmError> {
        let url = format!("{}/models", self.base_url.trim_end_matches('/'));
        let mut builder = self.http.get(&url);
        if let Some(key) = &self.api_key {
            builder = builder.bearer_auth(key);
        }
        let resp = builder.send().await?;
        let status = resp.status();
        let bytes = resp.bytes().await?;
        if !status.is_success() {
            let body = String::from_utf8_lossy(&bytes).into_owned();
            return Err(LlmError::Status {
                code: status.as_u16(),
                body,
            });
        }
        let parsed: ModelList = serde_json::from_slice(&bytes)?;
        Ok(parsed.data)
    }
}

#[cfg(test)]
mod tests;
