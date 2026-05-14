//! SMTP send helpers + transport abstraction for [`super::EmailChannel`].
//!
//! `EmailTransport` is a thin trait around `lettre::Message` send so
//! tests can substitute a recording transport without enabling lettre's
//! `stub-transport` feature (which conflicts with our chosen feature
//! set). The real impl wraps `AsyncSmtpTransport<Tokio1Executor>` with
//! `tokio1-rustls-tls` (matches the reqwest TLS stack used elsewhere —
//! no openssl dep, see `[[reference_local_dev_env]]`).
//!
//! refs: /specs/phase-2/architecture.md §2.9 "Email delivery"

use std::sync::Arc;

use async_trait::async_trait;
use lettre::message::header::ContentType;
use lettre::message::{Attachment, MultiPart, SinglePart};
use lettre::transport::smtp::AsyncSmtpTransport;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncTransport, Message, Tokio1Executor};
use tokio::sync::Mutex;

use crate::channel::ChannelError;

/// Sending side of [`super::EmailChannel`]. The boxed trait is
/// constructed from `EmailChannelConfig` at registration time; tests
/// inject [`RecordingTransport`] to capture the outbound `Message`
/// without opening a TCP connection.
#[async_trait]
pub trait EmailTransport: Send + Sync {
    /// Send `message`. Returns the channel-side identifier the
    /// upstream router should record (lettre returns SMTP queue id /
    /// `250 OK <id>`-style strings; we surface them verbatim).
    async fn send(&self, message: Message) -> Result<String, ChannelError>;
}

/// Configuration the [`LettreSmtpTransport`] consumes at boot.
#[derive(Debug, Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

/// Production SMTP transport: lettre over Tokio + rustls.
pub struct LettreSmtpTransport {
    inner: AsyncSmtpTransport<Tokio1Executor>,
}

impl LettreSmtpTransport {
    pub fn new(config: &SmtpConfig) -> Result<Self, ChannelError> {
        let creds = Credentials::new(config.username.clone(), config.password.clone());
        let inner = AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host)
            .map_err(|err| ChannelError::Internal(format!("smtp relay setup: {err}")))?
            .port(config.port)
            .credentials(creds)
            .build();
        Ok(Self { inner })
    }
}

#[async_trait]
impl EmailTransport for LettreSmtpTransport {
    async fn send(&self, message: Message) -> Result<String, ChannelError> {
        let response = self
            .inner
            .send(message)
            .await
            .map_err(|err| ChannelError::Transport(format!("smtp send: {err}")))?;
        // lettre's Response carries the multi-line server reply; we
        // join the first line (typically `250 OK <queue-id>`) for the
        // DeliveryReceipt's `external_id`.
        let line = response
            .first_line()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "250 OK".into());
        Ok(line)
    }
}

/// Test transport that records every sent message and never touches
/// the network. Cheap to clone — the captured log lives behind an
/// `Arc<Mutex<Vec<Message>>>` so callers can inspect concurrently.
#[derive(Default, Clone)]
pub struct RecordingTransport {
    pub captured: Arc<Mutex<Vec<Message>>>,
}

impl RecordingTransport {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn snapshot(&self) -> Vec<Message> {
        self.captured.lock().await.clone()
    }
}

#[async_trait]
impl EmailTransport for RecordingTransport {
    async fn send(&self, message: Message) -> Result<String, ChannelError> {
        self.captured.lock().await.push(message);
        Ok("250 OK recorded".into())
    }
}

/// Build a reply MIME for [`super::EmailChannel::DeliverySink`]. The
/// caller supplies the parent Message-ID + original subject so we can
/// set `In-Reply-To` / `References` / a `[Re:]`-prefixed subject.
///
/// `attachment_bytes` is read from `deliverable.rendered_content_path`
/// by the channel; we keep this helper byte-oriented so unit tests can
/// pin behaviour without staging files on disk.
#[allow(clippy::too_many_arguments)]
pub fn build_reply(
    from: &str,
    to: &str,
    in_reply_to_msgid: &str,
    original_subject: &str,
    body_text: &str,
    attachment_filename: &str,
    attachment_content_type: ContentType,
    attachment_bytes: Vec<u8>,
) -> Result<Message, ChannelError> {
    let subject = if original_subject
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("re:")
    {
        original_subject.to_string()
    } else {
        format!("Re: {original_subject}")
    };

    let from_mbox = from
        .parse()
        .map_err(|err| ChannelError::Internal(format!("invalid From {from:?}: {err}")))?;
    let to_mbox = to
        .parse()
        .map_err(|err| ChannelError::Internal(format!("invalid To {to:?}: {err}")))?;

    let attachment = Attachment::new(attachment_filename.to_string())
        .body(attachment_bytes, attachment_content_type);
    let body_part = SinglePart::builder()
        .header(ContentType::TEXT_PLAIN)
        .body(body_text.to_string());

    // Wrap the bare Message-ID in `<...>` if the caller passed it
    // unwrapped; RFC 5322 5.2.x requires the angle brackets and most
    // mail clients drop the header silently if they're missing.
    let normalized_msgid = if in_reply_to_msgid.starts_with('<') {
        in_reply_to_msgid.to_string()
    } else {
        format!("<{in_reply_to_msgid}>")
    };

    Message::builder()
        .from(from_mbox)
        .to(to_mbox)
        .subject(subject)
        .in_reply_to(normalized_msgid.clone())
        .references(normalized_msgid)
        .multipart(
            MultiPart::mixed()
                .singlepart(body_part)
                .singlepart(attachment),
        )
        .map_err(|err| ChannelError::Internal(format!("reply build: {err}")))
}

/// Build a plain status email for [`super::EmailChannel::NotifySink`].
pub fn build_notify(
    from: &str,
    to: &str,
    subject: &str,
    body_text: &str,
) -> Result<Message, ChannelError> {
    let from_mbox = from
        .parse()
        .map_err(|err| ChannelError::Internal(format!("invalid From {from:?}: {err}")))?;
    let to_mbox = to
        .parse()
        .map_err(|err| ChannelError::Internal(format!("invalid To {to:?}: {err}")))?;

    Message::builder()
        .from(from_mbox)
        .to(to_mbox)
        .subject(subject)
        .header(ContentType::TEXT_PLAIN)
        .body(body_text.to_string())
        .map_err(|err| ChannelError::Internal(format!("notify build: {err}")))
}
