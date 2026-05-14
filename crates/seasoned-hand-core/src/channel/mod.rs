//! Channel framework — the OS-shape keystone of Phase 2.
//!
//! A **Channel** is a single integration with an external system (one
//! Email account, one Slack workspace, one webhook endpoint). Each
//! channel may play 1, 2, or 3 of three role traits depending on what
//! the underlying system supports:
//!
//! - [`IntakeProvider`] — long-lived listener (HTTP server, IMAP
//!   poller, WS subscriber) that pushes briefs into the kernel.
//! - [`DeliverySink`] — short-lived call-on-demand send of a completed
//!   [`Deliverable`] back to a target.
//! - [`NotifySink`] — short-lived send of a status signal (not an
//!   artifact).
//!
//! The three traits intentionally stay separate so a unified
//! `Channel::handle(Operation)` doesn't force runtime dispatch over
//! roles a channel doesn't support (ntfy can't do intake). Each
//! concrete channel is one struct that implements 1-3 of the traits;
//! it registers itself via [`ChannelRegistration`] (a builder) and the
//! same `Arc<C>` is cloned into each role slot the channel populates.
//!
//! Story 2.4 ships only the trait surface + [`ChannelRegistry`].
//! Concrete channels (Webhook / Email / Chat / CLI / Ntfy) land in
//! stories 2.9–2.13. The IntakeRouter / DeliveryRouter that consume
//! the registry land in story 2.5.
//!
//! refs: /specs/phase-2/architecture.md §2.7
//! refs: /specs/phase-2/stories/story-2.4.md

pub mod chat;
pub mod delivery;
pub mod email;
pub mod intake;
pub mod notify;
pub mod webhook;

use std::collections::HashMap;
use std::sync::Arc;

use thiserror::Error;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub use delivery::{Deliverable, DeliveryReceipt, DeliverySink, DeliveryTarget};
pub use intake::{IntakeEvent, IntakeProvider};
pub use notify::{NotifyEvent, NotifyReceipt, NotifySink, NotifyTarget};

/// Error returned by any channel role operation.
///
/// Variants are intentionally distinct so the routing layer (story
/// 2.5) can branch retry policy on the failure mode without
/// re-parsing strings. Don't collapse `Http`, `Transport`, and
/// `RemoteRejected` — each maps to a different retry decision.
#[derive(Debug, Error)]
pub enum ChannelError {
    /// HTTP-layer failure (connection refused, TLS handshake, timeout
    /// before a status line was seen). Generally retryable.
    #[error("http error: {0}")]
    Http(String),
    /// Non-HTTP transport failure (SMTP, IMAP, Redis pub/sub, raw
    /// socket). Generally retryable.
    #[error("transport error: {0}")]
    Transport(String),
    /// Response or payload decode failure. Retrying won't help.
    #[error("decode error: {0}")]
    Decode(String),
    /// Remote saw the request and rejected it deliberately. Retry
    /// policy depends on `status` — 4xx generally no retry, 5xx
    /// usually one retry.
    #[error("remote rejected (status={status}): {message}")]
    RemoteRejected { status: u16, message: String },
    /// Operation aborted because the shared shutdown token was
    /// cancelled. Not a failure — propagated so the router can short
    /// out cleanly.
    #[error("operation cancelled")]
    Cancelled,
    /// Bug in the channel impl itself (invariant violation, missing
    /// config). Surfaced as fatal by the router.
    #[error("internal channel error: {0}")]
    Internal(String),
}

/// Builder for one channel's registration in the [`ChannelRegistry`].
///
/// The slot methods take `Arc<dyn Role>` — not `Arc<C>` — so a
/// concrete `C: IntakeProvider + DeliverySink + NotifySink` is wrapped
/// in `Arc::new(C { ... })` once by the caller, then the same `Arc`
/// is cloned (`channel.clone()`) into each role slot the channel
/// populates. Idiomatic for one struct playing multiple trait roles.
pub struct ChannelRegistration {
    name: String,
    intake: Option<Arc<dyn IntakeProvider>>,
    delivery: Option<Arc<dyn DeliverySink>>,
    notify: Option<Arc<dyn NotifySink>>,
}

impl ChannelRegistration {
    /// Start a new registration for the channel named `name`. The
    /// name must match the value returned by each role impl's
    /// `name()` so lookups by trait round-trip.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            intake: None,
            delivery: None,
            notify: None,
        }
    }

    #[must_use]
    pub fn with_intake(mut self, provider: Arc<dyn IntakeProvider>) -> Self {
        self.intake = Some(provider);
        self
    }

    #[must_use]
    pub fn with_delivery(mut self, sink: Arc<dyn DeliverySink>) -> Self {
        self.delivery = Some(sink);
        self
    }

    #[must_use]
    pub fn with_notify(mut self, sink: Arc<dyn NotifySink>) -> Self {
        self.notify = Some(sink);
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

struct ChannelEntry {
    intake: Option<Arc<dyn IntakeProvider>>,
    delivery: Option<Arc<dyn DeliverySink>>,
    notify: Option<Arc<dyn NotifySink>>,
}

/// One row of [`ChannelRegistry::health`] output — the registered name
/// plus the role slots that are populated. Consumed by the CLI
/// `channel list` command and the future `GET /v1/channels`
/// introspection endpoint (story 2.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelHealth {
    pub name: String,
    /// Subset of `["intake", "delivery", "notify"]` in that order.
    pub capabilities: Vec<&'static str>,
}

/// Owns the registered channels and exposes routing-friendly views.
///
/// Held inside `AppState` once story 2.3 wires it through. Story 2.4
/// ships the registry in isolation; story 2.5's IntakeRouter +
/// DeliveryRouter become its primary consumers.
#[derive(Default)]
pub struct ChannelRegistry {
    by_name: HashMap<String, ChannelEntry>,
}

impl ChannelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a registration. A second register with the same name
    /// replaces the previous slots — fine for tests, but production
    /// code should register each channel exactly once at startup.
    pub fn register(&mut self, reg: ChannelRegistration) {
        let entry = ChannelEntry {
            intake: reg.intake,
            delivery: reg.delivery,
            notify: reg.notify,
        };
        self.by_name.insert(reg.name, entry);
    }

    /// Iterate channels with an [`IntakeProvider`] role populated.
    /// Tuple is `(channel_name, provider_arc)`.
    pub fn iter_intake(&self) -> impl Iterator<Item = (&str, &Arc<dyn IntakeProvider>)> {
        self.by_name
            .iter()
            .filter_map(|(name, entry)| entry.intake.as_ref().map(|p| (name.as_str(), p)))
    }

    pub fn iter_delivery(&self) -> impl Iterator<Item = (&str, &Arc<dyn DeliverySink>)> {
        self.by_name
            .iter()
            .filter_map(|(name, entry)| entry.delivery.as_ref().map(|s| (name.as_str(), s)))
    }

    pub fn iter_notify(&self) -> impl Iterator<Item = (&str, &Arc<dyn NotifySink>)> {
        self.by_name
            .iter()
            .filter_map(|(name, entry)| entry.notify.as_ref().map(|s| (name.as_str(), s)))
    }

    /// Look up an [`IntakeProvider`] by channel name. Returns `None`
    /// if the channel is unregistered OR if it is registered but does
    /// not implement this role.
    pub fn get_intake(&self, name: &str) -> Option<Arc<dyn IntakeProvider>> {
        self.by_name
            .get(name)
            .and_then(|entry| entry.intake.clone())
    }

    pub fn get_delivery(&self, name: &str) -> Option<Arc<dyn DeliverySink>> {
        self.by_name
            .get(name)
            .and_then(|entry| entry.delivery.clone())
    }

    pub fn get_notify(&self, name: &str) -> Option<Arc<dyn NotifySink>> {
        self.by_name
            .get(name)
            .and_then(|entry| entry.notify.clone())
    }

    /// Snapshot of every registered channel's capabilities. Sorted by
    /// name so the introspection endpoint output is stable.
    pub fn health(&self) -> Vec<ChannelHealth> {
        let mut out: Vec<ChannelHealth> = self
            .by_name
            .iter()
            .map(|(name, entry)| {
                let mut capabilities = Vec::with_capacity(3);
                if entry.intake.is_some() {
                    capabilities.push("intake");
                }
                if entry.delivery.is_some() {
                    capabilities.push("delivery");
                }
                if entry.notify.is_some() {
                    capabilities.push("notify");
                }
                ChannelHealth {
                    name: name.clone(),
                    capabilities,
                }
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Spawn one Tokio task per registered [`IntakeProvider`]. Each
    /// task receives a clone of the shared `sink` (drained by the
    /// IntakeRouter in story 2.5) and the shared `shutdown` token
    /// (cancellation drains them all in one shot).
    ///
    /// Returns one [`JoinHandle`] per spawned provider. Order is
    /// unspecified — `HashMap` iteration order — which is fine
    /// because every consumer treats the handles as a set.
    pub fn spawn_intakes(
        &self,
        sink: mpsc::Sender<IntakeEvent>,
        shutdown: CancellationToken,
    ) -> Vec<JoinHandle<Result<(), ChannelError>>> {
        self.iter_intake()
            .map(|(_, provider)| {
                let provider = provider.clone();
                let sink = sink.clone();
                let shutdown = shutdown.clone();
                tokio::spawn(async move { provider.run(sink, shutdown).await })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
