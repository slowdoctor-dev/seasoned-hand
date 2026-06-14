//! Seasoned Hand server binary.
//! refs: /specs/phase-0/architecture.md §4.1

use std::net::SocketAddr;
use std::path::PathBuf;

use seasoned_hand_core::capability::{
    CapabilityProbe, assert_main_supports_tool_calling, warn_implied_slot_capability_mismatches,
};
use seasoned_hand_core::curator::retention::{
    CuratorRetentionJob, DEFAULT_RETENTION_INTERVAL_SECS, RetentionConfig, RetentionScheduler,
};
use seasoned_hand_core::curator::{
    CuratorConfig, CuratorRuntimeDeps, EmbeddingBudget, LlmSemanticAdjudicator,
    ProductionCuratorCycleExecutor, ProductionCuratorWorker, ProductionEmbeddingReranker,
    SqliteBacklogProbe, SqliteCandidateBuilder, SqliteConflictDetector, SqliteConsolidationEngine,
    SqliteKnowledgeDatasourceWriter, SqliteOperatorReviewQueue, SqliteRetrospectiveGenerator,
    SqliteWorkPatternExtractor,
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

// Story 5.22: strict-parse helpers moved to seasoned_hand_core::config::strict
// so server, CLI, and any worker spawn point share one implementation.
// Local re-imports keep the rest of main.rs reading the same.
use seasoned_hand_core::config::strict::{
    env_bool_or_default, env_f32_or_default, env_u32_or_default, env_u64_or_default,
};

fn load_curator_config_from_lookup<F>(lookup: &F) -> Result<Option<CuratorConfig>, String>
where
    F: Fn(&str) -> Option<String>,
{
    let enabled = env_bool_or_default(lookup, "SH_CURATOR_ENABLED", false)?;
    if !enabled {
        return Ok(None);
    }
    let soft = env_f32_or_default(lookup, "SH_CURATOR_EMBEDDING_SOFT_CAP_PCT", 0.08)?;
    let hard = env_f32_or_default(lookup, "SH_CURATOR_EMBEDDING_HARD_BREAKER_PCT", 0.12)?;
    if !(0.0..=1.0).contains(&soft) {
        return Err(format!(
            "SH_CURATOR_EMBEDDING_SOFT_CAP_PCT out of range {soft} (expected 0.0..=1.0)"
        ));
    }
    if !(0.0..=1.0).contains(&hard) {
        return Err(format!(
            "SH_CURATOR_EMBEDDING_HARD_BREAKER_PCT out of range {hard} (expected 0.0..=1.0)"
        ));
    }
    if hard < soft {
        return Err(format!(
            "SH_CURATOR_EMBEDDING_HARD_BREAKER_PCT ({hard}) must be >= SH_CURATOR_EMBEDDING_SOFT_CAP_PCT ({soft})"
        ));
    }

    Ok(Some(CuratorConfig {
        enabled: true,
        interval_seconds: env_u64_or_default(lookup, "SH_CURATOR_INTERVAL_SECONDS", 300)?,
        backlog_threshold: env_u32_or_default(lookup, "SH_CURATOR_BACKLOG_THRESHOLD", 10)?,
        max_candidates_per_cycle: env_u32_or_default(
            lookup,
            "SH_CURATOR_MAX_CANDIDATES_PER_CYCLE",
            50,
        )?,
        embedding_budget_monthly_tokens: env_u64_or_default(
            lookup,
            "SH_CURATOR_EMBEDDING_BUDGET_MONTHLY_TOKENS",
            50_000,
        )?,
        embedding_budget_soft_cap_pct: soft,
        embedding_budget_hard_breaker_pct: hard,
        embedding_model: lookup("SH_EMBEDDING_MODEL")
            .unwrap_or_else(|| "text-embedding-3-small".to_string()),
        auto_archive_enabled: env_bool_or_default(
            lookup,
            "SH_CURATOR_AUTO_ARCHIVE_ENABLED",
            false,
        )?,
        archive_recommend_min_confidence: env_f32_or_default(
            lookup,
            "SH_CURATOR_ARCHIVE_RECOMMEND_MIN_CONFIDENCE",
            0.40,
        )?,
        archive_apply_min_confidence: env_f32_or_default(
            lookup,
            "SH_CURATOR_ARCHIVE_APPLY_MIN_CONFIDENCE",
            0.55,
        )?,
        project_id: lookup("SH_CURATOR_PROJECT_ID").unwrap_or_else(|| "default".to_string()),
        org_aggregation_enabled: env_bool_or_default(lookup, "SH_CURATOR_ORG_AGGREGATION", false)?,
    }))
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

    // Story 5.22: strict-parse the verifier rollback flag so a
    // typo doesn't silently flip a release-safety setting.
    let rollback_flag = env_bool_or_default(
        &|k| std::env::var(k).ok(),
        "SEASONED_HAND_ROLLBACK_ON_VERIFIER_FAIL",
        false,
    )
    .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
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
        // Story 5.6: paired system-actor identity for notify writes (used by
        // per-domain stories that wire authorize() into notification paths).
        let notify_auth = seasoned_hand_core::auth::SystemAuth::for_worker(
            std::env::var("SH_NOTIFY_ORGANIZATION_ID")
                .unwrap_or_else(|_| "org-legacy-default".to_string()),
            std::env::var("SH_NOTIFY_TENANT_ID").unwrap_or_else(|_| "legacy-default".to_string()),
            "notify",
        );
        tracing::info!(
            system_actor = %notify_auth.actor_user_id,
            system_tenant = %notify_auth.tenant_id,
            "notify worker spawned",
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
        // Story 5.22: strict bool parse for the learning gate.
        let learning_enabled =
            env_bool_or_default(&|k| std::env::var(k).ok(), "SH_LEARNING_ENABLED", true)
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
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
        // Story 5.6: paired system-actor identity for verifier writes.
        let verifier_auth = seasoned_hand_core::auth::SystemAuth::for_worker(
            std::env::var("SH_VERIFIER_ORGANIZATION_ID")
                .unwrap_or_else(|_| "org-legacy-default".to_string()),
            std::env::var("SH_VERIFIER_TENANT_ID").unwrap_or_else(|_| "legacy-default".to_string()),
            "verifier",
        );
        tracing::info!(
            learning_enabled,
            rollback_on_fail = state.checkpoint_rollback_on_verifier_fail,
            system_actor = %verifier_auth.actor_user_id,
            system_tenant = %verifier_auth.tenant_id,
            "verifier worker + gate spawned",
        );
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
    let env_lookup = |key: &str| std::env::var(key).ok();
    let curator_config = load_curator_config_from_lookup(&env_lookup)
        .map_err(|msg| std::io::Error::new(std::io::ErrorKind::InvalidInput, msg))?;
    let curator_handle = if let Some(config) = curator_config {
        let embedding_slot = state.router.resolve(SlotName::Embedding);
        let embedding_llm = LlmClient::new(
            embedding_slot.base_url.clone(),
            embedding_slot.api_key.clone(),
        );
        // Story 5.18: when org aggregation is on, pin the builder to
        // the worker's tenant (org = tenant per V013); otherwise keep
        // Phase 4 project-scoped behavior.
        let candidate_builder = if config.org_aggregation_enabled {
            let tenant = std::env::var("SH_CURATOR_TENANT_ID")
                .unwrap_or_else(|_| "legacy-default".to_string());
            tracing::info!(
                tenant = %tenant,
                "curator org-wide candidate aggregation enabled",
            );
            std::sync::Arc::new(SqliteCandidateBuilder::new_with_org_aggregation(
                state.db.clone(),
                tenant,
            ))
        } else {
            std::sync::Arc::new(SqliteCandidateBuilder::new(state.db.clone()))
        };
        let consolidation_engine = std::sync::Arc::new(
            SqliteConsolidationEngine::new(state.db.clone()).with_archive_policy(
                config.auto_archive_enabled,
                config.archive_recommend_min_confidence,
                config.archive_apply_min_confidence,
            ),
        );
        let conflict_slot = state.router.resolve(SlotName::SessionSearch);
        let conflict_llm = LlmClient::new(
            conflict_slot.base_url.clone(),
            conflict_slot.api_key.clone(),
        );
        let retrospective_llm = LlmClient::new(
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
        let retrospective_generator = std::sync::Arc::new(SqliteRetrospectiveGenerator::new(
            state.db.clone(),
            retrospective_llm,
            conflict_slot.model.clone(),
        ));
        let work_pattern_extractor =
            std::sync::Arc::new(SqliteWorkPatternExtractor::new(state.db.clone()));
        let operator_review_queue =
            std::sync::Arc::new(SqliteOperatorReviewQueue::new(state.db.clone()));
        let enforce_l2_knowledge =
            env_bool_or_default(&env_lookup, "SH_CURATOR_L2_ENFORCE_KNOWLEDGE", true)
                .map_err(|msg| std::io::Error::new(std::io::ErrorKind::InvalidInput, msg))?;
        let enforce_l2_datasource =
            env_bool_or_default(&env_lookup, "SH_CURATOR_L2_ENFORCE_DATASOURCE", true)
                .map_err(|msg| std::io::Error::new(std::io::ErrorKind::InvalidInput, msg))?;
        let knowledge_datasource_writer =
            std::sync::Arc::new(SqliteKnowledgeDatasourceWriter::new(
                state.db.clone(),
                enforce_l2_knowledge,
                enforce_l2_datasource,
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
            CuratorRuntimeDeps {
                candidate_builder,
                reranker,
                consolidation_engine,
                conflict_detector,
                retrospective_generator,
                work_pattern_extractor,
                operator_review_queue,
                knowledge_datasource_writer,
            },
            config.max_candidates_per_cycle,
            config.backlog_threshold,
        ));
        let worker = ProductionCuratorWorker::new(
            config.clone(),
            state.db.clone(),
            state.events.clone(),
            std::sync::Arc::new(SqliteBacklogProbe::new(state.db.clone())),
            executor,
        );
        let token = curator_shutdown.clone();
        // Story 5.6: construct the curator worker's system-actor AuthContext.
        // Per-domain stories (5.17 curator tenant boundaries) will thread this
        // through `authorize(...)` for cross-tenant guards; here we capture it
        // at the spawn boundary so log lines + future audit_log rows attribute
        // every curator write to a stable system identity.
        let curator_auth = seasoned_hand_core::auth::SystemAuth::for_worker(
            std::env::var("SH_CURATOR_ORGANIZATION_ID")
                .unwrap_or_else(|_| "org-legacy-default".to_string()),
            std::env::var("SH_CURATOR_TENANT_ID").unwrap_or_else(|_| "legacy-default".to_string()),
            "curator",
        );
        tracing::info!(
            project_id = %config.project_id,
            interval_seconds = config.interval_seconds,
            backlog_threshold = config.backlog_threshold,
            auto_archive_enabled = config.auto_archive_enabled,
            system_actor = %curator_auth.actor_user_id,
            system_tenant = %curator_auth.tenant_id,
            "curator worker spawned",
        );
        Some(tokio::spawn(async move {
            if let Err(error) = worker.run(token).await {
                tracing::error!(%error, "curator worker exited with error");
            }
        }))
    } else {
        tracing::info!("curator worker not spawned (SH_CURATOR_ENABLED=false)");
        None
    };

    // Story 4.23 / Phase 4 close-out hardening iter-1: retention scheduler
    // runs on the same enabled gate as the curator worker — no curator
    // means no telemetry growth means no retention work. Default interval
    // is 24h; SH_CURATOR_RETENTION_INTERVAL_SEC overrides via the strict
    // numeric parser introduced by story 4.14.
    let retention_shutdown = curator_shutdown.clone();
    let retention_handle = if curator_handle.is_some() {
        let retention_project_id =
            std::env::var("SH_CURATOR_PROJECT_ID").unwrap_or_else(|_| "default".to_string());
        let interval_secs = env_u64_or_default(
            &env_lookup,
            "SH_CURATOR_RETENTION_INTERVAL_SEC",
            DEFAULT_RETENTION_INTERVAL_SECS,
        )
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
        let job = std::sync::Arc::new(CuratorRetentionJob::new(
            state.db.clone(),
            state.events.clone(),
            RetentionConfig::for_project(retention_project_id.clone()),
        ));
        let scheduler = RetentionScheduler::new(job, std::time::Duration::from_secs(interval_secs));
        // Story 5.6: paired system-actor identity for retention writes.
        let retention_auth = seasoned_hand_core::auth::SystemAuth::for_worker(
            std::env::var("SH_CURATOR_ORGANIZATION_ID")
                .unwrap_or_else(|_| "org-legacy-default".to_string()),
            std::env::var("SH_CURATOR_TENANT_ID").unwrap_or_else(|_| "legacy-default".to_string()),
            "retention",
        );
        tracing::info!(
            project_id = %retention_project_id,
            interval_seconds = interval_secs,
            system_actor = %retention_auth.actor_user_id,
            system_tenant = %retention_auth.tenant_id,
            "curator retention scheduler spawned",
        );
        Some(tokio::spawn(async move {
            scheduler.run(retention_shutdown).await;
        }))
    } else {
        None
    };

    // Story 2.17 / Phase 0 DEBT #16: spawn the workspace TTL cron.
    // Single-task loop that wakes every `SANDBOX_CLEANUP_INTERVAL_SEC`
    // (default 3600), tears down container + workspace for terminal-
    // state tasks past their per-status TTL. Active tasks
    // (running/paused) are never GC'd. Failures within the cycle are
    // absorbed; the loop itself never exits with Err.
    // Story 5.12: spawn the user_cost_ledger nearline writer. Tick
    // cadence defaults to 1h; `SH_USER_COST_INTERVAL_SEC` strict-parsed
    // override matches the `_INTERVAL_SEC` family.
    let user_cost_shutdown = tokio_util::sync::CancellationToken::new();
    let user_cost_handle = {
        let interval_secs = env_u64_or_default(
            &env_lookup,
            "SH_USER_COST_INTERVAL_SEC",
            seasoned_hand_core::billing::user_cost::DEFAULT_USER_COST_INTERVAL_SECS,
        )
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
        let writer = seasoned_hand_core::billing::NearlineWriter::new(state.db.clone());
        let token = user_cost_shutdown.clone();
        let user_cost_auth = seasoned_hand_core::auth::SystemAuth::for_worker(
            std::env::var("SH_USER_COST_ORGANIZATION_ID")
                .unwrap_or_else(|_| "org-legacy-default".to_string()),
            std::env::var("SH_USER_COST_TENANT_ID")
                .unwrap_or_else(|_| "legacy-default".to_string()),
            "user-cost",
        );
        tracing::info!(
            interval_seconds = interval_secs,
            system_actor = %user_cost_auth.actor_user_id,
            system_tenant = %user_cost_auth.tenant_id,
            "user_cost_ledger nearline writer spawned",
        );
        tokio::spawn(async move {
            writer
                .run(std::time::Duration::from_secs(interval_secs), token)
                .await;
        })
    };
    let user_cost_reconcile_shutdown = tokio_util::sync::CancellationToken::new();
    let user_cost_reconcile_handle = {
        let interval_secs = env_u64_or_default(
            &env_lookup,
            "SH_USER_COST_RECONCILE_INTERVAL_SEC",
            seasoned_hand_core::billing::DEFAULT_USER_COST_RECONCILE_INTERVAL_SECS,
        )
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
        let job = seasoned_hand_core::billing::ReconciliationJob::new(
            state.db.clone(),
            state.events.clone(),
        );
        let token = user_cost_reconcile_shutdown.clone();
        let reconcile_auth = seasoned_hand_core::auth::SystemAuth::for_worker(
            std::env::var("SH_USER_COST_RECONCILE_ORGANIZATION_ID")
                .unwrap_or_else(|_| "org-legacy-default".to_string()),
            std::env::var("SH_USER_COST_RECONCILE_TENANT_ID")
                .unwrap_or_else(|_| "legacy-default".to_string()),
            "user-cost-reconcile",
        );
        tracing::info!(
            interval_seconds = interval_secs,
            system_actor = %reconcile_auth.actor_user_id,
            system_tenant = %reconcile_auth.tenant_id,
            "user_cost reconciliation job spawned",
        );
        tokio::spawn(async move {
            job.run_daily(std::time::Duration::from_secs(interval_secs), token)
                .await;
        })
    };

    let ttl_shutdown = tokio_util::sync::CancellationToken::new();
    let ttl_handle = {
        let cron = state.workspace_ttl_cron.clone();
        let token = ttl_shutdown.clone();
        // Story 5.6: paired system-actor identity for ttl-cron writes.
        let ttl_auth = seasoned_hand_core::auth::SystemAuth::for_worker(
            std::env::var("SH_TTL_ORGANIZATION_ID")
                .unwrap_or_else(|_| "org-legacy-default".to_string()),
            std::env::var("SH_TTL_TENANT_ID").unwrap_or_else(|_| "legacy-default".to_string()),
            "ttl",
        );
        tracing::info!(
            system_actor = %ttl_auth.actor_user_id,
            system_tenant = %ttl_auth.tenant_id,
            "workspace ttl cron spawned",
        );
        tokio::spawn(async move {
            cron.run(token).await;
        })
    };

    let addr = bind_addr()?;
    tracing::info!(%addr, %database_url, %redis_url, "seasoned-hand-server starting");

    // SEC-IT1-H1 / issue #7 / ADR-018: identity is verified against
    // `auth_sessions` (opaque bearer/subprotocol token) by default. The legacy
    // plaintext `x-seasoned-hand-*` header path is accepted ONLY when
    // `SH_INSECURE_AUTH_HEADERS` is set, for loopback dev / tests / CLI. Warn on
    // both risky postures: a non-loopback bind, and the insecure-headers flag.
    if !addr.ip().is_loopback() {
        tracing::warn!(
            %addr,
            "SECURITY: binding a non-loopback address. Expose this port only \
             behind a trusted gateway; clients authenticate via /v1/auth/login \
             session tokens (ADR-018). See SECURITY.md."
        );
    }
    if std::env::var("SH_INSECURE_AUTH_HEADERS")
        .map(|v| matches!(v.trim(), "1" | "true" | "yes"))
        .unwrap_or(false)
    {
        tracing::warn!(
            "SECURITY: SH_INSECURE_AUTH_HEADERS is enabled — unverified \
             x-seasoned-hand-* identity headers are accepted. Intended for \
             loopback dev / tests / CLI only; do NOT enable in production."
        );
    }

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
    if let Some(handle) = retention_handle {
        log_join_error("curator-retention", handle.await);
    }

    user_cost_shutdown.cancel();
    log_join_error("user-cost", user_cost_handle.await);
    user_cost_reconcile_shutdown.cancel();
    log_join_error("user-cost-reconcile", user_cost_reconcile_handle.await);

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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{env_bool_or_default, load_curator_config_from_lookup};
    use seasoned_hand_core::curator::EmbeddingBudget;

    fn lookup_from(map: HashMap<&'static str, &'static str>) -> impl Fn(&str) -> Option<String> {
        move |key| map.get(key).map(|value| (*value).to_string())
    }

    #[test]
    fn curator_config_strict_parsing_accepts_valid_values() {
        let lookup = lookup_from(HashMap::from([
            ("SH_CURATOR_ENABLED", "true"),
            ("SH_CURATOR_INTERVAL_SECONDS", "120"),
            ("SH_CURATOR_BACKLOG_THRESHOLD", "5"),
            ("SH_CURATOR_MAX_CANDIDATES_PER_CYCLE", "25"),
            ("SH_CURATOR_EMBEDDING_BUDGET_MONTHLY_TOKENS", "100000"),
            ("SH_CURATOR_EMBEDDING_SOFT_CAP_PCT", "0.05"),
            ("SH_CURATOR_EMBEDDING_HARD_BREAKER_PCT", "0.10"),
            ("SH_CURATOR_AUTO_ARCHIVE_ENABLED", "1"),
            ("SH_CURATOR_ARCHIVE_RECOMMEND_MIN_CONFIDENCE", "0.45"),
            ("SH_CURATOR_ARCHIVE_APPLY_MIN_CONFIDENCE", "0.60"),
            ("SH_CURATOR_PROJECT_ID", "proj-x"),
        ]));
        let cfg = load_curator_config_from_lookup(&lookup)
            .expect("valid config parse")
            .expect("enabled config");
        assert_eq!(cfg.interval_seconds, 120);
        assert_eq!(cfg.backlog_threshold, 5);
        assert_eq!(cfg.max_candidates_per_cycle, 25);
        assert_eq!(cfg.embedding_budget_monthly_tokens, 100_000);
        assert_eq!(cfg.embedding_budget_soft_cap_pct, 0.05);
        assert_eq!(cfg.embedding_budget_hard_breaker_pct, 0.10);
        assert!(cfg.auto_archive_enabled);
        assert_eq!(cfg.project_id, "proj-x");
    }

    #[test]
    fn curator_config_strict_parsing_rejects_invalid_boolean() {
        let lookup = lookup_from(HashMap::from([("SH_CURATOR_ENABLED", "yes")]));
        let error = load_curator_config_from_lookup(&lookup).expect_err("should reject");
        assert!(error.contains("SH_CURATOR_ENABLED"));
    }

    #[test]
    fn curator_config_strict_parsing_rejects_invalid_caps() {
        let lookup = lookup_from(HashMap::from([
            ("SH_CURATOR_ENABLED", "1"),
            ("SH_CURATOR_EMBEDDING_SOFT_CAP_PCT", "0.2"),
            ("SH_CURATOR_EMBEDDING_HARD_BREAKER_PCT", "0.1"),
        ]));
        let error = load_curator_config_from_lookup(&lookup).expect_err("should reject");
        assert!(error.contains("HARD_BREAKER"));
        assert!(error.contains("SOFT_CAP"));
    }

    #[test]
    fn curator_l2_flags_are_strict_boolean() {
        let lookup = lookup_from(HashMap::from([("SH_CURATOR_L2_ENFORCE_KNOWLEDGE", "nope")]));
        let result = env_bool_or_default(&lookup, "SH_CURATOR_L2_ENFORCE_KNOWLEDGE", true);
        assert!(result.is_err());
    }

    #[test]
    fn embedding_budget_zero_baseline_fallback_is_absolute_cap() {
        let budget = EmbeddingBudget {
            monthly_embedding_tokens: 50_000,
            soft_cap_pct: 0.08,
            hard_breaker_pct: 0.12,
        };
        assert!(!budget.soft_cap_exceeded(49_999, 0));
        assert!(budget.soft_cap_exceeded(50_000, 0));
        assert!(!budget.breaker_open(49_999, 0));
        assert!(budget.breaker_open(50_000, 0));
    }
}
