//! `AppState` + its builders and channel-registration wiring.
//! Extracted from `lib.rs` (issue #43 — god-file decomposition, follow-up
//! to the #22 batch F `error.rs`/`guards.rs` slices). Pure code move:
//! behavior is pinned by the integration suite.

use std::collections::HashMap;
use std::sync::Arc;

use dashmap::DashMap;

use seasoned_hand_core::agent::breaker::BreakerRegistry;
use seasoned_hand_core::agent::init::briefing::UserResponse;
use seasoned_hand_core::agent::narrate::NarratorHook;
use seasoned_hand_core::agent::{AgentRunner, AgentRunnerDeps};
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
    webhook::WebhookChannel,
};
use seasoned_hand_core::cost::CostClient;
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
use seasoned_hand_core::events::sqlite::SqliteEventStore;
use seasoned_hand_core::intake::{IntakeEventStore, IntakeRouter};
use seasoned_hand_core::llm::LlmClient;
use seasoned_hand_core::notify::{NotificationsSentStore, NotifyConfig};
use seasoned_hand_core::plan::PlanManager;
use seasoned_hand_core::project::{ProjectStore, TaskStore};
use seasoned_hand_core::pubsub::RedisPool;
use seasoned_hand_core::router::{SlotName, SlotRouter};
use seasoned_hand_core::sandbox::SandboxClient;
use seasoned_hand_core::search::SearchClient;
use seasoned_hand_core::tools::builtin::all_with_task_deliver;
use seasoned_hand_core::verifier::{
    VerificationStore,
    invalidation::{DEFAULT_MAX_PATHS, InvalidationDetector},
};

use crate::WsInitializerSpawner;

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

        let invalidation_detector = Arc::new(InvalidationDetector::new(DEFAULT_MAX_PATHS));
        let dispatcher = Arc::new(
            ToolDispatcher::new(all_with_task_deliver(task_deliver_deps))
                .with_hook(narrator.clone())
                .with_hook(Arc::new(EventEmittingHook::new(events.clone())))
                .with_hook(Arc::new(InvalidationHook::with_detector(
                    events.clone(),
                    Some(redis_arc.clone()),
                    invalidation_detector.clone(),
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
            invalidation_detector: Some(invalidation_detector),
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
