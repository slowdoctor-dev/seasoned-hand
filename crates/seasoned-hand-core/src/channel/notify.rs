//! `NotifySink` role trait + `NotifyTarget` / `NotifyEvent` / `NotifyReceipt`.
//!
//! refs: /specs/phase-2/architecture.md §2.7
//! refs: /specs/phase-2/stories/story-2.4.md

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ChannelError;

/// Where a notification should be delivered. Same shape as
/// [`super::DeliveryTarget`] but kept as a distinct type so the
/// router-level routing tables can't accidentally cross-wire
/// delivery destinations into notify dispatches.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyTarget {
    pub channel: String,
    pub target_ref: String,
    pub metadata: Value,
}

/// Status signal pushed through a notify channel. Unlike a delivery
/// it does not carry an artifact — `payload` is a small JSON blob
/// (task transition, narrator line, escalation alert).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyEvent {
    /// Free-form classification used by the NotifyWorker to pick a
    /// rendering template (e.g., `"task_finished"`, `"escalation"`,
    /// `"narrator_milestone"`).
    pub trigger_kind: String,
    /// Optional — pre-task notifies (e.g., briefing escalation) may
    /// fire before a task exists.
    pub task_id: Option<String>,
    /// Channel-rendered payload (already-prepared title + body, plus
    /// any channel-specific extras).
    pub payload: Value,
}

/// Channel-side acknowledgement that a notification was accepted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyReceipt {
    pub channel: String,
    /// Some channels (ntfy, webhook) return an id; some (CLI stdout)
    /// do not.
    pub external_id: Option<String>,
    pub sent_at: i64,
    pub raw_response: Value,
}

/// Call-on-demand push of a status signal.
///
/// Many notifies per task; usually one [`super::DeliverySink`] call per
/// task. Same call-shape as `DeliverySink` with a different payload
/// type — kept as a separate trait so the type system enforces "ntfy
/// can't do delivery" at compile time.
#[async_trait]
pub trait NotifySink: Send + Sync {
    fn name(&self) -> &'static str;

    async fn notify(
        &self,
        target: &NotifyTarget,
        event: &NotifyEvent,
    ) -> Result<NotifyReceipt, ChannelError>;
}
