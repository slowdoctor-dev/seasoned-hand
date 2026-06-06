//! REST client for the control-plane `/v1` routes. Direct Rust port of
//! `frontend/lib/api.ts` using `gloo-net` (wasm `fetch`).

use crate::config::api_base;
use gloo_net::http::Request;
use seasoned_hand_dto::*;

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

// `Sandbox` + `SessionDetail` now live in `seasoned-hand-dto` (story 6.3).
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

pub async fn list_verifications(
    session_id: &str,
    limit: u32,
) -> ApiResult<VerificationListResponse> {
    get_json(&format!(
        "/v1/sessions/{}/verifications?limit={limit}",
        urlencode(session_id)
    ))
    .await
}

/// Root-level workspace listing for a session.
pub async fn list_workspace_root(session_id: &str) -> ApiResult<WorkspaceListing> {
    list_workspace_dir(session_id, "").await
}

/// Workspace listing for a sub-directory (`path` is workspace-relative, may be
/// empty for the root).
pub async fn list_workspace_dir(session_id: &str, path: &str) -> ApiResult<WorkspaceListing> {
    let tail = encode_path(path.trim_start_matches('/'));
    get_json(&format!("/v1/workspace/{}/{}", urlencode(session_id), tail)).await
}

/// Read a workspace file's contents as text.
pub async fn read_workspace_file(session_id: &str, path: &str) -> ApiResult<String> {
    let tail = encode_path(path.trim_start_matches('/'));
    let url = format!(
        "{}/v1/workspace/{}/{}",
        api_base(),
        urlencode(session_id),
        tail
    );
    let resp = Request::get(&url)
        .send()
        .await
        .map_err(|e| format!("GET {path} -> {e}"))?;
    if !resp.ok() {
        return Err(format!("GET {path} -> {}", resp.status()));
    }
    resp.text()
        .await
        .map_err(|e| format!("GET {path} text -> {e}"))
}

/// Percent-encode each path segment but keep `/` separators.
fn encode_path(path: &str) -> String {
    path.split('/').map(urlencode).collect::<Vec<_>>().join("/")
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
