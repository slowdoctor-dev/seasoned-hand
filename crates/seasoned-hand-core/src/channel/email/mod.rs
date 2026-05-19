//! `EmailChannel` — one struct, three role traits.
//!
//! Email is the most natural non-technical inbound channel — anyone who
//! can already send mail can delegate work by replying. Three role
//! impls share one struct (per the architecture §2.7 contract):
//!
//! - [`IntakeProvider::run`] is the FIRST channel with a real
//!   long-lived listener: an IMAP poll loop (default 30 s interval)
//!   that connects, `SELECT INBOX`, `SEARCH UNSEEN`, fetches each new
//!   message, validates it against the allow-list + subject prefix,
//!   pushes an [`IntakeEvent`] through the shared mpsc, and marks the
//!   UID `\Seen`. Errors back off exponentially up to a 5-minute cap;
//!   `shutdown.cancelled()` short-circuits the loop cleanly.
//! - [`DeliverySink::deliver`] crafts a lettre `Message` reply: the
//!   parsed `target.target_ref` (`"msgid:<...>"`) becomes
//!   `In-Reply-To` + `References`, the deliverable file is attached,
//!   and the subject is `[Re: ...]`-prefixed (de-duplicated).
//! - [`NotifySink::notify`] sends a plain text status email; the
//!   `target.target_ref` is the recipient address, NOT a Message-ID.
//!
//! Pluggable transports: the channel holds an `Arc<dyn EmailTransport>`
//! and an `Arc<dyn MailboxFetcher>` so the unit tests can drop in a
//! `RecordingTransport` + `MockMailbox` without staging an SMTP/IMAP
//! server. Production wiring uses `LettreSmtpTransport` +
//! `AsyncImapFetcher`.
//!
//! refs: /specs/phase-2/architecture.md §2.7, §2.8, §2.9, §9
//! refs: /specs/phase-2/stories/story-2.11.md

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use lettre::message::header::ContentType;
use mailparse::{MailHeaderMap, ParsedMail};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{
    ChannelError, Deliverable, DeliveryReceipt, DeliverySink, DeliveryTarget, IntakeEvent,
    IntakeProvider, NotifyEvent, NotifyReceipt, NotifySink, NotifyTarget,
};

use crate::time::now_micros;
pub mod allowlist;
pub mod imap;
pub mod smtp;

#[cfg(test)]
mod tests;

pub use allowlist::AllowList;
pub use imap::{AsyncImapFetcher, ImapConfig, MailboxFetcher, MockMailbox, RawMessage};
pub use smtp::{
    EmailTransport, LettreSmtpTransport, RecordingTransport, SmtpConfig, build_notify, build_reply,
};

/// Channel name registered into the [`crate::channel::ChannelRegistry`].
pub const CHANNEL_NAME: &str = "email";

/// Default subject prefix per architecture §9.
pub const DEFAULT_SUBJECT_PREFIX: &str = "[sh]";

/// Default IMAP poll cycle (30 s per architecture §2.8).
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(30);

/// Backoff ceiling for repeated IMAP failures (5 min per spec
/// "exponential backoff up to a 5-min cap").
pub const POLL_BACKOFF_CEILING: Duration = Duration::from_secs(300);

/// EmailChannel — one struct, three role traits. Constructed via
/// [`EmailChannel::builder`] (see also `register_email_channel` on
/// the server crate's `AppState` for the env-driven boot path).
pub struct EmailChannel {
    fetcher: Arc<dyn MailboxFetcher>,
    transport: Arc<dyn EmailTransport>,
    from_address: String,
    subject_prefix: String,
    allow_list: AllowList,
    poll_interval: Duration,
}

impl EmailChannel {
    pub fn builder() -> EmailChannelBuilder {
        EmailChannelBuilder::default()
    }

    /// Public for tests + the server crate's registration builder; in
    /// production paths prefer `EmailChannel::builder()`.
    pub fn new(
        fetcher: Arc<dyn MailboxFetcher>,
        transport: Arc<dyn EmailTransport>,
        from_address: String,
        subject_prefix: String,
        allow_list: AllowList,
        poll_interval: Duration,
    ) -> Self {
        Self {
            fetcher,
            transport,
            from_address,
            subject_prefix,
            allow_list,
            poll_interval,
        }
    }

    /// One poll cycle: fetch UNSEEN, gate each message, push surviving
    /// IntakeEvents into `sink`, mark seen. Exposed (`pub`) so unit
    /// tests can drive a single cycle without spinning the
    /// poll-with-backoff loop.
    pub async fn poll_once(&self, sink: &mpsc::Sender<IntakeEvent>) -> Result<usize, ChannelError> {
        let messages = self.fetcher.fetch_unseen().await?;
        let mut emitted = 0usize;
        for raw in messages {
            match self.process_message(&raw, sink).await {
                Ok(true) => emitted += 1,
                Ok(false) => {}
                Err(err) => {
                    // Per-message failure is logged but does not abort
                    // the cycle — one malformed mail must not poison
                    // the whole inbox.
                    tracing::warn!(uid = raw.uid, error = %err, "email: process_message failed");
                }
            }
            // Mark seen regardless of accept/reject — re-emitting a
            // rejected mail on every cycle would spam the operator.
            // The allow-list/subject-prefix gate is deterministic so
            // once-rejected stays rejected.
            if let Err(err) = self.fetcher.mark_seen(raw.uid).await {
                tracing::warn!(uid = raw.uid, error = %err, "email: mark_seen failed");
            }
        }
        Ok(emitted)
    }

    async fn process_message(
        &self,
        raw: &RawMessage,
        sink: &mpsc::Sender<IntakeEvent>,
    ) -> Result<bool, ChannelError> {
        let parsed = mailparse::parse_mail(&raw.bytes)
            .map_err(|err| ChannelError::Decode(format!("email parse: {err}")))?;

        let from = header_value(&parsed, "From").unwrap_or_default();
        let from_addr = parse_address(&from);
        let subject = header_value(&parsed, "Subject").unwrap_or_default();
        let message_id = header_value(&parsed, "Message-ID").unwrap_or_default();

        // Gate 1: allow-list. Default-deny per architecture §9 — an
        // empty list rejects every sender.
        if !self.allow_list.allows(&from_addr) {
            let reason = if self.allow_list.is_empty() {
                "allowlist_empty"
            } else {
                "sender_not_allowed"
            };
            tracing::warn!(
                from = %from_addr,
                reason,
                "email: intake_sender_rejected"
            );
            return Ok(false);
        }

        // Gate 2: subject prefix. Mail without the prefix is left
        // un-delivered (the operator's own non-intake mail must be
        // ignored without rejection noise).
        if !subject.trim_start().starts_with(&self.subject_prefix) {
            tracing::debug!(
                subject = %subject,
                prefix = %self.subject_prefix,
                "email: subject_prefix_missing — skipping"
            );
            return Ok(false);
        }

        // Gate 3: a usable text/plain body. Phase 2 non-goal: HTML-only
        // mail (story spec). Reject so operators see why their HTML
        // newsletter wasn't picked up.
        let body_plain = match extract_text_body(&parsed) {
            Some(b) => b,
            None => {
                tracing::warn!(
                    from = %from_addr,
                    reason = "intake_no_plain_body",
                    "email: intake_sender_rejected"
                );
                return Ok(false);
            }
        };

        let attachments = collect_attachments(&parsed);

        // SPF / DKIM signal: pulled out of Authentication-Results when
        // the upstream MTA stamped one. Failed signatures are surfaced
        // as metadata (story 2.11 acceptance) — they don't block
        // intake automatically in Phase 2.
        let auth_results = header_value(&parsed, "Authentication-Results").unwrap_or_default();
        let (spf, dkim) = parse_auth_results(&auth_results);

        let metadata = json!({
            "from": from_addr,
            "subject": subject,
            "message_id": message_id,
            "has_attachments": !attachments.is_empty(),
            "attachments": attachments
                .iter()
                .map(|a| json!({
                    "filename": a.filename,
                    "content_type": a.content_type,
                    "size": a.bytes.len(),
                }))
                .collect::<Vec<_>>(),
            "spf": spf,
            "dkim": dkim,
            "authentication_results": auth_results,
        });

        let reply_target = if !message_id.is_empty() {
            Some(DeliveryTarget {
                channel: CHANNEL_NAME.to_string(),
                target_ref: format!("msgid:{}", normalize_msgid(&message_id)),
                metadata: json!({
                    "to": from_addr,
                    "subject": subject,
                }),
            })
        } else {
            None
        };

        let intake_event = IntakeEvent {
            channel: CHANNEL_NAME.to_string(),
            intake_id: format!("imap:{}", raw.uid),
            brief_input: body_plain,
            reply_target,
            metadata,
            tenant_id: None,
            received_at: now_micros(),
        };

        sink.send(intake_event)
            .await
            .map_err(|err| ChannelError::Internal(format!("email: sink closed: {err}")))?;
        Ok(true)
    }
}

#[async_trait]
impl IntakeProvider for EmailChannel {
    fn name(&self) -> &'static str {
        CHANNEL_NAME
    }

    async fn run(
        &self,
        sink: mpsc::Sender<IntakeEvent>,
        shutdown: CancellationToken,
    ) -> Result<(), ChannelError> {
        let mut backoff = Duration::from_secs(0);
        let cycle = self.poll_interval;
        loop {
            // Sleep for the cycle interval (or the longer backoff
            // value) but bail immediately on shutdown.
            let wait = if backoff > cycle { backoff } else { cycle };
            tokio::select! {
                _ = shutdown.cancelled() => return Ok(()),
                _ = tokio::time::sleep(wait) => {}
            }

            match self.poll_once(&sink).await {
                Ok(_) => {
                    backoff = Duration::from_secs(0);
                }
                Err(err) => {
                    // Cap the doubling at POLL_BACKOFF_CEILING. First
                    // failure jumps straight to 30 s so a flapping
                    // server doesn't get hammered between flap cycles.
                    let next = if backoff.is_zero() {
                        Duration::from_secs(30)
                    } else {
                        std::cmp::min(backoff * 2, POLL_BACKOFF_CEILING)
                    };
                    tracing::warn!(
                        error = %err,
                        backoff_secs = next.as_secs(),
                        "email: poll cycle failed; backing off"
                    );
                    backoff = next;
                }
            }
        }
    }
}

#[async_trait]
impl DeliverySink for EmailChannel {
    fn name(&self) -> &'static str {
        CHANNEL_NAME
    }

    async fn deliver(
        &self,
        target: &DeliveryTarget,
        deliverable: &Deliverable,
    ) -> Result<DeliveryReceipt, ChannelError> {
        let msgid = target.target_ref.strip_prefix("msgid:").ok_or_else(|| {
            ChannelError::RemoteRejected {
                status: 400,
                message: format!(
                    "email: target_ref must start with 'msgid:', got {:?}",
                    target.target_ref
                ),
            }
        })?;

        let to = target
            .metadata
            .get("to")
            .and_then(Value::as_str)
            .ok_or_else(|| ChannelError::RemoteRejected {
                status: 400,
                message: "email: target.metadata.to (recipient address) is required".into(),
            })?
            .to_string();

        let original_subject = target
            .metadata
            .get("subject")
            .and_then(Value::as_str)
            .unwrap_or("");

        let attachment_bytes = tokio::fs::read(&deliverable.rendered_content_path)
            .await
            .map_err(|err| {
                ChannelError::Internal(format!(
                    "email: deliverable read {:?}: {err}",
                    deliverable.rendered_content_path
                ))
            })?;
        let content_type = guess_content_type(&deliverable.format);
        let attachment_filename = filename_for(deliverable);
        let body_text = format!(
            "Seasoned Hand has completed the task.\n\n\
             Deliverable: {} ({})\n\
             See the attached file.",
            deliverable.id, deliverable.format,
        );

        let message = build_reply(
            &self.from_address,
            &to,
            msgid,
            original_subject,
            &body_text,
            &attachment_filename,
            content_type,
            attachment_bytes,
        )?;

        let external_id = self.transport.send(message).await?;
        Ok(DeliveryReceipt {
            channel: CHANNEL_NAME.to_string(),
            external_id,
            delivered_at: now_micros(),
            raw_response: json!({"to": to, "in_reply_to": msgid}),
        })
    }
}

#[async_trait]
impl NotifySink for EmailChannel {
    fn name(&self) -> &'static str {
        CHANNEL_NAME
    }

    async fn notify(
        &self,
        target: &NotifyTarget,
        event: &NotifyEvent,
    ) -> Result<NotifyReceipt, ChannelError> {
        // NotifyTarget for email: target_ref is the bare recipient
        // address (no `msgid:` prefix — notifies don't thread into a
        // prior conversation).
        let to = target.target_ref.trim();
        if to.is_empty() {
            return Err(ChannelError::RemoteRejected {
                status: 400,
                message: "email: notify target_ref (recipient address) is empty".into(),
            });
        }

        let subject = format!("{} {}", self.subject_prefix, event.trigger_kind);
        let body_text =
            serde_json::to_string_pretty(&event.payload).unwrap_or_else(|_| "{}".to_string());

        let message = build_notify(&self.from_address, to, &subject, &body_text)?;
        let external_id = self.transport.send(message).await?;
        Ok(NotifyReceipt {
            channel: CHANNEL_NAME.to_string(),
            external_id: Some(external_id),
            sent_at: now_micros(),
            raw_response: json!({"to": to, "subject": subject}),
        })
    }
}

/// Builder for [`EmailChannel`] used by the server crate's
/// `register_email_channel` boot path. Each field is required EXCEPT
/// `subject_prefix` (defaults to `[sh]`) and `poll_interval` (30 s).
#[derive(Default)]
pub struct EmailChannelBuilder {
    fetcher: Option<Arc<dyn MailboxFetcher>>,
    transport: Option<Arc<dyn EmailTransport>>,
    from_address: Option<String>,
    subject_prefix: Option<String>,
    allow_list: Option<AllowList>,
    poll_interval: Option<Duration>,
}

impl EmailChannelBuilder {
    pub fn fetcher(mut self, fetcher: Arc<dyn MailboxFetcher>) -> Self {
        self.fetcher = Some(fetcher);
        self
    }

    pub fn transport(mut self, transport: Arc<dyn EmailTransport>) -> Self {
        self.transport = Some(transport);
        self
    }

    pub fn from_address(mut self, from: impl Into<String>) -> Self {
        self.from_address = Some(from.into());
        self
    }

    pub fn subject_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.subject_prefix = Some(prefix.into());
        self
    }

    pub fn allow_list(mut self, list: AllowList) -> Self {
        self.allow_list = Some(list);
        self
    }

    pub fn poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = Some(interval);
        self
    }

    pub fn build(self) -> Result<EmailChannel, ChannelError> {
        let fetcher = self
            .fetcher
            .ok_or_else(|| ChannelError::Internal("email: builder missing fetcher".into()))?;
        let transport = self
            .transport
            .ok_or_else(|| ChannelError::Internal("email: builder missing transport".into()))?;
        let from_address = self
            .from_address
            .ok_or_else(|| ChannelError::Internal("email: builder missing from_address".into()))?;
        Ok(EmailChannel::new(
            fetcher,
            transport,
            from_address,
            self.subject_prefix
                .unwrap_or_else(|| DEFAULT_SUBJECT_PREFIX.to_string()),
            self.allow_list.unwrap_or_default(),
            self.poll_interval.unwrap_or(DEFAULT_POLL_INTERVAL),
        ))
    }
}

// ---------------------------------------------------------------------------
// Mailparse helpers.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AttachmentInfo {
    pub filename: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

fn header_value(parsed: &ParsedMail<'_>, name: &str) -> Option<String> {
    parsed.headers.get_first_value(name)
}

fn parse_address(from_header: &str) -> String {
    // mailparse exposes a structured `addrparse_header` on a header,
    // but for sender allow-list matching we only need the literal
    // address. Use the structured parse to handle `"Name" <a@b>` and
    // bare `a@b` uniformly; fall back to the trimmed input otherwise.
    match mailparse::addrparse(from_header) {
        Ok(list) => list
            .iter()
            .find_map(|info| match info {
                mailparse::MailAddr::Single(s) => Some(s.addr.clone()),
                mailparse::MailAddr::Group(g) => g.addrs.first().map(|s| s.addr.clone()),
            })
            .unwrap_or_else(|| from_header.trim().to_string()),
        Err(_) => from_header.trim().to_string(),
    }
}

fn extract_text_body(parsed: &ParsedMail<'_>) -> Option<String> {
    if parsed.subparts.is_empty() {
        let ctype = parsed.ctype.mimetype.as_str();
        if ctype.starts_with("text/plain") || ctype.is_empty() {
            return parsed.get_body().ok();
        }
        return None;
    }
    for part in &parsed.subparts {
        if let Some(body) = extract_text_body(part) {
            return Some(body);
        }
    }
    None
}

fn collect_attachments(parsed: &ParsedMail<'_>) -> Vec<AttachmentInfo> {
    let mut out = Vec::new();
    walk_attachments(parsed, &mut out);
    out
}

fn walk_attachments(parsed: &ParsedMail<'_>, out: &mut Vec<AttachmentInfo>) {
    let disposition = parsed.get_content_disposition();
    let is_attachment = matches!(
        disposition.disposition,
        mailparse::DispositionType::Attachment | mailparse::DispositionType::Inline
    ) && disposition.params.contains_key("filename");
    if is_attachment {
        let filename = disposition
            .params
            .get("filename")
            .cloned()
            .unwrap_or_else(|| "attachment".to_string());
        let content_type = parsed.ctype.mimetype.clone();
        if let Ok(bytes) = parsed.get_body_raw() {
            out.push(AttachmentInfo {
                filename,
                content_type,
                bytes,
            });
        }
    }
    for part in &parsed.subparts {
        walk_attachments(part, out);
    }
}

fn parse_auth_results(value: &str) -> (Option<String>, Option<String>) {
    if value.is_empty() {
        return (None, None);
    }
    let lower = value.to_ascii_lowercase();
    let spf = capture_token(&lower, "spf=");
    let dkim = capture_token(&lower, "dkim=");
    (spf, dkim)
}

fn capture_token(haystack: &str, key: &str) -> Option<String> {
    let idx = haystack.find(key)?;
    let rest = &haystack[idx + key.len()..];
    let end = rest
        .find(|c: char| c.is_whitespace() || c == ';' || c == ',')
        .unwrap_or(rest.len());
    let token = rest[..end].trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

fn normalize_msgid(raw: &str) -> String {
    let trimmed = raw.trim();
    let inner = trimmed.trim_start_matches('<').trim_end_matches('>').trim();
    format!("<{inner}>")
}

fn guess_content_type(format: &str) -> ContentType {
    match format {
        "pdf" => ContentType::parse("application/pdf").unwrap_or(ContentType::TEXT_PLAIN),
        "docx" => ContentType::parse(
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        )
        .unwrap_or(ContentType::TEXT_PLAIN),
        "pptx" => ContentType::parse(
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        )
        .unwrap_or(ContentType::TEXT_PLAIN),
        "xlsx" => {
            ContentType::parse("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet")
                .unwrap_or(ContentType::TEXT_PLAIN)
        }
        "html" => ContentType::TEXT_HTML,
        "csv" => ContentType::parse("text/csv").unwrap_or(ContentType::TEXT_PLAIN),
        "json" => ContentType::parse("application/json").unwrap_or(ContentType::TEXT_PLAIN),
        // "md" + everything else (sandbox-rendered files default to
        // a plain MIME so unknown formats don't surface as octet-stream).
        _ => ContentType::TEXT_PLAIN,
    }
}

fn filename_for(deliverable: &Deliverable) -> String {
    std::path::Path::new(&deliverable.rendered_content_path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| format!("deliverable-{}.{}", deliverable.id, deliverable.format))
}
