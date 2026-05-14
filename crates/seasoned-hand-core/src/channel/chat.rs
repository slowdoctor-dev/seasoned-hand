//! `ChatChannel` — wraps the Phase 0 WebSocket as a [`Channel`].
//!
//! Story 2.9 makes the chat panel one of several intake/delivery
//! channels instead of a hardcoded special path. Only two of the three
//! role traits are implemented:
//!
//! - [`IntakeProvider::run`] is a no-op (returns `Ok(())` immediately).
//!   The WS server's `task_create` handler converts incoming commands
//!   into [`IntakeEvent`]s and pushes them through the registry's
//!   shared mpsc directly — there is no long-lived listener to
//!   schedule. The trait impl exists for uniformity with other
//!   channels and so registry introspection reports
//!   `capabilities: ["intake", "delivery"]`.
//! - [`DeliverySink::deliver`] appends a `Misc` event whose payload
//!   carries `kind: "Deliverable"`. The event flows through the
//!   existing Redis pubsub → WS subscription pipeline, and the WS
//!   payload renderer (`ws::build_payload`) emits the
//!   architecture §4 `{kind: "Deliverable", deliverable_id, format,
//!   file_ref, citations}` shape into the session stream.
//!
//! No [`NotifySink`](super::NotifySink) impl — chat has no push-notify
//! semantics distinct from regular messages. Users who want push
//! notifications use NtfyChannel (story 2.13) or EmailChannel (2.11).
//!
//! refs: /specs/phase-2/architecture.md §2.7, §2.9
//! refs: /specs/phase-2/stories/story-2.9.md

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{
    ChannelError, Deliverable, DeliveryReceipt, DeliverySink, DeliveryTarget, IntakeEvent,
    IntakeProvider,
};
use crate::events::{EventStore, EventType, NewEvent, sqlite::SqliteEventStore};

/// Required prefix for the chat channel's `DeliveryTarget::target_ref`.
/// The remainder is the WS session id the deliverable should surface in.
pub const TARGET_SESSION_PREFIX: &str = "session:";

/// Channel name registered into the [`ChannelRegistry`](super::ChannelRegistry).
pub const CHANNEL_NAME: &str = "chat";

pub struct ChatChannel {
    events: Arc<SqliteEventStore>,
}

impl ChatChannel {
    pub fn new(events: Arc<SqliteEventStore>) -> Self {
        Self { events }
    }
}

#[async_trait]
impl IntakeProvider for ChatChannel {
    fn name(&self) -> &'static str {
        CHANNEL_NAME
    }

    async fn run(
        &self,
        _sink: mpsc::Sender<IntakeEvent>,
        _shutdown: CancellationToken,
    ) -> Result<(), ChannelError> {
        // No long-lived listener — the WS server pushes IntakeEvents
        // through the registry's shared mpsc directly from the
        // `task_create` command handler.
        Ok(())
    }
}

#[async_trait]
impl DeliverySink for ChatChannel {
    fn name(&self) -> &'static str {
        CHANNEL_NAME
    }

    async fn deliver(
        &self,
        target: &DeliveryTarget,
        deliverable: &Deliverable,
    ) -> Result<DeliveryReceipt, ChannelError> {
        let session_id = target
            .target_ref
            .strip_prefix(TARGET_SESSION_PREFIX)
            .ok_or_else(|| {
                ChannelError::Internal(format!(
                    "chat: target_ref must start with '{TARGET_SESSION_PREFIX}', got {:?}",
                    target.target_ref
                ))
            })?;
        if session_id.is_empty() {
            return Err(ChannelError::Internal(
                "chat: target_ref carries empty session id".into(),
            ));
        }

        let citations: Value = deliverable
            .citations
            .as_ref()
            .map(|c| Value::Array(c.iter().copied().map(Value::from).collect()))
            .unwrap_or_else(|| json!([]));

        let data = json!({
            "kind": "Deliverable",
            "deliverable_id": deliverable.id,
            "format": deliverable.format,
            "file_ref": deliverable.rendered_content_path,
            "citations": citations,
        });

        let event = self
            .events
            .append(NewEvent {
                session_id: session_id.to_string(),
                event_type: EventType::Misc,
                source: "channel:chat".into(),
                data,
            })
            .await
            .map_err(|err| ChannelError::Internal(format!("chat: event append failed: {err}")))?;

        Ok(DeliveryReceipt {
            channel: CHANNEL_NAME.to_string(),
            external_id: event.id.to_string(),
            delivered_at: event.timestamp,
            raw_response: json!({ "event_id": event.id, "session_id": session_id }),
        })
    }
}

#[cfg(test)]
mod tests;
