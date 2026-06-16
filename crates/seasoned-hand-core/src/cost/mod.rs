//! Bifrost cost polling.
//! refs: /specs/phase-0/stories/story-0.16.md
//! refs: /specs/phase-0/architecture.md §4.4, §7

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CostError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("parse: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("status {code}: {body}")]
    Status { code: u16, body: String },
    #[error("clock error: {0}")]
    Clock(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CostSnapshot {
    pub total_cents: i64,
    #[serde(default = "default_currency")]
    pub currency: String,
    pub ts: i64,
}

#[derive(Clone)]
pub struct CostClient {
    http: reqwest::Client,
    base_url: String,
}

impl CostClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        // Issue #22: bound the Bifrost cost call so a hung gateway can't stall
        // cost snapshots (and the cost-cap polling that depends on them)
        // indefinitely. 15s matches the webhook/email network timeouts. The
        // builder config is static, so `expect` here is a construction invariant,
        // not a runtime-input failure (same idiom as WebhookChannel).
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("default cost reqwest client");
        Self {
            http,
            base_url: normalize_base_url(&base_url.into()),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub async fn snapshot(&self) -> Result<CostSnapshot, CostError> {
        let url = format!("{}/cost", self.base_url.trim_end_matches('/'));
        let resp = self.http.get(&url).send().await?;
        let status = resp.status();
        let bytes = resp.bytes().await?;
        if !status.is_success() {
            return Err(CostError::Status {
                code: status.as_u16(),
                body: String::from_utf8_lossy(&bytes).into_owned(),
            });
        }

        let value: Value = serde_json::from_slice(&bytes)?;
        Ok(CostSnapshot {
            total_cents: total_cents(&value),
            currency: value
                .get("currency")
                .and_then(Value::as_str)
                .unwrap_or("USD")
                .to_string(),
            ts: now_unix()?,
        })
    }

    pub async fn delta_cents(&self, baseline: &CostSnapshot) -> Result<i64, CostError> {
        let current = self.snapshot().await?;
        Ok(delta_between(baseline, &current))
    }
}

pub fn delta_between(baseline: &CostSnapshot, current: &CostSnapshot) -> i64 {
    (current.total_cents - baseline.total_cents).max(0)
}

fn total_cents(value: &Value) -> i64 {
    if let Some(cents) = value.get("total_cents").and_then(Value::as_i64) {
        cents
    } else if let Some(usd) = value.get("total_usd").and_then(Value::as_f64) {
        (usd * 100.0).round() as i64
    } else if let Some(usd) = value.get("total").and_then(Value::as_f64) {
        (usd * 100.0).round() as i64
    } else {
        0
    }
}

fn normalize_base_url(base_url: &str) -> String {
    base_url
        .trim_end_matches('/')
        .strip_suffix("/v1")
        .unwrap_or_else(|| base_url.trim_end_matches('/'))
        .to_string()
}

fn default_currency() -> String {
    "USD".into()
}

fn now_unix() -> Result<i64, CostError> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| CostError::Clock(error.to_string()))?
        .as_secs();
    i64::try_from(seconds).map_err(|error| CostError::Clock(error.to_string()))
}

#[cfg(test)]
mod tests;
