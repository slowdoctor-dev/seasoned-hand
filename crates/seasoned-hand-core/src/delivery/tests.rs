use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;

use super::router::{DeliveryRouter, DeliveryRouterError};
use super::{DeliveryEventStore, NewDeliveryEvent};
use crate::channel::{
    ChannelError, ChannelRegistration, ChannelRegistry, Deliverable, DeliveryReceipt, DeliverySink,
    DeliveryTarget, IntakeEvent,
};
use crate::db;
use crate::deliverable::{DeliverableStore, NewDeliverable};
use crate::events::sqlite::SqliteEventStore;
use crate::intake::IntakeEventStore;
use crate::project::{NewProject, NewTask, ProjectStore, TaskStore};

async fn seed_task_and_deliverable(pool: &db::DbPool) -> (String, String) {
    let projects = ProjectStore::new(pool.clone());
    let tasks = TaskStore::new(pool.clone());
    let deliverables = DeliverableStore::new(pool.clone());
    let pid = projects
        .insert(NewProject {
            tenant_id: None,
            title: "P".into(),
            description: None,
        })
        .await
        .unwrap();
    let task_id = tasks
        .insert(NewTask {
            project_id: pid,
            tenant_id: None,
            title: "T".into(),
            expected_due_at: None,
        })
        .await
        .unwrap();
    let did = deliverables
        .insert(NewDeliverable {
            task_id: task_id.clone(),
            tenant_id: None,
            format: "md".into(),
            source_content_path: None,
            source_content_sha256: None,
            rendered_content_path: "/workspace/.deliverables/d.md".into(),
            rendered_content_sha256: "b".repeat(64),
            content_size: 0,
            citations: None,
            provenance_manifest: json!({}),
        })
        .await
        .unwrap();
    (task_id, did)
}

fn target(channel: &str) -> DeliveryTarget {
    DeliveryTarget {
        channel: channel.into(),
        target_ref: "url:https://example.com/cb".into(),
        metadata: json!({}),
    }
}

#[tokio::test]
async fn delivery_event_store_crud() {
    let pool = db::open(":memory:").await.unwrap();
    let (task_id, did) = seed_task_and_deliverable(&pool).await;
    let store = DeliveryEventStore::new(pool);

    let ok_id = store
        .insert(NewDeliveryEvent {
            tenant_id: None,
            task_id: task_id.clone(),
            deliverable_id: did.clone(),
            channel: "webhook".into(),
            target: target("webhook"),
            ok: true,
            external_id: Some("ext-1".into()),
            error: None,
            delivered_at: 100,
        })
        .await
        .expect("insert ok");

    let fail_id = store
        .insert(NewDeliveryEvent {
            tenant_id: None,
            task_id: task_id.clone(),
            deliverable_id: did.clone(),
            channel: "webhook".into(),
            target: target("webhook"),
            ok: false,
            external_id: None,
            error: Some("connection refused".into()),
            delivered_at: 200,
        })
        .await
        .expect("insert fail");

    let by_task = store.list_by_task(&task_id).await.expect("by task");
    assert_eq!(by_task.len(), 2);
    assert_eq!(by_task[0].id, ok_id);
    assert!(by_task[0].ok);
    assert_eq!(by_task[0].external_id.as_deref(), Some("ext-1"));
    assert_eq!(by_task[1].id, fail_id);
    assert!(!by_task[1].ok);
    assert_eq!(by_task[1].error.as_deref(), Some("connection refused"));

    let by_deliv = store.list_by_deliverable(&did).await.expect("by deliv");
    assert_eq!(by_deliv.len(), 2);
    assert_eq!(by_deliv[0].deliverable_id, did);
}

// ---------------------------------------------------------------------------
// Story 2.5: DeliveryRouter tests.
// ---------------------------------------------------------------------------

/// Scripted `DeliverySink` that replays a fixed sequence of outcomes
/// across successive `deliver` calls and counts invocations. Each
/// outcome is consumed in order; if calls exceed the script the last
/// outcome is treated as a hard panic (test bug).
struct ScriptedSink {
    name: &'static str,
    script: Mutex<Vec<Result<DeliveryReceipt, ChannelError>>>,
    calls: AtomicUsize,
}

impl ScriptedSink {
    fn new(name: &'static str, outcomes: Vec<Result<DeliveryReceipt, ChannelError>>) -> Self {
        Self {
            name,
            script: Mutex::new(outcomes),
            calls: AtomicUsize::new(0),
        }
    }
    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl DeliverySink for ScriptedSink {
    fn name(&self) -> &'static str {
        self.name
    }
    async fn deliver(
        &self,
        _target: &DeliveryTarget,
        _deliverable: &Deliverable,
    ) -> Result<DeliveryReceipt, ChannelError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut script = self.script.lock().expect("ScriptedSink mutex poisoned");
        script
            .pop()
            .expect("ScriptedSink: more deliver() calls than scripted outcomes")
    }
}

/// Reverse-helper: outcomes are popped LIFO, so the test caller hands
/// in `[last_call, ..., first_call]`. This wrapper takes "first → last"
/// order to keep the test bodies readable.
fn script(
    outcomes: Vec<Result<DeliveryReceipt, ChannelError>>,
) -> Vec<Result<DeliveryReceipt, ChannelError>> {
    let mut v = outcomes;
    v.reverse();
    v
}

fn ok_receipt(channel: &str, ext: &str) -> DeliveryReceipt {
    DeliveryReceipt {
        channel: channel.into(),
        external_id: ext.into(),
        delivered_at: 1_700_000_000_000_000,
        raw_response: json!({}),
    }
}

struct RouterHarness {
    router: DeliveryRouter,
    delivery_store: Arc<DeliveryEventStore>,
    task_id: String,
    deliverable_id: String,
    channel_name: &'static str,
}

async fn build_router(sink: Arc<dyn DeliverySink>, channel_name: &'static str) -> RouterHarness {
    let pool = db::open(":memory:").await.unwrap();
    let projects = ProjectStore::new(pool.clone());
    let tasks = TaskStore::new(pool.clone());
    let deliverables = Arc::new(DeliverableStore::new(pool.clone()));
    let intake = Arc::new(IntakeEventStore::new(pool.clone()));
    let delivery_store = Arc::new(DeliveryEventStore::new(pool.clone()));
    let events = Arc::new(SqliteEventStore::new(pool.clone()));

    let pid = projects
        .insert(NewProject {
            tenant_id: None,
            title: "P".into(),
            description: None,
        })
        .await
        .unwrap();
    let task_id = tasks
        .insert(NewTask {
            project_id: pid,
            tenant_id: None,
            title: "T".into(),
            expected_due_at: None,
        })
        .await
        .unwrap();
    let did = deliverables
        .insert(NewDeliverable {
            task_id: task_id.clone(),
            tenant_id: None,
            format: "md".into(),
            source_content_path: None,
            source_content_sha256: None,
            rendered_content_path: "/workspace/.deliverables/d.md".into(),
            rendered_content_sha256: "c".repeat(64),
            content_size: 0,
            citations: None,
            provenance_manifest: json!({}),
        })
        .await
        .unwrap();

    // Intake row carrying the reply_target the router resolves.
    intake
        .insert(&IntakeEvent {
            channel: "webhook".into(),
            intake_id: "ext-1".into(),
            brief_input: "deliver me".into(),
            reply_target: Some(DeliveryTarget {
                channel: channel_name.into(),
                target_ref: "url:https://example.com/cb".into(),
                metadata: json!({}),
            }),
            metadata: json!({}),
            tenant_id: None,
            received_at: 1,
        })
        .await
        .unwrap();
    let intake_row = intake
        .get_by_intake_id("webhook", "ext-1")
        .await
        .unwrap()
        .unwrap();
    intake.link_to_task(&intake_row.id, &task_id).await.unwrap();

    let mut registry = ChannelRegistry::new();
    registry.register(ChannelRegistration::new(channel_name).with_delivery(sink));
    let registry = Arc::new(registry);

    let router = DeliveryRouter::new(
        registry,
        delivery_store.clone(),
        deliverables,
        intake,
        events,
        pool,
    )
    .with_retry_delay(Duration::ZERO);

    RouterHarness {
        router,
        delivery_store,
        task_id,
        deliverable_id: did,
        channel_name,
    }
}

#[tokio::test]
async fn delivery_router_dispatches_to_correct_channel() {
    let sink = Arc::new(ScriptedSink::new(
        "webhook-out",
        script(vec![Ok(ok_receipt("webhook-out", "msg-1"))]),
    ));
    let h = build_router(sink.clone() as Arc<dyn DeliverySink>, "webhook-out").await;

    let row = h.router.deliver_task(&h.task_id).await.expect("delivered");
    assert!(row.ok);
    assert_eq!(row.channel, "webhook-out");
    assert_eq!(row.external_id.as_deref(), Some("msg-1"));
    assert_eq!(row.task_id, h.task_id);
    assert_eq!(row.deliverable_id, h.deliverable_id);
    assert_eq!(sink.call_count(), 1);

    // One persisted success row, no extras.
    let rows = h
        .delivery_store
        .list_by_task(&h.task_id)
        .await
        .expect("list");
    assert_eq!(rows.len(), 1);
    assert!(rows[0].ok);
    let _ = h.channel_name; // silence unused warn
}

#[tokio::test]
async fn delivery_router_retries_5xx_once() {
    let sink = Arc::new(ScriptedSink::new(
        "webhook-out",
        script(vec![
            Err(ChannelError::Http("502 bad gateway".into())),
            Ok(ok_receipt("webhook-out", "msg-2")),
        ]),
    ));
    let h = build_router(sink.clone() as Arc<dyn DeliverySink>, "webhook-out").await;

    let row = h
        .router
        .deliver_task(&h.task_id)
        .await
        .expect("delivered after retry");
    assert!(row.ok);
    assert_eq!(row.external_id.as_deref(), Some("msg-2"));
    assert_eq!(sink.call_count(), 2);

    // Only the success row is persisted — the in-flight retry doesn't
    // emit an intermediate delivery_events row.
    let rows = h.delivery_store.list_by_task(&h.task_id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].ok);
}

#[tokio::test]
async fn delivery_router_no_retry_on_decode_error() {
    let sink = Arc::new(ScriptedSink::new(
        "webhook-out",
        script(vec![Err(ChannelError::Decode("garbage response".into()))]),
    ));
    let h = build_router(sink.clone() as Arc<dyn DeliverySink>, "webhook-out").await;

    let err = h
        .router
        .deliver_task(&h.task_id)
        .await
        .expect_err("decode is terminal");
    assert!(matches!(err, DeliveryRouterError::Terminal(_)));
    assert_eq!(sink.call_count(), 1);

    // Exactly one persisted row with ok = false + the error text.
    let rows = h.delivery_store.list_by_task(&h.task_id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].ok);
    assert!(rows[0].error.as_deref().unwrap().contains("garbage"));
}

#[tokio::test]
async fn delivery_router_no_retry_on_4xx_remote_reject() {
    let sink = Arc::new(ScriptedSink::new(
        "webhook-out",
        script(vec![Err(ChannelError::RemoteRejected {
            status: 422,
            message: "schema_violation".into(),
        })]),
    ));
    let h = build_router(sink.clone() as Arc<dyn DeliverySink>, "webhook-out").await;

    let err = h
        .router
        .deliver_task(&h.task_id)
        .await
        .expect_err("4xx is terminal");
    assert!(matches!(err, DeliveryRouterError::Terminal(_)));
    assert_eq!(sink.call_count(), 1);
    let rows = h.delivery_store.list_by_task(&h.task_id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].ok);
}

#[tokio::test]
async fn delivery_router_missing_sink_returns_error() {
    // Empty registry — channel name from intake doesn't resolve.
    let pool = db::open(":memory:").await.unwrap();
    let projects = ProjectStore::new(pool.clone());
    let tasks = TaskStore::new(pool.clone());
    let deliverables = Arc::new(DeliverableStore::new(pool.clone()));
    let intake = Arc::new(IntakeEventStore::new(pool.clone()));
    let delivery_store = Arc::new(DeliveryEventStore::new(pool.clone()));
    let events = Arc::new(SqliteEventStore::new(pool.clone()));
    let pid = projects
        .insert(NewProject {
            tenant_id: None,
            title: "P".into(),
            description: None,
        })
        .await
        .unwrap();
    let task_id = tasks
        .insert(NewTask {
            project_id: pid,
            tenant_id: None,
            title: "T".into(),
            expected_due_at: None,
        })
        .await
        .unwrap();
    deliverables
        .insert(NewDeliverable {
            task_id: task_id.clone(),
            tenant_id: None,
            format: "md".into(),
            source_content_path: None,
            source_content_sha256: None,
            rendered_content_path: "/workspace/.deliverables/d.md".into(),
            rendered_content_sha256: "c".repeat(64),
            content_size: 0,
            citations: None,
            provenance_manifest: json!({}),
        })
        .await
        .unwrap();
    intake
        .insert(&IntakeEvent {
            channel: "webhook".into(),
            intake_id: "ext-1".into(),
            brief_input: "x".into(),
            reply_target: Some(DeliveryTarget {
                channel: "ghost".into(),
                target_ref: "x".into(),
                metadata: json!({}),
            }),
            metadata: json!({}),
            tenant_id: None,
            received_at: 1,
        })
        .await
        .unwrap();
    let irow = intake
        .get_by_intake_id("webhook", "ext-1")
        .await
        .unwrap()
        .unwrap();
    intake.link_to_task(&irow.id, &task_id).await.unwrap();

    let registry = Arc::new(ChannelRegistry::new());
    let router = DeliveryRouter::new(registry, delivery_store, deliverables, intake, events, pool);

    let err = router
        .deliver_task(&task_id)
        .await
        .expect_err("missing sink");
    assert!(matches!(err, DeliveryRouterError::SinkMissing(name) if name == "ghost"));
}
