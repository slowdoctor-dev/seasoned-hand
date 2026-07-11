//! Seasoned Hand HTTP server.
//! refs: /specs/phase-0/architecture.md §4.1

use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::timeout::TimeoutLayer;

use seasoned_hand_core::auth::Action;
// (DeliverableStore imported via the broader `deliverable::` use below.)
use serde::Serialize;

mod auth;
// Issue #22 (batch F) + issue #43: HTTP error machinery, route guards, and the
// AppState wiring are extracted from this file. The remaining decomposition
// (per-domain route modules) is tracked in #43.
mod error;
mod guards;
pub mod initializer_spawner;
mod routes;
mod state;
pub mod ws;

use guards::{public, require_session_tenant, require_task_tenant, self_gated, with_auth};

pub use initializer_spawner::WsInitializerSpawner;
pub use state::{AppState, EmailChannelEnv, NarratorClassifierWiring};

// Glob-import the per-domain route modules so `app()`'s wiring (and the inline
// handler-level tests) keep referring to handlers by their unqualified names.
use routes::admin::*;
use routes::auth_routes::*;
use routes::channels::*;
use routes::events::*;
use routes::intake::*;
use routes::org::*;
use routes::projects::*;
use routes::sessions::*;
use routes::verifications::*;

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

/// Issue #22: per-request timeout for normal routes (excludes `/ws` + the CLI
/// long-poll). Generous enough for legitimate sandbox/DB work, but bounds a hung
/// handler from holding a connection indefinitely.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Issue #22: explicit request body cap (vs axum's silent 2 MB default). Intake
/// payloads are small JSON; 1 MiB is generous while bounding abuse.
const MAX_REQUEST_BODY_BYTES: usize = 1024 * 1024;

pub fn app(state: AppState) -> Router {
    // Issue #33: when SH_UI_DIST is configured, the built Dioxus bundle is served
    // as the router fallback. Cloned out before `with_state` consumes `state`.
    let ui_dist = state.ui_dist.clone();
    let router = Router::new()
        .route("/healthz", public(get(healthz)))
        // Issue #7 / ADR-018: auth endpoints — login is public (mints the first
        // credential); dev-login self-gates on loopback + flag.
        .route(
            "/v1/auth/login",
            public(axum::routing::post(post_auth_login_handler)),
        )
        .route(
            "/v1/auth/dev-login",
            self_gated(axum::routing::post(post_auth_dev_login_handler)),
        )
        // `/ws` is registered AFTER the TimeoutLayer below (issue #22) so the
        // long-lived WebSocket is excluded from the per-request timeout.
        .route("/v1/cost", self_gated(get(cost_snapshot)))
        .route(
            "/v1/sessions",
            with_auth(get(list_sessions), Action::TaskRead),
        )
        .route(
            "/v1/sessions/:id",
            with_auth(get(get_session), Action::TaskRead),
        )
        .route(
            "/v1/sessions/:id/events",
            with_auth(get(list_events), Action::TaskRead),
        )
        // Story 5.16: tenant-visible redacted event feed. Routes through
        // the auth middleware (any authenticated role) but no `Action`
        // gate — the tenant + visibility predicates inside
        // `visibility::query` ARE the gate.
        .route(
            "/v1/events/:session_id",
            with_auth(get(list_redacted_events), Action::TaskRead),
        )
        // Story 5.16: admin-only forensic raw-event read. Emits an
        // audit_log row per call (`Action::EventRawRead`).
        .route(
            "/v1/admin/events/:session_id/raw",
            with_auth(get(list_raw_events_admin), Action::EventRawRead),
        )
        .route(
            "/v1/sessions/:id/feature-list",
            with_auth(get(get_feature_list), Action::TaskRead),
        )
        .route(
            "/v1/sessions/:id/progress",
            with_auth(get(get_progress), Action::TaskRead),
        )
        // P5-HARD-IT5-H6: the workspace proxy serves raw sandbox files
        // (source, outputs, secrets) by session_id — the richest leak
        // surface. Previously loopback-only with NO auth/tenant check.
        // Now RBAC-gated + tenant-scoped (require_session_tenant in the
        // handler) so a tenant-A caller can't read tenant-B's sandbox.
        .route(
            "/v1/workspace/:session_id/*sub_path",
            with_auth(get(workspace_proxy), Action::TaskRead),
        )
        .route(
            "/v1/workspace/:session_id",
            with_auth(get(workspace_root), Action::TaskRead),
        )
        .route(
            "/v1/workspace/:session_id/",
            with_auth(get(workspace_root), Action::TaskRead),
        )
        .route(
            "/v1/sessions/:id/verifications",
            with_auth(get(list_verifications_handler), Action::TaskRead),
        )
        .route(
            "/v1/verifications/:id",
            with_auth(get(get_verification_handler), Action::TaskRead),
        )
        .route(
            "/v1/sessions/:id/checkpoints",
            with_auth(get(list_checkpoints_handler), Action::TaskRead),
        )
        .route(
            "/v1/sessions/:id/checkpoints/:checkpoint_id/rollback",
            with_auth(
                axum::routing::post(post_checkpoint_rollback_handler),
                Action::TaskWrite,
            ),
        )
        // Story 2.17 / Phase 0 DEBT #16: admin-token-gated manual
        // workspace cleanup. Same 3-guard pattern as the rollback
        // route above (configured-token / loopback / header match).
        .route(
            "/v1/admin/sandbox/cleanup",
            self_gated(axum::routing::post(post_admin_sandbox_cleanup_handler)),
        )
        // Story 2.5: channel introspection.
        .route("/v1/channels", self_gated(get(list_channels_handler)))
        .route(
            "/v1/channels/:name/health",
            self_gated(get(get_channel_health_handler)),
        )
        .route(
            "/v1/channels/:name/test",
            self_gated(axum::routing::post(post_channel_test_handler)),
        )
        // Story 2.10: WebhookChannel intake source — HTTP POST is the
        // long-lived listener (the channel's `IntakeProvider::run` is
        // a no-op and parks on shutdown, see channel/webhook/mod.rs).
        .route(
            "/v1/intake/webhook",
            self_gated(axum::routing::post(post_intake_webhook_handler)),
        )
        // Story 2.15: per-task provenance manifest. Returns the latest
        // deliverable's manifest by default, or a specific deliverable's
        // when `?deliverable_id=...` is supplied. Spilled (file-ref)
        // manifests are transparently inflated.
        .route(
            "/v1/tasks/:id/provenance",
            with_auth(get(get_task_provenance_handler), Action::TaskRead),
        )
        // Story 2.21a: project + task surface for the `seasoned-hand`
        // CLI binary. Loopback-only (Phase 2 single-operator); Phase 5
        // multi-user will lift the constraint behind real auth.
        .route(
            "/v1/projects",
            with_auth(get(list_projects_handler), Action::TaskRead),
        )
        .route(
            "/v1/projects",
            with_auth(
                axum::routing::post(create_project_handler),
                Action::TaskWrite,
            ),
        )
        .route(
            "/v1/projects/:id/archive",
            with_auth(
                axum::routing::post(archive_project_handler),
                Action::TaskWrite,
            ),
        )
        .route(
            "/v1/projects/:id/tasks",
            with_auth(get(list_project_tasks_handler), Action::TaskRead),
        )
        .route(
            "/v1/tasks/:id",
            with_auth(get(get_task_handler), Action::TaskRead),
        )
        .route(
            "/v1/tasks/:id/deliverables",
            with_auth(get(list_task_deliverables_handler), Action::TaskRead),
        )
        .route(
            "/v1/tasks/:id/pause",
            with_auth(
                axum::routing::post(post_task_pause_handler),
                Action::TaskWrite,
            ),
        )
        .route(
            "/v1/tasks/:id/resume",
            with_auth(
                axum::routing::post(post_task_resume_handler),
                Action::TaskWrite,
            ),
        )
        .route(
            "/v1/tasks/:id/cancel",
            with_auth(
                axum::routing::post(post_task_cancel_handler),
                Action::TaskWrite,
            ),
        )
        .route(
            "/v1/tasks/:id/handoff",
            with_auth(
                axum::routing::post(post_task_handoff_handler),
                Action::TaskHandoff,
            ),
        )
        .route(
            "/v1/tasks/:id/handoff/can",
            with_auth(get(get_task_handoff_can_handler), Action::TaskHandoff),
        )
        .route(
            "/v1/audit",
            with_auth(get(list_audit_handler), Action::AuditRead),
        )
        .route(
            "/v1/organizations/:slug/users",
            with_auth(get(list_org_users_handler), Action::MembershipManage),
        )
        .route(
            "/v1/organizations/:slug/users",
            with_auth(
                axum::routing::post(post_org_invite_user_handler),
                Action::MembershipManage,
            ),
        )
        .route(
            "/v1/user-cost/reconcile",
            with_auth(
                axum::routing::post(post_user_cost_reconcile_handler),
                Action::AuditRead,
            ),
        )
        .route(
            "/v1/sops/:id/shares",
            with_auth(get(list_sop_shares_handler), Action::SopShare),
        )
        .route(
            "/v1/sops/:id/shares",
            with_auth(
                axum::routing::post(post_sop_share_handler),
                Action::SopShare,
            ),
        )
        .route(
            "/v1/sops/:id/shares",
            with_auth(
                axum::routing::delete(delete_sop_share_handler),
                Action::SopShare,
            ),
        )
        // Story 2.21b: CLI intake / inbox / briefing-confirm surface
        // (loopback-only, same posture as the 2.21a routes above).
        // NOTE: `/v1/intake/cli` is registered AFTER the TimeoutLayer below
        // (issue #22) — its `task new --blocking` long-poll holds the request open
        // for up to CLI_INTAKE_DEFAULT_MAX_WAIT_SECS, so it must skip the timeout.
        .route(
            "/v1/inbox",
            with_auth(get(get_inbox_handler), Action::TaskRead),
        )
        .route(
            "/v1/briefings/:id/confirm",
            with_auth(
                axum::routing::post(post_briefing_confirm_handler),
                Action::TaskWrite,
            ),
        )
        // Issue #22: bound every *normal* request so a hung sandbox/DB handler
        // can't hold a connection open forever. Applies only to the routes
        // registered ABOVE this layer (axum layers wrap previously-added routes);
        // the long-lived `/ws` and the `/v1/intake/cli` long-poll are registered
        // BELOW so they keep their own (much longer / unbounded) lifetimes.
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .route("/ws", with_auth(get(ws::ws_upgrade), Action::TaskRead))
        .route(
            "/v1/intake/cli",
            self_gated(axum::routing::post(post_intake_cli_handler)),
        )
        // Issue #7 / ADR-018: make the verified-session store + insecure-headers
        // flag reachable by the per-route auth middleware (which runs as a
        // stateless `from_fn`) without threading state through every `with_auth`.
        .layer(Extension(auth::middleware::AuthDeps {
            sessions: state.auth_sessions.clone(),
            allow_insecure_headers: state.allow_insecure_headers,
        }))
        // Issue #22: cap request body size for EVERY route (placed last so it
        // wraps all routes incl. the intake handlers). axum's silent 2 MB default
        // is replaced by an explicit, smaller limit; `serde_json`'s own recursion
        // limit already bounds nesting depth, so size is the remaining vector.
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES));

    // Issue #33: serve the built UI bundle as the fallback. Named API routes
    // (`/v1/*`, `/ws`, `/healthz`) always win — the fallback only
    // fires when no route matches. `ServeDir` returns static assets; its own
    // `.fallback(ServeFile(index.html))` resolves unknown paths to the SPA shell
    // so client-side navigation works. Static serve is intentionally public
    // (unauthenticated): it's just the app shell + assets; the UI then calls the
    // auth-gated `/v1` + `/ws` API itself. Added after the auth `Extension` layer
    // so the static serve isn't wrapped by request-scoped API plumbing.
    let router = match ui_dist {
        Some(dir) => {
            let index_html = dir.join("index.html");
            let serve = ServeDir::new(&dir)
                .append_index_html_on_directories(true)
                .fallback(ServeFile::new(index_html));
            router.fallback_service(serve)
        }
        None => router,
    };

    router.with_state(state)
}

#[cfg(test)]
mod tests {
    //! Inline unit tests for handlers whose security guards can't be
    //! exercised through the integration harness in `tests/`
    //! (because that harness always lands at `127.0.0.1`).

    use super::*;
    use crate::error::ApiError;
    use axum::extract::{Path, Query};
    use seasoned_hand_core::auth::AuthContext;
    use seasoned_hand_core::router::SlotRouter;
    use seasoned_hand_core::sandbox::SandboxClient;
    use seasoned_hand_core::search::{SearchClient, SearchProvider};
    use seasoned_hand_core::verifier::routes::ListQuery as VerifyListQuery;
    use seasoned_hand_core::{db, pubsub};

    async fn empty_state() -> AppState {
        let pool = db::open(":memory:").await.expect("db");
        let redis = pubsub::RedisPool::new("redis://127.0.0.1:6").expect("redis url");
        let sandbox = SandboxClient::new(
            "ghcr.io/agent-infra/sandbox:1.0.0.152",
            std::env::temp_dir(),
        )
        .expect("sandbox client");
        let search = SearchClient::new(SearchProvider::Brave { api_key: None });
        let router = SlotRouter::default_for_bifrost();
        AppState::new(pool, redis, sandbox, search, router, Default::default())
    }

    fn test_auth() -> AuthContext {
        AuthContext {
            tenant_id: "legacy-default".into(),
            organization_id: "org-legacy-default".into(),
            actor_user_id: "user-test".into(),
            org_role: seasoned_hand_core::auth::Role::Admin,
            project_override_role: None,
        }
    }

    /// Story 1.13b regression: a non-loopback remote address must
    /// short-circuit to 403 `forbidden_non_loopback` before the token
    /// check runs, regardless of whether the admin token was supplied.
    /// The integration suite always lands at 127.0.0.1, so this guard
    /// can only be exercised at the handler level.
    #[tokio::test]
    async fn admin_rollback_refuses_non_loopback_remote() {
        let state = empty_state().await.with_admin_token("any-token");
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "X-Seasoned-Hand-Admin-Token",
            axum::http::HeaderValue::from_static("any-token"),
        );
        let remote: std::net::SocketAddr = "10.0.0.42:12345".parse().unwrap();
        let outcome = post_checkpoint_rollback_handler(
            State(state),
            axum::extract::ConnectInfo(remote),
            Extension(test_auth()),
            headers,
            Path(("sess-x".to_string(), "cp-x".to_string())),
            Json(RollbackBody { reason: "x".into() }),
        )
        .await;
        let err = outcome.expect_err("non-loopback must be 403");
        assert_eq!(err.0, StatusCode::FORBIDDEN);
        assert_eq!(err.1.0.error, "forbidden_non_loopback");
    }

    /// Story 2.21b: the long-poll `/v1/intake/cli` handler registers a
    /// pending oneshot, kicks the IntakeRouter, and `.await`s the
    /// receiver. We can drive both halves in-process by manually
    /// pushing a deliverable through `CliChannel::deliver` after a
    /// short delay; the handler future should resolve with the same
    /// deliverable.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cli_intake_long_poll_resolves_on_deliver() {
        use seasoned_hand_core::channel::cli::{CHANNEL_NAME, TARGET_INTAKE_PREFIX};
        use seasoned_hand_core::channel::{Deliverable, DeliverySink, DeliveryTarget, IntakeEvent};

        let state = empty_state().await.register_cli_channel();
        let intake_id = "cli:unit-test-1".to_string();
        let rx = state.cli_channel.register_pending(intake_id.clone());

        // Fire deliver() on the same channel from a spawned task to
        // emulate the DeliveryRouter side. A tiny delay ensures the
        // receiver is parked before we send.
        let cli_channel = state.cli_channel.clone();
        let intake_id_clone = intake_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            let target = DeliveryTarget {
                channel: CHANNEL_NAME.into(),
                target_ref: format!("{TARGET_INTAKE_PREFIX}{intake_id_clone}"),
                metadata: serde_json::json!({}),
            };
            let deliverable = Deliverable {
                id: "d-unit".into(),
                task_id: "t-unit".into(),
                tenant_id: None,
                format: "md".into(),
                source_content_path: None,
                source_content_sha256: None,
                rendered_content_path: "/workspace/.deliverables/d-unit.md".into(),
                rendered_content_sha256: "feedface".into(),
                content_size: 12,
                citations: None,
                provenance_manifest: serde_json::json!({}),
                created_at: 0,
            };
            cli_channel
                .deliver(&target, &deliverable)
                .await
                .expect("deliver ok");
        });

        // Round-trip the future without bothering with axum's extractor
        // layer — the routing test above exercises that. This test
        // pins the oneshot mechanics.
        let _event = IntakeEvent {
            channel: CHANNEL_NAME.into(),
            intake_id,
            brief_input: "ignored".into(),
            reply_target: None,
            metadata: serde_json::json!({}),
            tenant_id: None,
            received_at: 0,
        };
        let delivered = tokio::time::timeout(std::time::Duration::from_secs(2), rx)
            .await
            .expect("timeout")
            .expect("oneshot delivered");
        assert_eq!(delivered.id, "d-unit");
        assert_eq!(delivered.format, "md");
    }

    /// The 403 guard MUST run before the token comparison so that an
    /// attacker on a remote network cannot probe token validity via
    /// timing or 401/403 distinction.
    #[tokio::test]
    async fn admin_rollback_non_loopback_guard_runs_before_token_check() {
        let state = empty_state().await.with_admin_token("real-token");
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "X-Seasoned-Hand-Admin-Token",
            axum::http::HeaderValue::from_static("wrong-token"),
        );
        let remote: std::net::SocketAddr = "192.168.1.50:12345".parse().unwrap();
        let outcome = post_checkpoint_rollback_handler(
            State(state),
            axum::extract::ConnectInfo(remote),
            Extension(test_auth()),
            headers,
            Path(("sess-x".to_string(), "cp-x".to_string())),
            Json(RollbackBody { reason: "x".into() }),
        )
        .await;
        let err = outcome.expect_err("remote + wrong token still 403, not 401");
        assert_eq!(err.0, StatusCode::FORBIDDEN);
        assert_eq!(err.1.0.error, "forbidden_non_loopback");
    }

    /// /specs/REVIEW.md DEBT #48 + #59 close: the Phase 0/1 session + workspace
    /// GET routes were not loopback-gated. Every other `/v1/tasks/:id/*`
    /// sibling was. Smoke-cover one representative handler per group from
    /// each new gate so future contributors can't silently drop a guard.
    #[tokio::test]
    async fn list_sessions_refuses_non_loopback_remote() {
        let state = empty_state().await;
        let remote: std::net::SocketAddr = "10.0.0.42:12345".parse().unwrap();
        let outcome = list_sessions(
            State(state),
            axum::extract::ConnectInfo(remote),
            Extension(AuthContext {
                tenant_id: "legacy-default".into(),
                organization_id: "org-legacy-default".into(),
                actor_user_id: "user-test".into(),
                org_role: seasoned_hand_core::auth::Role::Admin,
                project_override_role: None,
            }),
            Query(SessionsListParams { limit: None }),
        )
        .await;
        let err = outcome.expect_err("non-loopback must be 403");
        assert_eq!(err.0, StatusCode::FORBIDDEN);
        assert_eq!(err.1.0.error, "forbidden_non_loopback");
    }

    #[tokio::test]
    async fn workspace_root_refuses_non_loopback_remote() {
        let state = empty_state().await;
        let remote: std::net::SocketAddr = "203.0.113.7:443".parse().unwrap();
        let outcome = workspace_root(
            State(state),
            axum::extract::ConnectInfo(remote),
            Extension(test_auth()),
            Path("any-session-id".to_string()),
        )
        .await;
        let err = outcome.expect_err("non-loopback must be 403");
        assert_eq!(err.0, StatusCode::FORBIDDEN);
        assert_eq!(err.1.0.error, "forbidden_non_loopback");
    }

    /// Codex review DEBT #69 — extend the loopback regression sweep
    /// to cover every gate added in commits 18d472d + 3721c37. If a
    /// future contributor removes `require_loopback(remote)?` from
    /// any of these handlers, this set catches it.
    ///
    /// Coverage matrix:
    /// - DEBT #48 / #59 — list_sessions ✓ (covered above),
    ///   workspace_root ✓ (covered above), get_session, list_events,
    ///   workspace_proxy, get_feature_list, get_progress
    /// - DEBT #65 (Codex Finding A) — list_checkpoints_handler,
    ///   list_verifications_handler, get_verification_handler
    /// - DEBT #66 (user-approved /ws gate) — covered via the WS test
    ///   below since the upgrade returns axum::response::Response
    ///   directly (not the standard handler `Result<_, (StatusCode,
    ///   Json<ApiError>)>` shape).
    /// - DEBT #70 — list_channels_handler, get_channel_health_handler,
    ///   post_channel_test_handler
    async fn assert_handler_refuses_non_loopback<F, Fut, T>(handler: F)
    where
        F: FnOnce(std::net::SocketAddr) -> Fut,
        Fut: std::future::Future<Output = Result<T, (StatusCode, Json<ApiError>)>>,
        T: std::fmt::Debug,
    {
        let remote: std::net::SocketAddr = "10.0.0.42:12345".parse().unwrap();
        let outcome = handler(remote).await;
        let err = outcome.expect_err("non-loopback must be 403");
        assert_eq!(err.0, StatusCode::FORBIDDEN);
        assert_eq!(err.1.0.error, "forbidden_non_loopback");
    }

    #[tokio::test]
    async fn get_session_refuses_non_loopback_remote() {
        let state = empty_state().await;
        assert_handler_refuses_non_loopback(|remote| {
            get_session(
                State(state.clone()),
                axum::extract::ConnectInfo(remote),
                Extension(test_auth()),
                Path("any".into()),
            )
        })
        .await;
    }

    #[tokio::test]
    async fn list_events_refuses_non_loopback_remote() {
        let state = empty_state().await;
        assert_handler_refuses_non_loopback(|remote| {
            list_events(
                State(state.clone()),
                axum::extract::ConnectInfo(remote),
                Extension(AuthContext {
                    tenant_id: "legacy-default".into(),
                    organization_id: "org-legacy-default".into(),
                    actor_user_id: "user-test".into(),
                    org_role: seasoned_hand_core::auth::Role::Admin,
                    project_override_role: None,
                }),
                Path("any".into()),
                Query(EventsQueryParams {
                    after_id: None,
                    event_type: None,
                    limit: None,
                }),
            )
        })
        .await;
    }

    #[tokio::test]
    async fn workspace_proxy_sub_path_refuses_non_loopback_remote() {
        let state = empty_state().await;
        assert_handler_refuses_non_loopback(|remote| {
            workspace_proxy(
                State(state.clone()),
                axum::extract::ConnectInfo(remote),
                Extension(test_auth()),
                Path(("any".into(), "sub/path.txt".into())),
            )
        })
        .await;
    }

    #[tokio::test]
    async fn get_feature_list_refuses_non_loopback_remote() {
        let state = empty_state().await;
        assert_handler_refuses_non_loopback(|remote| {
            get_feature_list(
                State(state.clone()),
                axum::extract::ConnectInfo(remote),
                Extension(test_auth()),
                Path("any".into()),
            )
        })
        .await;
    }

    #[tokio::test]
    async fn get_progress_refuses_non_loopback_remote() {
        let state = empty_state().await;
        assert_handler_refuses_non_loopback(|remote| {
            get_progress(
                State(state.clone()),
                axum::extract::ConnectInfo(remote),
                Extension(test_auth()),
                Path("any".into()),
                Query(ProgressQuery { lines: None }),
            )
        })
        .await;
    }

    #[tokio::test]
    async fn list_checkpoints_refuses_non_loopback_remote() {
        let state = empty_state().await;
        assert_handler_refuses_non_loopback(|remote| {
            list_checkpoints_handler(
                State(state.clone()),
                axum::extract::ConnectInfo(remote),
                Extension(test_auth()),
                Path("any".into()),
                Query(seasoned_hand_core::checkpoint::routes::ListQuery::default()),
            )
        })
        .await;
    }

    #[tokio::test]
    async fn list_verifications_refuses_non_loopback_remote() {
        let state = empty_state().await;
        assert_handler_refuses_non_loopback(|remote| {
            list_verifications_handler(
                State(state.clone()),
                axum::extract::ConnectInfo(remote),
                Extension(test_auth()),
                Path("any".into()),
                Query(VerifyListQuery::default()),
            )
        })
        .await;
    }

    #[tokio::test]
    async fn get_verification_refuses_non_loopback_remote() {
        let state = empty_state().await;
        assert_handler_refuses_non_loopback(|remote| {
            get_verification_handler(
                State(state.clone()),
                axum::extract::ConnectInfo(remote),
                Extension(test_auth()),
                Path("any".into()),
            )
        })
        .await;
    }

    #[tokio::test]
    async fn list_channels_refuses_non_loopback_remote() {
        let state = empty_state().await;
        assert_handler_refuses_non_loopback(|remote| {
            list_channels_handler(State(state.clone()), axum::extract::ConnectInfo(remote))
        })
        .await;
    }

    #[tokio::test]
    async fn get_channel_health_refuses_non_loopback_remote() {
        let state = empty_state().await;
        assert_handler_refuses_non_loopback(|remote| {
            get_channel_health_handler(
                State(state.clone()),
                axum::extract::ConnectInfo(remote),
                Path("any".into()),
            )
        })
        .await;
    }

    #[tokio::test]
    async fn post_channel_test_refuses_non_loopback_remote() {
        let state = empty_state().await;
        assert_handler_refuses_non_loopback(|remote| {
            post_channel_test_handler(
                State(state.clone()),
                axum::extract::ConnectInfo(remote),
                Path("any".into()),
                Query(ChannelTestQuery { role: None }),
            )
        })
        .await;
    }

    // SEC-IT1-H2: the 3 SOP-share handlers were the only sensitive Phase 5
    // routes missing the loopback gate. Lock the fix with regression sweeps.
    #[tokio::test]
    async fn post_sop_share_refuses_non_loopback_remote() {
        let state = empty_state().await;
        assert_handler_refuses_non_loopback(|remote| {
            post_sop_share_handler(
                State(state.clone()),
                axum::extract::ConnectInfo(remote),
                Extension(test_auth()),
                Path("any".into()),
                Json(SopShareBody {
                    user_email: "x@example.com".into(),
                    permission: "viewer".into(),
                    expected_updated_at: None,
                }),
            )
        })
        .await;
    }

    #[tokio::test]
    async fn delete_sop_share_refuses_non_loopback_remote() {
        let state = empty_state().await;
        assert_handler_refuses_non_loopback(|remote| {
            delete_sop_share_handler(
                State(state.clone()),
                axum::extract::ConnectInfo(remote),
                Extension(test_auth()),
                Path("any".into()),
                Json(SopUnshareBody {
                    user_email: "x@example.com".into(),
                    expected_updated_at: None,
                }),
            )
        })
        .await;
    }

    #[tokio::test]
    async fn list_sop_shares_refuses_non_loopback_remote() {
        let state = empty_state().await;
        assert_handler_refuses_non_loopback(|remote| {
            list_sop_shares_handler(
                State(state.clone()),
                axum::extract::ConnectInfo(remote),
                Extension(test_auth()),
                Path("any".into()),
            )
        })
        .await;
    }
}
