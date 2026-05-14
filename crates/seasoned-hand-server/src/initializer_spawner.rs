//! `WsInitializerSpawner` — server-side concrete impl of the
//! [`InitializerSpawner`](seasoned_hand_core::intake::InitializerSpawner)
//! trait. Story 2.8b close-out for Phase 2 DEBT #13 + #15:
//! the IntakeRouter calls this from `handle_event(...)` to spin a
//! sessions row, register the briefing mpsc sender, and tokio::spawn
//! the [`Initializer::run_with_confirmation`] confirm-gate.
//!
//! Synchronously returns `SpawnReceipt { session_id }` so the WS
//! `task_create` Ack can hand the chat client a stable session id
//! before the confirm gate emits its first `briefing_pending` Misc.
//!
//! Lifecycle on the background task:
//! 1. Build a fresh [`Initializer`] with `task_store` attached.
//! 2. Call `run_with_confirmation(session_id, task_id, brief, ...)`.
//! 3. Drop the briefing sender entry from `AppState::briefing_senders`
//!    once the confirm gate returns (Started / Cancelled / error).
//! 4. On `RunOutcome::Started` — kick the agent loop via
//!    `runner.resume(req)`. The plan is already seeded by step 2's
//!    `seed_plan_and_run`, so `resume` (seed_task=false) is the right
//!    entry point.
//!
//! refs: /specs/phase-2/architecture.md §2.2, §2.8
//! refs: /specs/phase-2/stories/story-2.8.md
//! refs: /specs/phase-2/DEBT.md #13, #15

use async_trait::async_trait;
use seasoned_hand_core::agent::RunRequest;
use seasoned_hand_core::agent::init::Initializer;
use seasoned_hand_core::agent::init::briefing::{RunConfig, RunOutcome, UserResponse};
use seasoned_hand_core::intake::spawner::{
    InitializerSpawner, SpawnError, SpawnReceipt, SpawnSpec,
};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::AppState;

/// Default Worker-loop step budget when the channel that originated
/// the intake didn't supply one (webhook / email). Mirrors the
/// historical WS chat default.
const DEFAULT_MAX_STEPS: u32 = 24;

/// Per-task `UserResponse` channel capacity. The Initializer only ever
/// has one outstanding briefing at a time, but allow a small buffer in
/// case the user fires confirm + edit in quick succession before the
/// confirm gate dequeues.
const BRIEFING_CHANNEL_CAPACITY: usize = 8;

pub struct WsInitializerSpawner {
    state: AppState,
}

impl WsInitializerSpawner {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl InitializerSpawner for WsInitializerSpawner {
    async fn spawn(&self, spec: SpawnSpec) -> Result<SpawnReceipt, SpawnError> {
        // 1. Mint or accept the caller-supplied session id. WS chat
        //    pre-allocates so its reply_target encodes `session:<id>`
        //    up-front; webhook / email defer to the spawner.
        let session_id = spec
            .session_id_hint
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        // 2. Persist the sessions row. Required by the event store
        //    FK check (events::sqlite::SqliteEventStore::append rejects
        //    appends against unknown session ids — the Initializer
        //    starts emitting Misc events immediately, so the row must
        //    exist before the spawn returns).
        insert_session_row(&self.state, &session_id, "RUNNING")
            .await
            .map_err(|e| SpawnError::Other(format!("insert_session_row: {e}")))?;

        // 3. Register the briefing sender keyed by task_id so the WS
        //    `briefing_confirm` cmd handler can route user actions
        //    into the per-task confirm-gate receiver.
        //
        //    Note (story 2.8b): we key on **task_id** rather than
        //    briefing_call_id even though each `edit` action mints a
        //    fresh call_id. The Initializer's `recv` is per-task —
        //    one mpsc::Receiver consumes responses across every
        //    call_id the gate emits. The in_reply_to_call_id is
        //    available to the Initializer but currently not enforced
        //    (loose match — DEBT-noted on phase-2 ledger).
        let (tx, rx) = mpsc::channel::<UserResponse>(BRIEFING_CHANNEL_CAPACITY);
        let prev = self.state.briefing_senders.insert(spec.task_id.clone(), tx);
        if prev.is_some() {
            // Replacing a sender means the previous receiver leaked or
            // the same task_id is being re-spawned. Both are
            // architecturally unexpected; log but don't fail.
            tracing::warn!(
                task_id = %spec.task_id,
                "briefing_senders: replaced existing sender for task_id (re-spawn?)"
            );
        }

        // 4. Fire-and-forget the confirm-gate run + agent loop.
        let state = self.state.clone();
        let task_id = spec.task_id.clone();
        let session_id_clone = session_id.clone();
        let brief_input = spec.brief_input.clone();
        let max_steps = spec.max_steps.unwrap_or(DEFAULT_MAX_STEPS);
        let cost_cap_cents = spec.cost_cap_cents;
        tokio::spawn(async move {
            let initializer = Initializer::new(
                state.router.clone(),
                state.plan_manager.clone(),
                state.sandbox.clone(),
                state.events.clone(),
            )
            .with_task_store(state.tasks.clone());

            let outcome = initializer
                .run_with_confirmation(
                    &session_id_clone,
                    &task_id,
                    &brief_input,
                    RunConfig::default(),
                    rx,
                )
                .await;

            // Always release the briefing sender slot once the gate
            // returns; otherwise a long-lived map slowly accretes dead
            // entries.
            state.briefing_senders.remove(&task_id);

            match outcome {
                Ok(RunOutcome::Started) => {
                    // Plan + feature-list / progress are already seeded
                    // inside `run_with_confirmation::seed_plan_and_run`,
                    // so we resume into the worker loop (seed_task=false).
                    let req = RunRequest {
                        session_id: session_id_clone,
                        input: brief_input,
                        max_steps,
                        cost_cap_cents,
                    };
                    if let Err(error) = state.runner.resume(req).await {
                        tracing::warn!(%error, %task_id, "agent runner resume failed after briefing confirm");
                    }
                }
                Ok(RunOutcome::Cancelled) => {
                    tracing::info!(%task_id, "briefing cancelled by user; agent loop NOT started");
                }
                Err(error) => {
                    tracing::warn!(%error, %task_id, "initializer confirm-gate failed");
                }
            }
        });

        Ok(SpawnReceipt { session_id })
    }
}

async fn insert_session_row(state: &AppState, session_id: &str, name: &str) -> Result<(), String> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now: i64 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_micros()).unwrap_or(i64::MAX))
        .unwrap_or(0);
    let id = session_id.to_string();
    let name = name.to_string();
    state
        .db
        .with_conn(move |conn| {
            conn.execute(
                "INSERT INTO sessions (id, created_at, updated_at, state) VALUES (?, ?, ?, ?)",
                (id, now, now, name),
            )
        })
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}
