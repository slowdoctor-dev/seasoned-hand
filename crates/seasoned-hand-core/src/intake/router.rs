//! `IntakeRouter` — drains the channel-framework mpsc and seeds the
//! Task lifecycle.
//!
//! Story 2.5 wires the [`IntakeProvider`](crate::channel::IntakeProvider)
//! workers (spawned by [`crate::channel::ChannelRegistry::spawn_intakes`])
//! into the persistence layer. For each [`IntakeEvent`] received the
//! router:
//!
//! 1. Validates the payload (non-empty brief, registered channel).
//! 2. Persists via [`IntakeEventStore::insert`]. The V008 UNIQUE
//!    `(channel, intake_id)` constraint makes this idempotent — a
//!    duplicate insert short-circuits without crashing the drain loop.
//! 3. Resolves the target project (`metadata.project_id` override, else
//!    the tenant's `Inbox` fallback) and inserts a new Task in
//!    `drafted` state.
//! 4. Links the persisted intake row to its task via
//!    [`IntakeEventStore::link_to_task`].
//!
//! Spawning the Initializer (legacy Phase 1 1.4 entry point) and the
//! confirmation gate land in story 2.8; the 4xx / `intake_rejected`
//! Misc emit on validation failure also waits for the WebhookChannel
//! HTTP intake endpoint (story 2.10) and a system-session strategy —
//! both tracked in Phase 2 DEBT.
//!
//! refs: /specs/phase-2/architecture.md §2.7, §2.8
//! refs: /specs/phase-2/stories/story-2.5.md
//! refs: /specs/phase-2/stories/story-2.8.md (DEBT #13 close-out: spawner attachment)

use std::sync::{Arc, OnceLock};

use thiserror::Error;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::spawner::{InitializerSpawner, SpawnSpec};
use super::store::{IntakeEventStore, IntakeStoreError};
use crate::channel::{ChannelRegistry, IntakeEvent};
use crate::project::{NewTask, ProjectError, ProjectStore, TaskError, TaskStore};

/// Outcome of a single [`IntakeRouter::handle_event`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandleOutcome {
    /// Brand-new intake event → persisted, task created, link wired.
    /// `session_id` is `Some` when an [`InitializerSpawner`] was
    /// attached (story 2.8b — DEBT #13 close-out) and successfully
    /// minted the per-task session row; `None` otherwise (router
    /// without spawner, or spawner failed and was downgraded to a
    /// `tracing::warn!`).
    Created {
        intake_event_id: String,
        task_id: String,
        session_id: Option<String>,
    },
    /// A row with the same `(channel, intake_id)` already exists. The
    /// V008 UNIQUE constraint short-circuits the insert so the drain
    /// loop never crashes on duplicate webhook redeliveries.
    DuplicateSkipped,
    /// Validation rejected the event (empty brief, unknown channel,
    /// etc.). Phase 2 logs and drops — see module docs for the
    /// deferred 4xx / Misc emit story.
    Rejected(RejectionReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RejectionReason {
    EmptyBrief,
    UnknownChannel(String),
}

#[derive(Debug, Error)]
pub enum IntakeRouterError {
    #[error("intake store: {0}")]
    Intake(#[from] IntakeStoreError),
    #[error("task store: {0}")]
    Task(#[from] TaskError),
    #[error("project store: {0}")]
    Project(#[from] ProjectError),
}

pub struct IntakeRouter {
    intake_store: Arc<IntakeEventStore>,
    task_store: Arc<TaskStore>,
    project_store: Arc<ProjectStore>,
    registry: Arc<ChannelRegistry>,
    /// Story 2.8b: optional handoff to the Phase 2 confirm-gate
    /// Initializer. Stays empty for routers built before AppState wires
    /// the spawner (and for the core unit tests that don't need the
    /// Initializer). `OnceLock` so attachment is single-shot and
    /// safely shared across the spawned drain loop without mutex
    /// contention.
    initializer_spawner: OnceLock<Arc<dyn InitializerSpawner>>,
}

impl IntakeRouter {
    pub fn new(
        intake_store: Arc<IntakeEventStore>,
        task_store: Arc<TaskStore>,
        project_store: Arc<ProjectStore>,
        registry: Arc<ChannelRegistry>,
    ) -> Self {
        Self {
            intake_store,
            task_store,
            project_store,
            registry,
            initializer_spawner: OnceLock::new(),
        }
    }

    /// Story 2.8b: attach the AppState-side spawner exactly once.
    /// Returns `Err(existing)` if a spawner is already attached so the
    /// caller can decide whether to log + ignore or panic on double-init.
    /// Production wiring calls this from
    /// `AppState::new`'s tail; the chat / webhook integration tests
    /// rely on it being set before `task_create` lands.
    pub fn attach_initializer_spawner(
        &self,
        spawner: Arc<dyn InitializerSpawner>,
    ) -> Result<(), Arc<dyn InitializerSpawner>> {
        self.initializer_spawner.set(spawner)
    }

    pub fn has_initializer_spawner(&self) -> bool {
        self.initializer_spawner.get().is_some()
    }

    /// Drain `rx` until either the senders are dropped or `shutdown` is
    /// cancelled. Per-event failures are logged via `tracing::warn!` and
    /// the loop keeps going — the router treats individual brief
    /// failures as recoverable (the next event might be fine) rather
    /// than tearing the whole intake plane down.
    pub async fn run(&self, mut rx: mpsc::Receiver<IntakeEvent>, shutdown: CancellationToken) {
        loop {
            tokio::select! {
                biased;
                _ = shutdown.cancelled() => {
                    tracing::info!("intake_router: shutdown signalled, draining remaining events");
                    // Drain anything already queued so we don't lose
                    // in-flight briefs on shutdown — `recv()` returns
                    // None once the senders are dropped.
                    while let Ok(event) = rx.try_recv() {
                        if let Err(err) = self.handle_event(event).await {
                            tracing::warn!(error = %err, "intake_router: drop-drain handle_event failed");
                        }
                    }
                    return;
                }
                maybe = rx.recv() => {
                    let Some(event) = maybe else { return; };
                    if let Err(err) = self.handle_event(event).await {
                        tracing::warn!(error = %err, "intake_router: handle_event failed");
                    }
                }
            }
        }
    }

    pub async fn handle_event(
        &self,
        event: IntakeEvent,
    ) -> Result<HandleOutcome, IntakeRouterError> {
        // Validate.
        if event.brief_input.trim().is_empty() {
            tracing::warn!(channel = %event.channel, intake_id = %event.intake_id,
                "intake_router: rejecting empty brief");
            return Ok(HandleOutcome::Rejected(RejectionReason::EmptyBrief));
        }
        if self.registry.get_intake(&event.channel).is_none() {
            tracing::warn!(channel = %event.channel, intake_id = %event.intake_id,
                "intake_router: rejecting brief from unregistered channel");
            return Ok(HandleOutcome::Rejected(RejectionReason::UnknownChannel(
                event.channel.clone(),
            )));
        }

        // Persist intake row — V008 UNIQUE(channel, intake_id) is the
        // idempotency key. A duplicate insert surfaces as a SQLite
        // UNIQUE constraint error which we catch and treat as a
        // no-op skip.
        let intake_event_id = match self.intake_store.insert(&event).await {
            Ok(id) => id,
            Err(err) if is_unique_violation(&err) => {
                tracing::info!(channel = %event.channel, intake_id = %event.intake_id,
                    "intake_router: duplicate intake_id, skipping");
                return Ok(HandleOutcome::DuplicateSkipped);
            }
            Err(err) => return Err(err.into()),
        };

        // Resolve project: explicit override → Inbox fallback.
        let project_id = match event
            .metadata
            .get("project_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
        {
            Some(pid) => pid,
            None => {
                self.project_store
                    .find_or_create_inbox(event.tenant_id.as_deref())
                    .await?
            }
        };

        // Create the task. Title defaults to a truncated form of the
        // brief — the Initializer (Phase 1 1.4 / story 2.8) overwrites
        // it once a structured brief is authored.
        let title = derive_title(&event.brief_input);
        let task_id = self
            .task_store
            .insert(NewTask {
                project_id,
                tenant_id: event.tenant_id.clone(),
                title,
                expected_due_at: None,
            })
            .await?;

        // Late-bind intake → task.
        self.intake_store
            .link_to_task(&intake_event_id, &task_id)
            .await?;

        // Story 2.8b: hand the freshly-created task off to the confirm-gate
        // Initializer if a spawner is wired. Spawner failures stay
        // non-fatal — the intake row + drafted task are already
        // persisted, so we log and surface `session_id: None` instead of
        // tearing down the intake path. The caller can probe
        // `HandleOutcome::Created.session_id` to decide whether to retry.
        // `session_id_hint` flows from intake metadata (operator- or
        // webhook-supplied) straight into `sessions.id` (primary key)
        // and later into `workspace_root.join(session_id)` plus the
        // docker container name. An attacker who can submit intake
        // (today: anyone with the webhook intake token) can plant a
        // `..`-laden id that the TTL cron later resolves into a
        // host-side `rm -rf` outside the workspace bind-mount. Strict
        // UUID-like allowlist closes that. We drop (not error) so the
        // intake still creates the task — the spawner just mints a
        // fresh UUID. (REVIEW §1/G, proposed DEBT #35)
        let session_id_hint = event
            .metadata
            .get("session_id_hint")
            .and_then(serde_json::Value::as_str)
            .filter(|hint| is_safe_session_id(hint))
            .map(str::to_string);
        if event
            .metadata
            .get("session_id_hint")
            .and_then(serde_json::Value::as_str)
            .is_some()
            && session_id_hint.is_none()
        {
            tracing::warn!(
                channel = %event.channel,
                intake_id = %event.intake_id,
                "intake_router: dropping unsafe session_id_hint metadata field"
            );
        }
        let max_steps = event
            .metadata
            .get("max_steps")
            .and_then(serde_json::Value::as_u64)
            .and_then(|v| u32::try_from(v).ok());
        let cost_cap_cents = event
            .metadata
            .get("cost_cap_cents")
            .and_then(serde_json::Value::as_u64)
            .and_then(|v| u32::try_from(v).ok());
        let session_id = match self.initializer_spawner.get() {
            Some(spawner) => {
                let spec = SpawnSpec {
                    task_id: task_id.clone(),
                    brief_input: event.brief_input.clone(),
                    reply_target: event.reply_target.clone(),
                    session_id_hint,
                    max_steps,
                    cost_cap_cents,
                };
                match spawner.spawn(spec).await {
                    Ok(receipt) => Some(receipt.session_id),
                    Err(error) => {
                        tracing::warn!(%error, channel = %event.channel,
                            task_id = %task_id,
                            "intake_router: initializer spawner failed; task remains drafted");
                        None
                    }
                }
            }
            None => None,
        };

        Ok(HandleOutcome::Created {
            intake_event_id,
            task_id,
            session_id,
        })
    }
}

/// rusqlite reports UNIQUE constraint violations as
/// `Error::SqliteFailure { extended_code: 2067, .. }` or by message —
/// we match on the message to stay version-agnostic, mirroring how the
/// V008 store-level test already pins this shape.
fn is_unique_violation(err: &IntakeStoreError) -> bool {
    let msg = err.to_string();
    msg.contains("UNIQUE") || msg.contains("unique")
}

/// Accept a `session_id_hint` only if it matches the same strict
/// alphabet used by server-minted UUIDs (`[A-Za-z0-9-]`, length
/// 1..=64). Anything else is dropped silently — the spawner mints a
/// fresh UUID and the task still proceeds.
///
/// See `handle_event`'s threat model (REVIEW §1/G, proposed DEBT #35):
/// the hint flows into `sessions.id` (primary key) → `workspace_root.join(...)` →
/// `tokio::fs::remove_dir_all(...)` on the TTL cron + the docker
/// container name. `..` segments here are a host-side path-traversal
/// primitive.
///
/// Phase 4 security hardening iter-2 F1: this is now a thin wrapper over
/// the canonical `sandbox::is_safe_session_id` so the intake layer and
/// the sandbox layer share one definition (and one set of test cases).
fn is_safe_session_id(hint: &str) -> bool {
    crate::sandbox::is_safe_session_id(hint)
}

/// Truncate brief input to a single line ≤ 200 chars for the Task's
/// initial title. The Initializer rewrites this with the authored
/// brief's title field.
fn derive_title(brief: &str) -> String {
    let first_line = brief.lines().next().unwrap_or("").trim();
    if first_line.chars().count() <= 200 {
        return first_line.to_string();
    }
    first_line.chars().take(200).collect()
}
