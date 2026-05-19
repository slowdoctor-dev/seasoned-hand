//! `WebhookChannel` — one struct, three role traits.
//!
//! The minimum-viable OS surface: any external system speaks HTTP to
//! us (inbound `POST /v1/intake/webhook`) and gets HTTP back (outbound
//! POST to the operator-supplied callback URL on delivery / notify).
//!
//! Architectural split:
//! - [`WebhookChannel`] owns shared config (intake token, reqwest
//!   client, SSRF allow-list) and implements the three role traits.
//! - [`IntakeProvider::run`] is a no-op — the actual intake source is
//!   the axum route `POST /v1/intake/webhook` mounted by the server
//!   crate. The handler reads [`WebhookChannel::verify_intake_token`]
//!   to gate access, then constructs an [`IntakeEvent`] and hands it
//!   to the existing [`crate::intake::IntakeRouter::handle_event`]
//!   path (same flow the WS handler already uses). This matches the
//!   ChatChannel pattern: trait impl exists for uniformity and
//!   registry introspection, but the intake source is external to
//!   `run()`.
//! - [`DeliverySink::deliver`] / [`NotifySink::notify`] POST JSON to
//!   `target.target_ref` after the SSRF guard clears the resolved
//!   address. Errors map onto the [`ChannelError`] variants the
//!   [`crate::delivery::DeliveryRouter`] retries on: `RemoteRejected`
//!   for HTTP status responses (5xx retryable, 4xx terminal), `Http`
//!   for connection-level failures.
//!
//! refs: /specs/phase-2/architecture.md §2.7, §2.8, §2.9, §9
//! refs: /specs/phase-2/stories/story-2.10.md

use std::sync::Arc;

use async_trait::async_trait;
use ipnet::IpNet;
use reqwest::{Client, Url};
use serde_json::json;
use subtle::ConstantTimeEq;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{
    ChannelError, Deliverable, DeliveryReceipt, DeliverySink, DeliveryTarget, IntakeEvent,
    IntakeProvider, NotifyEvent, NotifyReceipt, NotifySink, NotifyTarget,
};

use crate::time::now_micros;
pub mod ssrf;

#[cfg(test)]
mod tests;

/// Channel name registered into the [`crate::channel::ChannelRegistry`].
pub const CHANNEL_NAME: &str = "webhook";

/// Outcome of [`WebhookChannel::verify_intake_token`]. The HTTP handler
/// in the server crate maps each variant to its spec-defined response
/// code so the contract (503 / 401 / 202) lives in one place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenCheck {
    /// `SEASONED_HAND_INTAKE_TOKEN` env was empty at boot — the
    /// endpoint is intentionally disabled. Handler returns 503
    /// `intake_token_not_configured`.
    NotConfigured,
    /// Header missing or mismatched. Constant-time compared so a
    /// remote attacker can't probe token validity by response timing.
    /// Handler returns 401.
    Mismatch,
    /// Token matches; proceed with intake.
    Ok,
}

/// Webhook channel implementation.
///
/// All three role slots are filled by the same struct. Constructed
/// once at boot and registered as `Arc<WebhookChannel>` — the same
/// `Arc` is cloned into the registry's intake / delivery / notify
/// slots so role-introspection sees the full capability set.
pub struct WebhookChannel {
    intake_token: Arc<String>,
    http: Client,
    allowlist: Vec<IpNet>,
}

impl WebhookChannel {
    /// `intake_token` is `Arc<String>` so the same allocation is shared
    /// between the channel impl and the HTTP route handler (the server
    /// crate stores its own clone on [`crate::channel`]'s consumer for
    /// the route's pre-flight check). Empty string means the intake
    /// endpoint is disabled.
    pub fn new(intake_token: Arc<String>, http: Client, allowlist: Vec<IpNet>) -> Self {
        Self {
            intake_token,
            http,
            allowlist,
        }
    }

    /// Build with a default reqwest client (rustls-tls, 15 s overall
    /// timeout). Convenience for tests and the default production
    /// path; main.rs can substitute a custom client when needed.
    pub fn with_default_client(intake_token: Arc<String>, allowlist: Vec<IpNet>) -> Self {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("default reqwest client");
        Self::new(intake_token, http, allowlist)
    }

    /// Snapshot of the configured intake token. Exposed for the server
    /// crate's route handler so the token-compare logic lives in one
    /// place and the channel impl stays the source of truth.
    pub fn intake_token(&self) -> &Arc<String> {
        &self.intake_token
    }

    /// Constant-time compare the supplied header value against the
    /// configured intake token. Returns [`TokenCheck::NotConfigured`]
    /// if the token was never set (empty env at boot) — the handler
    /// surfaces that as 503 so operators can tell "unconfigured" apart
    /// from "wrong token".
    pub fn verify_intake_token(&self, supplied: Option<&str>) -> TokenCheck {
        if self.intake_token.is_empty() {
            return TokenCheck::NotConfigured;
        }
        let supplied = supplied.unwrap_or("");
        if supplied
            .as_bytes()
            .ct_eq(self.intake_token.as_bytes())
            .into()
        {
            TokenCheck::Ok
        } else {
            TokenCheck::Mismatch
        }
    }

    async fn post_json(
        &self,
        url_str: &str,
        body: serde_json::Value,
    ) -> Result<(reqwest::StatusCode, serde_json::Value), ChannelError> {
        let url = Url::parse(url_str).map_err(|e| ChannelError::RemoteRejected {
            status: 400,
            message: format!("invalid url: {e}"),
        })?;

        if let Err(err) = ssrf::assert_public_address(&url, &self.allowlist).await {
            return Err(match err {
                ssrf::AssertError::Rejected(_) => ChannelError::RemoteRejected {
                    status: 400,
                    message: "private_address_rejected".into(),
                },
                ssrf::AssertError::HostMissing => ChannelError::RemoteRejected {
                    status: 400,
                    message: "host_missing".into(),
                },
                ssrf::AssertError::Resolve(msg) => ChannelError::Transport(msg),
            });
        }

        let resp = self
            .http
            .post(url)
            .json(&body)
            .send()
            .await
            .map_err(|e| ChannelError::Http(e.to_string()))?;

        let status = resp.status();
        // Body is best-effort for audit; a parse failure on a successful
        // response is not fatal because the success signal lives in
        // `status`.
        let body_val: serde_json::Value = resp
            .json::<serde_json::Value>()
            .await
            .unwrap_or(serde_json::Value::Null);

        if !status.is_success() {
            return Err(ChannelError::RemoteRejected {
                status: status.as_u16(),
                message: body_val.to_string(),
            });
        }
        Ok((status, body_val))
    }
}

#[async_trait]
impl IntakeProvider for WebhookChannel {
    fn name(&self) -> &'static str {
        CHANNEL_NAME
    }

    async fn run(
        &self,
        _sink: mpsc::Sender<IntakeEvent>,
        shutdown: CancellationToken,
    ) -> Result<(), ChannelError> {
        // The webhook intake source is the axum route
        // `POST /v1/intake/webhook` mounted by the server crate — see
        // module docs. The trait impl exists so registry introspection
        // reports `intake` in this channel's capability list and so
        // `ChannelRegistry::spawn_intakes` can iterate uniformly.
        // We park on `shutdown` so the spawned task lives for the
        // process lifetime (otherwise it would resolve immediately and
        // the JoinHandle would surface as "completed" in any future
        // health check).
        shutdown.cancelled().await;
        Ok(())
    }
}

#[async_trait]
impl DeliverySink for WebhookChannel {
    fn name(&self) -> &'static str {
        CHANNEL_NAME
    }

    async fn deliver(
        &self,
        target: &DeliveryTarget,
        deliverable: &Deliverable,
    ) -> Result<DeliveryReceipt, ChannelError> {
        let url = decode_url_ref(&target.target_ref)?;
        let content_url = format!(
            "/v1/tasks/{}/deliverables/{}/content",
            deliverable.task_id, deliverable.id
        );
        let body = json!({
            "task_id": deliverable.task_id,
            "deliverable_id": deliverable.id,
            "format": deliverable.format,
            "content_url": content_url,
            "provenance_manifest": deliverable.provenance_manifest,
            "status": "completed",
        });
        let (status, raw) = self.post_json(&url, body).await?;
        Ok(DeliveryReceipt {
            channel: CHANNEL_NAME.to_string(),
            external_id: format!("http:{}", status.as_u16()),
            delivered_at: now_micros(),
            raw_response: raw,
        })
    }
}

#[async_trait]
impl NotifySink for WebhookChannel {
    fn name(&self) -> &'static str {
        CHANNEL_NAME
    }

    async fn notify(
        &self,
        target: &NotifyTarget,
        event: &NotifyEvent,
    ) -> Result<NotifyReceipt, ChannelError> {
        let url = decode_url_ref(&target.target_ref)?;
        let body = json!({
            "task_id": event.task_id,
            "trigger_kind": event.trigger_kind,
            "payload": event.payload,
        });
        let (status, raw) = self.post_json(&url, body).await?;
        Ok(NotifyReceipt {
            channel: CHANNEL_NAME.to_string(),
            external_id: Some(format!("http:{}", status.as_u16())),
            sent_at: now_micros(),
            raw_response: raw,
        })
    }
}

/// Channel-specific target_ref shape: `"url:https://..."` per
/// architecture §2.7. We accept the bare URL as a fallback so existing
/// tests / fixtures don't have to be rewritten when they call
/// WebhookChannel directly.
fn decode_url_ref(target_ref: &str) -> Result<String, ChannelError> {
    let url = target_ref.strip_prefix("url:").unwrap_or(target_ref).trim();
    if url.is_empty() {
        return Err(ChannelError::RemoteRejected {
            status: 400,
            message: "empty_target_ref".into(),
        });
    }
    Ok(url.to_string())
}
