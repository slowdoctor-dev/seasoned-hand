//! Seasoned Hand HTTP server.
//! refs: /specs/phase-0/architecture.md §4.1

use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    middleware,
    response::IntoResponse,
    routing::{MethodRouter, get},
};
use dashmap::DashMap;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::timeout::TimeoutLayer;

use seasoned_hand_core::agent::breaker::BreakerRegistry;
use seasoned_hand_core::agent::init::briefing::UserResponse;
use seasoned_hand_core::agent::init::feature_list::FeatureList;
use seasoned_hand_core::agent::init::progress;
use seasoned_hand_core::agent::narrate::NarratorHook;
use seasoned_hand_core::agent::{AgentRunner, AgentRunnerDeps};
use seasoned_hand_core::audit::{AuditLogger, AuditQuery};
use seasoned_hand_core::auth::{Action, AuthContext, authorize_coarse};
use seasoned_hand_core::billing::{ReconciliationJob, ReconciliationReport};
use seasoned_hand_core::browser::tracks::PostBrowserActionHook;
use seasoned_hand_core::capability::ModelCapabilities;
use seasoned_hand_core::channel::{
    ChannelRegistration, ChannelRegistry,
    chat::ChatChannel,
    cli::CliChannel,
    email::{
        AllowList, AsyncImapFetcher, EmailChannel, ImapConfig, LettreSmtpTransport, SmtpConfig,
    },
    ntfy::NtfyChannel,
    webhook::{TokenCheck, WebhookChannel},
};
use seasoned_hand_core::cost::{CostClient, CostSnapshot};
use seasoned_hand_core::db::DbPool;
// (DeliverableStore imported via the broader `deliverable::` use below.)
use seasoned_hand_core::deliverable::{
    DeliverableStore, PlannerSimplifyLlm, RendererDispatcher, TaskDeliverDeps,
};
use seasoned_hand_core::delivery::{DeliveryEventStore, DeliveryRouter};
use seasoned_hand_core::dispatch::mask::DefaultMaskPolicy;
use seasoned_hand_core::dispatch::{
    ToolDispatcher,
    hooks::{EventEmittingHook, InvalidationHook},
};
use seasoned_hand_core::events::{EventQuery, EventStore, EventType, sqlite::SqliteEventStore};
use seasoned_hand_core::handoff::{HandoffRequest, TaskHandoffService};
use seasoned_hand_core::intake::{IntakeEventStore, IntakeRouter};
use seasoned_hand_core::llm::LlmClient;
use seasoned_hand_core::notify::{NotificationsSentStore, NotifyConfig};
use seasoned_hand_core::org::{InvitationError, InvitationService, InviteOutcome, MembershipRow};
use seasoned_hand_core::plan::PlanManager;
use seasoned_hand_core::project::{ProjectStore, TaskStore};
use seasoned_hand_core::pubsub::RedisPool;
use seasoned_hand_core::router::{SlotName, SlotRouter};
use seasoned_hand_core::sandbox::SandboxClient;
use seasoned_hand_core::search::SearchClient;
use seasoned_hand_core::sharing::sop::{SopPermission, SopShareError, SopShareService};
use seasoned_hand_core::tools::builtin::all_with_task_deliver;
use seasoned_hand_core::verifier::{
    VerificationStore,
    routes::{ListQuery as VerifyListQuery, get_verification, list_verifications},
};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

mod auth;
pub mod initializer_spawner;
pub mod ws;

pub use initializer_spawner::WsInitializerSpawner;

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
    /// Issue #7 / ADR-018: opaque session-token store. Powers `/v1/auth/login`
    /// and the DB-backed verification in the auth middleware.
    pub auth_sessions: Arc<seasoned_hand_core::auth::AuthSessionStore>,
    /// Issue #7 / ADR-018: when `true` (`SH_INSECURE_AUTH_HEADERS` set), the
    /// legacy client-asserted `x-seasoned-hand-*` header path is accepted for
    /// loopback dev / tests / CLI. Default `false` → only verified session tokens
    /// authenticate.
    pub allow_insecure_headers: bool,
    /// Story 1.13b: opt-in flag that lets the VerifierGate trigger a
    /// checkpoint rollback when a verdict carries
    /// `rollback_required: true`. Default `false` per phase-1/DEBT.md #3
    /// — Phase 2 retrospective will decide whether to flip this.
    pub checkpoint_rollback_on_verifier_fail: bool,
    /// Story 1.17: per-session cancellation tokens used by ws task_cancel.
    pub cancel_tokens: Arc<DashMap<String, tokio_util::sync::CancellationToken>>,
    pub breakers: Arc<BreakerRegistry>,
    /// Story 2.20: the same `NarratorHook` Arc that's wired into the
    /// `dispatcher`'s hook chain. Exposed here so `with_narrator_classifier`
    /// can call `attach_classifier(...)` on it after AppState
    /// construction — closes Phase 1 1.15 deferred plumbing.
    pub narrator: Arc<NarratorHook>,
    /// Story 2.2: V006 `projects` persistence handle.
    pub projects: Arc<ProjectStore>,
    /// Story 2.2: V006 `tasks` persistence handle (state-machine
    /// guarded transitions).
    pub tasks: Arc<TaskStore>,
    /// Story 2.3: V007 `deliverables` persistence handle. HTTP routes
    /// land in 2.10 / 2.15 / 2.22.
    pub deliverables: Arc<DeliverableStore>,
    /// Story 2.3: V008 `intake_events` persistence handle. Consumed by
    /// the IntakeRouter (story 2.5).
    pub intake_events: Arc<IntakeEventStore>,
    /// Story 2.3: V008 `delivery_events` persistence handle. Consumed
    /// by the DeliveryRouter (story 2.5).
    pub delivery_events: Arc<DeliveryEventStore>,
    /// Story 2.3: V008 `notifications_sent` persistence handle.
    /// Consumed by the NotifyWorker (story 2.5).
    pub notifications_sent: Arc<NotificationsSentStore>,
    /// Story 2.4 / 2.10: registered channels (intake / delivery /
    /// notify roles). Built with the always-on chat baseline by
    /// `AppState::new`; main.rs adds further channels at boot via
    /// `register_channel` (story 2.10 — DEBT #17 pay-down replaces the
    /// previous `with_channels` builder which silently dropped the
    /// chat baseline).
    pub channels: Arc<ChannelRegistry>,
    /// Story 2.10: shared `Arc<String>` mirroring the same allocation
    /// stored inside the registered `WebhookChannel`. The webhook
    /// intake HTTP handler (`POST /v1/intake/webhook`) reads this
    /// directly to gate access — keeping a top-level handle avoids
    /// downcasting the channel registry's `Arc<dyn IntakeProvider>`.
    /// Empty when `SEASONED_HAND_INTAKE_TOKEN` is unset; the handler
    /// then returns 503 `intake_token_not_configured`.
    pub webhook_intake_token: Arc<String>,
    /// Story 2.5: IntakeRouter consumes the channel-framework mpsc
    /// and seeds Tasks. Built referencing `self.channels`;
    /// `with_channels` rebuilds it so the registry Arc stays in sync.
    pub intake_router: Arc<IntakeRouter>,
    /// Story 2.5: DeliveryRouter dispatches completed deliverables.
    /// Built referencing `self.channels`; rebuilt by `with_channels`.
    pub delivery_router: Arc<DeliveryRouter>,
    /// Story 2.8b (Phase 2 DEBT #13 close-out): per-task mpsc sender
    /// map keyed by `task_id`. The
    /// [`WsInitializerSpawner`](crate::WsInitializerSpawner) inserts on
    /// briefing-gate spawn; the WS `briefing_confirm` cmd handler
    /// reads to forward `UserResponse` envelopes; the spawner's
    /// background task removes once the confirm gate returns.
    ///
    /// Key choice: **task_id**, not `briefing_call_id`. The Initializer
    /// holds one `mpsc::Receiver<UserResponse>` per task — each `edit`
    /// action mints a fresh `briefing_call_id` but reuses the same
    /// receiver, so per-call-id keying would require an additional
    /// indirection without buying anything.
    pub briefing_senders: Arc<DashMap<String, tokio::sync::mpsc::Sender<UserResponse>>>,
    /// Story 2.12: per-trigger notify routing + per-channel default
    /// targets, parsed from `config/notify.toml` at boot. Empty when
    /// the file is missing — the listener silently skips every
    /// trigger and no notifies are emitted (a clean default for
    /// operators who don't want push notifications).
    pub notify_config: Arc<NotifyConfig>,
    /// Story 2.17 / Phase 0 DEBT #16: workspace-TTL cron. Spawned in
    /// `main.rs` under a shutdown token; the
    /// `POST /v1/admin/sandbox/cleanup` route uses this same handle to
    /// run a single cycle on demand.
    pub workspace_ttl_cron:
        Arc<seasoned_hand_core::task::WorkspaceTtlCron<seasoned_hand_core::sandbox::SandboxClient>>,
    /// Story 2.21a / Phase 2 DEBT #23: registered `CliChannel` shared
    /// between (a) the in-process `register_pending` / `submit` site
    /// the CLI binary will use when it shares an `AppState` with the
    /// server (story 2.21b `task new --blocking`) and (b) the
    /// `DeliveryRouter`'s `cli` reply_target slot. None until
    /// [`AppState::register_cli_channel`] runs from `main.rs`; the
    /// channel itself is harmless to register early — its
    /// `IntakeProvider::run` parks on shutdown.
    pub cli_channel: Arc<CliChannel>,
    /// Issue #33: when `Some`, the control plane serves the built Dioxus UI
    /// bundle at this directory as the router fallback (static assets + SPA
    /// `index.html`), so a single binary self-hosts both the `/v1` + `/ws` API
    /// and the web UI. `None` (the default, and whenever `SH_UI_DIST` is unset)
    /// leaves the server API-only — unmatched paths 404 as before, which is what
    /// every test relies on. Set from `SH_UI_DIST` in `main.rs`
    /// ([`AppState::with_ui_dist`]); the env path is validated at boot.
    pub ui_dist: Option<std::path::PathBuf>,
}

/// Story 2.20: configuration bundle for the NarratorHook's
/// classifier-slot LLM path. Constructed by `main.rs` after loading
/// `config/prompts/narrator.system.txt` and resolving the Classifier
/// slot; applied via `AppState::with_narrator_classifier`.
pub struct NarratorClassifierWiring {
    pub llm: Arc<LlmClient>,
    pub model: String,
    pub system_prompt: Arc<String>,
}

/// Story 2.11: env-shaped inputs for [`AppState::register_email_channel`].
/// `from_env()` reads the operator's environment so `main.rs` is one
/// line; tests construct directly to avoid racing on process state.
///
/// IMAP_HOST / IMAP_USERNAME / IMAP_PASSWORD are mandatory — leaving any
/// blank disables the whole channel (default-deny by absence). SMTP
/// envs default to the IMAP host on port 587 with the same credentials
/// (matches the common single-mailbox setup).
pub struct EmailChannelEnv {
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_username: String,
    pub imap_password: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_username: String,
    pub smtp_password: String,
    pub from_address: String,
    pub subject_prefix: String,
    pub allowed_senders_raw: String,
    pub poll_interval_secs: u64,
}

/// Resolved [`EmailChannel`] config. Internal — produced by
/// [`EmailChannelEnv::into_config`] only.
struct EmailChannelConfig {
    imap: ImapConfig,
    smtp: SmtpConfig,
    from_address: String,
    subject_prefix: String,
    allow_list: AllowList,
    poll_interval: std::time::Duration,
}

impl EmailChannelEnv {
    /// Read every relevant env var. Missing IMAP_PORT / SMTP_PORT
    /// fall to 993 / 587; missing IMAP_POLL_INTERVAL_SECS to 30.
    /// Missing INTAKE_EMAIL_ALLOWED_SENDERS stays empty — the
    /// `EmailChannel` itself enforces default-deny on an empty list
    /// (architecture §9 / phase-2/DEBT.md #4).
    pub fn from_env() -> Self {
        let imap_host = std::env::var("IMAP_HOST").unwrap_or_default();
        let imap_username = std::env::var("IMAP_USERNAME").unwrap_or_default();
        let imap_password = std::env::var("IMAP_PASSWORD").unwrap_or_default();
        let imap_port = std::env::var("IMAP_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(993);

        let smtp_host = std::env::var("SMTP_HOST").unwrap_or_else(|_| imap_host.clone());
        let smtp_username =
            std::env::var("SMTP_USERNAME").unwrap_or_else(|_| imap_username.clone());
        let smtp_password =
            std::env::var("SMTP_PASSWORD").unwrap_or_else(|_| imap_password.clone());
        let smtp_port = std::env::var("SMTP_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(587);

        // FROM_ADDRESS defaults to the SMTP/IMAP username (typically
        // the mailbox itself).
        let from_address =
            std::env::var("EMAIL_FROM_ADDRESS").unwrap_or_else(|_| smtp_username.clone());
        let subject_prefix = std::env::var("EMAIL_SUBJECT_PREFIX")
            .unwrap_or_else(|_| seasoned_hand_core::channel::email::DEFAULT_SUBJECT_PREFIX.into());
        let allowed_senders_raw = std::env::var("INTAKE_EMAIL_ALLOWED_SENDERS").unwrap_or_default();
        let poll_interval_secs = std::env::var("IMAP_POLL_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);

        Self {
            imap_host,
            imap_port,
            imap_username,
            imap_password,
            smtp_host,
            smtp_port,
            smtp_username,
            smtp_password,
            from_address,
            subject_prefix,
            allowed_senders_raw,
            poll_interval_secs,
        }
    }

    fn into_config(self) -> Option<EmailChannelConfig> {
        if self.imap_host.is_empty()
            || self.imap_username.is_empty()
            || self.imap_password.is_empty()
        {
            return None;
        }
        let allow_list = match AllowList::parse(&self.allowed_senders_raw) {
            Ok(al) => al,
            Err(error) => {
                tracing::warn!(
                    %error,
                    "INTAKE_EMAIL_ALLOWED_SENDERS parse failed; falling back to deny-all",
                );
                AllowList::default()
            }
        };
        let from_address = if self.from_address.is_empty() {
            self.imap_username.clone()
        } else {
            self.from_address
        };
        Some(EmailChannelConfig {
            imap: ImapConfig {
                host: self.imap_host,
                port: self.imap_port,
                username: self.imap_username,
                password: self.imap_password,
            },
            smtp: SmtpConfig {
                host: self.smtp_host,
                port: self.smtp_port,
                username: self.smtp_username,
                password: self.smtp_password,
            },
            from_address,
            subject_prefix: self.subject_prefix,
            allow_list,
            poll_interval: std::time::Duration::from_secs(self.poll_interval_secs.max(1)),
        })
    }
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
        // Story 1.15: NarratorHook runs first so the
        // `Message{ui:"narrate"}` event lands before the Action event
        // for clean UI ordering. Templated-only at this point;
        // classifier-slot LLM path becomes live the moment
        // `with_narrator_classifier` writes the OnceLock (story 2.20).
        // The same Arc lives in both `state.narrator` and the
        // dispatcher's hook chain, so the boot-time `attach_classifier`
        // call mutates the in-chain hook directly.
        let narrator = Arc::new(NarratorHook::new(events.clone()));

        // Story 2.14: build the Deliverable store + RendererDispatcher
        // BEFORE the ToolDispatcher so `task_deliver` lands in the
        // catalog with its production deps. The other Phase 2 stores
        // are built here too — moving them up the file (they used to
        // sit below the AgentRunner) keeps every share-from-`db` store
        // in one block.
        let projects = Arc::new(seasoned_hand_core::project::ProjectStore::new(db.clone()));
        let tasks_store = Arc::new(seasoned_hand_core::project::TaskStore::new(db.clone()));
        let deliverables = Arc::new(DeliverableStore::new(db.clone()));
        let intake_events = Arc::new(IntakeEventStore::new(db.clone()));
        let delivery_events = Arc::new(DeliveryEventStore::new(db.clone()));
        let notifications_sent = Arc::new(NotificationsSentStore::new(db.clone()));
        let renderer = Arc::new(RendererDispatcher::new(sandbox.clone()));
        // Story 2.15: provenance builder needs the verifier + checkpoint
        // stores. Lifted ahead of `task_deliver_deps` so it can hand the
        // ProvenanceDeps bundle to `TaskDeliverDeps`.
        let verifications = Arc::new(VerificationStore::new(db.clone()));
        let checkpoints = Arc::new(seasoned_hand_core::checkpoint::CheckpointStore::new(
            db.clone(),
        ));
        let task_deliver_deps = TaskDeliverDeps {
            deliverables: deliverables.clone(),
            renderer: renderer.clone(),
            db: db.clone(),
            planner_llm: Some(Arc::new(PlannerSimplifyLlm::from_router(&router))),
            provenance: seasoned_hand_core::deliverable::task_deliver::ProvenanceDeps {
                task_store: tasks_store.clone(),
                project_store: projects.clone(),
                intake_store: intake_events.clone(),
                delivery_store: delivery_events.clone(),
                events: events.clone(),
                verifications: verifications.clone(),
                checkpoints: checkpoints.clone(),
            },
        };

        let dispatcher = Arc::new(
            ToolDispatcher::new(all_with_task_deliver(task_deliver_deps))
                .with_hook(narrator.clone())
                .with_hook(Arc::new(EventEmittingHook::new(events.clone())))
                .with_hook(Arc::new(InvalidationHook::new(
                    events.clone(),
                    Some(redis_arc.clone()),
                )))
                .with_hook(Arc::new(PostBrowserActionHook::new(events.clone()))),
        );
        let verifier_enabled = router.verifier_enabled();
        let checkpoint_labels =
            Arc::new(seasoned_hand_core::checkpoint::CheckpointLabelBuffer::new());
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
        // Story 2.9: ChatChannel wraps the existing WS as both an
        // IntakeProvider (no-op `run`; the WS server pushes IntakeEvents
        // synchronously via `intake_router.handle_event`) and a
        // DeliverySink (appends a `Misc{kind:"Deliverable"}` event that
        // the WS payload renderer reshapes per architecture §4). No
        // NotifySink — chat has no push-notify semantics distinct from
        // regular messages.
        //
        // Other concrete channels (webhook, email, cli, ntfy) land in
        // stories 2.10–2.13; main.rs will need a `with_channels` story
        // that *merges* additional registrations on top of the chat
        // baseline rather than replacing the registry wholesale.
        let mut channels = ChannelRegistry::new();
        let chat = Arc::new(ChatChannel::new(events.clone()));
        channels.register(
            ChannelRegistration::new("chat")
                .with_intake(chat.clone())
                .with_delivery(chat),
        );
        let channels = Arc::new(channels);
        // Story 2.5: routers reference the (empty for now) registry
        // Arc. `with_channels` swaps the registry and rebuilds both
        // routers so the slot stays consistent.
        let intake_router = Arc::new(IntakeRouter::new(
            intake_events.clone(),
            tasks_store.clone(),
            projects.clone(),
            channels.clone(),
        ));
        let delivery_router = Arc::new(DeliveryRouter::new(
            channels.clone(),
            delivery_events.clone(),
            deliverables.clone(),
            intake_events.clone(),
            events.clone(),
            db.clone(),
        ));
        let briefing_senders = Arc::new(DashMap::new());
        let notify_config = Arc::new(NotifyConfig::empty());
        // Story 2.17: workspace TTL cron. Configuration is env-driven
        // (`SANDBOX_CLEANUP_INTERVAL_SEC`, `SANDBOX_TTL_*_DAYS`) — tests
        // don't set those so the production defaults apply, but the
        // cron only fires from `main.rs` (it isn't spawned by
        // `AppState::new`), so test runs never trigger a cycle.
        let workspace_ttl_cron = Arc::new(seasoned_hand_core::task::WorkspaceTtlCron::new(
            tasks_store.clone(),
            events.clone(),
            sandbox.clone(),
            db.clone(),
            seasoned_hand_core::task::TtlConfig::from_env(),
        ));
        // Issue #7 / ADR-018: verified-session store + the insecure-headers
        // escape hatch (default off → client-asserted headers are not trusted).
        let auth_sessions = Arc::new(seasoned_hand_core::auth::AuthSessionStore::new(db.clone()));
        let allow_insecure_headers = std::env::var("SH_INSECURE_AUTH_HEADERS")
            .map(|v| matches!(v.trim(), "1" | "true" | "yes"))
            .unwrap_or(false);

        let state = Self {
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
            auth_sessions,
            allow_insecure_headers,
            checkpoint_rollback_on_verifier_fail,
            cancel_tokens,
            breakers,
            narrator,
            projects,
            tasks: tasks_store,
            deliverables,
            intake_events,
            delivery_events,
            notifications_sent,
            channels,
            intake_router,
            delivery_router,
            webhook_intake_token: Arc::new(String::new()),
            briefing_senders,
            notify_config,
            workspace_ttl_cron,
            cli_channel: Arc::new(CliChannel::new()),
            ui_dist: None,
        };
        // Story 2.8b: attach the confirm-gate Initializer spawner so
        // every Created intake event flows through to the briefing
        // protocol. The spawner clones `state` for its own use — that's
        // safe because `AppState` is `Clone` (all fields are `Arc`-shaped)
        // and `OnceLock::set` is atomic.
        attach_initializer_spawner(&state);
        state
    }

    /// Story 2.10 / DEBT #17: merge one channel registration on top of
    /// the existing registry (which already carries the chat baseline
    /// from `AppState::new`). Replaces the previous `with_channels`
    /// builder, which swapped the whole registry and silently dropped
    /// pre-registered channels.
    ///
    /// Rebuilds `intake_router` + `delivery_router` so both hold the
    /// freshly-populated registry Arc — the routers store the registry
    /// by `Arc<ChannelRegistry>`, so the previous Arc would otherwise
    /// stay live in their slots.
    pub fn register_channel(mut self, registration: ChannelRegistration) -> Self {
        // Move the existing entries into a fresh registry, then add
        // the new registration on top. We can't mutate
        // `Arc<ChannelRegistry>` directly because the routers hold
        // their own Arc clones — building a fresh registry + Arc and
        // re-pointing the routers is the only consistent path.
        let mut next = ChannelRegistry::new();
        for health in self.channels.health() {
            let name = health.name.clone();
            let mut reg = ChannelRegistration::new(&name);
            if let Some(p) = self.channels.get_intake(&name) {
                reg = reg.with_intake(p);
            }
            if let Some(s) = self.channels.get_delivery(&name) {
                reg = reg.with_delivery(s);
            }
            if let Some(s) = self.channels.get_notify(&name) {
                reg = reg.with_notify(s);
            }
            next.register(reg);
        }
        next.register(registration);
        self.channels = Arc::new(next);
        self.intake_router = Arc::new(IntakeRouter::new(
            self.intake_events.clone(),
            self.tasks.clone(),
            self.projects.clone(),
            self.channels.clone(),
        ));
        self.delivery_router = Arc::new(DeliveryRouter::new(
            self.channels.clone(),
            self.delivery_events.clone(),
            self.deliverables.clone(),
            self.intake_events.clone(),
            self.events.clone(),
            self.db.clone(),
        ));
        // Story 2.8b: the IntakeRouter we just rebuilt has a fresh
        // (empty) OnceLock for the spawner — re-attach so chat-baseline
        // + every subsequent channel registration keep flowing through
        // the briefing-confirm Initializer path.
        attach_initializer_spawner(&self);
        self
    }

    /// Story 2.10: register the production `WebhookChannel` (intake +
    /// delivery + notify) and snapshot the intake token onto AppState
    /// so the `POST /v1/intake/webhook` route handler can read it
    /// without downcasting the registry's `Arc<dyn IntakeProvider>`.
    /// Same `Arc<String>` is shared with the channel itself.
    pub fn register_webhook_channel(
        mut self,
        intake_token: Arc<String>,
        allowlist: Vec<ipnet::IpNet>,
    ) -> Self {
        let channel = Arc::new(WebhookChannel::with_default_client(
            intake_token.clone(),
            allowlist,
        ));
        self.webhook_intake_token = intake_token;
        self.register_channel(
            ChannelRegistration::new(seasoned_hand_core::channel::webhook::CHANNEL_NAME)
                .with_intake(channel.clone())
                .with_delivery(channel.clone())
                .with_notify(channel),
        )
    }

    /// Story 2.11: register the production `EmailChannel` (IMAP intake
    /// poller + lettre SMTP delivery + lettre SMTP notify). Returns
    /// `self` unchanged when the supplied `EmailChannelEnv` is
    /// disabled (missing IMAP host / username — see
    /// [`EmailChannelEnv::resolve`]) so the boot path is one-line in
    /// `main.rs` regardless of operator config.
    pub fn register_email_channel(self, env: EmailChannelEnv) -> Self {
        let Some(EmailChannelConfig {
            imap,
            smtp,
            from_address,
            subject_prefix,
            allow_list,
            poll_interval,
        }) = env.into_config()
        else {
            tracing::info!(
                "email channel disabled (missing IMAP_HOST / IMAP_USERNAME / IMAP_PASSWORD)"
            );
            return self;
        };

        let smtp_transport = match LettreSmtpTransport::new(&smtp) {
            Ok(t) => t,
            Err(error) => {
                tracing::warn!(%error, "email channel disabled: SMTP transport setup failed");
                return self;
            }
        };
        let fetcher = Arc::new(AsyncImapFetcher::new(imap));
        let transport = Arc::new(smtp_transport);
        let channel = match EmailChannel::builder()
            .fetcher(fetcher)
            .transport(transport)
            .from_address(from_address)
            .subject_prefix(subject_prefix)
            .allow_list(allow_list)
            .poll_interval(poll_interval)
            .build()
        {
            Ok(c) => Arc::new(c),
            Err(error) => {
                tracing::warn!(%error, "email channel disabled: builder rejected config");
                return self;
            }
        };

        self.register_channel(
            ChannelRegistration::new(seasoned_hand_core::channel::email::CHANNEL_NAME)
                .with_intake(channel.clone())
                .with_delivery(channel.clone())
                .with_notify(channel),
        )
    }

    /// Story 2.12: register the production [`NtfyChannel`]. Notify-only,
    /// so only the `with_notify` slot is filled.
    ///
    /// `NTFY_HOST` defaults to `https://ntfy.sh`; main.rs only calls
    /// this when `NTFY_TOPIC` env is non-empty (the topic itself is a
    /// per-trigger / per-notify concern resolved by
    /// [`NotifyConfig::resolve`](seasoned_hand_core::notify::NotifyConfig)).
    pub fn register_ntfy_channel(self, host: impl Into<String>) -> Self {
        let channel = Arc::new(NtfyChannel::with_default_client(host));
        self.register_channel(
            ChannelRegistration::new(seasoned_hand_core::channel::ntfy::CHANNEL_NAME)
                .with_notify(channel),
        )
    }

    /// Story 2.12: swap in the operator-provided notify config. Default
    /// (`AppState::new`) is an empty config — every trigger is silently
    /// disabled until main.rs supplies a parsed `config/notify.toml`.
    pub fn with_notify_config(mut self, config: Arc<NotifyConfig>) -> Self {
        self.notify_config = config;
        self
    }

    /// Story 2.21a / Phase 2 DEBT #23: register the `CliChannel` already
    /// built by `AppState::new` into the channel registry under both the
    /// intake and delivery slots. Notify is intentionally unfilled —
    /// terminal push semantics live on ntfy / email per `channel/cli.rs`
    /// docs. Idempotent in practice: `register_channel` rebuilds the
    /// registry from existing entries, so calling this twice just
    /// re-points the slot at the same `Arc<CliChannel>`.
    ///
    /// `main.rs` calls this after `register_ntfy_channel` so the CLI
    /// slot is always present in production AppState. The 2.21b
    /// `task new --blocking` path will read
    /// `AppState::cli_channel.register_pending(...)` directly so the
    /// in-process oneshot path works without round-tripping through
    /// HTTP.
    pub fn register_cli_channel(self) -> Self {
        let channel = self.cli_channel.clone();
        self.register_channel(
            ChannelRegistration::new(seasoned_hand_core::channel::cli::CHANNEL_NAME)
                .with_intake(channel.clone())
                .with_delivery(channel),
        )
    }

    /// Story 2.20: attach the NarratorHook's classifier-slot LLM path.
    /// Closes the deferred plumbing called out in story 1.15 Execution
    /// notes. Called once at boot from `main.rs` after loading the
    /// narrator system prompt from disk; tests skip it (templated-only
    /// stays the default for the existing 6 Phase-1 narrate tests).
    pub fn with_narrator_classifier(self, wiring: NarratorClassifierWiring) -> Self {
        if let Err(_existing) =
            self.narrator
                .attach_classifier(wiring.llm, wiring.model, wiring.system_prompt)
        {
            tracing::warn!(
                "narrator classifier already attached; ignoring second with_narrator_classifier call"
            );
        }
        self
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

    /// Issue #7 / ADR-018: enable the legacy `x-seasoned-hand-*` header auth path
    /// (equivalent to setting `SH_INSECURE_AUTH_HEADERS`). Primarily for tests and
    /// loopback dev that assert header-based identity without a session login.
    pub fn allow_insecure_auth_headers(mut self) -> Self {
        self.allow_insecure_headers = true;
        self
    }

    /// Story 1.13b: enable the opt-in Verifier-driven rollback path.
    /// Defaults `false` per phase-1/DEBT.md #3.
    pub fn with_rollback_on_verifier_fail(mut self, enabled: bool) -> Self {
        self.checkpoint_rollback_on_verifier_fail = enabled;
        self
    }

    /// Issue #33: serve the built Dioxus UI bundle at `dir` as the router
    /// fallback (see [`AppState::ui_dist`]). `main.rs` calls this with the
    /// `SH_UI_DIST` path after validating the directory exists; tests pass a
    /// temp dir directly.
    pub fn with_ui_dist(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.ui_dist = Some(dir.into());
        self
    }
}

/// Story 2.8b: attach the production [`WsInitializerSpawner`] to a
/// freshly-built [`AppState::intake_router`]. Called once from
/// `AppState::new` and again after every `register_channel`, since
/// both code paths swap in a brand-new `IntakeRouter` whose `OnceLock`
/// starts empty.
fn attach_initializer_spawner(state: &AppState) {
    let spawner = Arc::new(WsInitializerSpawner::new(state.clone()))
        as Arc<dyn seasoned_hand_core::intake::InitializerSpawner>;
    if state
        .intake_router
        .attach_initializer_spawner(spawner)
        .is_err()
    {
        // Belt-and-braces — `attach_initializer_spawner` is only called
        // immediately after `IntakeRouter::new`, so the OnceLock should
        // always be empty here. Logging keeps the symptom visible if
        // a future refactor reuses the same router.
        tracing::warn!(
            "intake_router: initializer_spawner already attached — possible double-init"
        );
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

type ApiErrorResponse = (StatusCode, Json<ApiError>);
type ApiResult<T> = Result<T, ApiErrorResponse>;

fn api_err(status: StatusCode, code: String) -> ApiErrorResponse {
    (status, Json(ApiError { error: code }))
}

/// Issue #21 — explicit route auth classification. Every route in `app()` is
/// wrapped by exactly one of `with_auth` (protected), `public`, or `self_gated`,
/// so there are no bare/unclassified routes that silently skip auth. `with_auth`
/// attaches the verified-session + coarse-RBAC middleware; `public`/`self_gated`
/// are explicit markers that document why a route carries no session gate.
fn with_auth(route: MethodRouter<AppState>, action: Action) -> MethodRouter<AppState> {
    route
        .route_layer(middleware::from_fn(auth::middleware::require_auth_context))
        .layer(Extension(auth::middleware::RouteAction(action)))
}

/// A genuinely public route (no authentication): health + the login endpoints
/// that mint the first credential. Identity wrapper — the classification is the
/// documentation.
fn public(route: MethodRouter<AppState>) -> MethodRouter<AppState> {
    route
}

/// A route that performs its OWN authentication in the handler (loopback and/or
/// admin/webhook token) rather than via a verified session — operational /
/// machine endpoints. Identity wrapper; the handler MUST self-guard.
fn self_gated(route: MethodRouter<AppState>) -> MethodRouter<AppState> {
    route
}

fn authorize_in_handler(action: Action, ctx: &AuthContext) -> ApiResult<()> {
    authorize_coarse(action, ctx).map_err(|err| match err {
        seasoned_hand_core::auth::AuthError::MissingTenantContext => {
            api_err(StatusCode::UNAUTHORIZED, "unauthorized_context".into())
        }
        seasoned_hand_core::auth::AuthError::Unauthorized { .. } => {
            api_err(StatusCode::FORBIDDEN, "forbidden_action".into())
        }
    })
}

/// Hardening P5-HARD-IT3-H4: confirm a task belongs to the caller's
/// tenant before any single-resource `:id` operation. The RBAC
/// `with_auth(..., Action::TaskWrite/TaskRead)` layer gates the *verb*
/// but not the *row* — without this guard a tenant-A caller could
/// pause/resume/cancel/read tenant-B's task by id. Returns 404 (not
/// 403) on a tenant mismatch so cross-tenant existence isn't leaked,
/// identical to a genuinely missing id.
pub(crate) async fn require_task_tenant(
    state: &AppState,
    task_id: &str,
    auth: &AuthContext,
) -> ApiResult<()> {
    let task = state.tasks.get(task_id).await.map_err(|e| match e {
        seasoned_hand_core::project::TaskError::NotFound(_) => {
            api_err(StatusCode::NOT_FOUND, "task_not_found".into())
        }
        other => {
            tracing::error!(error = %other, "require_task_tenant::lookup");
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
        }
    })?;
    if task.tenant_id.as_deref() != Some(auth.tenant_id.as_str()) {
        return Err(api_err(StatusCode::NOT_FOUND, "task_not_found".into()));
    }
    Ok(())
}

async fn require_project_tenant(
    state: &AppState,
    project_id: &str,
    auth: &AuthContext,
) -> ApiResult<()> {
    let project = state.projects.get(project_id).await.map_err(|e| match e {
        seasoned_hand_core::project::ProjectError::NotFound(_) => {
            api_err(StatusCode::NOT_FOUND, "project_not_found".into())
        }
        other => {
            tracing::error!(error = %other, "require_project_tenant::lookup");
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
        }
    })?;
    if project.tenant_id.as_deref() != Some(auth.tenant_id.as_str()) {
        return Err(api_err(StatusCode::NOT_FOUND, "project_not_found".into()));
    }
    Ok(())
}

/// Canonical "session `s` belongs to the bound tenant" predicate (issue #22
/// batch B review). **Fail-closed**: every present *direct* parent (project via
/// `s.project_id`, task via `s.task_id`) must match the tenant, and orphan
/// sessions with no parent are excluded. A row whose project and task resolve to
/// *different* tenants therefore belongs to **neither** — it is corrupt and
/// invisible to all, instead of leaking to whichever parent a tenant happens to
/// share (a plain `COALESCE(p, t)` trusted one parent and ignored a conflicting
/// other). Requires `LEFT JOIN projects p ON p.id = s.project_id` and
/// `LEFT JOIN tasks t ON t.id = s.task_id`, and binds the tenant parameter
/// **twice** (in this clause's order). FK enforcement means a non-null
/// `project_id`/`task_id` with a NULL joined `tenant_id` can only be a dangling
/// reference, which this clause also rejects.
const SESSION_TENANT_PREDICATE: &str = "(s.project_id IS NULL OR p.tenant_id = ?) \
     AND (s.task_id IS NULL OR t.tenant_id = ?) \
     AND (s.project_id IS NOT NULL OR s.task_id IS NOT NULL)";

async fn require_session_tenant(
    state: &AppState,
    session_id: &str,
    auth: &AuthContext,
) -> ApiResult<()> {
    let sid = session_id.to_string();
    let tenant = auth.tenant_id.clone();
    let exists = state
        .db
        .with_conn(move |conn| {
            conn.query_row::<i64, _, _>(
                &format!(
                    "SELECT 1
                       FROM sessions s
                       LEFT JOIN projects p ON p.id = s.project_id
                       LEFT JOIN tasks t ON t.id = s.task_id
                      WHERE s.id = ? AND {SESSION_TENANT_PREDICATE}"
                ),
                rusqlite::params![sid, tenant, tenant],
                |row| row.get(0),
            )
            .is_ok()
        })
        .await;
    if !exists {
        return Err(api_err(StatusCode::NOT_FOUND, "session_not_found".into()));
    }
    Ok(())
}

async fn require_verification_tenant(
    state: &AppState,
    verification_id: &str,
    auth: &AuthContext,
) -> ApiResult<()> {
    let verification_id = verification_id.to_string();
    let tenant_id = auth.tenant_id.clone();
    let exists = state
        .db
        .with_conn(move |conn| {
            conn.query_row::<i64, _, _>(
                &format!(
                    "SELECT 1
                       FROM verifications v
                       JOIN sessions s ON s.id = v.session_id
                  LEFT JOIN projects p ON p.id = s.project_id
                  LEFT JOIN tasks t ON t.id = s.task_id
                      WHERE v.id = ? AND {SESSION_TENANT_PREDICATE}"
                ),
                rusqlite::params![verification_id, tenant_id, tenant_id],
                |row| row.get(0),
            )
            .is_ok()
        })
        .await;
    if !exists {
        return Err(api_err(
            StatusCode::NOT_FOUND,
            "verification_not_found".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize, Default)]
struct ProgressQuery {
    lines: Option<usize>,
}

async fn list_events(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(session_id): Path<String>,
    Query(params): Query<EventsQueryParams>,
) -> Result<Json<Vec<seasoned_hand_core::events::Event>>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskRead, &auth_ctx)?;
    let event_type = match params.event_type.as_deref() {
        Some(s) => Some(
            EventType::from_str(s)
                .map_err(|_| api_err(StatusCode::BAD_REQUEST, "unknown_event_type".into()))?,
        ),
        None => None,
    };

    let filter = EventQuery {
        after_id: params.after_id,
        event_type,
        limit: params.limit,
    };

    // Issue #22: route through the canonical tenant guard instead of an inline
    // `JOIN projects ... p.tenant_id = ?`. The inner join excluded chat-spawned
    // sessions (project_id NULL, tenancy from task_id); `require_session_tenant`
    // applies the shared fail-closed `SESSION_TENANT_PREDICATE`, so the legitimate
    // owner of a task-spawned session no longer gets a spurious 404 (and a
    // mismatched-parent session stays invisible to every tenant).
    require_session_tenant(&state, &session_id, &auth_ctx).await?;

    match state.events.query(&session_id, filter).await {
        Ok(events) => Ok(Json(events)),
        Err(seasoned_hand_core::events::EventError::SessionNotFound(_)) => {
            Err(api_err(StatusCode::NOT_FOUND, "session_not_found".into()))
        }
        Err(other) => {
            tracing::error!(error = %other, "events query failed");
            Err(api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error".into(),
            ))
        }
    }
}

// --- Story 5.16: tenant-visible event read + admin raw-event route -----

#[derive(Debug, Deserialize, Default)]
pub struct VisibleEventsQueryParams {
    pub after_event_id: Option<i64>,
    pub limit: Option<usize>,
}

/// `GET /v1/events/:session_id` — returns rows from `tenant_event_view`
/// filtered by the caller's tenant + role visibility. Redacted at write
/// time (story 5.14); no raw `events.data` is exposed here regardless
/// of role.
async fn list_redacted_events(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(session_id): Path<String>,
    Query(params): Query<VisibleEventsQueryParams>,
) -> Result<
    Json<Vec<seasoned_hand_core::events::visibility::VisibleEventRow>>,
    (StatusCode, Json<ApiError>),
> {
    require_loopback(remote)?;
    // No `Action`-level gate here — the tenant + visibility predicates
    // inside `visibility::query` ARE the gate (architecture §7).
    let q = seasoned_hand_core::events::visibility::EventReadQuery {
        after_event_id: params.after_event_id,
        limit: params.limit,
    };
    match seasoned_hand_core::events::visibility::query(&state.db, &auth_ctx, &session_id, q).await
    {
        Ok(rows) => Ok(Json(rows)),
        Err(err) => {
            tracing::error!(error = %err, "visibility::query failed");
            Err(api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error".into(),
            ))
        }
    }
}

/// `GET /v1/admin/events/:session_id/raw` — admin-only forensic read of
/// raw `events.data`. Gated by `Action::EventRawRead`; every call
/// writes an `audit_log` row via [`AuditLogger`] before returning, so
/// the access is non-repudiable. Cross-tenant admins are blocked even
/// with the action right — the session's tenant must match the
/// caller's tenant.
async fn list_raw_events_admin(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(session_id): Path<String>,
    Query(params): Query<VisibleEventsQueryParams>,
) -> Result<
    Json<Vec<seasoned_hand_core::events::visibility::RawEventRow>>,
    (StatusCode, Json<ApiError>),
> {
    require_loopback(remote)?;
    let audit = AuditLogger::new(state.db.clone(), state.events.clone());
    let q = seasoned_hand_core::events::visibility::EventReadQuery {
        after_event_id: params.after_event_id,
        limit: params.limit,
    };
    match seasoned_hand_core::events::visibility::query_raw(
        &state.db,
        &auth_ctx,
        &audit,
        &session_id,
        q,
    )
    .await
    {
        Ok(rows) => Ok(Json(rows)),
        Err(seasoned_hand_core::events::visibility::VisibilityQueryError::Auth(_)) => {
            Err(api_err(StatusCode::FORBIDDEN, "forbidden_action".into()))
        }
        Err(err) => {
            tracing::error!(error = %err, "visibility::query_raw failed");
            Err(api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error".into(),
            ))
        }
    }
}

// Session response shapes are shared with the wasm UI via seasoned-hand-dto
// (story 6.3b). `SandboxInfo` is the dto `Sandbox`.
use seasoned_hand_dto::{Sandbox as SandboxInfo, SessionDetail, SessionState, SessionSummary};

#[derive(Debug, Deserialize, Default)]
pub struct SessionsListParams {
    pub limit: Option<usize>,
}

async fn list_sessions(
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

async fn get_session(
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

async fn workspace_proxy(
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

async fn workspace_proxy_inner(
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

async fn cost_snapshot(
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

async fn get_feature_list(
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

async fn get_progress(
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

// ---------------------------------------------------------------------------
// Issue #7 / ADR-018: verified-session auth endpoints.

#[derive(Deserialize)]
struct LoginRequest {
    invitation_token: String,
}

#[derive(Serialize)]
struct LoginResponse {
    access_token: String,
    expires_at: i64,
    tenant_id: String,
    organization_id: String,
    actor_user_id: String,
    org_role: String,
}

fn role_str(role: seasoned_hand_core::auth::Role) -> &'static str {
    use seasoned_hand_core::auth::Role;
    match role {
        Role::Admin => "admin",
        Role::User => "user",
        Role::Viewer => "viewer",
    }
}

fn login_response(result: seasoned_hand_core::auth::LoginResult) -> Json<LoginResponse> {
    Json(LoginResponse {
        access_token: result.token,
        expires_at: result.expires_at,
        tenant_id: result.context.tenant_id,
        organization_id: result.context.organization_id,
        actor_user_id: result.context.actor_user_id,
        org_role: role_str(result.context.org_role).to_string(),
    })
}

/// Exchange a single-use invitation token for a session token. Public
/// (unauthenticated) by necessity — it mints the first credential.
async fn post_auth_login_handler(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, Json<ApiError>)> {
    use seasoned_hand_core::auth::AuthLoginError;
    match state.auth_sessions.login(&body.invitation_token).await {
        Ok(result) => Ok(login_response(result)),
        Err(AuthLoginError::InvalidInvitation) => Err(api_err(
            StatusCode::UNAUTHORIZED,
            "invalid_invitation".into(),
        )),
        Err(AuthLoginError::NoMembership) => {
            Err(api_err(StatusCode::FORBIDDEN, "no_membership".into()))
        }
        Err(AuthLoginError::Db(_)) => Err(api_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "login_failed".into(),
        )),
    }
}

/// Loopback-gated dev affordance: issue a session for a default dev identity so
/// the browser UI works in local dev before the real client login flow (#26).
/// Refuses unless `SH_INSECURE_AUTH_HEADERS` is set AND the caller is loopback.
async fn post_auth_dev_login_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
) -> Result<Json<LoginResponse>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    if !state.allow_insecure_headers {
        return Err(api_err(StatusCode::FORBIDDEN, "dev_login_disabled".into()));
    }
    state
        .auth_sessions
        .issue_dev_session()
        .await
        .map(login_response)
        .map_err(|_| api_err(StatusCode::INTERNAL_SERVER_ERROR, "dev_login_failed".into()))
}

// ---------------------------------------------------------------------------
// Story 2.5: channel HTTP routes.
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct ChannelHealthDto {
    name: String,
    capabilities: Vec<&'static str>,
}

impl From<seasoned_hand_core::channel::ChannelHealth> for ChannelHealthDto {
    fn from(h: seasoned_hand_core::channel::ChannelHealth) -> Self {
        Self {
            name: h.name,
            capabilities: h.capabilities,
        }
    }
}

async fn list_channels_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
) -> Result<Json<Vec<ChannelHealthDto>>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    let snapshot = state
        .channels
        .health()
        .into_iter()
        .map(ChannelHealthDto::from)
        .collect();
    Ok(Json(snapshot))
}

async fn get_channel_health_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Path(name): Path<String>,
) -> Result<Json<ChannelHealthDto>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    state
        .channels
        .health()
        .into_iter()
        .find(|h| h.name == name)
        .map(ChannelHealthDto::from)
        .map(Json)
        .ok_or_else(|| api_err(StatusCode::NOT_FOUND, "channel_not_found".into()))
}

#[derive(Debug, Deserialize)]
struct ChannelTestQuery {
    role: Option<String>,
}

#[derive(Debug, Serialize)]
struct ChannelTestResponse {
    name: String,
    role: String,
    ok: bool,
}

/// Phase 2 stub: confirm `channel` is registered AND has the requested
/// role implemented. Real synthetic round-trips land per-channel in
/// stories 2.9–2.13 (each can specialise `dry-run`).
async fn post_channel_test_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Path(name): Path<String>,
    Query(q): Query<ChannelTestQuery>,
) -> Result<Json<ChannelTestResponse>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    let role = q.role.as_deref().unwrap_or("delivery");
    let registered = match role {
        "intake" => state.channels.get_intake(&name).is_some(),
        "delivery" => state.channels.get_delivery(&name).is_some(),
        "notify" => state.channels.get_notify(&name).is_some(),
        _other => {
            return Err(api_err(StatusCode::BAD_REQUEST, "unknown_role".into()));
        }
    };
    if !registered {
        // Distinguish channel-missing from role-missing so operators
        // know whether to fix the registration or pick a different role.
        let channel_exists = state.channels.health().iter().any(|h| h.name == name);
        let err = if channel_exists {
            "role_not_implemented"
        } else {
            "channel_not_found"
        };
        return Err(api_err(StatusCode::NOT_FOUND, err.to_string()));
    }
    Ok(Json(ChannelTestResponse {
        name,
        role: role.to_string(),
        ok: true,
    }))
}

// ---------------------------------------------------------------------------
// Story 2.10: WebhookChannel intake — POST /v1/intake/webhook.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct WebhookIntakeBody {
    brief: String,
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    reply_target: Option<seasoned_hand_core::channel::DeliveryTarget>,
    #[serde(default)]
    metadata: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct WebhookIntakeAck {
    task_id: String,
    /// Phase 2 reserves this slot per architecture §2.8 — the
    /// briefing-confirmation flow that fills it lands in story 2.8.
    /// Returning `None` is preferable to omitting the field so the
    /// response shape is stable across the briefing rollout.
    briefing_call_id: Option<String>,
}

async fn post_intake_webhook_handler(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    body: Result<Json<WebhookIntakeBody>, axum::extract::rejection::JsonRejection>,
) -> Result<(StatusCode, Json<WebhookIntakeAck>), (StatusCode, Json<ApiError>)> {
    use seasoned_hand_core::channel::IntakeEvent;
    use seasoned_hand_core::channel::webhook::CHANNEL_NAME as WEBHOOK_NAME;
    use seasoned_hand_core::intake::router::{HandleOutcome, RejectionReason};

    // Locate the registered WebhookChannel's intake provider so we
    // can re-use its constant-time token check. Falling back to the
    // raw `webhook_intake_token` keeps the handler honest if a future
    // refactor changes the registration shape.
    let token_check = if !state.webhook_intake_token.is_empty() {
        let supplied = headers
            .get("X-Seasoned-Hand-Intake-Token")
            .and_then(|h| h.to_str().ok());
        use subtle::ConstantTimeEq;
        let ok: bool = supplied
            .unwrap_or("")
            .as_bytes()
            .ct_eq(state.webhook_intake_token.as_bytes())
            .into();
        if ok {
            TokenCheck::Ok
        } else {
            TokenCheck::Mismatch
        }
    } else {
        TokenCheck::NotConfigured
    };

    match token_check {
        TokenCheck::NotConfigured => {
            return Err(api_err(
                StatusCode::SERVICE_UNAVAILABLE,
                "intake_token_not_configured".into(),
            ));
        }
        TokenCheck::Mismatch => {
            return Err(api_err(
                StatusCode::UNAUTHORIZED,
                "unauthorized_token".into(),
            ));
        }
        TokenCheck::Ok => {}
    }

    let Json(body) = body.map_err(|error| {
        tracing::warn!(%error, "webhook intake: invalid JSON body (returning 400)");
        api_err(StatusCode::BAD_REQUEST, "invalid_json_body".into())
    })?;

    let mut metadata = body.metadata.unwrap_or_else(|| serde_json::json!({}));
    if let Some(pid) = body.project_id.as_ref()
        && let Some(obj) = metadata.as_object_mut()
    {
        obj.insert("project_id".into(), serde_json::Value::String(pid.clone()));
    }

    let intake_event = IntakeEvent {
        channel: WEBHOOK_NAME.into(),
        intake_id: format!("http:{}", uuid::Uuid::new_v4()),
        brief_input: body.brief,
        reply_target: body.reply_target,
        metadata,
        tenant_id: None,
        received_at: now_micros(),
    };

    match state.intake_router.handle_event(intake_event).await {
        Ok(HandleOutcome::Created { task_id, .. }) => Ok((
            StatusCode::ACCEPTED,
            Json(WebhookIntakeAck {
                task_id,
                briefing_call_id: None,
            }),
        )),
        Ok(HandleOutcome::DuplicateSkipped) => Err(api_err(
            StatusCode::CONFLICT,
            "intake_rejected:duplicate_intake_id".into(),
        )),
        Ok(HandleOutcome::Rejected(reason)) => {
            // DEBT #12 close-out for the webhook surface: validation
            // rejection surfaces as 4xx with the spec-shaped
            // `intake_rejected:<reason>` payload. The pre-task Misc
            // event remains deferred until a system-session strategy
            // exists.
            let reason_code = match reason {
                RejectionReason::EmptyBrief => "empty_brief",
                RejectionReason::UnknownChannel(_) => "unknown_channel",
            };
            Err(api_err(
                StatusCode::BAD_REQUEST,
                format!("intake_rejected:{reason_code}"),
            ))
        }
        Err(error) => {
            tracing::error!(%error, "webhook intake: IntakeRouter error");
            Err(api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error".into(),
            ))
        }
    }
}

use seasoned_hand_core::time::now_micros;

// ---------------------------------------------------------------------------
// Loopback guard helper — shared by Phase 1 admin routes (1.13b rollback,
// 2.17 cleanup) AND the Phase 2 CLI surface (2.21a /v1/projects + /v1/tasks).
// Phase 2 single-operator deployments bind the server to `127.0.0.1`;
// Phase 5 multi-user will replace this with real auth, but the
// guard's job stays the same — keep these routes off the public surface.
// ---------------------------------------------------------------------------

fn require_loopback(remote: SocketAddr) -> ApiResult<()> {
    if remote.ip().is_loopback() {
        Ok(())
    } else {
        Err(api_err(
            StatusCode::FORBIDDEN,
            "forbidden_non_loopback".into(),
        ))
    }
}

const ADMIN_TOKEN_HEADER: &str = "X-Seasoned-Hand-Admin-Token";

fn require_admin_token_configured(state: &AppState) -> ApiResult<()> {
    if state.admin_token.is_empty() {
        return Err(api_err(
            StatusCode::SERVICE_UNAVAILABLE,
            "admin_token_not_configured".into(),
        ));
    }
    Ok(())
}

fn require_admin_token_header(state: &AppState, headers: &HeaderMap) -> ApiResult<()> {
    let token_hdr = headers
        .get(ADMIN_TOKEN_HEADER)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let ok: bool = token_hdr
        .as_bytes()
        .ct_eq(state.admin_token.as_bytes())
        .into();
    if ok {
        Ok(())
    } else {
        Err(api_err(
            StatusCode::UNAUTHORIZED,
            "unauthorized_token".into(),
        ))
    }
}

fn require_admin_route(state: &AppState, remote: SocketAddr, headers: &HeaderMap) -> ApiResult<()> {
    // Guard order is intentional:
    // 1. Missing server config is a local operator setup error.
    // 2. Non-loopback peers stop before token comparison, preserving
    //    the timing/status behavior pinned by the admin route tests.
    // 3. Token comparison is constant-time defense in depth.
    require_admin_token_configured(state)?;
    require_loopback(remote)?;
    require_admin_token_header(state, headers)
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
    Extension(auth_ctx): Extension<AuthContext>,
    headers: HeaderMap,
    Path((session_id, checkpoint_id)): Path<(String, String)>,
    Json(body): Json<RollbackBody>,
) -> ApiResult<(StatusCode, Json<RollbackResponse>)> {
    require_admin_route(&state, remote, &headers)?;
    authorize_in_handler(Action::TaskWrite, &auth_ctx)?;
    require_session_tenant(&state, &session_id, &auth_ctx).await?;
    // Guard 4: reason length.
    if body.reason.len() > 200 {
        return Err(api_err(StatusCode::BAD_REQUEST, "reason_too_long".into()));
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
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
        })?;
    match session_state.as_deref() {
        Some("RUNNING") | Some("VERIFYING") => {
            return Err(api_err(StatusCode::CONFLICT, "wrong_state".into()));
        }
        None => {
            return Err(api_err(StatusCode::NOT_FOUND, "session_not_found".into()));
        }
        _ => {}
    }

    // Guard 6: sandbox must not be paused.
    let paused = state.sandbox.is_paused(&session_id).await.map_err(|e| {
        tracing::warn!(error = %e, "rollback: sandbox paused-state probe failed");
        api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
    })?;
    if paused {
        return Err(api_err(StatusCode::CONFLICT, "sandbox_paused".into()));
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
        matcher_mode: seasoned_hand_core::matcher::MatcherMode::Production,
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
        return Err(api_err(status, err_kind));
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
// Story 2.17: admin workspace-cleanup handler.
// ---------------------------------------------------------------------------

async fn post_admin_sandbox_cleanup_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
) -> ApiResult<(StatusCode, Json<seasoned_hand_core::task::TtlCleanupReport>)> {
    require_admin_route(&state, remote, &headers)?;
    let report = state.workspace_ttl_cron.cleanup_cycle().await;
    Ok((StatusCode::OK, Json(report)))
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
        RouteOutcome::NotFound(msg) => Err(api_err(StatusCode::NOT_FOUND, msg)),
        RouteOutcome::Internal(msg) => {
            tracing::error!(error = %msg, route = label, "route failed");
            Err(api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error".into(),
            ))
        }
    }
}

async fn list_checkpoints_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(session_id): Path<String>,
    Query(q): Query<seasoned_hand_core::checkpoint::routes::ListQuery>,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskRead, &auth_ctx)?;
    require_session_tenant(&state, &session_id, &auth_ctx).await?;
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
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(session_id): Path<String>,
    Query(q): Query<VerifyListQuery>,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskRead, &auth_ctx)?;
    require_session_tenant(&state, &session_id, &auth_ctx).await?;
    render_outcome(
        "list_verifications",
        list_verifications(&state.verifications, &session_id, q).await,
    )
}

async fn get_verification_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(id): Path<String>,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskRead, &auth_ctx)?;
    require_verification_tenant(&state, &id, &auth_ctx).await?;
    render_outcome(
        "get_verification",
        get_verification(&state.verifications, &id).await,
    )
}

// ---------------------------------------------------------------------------
// Story 2.15: per-task provenance manifest.
// ---------------------------------------------------------------------------

async fn get_task_provenance_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(task_id): Path<String>,
    Query(q): Query<seasoned_hand_core::provenance::GetTaskProvenanceQuery>,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    // Loopback gate matches every sibling /v1/tasks/:id/* handler; provenance
    // manifests can include PII (sender addresses, brief content, intake
    // metadata) so they must not leak at HOST=0.0.0.0 binds. See REVIEW
    // §5 cross-cutting issue #1 / proposed DEBT #34.
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskRead, &auth_ctx)?;
    require_task_tenant(&state, &task_id, &auth_ctx).await?;
    use seasoned_hand_core::provenance::{GetTaskProvenanceDeps, get_task_provenance};
    let deps = GetTaskProvenanceDeps {
        deliverables: state.deliverables.as_ref(),
        delivery_events: state.delivery_events.as_ref(),
        sandbox: state.sandbox.as_ref(),
        db: &state.db,
    };
    render_outcome(
        "get_task_provenance",
        get_task_provenance(&task_id, q, deps).await,
    )
}

// ---------------------------------------------------------------------------
// Story 2.21a: project + task HTTP routes that back the
// `seasoned-hand` CLI binary. Loopback-only — Phase 5 multi-user will
// add real auth and lift the constraint (BASELINE §8). The pause /
// resume / cancel routes delegate to the shared `ws::handle_task_*`
// helpers so the WS and HTTP entrypoints stay structurally identical.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
struct ProjectsListQuery {
    limit: Option<usize>,
    cursor: Option<i64>,
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateProjectBody {
    title: String,
    #[serde(default)]
    description: Option<String>,
}

async fn list_projects_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Query(q): Query<ProjectsListQuery>,
) -> Result<Json<Vec<seasoned_hand_core::project::Project>>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskRead, &auth_ctx)?;
    let status = match q.status.as_deref() {
        Some("active") => Some(seasoned_hand_core::project::ProjectStatus::Active),
        Some("archived") => Some(seasoned_hand_core::project::ProjectStatus::Archived),
        Some(_other) => {
            return Err(api_err(StatusCode::BAD_REQUEST, "unknown_status".into()));
        }
        None => None,
    };
    let limit = q.limit.unwrap_or(50);
    state
        .projects
        .list_by_tenant(&auth_ctx.tenant_id, status, q.cursor, limit)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!(error = %e, "list_projects");
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
        })
}

async fn create_project_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Json(body): Json<CreateProjectBody>,
) -> Result<(StatusCode, Json<seasoned_hand_core::project::Project>), (StatusCode, Json<ApiError>)>
{
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskWrite, &auth_ctx)?;
    if body.title.trim().is_empty() {
        return Err(api_err(StatusCode::BAD_REQUEST, "empty_title".into()));
    }
    let id = state
        .projects
        .insert(seasoned_hand_core::project::NewProject {
            tenant_id: Some(auth_ctx.tenant_id.clone()),
            title: body.title,
            description: body.description,
        })
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "create_project");
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
        })?;
    let row = state.projects.get(&id).await.map_err(|e| {
        tracing::error!(error = %e, "create_project::get");
        api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
    })?;
    Ok((StatusCode::CREATED, Json(row)))
}

async fn archive_project_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskWrite, &auth_ctx)?;
    require_project_tenant(&state, &id, &auth_ctx).await?;
    match state
        .projects
        .set_status(&id, seasoned_hand_core::project::ProjectStatus::Archived)
        .await
    {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(seasoned_hand_core::project::ProjectError::NotFound(_)) => {
            Err(api_err(StatusCode::NOT_FOUND, "project_not_found".into()))
        }
        Err(e) => {
            tracing::error!(error = %e, "archive_project");
            Err(api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error".into(),
            ))
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct TasksListQuery {
    limit: Option<usize>,
    cursor: Option<i64>,
    status: Option<String>,
}

async fn list_project_tasks_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(project_id): Path<String>,
    Query(q): Query<TasksListQuery>,
) -> Result<Json<Vec<seasoned_hand_core::project::Task>>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskRead, &auth_ctx)?;
    require_project_tenant(&state, &project_id, &auth_ctx).await?;
    let status = match q.status.as_deref() {
        Some(s) => match seasoned_hand_core::project::TaskStatus::from_db_str(s) {
            Ok(st) => Some(st),
            Err(_) => {
                return Err(api_err(StatusCode::BAD_REQUEST, "unknown_status".into()));
            }
        },
        None => None,
    };
    let limit = q.limit.unwrap_or(50);
    state
        .tasks
        .list_by_project_and_tenant(&project_id, &auth_ctx.tenant_id, status, q.cursor, limit)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!(error = %e, "list_project_tasks");
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
        })
}

async fn get_task_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(id): Path<String>,
) -> Result<Json<seasoned_hand_core::project::Task>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskRead, &auth_ctx)?;
    require_task_tenant(&state, &id, &auth_ctx).await?;
    match state.tasks.get(&id).await {
        Ok(task) => Ok(Json(task)),
        Err(seasoned_hand_core::project::TaskError::NotFound(_)) => {
            Err(api_err(StatusCode::NOT_FOUND, "task_not_found".into()))
        }
        Err(e) => {
            tracing::error!(error = %e, "get_task");
            Err(api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error".into(),
            ))
        }
    }
}

/// Story 2.22 backend: list every Deliverable row for a task and return
/// the latest session_id alongside. The frontend AgentComputer
/// `DeliverablesTab` joins these to build a download URL via the
/// existing `GET /v1/workspace/:session_id/*sub_path` proxy.
// Shared with the wasm UI via seasoned-hand-dto (story 6.3b); wraps the
// re-exported Deliverable (itself a dto type) + the latest session id.
use seasoned_hand_dto::TaskDeliverablesResponse;

async fn list_task_deliverables_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(task_id): Path<String>,
) -> Result<Json<TaskDeliverablesResponse>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskRead, &auth_ctx)?;
    require_task_tenant(&state, &task_id, &auth_ctx).await?;
    let deliverables = state
        .deliverables
        .list_by_task_and_tenant(&task_id, &auth_ctx.tenant_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "list_task_deliverables");
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
        })?;
    let latest_session_id = ws::lookup_latest_session_for_task(&state, &task_id).await;
    Ok(Json(TaskDeliverablesResponse {
        deliverables,
        latest_session_id,
    }))
}

#[derive(Debug, Deserialize, Default)]
struct TaskPauseBody {
    #[serde(default)]
    durable: Option<bool>,
}

async fn post_task_pause_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(task_id): Path<String>,
    body: Option<Json<TaskPauseBody>>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskWrite, &auth_ctx)?;
    // Confirm the task exists AND belongs to the caller's tenant
    // (P5-HARD-IT3-H4) before we touch session state.
    require_task_tenant(&state, &task_id, &auth_ctx).await?;
    let durable = body.and_then(|Json(b)| b.durable).unwrap_or(true);
    let session_id = ws::lookup_latest_session_for_task(&state, &task_id)
        .await
        .ok_or(api_err(StatusCode::CONFLICT, "no_active_session".into()))?;
    map_lifecycle_result(ws::handle_task_pause(&state, &session_id, durable).await)
}

async fn post_task_resume_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(task_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskWrite, &auth_ctx)?;
    // P5-HARD-IT3-H4: tenant-scope the task before resuming.
    require_task_tenant(&state, &task_id, &auth_ctx).await?;
    let session_id = ws::lookup_latest_session_for_task(&state, &task_id)
        .await
        .ok_or(api_err(StatusCode::CONFLICT, "no_active_session".into()))?;
    map_lifecycle_result(ws::handle_task_resume(&state, &session_id).await)
}

async fn post_task_cancel_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(task_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskWrite, &auth_ctx)?;
    // P5-HARD-IT3-H4: tenant-scope BEFORE the set_status write — cancel
    // directly mutates the row, so a missing tenant check here is a
    // cross-tenant write (the worst of the H4 class).
    require_task_tenant(&state, &task_id, &auth_ctx).await?;
    // Drive the Task state machine first — Phase 2 widened
    // `legal_transitions` so Drafted/Briefed/Confirmed/Running/Paused
    // all → Cancelled. Terminal task → 409 wrong_state. NotFound → 404.
    match state
        .tasks
        .set_status(&task_id, seasoned_hand_core::project::TaskStatus::Cancelled)
        .await
    {
        Ok(()) => {}
        Err(seasoned_hand_core::project::TaskError::NotFound(_)) => {
            return Err(api_err(StatusCode::NOT_FOUND, "task_not_found".into()));
        }
        Err(seasoned_hand_core::project::TaskError::IllegalTransition { from, .. }) => {
            return Err(api_err(
                StatusCode::CONFLICT,
                format!("wrong_state:{}", from.as_db_str()),
            ));
        }
        Err(other) => {
            tracing::error!(error = %other, "task_cancel::set_status");
            return Err(api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error".into(),
            ));
        }
    }
    // If there's a live session, cascade the cancel through the same
    // ws helper so sandbox teardown + Misc emission run exactly once.
    // No active session is fine — Drafted/Briefed task cancels never
    // had one to begin with.
    if let Some(session_id) = ws::lookup_latest_session_for_task(&state, &task_id).await
        && let Err(reason) = ws::handle_task_cancel(&state, &session_id).await
    {
        // Already-terminal session is fine on a cancel — the task row
        // is now Cancelled regardless. Surface other errors.
        if reason != "wrong_state" {
            tracing::warn!(
                %reason,
                %session_id,
                "task_cancel: session-side teardown reported a non-terminal error"
            );
        }
    }
    Ok(StatusCode::ACCEPTED)
}

#[derive(Debug, Deserialize)]
struct TaskHandoffBody {
    to_user_email: String,
    reason: Option<String>,
    expected_updated_at: Option<i64>,
}

#[derive(Debug, Serialize)]
struct TaskHandoffCanResponse {
    can_handoff: bool,
    reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct AuditListQuery {
    actor: Option<String>,
    action: Option<String>,
    since: Option<String>,
    limit: Option<usize>,
    task: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UserCostReconcileBody {
    month_yyyymm: String,
}

#[derive(Debug, Deserialize)]
struct InviteUserBody {
    email: String,
    role: String,
}

#[derive(Debug, Deserialize)]
struct SopShareBody {
    user_email: String,
    permission: String,
    /// Story 5.21: optimistic concurrency precondition. When present
    /// and the live share row's `updated_at` doesn't match, the
    /// response is `409 stale_revision`.
    #[serde(default)]
    expected_updated_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct SopUnshareBody {
    user_email: String,
    /// Story 5.21: optimistic concurrency precondition (see SopShareBody).
    #[serde(default)]
    expected_updated_at: Option<i64>,
}

#[derive(Debug, Serialize)]
struct SopShareDto {
    id: String,
    tenant_id: String,
    sop_id: String,
    subject_type: String,
    subject_id: String,
    subject_email: Option<String>,
    permission: String,
    granted_by_user_id: String,
    created_at: i64,
    updated_at: i64,
}

impl From<seasoned_hand_core::sharing::sop::SopShareRow> for SopShareDto {
    fn from(value: seasoned_hand_core::sharing::sop::SopShareRow) -> Self {
        Self {
            id: value.id,
            tenant_id: value.tenant_id,
            sop_id: value.sop_id,
            subject_type: value.subject_type,
            subject_id: value.subject_id,
            subject_email: value.subject_email,
            permission: match value.permission {
                SopPermission::Viewer => "viewer".into(),
                SopPermission::Editor => "editor".into(),
                SopPermission::Owner => "owner".into(),
            },
            granted_by_user_id: value.granted_by_user_id,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

async fn post_task_handoff_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(task_id): Path<String>,
    Json(body): Json<TaskHandoffBody>,
) -> ApiResult<(
    StatusCode,
    Json<seasoned_hand_core::handoff::HandoffOutcome>,
)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskHandoff, &auth_ctx)?;
    let handoff = TaskHandoffService::new(
        state.db.clone(),
        state.events.clone(),
        AuditLogger::new(state.db.clone(), state.events.clone()),
    );
    let outcome = handoff
        .handoff(
            &auth_ctx,
            HandoffRequest {
                task_id,
                to_user_email: body.to_user_email,
                reason: body.reason,
                expected_updated_at: body.expected_updated_at,
            },
        )
        .await
        .map_err(map_handoff_error)?;
    Ok((StatusCode::OK, Json(outcome)))
}

async fn get_task_handoff_can_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(task_id): Path<String>,
) -> ApiResult<Json<TaskHandoffCanResponse>> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskHandoff, &auth_ctx)?;
    // Issue #8: the coarse RBAC gate lets a User reach this read; scope the task
    // to the caller's tenant (404 on mismatch) so cross-tenant handoff-status
    // isn't exposed by id.
    require_task_tenant(&state, &task_id, &auth_ctx).await?;
    let handoff = TaskHandoffService::new(
        state.db.clone(),
        state.events.clone(),
        AuditLogger::new(state.db.clone(), state.events.clone()),
    );
    let can = handoff
        .can_handoff(&task_id)
        .await
        .map_err(map_handoff_error)?;
    let reason = if can {
        None
    } else {
        Some("pause required or terminal/unknown task".to_string())
    };
    Ok(Json(TaskHandoffCanResponse {
        can_handoff: can,
        reason,
    }))
}

async fn list_audit_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Query(q): Query<AuditListQuery>,
) -> ApiResult<Json<Vec<seasoned_hand_core::audit::AuditRow>>> {
    require_loopback(remote)?;
    authorize_in_handler(Action::AuditRead, &auth_ctx)?;
    let logger = AuditLogger::new(state.db.clone(), state.events.clone());
    let action = match q.action.as_deref() {
        Some(v) => Some(
            parse_audit_action(v)
                .ok_or(api_err(StatusCode::BAD_REQUEST, "invalid_action".into()))?,
        ),
        None => None,
    };
    let since_micros = q
        .since
        .as_deref()
        .map(parse_since_to_micros)
        .transpose()
        .map_err(|_| api_err(StatusCode::BAD_REQUEST, "invalid_since".into()))?;
    let rows = logger
        .query(
            &auth_ctx,
            AuditQuery {
                actor_user_id: q.actor,
                action,
                since_micros,
                limit: q.limit,
            },
        )
        .await
        .map_err(map_audit_query_error)?;
    let rows = if let Some(task_id) = q.task {
        rows.into_iter()
            .filter(|r| r.resource_type == "task" && r.resource_id == task_id)
            .collect()
    } else {
        rows
    };
    Ok(Json(rows))
}

async fn post_user_cost_reconcile_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Json(body): Json<UserCostReconcileBody>,
) -> ApiResult<Json<ReconciliationReport>> {
    require_loopback(remote)?;
    authorize_in_handler(Action::AuditRead, &auth_ctx)?;
    if !is_valid_month_yyyymm(&body.month_yyyymm) {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            "invalid_month_yyyymm".into(),
        ));
    }
    let job = ReconciliationJob::new(state.db.clone(), state.events.clone());
    let mut report = job.run(&body.month_yyyymm).await.map_err(|error| {
        tracing::error!(%error, "user_cost reconcile failed");
        api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
    })?;
    // P5-HARD-IT7-M10: the reconciliation job aggregates across ALL
    // tenants (it's a global ops cron). The HTTP trigger is admin-gated,
    // but an admin is scoped to their own tenant — so restrict the
    // returned drift findings (which carry tenant_id/user_id/cost) to
    // the caller's tenant before responding.
    report.drifts.retain(|d| d.tenant_id == auth_ctx.tenant_id);
    report.drifted_rows = report.drifts.len();
    Ok(Json(report))
}

async fn post_org_invite_user_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(slug): Path<String>,
    Json(body): Json<InviteUserBody>,
) -> ApiResult<(StatusCode, Json<InviteOutcome>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::MembershipManage, &auth_ctx)?;
    let service = InvitationService::new(
        state.db.clone(),
        AuditLogger::new(state.db.clone(), state.events.clone()),
    );
    let out = service
        .invite_user(&auth_ctx, &slug, &body.email, &body.role)
        .await
        .map_err(map_invitation_error)?;
    Ok((StatusCode::OK, Json(out)))
}

async fn list_org_users_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(slug): Path<String>,
) -> ApiResult<Json<Vec<MembershipRow>>> {
    require_loopback(remote)?;
    authorize_in_handler(Action::MembershipManage, &auth_ctx)?;
    let service = InvitationService::new(
        state.db.clone(),
        AuditLogger::new(state.db.clone(), state.events.clone()),
    );
    let rows = service
        .list_org_users(&auth_ctx, &slug)
        .await
        .map_err(map_invitation_error)?;
    Ok(Json(rows))
}

fn parse_audit_action(value: &str) -> Option<seasoned_hand_core::audit::AuditAction> {
    use seasoned_hand_core::audit::AuditAction;
    match value {
        "task.handoff" => Some(AuditAction::TaskHandoff),
        "task.cancel" => Some(AuditAction::TaskCancel),
        "sop.share" => Some(AuditAction::SopShare),
        "sop.unshare" => Some(AuditAction::SopUnshare),
        "playbook.share" => Some(AuditAction::PlaybookShare),
        "playbook.unshare" => Some(AuditAction::PlaybookUnshare),
        "playbook.approve" => Some(AuditAction::PlaybookApprove),
        "user.invite" => Some(AuditAction::UserInvite),
        "user.deactivate" => Some(AuditAction::UserDeactivate),
        "membership.update" => Some(AuditAction::MembershipUpdate),
        "event.raw_read" => Some(AuditAction::EventRawRead),
        _ => None,
    }
}

fn map_invitation_error(err: InvitationError) -> ApiErrorResponse {
    match err {
        InvitationError::Auth(seasoned_hand_core::auth::AuthError::MissingTenantContext) => {
            api_err(StatusCode::UNAUTHORIZED, "unauthorized_context".into())
        }
        InvitationError::Auth(seasoned_hand_core::auth::AuthError::Unauthorized { .. }) => {
            api_err(StatusCode::FORBIDDEN, "forbidden_action".into())
        }
        InvitationError::OrganizationNotFound(_) => {
            api_err(StatusCode::NOT_FOUND, "organization_not_found".into())
        }
        InvitationError::CrossTenantDenied => {
            api_err(StatusCode::FORBIDDEN, "cross_tenant_denied".into())
        }
        InvitationError::InvalidRole(_) => api_err(StatusCode::BAD_REQUEST, "invalid_role".into()),
        InvitationError::Sqlite(_) | InvitationError::AuditWrite(_) => {
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
        }
    }
}

fn parse_since_to_micros(value: &str) -> Result<i64, ()> {
    value.parse::<i64>().map_err(|_| ())
}

fn is_valid_month_yyyymm(value: &str) -> bool {
    value.len() == 6
        && value.chars().all(|c| c.is_ascii_digit())
        && value[4..6]
            .parse::<u32>()
            .is_ok_and(|m| (1..=12).contains(&m))
}

fn map_handoff_error(err: seasoned_hand_core::handoff::HandoffError) -> ApiErrorResponse {
    use seasoned_hand_core::handoff::HandoffError;
    match err {
        HandoffError::Auth(seasoned_hand_core::auth::AuthError::MissingTenantContext) => {
            api_err(StatusCode::UNAUTHORIZED, "unauthorized_context".into())
        }
        HandoffError::Auth(seasoned_hand_core::auth::AuthError::Unauthorized { .. }) => {
            api_err(StatusCode::FORBIDDEN, "forbidden_action".into())
        }
        HandoffError::TaskNotFound(_) => api_err(StatusCode::NOT_FOUND, "task_not_found".into()),
        HandoffError::UserNotFound(_) => api_err(StatusCode::NOT_FOUND, "user_not_found".into()),
        HandoffError::TerminalState(_) => api_err(StatusCode::CONFLICT, "task_terminal".into()),
        HandoffError::MustPauseFirst(_) => api_err(StatusCode::CONFLICT, "pause_required".into()),
        HandoffError::StaleRevision { .. } => {
            api_err(StatusCode::CONFLICT, "stale_revision".into())
        }
        HandoffError::InvalidStatus(_) => {
            api_err(StatusCode::CONFLICT, "invalid_task_status".into())
        }
        HandoffError::Sqlite(error) => {
            tracing::error!(%error, "handoff sqlite error");
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
        }
        HandoffError::Event(error) => {
            tracing::error!(%error, "handoff event error");
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
        }
    }
}

fn map_audit_query_error(err: seasoned_hand_core::audit::AuditQueryError) -> ApiErrorResponse {
    use seasoned_hand_core::audit::AuditQueryError;
    match err {
        AuditQueryError::Auth(seasoned_hand_core::auth::AuthError::MissingTenantContext) => {
            api_err(StatusCode::UNAUTHORIZED, "unauthorized_context".into())
        }
        AuditQueryError::Auth(seasoned_hand_core::auth::AuthError::Unauthorized { .. }) => {
            api_err(StatusCode::FORBIDDEN, "forbidden_action".into())
        }
        AuditQueryError::InvalidAction(_) => {
            api_err(StatusCode::BAD_REQUEST, "invalid_action_db".into())
        }
        AuditQueryError::Sqlite(error) => {
            tracing::error!(%error, "audit query sqlite error");
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
        }
    }
}

async fn post_sop_share_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(sop_id): Path<String>,
    Json(body): Json<SopShareBody>,
) -> ApiResult<(StatusCode, Json<SopShareDto>)> {
    // SEC-IT1-H2: match the loopback defense-in-depth every other
    // sensitive Phase 5 handler applies (these 3 SOP-share routes were
    // the only sensitive handlers missing it).
    require_loopback(remote)?;
    authorize_in_handler(Action::SopShare, &auth_ctx)?;
    let permission = parse_sop_permission(&body.permission)?;
    let service = SopShareService::new(state.db.clone());
    let row = service
        .share(
            &auth_ctx,
            &sop_id,
            &body.user_email,
            permission,
            body.expected_updated_at,
        )
        .await
        .map_err(map_sop_share_error)?;
    Ok((StatusCode::OK, Json(SopShareDto::from(row))))
}

async fn delete_sop_share_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(sop_id): Path<String>,
    Json(body): Json<SopUnshareBody>,
) -> ApiResult<StatusCode> {
    require_loopback(remote)?; // SEC-IT1-H2
    authorize_in_handler(Action::SopShare, &auth_ctx)?;
    let service = SopShareService::new(state.db.clone());
    let deleted = service
        .unshare(
            &auth_ctx,
            &sop_id,
            &body.user_email,
            body.expected_updated_at,
        )
        .await
        .map_err(map_sop_share_error)?;
    if !deleted {
        return Err(api_err(StatusCode::NOT_FOUND, "share_not_found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn list_sop_shares_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(sop_id): Path<String>,
) -> ApiResult<Json<Vec<SopShareDto>>> {
    require_loopback(remote)?; // SEC-IT1-H2
    authorize_in_handler(Action::SopShare, &auth_ctx)?;
    let service = SopShareService::new(state.db.clone());
    let rows = service
        .list_for_sop(&auth_ctx, &sop_id)
        .await
        .map_err(map_sop_share_error)?;
    let out = rows.into_iter().map(SopShareDto::from).collect();
    Ok(Json(out))
}

fn parse_sop_permission(value: &str) -> ApiResult<SopPermission> {
    match value {
        "viewer" => Ok(SopPermission::Viewer),
        "editor" => Ok(SopPermission::Editor),
        "owner" => Ok(SopPermission::Owner),
        _ => Err(api_err(
            StatusCode::BAD_REQUEST,
            "invalid_permission".into(),
        )),
    }
}

fn map_sop_share_error(err: SopShareError) -> ApiErrorResponse {
    match err {
        SopShareError::Auth(seasoned_hand_core::auth::AuthError::MissingTenantContext) => {
            api_err(StatusCode::UNAUTHORIZED, "unauthorized_context".into())
        }
        SopShareError::Auth(seasoned_hand_core::auth::AuthError::Unauthorized { .. }) => {
            api_err(StatusCode::FORBIDDEN, "forbidden_action".into())
        }
        SopShareError::SopNotFound(_) => api_err(StatusCode::NOT_FOUND, "sop_not_found".into()),
        SopShareError::UserNotFound(_) => api_err(StatusCode::NOT_FOUND, "user_not_found".into()),
        SopShareError::InvalidPermission(_) => api_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid_permission_db".into(),
        ),
        SopShareError::StaleRevision(_) => api_err(StatusCode::CONFLICT, "stale_revision".into()),
        SopShareError::Db(error) => {
            tracing::error!(%error, "sop_share db error");
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
        }
    }
}

fn map_lifecycle_result(
    res: Result<(), String>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    match res {
        Ok(()) => Ok(StatusCode::ACCEPTED),
        Err(reason) => {
            let status = match reason.as_str() {
                "wrong_state" => StatusCode::CONFLICT,
                "unknown_session" => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            Err(api_err(status, reason))
        }
    }
}

// ---------------------------------------------------------------------------
// Story 2.21b: CLI intake + inbox + briefing-confirm HTTP routes.
//
// All three are loopback-only (Phase 2 single-operator); Phase 5
// multi-user adds real auth. The CLI binary owns these — they back
// `seasoned-hand task new`, `seasoned-hand inbox`, and
// `seasoned-hand brief {confirm,edit,cancel}` respectively.
// ---------------------------------------------------------------------------

/// Default ceiling for the long-poll `task new` blocking flow. Operators
/// who want to wait longer can bump via env; tests override via the
/// `?max_wait_ms=` query param. The Initializer's confirm gate plus
/// agent loop comfortably fit inside 10 minutes for Phase 2 briefs;
/// anything longer is "go reach for `--detach`".
const CLI_INTAKE_DEFAULT_MAX_WAIT_SECS: u64 = 600;

#[derive(Debug, Deserialize)]
struct CliIntakeBody {
    brief: String,
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    metadata: Option<serde_json::Value>,
    /// When `true` (default) the handler holds the request open until
    /// the deliverable lands (or the timeout fires). `false` (matches
    /// the CLI's `--detach` flag) acks as soon as the task row is
    /// minted; the deliverable lands in `~/.seasoned-hand/deliverables/`.
    #[serde(default = "default_cli_intake_wait")]
    wait: bool,
}

fn default_cli_intake_wait() -> bool {
    true
}

#[derive(Debug, Deserialize, Default)]
struct CliIntakeQuery {
    /// Test seam — override the long-poll ceiling without waiting the
    /// full env-derived window. Production callers (the CLI) don't set
    /// this; the smoke test does.
    max_wait_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
struct CliIntakeAck {
    task_id: String,
    intake_id: String,
    /// `Some` on the blocking happy path; `None` when `wait=false` or
    /// when the deliver timed out (a follow-up `task deliverable` /
    /// `inbox` call surfaces it once it lands).
    #[serde(skip_serializing_if = "Option::is_none")]
    deliverable: Option<seasoned_hand_core::deliverable::Deliverable>,
    /// Phase 2 reserves this slot — the briefing-confirm gate is keyed
    /// by `task_id` (DEBT #20) so we echo `task_id` back here as a
    /// stable handle the CLI can hand to `brief confirm/edit/cancel`.
    briefing_call_id: Option<String>,
}

async fn post_intake_cli_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Query(q): Query<CliIntakeQuery>,
    Json(body): Json<CliIntakeBody>,
) -> Result<(StatusCode, Json<CliIntakeAck>), (StatusCode, Json<ApiError>)> {
    use seasoned_hand_core::channel::cli::{CHANNEL_NAME, INTAKE_ID_PREFIX, TARGET_INTAKE_PREFIX};
    use seasoned_hand_core::channel::{DeliveryTarget, IntakeEvent};
    use seasoned_hand_core::intake::router::{HandleOutcome, RejectionReason};

    require_loopback(remote)?;

    if body.brief.trim().is_empty() {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            "intake_rejected:empty_brief".into(),
        ));
    }

    let intake_id = format!("{INTAKE_ID_PREFIX}{}", uuid::Uuid::new_v4());
    let tenant_id = headers
        .get("x-seasoned-hand-tenant-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("legacy-default")
        .to_string();

    // Register the oneshot BEFORE handing the event to the router so a
    // very fast deliver() can't race past our pending slot.
    let rx_opt = if body.wait {
        Some(state.cli_channel.register_pending(intake_id.clone()))
    } else {
        None
    };

    let mut metadata = body.metadata.unwrap_or_else(|| serde_json::json!({}));
    if let Some(obj) = metadata.as_object_mut()
        && let Some(pid) = body.project_id.as_ref()
    {
        obj.insert("project_id".into(), serde_json::Value::String(pid.clone()));
    }

    let event = IntakeEvent {
        channel: CHANNEL_NAME.into(),
        intake_id: intake_id.clone(),
        brief_input: body.brief,
        reply_target: Some(DeliveryTarget {
            channel: CHANNEL_NAME.into(),
            target_ref: format!("{TARGET_INTAKE_PREFIX}{intake_id}"),
            metadata: serde_json::json!({}),
        }),
        metadata,
        tenant_id: Some(tenant_id),
        received_at: now_micros(),
    };

    let task_id = match state.intake_router.handle_event(event).await {
        Ok(HandleOutcome::Created { task_id, .. }) => task_id,
        Ok(HandleOutcome::DuplicateSkipped) => {
            // Shouldn't happen — we mint a fresh UUID — but stay honest.
            if let Some(_rx) = rx_opt {
                state.cli_channel.drop_pending(&intake_id);
            }
            return Err(api_err(
                StatusCode::CONFLICT,
                "intake_rejected:duplicate_intake_id".into(),
            ));
        }
        Ok(HandleOutcome::Rejected(reason)) => {
            if let Some(_rx) = rx_opt {
                state.cli_channel.drop_pending(&intake_id);
            }
            let reason_code = match reason {
                RejectionReason::EmptyBrief => "empty_brief",
                RejectionReason::UnknownChannel(_) => "unknown_channel",
            };
            return Err(api_err(
                StatusCode::BAD_REQUEST,
                format!("intake_rejected:{reason_code}"),
            ));
        }
        Err(error) => {
            if let Some(_rx) = rx_opt {
                state.cli_channel.drop_pending(&intake_id);
            }
            tracing::error!(%error, "cli intake: IntakeRouter error");
            return Err(api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error".into(),
            ));
        }
    };

    // Detached: ack the task_id; the deliverable lands in the fallback
    // file path. The CLI's `task deliverable <id>` (and `inbox`) surface
    // it after the fact.
    let Some(rx) = rx_opt else {
        return Ok((
            StatusCode::ACCEPTED,
            Json(CliIntakeAck {
                task_id: task_id.clone(),
                intake_id,
                deliverable: None,
                briefing_call_id: Some(task_id),
            }),
        ));
    };

    let max_wait = match q.max_wait_ms {
        Some(ms) => std::time::Duration::from_millis(ms),
        None => std::time::Duration::from_secs(
            std::env::var("CLI_INTAKE_MAX_WAIT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(CLI_INTAKE_DEFAULT_MAX_WAIT_SECS),
        ),
    };

    match tokio::time::timeout(max_wait, rx).await {
        Ok(Ok(deliverable)) => Ok((
            StatusCode::OK,
            Json(CliIntakeAck {
                task_id: task_id.clone(),
                intake_id,
                deliverable: Some(deliverable),
                briefing_call_id: Some(task_id),
            }),
        )),
        Ok(Err(_recv_err)) => {
            // Sender dropped — the file fallback will catch the
            // deliverable. Surface 504 so the CLI knows to look at
            // the inbox / fallback dir.
            tracing::warn!(%task_id, %intake_id, "cli intake deliver sender dropped before response");
            Err(api_err(
                StatusCode::GATEWAY_TIMEOUT,
                "deliver_dropped:pending_delivery".into(),
            ))
        }
        Err(_elapsed) => {
            // Leave the pending sender registered — when the
            // deliverable finally lands, CliChannel::deliver hits the
            // oneshot, gets a dropped-receiver, and falls back to the
            // file path. The operator can still recover the artifact.
            tracing::warn!(%task_id, %intake_id, "cli intake timed out waiting for delivery");
            Err(api_err(
                StatusCode::GATEWAY_TIMEOUT,
                "deliver_timeout:pending_delivery".into(),
            ))
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct InboxQuery {
    project_id: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
struct InboxEntry {
    /// Phase 2 alias: briefing_id := task_id. The Initializer mints a
    /// fresh `briefing_call_id` per edit cycle but reuses the same
    /// per-task mpsc sender (`AppState::briefing_senders` keyed by
    /// `task_id`), so the confirm route only needs a `task_id`. DEBT
    /// #20 documents the loose-match contract; tightening it later
    /// won't break this surface as long as the alias stays.
    briefing_id: String,
    task_id: String,
    project_id: String,
    title: String,
    brief: Option<serde_json::Value>,
    created_at: i64,
}

/// Local typedef just so clippy::type_complexity stops warning at the
/// `with_conn` closure return type — the inbox row is a flat 5-tuple
/// out of a raw SQL projection and adding a struct here would be churn.
type InboxRow = (String, String, String, Option<String>, i64);

async fn get_inbox_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Query(q): Query<InboxQuery>,
) -> Result<Json<Vec<InboxEntry>>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    authorize_in_handler(Action::TaskRead, &auth_ctx)?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as i64;
    let project_id = q.project_id.clone();
    // P5-HARD-IT7-M7: the inbox is a LIST endpoint story 5.5 missed — it
    // returned every tenant's briefed-task titles + brief content. Scope
    // it to the caller's tenant.
    let tenant = auth_ctx.tenant_id.clone();
    let rows: Vec<InboxRow> = state
        .db
        .with_conn(move |conn| -> rusqlite::Result<Vec<InboxRow>> {
            let (sql, mapped) = match project_id.as_deref() {
                Some(_) => (
                    "SELECT id, project_id, title, brief, created_at \
                           FROM tasks \
                          WHERE status = 'briefed' AND tenant_id = ? AND project_id = ? \
                          ORDER BY created_at DESC LIMIT ?",
                    true,
                ),
                None => (
                    "SELECT id, project_id, title, brief, created_at \
                           FROM tasks \
                          WHERE status = 'briefed' AND tenant_id = ? \
                          ORDER BY created_at DESC LIMIT ?",
                    false,
                ),
            };
            let mut stmt = conn.prepare(sql)?;
            let mapper = |row: &rusqlite::Row<'_>| -> rusqlite::Result<_> {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            };
            if mapped {
                let pid = project_id.unwrap();
                stmt.query_map(rusqlite::params![tenant, pid, limit], mapper)?
                    .collect()
            } else {
                stmt.query_map(rusqlite::params![tenant, limit], mapper)?
                    .collect()
            }
        })
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "inbox query failed");
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error".into())
        })?;

    let entries = rows
        .into_iter()
        .map(|(task_id, project_id, title, brief_text, created_at)| {
            let brief = brief_text
                .as_deref()
                .and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok());
            InboxEntry {
                briefing_id: task_id.clone(),
                task_id,
                project_id,
                title,
                brief,
                created_at,
            }
        })
        .collect();

    Ok(Json(entries))
}

#[derive(Debug, Deserialize)]
struct BriefingConfirmBody {
    action: String,
    #[serde(default)]
    edits: Option<seasoned_hand_core::agent::init::briefing::PartialBrief>,
}

async fn post_briefing_confirm_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Extension(auth_ctx): Extension<AuthContext>,
    Path(briefing_id): Path<String>,
    Json(body): Json<BriefingConfirmBody>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    use seasoned_hand_core::agent::init::briefing::{BriefingAction, UserResponse};

    require_loopback(remote)?;
    // P5-HARD-IT7-H8: confirming/cancelling/editing a briefing advances
    // a task's lifecycle — it was reachable with NO auth and NO tenant
    // check, letting any local caller drive another tenant's briefed
    // task. Gate it (TaskWrite) + tenant-scope the task below.
    authorize_in_handler(Action::TaskWrite, &auth_ctx)?;

    // Translate the wire action → BriefingAction enum.
    let action = match body.action.as_str() {
        "confirm" => BriefingAction::Confirm,
        "cancel" => BriefingAction::Cancel,
        "edit" => match body.edits {
            Some(edits) => BriefingAction::Edit { edits },
            None => {
                return Err(api_err(StatusCode::BAD_REQUEST, "missing_edits".into()));
            }
        },
        _ => {
            return Err(api_err(StatusCode::BAD_REQUEST, "invalid_action".into()));
        }
    };

    // Phase 2 alias: briefing_id := task_id (see InboxEntry doc).
    let task_id = briefing_id;

    // P5-HARD-IT7-H8: tenant-scope before touching the task's briefing.
    require_task_tenant(&state, &task_id, &auth_ctx).await?;

    // The Initializer reuses the same per-task receiver across every
    // call_id it emits, so the `in_reply_to_call_id` echo is loose
    // (DEBT #20). The handler tracks the most recent call_id only when
    // a future tightening lands.
    let sender = state
        .briefing_senders
        .get(&task_id)
        .map(|entry| entry.value().clone());
    let Some(sender) = sender else {
        return Err(api_err(StatusCode::NOT_FOUND, "no_pending_briefing".into()));
    };

    let response = UserResponse {
        in_reply_to_call_id: task_id.clone(),
        action,
    };
    sender
        .send(response)
        .await
        .map_err(|_| api_err(StatusCode::CONFLICT, "briefing_receiver_closed".into()))?;

    Ok(StatusCode::ACCEPTED)
}

#[cfg(test)]
mod tests {
    //! Inline unit tests for handlers whose security guards can't be
    //! exercised through the integration harness in `tests/`
    //! (because that harness always lands at `127.0.0.1`).

    use super::*;
    use seasoned_hand_core::router::SlotRouter;
    use seasoned_hand_core::sandbox::SandboxClient;
    use seasoned_hand_core::search::{SearchClient, SearchProvider};
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
