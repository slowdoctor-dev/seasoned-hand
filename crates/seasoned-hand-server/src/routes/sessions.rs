//! Session read routes, workspace listing/proxy, cost snapshot, feature list, progress.
//! Moved from `lib.rs` (issue #43); pure code move.

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};

use seasoned_hand_core::agent::init::feature_list::FeatureList;
use seasoned_hand_core::agent::init::progress;
use seasoned_hand_core::auth::{Action, AuthContext};
use seasoned_hand_core::cost::CostSnapshot;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::error::{ApiError, api_err};
use crate::guards::{
    SESSION_TENANT_PREDICATE, authorize_in_handler, require_loopback, require_session_tenant,
};

#[derive(Debug, Deserialize, Default)]
pub(crate) struct ProgressQuery {
    pub(crate) lines: Option<usize>,
}

// Session response shapes are shared with the wasm UI via seasoned-hand-dto
// (story 6.3b). `SandboxInfo` is the dto `Sandbox`.
use seasoned_hand_dto::{Sandbox as SandboxInfo, SessionDetail, SessionState, SessionSummary};

#[derive(Debug, Deserialize, Default)]
pub struct SessionsListParams {
    pub limit: Option<usize>,
}

pub(crate) async fn list_sessions(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Query(params): Query<SessionsListParams>,
) -> Result<Json<Vec<SessionSummary>>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskRead, &auth_ctx)?;
    let limit = params.limit.unwrap_or(50).clamp(1, 500) as i64;
    let tenant_id = auth_ctx.tenant_id.clone();
    let sessions = state
        .db
        .with_conn(move |conn| -> rusqlite::Result<Vec<SessionSummary>> {
            // Issue #22: the previous filter matched `sessions.project_id IN
            // (SELECT id FROM tasks ...)` — overloading a project id against task
            // ids, so it returned the wrong set (and dropped chat-spawned sessions
            // whose tenancy comes from `task_id`). Use the canonical fail-closed
            // tenancy predicate shared with `require_session_tenant` (issue #22
            // review): every present parent must match, mismatched/orphan rows are
            // excluded. Binds the tenant TWICE (per `SESSION_TENANT_PREDICATE`).
            let mut stmt = conn.prepare(&format!(
                "SELECT s.id, s.created_at, s.updated_at, s.state, s.title, s.cost_cents, s.tool_calls \
                 FROM sessions s \
                 LEFT JOIN projects p ON p.id = s.project_id \
                 LEFT JOIN tasks t ON t.id = s.task_id \
                 WHERE {SESSION_TENANT_PREDICATE} \
                 ORDER BY s.updated_at DESC LIMIT ?"
            ))?;
            let rows = stmt.query_map(rusqlite::params![tenant_id, tenant_id, limit], |row| {
                let state_str: String = row.get(3)?;
                Ok(SessionSummary {
                    id: row.get(0)?,
                    created_at: row.get(1)?,
                    updated_at: row.get(2)?,
                    state: SessionState::from_db_str(&state_str).map_err(|_| {
                        rusqlite::Error::InvalidColumnType(
                            3,
                            "state".into(),
                            rusqlite::types::Type::Text,
                        )
                    })?,
                    title: row.get(4)?,
                    cost_cents: row.get(5)?,
                    tool_calls: row.get(6)?,
                })
            })?;
            rows.collect()
        })
        .await
        .map_err(|e| {
            tracing::error!(%e, "list_sessions db error");
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "db_error".into())
        })?;
    Ok(Json(sessions))
}

pub(crate) async fn get_session(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionDetail>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskRead, &auth_ctx)?;
    require_session_tenant(&state, &session_id, &auth_ctx).await?;
    let id_for_query = session_id.clone();
    let summary = state
        .db
        .with_conn(move |conn| -> rusqlite::Result<Option<SessionSummary>> {
            let mut stmt = conn.prepare(
                "SELECT id, created_at, updated_at, state, title, cost_cents, tool_calls \
                 FROM sessions WHERE id = ?",
            )?;
            let mut rows = stmt.query_map([id_for_query], |row| {
                let state_str: String = row.get(3)?;
                Ok(SessionSummary {
                    id: row.get(0)?,
                    created_at: row.get(1)?,
                    updated_at: row.get(2)?,
                    state: SessionState::from_db_str(&state_str).map_err(|_| {
                        rusqlite::Error::InvalidColumnType(
                            3,
                            "state".into(),
                            rusqlite::types::Type::Text,
                        )
                    })?,
                    title: row.get(4)?,
                    cost_cents: row.get(5)?,
                    tool_calls: row.get(6)?,
                })
            })?;
            match rows.next() {
                Some(row) => Ok(Some(row?)),
                None => Ok(None),
            }
        })
        .await
        .map_err(|e| {
            tracing::error!(%e, "get_session db error");
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "db_error".into())
        })?;

    let summary =
        summary.ok_or_else(|| api_err(StatusCode::NOT_FOUND, "session_not_found".into()))?;

    let sandbox = state.sandbox.get(&session_id).await.map(|h| SandboxInfo {
        api_url: h.api_url,
        novnc_url: h.novnc_url,
        ttyd_url: h.ttyd_url,
    });

    Ok(Json(SessionDetail { summary, sandbox }))
}

pub(crate) const WORKSPACE_FILE_CAP_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Serialize)]
pub(crate) struct WorkspaceEntry {
    name: String,
    #[serde(rename = "type")]
    kind: &'static str,
    size: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum WorkspaceResponse {
    Dir { entries: Vec<WorkspaceEntry> },
}

pub(crate) async fn workspace_root(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(session_id): Path<String>,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskRead, &auth_ctx)?;
    // P5-HARD-IT5-H6: tenant-scope before serving the workspace root.
    require_session_tenant(&state, &session_id, &auth_ctx).await?;
    workspace_proxy_inner(state, session_id, String::new()).await
}

pub(crate) async fn workspace_proxy(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path((session_id, sub_path)): Path<(String, String)>,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskRead, &auth_ctx)?;
    // P5-HARD-IT5-H6: tenant-scope before serving any sandbox file.
    require_session_tenant(&state, &session_id, &auth_ctx).await?;
    workspace_proxy_inner(state, session_id, sub_path).await
}

pub(crate) async fn workspace_proxy_inner(
    state: AppState,
    session_id: String,
    sub_path: String,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    use axum::http::header;
    use axum::response::Response;

    if sub_path.starts_with('/') || sub_path.split('/').any(|seg| seg == "..") {
        return Err(api_err(StatusCode::BAD_REQUEST, "path_traversal".into()));
    }

    let Some(handle) = state.sandbox.get(&session_id).await else {
        return Err(api_err(
            StatusCode::NOT_FOUND,
            "no_sandbox_for_session".into(),
        ));
    };

    let target = if sub_path.is_empty() {
        handle.workspace_host_path.clone()
    } else {
        handle.workspace_host_path.join(&sub_path)
    };

    // SEC-IT4-M2: the `..`/leading-slash guard above only inspects the request
    // path, not on-disk symlinks. Untrusted sandbox code can plant a symlink
    // inside the bind-mounted workspace (`ln -s /etc/passwd leak`); the
    // metadata/read calls below follow symlinks, so without this the owning
    // tenant could read arbitrary host files through the proxy. Resolve the
    // real path and require it to stay inside the (canonicalized) workspace
    // root before touching the filesystem.
    let canonical_root = tokio::fs::canonicalize(&handle.workspace_host_path)
        .await
        .map_err(|_e| {
            api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "workspace_root_unavailable".into(),
            )
        })?;
    let target = tokio::fs::canonicalize(&target)
        .await
        .map_err(|_e| api_err(StatusCode::NOT_FOUND, "workspace_not_found".into()))?;
    if !target.starts_with(&canonical_root) {
        return Err(api_err(StatusCode::BAD_REQUEST, "path_traversal".into()));
    }

    let metadata = tokio::fs::metadata(&target)
        .await
        .map_err(|_e| api_err(StatusCode::NOT_FOUND, "workspace_not_found".into()))?;

    if metadata.is_dir() {
        let mut entries = Vec::new();
        let mut read_dir = tokio::fs::read_dir(&target).await.map_err(|_e| {
            api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "workspace_readdir_failed".into(),
            )
        })?;
        while let Ok(Some(entry)) = read_dir.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(entry_md) = entry.metadata().await else {
                continue;
            };
            let (kind, size) = if entry_md.is_dir() {
                ("dir", None)
            } else {
                ("file", Some(entry_md.len()))
            };
            entries.push(WorkspaceEntry { name, kind, size });
        }
        entries.sort_by(|a, b| match (a.kind, b.kind) {
            ("dir", "file") => std::cmp::Ordering::Less,
            ("file", "dir") => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        });
        let body = serde_json::to_vec(&WorkspaceResponse::Dir { entries }).unwrap_or_default();
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(body))
            .unwrap_or_else(|_| Response::new(axum::body::Body::empty())));
    }

    if metadata.len() > WORKSPACE_FILE_CAP_BYTES {
        tracing::warn!(
            bytes = metadata.len(),
            cap = WORKSPACE_FILE_CAP_BYTES,
            "workspace file exceeds response cap"
        );
        return Err(api_err(
            StatusCode::PAYLOAD_TOO_LARGE,
            "workspace_file_too_large".into(),
        ));
    }

    let bytes = tokio::fs::read(&target).await.map_err(|_e| {
        api_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "workspace_read_failed".into(),
        )
    })?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(axum::body::Body::from(bytes))
        .unwrap_or_else(|_| Response::new(axum::body::Body::empty())))
}

pub(crate) async fn cost_snapshot(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
) -> Result<Json<CostSnapshot>, (StatusCode, Json<ApiError>)> {
    // Issue #21: the cost snapshot is GLOBAL (not tenant-scoped); restrict it to
    // loopback (host/ops) callers rather than leaving it unauthenticated.
    require_loopback(remote)?;
    match state.cost.snapshot().await {
        Ok(snapshot) => Ok(Json(snapshot)),
        Err(error) => {
            tracing::warn!(%error, "cost snapshot proxy failed");
            Err(api_err(
                StatusCode::SERVICE_UNAVAILABLE,
                "cost_unavailable".into(),
            ))
        }
    }
}

pub(crate) async fn get_feature_list(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(session_id): Path<String>,
) -> Result<Json<FeatureList>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskRead, &auth_ctx)?;
    require_session_tenant(&state, &session_id, &auth_ctx).await?;
    let bytes = state
        .sandbox
        .read_workspace_file(&session_id, "feature-list.json")
        .await
        .map_err(|error| {
            tracing::warn!(
                session_id = %session_id,
                %error,
                "feature-list.json read failed (returning 404)",
            );
            api_err(StatusCode::NOT_FOUND, "feature_list_not_found".into())
        })?;
    let parsed = serde_json::from_slice::<FeatureList>(&bytes).map_err(|error| {
        tracing::warn!(
            session_id = %session_id,
            line = error.line(),
            column = error.column(),
            %error,
            "feature-list.json parse failed (returning 500)",
        );
        api_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "feature_list_invalid".into(),
        )
    })?;
    Ok(Json(parsed))
}

pub(crate) async fn get_progress(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(session_id): Path<String>,
    Query(q): Query<ProgressQuery>,
) -> Result<String, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskRead, &auth_ctx)?;
    require_session_tenant(&state, &session_id, &auth_ctx).await?;
    let bytes = state
        .sandbox
        .read_workspace_file(&session_id, "progress.txt")
        .await
        .map_err(|error| {
            tracing::warn!(
                session_id = %session_id,
                %error,
                "progress.txt read failed (returning 404)",
            );
            api_err(StatusCode::NOT_FOUND, "progress_not_found".into())
        })?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(progress::tail_lines(&text, q.lines.unwrap_or(200)))
}
