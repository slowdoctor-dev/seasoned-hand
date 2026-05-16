//! Story 2.5 — `/v1/channels*` integration coverage.
//!
//! Exercises the three introspection routes against a populated
//! [`ChannelRegistry`] installed via `AppState::with_channels`. The
//! channel impls used here are minimal trait stubs — concrete channels
//! (WebhookChannel, EmailChannel, ...) land in stories 2.9–2.13.
//!
//! refs: /specs/phase-2/stories/story-2.5.md

use std::sync::Arc;

use async_trait::async_trait;
use axum::http::StatusCode;
use seasoned_hand_core::channel::{
    ChannelError, ChannelRegistration, Deliverable, DeliveryReceipt, DeliverySink, DeliveryTarget,
    IntakeEvent, IntakeProvider, NotifyEvent, NotifyReceipt, NotifySink, NotifyTarget,
};
use seasoned_hand_core::router::SlotRouter;
use seasoned_hand_core::sandbox::SandboxClient;
use seasoned_hand_core::search::{SearchClient, SearchProvider};
use seasoned_hand_core::{db, pubsub};
use seasoned_hand_server::{AppState, app};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

struct StubChannel(&'static str);

#[async_trait]
impl IntakeProvider for StubChannel {
    fn name(&self) -> &'static str {
        self.0
    }
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
impl DeliverySink for StubChannel {
    fn name(&self) -> &'static str {
        self.0
    }
    async fn deliver(
        &self,
        target: &DeliveryTarget,
        _: &Deliverable,
    ) -> Result<DeliveryReceipt, ChannelError> {
        Ok(DeliveryReceipt {
            channel: target.channel.clone(),
            external_id: "ext".into(),
            delivered_at: 0,
            raw_response: serde_json::json!({}),
        })
    }
}

#[async_trait]
impl NotifySink for StubChannel {
    fn name(&self) -> &'static str {
        self.0
    }
    async fn notify(
        &self,
        target: &NotifyTarget,
        _: &NotifyEvent,
    ) -> Result<NotifyReceipt, ChannelError> {
        Ok(NotifyReceipt {
            channel: target.channel.clone(),
            external_id: Some("ntfy-ext".into()),
            sent_at: 0,
            raw_response: serde_json::json!({}),
        })
    }
}

async fn boot_with_channels() -> String {
    let pool = db::open(":memory:").await.unwrap();
    let redis = pubsub::RedisPool::new("redis://127.0.0.1:6").unwrap();
    let sandbox = SandboxClient::new(
        "ghcr.io/agent-infra/sandbox:1.0.0.152",
        std::env::temp_dir(),
    )
    .unwrap();
    let search = SearchClient::new(SearchProvider::Brave { api_key: None });
    let router = SlotRouter::default_for_bifrost();

    // Story 2.10 / DEBT #17: `register_channel` merges on top of the
    // chat baseline that `AppState::new` already registered. The stub
    // entries below stack on that baseline, so the introspection
    // assertions further down account for "chat" + "webhook" + "ntfy".
    let webhook = Arc::new(StubChannel("webhook"));
    let ntfy = Arc::new(StubChannel("ntfy"));
    let state = AppState::new(pool, redis, sandbox, search, router, Default::default())
        .register_channel(
            ChannelRegistration::new("webhook")
                .with_intake(webhook.clone() as Arc<dyn IntakeProvider>)
                .with_delivery(webhook.clone() as Arc<dyn DeliverySink>)
                .with_notify(webhook as Arc<dyn NotifySink>),
        )
        .register_channel(
            ChannelRegistration::new("ntfy").with_notify(ntfy as Arc<dyn NotifySink>),
        );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn get_v1_channels_lists_capabilities() {
    let base = boot_with_channels().await;
    let resp = reqwest::get(format!("{base}/v1/channels")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.unwrap();
    let arr = body.as_array().expect("array");
    // Chat baseline (story 2.9) + webhook + ntfy stubs registered above.
    assert_eq!(arr.len(), 3);

    // Stable sort by name (per ChannelRegistry::health contract):
    // "chat" < "ntfy" < "webhook" alphabetically.
    assert_eq!(arr[0]["name"], "chat");
    assert_eq!(
        arr[0]["capabilities"],
        serde_json::json!(["intake", "delivery"])
    );
    assert_eq!(arr[1]["name"], "ntfy");
    assert_eq!(arr[1]["capabilities"], serde_json::json!(["notify"]));
    assert_eq!(arr[2]["name"], "webhook");
    assert_eq!(
        arr[2]["capabilities"],
        serde_json::json!(["intake", "delivery", "notify"])
    );
}

#[tokio::test]
async fn get_v1_channel_health_returns_one_row() {
    let base = boot_with_channels().await;
    let resp = reqwest::get(format!("{base}/v1/channels/ntfy/health"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["name"], "ntfy");
    assert_eq!(body["capabilities"], serde_json::json!(["notify"]));

    let missing = reqwest::get(format!("{base}/v1/channels/nope/health"))
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn post_v1_channel_test_validates_role() {
    let base = boot_with_channels().await;
    let client = reqwest::Client::new();

    // Channel + role both present → 200 OK.
    let ok = client
        .post(format!("{base}/v1/channels/webhook/test?role=delivery"))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    let body: Value = ok.json().await.unwrap();
    assert_eq!(body["ok"], true);
    assert_eq!(body["role"], "delivery");

    // Channel exists but role not implemented → 404 role_not_implemented.
    let no_role = client
        .post(format!("{base}/v1/channels/ntfy/test?role=intake"))
        .send()
        .await
        .unwrap();
    assert_eq!(no_role.status(), StatusCode::NOT_FOUND);
    let body: Value = no_role.json().await.unwrap();
    assert_eq!(body["error"], "role_not_implemented");

    // Channel doesn't exist → 404 channel_not_found.
    let no_chan = client
        .post(format!("{base}/v1/channels/ghost/test?role=delivery"))
        .send()
        .await
        .unwrap();
    assert_eq!(no_chan.status(), StatusCode::NOT_FOUND);
    let body: Value = no_chan.json().await.unwrap();
    assert_eq!(body["error"], "channel_not_found");

    // Unknown role → 400.
    let bad_role = client
        .post(format!("{base}/v1/channels/webhook/test?role=junk"))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_role.status(), StatusCode::BAD_REQUEST);
}
