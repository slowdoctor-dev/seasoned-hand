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
use seasoned_hand_core::agent::init::briefing::UserResponse;
use seasoned_hand_core::agent::init::feature_list::FeatureList;
use seasoned_hand_core::agent::init::progress;
use seasoned_hand_core::agent::narrate::NarratorHook;
use seasoned_hand_core::agent::{AgentRunner, AgentRunnerDeps};
use seasoned_hand_core::browser::tracks::PostBrowserActionHook;
use seasoned_hand_core::capability::ModelCapabilities;
use seasoned_hand_core::channel::{
    ChannelRegistration, ChannelRegistry,
    chat::ChatChannel,
    email::{
        AllowList, AsyncImapFetcher, EmailChannel, ImapConfig, LettreSmtpTransport, SmtpConfig,
    },
    webhook::{TokenCheck, WebhookChannel},
};
use seasoned_hand_core::cost::{CostClient, CostSnapshot};
use seasoned_hand_core::db::DbPool;
use seasoned_hand_core::deliverable::DeliverableStore;
use seasoned_hand_core::delivery::{DeliveryEventStore, DeliveryRouter};
use seasoned_hand_core::dispatch::mask::DefaultMaskPolicy;
use seasoned_hand_core::dispatch::{
    ToolDispatcher,
    hooks::{EventEmittingHook, InvalidationHook},
};
use seasoned_hand_core::events::{EventQuery, EventStore, EventType, sqlite::SqliteEventStore};
use seasoned_hand_core::intake::{IntakeEventStore, IntakeRouter};
use seasoned_hand_core::llm::LlmClient;
use seasoned_hand_core::notify::NotificationsSentStore;
use seasoned_hand_core::plan::PlanManager;
use seasoned_hand_core::project::{ProjectStore, TaskStore};
use seasoned_hand_core::pubsub::RedisPool;
use seasoned_hand_core::router::{SlotName, SlotRouter};
use seasoned_hand_core::sandbox::SandboxClient;
use seasoned_hand_core::search::SearchClient;
use seasoned_hand_core::skill::{PlaybookStore, SkillStore};
use seasoned_hand_core::tools::register_builtin_tools;
use seasoned_hand_core::verifier::{
    VerificationStore,
    routes::{ListQuery as VerifyListQuery, get_verification, list_verifications},
};
use serde::{Deserialize, Serialize};

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
    /// Story 2.3 / DEBT #6: V009 `skills` reservation handle. Phase 2
    /// never writes; Phase 3 Curator populates.
    pub skills: Arc<SkillStore>,
    /// Story 2.3 / DEBT #6: V009 `playbooks` reservation handle.
    /// Phase 2 never writes; Phase 3 Curator populates.
    pub playbooks: Arc<PlaybookStore>,
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
        let dispatcher = Arc::new(
            ToolDispatcher::new(register_builtin_tools())
                .with_hook(narrator.clone())
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
        // Story 2.3: Phase 2 OS-shape persistence handles. All eight
        // stores share the same pool — concurrency is gated by the
        // pool's inner `Mutex<Connection>`, so this is safe.
        let projects = Arc::new(ProjectStore::new(db.clone()));
        let tasks_store = Arc::new(TaskStore::new(db.clone()));
        let deliverables = Arc::new(DeliverableStore::new(db.clone()));
        let intake_events = Arc::new(IntakeEventStore::new(db.clone()));
        let delivery_events = Arc::new(DeliveryEventStore::new(db.clone()));
        let notifications_sent = Arc::new(NotificationsSentStore::new(db.clone()));
        let skills = Arc::new(SkillStore::new(db.clone()));
        let playbooks = Arc::new(PlaybookStore::new(db.clone()));
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
            skills,
            playbooks,
            channels,
            intake_router,
            delivery_router,
            webhook_intake_token: Arc::new(String::new()),
            briefing_senders,
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

    /// Story 1.13b: enable the opt-in Verifier-driven rollback path.
    /// Defaults `false` per phase-1/DEBT.md #3.
    pub fn with_rollback_on_verifier_fail(mut self, enabled: bool) -> Self {
        self.checkpoint_rollback_on_verifier_fail = enabled;
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
        // Story 2.5: channel introspection.
        .route("/v1/channels", get(list_channels_handler))
        .route("/v1/channels/:name/health", get(get_channel_health_handler))
        .route(
            "/v1/channels/:name/test",
            axum::routing::post(post_channel_test_handler),
        )
        // Story 2.10: WebhookChannel intake source — HTTP POST is the
        // long-lived listener (the channel's `IntakeProvider::run` is
        // a no-op and parks on shutdown, see channel/webhook/mod.rs).
        .route(
            "/v1/intake/webhook",
            axum::routing::post(post_intake_webhook_handler),
        )
        .with_state(state)
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

async fn list_channels_handler(State(state): State<AppState>) -> Json<Vec<ChannelHealthDto>> {
    let snapshot = state
        .channels
        .health()
        .into_iter()
        .map(ChannelHealthDto::from)
        .collect();
    Json(snapshot)
}

async fn get_channel_health_handler(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<ChannelHealthDto>, (StatusCode, Json<ApiError>)> {
    state
        .channels
        .health()
        .into_iter()
        .find(|h| h.name == name)
        .map(ChannelHealthDto::from)
        .map(Json)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiError {
                    error: "channel_not_found".into(),
                }),
            )
        })
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
    Path(name): Path<String>,
    Query(q): Query<ChannelTestQuery>,
) -> Result<Json<ChannelTestResponse>, (StatusCode, Json<ApiError>)> {
    let role = q.role.as_deref().unwrap_or("delivery");
    let registered = match role {
        "intake" => state.channels.get_intake(&name).is_some(),
        "delivery" => state.channels.get_delivery(&name).is_some(),
        "notify" => state.channels.get_notify(&name).is_some(),
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiError {
                    error: format!("unknown role: {other}"),
                }),
            ));
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
        return Err((StatusCode::NOT_FOUND, Json(ApiError { error: err.into() })));
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
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ApiError {
                    error: "intake_token_not_configured".into(),
                }),
            ));
        }
        TokenCheck::Mismatch => {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ApiError {
                    error: "unauthorized_token".into(),
                }),
            ));
        }
        TokenCheck::Ok => {}
    }

    let Json(body) = body.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiError {
                error: "invalid_json_body".into(),
            }),
        )
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
        received_at: now_unix_micros(),
    };

    match state.intake_router.handle_event(intake_event).await {
        Ok(HandleOutcome::Created { task_id, .. }) => Ok((
            StatusCode::ACCEPTED,
            Json(WebhookIntakeAck {
                task_id,
                briefing_call_id: None,
            }),
        )),
        Ok(HandleOutcome::DuplicateSkipped) => Err((
            StatusCode::CONFLICT,
            Json(ApiError {
                error: "intake_rejected:duplicate_intake_id".into(),
            }),
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
            Err((
                StatusCode::BAD_REQUEST,
                Json(ApiError {
                    error: format!("intake_rejected:{reason_code}"),
                }),
            ))
        }
        Err(error) => {
            tracing::error!(%error, "webhook intake: IntakeRouter error");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: "internal_error".into(),
                }),
            ))
        }
    }
}

fn now_unix_micros() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0)
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
            headers,
            Path(("sess-x".to_string(), "cp-x".to_string())),
            Json(RollbackBody { reason: "x".into() }),
        )
        .await;
        let err = outcome.expect_err("non-loopback must be 403");
        assert_eq!(err.0, StatusCode::FORBIDDEN);
        assert_eq!(err.1.0.error, "forbidden_non_loopback");
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
            headers,
            Path(("sess-x".to_string(), "cp-x".to_string())),
            Json(RollbackBody { reason: "x".into() }),
        )
        .await;
        let err = outcome.expect_err("remote + wrong token still 403, not 401");
        assert_eq!(err.0, StatusCode::FORBIDDEN);
        assert_eq!(err.1.0.error, "forbidden_non_loopback");
    }
}
