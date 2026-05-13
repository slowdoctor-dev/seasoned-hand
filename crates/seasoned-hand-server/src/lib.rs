//! Seasoned Hand HTTP server.
//! refs: /specs/phase-0/architecture.md §4.1

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use dashmap::DashMap;
use seasoned_hand_core::agent::breaker::BreakerRegistry;
use seasoned_hand_core::agent::init::feature_list::FeatureList;
use seasoned_hand_core::agent::init::progress;
use seasoned_hand_core::agent::narrate::NarratorHook;
use seasoned_hand_core::agent::{AgentRunner, AgentRunnerDeps};
use seasoned_hand_core::browser::tracks::PostBrowserActionHook;
use seasoned_hand_core::capability::ModelCapabilities;
use seasoned_hand_core::cost::{CostClient, CostSnapshot};
use seasoned_hand_core::db::DbPool;
use seasoned_hand_core::dispatch::mask::DefaultMaskPolicy;
use seasoned_hand_core::dispatch::{
    ToolDispatcher,
    hooks::{EventEmittingHook, InvalidationHook},
};
use seasoned_hand_core::events::{EventQuery, EventStore, EventType, sqlite::SqliteEventStore};
use seasoned_hand_core::llm::LlmClient;
use seasoned_hand_core::plan::PlanManager;
use seasoned_hand_core::pubsub::RedisPool;
use seasoned_hand_core::router::{SlotName, SlotRouter};
use seasoned_hand_core::sandbox::SandboxClient;
use seasoned_hand_core::search::SearchClient;
use seasoned_hand_core::tools::register_builtin_tools;
use seasoned_hand_core::verifier::{
    VerificationStore,
    routes::{ListQuery as VerifyListQuery, get_verification, list_verifications},
};
use serde::{Deserialize, Serialize};

pub mod ws;

#[derive(Clone)]
pub struct AppState {
    pub db: DbPool,
    pub redis: RedisPool,
    pub events: Arc<SqliteEventStore>,
    pub sandbox: Arc<SandboxClient>,
    pub search: Arc<SearchClient>,
    pub dispatcher: Arc<ToolDispatcher>,
    pub router: Arc<SlotRouter>,
    pub capabilities: Arc<HashMap<String, ModelCapabilities>>,
    pub cost: Arc<CostClient>,
    pub plan_manager: Arc<PlanManager>,
    pub runner: Arc<AgentRunner>,
    /// Story 1.8: copy of `SlotRouter::verifier_enabled()` snapshotted at
    /// `AppState::new` time. Story 1.9's Verifier Worker reads this to
    /// decide whether to spawn.
    pub verifier_enabled: bool,
    /// Story 1.9: FAIL-biased verifier system prompt loaded from
    /// `config/prompts/verifier.system.txt` at boot when
    /// `verifier_enabled` is true; empty string when verifier is
    /// disabled.
    pub verifier_system_prompt: Arc<String>,
    /// Story 1.9: persistence handle for the `verifications` table.
    pub verifications: Arc<VerificationStore>,
    /// Story 1.13: in-memory one-shot label slot consumed by the next
    /// `Plan{op:"advance"}` checkpoint. Written by the `checkpoint_label`
    /// LLM tool, read+cleared by the `CheckpointManager`.
    pub checkpoint_labels: Arc<seasoned_hand_core::checkpoint::CheckpointLabelBuffer>,
    /// Story 1.13: persistence handle for the `checkpoints` table.
    pub checkpoints: Arc<seasoned_hand_core::checkpoint::CheckpointStore>,
    /// Story 1.13b: admin token from `SEASONED_HAND_ADMIN_TOKEN` env.
    /// Empty when unset — the admin rollback route fails with
    /// `503 admin_token_not_configured` instead of allowing
    /// unauthenticated access (PRINCIPLE #10: fail visibly).
    pub admin_token: Arc<String>,
    /// Story 1.13b: opt-in flag that lets the VerifierGate trigger a
    /// checkpoint rollback when a verdict carries
    /// `rollback_required: true`. Default `false` per phase-1/DEBT.md #3
    /// — Phase 2 retrospective will decide whether to flip this.
    pub checkpoint_rollback_on_verifier_fail: bool,
    /// Story 1.17: per-session cancellation tokens used by ws task_cancel.
    pub cancel_tokens: Arc<DashMap<String, tokio_util::sync::CancellationToken>>,
    pub breakers: Arc<BreakerRegistry>,
}

impl AppState {
    pub fn new(
        db: DbPool,
        redis: RedisPool,
        sandbox: SandboxClient,
        search: SearchClient,
        router: SlotRouter,
        capabilities: HashMap<String, ModelCapabilities>,
    ) -> Self {
        let events = Arc::new(SqliteEventStore::with_redis(db.clone(), redis.clone()));
        let sandbox = Arc::new(sandbox);
        let search = Arc::new(search);
        let redis_arc = Arc::new(redis.clone());
        let dispatcher = Arc::new(
            ToolDispatcher::new(register_builtin_tools())
                // Story 1.15: NarratorHook runs first so the
                // `Message{ui:"narrate"}` event lands before the Action
                // event for clean UI ordering. Templated-only at boot;
                // classifier-slot LLM path is opt-in (see
                // story-1.15.md execution notes — deferred plumbing).
                .with_hook(Arc::new(NarratorHook::new(events.clone())))
                .with_hook(Arc::new(EventEmittingHook::new(events.clone())))
                .with_hook(Arc::new(InvalidationHook::new(
                    events.clone(),
                    Some(redis_arc.clone()),
                )))
                .with_hook(Arc::new(PostBrowserActionHook::new(events.clone()))),
        );
        let verifier_enabled = router.verifier_enabled();
        let verifications = Arc::new(VerificationStore::new(db.clone()));
        let checkpoint_labels =
            Arc::new(seasoned_hand_core::checkpoint::CheckpointLabelBuffer::new());
        let checkpoints = Arc::new(seasoned_hand_core::checkpoint::CheckpointStore::new(
            db.clone(),
        ));
        // Story 1.13b: admin_token / rollback flag default empty/false;
        // production main.rs reads them from env and calls the
        // builder methods. Tests can do the same without touching
        // process-wide environment variables.
        let admin_token = Arc::new(String::new());
        let checkpoint_rollback_on_verifier_fail = false;
        let cancel_tokens = Arc::new(DashMap::new());
        let breakers = Arc::new(BreakerRegistry::new());
        let router = Arc::new(router);
        let plan_manager = Arc::new(PlanManager::new(db.clone(), events.clone()));
        let main_slot = router.resolve(SlotName::Main);
        let llm = LlmClient::new(main_slot.base_url.clone(), main_slot.api_key.clone());
        let cost = Arc::new(CostClient::new(main_slot.base_url.clone()));
        let runner = Arc::new(AgentRunner::new(AgentRunnerDeps {
            llm,
            dispatcher: dispatcher.clone(),
            events: events.clone(),
            router: router.clone(),
            sandbox: sandbox.clone(),
            search: search.clone(),
            cost: cost.clone(),
            sessions: db.clone(),
            plan_manager: plan_manager.clone(),
            mask_policy: Arc::new(DefaultMaskPolicy),
            checkpoint_labels: checkpoint_labels.clone(),
            checkpoints: checkpoints.clone(),
            redis: redis_arc.clone(),
            breakers: breakers.clone(),
            cancel_tokens: cancel_tokens.clone(),
        }));
        Self {
            db,
            redis,
            events,
            sandbox,
            search,
            dispatcher,
            router,
            capabilities: Arc::new(capabilities),
            cost,
            plan_manager,
            runner,
            verifier_enabled,
            verifier_system_prompt: Arc::new(String::new()),
            verifications,
            checkpoint_labels,
            checkpoints,
            admin_token,
            checkpoint_rollback_on_verifier_fail,
            cancel_tokens,
            breakers,
        }
    }

    /// Story 1.9: replace the (default-empty) verifier system prompt
    /// with content loaded from `config/prompts/verifier.system.txt` at
    /// server bootstrap. Main.rs is the canonical caller; tests can
    /// skip this (they never exercise the verifier loop).
    pub fn with_verifier_prompt(mut self, prompt: Arc<String>) -> Self {
        self.verifier_system_prompt = prompt;
        self
    }

    /// Story 1.13b: set the admin token for the rollback endpoint.
    /// Empty string keeps the endpoint disabled (returns 503). Main.rs
    /// reads from `SEASONED_HAND_ADMIN_TOKEN`; tests construct
    /// explicitly to avoid racing on process env vars.
    pub fn with_admin_token(mut self, token: impl Into<String>) -> Self {
        self.admin_token = Arc::new(token.into());
        self
    }

    /// Story 1.13b: enable the opt-in Verifier-driven rollback path.
    /// Defaults `false` per phase-1/DEBT.md #3.
    pub fn with_rollback_on_verifier_fail(mut self, enabled: bool) -> Self {
        self.checkpoint_rollback_on_verifier_fail = enabled;
        self
    }
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
    db: String,
    redis: String,
}

async fn healthz(State(state): State<AppState>) -> impl IntoResponse {
    let db_ok = state
        .db
        .with_conn(|conn| conn.prepare("SELECT 1").is_ok())
        .await;
    let redis_ok = state.redis.ping().await.is_ok();

    let (status_code, status_text) = if db_ok && redis_ok {
        (StatusCode::OK, "ok")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "degraded")
    };
    (
        status_code,
        Json(Health {
            status: status_text,
            version: seasoned_hand_core::version(),
            db: if db_ok { "ok" } else { "unreachable" }.into(),
            redis: if redis_ok { "ok" } else { "unreachable" }.into(),
        }),
    )
}

#[derive(Debug, Deserialize, Default)]
pub struct EventsQueryParams {
    pub after_id: Option<i64>,
    #[serde(rename = "type")]
    pub event_type: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Serialize)]
struct ApiError {
    error: String,
}

#[derive(Debug, Deserialize, Default)]
struct ProgressQuery {
    lines: Option<usize>,
}

async fn list_events(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(params): Query<EventsQueryParams>,
) -> Result<Json<Vec<seasoned_hand_core::events::Event>>, (StatusCode, Json<ApiError>)> {
    let event_type = match params.event_type.as_deref() {
        Some(s) => Some(EventType::from_str(s).map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiError {
                    error: format!("unknown event type: {s}"),
                }),
            )
        })?),
        None => None,
    };

    let filter = EventQuery {
        after_id: params.after_id,
        event_type,
        limit: params.limit,
    };

    let session_exists = state
        .db
        .with_conn({
            let session_id = session_id.clone();
            move |conn| {
                conn.query_row::<i64, _, _>(
                    "SELECT 1 FROM sessions WHERE id = ?",
                    [&session_id],
                    |row| row.get(0),
                )
                .is_ok()
            }
        })
        .await;
    if !session_exists {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: "session_not_found".into(),
            }),
        ));
    }

    match state.events.query(&session_id, filter).await {
        Ok(events) => Ok(Json(events)),
        Err(seasoned_hand_core::events::EventError::SessionNotFound(_)) => Err((
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: "session_not_found".into(),
            }),
        )),
        Err(other) => {
            tracing::error!(error = %other, "events query failed");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: "internal_error".into(),
                }),
            ))
        }
    }
}

#[derive(Debug, Serialize)]
struct SessionSummary {
    id: String,
    created_at: i64,
    updated_at: i64,
    state: String,
    title: Option<String>,
    cost_cents: i64,
    tool_calls: i64,
}

#[derive(Debug, Serialize)]
struct SandboxInfo {
    api_url: String,
    novnc_url: String,
    ttyd_url: String,
}

#[derive(Debug, Serialize)]
struct SessionDetail {
    #[serde(flatten)]
    summary: SessionSummary,
    sandbox: Option<SandboxInfo>,
}

#[derive(Debug, Deserialize, Default)]
pub struct SessionsListParams {
    pub limit: Option<usize>,
}

async fn list_sessions(
    State(state): State<AppState>,
    Query(params): Query<SessionsListParams>,
) -> Result<Json<Vec<SessionSummary>>, (StatusCode, Json<ApiError>)> {
    let limit = params.limit.unwrap_or(50).clamp(1, 500) as i64;
    let sessions = state
        .db
        .with_conn(move |conn| -> rusqlite::Result<Vec<SessionSummary>> {
            let mut stmt = conn.prepare(
                "SELECT id, created_at, updated_at, state, title, cost_cents, tool_calls \
                 FROM sessions ORDER BY updated_at DESC LIMIT ?",
            )?;
            let rows = stmt.query_map([limit], |row| {
                Ok(SessionSummary {
                    id: row.get(0)?,
                    created_at: row.get(1)?,
                    updated_at: row.get(2)?,
                    state: row.get(3)?,
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
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: "db_error".into(),
                }),
            )
        })?;
    Ok(Json(sessions))
}

async fn get_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionDetail>, (StatusCode, Json<ApiError>)> {
    let id_for_query = session_id.clone();
    let summary = state
        .db
        .with_conn(move |conn| -> rusqlite::Result<Option<SessionSummary>> {
            let mut stmt = conn.prepare(
                "SELECT id, created_at, updated_at, state, title, cost_cents, tool_calls \
                 FROM sessions WHERE id = ?",
            )?;
            let mut rows = stmt.query_map([id_for_query], |row| {
                Ok(SessionSummary {
                    id: row.get(0)?,
                    created_at: row.get(1)?,
                    updated_at: row.get(2)?,
                    state: row.get(3)?,
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
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: "db_error".into(),
                }),
            )
        })?;

    let summary = summary.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: "session_not_found".into(),
            }),
        )
    })?;

    let sandbox = state.sandbox.get(&session_id).await.map(|h| SandboxInfo {
        api_url: h.api_url,
        novnc_url: h.novnc_url,
        ttyd_url: h.ttyd_url,
    });

    Ok(Json(SessionDetail { summary, sandbox }))
}

const WORKSPACE_FILE_CAP_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Serialize)]
struct WorkspaceEntry {
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

async fn workspace_root(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    workspace_proxy_inner(state, session_id, String::new()).await
}

async fn workspace_proxy(
    State(state): State<AppState>,
    Path((session_id, sub_path)): Path<(String, String)>,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    workspace_proxy_inner(state, session_id, sub_path).await
}

async fn workspace_proxy_inner(
    state: AppState,
    session_id: String,
    sub_path: String,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    use axum::http::header;
    use axum::response::Response;

    if sub_path.starts_with('/') || sub_path.split('/').any(|seg| seg == "..") {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiError {
                error: "path_traversal".into(),
            }),
        ));
    }

    let Some(handle) = state.sandbox.get(&session_id).await else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: "no_sandbox_for_session".into(),
            }),
        ));
    };

    let target = if sub_path.is_empty() {
        handle.workspace_host_path.clone()
    } else {
        handle.workspace_host_path.join(&sub_path)
    };

    let metadata = tokio::fs::metadata(&target).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: format!("not_found: {e}"),
            }),
        )
    })?;

    if metadata.is_dir() {
        let mut entries = Vec::new();
        let mut read_dir = tokio::fs::read_dir(&target).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: format!("readdir: {e}"),
                }),
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
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(ApiError {
                error: format!(
                    "file_too_large: {} bytes (cap {WORKSPACE_FILE_CAP_BYTES})",
                    metadata.len()
                ),
            }),
        ));
    }

    let bytes = tokio::fs::read(&target).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: format!("read: {e}"),
            }),
        )
    })?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(axum::body::Body::from(bytes))
        .unwrap_or_else(|_| Response::new(axum::body::Body::empty())))
}

async fn cost_snapshot(
    State(state): State<AppState>,
) -> Result<Json<CostSnapshot>, (StatusCode, Json<ApiError>)> {
    match state.cost.snapshot().await {
        Ok(snapshot) => Ok(Json(snapshot)),
        Err(error) => {
            tracing::warn!(%error, "cost snapshot proxy failed");
            Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ApiError {
                    error: "cost_unavailable".into(),
                }),
            ))
        }
    }
}

async fn get_feature_list(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<FeatureList>, (StatusCode, Json<ApiError>)> {
    let bytes = state
        .sandbox
        .read_workspace_file(&session_id, "feature-list.json")
        .await
        .map_err(|_| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiError {
                    error: "feature_list_not_found".into(),
                }),
            )
        })?;
    let parsed = serde_json::from_slice::<FeatureList>(&bytes).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: "feature_list_invalid".into(),
            }),
        )
    })?;
    Ok(Json(parsed))
}

async fn get_progress(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(q): Query<ProgressQuery>,
) -> Result<String, (StatusCode, Json<ApiError>)> {
    let bytes = state
        .sandbox
        .read_workspace_file(&session_id, "progress.txt")
        .await
        .map_err(|_| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiError {
                    error: "progress_not_found".into(),
                }),
            )
        })?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(progress::tail_lines(&text, q.lines.unwrap_or(200)))
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/ws", get(ws::ws_upgrade))
        .route("/v1/cost", get(cost_snapshot))
        .route("/v1/sessions", get(list_sessions))
        .route("/v1/sessions/:id", get(get_session))
        .route("/v1/sessions/:id/events", get(list_events))
        .route("/v1/sessions/:id/feature-list", get(get_feature_list))
        .route("/v1/sessions/:id/progress", get(get_progress))
        .route("/v1/workspace/:session_id/*sub_path", get(workspace_proxy))
        .route("/v1/workspace/:session_id", get(workspace_root))
        .route("/v1/workspace/:session_id/", get(workspace_root))
        .route(
            "/v1/sessions/:id/verifications",
            get(list_verifications_handler),
        )
        .route("/v1/verifications/:id", get(get_verification_handler))
        .route(
            "/v1/sessions/:id/checkpoints",
            get(list_checkpoints_handler),
        )
        .route(
            "/v1/sessions/:id/checkpoints/:checkpoint_id/rollback",
            axum::routing::post(post_checkpoint_rollback_handler),
        )
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Story 1.13b: admin rollback handler. Loopback-bound, token-gated.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RollbackBody {
    reason: String,
}

#[derive(Debug, Serialize)]
struct RollbackResponse {
    checkpoint_id: String,
    rolled_back_at: i64,
}

async fn post_checkpoint_rollback_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    headers: axum::http::HeaderMap,
    Path((session_id, checkpoint_id)): Path<(String, String)>,
    Json(body): Json<RollbackBody>,
) -> Result<(StatusCode, Json<RollbackResponse>), (StatusCode, Json<ApiError>)> {
    // Guard 1: admin token must be configured at boot.
    if state.admin_token.is_empty() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiError {
                error: "admin_token_not_configured".into(),
            }),
        ));
    }
    // Guard 2: loopback only.
    if !remote.ip().is_loopback() {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ApiError {
                error: "forbidden_non_loopback".into(),
            }),
        ));
    }
    // Guard 3: token header match.
    let token_hdr = headers
        .get("X-Seasoned-Hand-Admin-Token")
        .and_then(|h| h.to_str().ok());
    if token_hdr != Some(state.admin_token.as_str()) {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiError {
                error: "unauthorized_token".into(),
            }),
        ));
    }
    // Guard 4: reason length.
    if body.reason.len() > 200 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiError {
                error: "reason_too_long".into(),
            }),
        ));
    }

    // Guard 5: session state must NOT be RUNNING or VERIFYING.
    let session_state = state
        .db
        .with_conn({
            let sid = session_id.clone();
            move |conn| -> rusqlite::Result<Option<String>> {
                let mut stmt = conn.prepare("SELECT state FROM sessions WHERE id = ?")?;
                let mut rows =
                    stmt.query_map(rusqlite::params![sid], |row| row.get::<_, String>(0))?;
                match rows.next() {
                    Some(r) => Ok(Some(r?)),
                    None => Ok(None),
                }
            }
        })
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "rollback: session state query");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: "internal_error".into(),
                }),
            )
        })?;
    match session_state.as_deref() {
        Some("RUNNING") | Some("VERIFYING") => {
            return Err((
                StatusCode::CONFLICT,
                Json(ApiError {
                    error: "wrong_state".into(),
                }),
            ));
        }
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiError {
                    error: "session_not_found".into(),
                }),
            ));
        }
        _ => {}
    }

    // Guard 6: sandbox must not be paused.
    let paused = state.sandbox.is_paused(&session_id).await.map_err(|e| {
        tracing::warn!(error = %e, "rollback: sandbox paused-state probe failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: "internal_error".into(),
            }),
        )
    })?;
    if paused {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiError {
                error: "sandbox_paused".into(),
            }),
        ));
    }

    // All guards passed — dispatch the internal tool. The mask layer
    // affects only what's exposed to the LLM, so direct dispatch works.
    let ctx = seasoned_hand_core::tools::ToolContext {
        session_id: session_id.clone(),
        mask_mode: seasoned_hand_core::dispatch::mask::AgentMode::Internal,
        events: state.events.clone(),
        sandbox: state.sandbox.clone(),
        search: state.search.clone(),
        plan_manager: state.plan_manager.clone(),
        checkpoint_labels: state.checkpoint_labels.clone(),
        checkpoints: state.checkpoints.clone(),
    };
    let out = state
        .dispatcher
        .dispatch(
            &ctx,
            "checkpoint_rollback",
            serde_json::json!({
                "checkpoint_id": checkpoint_id,
                "reason": body.reason,
                "rolled_back_by": "admin:cli",
            }),
        )
        .await;
    if !out.ok {
        let err_kind = out
            .error
            .as_ref()
            .map(|e| e.kind.clone())
            .unwrap_or_else(|| "tool_error".to_string());
        let status = match err_kind.as_str() {
            "checkpoint_not_found" => StatusCode::NOT_FOUND,
            "reason_too_long" => StatusCode::BAD_REQUEST,
            "revert_failed" => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        return Err((status, Json(ApiError { error: err_kind })));
    }
    let rolled_back_at = out
        .output
        .get("rolled_back_at")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    Ok((
        StatusCode::ACCEPTED,
        Json(RollbackResponse {
            checkpoint_id,
            rolled_back_at,
        }),
    ))
}

// ---------------------------------------------------------------------------
// Story 1.13: checkpoints list HTTP handler.
// ---------------------------------------------------------------------------

/// Translate the shared `RouteOutcome<T>` into an axum response. `label`
/// is logged on the Internal arm so the access log carries which route
/// failed. The Ok arm hand-rolls the Response so a serde failure doesn't
/// panic the request (we fall back to an empty body — the caller will
/// see a 200 with no JSON, which is preferable to a 500 from a panic
/// during error rendering).
fn render_outcome<T: serde::Serialize>(
    label: &'static str,
    outcome: seasoned_hand_core::routes::RouteOutcome<T>,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    use axum::http::header;
    use axum::response::Response;
    use seasoned_hand_core::routes::RouteOutcome;
    match outcome {
        RouteOutcome::Ok(body) => {
            let bytes = serde_json::to_vec(&body).unwrap_or_default();
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/json")
                .body(axum::body::Body::from(bytes))
                .unwrap_or_else(|_| Response::new(axum::body::Body::empty())))
        }
        RouteOutcome::NotFound(msg) => Err((StatusCode::NOT_FOUND, Json(ApiError { error: msg }))),
        RouteOutcome::Internal(msg) => {
            tracing::error!(error = %msg, route = label, "route failed");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: "internal_error".into(),
                }),
            ))
        }
    }
}

async fn list_checkpoints_handler(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(q): Query<seasoned_hand_core::checkpoint::routes::ListQuery>,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    use seasoned_hand_core::checkpoint::routes::list_checkpoints;
    render_outcome(
        "list_checkpoints",
        list_checkpoints(&state.checkpoints, &session_id, q).await,
    )
}

// ---------------------------------------------------------------------------
// Story 1.9: verifier HTTP route handlers — thin axum wrappers over the
// pure RouteOutcome layer in seasoned_hand_core::verifier::routes.
// ---------------------------------------------------------------------------

async fn list_verifications_handler(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(q): Query<VerifyListQuery>,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    render_outcome(
        "list_verifications",
        list_verifications(&state.verifications, &session_id, q).await,
    )
}

async fn get_verification_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    render_outcome(
        "get_verification",
        get_verification(&state.verifications, &id).await,
    )
}
