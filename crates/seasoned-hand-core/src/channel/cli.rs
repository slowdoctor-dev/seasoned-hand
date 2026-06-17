//! `CliChannel` — fifth and last Phase-2 channel. Lets the
//! `seasoned-hand task new "..."` CLI invocation (story 2.21) submit
//! work and block until the deliverable comes back.
//!
//! Implements:
//! - [`IntakeProvider`] — `run()` is a no-op (intake source is the
//!   CLI binary calling [`CliChannel::submit`] in-process, similar to
//!   the chat/webhook pattern where the channel framework registry is
//!   the routing target but the request originates outside `run()`).
//! - [`DeliverySink`] — delivers via a `tokio::sync::oneshot` keyed by
//!   `intake_id`. Falls back to writing a file under
//!   `~/.seasoned-hand/deliverables/<deliverable_id>.<ext>` when no
//!   sender is registered (CLI exited / `--detach` flag).
//!
//! Notify is intentionally NOT implemented — terminal push is awkward;
//! operators use ntfy or email for status notifications.
//!
//! refs: /specs/phase-2/architecture.md §2.7 (channel matrix), §2.10 (CLI)
//! refs: /specs/phase-2/stories/story-2.13.md

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::{
    ChannelError, Deliverable, DeliveryReceipt, DeliverySink, DeliveryTarget, IntakeEvent,
    IntakeProvider,
};

use crate::time::now_micros;
/// Channel name registered into the [`crate::channel::ChannelRegistry`].
pub const CHANNEL_NAME: &str = "cli";

/// Prefix on [`DeliveryTarget::target_ref`] — encodes the
/// `intake_id` the CLI invocation registered with [`CliChannel::register_pending`].
/// CLI's `task new` builds this when constructing its `reply_target`.
pub const TARGET_INTAKE_PREFIX: &str = "intake:";

/// Channel name for the IntakeEvent the CLI binary emits.
pub const INTAKE_ID_PREFIX: &str = "cli:";

/// One outstanding deliverable per running CLI invocation. The
/// `oneshot::Sender` is consumed on first call to
/// [`CliChannel::deliver`]; the map slot is removed atomically.
pub struct CliChannel {
    pending: Arc<DashMap<String, oneshot::Sender<Deliverable>>>,
    /// Optional override for the fallback directory. Defaults to
    /// `~/.seasoned-hand/deliverables/`. Tests override to a tmpdir
    /// so they don't write into the developer's actual home dir.
    fallback_dir: Option<PathBuf>,
}

impl Default for CliChannel {
    fn default() -> Self {
        Self::new()
    }
}

impl CliChannel {
    pub fn new() -> Self {
        Self {
            pending: Arc::new(DashMap::new()),
            fallback_dir: None,
        }
    }

    /// Test seam — override the fallback dir so tests can inspect the
    /// written file without polluting `$HOME`.
    pub fn with_fallback_dir(mut self, dir: PathBuf) -> Self {
        self.fallback_dir = Some(dir);
        self
    }

    /// Register a one-shot sender keyed by `intake_id`. Returns the
    /// matched receiver the CLI invocation awaits on. Called by the
    /// CLI's `task new` subcommand BEFORE pushing the IntakeEvent so
    /// there's no race with a fast deliver().
    pub fn register_pending(&self, intake_id: impl Into<String>) -> oneshot::Receiver<Deliverable> {
        let (tx, rx) = oneshot::channel();
        self.pending.insert(intake_id.into(), tx);
        rx
    }

    /// Drop a pending registration without delivering — used by the
    /// CLI when the user Ctrl-Cs the blocking invocation.
    pub fn drop_pending(&self, intake_id: &str) {
        self.pending.remove(intake_id);
    }

    /// Number of pending senders currently registered. Diagnostic
    /// surface — production callers don't need this; tests + the
    /// `/v1/channels/cli/health` route can use it.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Resolved fallback dir — explicit override else `$HOME` lookup.
    /// Returns `None` only on systems where `HOME` is unset (Windows
    /// runtimes that don't set it, exotic CI) — callers treat that as
    /// "skip fallback, just log".
    fn resolve_fallback_dir(&self) -> Option<PathBuf> {
        if let Some(dir) = &self.fallback_dir {
            return Some(dir.clone());
        }
        let home = std::env::var("HOME").ok()?;
        Some(PathBuf::from(home).join(".seasoned-hand/deliverables"))
    }

    async fn write_fallback_file(
        &self,
        deliverable: &Deliverable,
    ) -> Result<Option<PathBuf>, ChannelError> {
        let Some(dir) = self.resolve_fallback_dir() else {
            return Ok(None);
        };
        if let Err(error) = tokio::fs::create_dir_all(&dir).await {
            tracing::warn!(
                %error,
                path = %dir.display(),
                "cli_channel: failed to ensure fallback dir; deliverable lost"
            );
            return Ok(None);
        }
        if !is_safe_file_stem(&deliverable.id) {
            return Err(ChannelError::Internal(format!(
                "cli_channel: unsafe deliverable id rejected: {:?}",
                deliverable.id
            )));
        }
        let ext = format_extension(&deliverable.format);
        let path = dir.join(format!("{}.{}", deliverable.id, ext));
        let manifest = match serde_json::to_vec_pretty(deliverable) {
            Ok(v) => v,
            Err(error) => {
                tracing::warn!(
                    %error,
                    "cli_channel: serialize deliverable manifest failed; skipping fallback"
                );
                return Ok(None);
            }
        };
        if let Err(error) = tokio::fs::write(&path, &manifest).await {
            tracing::warn!(
                %error,
                path = %path.display(),
                "cli_channel: write fallback failed"
            );
            return Ok(None);
        }
        Ok(Some(path))
    }
}

#[async_trait]
impl IntakeProvider for CliChannel {
    fn name(&self) -> &'static str {
        CHANNEL_NAME
    }

    /// Intake source is the in-process CLI binary calling
    /// [`CliChannel::register_pending`] + pushing the IntakeEvent
    /// directly into `IntakeRouter::handle_event`. The `run()`
    /// lifecycle just parks on shutdown so the registry's
    /// `spawn_intakes` loop has a uniform shape across all channels.
    async fn run(
        &self,
        _sink: mpsc::Sender<IntakeEvent>,
        shutdown: CancellationToken,
    ) -> Result<(), ChannelError> {
        shutdown.cancelled().await;
        Ok(())
    }
}

#[async_trait]
impl DeliverySink for CliChannel {
    fn name(&self) -> &'static str {
        CHANNEL_NAME
    }

    async fn deliver(
        &self,
        target: &DeliveryTarget,
        deliverable: &Deliverable,
    ) -> Result<DeliveryReceipt, ChannelError> {
        let intake_id = target
            .target_ref
            .strip_prefix(TARGET_INTAKE_PREFIX)
            .ok_or_else(|| ChannelError::RemoteRejected {
                status: 400,
                message: format!(
                    "expected target_ref `{TARGET_INTAKE_PREFIX}<id>`, got `{}`",
                    target.target_ref
                ),
            })?;
        if intake_id.is_empty() {
            return Err(ChannelError::RemoteRejected {
                status: 400,
                message: "empty intake_id".into(),
            });
        }

        // Try the one-shot path first — happy case is the CLI is
        // blocking on `rx.await`. `DashMap::remove` returns the
        // `(key, value)` pair if present so we don't leave a stale
        // entry behind on failed send (receiver dropped).
        if let Some((_, tx)) = self.pending.remove(intake_id) {
            match tx.send(deliverable.clone()) {
                Ok(()) => {
                    return Ok(DeliveryReceipt {
                        channel: CHANNEL_NAME.into(),
                        external_id: format!("cli:oneshot:{intake_id}"),
                        delivered_at: now_micros(),
                        raw_response: serde_json::json!({
                            "delivered_via": "oneshot",
                            "intake_id": intake_id,
                        }),
                    });
                }
                Err(_unsent) => {
                    // Receiver was dropped between register + deliver
                    // (CLI exited mid-flight). Fall through to the
                    // file-fallback path.
                    tracing::info!(
                        %intake_id,
                        "cli_channel: oneshot receiver dropped; falling back to file"
                    );
                }
            }
        }

        // Fallback: write to ~/.seasoned-hand/deliverables/<id>.<ext>.
        // Detached CLI invocations land here from the first deliver
        // (no pending sender was ever registered).
        let path = self.write_fallback_file(deliverable).await?;
        Ok(DeliveryReceipt {
            channel: CHANNEL_NAME.into(),
            external_id: match &path {
                Some(p) => format!("cli:file:{}", p.display()),
                None => "cli:file:dropped".into(),
            },
            delivered_at: now_micros(),
            raw_response: serde_json::json!({
                "delivered_via": "file",
                "intake_id": intake_id,
                "path": path.as_ref().map(|p| p.display().to_string()),
            }),
        })
    }
}

/// Map architecture §2.3 format names to file extensions. `code` and
/// `url` are deferred placeholders (see §2.3 footnote); we use
/// `json` as the manifest extension so the fallback file is always
/// a structured artifact rather than a raw text dump.
fn format_extension(format: &str) -> &'static str {
    match format {
        "docx" => "docx",
        "pdf" => "pdf",
        "html" => "html",
        "pptx" => "pptx",
        "xlsx" => "xlsx",
        "csv" => "csv",
        "md" => "md",
        "json" | "code" | "url" => "json",
        _ => "json",
    }
}

fn is_safe_file_stem(stem: &str) -> bool {
    !stem.is_empty() && stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fake_deliverable(id: &str, format: &str) -> Deliverable {
        Deliverable {
            id: id.into(),
            task_id: "task-1".into(),
            tenant_id: None,
            format: format.into(),
            source_content_path: Some("/workspace/.deliverables/source.md".into()),
            source_content_sha256: Some("deadbeef".into()),
            rendered_content_path: format!("/workspace/.deliverables/{id}.{format}"),
            rendered_content_sha256: "feedface".into(),
            content_size: 1024,
            citations: Some(vec![1, 2, 3]),
            provenance_manifest: json!({}),
            created_at: 0,
        }
    }

    fn target_for(intake_id: &str) -> DeliveryTarget {
        DeliveryTarget {
            channel: CHANNEL_NAME.into(),
            target_ref: format!("{TARGET_INTAKE_PREFIX}{intake_id}"),
            metadata: json!({}),
        }
    }

    #[tokio::test]
    async fn cli_channel_intake_and_delivery_roundtrip() {
        let channel = CliChannel::new();
        let rx = channel.register_pending("cli:42");
        assert_eq!(channel.pending_count(), 1);

        // Spawn the "CLI invocation" waiting on the deliverable.
        let recv_task = tokio::spawn(async move { rx.await.expect("oneshot must deliver") });

        // Server-side: deliverable lands.
        let deliverable = fake_deliverable("d-1", "md");
        let receipt = channel
            .deliver(&target_for("cli:42"), &deliverable)
            .await
            .expect("deliver ok");
        assert_eq!(receipt.channel, CHANNEL_NAME);
        assert_eq!(receipt.external_id, "cli:oneshot:cli:42");
        assert_eq!(channel.pending_count(), 0, "sender slot removed on deliver");

        let delivered = recv_task.await.expect("join");
        assert_eq!(delivered.id, deliverable.id);
        assert_eq!(delivered.format, "md");
    }

    #[tokio::test]
    async fn cli_channel_detach_skips_oneshot() {
        // No `register_pending` → simulates `--detach` mode. deliver()
        // must NOT block looking for a sender; it falls through to
        // file fallback.
        let tmp = tempfile::tempdir().expect("tmpdir");
        let channel = CliChannel::new().with_fallback_dir(tmp.path().into());
        assert_eq!(channel.pending_count(), 0);

        let deliverable = fake_deliverable("d-detach", "json");
        let receipt = channel
            .deliver(&target_for("cli:99"), &deliverable)
            .await
            .expect("deliver ok");
        assert!(
            receipt.external_id.starts_with("cli:file:"),
            "fallback file path: {}",
            receipt.external_id
        );
        let path = tmp.path().join("d-detach.json");
        assert!(path.exists(), "fallback file written: {}", path.display());
    }

    #[tokio::test]
    async fn cli_channel_fallback_to_file_when_pending_missing() {
        // Register a pending sender, then drop the receiver before
        // deliver() lands → fallback fires.
        let tmp = tempfile::tempdir().expect("tmpdir");
        let channel = CliChannel::new().with_fallback_dir(tmp.path().into());
        let rx = channel.register_pending("cli:5");
        drop(rx); // CLI invocation exited.

        let deliverable = fake_deliverable("d-orphan", "docx");
        let receipt = channel
            .deliver(&target_for("cli:5"), &deliverable)
            .await
            .expect("deliver ok");
        assert!(
            receipt.external_id.starts_with("cli:file:"),
            "fell back to file: {}",
            receipt.external_id
        );
        assert!(
            tmp.path().join("d-orphan.docx").exists(),
            "fallback file written with format-derived ext"
        );
    }

    #[tokio::test]
    async fn cli_channel_rejects_unsafe_deliverable_id_on_fallback() {
        let tmp = tempfile::tempdir().expect("tmpdir");
        let channel = CliChannel::new().with_fallback_dir(tmp.path().into());

        let err = channel
            .deliver(
                &target_for("cli:unsafe"),
                &fake_deliverable("../evil", "md"),
            )
            .await
            .expect_err("unsafe id must fail");
        match err {
            ChannelError::Internal(msg) => assert!(msg.contains("unsafe deliverable id")),
            other => panic!("expected Internal, got {other:?}"),
        }
        assert!(
            !tmp.path().join("../evil.md").exists(),
            "unsafe fallback path must never be created"
        );
    }

    #[tokio::test]
    async fn cli_channel_rejects_bad_target_ref() {
        let channel = CliChannel::new();
        let err = channel
            .deliver(
                &DeliveryTarget {
                    channel: CHANNEL_NAME.into(),
                    target_ref: "wrong-prefix:foo".into(),
                    metadata: json!({}),
                },
                &fake_deliverable("d", "md"),
            )
            .await
            .expect_err("bad prefix");
        match err {
            ChannelError::RemoteRejected { status, .. } => assert_eq!(status, 400),
            other => panic!("expected RemoteRejected, got {other:?}"),
        }
    }
}
