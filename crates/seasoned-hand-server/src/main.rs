//! Seasoned Hand server binary.
//! refs: /specs/phase-0/architecture.md §4.1

use std::net::SocketAddr;
use std::path::PathBuf;

use seasoned_hand_core::capability::{
    CapabilityProbe, assert_main_supports_tool_calling, warn_implied_slot_capability_mismatches,
};
use seasoned_hand_core::curator::{
    CuratorConfig, EmbeddingBudget, LlmSemanticAdjudicator, ProductionCuratorCycleExecutor,
    ProductionCuratorWorker, ProductionEmbeddingReranker, SqliteBacklogProbe,
    SqliteCandidateBuilder, SqliteConflictDetector, SqliteConsolidationEngine,
};
use seasoned_hand_core::llm::LlmClient;
use seasoned_hand_core::router::{SlotName, SlotRouter};
use seasoned_hand_core::sandbox::SandboxClient;
use seasoned_hand_core::search::SearchClient;
use seasoned_hand_core::{db, pubsub};
use seasoned_hand_server::{AppState, EmailChannelEnv, app};
use tracing_subscriber::EnvFilter;

fn log_join_error(name: &str, result: Result<(), tokio::task::JoinError>) {
    if let Err(error) = result {
        tracing::warn!(task = name, %error, "background task join failed");
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite:./data/seasoned-hand.db".to_string());
    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
    let sandbox_image = std::env::var("AIO_SANDBOX_IMAGE")
        .unwrap_or_else(|_| "ghcr.io/agent-infra/sandbox:1.0.0.152".to_string());
    let workspace_root: PathBuf = std::env::var("SANDBOX_WORKSPACE_HOST")
        .unwrap_or_else(|_| "./data/workspaces".into())
        .into();

    let db = db::open(&database_url).await?;
    let redis = pubsub::RedisPool::new(&redis_url)?;
    if let Err(e) = redis.ping().await {
        tracing::warn!(error = %e, %redis_url, "redis ping failed at startup; healthz will report degraded until reachable");
    }
    let sandbox = SandboxClient::new(sandbox_image, workspace_root)?;
    let search = SearchClient::brave_from_env();

    let slots_path =
        std::env::var("SLOTS_CONFIG_PATH").unwrap_or_else(|_| "config/slots.yaml".into());
    let router = if std::path::Path::new(&slots_path).exists() {
        match SlotRouter::from_yaml(&slots_path) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, %slots_path, "slots config parse failed; falling back to default");
                SlotRouter::default_for_bifrost()
            }
        }
    } else {
        tracing::info!(%slots_path, "slots config not found; using default (main -> agent-primary)");
        SlotRouter::default_for_bifrost()
    };
    let main_slot = router.resolve(SlotName::Main);
    let llm = LlmClient::new(main_slot.base_url.clone(), main_slot.api_key.clone());
    let probe = CapabilityProbe::new(llm);
    let capabilities = match probe.probe_models().await {
        Ok(probed) => probed,
        Err(error) => {
            tracing::warn!(%error, "capability probe failed; falling back to built-in table");
            Default::default()
        }
    };
    assert_main_supports_tool_calling(&router, &capabilities)?;
    warn_implied_slot_capability_mismatches(&router, &capabilities);

    let mut state = AppState::new(db, redis, sandbox, search, router, capabilities);

    // Story 1.13b: load admin-token + rollback-flag env vars and
    // apply via builder. Done in main.rs (not AppState::new) so tests
    // don't race on process-wide env state.
    let admin_token = std::env::var("SEASONED_HAND_ADMIN_TOKEN").unwrap_or_default();
    state = state.with_admin_token(admin_token);

    // Story 2.10: register the production WebhookChannel and snapshot
    // the intake token onto AppState so the `POST /v1/intake/webhook`
    // route handler can authenticate without downcasting the registry.
    //
    // `SEASONED_HAND_INTAKE_TOKEN` unset / empty → endpoint disabled
    // (handler returns 503). `WEBHOOK_DELIVERY_ALLOWLIST` is a
    // comma-separated list of CIDRs that bypass the default-deny SSRF
    // guard; unset → empty allow-list (default-deny only, see
    // phase-2/DEBT.md #1).
    let webhook_intake_token =
        std::sync::Arc::new(std::env::var("SEASONED_HAND_INTAKE_TOKEN").unwrap_or_default());
    let allowlist = match std::env::var("WEBHOOK_DELIVERY_ALLOWLIST") {
        Ok(raw) => match seasoned_hand_core::channel::webhook::ssrf::parse_allowlist(&raw) {
            Ok(nets) => nets,
            Err(error) => {
                tracing::warn!(%error, "WEBHOOK_DELIVERY_ALLOWLIST parse failed; ignoring");
                Vec::new()
            }
        },
        Err(_) => Vec::new(),
    };
    state = state.register_webhook_channel(webhook_intake_token, allowlist);

    // Story 2.11: register the production EmailChannel. IMAP_HOST /
    // IMAP_USERNAME / IMAP_PASSWORD must all be set; otherwise the
    // channel is disabled cleanly. INTAKE_EMAIL_ALLOWED_SENDERS empty
    // → deny-all (architecture §9, phase-2/DEBT.md #4) — the channel
    // logs `intake_sender_rejected{reason:"allowlist_empty"}` for
    // every dropped message.
    state = state.register_email_channel(EmailChannelEnv::from_env());

    // Story 2.12: register the production NtfyChannel + load the
    // operator's `config/notify.toml`. Channel is registered only when
    // `NTFY_TOPIC` is set (the topic itself lives in the notify config
    // per-channel `default_target`, but presence of the env signals
    // "operator wants ntfy"). Missing config file is non-fatal — every
    // trigger silently disabled.
    if std::env::var("NTFY_TOPIC")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        let ntfy_host = std::env::var("NTFY_HOST")
            .unwrap_or_else(|_| seasoned_hand_core::channel::ntfy::DEFAULT_HOST.into());
        state = state.register_ntfy_channel(ntfy_host);
    }

    // Story 2.21a / Phase 2 DEBT #23: register the always-on `cli`
    // channel into the registry. The same `Arc<CliChannel>` already
    // lives on `state.cli_channel` (built by `AppState::new`) — this
    // step just wires it into the `ChannelRegistry` so the
    // `DeliveryRouter` can route `cli` reply_targets and `GET
    // /v1/channels` lists it.
    state = state.register_cli_channel();
    let notify_config_path =
        std::env::var("NOTIFY_CONFIG_PATH").unwrap_or_else(|_| "config/notify.toml".into());
    let notify_config =
        match seasoned_hand_core::notify::NotifyConfig::from_path(&notify_config_path) {
            Ok(cfg) => {
                tracing::info!(
                    path = %notify_config_path,
                    triggers = cfg.triggers.len(),
                    channels = cfg.channels.len(),
                    "notify config loaded"
                );
                cfg
            }
            Err(error) => {
                tracing::info!(
                    %error,
                    path = %notify_config_path,
                    "notify config not found / unparseable; notifications silently disabled"
                );
                seasoned_hand_core::notify::NotifyConfig::empty()
            }
        };
    let notify_config = std::sync::Arc::new(notify_config);
    state = state.with_notify_config(notify_config.clone());

    let rollback_flag = std::env::var("SEASONED_HAND_ROLLBACK_ON_VERIFIER_FAIL")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    state = state.with_rollback_on_verifier_fail(rollback_flag);

    // Phase 1 / story 1.9: when the verifier slot is enabled, load the
    // FAIL-biased system prompt from disk. Missing file is a startup-
    // fatal configuration error.
    if state.verifier_enabled {
        let prompt_path = std::env::var("VERIFIER_PROMPT_PATH")
            .unwrap_or_else(|_| "config/prompts/verifier.system.txt".to_string());
        let prompt = seasoned_hand_core::verifier::load_system_prompt(&prompt_path)?;
        state = state.with_verifier_prompt(std::sync::Arc::new(prompt));
        tracing::info!(path = %prompt_path, "verifier system prompt loaded");
    }

    // Phase 2 / story 2.20: wire the NarratorHook's classifier-slot LLM
    // path. Closes the deferred plumbing from story 1.15 Execution
    // notes. Missing prompt file is non-fatal — narration degrades to
    // templated-only ("Invoking {tool}" for action-changing tools).
    let narrator_prompt_path = std::env::var("NARRATOR_PROMPT_PATH")
        .unwrap_or_else(|_| "config/prompts/narrator.system.txt".to_string());
    match std::fs::read_to_string(&narrator_prompt_path) {
        Ok(prompt) => {
            use seasoned_hand_server::NarratorClassifierWiring;
            // Snapshot the slot fields BEFORE consuming `state` — the
            // resolver returns a borrow into `state.router`.
            let (classifier_base_url, classifier_api_key, classifier_model) = {
                let slot = state.router.resolve(SlotName::Classifier);
                (
                    slot.base_url.clone(),
                    slot.api_key.clone(),
                    slot.model.clone(),
                )
            };
            let classifier_llm = LlmClient::new(classifier_base_url, classifier_api_key);
            state = state.with_narrator_classifier(NarratorClassifierWiring {
                llm: std::sync::Arc::new(classifier_llm),
                model: classifier_model.clone(),
                system_prompt: std::sync::Arc::new(prompt),
            });
            tracing::info!(
                path = %narrator_prompt_path,
                slot_model = %classifier_model,
                "narrator classifier wired",
            );
        }
        Err(error) => {
            tracing::warn!(
                %error,
                path = %narrator_prompt_path,
                "narrator classifier prompt missing; narration falls through to templated-only",
            );
        }
    }

    // Phase 1: rehydrate sandbox handle cache from Docker before binding the
    // listener so existing per-session containers from a prior boot are
    // re-attached to live sessions and orphans are logged. Non-fatal: if
    // Docker is unreachable (test harness, missing socket), continue with an
    // empty cache. refs: /specs/phase-1/stories/story-1.2.md
    match state.sandbox.rehydrate_from_docker(&state.db).await {
        Ok(report) => tracing::info!(
            restored = report.restored,
            orphans = report.orphans,
            errors = report.errors.len(),
            "sandbox cache rehydrated"
        ),
        Err(error) => tracing::error!(
            %error,
            "sandbox rehydration failed; continuing with empty cache"
        ),
    }

    // Story 2.10 / DEBT #16: spawn the IntakeRouter drain loop and the
    // long-lived intake providers (`ChannelRegistry::spawn_intakes`).
    // Both Chat and Webhook channels have a no-op `run()` today — Chat
    // pushes synchronously through `intake_router.handle_event` from
    // the WS handler, Webhook pushes synchronously from the
    // `POST /v1/intake/webhook` route. The drain loop exists so future
    // polling intake providers (EmailChannel, story 2.11) can fan into
    // a single ordered queue without each provider needing its own
    // wiring.
    let intake_shutdown = tokio_util::sync::CancellationToken::new();
    let (intake_tx, intake_rx) = tokio::sync::mpsc::channel(64);
    let intake_handle = {
        let router = state.intake_router.clone();
        let token = intake_shutdown.clone();
        tokio::spawn(async move {
            router.run(intake_rx, token).await;
        })
    };
    let intake_provider_handles = state
        .channels
        .spawn_intakes(intake_tx, intake_shutdown.clone());

    // Story 2.12: spawn the NotifyWorker + NotifyEventListener.
    //
    // - Listener PSUBSCRIBEs to `sh:events:*` and XADDs a `notify_request`
    //   for every Misc event matching a configured trigger.
    // - Worker XREADGROUPs `notify_request` and fans each entry into
    //   per-channel `NotifySink::notify` calls (ntfy / webhook / email).
    //
    // Both honour `notify_shutdown` so graceful shutdown drains
    // in-flight dispatches.
    let notify_shutdown = tokio_util::sync::CancellationToken::new();
    let notify_worker_handle = {
        let redis = std::sync::Arc::new(state.redis.clone());
        let resolver: std::sync::Arc<dyn seasoned_hand_core::notify::TargetResolver> =
            notify_config.clone();
        let worker = seasoned_hand_core::notify::NotifyWorker::new(
            state.channels.clone(),
            state.notifications_sent.clone(),
            resolver,
        );
        let token = notify_shutdown.clone();
        tokio::spawn(async move {
            worker.run(redis, token).await;
        })
    };
    let notify_listener_handle = {
        let redis = std::sync::Arc::new(state.redis.clone());
        let dispatch: std::sync::Arc<dyn seasoned_hand_core::notify::NotifyDispatch> =
            std::sync::Arc::new(seasoned_hand_core::notify::RedisNotifyDispatch::new(
                redis.clone(),
            ));
        let listener = std::sync::Arc::new(seasoned_hand_core::notify::NotifyEventListener::new(
            notify_config.clone(),
            dispatch,
        ));
        let token = notify_shutdown.clone();
        tokio::spawn(async move {
            listener.run(redis, token).await;
        })
    };

    // Story 1.13: spawn the Checkpoint Manager. The Phase 1 baseline run
    // loop is a polling no-op; the real Plan{op:"advance"} fanout lands
    // alongside the global event bus in story 1.20 E2E. The manager's
    // `handle_plan_advance` is the unit the agent runner will call once
    // the bus exists.
    let checkpoint_shutdown = tokio_util::sync::CancellationToken::new();
    let checkpoint_handle = {
        use seasoned_hand_core::checkpoint::{
            CheckpointManager, CheckpointManagerDeps, SandboxGitShell,
        };
        let git: std::sync::Arc<dyn seasoned_hand_core::checkpoint::GitShell> =
            std::sync::Arc::new(SandboxGitShell::new(state.sandbox.clone()));
        let manager = CheckpointManager::new(CheckpointManagerDeps {
            store: state.checkpoints.clone(),
            labels: state.checkpoint_labels.clone(),
            events: state.events.clone(),
            git,
        });
        let token = checkpoint_shutdown.clone();
        tokio::spawn(async move {
            if let Err(error) = manager.run(token).await {
                tracing::error!(%error, "checkpoint manager exited with error");
            }
        })
    };

    // Story 1.9b: spawn the Verifier Worker if verifier is enabled.
    // The worker's `run()` returns Ok(()) immediately when disabled, so
    // we can spawn unconditionally for symmetry, but skipping the spawn
    // keeps the runtime smaller when the verifier slot isn't configured.
    let verifier_shutdown = tokio_util::sync::CancellationToken::new();
    let verifier_handle = if state.verifier_enabled {
        use seasoned_hand_core::verifier::{
            Worker, WorkerDeps, extraction_handler::PlannerSlotExtractionHandler,
            gate::VerifierGate,
        };
        let deps = WorkerDeps::from_router(
            &state.router,
            state.plan_manager.clone(),
            state.events.clone(),
            state.sandbox.clone(),
            state.verifications.clone(),
            state.cost.clone(),
            state.verifier_system_prompt.clone(),
            state.cancel_tokens.clone(),
        );
        let worker = Worker::new(deps);
        // Story 1.13b: production rollback handler — looks up the
        // latest checkpoint for the session and dispatches the
        // (LLM-masked) `checkpoint_rollback` tool. Attached
        // unconditionally; the rollback only fires when
        // `checkpoint_rollback_on_verifier_fail` (env-gated) is true.
        let rollback_handler = std::sync::Arc::new(ProductionRollbackHandler::new(state.clone()))
            as std::sync::Arc<dyn seasoned_hand_core::verifier::gate::RollbackHandler>;
        let learning_enabled = std::env::var("SH_LEARNING_ENABLED")
            .map(|v| !(v == "0" || v.eq_ignore_ascii_case("false")))
            .unwrap_or(true);
        let mut gate =
            VerifierGate::new(state.db.clone(), state.events.clone(), state.runner.clone())
                .with_rollback(rollback_handler, state.checkpoint_rollback_on_verifier_fail);
        if learning_enabled {
            let extraction = PlannerSlotExtractionHandler::new(
                state.db.clone(),
                state.events.clone(),
                state.router.clone(),
            );
            gate = gate.with_extraction(std::sync::Arc::new(extraction));
        } else {
            tracing::info!("learning extraction disabled (SH_LEARNING_ENABLED=false)");
        }
        let redis = std::sync::Arc::new(state.redis.clone());
        let token = verifier_shutdown.clone();
        let gate_token = verifier_shutdown.clone();
        Some(tokio::spawn(async move {
            let worker_task = tokio::spawn(async move {
                if let Err(error) = worker.run(true, redis, token).await {
                    tracing::error!(%error, "verifier worker exited with error");
                }
            });
            let gate_task = tokio::spawn(async move {
                gate.run(gate_token).await;
            });
            let _ = tokio::join!(worker_task, gate_task);
        }))
    } else {
        tracing::info!("verifier worker not spawned (verifier_enabled = false)");
        None
    };

    let curator_shutdown = tokio_util::sync::CancellationToken::new();
    let curator_enabled = std::env::var("SH_CURATOR_ENABLED")
        .map(|v| !(v == "0" || v.eq_ignore_ascii_case("false")))
        .unwrap_or(false);
    let curator_handle = if curator_enabled {
        let config = CuratorConfig {
            enabled: true,
            interval_seconds: std::env::var("SH_CURATOR_INTERVAL_SECONDS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(300),
            backlog_threshold: std::env::var("SH_CURATOR_BACKLOG_THRESHOLD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
            max_candidates_per_cycle: std::env::var("SH_CURATOR_MAX_CANDIDATES_PER_CYCLE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(50),
            embedding_budget_monthly_tokens: std::env::var(
                "SH_CURATOR_EMBEDDING_BUDGET_MONTHLY_TOKENS",
            )
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(50_000),
            embedding_budget_soft_cap_pct: std::env::var("SH_CURATOR_EMBEDDING_SOFT_CAP_PCT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.08),
            embedding_budget_hard_breaker_pct: std::env::var(
                "SH_CURATOR_EMBEDDING_HARD_BREAKER_PCT",
            )
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.12),
            embedding_model: std::env::var("SH_EMBEDDING_MODEL")
                .unwrap_or_else(|_| "text-embedding-3-small".to_string()),
            project_id: std::env::var("SH_CURATOR_PROJECT_ID")
                .unwrap_or_else(|_| "default".to_string()),
        };
        let embedding_slot = state.router.resolve(SlotName::Embedding);
        let embedding_llm = LlmClient::new(
            embedding_slot.base_url.clone(),
            embedding_slot.api_key.clone(),
        );
        let candidate_builder = std::sync::Arc::new(SqliteCandidateBuilder::new(state.db.clone()));
        let consolidation_engine =
            std::sync::Arc::new(SqliteConsolidationEngine::new(state.db.clone()));
        let conflict_slot = state.router.resolve(SlotName::SessionSearch);
        let conflict_llm = LlmClient::new(
            conflict_slot.base_url.clone(),
            conflict_slot.api_key.clone(),
        );
        let conflict_detector = std::sync::Arc::new(SqliteConflictDetector::new(
            state.db.clone(),
            std::sync::Arc::new(LlmSemanticAdjudicator::new(
                conflict_llm,
                conflict_slot.model.clone(),
            )),
        ));
        let reranker = std::sync::Arc::new(ProductionEmbeddingReranker::new(
            embedding_llm,
            config.embedding_model.clone(),
            EmbeddingBudget {
                monthly_embedding_tokens: config.embedding_budget_monthly_tokens,
                soft_cap_pct: config.embedding_budget_soft_cap_pct,
                hard_breaker_pct: config.embedding_budget_hard_breaker_pct,
            },
        ));
        let executor = std::sync::Arc::new(ProductionCuratorCycleExecutor::new(
            candidate_builder,
            reranker,
            consolidation_engine,
            conflict_detector,
            config.max_candidates_per_cycle,
        ));
        let worker = ProductionCuratorWorker::new(
            config.clone(),
            state.db.clone(),
            state.events.clone(),
            std::sync::Arc::new(SqliteBacklogProbe::new(state.db.clone())),
            executor,
        );
        let token = curator_shutdown.clone();
        Some(tokio::spawn(async move {
            if let Err(error) = worker.run(token).await {
                tracing::error!(%error, "curator worker exited with error");
            }
        }))
    } else {
        tracing::info!("curator worker not spawned (SH_CURATOR_ENABLED=false)");
        None
    };

    // Story 2.17 / Phase 0 DEBT #16: spawn the workspace TTL cron.
    // Single-task loop that wakes every `SANDBOX_CLEANUP_INTERVAL_SEC`
    // (default 3600), tears down container + workspace for terminal-
    // state tasks past their per-status TTL. Active tasks
    // (running/paused) are never GC'd. Failures within the cycle are
    // absorbed; the loop itself never exits with Err.
    let ttl_shutdown = tokio_util::sync::CancellationToken::new();
    let ttl_handle = {
        let cron = state.workspace_ttl_cron.clone();
        let token = ttl_shutdown.clone();
        tokio::spawn(async move {
            cron.run(token).await;
        })
    };

    let addr = bind_addr()?;
    tracing::info!(%addr, %database_url, %redis_url, "seasoned-hand-server starting");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    // Story 1.13b: the rollback admin endpoint needs `ConnectInfo<SocketAddr>`
    // to enforce the loopback guard, so we use
    // `into_make_service_with_connect_info` instead of the plain Router.
    axum::serve(
        listener,
        app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    // Signal the verifier worker to drain and exit.
    verifier_shutdown.cancel();
    if let Some(handle) = verifier_handle {
        log_join_error("verifier", handle.await);
    }

    curator_shutdown.cancel();
    if let Some(handle) = curator_handle {
        log_join_error("curator", handle.await);
    }

    // Story 1.13: drain the checkpoint manager.
    checkpoint_shutdown.cancel();
    log_join_error("checkpoint", checkpoint_handle.await);

    // Story 2.10 / DEBT #16: drain the intake plane. Cancelling the
    // token signals every long-lived `IntakeProvider::run` AND the
    // drain loop; we join the drain first so any in-flight events
    // queued by the providers land on the persistence path before the
    // task exits.
    intake_shutdown.cancel();
    for handle in intake_provider_handles {
        match handle.await {
            Err(join_error) => {
                tracing::warn!(
                    task = "intake_provider",
                    %join_error,
                    "intake provider join failed",
                );
            }
            Ok(Err(run_error)) => {
                tracing::warn!(
                    task = "intake_provider",
                    %run_error,
                    "intake provider exited with error",
                );
            }
            Ok(Ok(())) => {}
        }
    }
    log_join_error("intake_router", intake_handle.await);

    // Story 2.12: drain the notify plane.
    notify_shutdown.cancel();
    log_join_error("notify_listener", notify_listener_handle.await);
    log_join_error("notify_worker", notify_worker_handle.await);

    // Story 2.17: drain the workspace TTL cron.
    ttl_shutdown.cancel();
    log_join_error("workspace_ttl", ttl_handle.await);

    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

fn bind_addr() -> Result<SocketAddr, std::net::AddrParseError> {
    let host = std::env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = std::env::var("PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(3000);

    format!("{host}:{port}").parse()
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::warn!(%error, "failed to listen for shutdown signal");
    }
    tracing::info!("shutdown signal received");
}

/// Story 1.13b: production `RollbackHandler` for the VerifierGate's
/// opt-in path. Looks up the latest non-rolled-back checkpoint and
/// dispatches the (LLM-masked) `checkpoint_rollback` tool.
struct ProductionRollbackHandler {
    state: seasoned_hand_server::AppState,
}

impl ProductionRollbackHandler {
    fn new(state: seasoned_hand_server::AppState) -> Self {
        Self { state }
    }
}

#[async_trait::async_trait]
impl seasoned_hand_core::verifier::gate::RollbackHandler for ProductionRollbackHandler {
    async fn rollback_latest(&self, session_id: &str, reason: &str) -> bool {
        let latest = match self.state.checkpoints.latest_for_session(session_id).await {
            Ok(Some(cp)) => cp,
            Ok(None) => {
                tracing::warn!(%session_id, "no rollback candidate (no un-reverted checkpoints)");
                return false;
            }
            Err(error) => {
                tracing::warn!(%session_id, %error, "rollback: latest_for_session failed");
                return false;
            }
        };
        let ctx = seasoned_hand_core::tools::ToolContext {
            session_id: session_id.to_string(),
            mask_mode: seasoned_hand_core::dispatch::mask::AgentMode::Internal,
            events: self.state.events.clone(),
            sandbox: self.state.sandbox.clone(),
            search: self.state.search.clone(),
            plan_manager: self.state.plan_manager.clone(),
            checkpoint_labels: self.state.checkpoint_labels.clone(),
            checkpoints: self.state.checkpoints.clone(),
            matcher_mode: seasoned_hand_core::matcher::MatcherMode::Production,
        };
        let out = self
            .state
            .dispatcher
            .dispatch(
                &ctx,
                "checkpoint_rollback",
                serde_json::json!({
                    "checkpoint_id": latest.id,
                    "reason": reason,
                    "rolled_back_by": "verifier",
                }),
            )
            .await;
        out.ok
    }
}
