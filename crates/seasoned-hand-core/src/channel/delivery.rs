//! `DeliverySink` role trait + `DeliveryTarget` / `DeliveryReceipt` / `Deliverable`.
//!
//! refs: /specs/phase-2/architecture.md §2.7, §2.9, §2.11
//! refs: /specs/phase-2/stories/story-2.4.md

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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

/// Minimal Deliverable shape needed for the routing layer. Story 2.3
/// lands the V007 `deliverables` table + persistence; this struct
/// mirrors the columns the routing layer references so the in-memory
/// shape and the on-disk shape stay aligned. Story 2.3 may move this
/// type into a dedicated `deliverable` module — until then it lives
/// here so the [`DeliverySink`] trait can be declared self-contained
/// in story 2.4.
///
/// refs: /specs/phase-2/architecture.md §3 V007
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deliverable {
    pub id: String,
    pub task_id: String,
    pub tenant_id: Option<String>,
    /// One of: `docx | pdf | html | pptx | xlsx | csv | md | json | code | url`.
    pub format: String,
    /// Workspace path of the rendered artifact (sandbox-relative).
    pub rendered_content_path: String,
    pub rendered_content_sha256: String,
    pub content_size: i64,
    /// Provenance manifest as defined in architecture §2.11. Stored
    /// here as opaque JSON so 2.4 doesn't redefine the schema.
    pub provenance_manifest: Value,
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
