//! REST client for the control-plane `/v1` routes. Direct Rust port of
//! `frontend/lib/api.ts` using `gloo-net` (wasm `fetch`).

use crate::auth;
use crate::config::api_base;
use gloo_net::http::{Request, RequestBuilder, Response};
use seasoned_hand_dto::*;

/// Error string surfaced to the UI (kept simple; callers render it inline).
pub type ApiResult<T> = Result<T, String>;

/// Attach `Authorization: Bearer <token>` when a verified session is available
/// (issue #26 / ADR-018). Requests fired before login carry no token and the
/// server replies 401, which routes through [`check_status`] to the login flow.
fn with_auth(builder: RequestBuilder) -> RequestBuilder {
    match auth::current_token() {
        Some(token) => builder.header("Authorization", &format!("Bearer {token}")),
        None => builder,
    }
}

/// Treat a 401 as "session no longer valid": clear it and return to the login
/// screen. Any other non-2xx becomes a plain error string.
fn check_status(resp: &Response, ctx: &str) -> ApiResult<()> {
    if resp.status() == 401 {
        auth::on_unauthorized();
    }
    if resp.ok() {
        Ok(())
    } else {
        Err(format!("{ctx} -> {}", resp.status()))
    }
}

async fn get_json<T: serde::de::DeserializeOwned>(path: &str) -> ApiResult<T> {
    let url = format!("{}{}", api_base(), path);
    let resp = with_auth(Request::get(&url))
        .send()
        .await
        .map_err(|e| format!("GET {path} -> {e}"))?;
    check_status(&resp, &format!("GET {path}"))?;
    resp.json::<T>()
        .await
        .map_err(|e| format!("GET {path} decode -> {e}"))
}

async fn post_json<B: serde::Serialize, T: serde::de::DeserializeOwned>(
    path: &str,
    body: &B,
) -> ApiResult<T> {
    let url = format!("{}{}", api_base(), path);
    let resp = with_auth(Request::post(&url))
        .json(body)
        .map_err(|e| format!("POST {path} encode -> {e}"))?
        .send()
        .await
        .map_err(|e| format!("POST {path} -> {e}"))?;
    check_status(&resp, &format!("POST {path}"))?;
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
    let resp = with_auth(Request::get(&url))
        .send()
        .await
        .map_err(|e| format!("GET {path} -> {e}"))?;
    check_status(&resp, &format!("GET {path}"))?;
    resp.text()
        .await
        .map_err(|e| format!("GET {path} text -> {e}"))
}

/// Read a workspace file's contents as raw bytes (issue #3: Track C
/// screenshots — the workspace proxy is auth-gated, so a plain `<img src>` URL
/// can't carry the bearer token; callers fetch bytes and build a `data:` URL).
pub async fn read_workspace_file_bytes(session_id: &str, path: &str) -> ApiResult<Vec<u8>> {
    let tail = encode_path(path.trim_start_matches('/'));
    let url = format!(
        "{}/v1/workspace/{}/{}",
        api_base(),
        urlencode(session_id),
        tail
    );
    let resp = with_auth(Request::get(&url))
        .send()
        .await
        .map_err(|e| format!("GET {path} -> {e}"))?;
    check_status(&resp, &format!("GET {path}"))?;
    resp.binary()
        .await
        .map_err(|e| format!("GET {path} bytes -> {e}"))
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
