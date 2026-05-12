//! Seasoned Hand server binary.
//! refs: /specs/phase-0/architecture.md §4.1

use std::net::SocketAddr;
use std::path::PathBuf;

use seasoned_hand_core::capability::{
    CapabilityProbe, assert_main_supports_tool_calling, warn_implied_slot_capability_mismatches,
};
use seasoned_hand_core::llm::LlmClient;
use seasoned_hand_core::router::{SlotName, SlotRouter};
use seasoned_hand_core::sandbox::SandboxClient;
use seasoned_hand_core::search::SearchClient;
use seasoned_hand_core::{db, pubsub};
use seasoned_hand_server::{AppState, app};
use tracing_subscriber::EnvFilter;

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

    let state = AppState::new(db, redis, sandbox, search, router, capabilities);

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

    let addr = bind_addr()?;
    tracing::info!(%addr, %database_url, %redis_url, "seasoned-hand-server starting");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;

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
