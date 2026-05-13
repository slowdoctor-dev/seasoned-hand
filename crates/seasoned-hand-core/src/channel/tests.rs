use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{
    ChannelError, ChannelRegistration, ChannelRegistry, Deliverable, DeliveryReceipt, DeliverySink,
    DeliveryTarget, IntakeEvent, IntakeProvider, NotifyEvent, NotifyReceipt, NotifySink,
    NotifyTarget,
};

/// Three-role mock channel: instruments each trait method with an
/// `AtomicUsize` counter so tests can assert which role(s) the
/// registry routes to. The same struct registered three different
/// ways exercises every dispatch path.
struct TestChannel {
    name: &'static str,
    intake_run_calls: AtomicUsize,
    delivery_calls: AtomicUsize,
    notify_calls: AtomicUsize,
}

impl TestChannel {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            intake_run_calls: AtomicUsize::new(0),
            delivery_calls: AtomicUsize::new(0),
            notify_calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl IntakeProvider for TestChannel {
    fn name(&self) -> &'static str {
        self.name
    }

    async fn run(
        &self,
        _sink: mpsc::Sender<IntakeEvent>,
        shutdown: CancellationToken,
    ) -> Result<(), ChannelError> {
        self.intake_run_calls.fetch_add(1, Ordering::SeqCst);
        // Long-lived: block until the registry drops the token.
        shutdown.cancelled().await;
        Ok(())
    }
}

#[async_trait]
impl DeliverySink for TestChannel {
    fn name(&self) -> &'static str {
        self.name
    }

    async fn deliver(
        &self,
        target: &DeliveryTarget,
        _deliverable: &Deliverable,
    ) -> Result<DeliveryReceipt, ChannelError> {
        self.delivery_calls.fetch_add(1, Ordering::SeqCst);
        Ok(DeliveryReceipt {
            channel: target.channel.clone(),
            external_id: "test-ext-delivery".into(),
            delivered_at: 0,
            raw_response: json!({}),
        })
    }
}

#[async_trait]
impl NotifySink for TestChannel {
    fn name(&self) -> &'static str {
        self.name
    }

    async fn notify(
        &self,
        target: &NotifyTarget,
        _event: &NotifyEvent,
    ) -> Result<NotifyReceipt, ChannelError> {
        self.notify_calls.fetch_add(1, Ordering::SeqCst);
        Ok(NotifyReceipt {
            channel: target.channel.clone(),
            external_id: Some("test-ext-notify".into()),
            sent_at: 0,
            raw_response: json!({}),
        })
    }
}

fn delivery_target(name: &str) -> DeliveryTarget {
    DeliveryTarget {
        channel: name.into(),
        target_ref: "test-ref".into(),
        metadata: json!({}),
    }
}

fn notify_target(name: &str) -> NotifyTarget {
    NotifyTarget {
        channel: name.into(),
        target_ref: "test-ref".into(),
        metadata: json!({}),
    }
}

fn sample_deliverable() -> Deliverable {
    Deliverable {
        id: "deliv-1".into(),
        task_id: "task-1".into(),
        tenant_id: None,
        format: "md".into(),
        rendered_content_path: "/workspace/.deliverables/deliv-1.md".into(),
        rendered_content_sha256: "0".repeat(64),
        content_size: 0,
        provenance_manifest: json!({}),
    }
}

fn sample_notify_event() -> NotifyEvent {
    NotifyEvent {
        trigger_kind: "test_kind".into(),
        task_id: Some("task-1".into()),
        payload: json!({}),
    }
}

#[tokio::test]
async fn registry_roundtrip_three_roles() {
    let mut registry = ChannelRegistry::new();
    let ch = Arc::new(TestChannel::new("multi"));

    registry.register(
        ChannelRegistration::new("multi")
            .with_intake(ch.clone() as Arc<dyn IntakeProvider>)
            .with_delivery(ch.clone() as Arc<dyn DeliverySink>)
            .with_notify(ch.clone() as Arc<dyn NotifySink>),
    );

    let intake = registry.get_intake("multi").expect("intake registered");
    let delivery = registry.get_delivery("multi").expect("delivery registered");
    let notify = registry.get_notify("multi").expect("notify registered");

    assert_eq!(<dyn IntakeProvider>::name(&*intake), "multi");
    assert_eq!(<dyn DeliverySink>::name(&*delivery), "multi");
    assert_eq!(<dyn NotifySink>::name(&*notify), "multi");

    // Exercise the delivery + notify call paths and confirm dispatch
    // increments the per-role counters on the underlying TestChannel.
    delivery
        .deliver(&delivery_target("multi"), &sample_deliverable())
        .await
        .expect("deliver");
    notify
        .notify(&notify_target("multi"), &sample_notify_event())
        .await
        .expect("notify");
    assert_eq!(ch.delivery_calls.load(Ordering::SeqCst), 1);
    assert_eq!(ch.notify_calls.load(Ordering::SeqCst), 1);

    // Health snapshot reports all three caps in stable order.
    let health = registry.health();
    assert_eq!(health.len(), 1);
    assert_eq!(health[0].name, "multi");
    assert_eq!(health[0].capabilities, vec!["intake", "delivery", "notify"]);
}

#[tokio::test]
async fn registry_intake_only_channel() {
    let mut registry = ChannelRegistry::new();
    let ch = Arc::new(TestChannel::new("ingest"));

    registry
        .register(ChannelRegistration::new("ingest").with_intake(ch as Arc<dyn IntakeProvider>));

    assert!(registry.get_intake("ingest").is_some());
    assert!(registry.get_delivery("ingest").is_none());
    assert!(registry.get_notify("ingest").is_none());

    let health = registry.health();
    assert_eq!(health.len(), 1);
    assert_eq!(health[0].capabilities, vec!["intake"]);
}

#[tokio::test]
async fn registry_lookup_returns_none_for_missing_role() {
    let mut registry = ChannelRegistry::new();
    let ch = Arc::new(TestChannel::new("deliver-only"));

    registry.register(
        ChannelRegistration::new("deliver-only").with_delivery(ch as Arc<dyn DeliverySink>),
    );

    // Registered, but no intake/notify role — every other lookup
    // returns None, distinct from "channel doesn't exist at all".
    assert!(registry.get_intake("deliver-only").is_none());
    assert!(registry.get_notify("deliver-only").is_none());
    assert!(registry.get_delivery("deliver-only").is_some());

    // Unregistered channel returns None across every role.
    assert!(registry.get_intake("nope").is_none());
    assert!(registry.get_delivery("nope").is_none());
    assert!(registry.get_notify("nope").is_none());
}

#[tokio::test]
async fn registry_iter_intake_yields_only_intake_channels() {
    let mut registry = ChannelRegistry::new();

    let a = Arc::new(TestChannel::new("a"));
    let b = Arc::new(TestChannel::new("b"));
    let c = Arc::new(TestChannel::new("c"));

    // a: intake-only. b: delivery-only. c: intake + notify.
    registry.register(ChannelRegistration::new("a").with_intake(a as Arc<dyn IntakeProvider>));
    registry.register(ChannelRegistration::new("b").with_delivery(b as Arc<dyn DeliverySink>));
    registry.register(
        ChannelRegistration::new("c")
            .with_intake(c.clone() as Arc<dyn IntakeProvider>)
            .with_notify(c as Arc<dyn NotifySink>),
    );

    let mut intake_names: Vec<&str> = registry.iter_intake().map(|(name, _)| name).collect();
    intake_names.sort();
    assert_eq!(intake_names, vec!["a", "c"]);

    let mut delivery_names: Vec<&str> = registry.iter_delivery().map(|(name, _)| name).collect();
    delivery_names.sort();
    assert_eq!(delivery_names, vec!["b"]);

    let mut notify_names: Vec<&str> = registry.iter_notify().map(|(name, _)| name).collect();
    notify_names.sort();
    assert_eq!(notify_names, vec!["c"]);
}

#[tokio::test]
async fn spawn_intakes_returns_one_handle_per_intake_provider() {
    let mut registry = ChannelRegistry::new();

    let a = Arc::new(TestChannel::new("a"));
    let b = Arc::new(TestChannel::new("b"));
    let c = Arc::new(TestChannel::new("c"));

    // Two intake providers (a, c) + one delivery-only (b). The
    // delivery-only channel must NOT be spawned.
    registry.register(ChannelRegistration::new("a").with_intake(a as Arc<dyn IntakeProvider>));
    registry.register(ChannelRegistration::new("b").with_delivery(b as Arc<dyn DeliverySink>));
    registry.register(ChannelRegistration::new("c").with_intake(c as Arc<dyn IntakeProvider>));

    let (sink, _rx) = mpsc::channel::<IntakeEvent>(16);
    let shutdown = CancellationToken::new();
    let handles = registry.spawn_intakes(sink, shutdown.clone());

    // The contract is handle count = intake-provider count; long-lived
    // tasks are NOT required to complete during the assertion.
    assert_eq!(handles.len(), 2);

    // Cleanup: cancel the token so the TestChannel `run` loops exit
    // and the harness doesn't leak background work between tests.
    shutdown.cancel();
    for handle in handles {
        handle
            .await
            .expect("intake task panicked")
            .expect("intake task returned err");
    }
}
