//! `DeliverySink` role trait + `DeliveryTarget` / `DeliveryReceipt`.
//!
//! The [`Deliverable`] shape itself lives in [`crate::deliverable`]
//! since story 2.3 (V007); it's re-exported here so the `DeliverySink`
//! trait signature stays self-contained. Closes Phase 2 DEBT #10.
//!
//! refs: /specs/phase-2/architecture.md §2.7, §2.9, §2.11
//! refs: /specs/phase-2/stories/story-2.4.md
//! refs: /specs/phase-2/stories/story-2.3.md

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use crate::deliverable::Deliverable;

use super::ChannelError;

/// Where a deliverable should land. Channel-agnostic at the type level
/// (the `channel` field names which registered channel to dispatch to);
/// channel-specific routing lives in `target_ref` + `metadata`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryTarget {
    /// Registered channel name (matches the channel's `name()`).
    pub channel: String,
    /// Channel-specific address. Examples: `"msgid:<...>"` for email,
    /// `"thread:T01/C01/1234567890.123"` for chat, `"url:https://..."`
    /// for webhook.
    pub target_ref: String,
    /// Free-form structured context the channel impl interprets.
    pub metadata: Value,
}

/// Channel-side acknowledgement that a deliverable was accepted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryReceipt {
    /// Echo of the routed channel name.
    pub channel: String,
    /// External id assigned by the receiving system (posted message id,
    /// SMTP queue id, ticket number).
    pub external_id: String,
    /// Wall-clock acceptance time, microseconds since epoch.
    pub delivered_at: i64,
    /// Raw transport response, retained for audit / debugging.
    pub raw_response: Value,
}

/// Call-on-demand send of a [`Deliverable`] to a [`DeliveryTarget`].
///
/// Short-lived: invoked once per delivery attempt. Retry policy lives
/// in the DeliveryRouter (story 2.5), not here — the impl just reports
/// the outcome via [`ChannelError`] variants.
#[async_trait]
pub trait DeliverySink: Send + Sync {
    fn name(&self) -> &'static str;

    async fn deliver(
        &self,
        target: &DeliveryTarget,
        deliverable: &Deliverable,
    ) -> Result<DeliveryReceipt, ChannelError>;
}
