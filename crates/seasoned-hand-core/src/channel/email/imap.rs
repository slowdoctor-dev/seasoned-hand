//! IMAP fetcher abstraction for [`super::EmailChannel`].
//!
//! `MailboxFetcher` keeps the polling logic in `mod.rs` independent of
//! the concrete IMAP backend so unit tests inject a `MockMailbox` with
//! canned RFC 822 bytes. The real impl in [`AsyncImapFetcher`] uses
//! `async-imap` over `tokio-rustls`.
//!
//! refs: /specs/phase-2/architecture.md §2.8 "Email intake"

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::channel::ChannelError;

use super::EMAIL_NETWORK_TIMEOUT;

/// One fetched message: the IMAP UID + raw RFC 822 bytes. The poll
/// loop hands the bytes to `mailparse` to extract sender / subject /
/// body / attachments.
#[derive(Debug, Clone)]
pub struct RawMessage {
    pub uid: u32,
    pub bytes: Vec<u8>,
}

/// Long-lived IMAP connection abstraction. Both methods are
/// idempotent on the per-message level (re-marking an already-seen UID
/// is a no-op for IMAP).
#[async_trait]
pub trait MailboxFetcher: Send + Sync {
    /// Fetch every UNSEEN message in the inbox. Returns an empty vec
    /// when nothing is pending; an `Err` only on transport failure
    /// (the poll loop counts toward exponential backoff).
    async fn fetch_unseen(&self) -> Result<Vec<RawMessage>, ChannelError>;

    /// Mark `uid` as `\Seen` so the next poll cycle does not re-emit
    /// it. Failure here is logged but not fatal — the next pass will
    /// re-deliver, and the IntakeRouter is idempotent on
    /// `(channel, intake_id)` (V008 UNIQUE constraint).
    async fn mark_seen(&self, uid: u32) -> Result<(), ChannelError>;
}

/// IMAP credentials + endpoint the [`AsyncImapFetcher`] connects to.
#[derive(Clone)]
pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

impl fmt::Debug for ImapConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImapConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &"***")
            .finish()
    }
}

/// Production fetcher: opens a fresh TLS-wrapped IMAP session per
/// poll cycle. Cycle frequency (default 30 s) means the per-cycle
/// connect cost is amortised, and the alternative — keeping one long
/// session open — fights aggressively with operator-side IMAP IDLE
/// keep-alive and connection-limit policies.
pub struct AsyncImapFetcher {
    config: ImapConfig,
}

impl AsyncImapFetcher {
    pub fn new(config: ImapConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl MailboxFetcher for AsyncImapFetcher {
    async fn fetch_unseen(&self) -> Result<Vec<RawMessage>, ChannelError> {
        run_session(&self.config, |session| {
            Box::pin(async move {
                use futures_util::StreamExt;
                timeout(EMAIL_NETWORK_TIMEOUT, session.select("INBOX"))
                    .await
                    .map_err(|_| ChannelError::Transport("imap select timed out".into()))?
                    .map_err(|err| ChannelError::Transport(format!("imap select: {err}")))?;
                let unseen = timeout(EMAIL_NETWORK_TIMEOUT, session.uid_search("UNSEEN"))
                    .await
                    .map_err(|_| ChannelError::Transport("imap uid_search timed out".into()))?
                    .map_err(|err| ChannelError::Transport(format!("imap uid_search: {err}")))?;
                if unseen.is_empty() {
                    return Ok(Vec::new());
                }
                let mut uids = unseen.into_iter().collect::<Vec<_>>();
                uids.sort();
                let set = uids
                    .iter()
                    .map(|u| u.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                let mut out = Vec::with_capacity(uids.len());
                let mut stream = timeout(
                    EMAIL_NETWORK_TIMEOUT,
                    session.uid_fetch(&set, "(UID RFC822)"),
                )
                .await
                .map_err(|_| ChannelError::Transport("imap uid_fetch timed out".into()))?
                .map_err(|err| ChannelError::Transport(format!("imap uid_fetch: {err}")))?;
                loop {
                    let item = timeout(EMAIL_NETWORK_TIMEOUT, stream.next())
                        .await
                        .map_err(|_| {
                            ChannelError::Transport("imap fetch stream timed out".into())
                        })?;
                    let Some(item) = item else { break };
                    let msg = item.map_err(|err| {
                        ChannelError::Transport(format!("imap fetch stream: {err}"))
                    })?;
                    let Some(uid) = msg.uid else { continue };
                    let Some(body) = msg.body() else { continue };
                    out.push(RawMessage {
                        uid,
                        bytes: body.to_vec(),
                    });
                }
                drop(stream);
                Ok(out)
            })
        })
        .await
    }

    async fn mark_seen(&self, uid: u32) -> Result<(), ChannelError> {
        run_session(&self.config, |session| {
            Box::pin(async move {
                use futures_util::StreamExt;
                timeout(EMAIL_NETWORK_TIMEOUT, session.select("INBOX"))
                    .await
                    .map_err(|_| ChannelError::Transport("imap select timed out".into()))?
                    .map_err(|err| ChannelError::Transport(format!("imap select: {err}")))?;
                let mut store_stream = timeout(
                    EMAIL_NETWORK_TIMEOUT,
                    session.uid_store(uid.to_string(), "+FLAGS (\\Seen)"),
                )
                .await
                .map_err(|_| ChannelError::Transport("imap uid_store timed out".into()))?
                .map_err(|err| ChannelError::Transport(format!("imap uid_store: {err}")))?;
                // Drain the response stream so the server-side STORE
                // completes before we LOGOUT in `run_session`.
                loop {
                    let item = timeout(EMAIL_NETWORK_TIMEOUT, store_stream.next())
                        .await
                        .map_err(|_| {
                            ChannelError::Transport("imap uid_store stream timed out".into())
                        })?;
                    let Some(item) = item else { break };
                    item.map_err(|err| {
                        ChannelError::Transport(format!("imap uid_store stream: {err}"))
                    })?;
                }
                Ok(())
            })
        })
        .await
    }
}

/// Open a fresh TLS-wrapped IMAP session, run `body` against it, then
/// LOGOUT (best-effort). The closure form keeps the concrete
/// `Session<TlsStream<TcpStream>>` type internal to this module.
async fn run_session<F, R>(config: &ImapConfig, body: F) -> Result<R, ChannelError>
where
    F: for<'a> FnOnce(
        &'a mut async_imap::Session<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<R, ChannelError>> + Send + 'a>,
    >,
{
    use tokio::net::TcpStream;
    use tokio_rustls::TlsConnector;
    use tokio_rustls::rustls::ClientConfig;
    use tokio_rustls::rustls::pki_types::ServerName;

    let addr = format!("{}:{}", config.host, config.port);
    let tcp = timeout(EMAIL_NETWORK_TIMEOUT, TcpStream::connect(&addr))
        .await
        .map_err(|_| ChannelError::Transport(format!("imap tcp connect {addr}: timed out")))?
        .map_err(|err| ChannelError::Transport(format!("imap tcp connect {addr}: {err}")))?;

    let tls_config = ClientConfig::builder()
        .with_root_certificates(default_root_store())
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(tls_config));
    let dns_name = ServerName::try_from(config.host.clone())
        .map_err(|err| ChannelError::Internal(format!("imap dns name {:?}: {err}", config.host)))?;
    let tls = timeout(EMAIL_NETWORK_TIMEOUT, connector.connect(dns_name, tcp))
        .await
        .map_err(|_| ChannelError::Transport("imap tls handshake timed out".into()))?
        .map_err(|err| ChannelError::Transport(format!("imap tls handshake: {err}")))?;

    let client = async_imap::Client::new(tls);
    let mut session = timeout(
        EMAIL_NETWORK_TIMEOUT,
        client.login(&config.username, &config.password),
    )
    .await
    .map_err(|_| ChannelError::Transport("imap login timed out".into()))?
    .map_err(|(err, _)| ChannelError::Transport(format!("imap login: {err}")))?;

    let outcome = body(&mut session).await;
    let _ = timeout(EMAIL_NETWORK_TIMEOUT, session.logout()).await;
    outcome
}

fn default_root_store() -> tokio_rustls::rustls::RootCertStore {
    use tokio_rustls::rustls::RootCertStore;
    let mut store = RootCertStore::empty();
    store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    store
}

/// Test fetcher with a queue of `RawMessage`s. `fetch_unseen` drains
/// the queue (next call returns `[]` until the queue is refilled).
/// `mark_seen` records the UIDs in `seen_uids` so tests can assert
/// the channel acknowledged the message.
#[derive(Default, Clone)]
pub struct MockMailbox {
    pub queue: Arc<Mutex<Vec<RawMessage>>>,
    pub seen_uids: Arc<Mutex<Vec<u32>>>,
}

impl MockMailbox {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn enqueue(&self, msg: RawMessage) {
        self.queue.lock().await.push(msg);
    }

    pub async fn seen_snapshot(&self) -> Vec<u32> {
        self.seen_uids.lock().await.clone()
    }
}

#[async_trait]
impl MailboxFetcher for MockMailbox {
    async fn fetch_unseen(&self) -> Result<Vec<RawMessage>, ChannelError> {
        let mut queue = self.queue.lock().await;
        let drained = std::mem::take(&mut *queue);
        Ok(drained)
    }

    async fn mark_seen(&self, uid: u32) -> Result<(), ChannelError> {
        self.seen_uids.lock().await.push(uid);
        Ok(())
    }
}
