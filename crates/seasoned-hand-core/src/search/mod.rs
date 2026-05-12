//! Web search clients. Phase 0 ships Brave; Tavily reserved for Phase 1.
//! refs: /specs/phase-0/architecture.md §4.3

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("missing api key for provider {0}")]
    MissingApiKey(&'static str),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("provider {0} not implemented in Phase 0")]
    ProviderNotImplemented(&'static str),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[derive(Clone)]
pub enum SearchProvider {
    Brave { api_key: Option<String> },
    Tavily { api_key: Option<String> },
}

#[derive(Clone)]
pub struct SearchClient {
    inner: reqwest::Client,
    provider: SearchProvider,
}

impl SearchClient {
    pub fn new(provider: SearchProvider) -> Self {
        Self {
            inner: reqwest::Client::new(),
            provider,
        }
    }

    pub fn brave_from_env() -> Self {
        Self::new(SearchProvider::Brave {
            api_key: std::env::var("BRAVE_API_KEY")
                .ok()
                .filter(|s| !s.is_empty()),
        })
    }

    pub async fn web_search(
        &self,
        query: &str,
        max_results: usize,
    ) -> Result<Vec<SearchHit>, SearchError> {
        match &self.provider {
            SearchProvider::Brave { api_key: None } => Err(SearchError::MissingApiKey("brave")),
            SearchProvider::Brave { api_key: Some(key) } => {
                self.brave_search(query, max_results, key, "https://api.search.brave.com")
                    .await
            }
            SearchProvider::Tavily { .. } => Err(SearchError::ProviderNotImplemented("tavily")),
        }
    }

    async fn brave_search(
        &self,
        query: &str,
        max_results: usize,
        api_key: &str,
        base_url: &str,
    ) -> Result<Vec<SearchHit>, SearchError> {
        let url = format!(
            "{}/res/v1/web/search?q={}&count={}",
            base_url,
            urlencoding::encode(query),
            max_results.clamp(1, 20)
        );
        let resp = self
            .inner
            .get(&url)
            .header("X-Subscription-Token", api_key)
            .header("Accept", "application/json")
            .send()
            .await?
            .error_for_status()?;
        let body: BraveResp = resp.json().await?;
        Ok(body
            .web
            .map(|w| w.results)
            .unwrap_or_default()
            .into_iter()
            .map(|r| SearchHit {
                title: r.title,
                url: r.url,
                snippet: r.description,
            })
            .collect())
    }
}

#[derive(Deserialize)]
struct BraveResp {
    web: Option<BraveWeb>,
}

#[derive(Deserialize)]
struct BraveWeb {
    results: Vec<BraveResult>,
}

#[derive(Deserialize)]
struct BraveResult {
    title: String,
    url: String,
    #[serde(default)]
    description: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn returns_missing_api_key_when_brave_unset() {
        let c = SearchClient::new(SearchProvider::Brave { api_key: None });
        let err = c.web_search("anything", 5).await.unwrap_err();
        matches!(err, SearchError::MissingApiKey("brave"));
    }

    #[tokio::test]
    async fn tavily_returns_not_implemented() {
        let c = SearchClient::new(SearchProvider::Tavily {
            api_key: Some("k".into()),
        });
        let err = c.web_search("anything", 5).await.unwrap_err();
        matches!(err, SearchError::ProviderNotImplemented("tavily"));
    }
}
