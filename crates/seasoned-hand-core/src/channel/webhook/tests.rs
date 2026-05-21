//! Unit coverage for [`super::WebhookChannel`].
//!
//! The intake-side tests (token check 503/401/Ok) live here as
//! constructor-driven cases; the full HTTP route flow (axum handler +
//! IntakeRouter round-trip) is exercised in
//! `seasoned-hand-server/tests/webhook_intake.rs`.
//!
//! refs: /specs/phase-2/stories/story-2.10.md

use std::sync::Arc;

use ipnet::IpNet;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::ssrf::{is_publicly_routable, parse_allowlist};
use super::{CHANNEL_NAME, TokenCheck, WebhookChannel};
use crate::channel::{
    ChannelError, Deliverable, DeliverySink, DeliveryTarget, NotifyEvent, NotifySink, NotifyTarget,
};

fn sample_deliverable() -> Deliverable {
    Deliverable {
        id: "deliv-1".into(),
        task_id: "task-1".into(),
        tenant_id: None,
        format: "md".into(),
        source_content_path: None,
        source_content_sha256: None,
        rendered_content_path: "/workspace/.deliverables/deliv-1.md".into(),
        rendered_content_sha256: "abc123".into(),
        content_size: 42,
        citations: Some(vec![1]),
        provenance_manifest: json!({"task_id": "task-1"}),
        created_at: 0,
    }
}

#[tokio::test]
async fn verify_intake_token_returns_not_configured_when_empty() {
    let chan = WebhookChannel::with_default_client(Arc::new(String::new()), vec![]);
    assert_eq!(
        chan.verify_intake_token(Some("anything")),
        TokenCheck::NotConfigured
    );
    assert_eq!(chan.verify_intake_token(None), TokenCheck::NotConfigured);
}

#[tokio::test]
async fn verify_intake_token_rejects_missing_or_wrong() {
    let chan = WebhookChannel::with_default_client(Arc::new("secret-token".into()), vec![]);
    assert_eq!(chan.verify_intake_token(None), TokenCheck::Mismatch);
    assert_eq!(chan.verify_intake_token(Some("")), TokenCheck::Mismatch);
    assert_eq!(chan.verify_intake_token(Some("nope")), TokenCheck::Mismatch);
}

#[tokio::test]
async fn verify_intake_token_accepts_exact_match() {
    let chan = WebhookChannel::with_default_client(Arc::new("secret-token".into()), vec![]);
    assert_eq!(
        chan.verify_intake_token(Some("secret-token")),
        TokenCheck::Ok
    );
}

/// `webhook_delivery_posts_callback` — DeliverySink POSTs the spec
/// body shape to the operator-supplied URL. The host is the wiremock
/// mock server (loopback) so we exercise the SSRF allow-list bypass at
/// the same time.
#[tokio::test]
async fn webhook_delivery_posts_callback() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/cb"))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({"ok": true})))
        .mount(&mock)
        .await;

    // Allow-list bypass: wiremock binds to 127.0.0.1. Without the
    // bypass the SSRF guard rejects the call (default-deny posture).
    let allowlist = parse_allowlist("127.0.0.0/8").unwrap();
    let chan = WebhookChannel::with_default_client(Arc::new("t".into()), allowlist);
    let target = DeliveryTarget {
        channel: CHANNEL_NAME.into(),
        target_ref: format!("url:{}/cb", mock.uri()),
        metadata: json!({}),
    };

    let receipt = chan
        .deliver(&target, &sample_deliverable())
        .await
        .expect("delivery ok");
    assert_eq!(receipt.channel, CHANNEL_NAME);
    assert_eq!(receipt.external_id, "http:202");
    assert_eq!(receipt.raw_response, json!({"ok": true}));

    let received = mock.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["task_id"], "task-1");
    assert_eq!(body["deliverable_id"], "deliv-1");
    assert_eq!(body["format"], "md");
    assert_eq!(
        body["content_url"],
        "/v1/tasks/task-1/deliverables/deliv-1/content"
    );
    assert_eq!(body["status"], "completed");
    assert_eq!(body["provenance_manifest"]["task_id"], "task-1");
}

/// `webhook_delivery_retries_5xx_via_router` — the DeliveryRouter's
/// retry policy already lives in `delivery/router.rs`; this test pins
/// the contract from the channel side, namely that a 5xx response
/// surfaces as the `RemoteRejected { status: 5xx, .. }` variant the
/// router branches on (see `delivery::router::is_retryable`).
#[tokio::test]
async fn webhook_delivery_5xx_surfaces_as_remote_rejected_5xx() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/cb"))
        .respond_with(ResponseTemplate::new(503).set_body_json(json!({"err": "down"})))
        .mount(&mock)
        .await;
    let allowlist = parse_allowlist("127.0.0.0/8").unwrap();
    let chan = WebhookChannel::with_default_client(Arc::new("t".into()), allowlist);
    let target = DeliveryTarget {
        channel: CHANNEL_NAME.into(),
        target_ref: format!("url:{}/cb", mock.uri()),
        metadata: json!({}),
    };
    let err = chan
        .deliver(&target, &sample_deliverable())
        .await
        .expect_err("5xx must error");
    match err {
        ChannelError::RemoteRejected { status, .. } => {
            assert!(
                (500..600).contains(&status),
                "expected 5xx status, got {status}"
            );
        }
        other => panic!("expected RemoteRejected, got {other:?}"),
    }
}

/// `webhook_delivery_rejects_private_ip` — the SSRF guard fires
/// terminally (400 `private_address_rejected`) when the target URL
/// host resolves to a private address and no allow-list entry covers
/// it. No outbound HTTP is attempted.
#[tokio::test]
async fn webhook_delivery_rejects_private_ip() {
    let chan = WebhookChannel::with_default_client(Arc::new("t".into()), vec![]);
    let target = DeliveryTarget {
        channel: CHANNEL_NAME.into(),
        target_ref: "url:http://10.0.0.1/admin".into(),
        metadata: json!({}),
    };
    let err = chan
        .deliver(&target, &sample_deliverable())
        .await
        .expect_err("private IP must be rejected");
    match err {
        ChannelError::RemoteRejected { status, message } => {
            assert_eq!(status, 400);
            assert_eq!(message, "private_address_rejected");
        }
        other => panic!("expected RemoteRejected/400, got {other:?}"),
    }
}

/// `webhook_delivery_allows_private_ip_with_allowlist` — operator
/// can opt out per CIDR. With `10.0.0.0/8` allow-listed the SSRF
/// guard passes; the HTTP attempt then fails with `Http(_)` because
/// nothing is actually listening on 10.0.0.1, but the test only cares
/// that the rejection variant is NOT the SSRF terminal 400.
#[tokio::test]
async fn webhook_delivery_allows_private_ip_with_allowlist() {
    let allowlist: Vec<IpNet> = parse_allowlist("10.0.0.0/8").unwrap();
    let http = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_millis(150))
        .timeout(std::time::Duration::from_millis(500))
        .build()
        .unwrap();
    let chan = WebhookChannel::new(Arc::new("t".into()), http, allowlist);
    let target = DeliveryTarget {
        channel: CHANNEL_NAME.into(),
        target_ref: "url:http://10.0.0.1/admin".into(),
        metadata: json!({}),
    };
    let err = chan
        .deliver(&target, &sample_deliverable())
        .await
        .expect_err("nothing is listening; expect a transport-level error");
    // The SSRF guard must have passed: the failure is a transport
    // problem, not the `private_address_rejected` terminal 400.
    match err {
        ChannelError::Http(_) => {}
        ChannelError::Transport(_) => {}
        ChannelError::RemoteRejected { message, .. } if message == "private_address_rejected" => {
            panic!("allow-list bypass did not apply — SSRF still rejected the call");
        }
        other => panic!("unexpected error variant: {other:?}"),
    }
}

/// `webhook_delivery_does_not_follow_redirects` — SEC-IT2-M1 regression.
/// The SSRF guard validates only the initial URL, so the production
/// client must NOT follow redirects: an allowed (loopback, allow-listed)
/// target that 302-redirects to an internal path must surface the 302
/// itself and never fetch the redirect target. Without
/// `Policy::none()` this is a metadata-endpoint SSRF bypass.
#[tokio::test]
async fn webhook_delivery_does_not_follow_redirects() {
    let mock = MockServer::start().await;
    // The validated entry point: a 302 pointing at an internal path.
    Mock::given(method("POST"))
        .and(path("/redirect"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", "/internal-metadata"))
        .expect(1)
        .mount(&mock)
        .await;
    // The redirect target must NEVER be hit (would be the bypassed,
    // unvalidated address in a real attack).
    Mock::given(path("/internal-metadata"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"leaked": true})))
        .expect(0)
        .mount(&mock)
        .await;
    let allowlist = parse_allowlist("127.0.0.0/8").unwrap();
    let chan = WebhookChannel::with_default_client(Arc::new("t".into()), allowlist);
    let target = DeliveryTarget {
        channel: CHANNEL_NAME.into(),
        target_ref: format!("url:{}/redirect", mock.uri()),
        metadata: json!({}),
    };
    let err = chan
        .deliver(&target, &sample_deliverable())
        .await
        .expect_err("a non-followed 302 is not a success");
    match err {
        ChannelError::RemoteRejected { status, .. } => {
            assert_eq!(status, 302, "the 302 must surface unfollowed");
        }
        other => panic!("expected RemoteRejected/302, got {other:?}"),
    }
    // `.expect(0)` on `/internal-metadata` is verified on MockServer drop:
    // if the redirect were followed, that assertion would panic here.
    drop(mock);
}

/// `webhook_notify_posts_to_target` — NotifySink POSTs the spec
/// `{task_id?, trigger_kind, payload}` body to the target URL.
#[tokio::test]
async fn webhook_notify_posts_to_target() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/notify"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&mock)
        .await;
    let allowlist = parse_allowlist("127.0.0.0/8").unwrap();
    let chan = WebhookChannel::with_default_client(Arc::new("t".into()), allowlist);
    let target = NotifyTarget {
        channel: CHANNEL_NAME.into(),
        target_ref: format!("url:{}/notify", mock.uri()),
        metadata: json!({}),
    };
    let event = NotifyEvent {
        trigger_kind: "task_finished".into(),
        task_id: Some("task-99".into()),
        payload: json!({"title": "done", "body": "ok"}),
    };
    let receipt = chan.notify(&target, &event).await.expect("notify ok");
    assert_eq!(receipt.channel, CHANNEL_NAME);
    assert_eq!(receipt.external_id.as_deref(), Some("http:200"));

    let received = mock.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["task_id"], "task-99");
    assert_eq!(body["trigger_kind"], "task_finished");
    assert_eq!(body["payload"]["title"], "done");
}

#[test]
fn ssrf_helper_classifies_public_vs_private_ips() {
    let public: std::net::IpAddr = "1.1.1.1".parse().unwrap();
    let private: std::net::IpAddr = "10.0.0.1".parse().unwrap();
    let loopback: std::net::IpAddr = "127.0.0.1".parse().unwrap();
    assert!(is_publicly_routable(public));
    assert!(!is_publicly_routable(private));
    assert!(!is_publicly_routable(loopback));
}
