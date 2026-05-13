//! `IntakeProvider` role trait + `IntakeEvent` payload.
//!
//! refs: /specs/phase-2/architecture.md §2.7, §2.8
//! refs: /specs/phase-2/stories/story-2.4.md

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{ChannelError, DeliveryTarget};

/// One inbound brief carried from an external system into the kernel.
///
/// Persisted by the IntakeRouter (story 2.5) into the V008
/// `intake_events` table; field shapes mirror that schema so the
/// router can round-trip without translation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntakeEvent {
    /// Name of the registered [`super::ChannelRegistry`] entry that
    /// produced this event (matches the channel's `name()`).
    pub channel: String,
    /// External id unique within `channel` (HTTP request id, IMAP UID,
    /// chat message id, ...). Combined with `channel` it is the
    /// idempotency key for V008.
    pub intake_id: String,
    /// Natural-language brief from the caller. Becomes the seed for
    /// the Briefing protocol (§2.2).
    pub brief_input: String,
    /// Where to send the eventual deliverable. `None` means "the
    /// caller did not specify; default to the channel's reply path".
    pub reply_target: Option<DeliveryTarget>,
    /// Channel-specific structured context (sender address, subject,
    /// signature headers, ...). Stored verbatim for audit.
    pub metadata: Value,
    /// Multi-tenant slot — nullable in Phase 2, NOT NULL in Phase 5.
    pub tenant_id: Option<String>,
    /// Wall-clock receive time, microseconds since epoch.
    pub received_at: i64,
}

/// Long-lived listener that pushes [`IntakeEvent`]s into the kernel.
///
/// Implementations own one external connection (HTTP server, IMAP
/// poller, WS subscriber). The trait is object-safe — `#[async_trait]`
/// boxes the future and there are no generic methods — so the registry
/// can hold `Arc<dyn IntakeProvider>` slots.
#[async_trait]
pub trait IntakeProvider: Send + Sync {
    /// Stable identifier matching the registered channel name. Must
    /// be a static string so introspection (`/v1/channels`, CLI
    /// `channel list`) can render it without allocation.
    fn name(&self) -> &'static str;

    /// Run the listener's lifecycle. Push each new brief into `sink`;
    /// drop out cleanly when `shutdown` is cancelled. Returning `Ok(())`
    /// means "shut down on request"; any `Err` is a hard failure that
    /// the IntakeRouter (story 2.5) will log and surface via health.
    async fn run(
        &self,
        sink: mpsc::Sender<IntakeEvent>,
        shutdown: CancellationToken,
    ) -> Result<(), ChannelError>;
}
