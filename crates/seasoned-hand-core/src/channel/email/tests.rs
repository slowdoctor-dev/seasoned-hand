//! Unit coverage for [`super::EmailChannel`].
//!
//! IMAP polling is exercised against [`MockMailbox`] (canned RFC 822
//! bytes); SMTP send is exercised against [`RecordingTransport`]. The
//! real `AsyncImapFetcher` + `LettreSmtpTransport` paths are tested
//! end-to-end at the integration layer (story 2.11 acceptance keeps
//! live IMAP / SMTP out of CI).
//!
//! refs: /specs/phase-2/stories/story-2.11.md

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tempfile::TempDir;
use tokio::sync::mpsc;

use super::*;
use crate::channel::{Deliverable, DeliveryTarget, IntakeEvent, NotifyEvent, NotifyTarget};

const PLAIN_INTAKE: &[u8] = b"\
From: \"Operator\" <operator@example.com>\r\n\
To: bot@example.com\r\n\
Subject: [sh] Summarize the Q4 deck\r\n\
Message-ID: <abc123@mail.example.com>\r\n\
Authentication-Results: mx.example.com; spf=pass smtp.mailfrom=operator@example.com; dkim=pass header.d=example.com\r\n\
MIME-Version: 1.0\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Please summarize the deck and email it back.\r\n";

const UNKNOWN_SENDER: &[u8] = b"\
From: stranger@example.org\r\n\
To: bot@example.com\r\n\
Subject: [sh] please help\r\n\
Message-ID: <stranger@mail.example.org>\r\n\
MIME-Version: 1.0\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Hi from outside.\r\n";

const NO_PREFIX: &[u8] = b"\
From: operator@example.com\r\n\
To: bot@example.com\r\n\
Subject: weekend trip plans\r\n\
Message-ID: <plans@mail.example.com>\r\n\
MIME-Version: 1.0\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Not a Seasoned Hand intake.\r\n";

fn allow_operator() -> AllowList {
    AllowList::parse("operator@example.com").expect("compile allow-list")
}

fn channel_with(
    fetcher: Arc<MockMailbox>,
    transport: Arc<RecordingTransport>,
    allow_list: AllowList,
) -> EmailChannel {
    EmailChannel::builder()
        .fetcher(fetcher)
        .transport(transport)
        .from_address("Seasoned Hand <bot@example.com>")
        .allow_list(allow_list)
        .poll_interval(Duration::from_millis(10))
        .build()
        .expect("build channel")
}

#[tokio::test]
async fn email_imap_intake_creates_event_from_test_message() {
    let mailbox = Arc::new(MockMailbox::new());
    mailbox
        .enqueue(RawMessage {
            uid: 7,
            bytes: PLAIN_INTAKE.to_vec(),
        })
        .await;
    let transport = Arc::new(RecordingTransport::new());
    let channel = channel_with(mailbox.clone(), transport, allow_operator());

    let (tx, mut rx) = mpsc::channel::<IntakeEvent>(8);
    let emitted = channel.poll_once(&tx).await.expect("poll ok");
    assert_eq!(emitted, 1);

    let event = rx.recv().await.expect("event emitted");
    assert_eq!(event.channel, CHANNEL_NAME);
    assert_eq!(event.intake_id, "imap:7");
    assert_eq!(
        event.brief_input.trim(),
        "Please summarize the deck and email it back."
    );
    let target = event.reply_target.expect("reply_target set");
    assert_eq!(target.channel, CHANNEL_NAME);
    assert_eq!(target.target_ref, "msgid:<abc123@mail.example.com>");
    assert_eq!(
        target.metadata.get("to").and_then(|v| v.as_str()),
        Some("operator@example.com")
    );
    assert_eq!(
        event.metadata.get("from").and_then(|v| v.as_str()),
        Some("operator@example.com")
    );
    assert_eq!(
        event.metadata.get("subject").and_then(|v| v.as_str()),
        Some("[sh] Summarize the Q4 deck")
    );

    // Mark-seen MUST fire so the next poll cycle doesn't re-emit
    // the same UID.
    assert_eq!(mailbox.seen_snapshot().await, vec![7]);
}

#[tokio::test]
async fn email_intake_rejects_unknown_sender() {
    let mailbox = Arc::new(MockMailbox::new());
    mailbox
        .enqueue(RawMessage {
            uid: 9,
            bytes: UNKNOWN_SENDER.to_vec(),
        })
        .await;
    let transport = Arc::new(RecordingTransport::new());
    let channel = channel_with(mailbox.clone(), transport, allow_operator());

    let (tx, mut rx) = mpsc::channel::<IntakeEvent>(8);
    let emitted = channel.poll_once(&tx).await.expect("poll ok");
    assert_eq!(emitted, 0);
    assert!(rx.try_recv().is_err(), "no event must be emitted");
    // Even a rejected message is marked-seen so it doesn't recur.
    assert_eq!(mailbox.seen_snapshot().await, vec![9]);
}

#[tokio::test]
async fn email_intake_default_deny_when_allowlist_empty() {
    let mailbox = Arc::new(MockMailbox::new());
    mailbox
        .enqueue(RawMessage {
            uid: 10,
            bytes: PLAIN_INTAKE.to_vec(),
        })
        .await;
    let transport = Arc::new(RecordingTransport::new());
    // Default-empty allow-list — architecture §9 default-deny.
    let channel = channel_with(mailbox.clone(), transport, AllowList::default());

    let (tx, mut rx) = mpsc::channel::<IntakeEvent>(8);
    let emitted = channel.poll_once(&tx).await.expect("poll ok");
    assert_eq!(emitted, 0);
    assert!(rx.try_recv().is_err(), "default-deny must drop the event");
}

#[tokio::test]
async fn email_intake_requires_subject_prefix() {
    let mailbox = Arc::new(MockMailbox::new());
    mailbox
        .enqueue(RawMessage {
            uid: 11,
            bytes: NO_PREFIX.to_vec(),
        })
        .await;
    let transport = Arc::new(RecordingTransport::new());
    let channel = channel_with(mailbox.clone(), transport, allow_operator());

    let (tx, mut rx) = mpsc::channel::<IntakeEvent>(8);
    let emitted = channel.poll_once(&tx).await.expect("poll ok");
    assert_eq!(emitted, 0);
    assert!(rx.try_recv().is_err(), "missing prefix must skip");
    assert_eq!(
        mailbox.seen_snapshot().await,
        vec![11],
        "skipped mail still marked seen so it doesn't recur"
    );
}

#[tokio::test]
async fn email_intake_records_dkim_pass_in_metadata() {
    let mailbox = Arc::new(MockMailbox::new());
    mailbox
        .enqueue(RawMessage {
            uid: 12,
            bytes: PLAIN_INTAKE.to_vec(),
        })
        .await;
    let transport = Arc::new(RecordingTransport::new());
    let channel = channel_with(mailbox, transport, allow_operator());

    let (tx, mut rx) = mpsc::channel::<IntakeEvent>(8);
    channel.poll_once(&tx).await.expect("poll ok");
    let event = rx.recv().await.expect("event emitted");

    // SPF + DKIM both stamped pass by the upstream MTA; we surface
    // them verbatim into metadata for audit + later operator-driven
    // policy tightening (Phase 5).
    assert_eq!(
        event.metadata.get("spf").and_then(|v| v.as_str()),
        Some("pass")
    );
    assert_eq!(
        event.metadata.get("dkim").and_then(|v| v.as_str()),
        Some("pass")
    );
}

#[tokio::test]
async fn email_delivery_sends_reply_with_attachment() {
    let tmp = TempDir::new().expect("tmpdir");
    let path = tmp.path().join("deliv-1.md");
    std::fs::write(&path, b"# summary\nContent.\n").expect("write deliv");

    let mailbox = Arc::new(MockMailbox::new());
    let transport = Arc::new(RecordingTransport::new());
    let channel = channel_with(mailbox, transport.clone(), allow_operator());

    let target = DeliveryTarget {
        channel: CHANNEL_NAME.into(),
        target_ref: "msgid:<abc123@mail.example.com>".into(),
        metadata: json!({
            "to": "operator@example.com",
            "subject": "[sh] Summarize the Q4 deck",
        }),
    };
    let deliverable = Deliverable {
        id: "deliv-1".into(),
        task_id: "task-1".into(),
        tenant_id: None,
        format: "md".into(),
        source_content_path: None,
        source_content_sha256: None,
        rendered_content_path: path.to_string_lossy().into_owned(),
        rendered_content_sha256: "abc".into(),
        content_size: 19,
        citations: None,
        provenance_manifest: json!({}),
        created_at: 0,
    };
    let receipt = channel
        .deliver(&target, &deliverable)
        .await
        .expect("deliver ok");
    assert_eq!(receipt.channel, CHANNEL_NAME);
    assert!(receipt.external_id.starts_with("250"), "lettre 250-status");

    let captured = transport.snapshot().await;
    assert_eq!(captured.len(), 1);
    let raw = String::from_utf8_lossy(&captured[0].formatted()).to_string();
    // Subject is `[Re:]`-prefixed and threading headers are present.
    assert!(
        raw.contains("Subject: Re: [sh] Summarize the Q4 deck"),
        "{raw}"
    );
    assert!(
        raw.contains("In-Reply-To: <abc123@mail.example.com>"),
        "{raw}"
    );
    assert!(
        raw.contains("References: <abc123@mail.example.com>"),
        "{raw}"
    );
    assert!(
        raw.contains("filename=\"deliv-1.md\""),
        "attachment filename"
    );
    // Body bytes survive the multipart encode.
    assert!(
        raw.contains("# summary") || raw.contains("IyBzdW1tYXJ5"),
        "body present (raw or base64)"
    );
}

#[tokio::test]
async fn email_delivery_does_not_double_re_prefix() {
    let tmp = TempDir::new().expect("tmpdir");
    let path = tmp.path().join("deliv-2.md");
    std::fs::write(&path, b"x").expect("write deliv");

    let mailbox = Arc::new(MockMailbox::new());
    let transport = Arc::new(RecordingTransport::new());
    let channel = channel_with(mailbox, transport.clone(), allow_operator());

    // Original subject already prefixed `Re:` — must not become `Re: Re: ...`.
    let target = DeliveryTarget {
        channel: CHANNEL_NAME.into(),
        target_ref: "msgid:<thread@x>".into(),
        metadata: json!({
            "to": "operator@example.com",
            "subject": "Re: [sh] Original Topic",
        }),
    };
    let deliverable = Deliverable {
        id: "deliv-2".into(),
        task_id: "task-2".into(),
        tenant_id: None,
        format: "md".into(),
        source_content_path: None,
        source_content_sha256: None,
        rendered_content_path: path.to_string_lossy().into_owned(),
        rendered_content_sha256: "y".into(),
        content_size: 1,
        citations: None,
        provenance_manifest: json!({}),
        created_at: 0,
    };
    channel
        .deliver(&target, &deliverable)
        .await
        .expect("deliver ok");
    let captured = transport.snapshot().await;
    let raw = String::from_utf8_lossy(&captured[0].formatted()).to_string();
    assert!(raw.contains("Subject: Re: [sh] Original Topic"), "{raw}");
    assert!(!raw.contains("Re: Re:"), "double prefix leaked: {raw}");
}

#[tokio::test]
async fn email_notify_sends_status_message() {
    let mailbox = Arc::new(MockMailbox::new());
    let transport = Arc::new(RecordingTransport::new());
    let channel = channel_with(mailbox, transport.clone(), allow_operator());

    let target = NotifyTarget {
        channel: CHANNEL_NAME.into(),
        target_ref: "operator@example.com".into(),
        metadata: json!({}),
    };
    let event = NotifyEvent {
        trigger_kind: "task_finished".into(),
        task_id: Some("task-99".into()),
        payload: json!({"title": "done", "summary": "ok"}),
    };
    let receipt = channel.notify(&target, &event).await.expect("notify ok");
    assert_eq!(receipt.channel, CHANNEL_NAME);
    assert!(receipt.external_id.is_some());

    let captured = transport.snapshot().await;
    assert_eq!(captured.len(), 1);
    let raw = String::from_utf8_lossy(&captured[0].formatted()).to_string();
    assert!(raw.contains("Subject: [sh] task_finished"), "{raw}");
    assert!(raw.contains("To: operator@example.com"), "{raw}");
    // Body contains the pretty-printed payload (raw or quoted-printable
    // depending on lettre's encoding decision; one or the other must
    // be present).
    let body_present = raw.contains("\"title\"") || raw.contains("title");
    assert!(body_present, "payload not in body: {raw}");
}

#[tokio::test]
async fn email_intake_run_loop_honours_shutdown() {
    let mailbox = Arc::new(MockMailbox::new());
    let transport = Arc::new(RecordingTransport::new());
    let channel = Arc::new(channel_with(mailbox, transport, allow_operator()));

    let (tx, _rx) = mpsc::channel::<IntakeEvent>(4);
    let shutdown = CancellationToken::new();
    let token = shutdown.clone();
    let chan = channel.clone();
    let handle = tokio::spawn(async move { chan.run(tx, token).await });

    // Give the loop a beat to enter its first sleep, then cancel.
    tokio::time::sleep(Duration::from_millis(20)).await;
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(1), handle)
        .await
        .expect("loop did not exit on shutdown")
        .expect("loop join")
        .expect("loop result");
}
