//! REST client for the control-plane `/v1` routes. Direct Rust port of
//! `frontend/lib/api.ts` using `gloo-net` (wasm `fetch`).

use crate::config::api_base;
use crate::dto::*;
use gloo_net::http::Request;

/// Error string surfaced to the UI (kept simple; callers render it inline).
pub type ApiResult<T> = Result<T, String>;

async fn get_json<T: serde::de::DeserializeOwned>(path: &str) -> ApiResult<T> {
    let url = format!("{}{}", api_base(), path);
    let resp = Request::get(&url)
        .send()
        .await
        .map_err(|e| format!("GET {path} -> {e}"))?;
    if !resp.ok() {
        return Err(format!("GET {path} -> {}", resp.status()));
    }
    resp.json::<T>()
        .await
        .map_err(|e| format!("GET {path} decode -> {e}"))
}

async fn post_json<B: serde::Serialize, T: serde::de::DeserializeOwned>(
    path: &str,
    body: &B,
) -> ApiResult<T> {
    let url = format!("{}{}", api_base(), path);
    let resp = Request::post(&url)
        .json(body)
        .map_err(|e| format!("POST {path} encode -> {e}"))?
        .send()
        .await
        .map_err(|e| format!("POST {path} -> {e}"))?;
    if !resp.ok() {
        return Err(format!("POST {path} -> {}", resp.status()));
    }
    resp.json::<T>()
        .await
        .map_err(|e| format!("POST {path} decode -> {e}"))
}

pub async fn list_sessions(limit: u32) -> ApiResult<Vec<SessionSummary>> {
    get_json(&format!("/v1/sessions?limit={limit}")).await
}

/// Sandbox endpoints surfaced by `GET /v1/sessions/:id` (mirrors api.ts).
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct Sandbox {
    pub novnc_url: String,
    pub ttyd_url: String,
    pub api_url: String,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct SessionDetail {
    #[serde(flatten)]
    pub summary: SessionSummary,
    pub sandbox: Option<Sandbox>,
}

pub async fn get_session(id: &str) -> ApiResult<SessionDetail> {
    get_json(&format!("/v1/sessions/{}", urlencode(id))).await
}

pub async fn list_projects(limit: u32) -> ApiResult<Vec<Project>> {
    get_json(&format!("/v1/projects?limit={limit}")).await
}

pub async fn get_project(id: &str) -> ApiResult<Project> {
    get_json(&format!("/v1/projects/{}", urlencode(id))).await
}

#[derive(serde::Serialize)]
struct CreateProjectBody<'a> {
    title: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
}

pub async fn create_project(title: &str, description: Option<&str>) -> ApiResult<Project> {
    post_json("/v1/projects", &CreateProjectBody { title, description }).await
}

pub async fn list_tasks(project_id: &str, limit: u32) -> ApiResult<Vec<Task>> {
    get_json(&format!(
        "/v1/projects/{}/tasks?limit={limit}",
        urlencode(project_id)
    ))
    .await
}

pub async fn get_task(id: &str) -> ApiResult<Task> {
    get_json(&format!("/v1/tasks/{}", urlencode(id))).await
}

pub async fn get_task_deliverables(task_id: &str) -> ApiResult<TaskDeliverablesResponse> {
    get_json(&format!("/v1/tasks/{}/deliverables", urlencode(task_id))).await
}

/// Minimal percent-encoding for path segments (ids are uuid-like but encode
/// defensively, matching `encodeURIComponent` in the TS layer).
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
