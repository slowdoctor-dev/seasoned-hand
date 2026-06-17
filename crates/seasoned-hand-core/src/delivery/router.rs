//! `DeliveryRouter` — dispatches a completed task's [`Deliverable`] to
//! the channel named by its `reply_target`.
//!
//! Story 2.5 wires the V007 `DeliverableStore` + V008
//! `delivery_events` log to the channel-framework `DeliverySink` role.
//! [`DeliveryRouter::deliver_task`] is the single entry point: it
//! looks up the task's originating intake event for the default
//! `reply_target`, resolves the matching sink in the
//! [`ChannelRegistry`], calls
//! [`DeliverableStore::assert_exists`](crate::deliverable::DeliverableStore::assert_exists)
//! as the existence guard (DEBT #11 contract), invokes
//! [`DeliverySink::deliver`], and persists a
//! [`DeliveryEvent`](crate::delivery::DeliveryEventRow).
//!
//! Retry policy follows the channel-error variants:
//! - `Http(_)`, `Transport(_)`, and `RemoteRejected { status: 500..=599 }`
//!   get exactly one retry after `retry_delay` (default 30 s).
//! - `RemoteRejected { status: < 500 }`, `Decode(_)`, and `Internal(_)`
//!   are terminal — no retry.
//! - `Cancelled` propagates (shutdown).
//!
//! On terminal failure the router persists a row with `ok = false` +
//! the error message and best-effort appends a
//! `Misc{kind:"delivery_failed"}` event tagged to the task's most
//! recent session. Tasks with no live session (e.g. test fixtures)
//! skip the Misc emit with a tracing warning.
//!
//! refs: /specs/phase-2/architecture.md §2.9
//! refs: /specs/phase-2/stories/story-2.5.md

use std::sync::Arc;
use std::time::Duration;

use rusqlite::OptionalExtension;
use thiserror::Error;
use tokio::time::sleep;

use crate::channel::{ChannelError, ChannelRegistry, DeliveryTarget};
use crate::db::DbPool;
use crate::deliverable::{Deliverable, DeliverableError, DeliverableStore};
use crate::delivery::store::{
    DeliveryEventRow, DeliveryEventStore, DeliveryStoreError, NewDeliveryEvent,
};
use crate::events::{EventStore, EventType, NewEvent, sqlite::SqliteEventStore};
use crate::intake::store::{IntakeEventStore, IntakeStoreError};
use crate::time::now_micros;

/// Default retry pause for transient delivery failures. Spec §2.9 sets
/// this at 30 s; tests override via [`DeliveryRouter::with_retry_delay`].
pub const DEFAULT_RETRY_DELAY: Duration = Duration::from_secs(30);

#[derive(Debug, Error)]
pub enum DeliveryRouterError {
    #[error("no deliverable for task: {0}")]
    DeliverableMissing(String),
    #[error("intake row missing for task: {0}")]
    IntakeMissing(String),
    #[error("task {task_id} has no reply_target on its intake row")]
    ReplyTargetMissing { task_id: String },
    #[error("no DeliverySink registered for channel: {0}")]
    SinkMissing(String),
    #[error("delivery cancelled")]
    Cancelled,
    #[error("delivery failed terminally: {0}")]
    Terminal(String),
    #[error("deliverable store: {0}")]
    Deliverable(#[from] DeliverableError),
    #[error("delivery store: {0}")]
    Delivery(#[from] DeliveryStoreError),
    #[error("intake store: {0}")]
    Intake(#[from] IntakeStoreError),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

pub struct DeliveryRouter {
    registry: Arc<ChannelRegistry>,
    delivery_store: Arc<DeliveryEventStore>,
    deliverable_store: Arc<DeliverableStore>,
    intake_store: Arc<IntakeEventStore>,
    events: Arc<SqliteEventStore>,
    db: DbPool,
    retry_delay: Duration,
}

impl DeliveryRouter {
    pub fn new(
        registry: Arc<ChannelRegistry>,
        delivery_store: Arc<DeliveryEventStore>,
        deliverable_store: Arc<DeliverableStore>,
        intake_store: Arc<IntakeEventStore>,
        events: Arc<SqliteEventStore>,
        db: DbPool,
    ) -> Self {
        Self {
            registry,
            delivery_store,
            deliverable_store,
            intake_store,
            events,
            db,
            retry_delay: DEFAULT_RETRY_DELAY,
        }
    }

    /// Override the retry pause — production keeps the 30 s default;
    /// tests use `Duration::ZERO` so `retries_5xx_once` doesn't block
    /// the harness for half a minute.
    #[must_use]
    pub fn with_retry_delay(mut self, delay: Duration) -> Self {
        self.retry_delay = delay;
        self
    }

    /// Dispatch the most recent [`Deliverable`] for `task_id` to the
    /// channel named by its intake's `reply_target`. Returns the
    /// persisted [`DeliveryEventRow`] on success; on terminal failure
    /// returns an error AND persists a `ok = false` row so the audit
    /// trail records the attempt either way.
    pub async fn deliver_task(
        &self,
        task_id: &str,
    ) -> Result<DeliveryEventRow, DeliveryRouterError> {
        let deliverables = self.deliverable_store.list_by_task(task_id).await?;
        let deliverable = deliverables
            .into_iter()
            .next_back()
            .ok_or_else(|| DeliveryRouterError::DeliverableMissing(task_id.to_string()))?;

        let intake = self
            .intake_store
            .get_by_task_id(task_id)
            .await?
            .ok_or_else(|| DeliveryRouterError::IntakeMissing(task_id.to_string()))?;

        let target =
            intake
                .reply_target
                .clone()
                .ok_or_else(|| DeliveryRouterError::ReplyTargetMissing {
                    task_id: task_id.to_string(),
                })?;

        let sink = self
            .registry
            .get_delivery(&target.channel)
            .ok_or_else(|| DeliveryRouterError::SinkMissing(target.channel.clone()))?;

        // Existence guard — closes the DEBT #11 contract.
        self.deliverable_store
            .assert_exists(&deliverable.id)
            .await?;

        // First attempt.
        let attempt_outcome = sink.deliver(&target, &deliverable).await;
        let final_outcome = match attempt_outcome {
            Ok(receipt) => Ok(receipt),
            Err(err) if is_retryable(&err) => {
                tracing::warn!(error = %err, channel = %target.channel,
                    "delivery_router: retryable failure, scheduling single retry");
                if self.retry_delay > Duration::ZERO {
                    sleep(self.retry_delay).await;
                }
                sink.deliver(&target, &deliverable).await
            }
            Err(ChannelError::Cancelled) => {
                return Err(DeliveryRouterError::Cancelled);
            }
            Err(other) => Err(other),
        };

        match final_outcome {
            Ok(receipt) => self.persist_success(&deliverable, &target, receipt).await,
            Err(ChannelError::Cancelled) => Err(DeliveryRouterError::Cancelled),
            Err(err) => {
                self.persist_failure(task_id, &deliverable, &target, &err)
                    .await
            }
        }
    }

    async fn persist_success(
        &self,
        deliverable: &Deliverable,
        target: &DeliveryTarget,
        receipt: crate::channel::DeliveryReceipt,
    ) -> Result<DeliveryEventRow, DeliveryRouterError> {
        let new = NewDeliveryEvent {
            tenant_id: deliverable.tenant_id.clone(),
            task_id: deliverable.task_id.clone(),
            deliverable_id: deliverable.id.clone(),
            channel: target.channel.clone(),
            target: target.clone(),
            ok: true,
            external_id: Some(receipt.external_id),
            error: None,
            delivered_at: receipt.delivered_at,
        };
        let id = self.delivery_store.insert(new.clone()).await?;
        Ok(DeliveryEventRow {
            id,
            tenant_id: new.tenant_id,
            task_id: new.task_id,
            deliverable_id: new.deliverable_id,
            channel: new.channel,
            target: new.target,
            ok: new.ok,
            external_id: new.external_id,
            error: new.error,
            delivered_at: new.delivered_at,
        })
    }

    async fn persist_failure(
        &self,
        task_id: &str,
        deliverable: &Deliverable,
        target: &DeliveryTarget,
        err: &ChannelError,
    ) -> Result<DeliveryEventRow, DeliveryRouterError> {
        let now = now_micros();
        // Issue #23: scrub credentials from the error text before it's persisted to
        // `delivery_events` (a channel error can carry a URL with an embedded token).
        let error_text = crate::text::scrub_secrets(&err.to_string());
        let new = NewDeliveryEvent {
            tenant_id: deliverable.tenant_id.clone(),
            task_id: deliverable.task_id.clone(),
            deliverable_id: deliverable.id.clone(),
            channel: target.channel.clone(),
            target: target.clone(),
            ok: false,
            external_id: None,
            error: Some(error_text.clone()),
            delivered_at: now,
        };
        let delivery_event_id = self.delivery_store.insert(new.clone()).await?;
        tracing::debug!(%delivery_event_id, "delivery_router: failure row persisted");

        // Best-effort Misc emit. Pre-completed tasks may have no
        // session attached yet; in that case we log and continue —
        // the persisted delivery_event row is the canonical audit
        // record either way.
        if let Some(session_id) = self.latest_session_for_task(task_id).await? {
            let payload = serde_json::json!({
                "kind": "delivery_failed",
                "deliverable_id": deliverable.id,
                "channel": target.channel,
                "error": error_text,
            });
            if let Err(e) = self
                .events
                .append(NewEvent {
                    session_id,
                    event_type: EventType::Misc,
                    source: "delivery_router".into(),
                    data: payload,
                })
                .await
            {
                tracing::warn!(error = %e, "delivery_router: Misc{{delivery_failed}} append failed");
            }
        } else {
            tracing::warn!(task_id = %task_id, error = %error_text,
                "delivery_router: terminal failure but no session — skipping Misc emit");
        }

        Err(DeliveryRouterError::Terminal(error_text))
    }

    async fn latest_session_for_task(
        &self,
        task_id: &str,
    ) -> Result<Option<String>, DeliveryRouterError> {
        let tid = task_id.to_string();
        let result: rusqlite::Result<Option<String>> = self
            .db
            .with_conn(move |conn| {
                conn.query_row(
                    "SELECT id FROM sessions WHERE task_id = ? \
                      ORDER BY updated_at DESC LIMIT 1",
                    [&tid],
                    |row| row.get::<_, String>(0),
                )
                .optional()
            })
            .await;
        Ok(result?)
    }
}

fn is_retryable(err: &ChannelError) -> bool {
    match err {
        ChannelError::Http(_) | ChannelError::Transport(_) => true,
        ChannelError::RemoteRejected { status, .. } => (500..600).contains(status),
        ChannelError::Decode(_) | ChannelError::Internal(_) | ChannelError::Cancelled => false,
    }
}
