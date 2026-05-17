//! Seasoned Hand HTTP server.
//! refs: /specs/phase-0/architecture.md §4.1

use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
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
use seasoned_hand_core::intake::{IntakeEventStore, IntakeRouter};
use seasoned_hand_core::llm::LlmClient;
use seasoned_hand_core::notify::{NotificationsSentStore, NotifyConfig};
use seasoned_hand_core::plan::PlanManager;
use seasoned_hand_core::project::{ProjectStore, TaskStore};
use seasoned_hand_core::pubsub::RedisPool;
use seasoned_hand_core::router::{SlotName, SlotRouter};
use seasoned_hand_core::sandbox::SandboxClient;
use seasoned_hand_core::search::SearchClient;
use seasoned_hand_core::skill::{PlaybookStore, SkillStore};
use seasoned_hand_core::tools::builtin::all_with_task_deliver;
use seasoned_hand_core::verifier::{
    VerificationStore,
    routes::{ListQuery as VerifyListQuery, get_verification, list_verifications},
};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

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
        let skills = Arc::new(SkillStore::new(db.clone()));
        let playbooks = Arc::new(PlaybookStore::new(db.clone()));
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
            notify_config,
            workspace_ttl_cron,
            cli_channel: Arc::new(CliChannel::new()),
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

type ApiErrorResponse = (StatusCode, Json<ApiError>);
type ApiResult<T> = Result<T, ApiErrorResponse>;

#[derive(Debug, Deserialize, Default)]
struct ProgressQuery {
    lines: Option<usize>,
}

async fn list_events(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Path(session_id): Path<String>,
    Query(params): Query<EventsQueryParams>,
) -> Result<Json<Vec<seasoned_hand_core::events::Event>>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    let event_type = match params.event_type.as_deref() {
        Some(s) => Some(EventType::from_str(s).map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiError {
                    error: "unknown_event_type".into(),
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
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Query(params): Query<SessionsListParams>,
) -> Result<Json<Vec<SessionSummary>>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
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
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionDetail>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
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
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Path(session_id): Path<String>,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    workspace_proxy_inner(state, session_id, String::new()).await
}

async fn workspace_proxy(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Path((session_id, sub_path)): Path<(String, String)>,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
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

    let metadata = tokio::fs::metadata(&target).await.map_err(|_e| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: "workspace_not_found".into(),
            }),
        )
    })?;

    if metadata.is_dir() {
        let mut entries = Vec::new();
        let mut read_dir = tokio::fs::read_dir(&target).await.map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: "workspace_readdir_failed".into(),
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
        tracing::warn!(
            bytes = metadata.len(),
            cap = WORKSPACE_FILE_CAP_BYTES,
            "workspace file exceeds response cap"
        );
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(ApiError {
                error: "workspace_file_too_large".into(),
            }),
        ));
    }

    let bytes = tokio::fs::read(&target).await.map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: "workspace_read_failed".into(),
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
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Path(session_id): Path<String>,
) -> Result<Json<FeatureList>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
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
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Path(session_id): Path<String>,
    Query(q): Query<ProgressQuery>,
) -> Result<String, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
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
        // Story 2.17 / Phase 0 DEBT #16: admin-token-gated manual
        // workspace cleanup. Same 3-guard pattern as the rollback
        // route above (configured-token / loopback / header match).
        .route(
            "/v1/admin/sandbox/cleanup",
            axum::routing::post(post_admin_sandbox_cleanup_handler),
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
        // Story 2.15: per-task provenance manifest. Returns the latest
        // deliverable's manifest by default, or a specific deliverable's
        // when `?deliverable_id=...` is supplied. Spilled (file-ref)
        // manifests are transparently inflated.
        .route("/v1/tasks/:id/provenance", get(get_task_provenance_handler))
        // Story 2.21a: project + task surface for the `seasoned-hand`
        // CLI binary. Loopback-only (Phase 2 single-operator); Phase 5
        // multi-user will lift the constraint behind real auth.
        .route(
            "/v1/projects",
            get(list_projects_handler).post(create_project_handler),
        )
        .route(
            "/v1/projects/:id/archive",
            axum::routing::post(archive_project_handler),
        )
        .route("/v1/projects/:id/tasks", get(list_project_tasks_handler))
        .route("/v1/tasks/:id", get(get_task_handler))
        .route(
            "/v1/tasks/:id/deliverables",
            get(list_task_deliverables_handler),
        )
        .route(
            "/v1/tasks/:id/pause",
            axum::routing::post(post_task_pause_handler),
        )
        .route(
            "/v1/tasks/:id/resume",
            axum::routing::post(post_task_resume_handler),
        )
        .route(
            "/v1/tasks/:id/cancel",
            axum::routing::post(post_task_cancel_handler),
        )
        // Story 2.21b: CLI intake / inbox / briefing-confirm surface
        // (loopback-only, same posture as the 2.21a routes above).
        .route(
            "/v1/intake/cli",
            axum::routing::post(post_intake_cli_handler),
        )
        .route("/v1/inbox", get(get_inbox_handler))
        .route(
            "/v1/briefings/:id/confirm",
            axum::routing::post(post_briefing_confirm_handler),
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
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiError {
                    error: "unknown_role".into(),
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
        Err((
            StatusCode::FORBIDDEN,
            Json(ApiError {
                error: "forbidden_non_loopback".into(),
            }),
        ))
    }
}

const ADMIN_TOKEN_HEADER: &str = "X-Seasoned-Hand-Admin-Token";

fn require_admin_token_configured(state: &AppState) -> ApiResult<()> {
    if state.admin_token.is_empty() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiError {
                error: "admin_token_not_configured".into(),
            }),
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
        Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiError {
                error: "unauthorized_token".into(),
            }),
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
    headers: HeaderMap,
    Path((session_id, checkpoint_id)): Path<(String, String)>,
    Json(body): Json<RollbackBody>,
) -> ApiResult<(StatusCode, Json<RollbackResponse>)> {
    require_admin_route(&state, remote, &headers)?;
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
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Path(session_id): Path<String>,
    Query(q): Query<seasoned_hand_core::checkpoint::routes::ListQuery>,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
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
    Path(session_id): Path<String>,
    Query(q): Query<VerifyListQuery>,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    render_outcome(
        "list_verifications",
        list_verifications(&state.verifications, &session_id, q).await,
    )
}

async fn get_verification_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Path(id): Path<String>,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
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
    Path(task_id): Path<String>,
    Query(q): Query<seasoned_hand_core::provenance::GetTaskProvenanceQuery>,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    // Loopback gate matches every sibling /v1/tasks/:id/* handler; provenance
    // manifests can include PII (sender addresses, brief content, intake
    // metadata) so they must not leak at HOST=0.0.0.0 binds. See REVIEW
    // §5 cross-cutting issue #1 / proposed DEBT #34.
    require_loopback(remote)?;
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
    #[serde(default)]
    tenant_id: Option<String>,
}

async fn list_projects_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Query(q): Query<ProjectsListQuery>,
) -> Result<Json<Vec<seasoned_hand_core::project::Project>>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    let status = match q.status.as_deref() {
        Some("active") => Some(seasoned_hand_core::project::ProjectStatus::Active),
        Some("archived") => Some(seasoned_hand_core::project::ProjectStatus::Archived),
        Some(_other) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiError {
                    error: "unknown_status".into(),
                }),
            ));
        }
        None => None,
    };
    let limit = q.limit.unwrap_or(50);
    state
        .projects
        .list(status, q.cursor, limit)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!(error = %e, "list_projects");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: "internal_error".into(),
                }),
            )
        })
}

async fn create_project_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Json(body): Json<CreateProjectBody>,
) -> Result<(StatusCode, Json<seasoned_hand_core::project::Project>), (StatusCode, Json<ApiError>)>
{
    require_loopback(remote)?;
    if body.title.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiError {
                error: "empty_title".into(),
            }),
        ));
    }
    let id = state
        .projects
        .insert(seasoned_hand_core::project::NewProject {
            tenant_id: body.tenant_id,
            title: body.title,
            description: body.description,
        })
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "create_project");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: "internal_error".into(),
                }),
            )
        })?;
    let row = state.projects.get(&id).await.map_err(|e| {
        tracing::error!(error = %e, "create_project::get");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: "internal_error".into(),
            }),
        )
    })?;
    Ok((StatusCode::CREATED, Json(row)))
}

async fn archive_project_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    match state
        .projects
        .set_status(&id, seasoned_hand_core::project::ProjectStatus::Archived)
        .await
    {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(seasoned_hand_core::project::ProjectError::NotFound(_)) => Err((
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: "project_not_found".into(),
            }),
        )),
        Err(e) => {
            tracing::error!(error = %e, "archive_project");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: "internal_error".into(),
                }),
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
    Path(project_id): Path<String>,
    Query(q): Query<TasksListQuery>,
) -> Result<Json<Vec<seasoned_hand_core::project::Task>>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    let status = match q.status.as_deref() {
        Some(s) => match seasoned_hand_core::project::TaskStatus::from_db_str(s) {
            Ok(st) => Some(st),
            Err(_) => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiError {
                        error: "unknown_status".into(),
                    }),
                ));
            }
        },
        None => None,
    };
    let limit = q.limit.unwrap_or(50);
    state
        .tasks
        .list_by_project(&project_id, status, q.cursor, limit)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!(error = %e, "list_project_tasks");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: "internal_error".into(),
                }),
            )
        })
}

async fn get_task_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Path(id): Path<String>,
) -> Result<Json<seasoned_hand_core::project::Task>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    match state.tasks.get(&id).await {
        Ok(task) => Ok(Json(task)),
        Err(seasoned_hand_core::project::TaskError::NotFound(_)) => Err((
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: "task_not_found".into(),
            }),
        )),
        Err(e) => {
            tracing::error!(error = %e, "get_task");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: "internal_error".into(),
                }),
            ))
        }
    }
}

/// Story 2.22 backend: list every Deliverable row for a task and return
/// the latest session_id alongside. The frontend AgentComputer
/// `DeliverablesTab` joins these to build a download URL via the
/// existing `GET /v1/workspace/:session_id/*sub_path` proxy.
#[derive(Debug, serde::Serialize)]
struct TaskDeliverablesResponse {
    deliverables: Vec<seasoned_hand_core::deliverable::Deliverable>,
    latest_session_id: Option<String>,
}

async fn list_task_deliverables_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Path(task_id): Path<String>,
) -> Result<Json<TaskDeliverablesResponse>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    state.tasks.get(&task_id).await.map_err(|e| match e {
        seasoned_hand_core::project::TaskError::NotFound(_) => (
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: "task_not_found".into(),
            }),
        ),
        other => {
            tracing::error!(error = %other, "list_task_deliverables::lookup");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: "internal_error".into(),
                }),
            )
        }
    })?;
    let deliverables = state
        .deliverables
        .list_by_task(&task_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "list_task_deliverables");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: "internal_error".into(),
                }),
            )
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
    Path(task_id): Path<String>,
    body: Option<Json<TaskPauseBody>>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    // Confirm the task exists so the 404 path mirrors `get_task_handler`
    // before we touch session state.
    state.tasks.get(&task_id).await.map_err(|e| match e {
        seasoned_hand_core::project::TaskError::NotFound(_) => (
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: "task_not_found".into(),
            }),
        ),
        other => {
            tracing::error!(error = %other, "task_pause::lookup");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: "internal_error".into(),
                }),
            )
        }
    })?;
    let durable = body.and_then(|Json(b)| b.durable).unwrap_or(true);
    let session_id = ws::lookup_latest_session_for_task(&state, &task_id)
        .await
        .ok_or((
            StatusCode::CONFLICT,
            Json(ApiError {
                error: "no_active_session".into(),
            }),
        ))?;
    map_lifecycle_result(ws::handle_task_pause(&state, &session_id, durable).await)
}

async fn post_task_resume_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Path(task_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    state.tasks.get(&task_id).await.map_err(|e| match e {
        seasoned_hand_core::project::TaskError::NotFound(_) => (
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: "task_not_found".into(),
            }),
        ),
        other => {
            tracing::error!(error = %other, "task_resume::lookup");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: "internal_error".into(),
                }),
            )
        }
    })?;
    let session_id = ws::lookup_latest_session_for_task(&state, &task_id)
        .await
        .ok_or((
            StatusCode::CONFLICT,
            Json(ApiError {
                error: "no_active_session".into(),
            }),
        ))?;
    map_lifecycle_result(ws::handle_task_resume(&state, &session_id).await)
}

async fn post_task_cancel_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Path(task_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
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
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiError {
                    error: "task_not_found".into(),
                }),
            ));
        }
        Err(seasoned_hand_core::project::TaskError::IllegalTransition { from, .. }) => {
            return Err((
                StatusCode::CONFLICT,
                Json(ApiError {
                    error: format!("wrong_state:{}", from.as_db_str()),
                }),
            ));
        }
        Err(other) => {
            tracing::error!(error = %other, "task_cancel::set_status");
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: "internal_error".into(),
                }),
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
            Err((status, Json(ApiError { error: reason })))
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
    Query(q): Query<CliIntakeQuery>,
    Json(body): Json<CliIntakeBody>,
) -> Result<(StatusCode, Json<CliIntakeAck>), (StatusCode, Json<ApiError>)> {
    use seasoned_hand_core::channel::cli::{CHANNEL_NAME, INTAKE_ID_PREFIX, TARGET_INTAKE_PREFIX};
    use seasoned_hand_core::channel::{DeliveryTarget, IntakeEvent};
    use seasoned_hand_core::intake::router::{HandleOutcome, RejectionReason};

    require_loopback(remote)?;

    if body.brief.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiError {
                error: "intake_rejected:empty_brief".into(),
            }),
        ));
    }

    let intake_id = format!("{INTAKE_ID_PREFIX}{}", uuid::Uuid::new_v4());

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
        tenant_id: None,
        received_at: now_unix_micros(),
    };

    let task_id = match state.intake_router.handle_event(event).await {
        Ok(HandleOutcome::Created { task_id, .. }) => task_id,
        Ok(HandleOutcome::DuplicateSkipped) => {
            // Shouldn't happen — we mint a fresh UUID — but stay honest.
            if let Some(_rx) = rx_opt {
                state.cli_channel.drop_pending(&intake_id);
            }
            return Err((
                StatusCode::CONFLICT,
                Json(ApiError {
                    error: "intake_rejected:duplicate_intake_id".into(),
                }),
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
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiError {
                    error: format!("intake_rejected:{reason_code}"),
                }),
            ));
        }
        Err(error) => {
            if let Some(_rx) = rx_opt {
                state.cli_channel.drop_pending(&intake_id);
            }
            tracing::error!(%error, "cli intake: IntakeRouter error");
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: "internal_error".into(),
                }),
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
            Err((
                StatusCode::GATEWAY_TIMEOUT,
                Json(ApiError {
                    error: "deliver_dropped:pending_delivery".into(),
                }),
            ))
        }
        Err(_elapsed) => {
            // Leave the pending sender registered — when the
            // deliverable finally lands, CliChannel::deliver hits the
            // oneshot, gets a dropped-receiver, and falls back to the
            // file path. The operator can still recover the artifact.
            tracing::warn!(%task_id, %intake_id, "cli intake timed out waiting for delivery");
            Err((
                StatusCode::GATEWAY_TIMEOUT,
                Json(ApiError {
                    error: "deliver_timeout:pending_delivery".into(),
                }),
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
    Query(q): Query<InboxQuery>,
) -> Result<Json<Vec<InboxEntry>>, (StatusCode, Json<ApiError>)> {
    require_loopback(remote)?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as i64;
    let project_id = q.project_id.clone();
    let rows: Vec<InboxRow> = state
        .db
        .with_conn(move |conn| -> rusqlite::Result<Vec<InboxRow>> {
            let (sql, mapped) = match project_id.as_deref() {
                Some(_) => (
                    "SELECT id, project_id, title, brief, created_at \
                           FROM tasks \
                          WHERE status = 'briefed' AND project_id = ? \
                          ORDER BY created_at DESC LIMIT ?",
                    true,
                ),
                None => (
                    "SELECT id, project_id, title, brief, created_at \
                           FROM tasks \
                          WHERE status = 'briefed' \
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
                stmt.query_map(rusqlite::params![pid, limit], mapper)?
                    .collect()
            } else {
                stmt.query_map(rusqlite::params![limit], mapper)?.collect()
            }
        })
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "inbox query failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: "internal_error".into(),
                }),
            )
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
    Path(briefing_id): Path<String>,
    Json(body): Json<BriefingConfirmBody>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    use seasoned_hand_core::agent::init::briefing::{BriefingAction, UserResponse};

    require_loopback(remote)?;

    // Translate the wire action → BriefingAction enum.
    let action = match body.action.as_str() {
        "confirm" => BriefingAction::Confirm,
        "cancel" => BriefingAction::Cancel,
        "edit" => match body.edits {
            Some(edits) => BriefingAction::Edit { edits },
            None => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiError {
                        error: "missing_edits".into(),
                    }),
                ));
            }
        },
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiError {
                    error: "invalid_action".into(),
                }),
            ));
        }
    };

    // Phase 2 alias: briefing_id := task_id (see InboxEntry doc).
    let task_id = briefing_id;

    // The Initializer reuses the same per-task receiver across every
    // call_id it emits, so the `in_reply_to_call_id` echo is loose
    // (DEBT #20). The handler tracks the most recent call_id only when
    // a future tightening lands.
    let sender = state
        .briefing_senders
        .get(&task_id)
        .map(|entry| entry.value().clone());
    let Some(sender) = sender else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: "no_pending_briefing".into(),
            }),
        ));
    };

    let response = UserResponse {
        in_reply_to_call_id: task_id.clone(),
        action,
    };
    sender.send(response).await.map_err(|_| {
        (
            StatusCode::CONFLICT,
            Json(ApiError {
                error: "briefing_receiver_closed".into(),
            }),
        )
    })?;

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
}
