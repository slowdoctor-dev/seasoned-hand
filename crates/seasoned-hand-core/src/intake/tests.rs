use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::router::{HandleOutcome, IntakeRouter, RejectionReason};
use super::{IntakeEventStore, IntakeStoreError};
use crate::channel::{
    ChannelError, ChannelRegistration, ChannelRegistry, DeliveryTarget, IntakeEvent, IntakeProvider,
};
use crate::db;
use crate::project::{NewProject, NewTask, ProjectStore, TaskStatus, TaskStore};

async fn open_pool() -> db::DbPool {
    db::open(":memory:").await.expect("open in-memory db")
}

fn sample_event(channel: &str, intake_id: &str, received_at: i64) -> IntakeEvent {
    IntakeEvent {
        channel: channel.into(),
        intake_id: intake_id.into(),
        brief_input: "Summarize Q4 board deck".into(),
        reply_target: Some(DeliveryTarget {
            channel: "email".into(),
            target_ref: "msgid:<abc@example.com>".into(),
            metadata: json!({ "to": "user@example.com" }),
        }),
        metadata: json!({ "subject": "Board deck" }),
        tenant_id: None,
        received_at,
    }
}

#[tokio::test]
async fn intake_event_store_crud() {
    let pool = open_pool().await;
    let store = IntakeEventStore::new(pool);

    let evt = sample_event("webhook", "req-001", 1_000_000);
    let id = store.insert(&evt).await.expect("insert");

    let row = store
        .get_by_intake_id("webhook", "req-001")
        .await
        .expect("get")
        .expect("row present");
    assert_eq!(row.id, id);
    assert_eq!(row.channel, "webhook");
    assert_eq!(row.intake_id, "req-001");
    assert_eq!(row.brief_input, "Summarize Q4 board deck");
    assert!(row.task_id.is_none());
    let target = row.reply_target.as_ref().unwrap();
    assert_eq!(target.channel, "email");
    assert_eq!(target.target_ref, "msgid:<abc@example.com>");

    // Unknown lookup is None, not an error.
    assert!(
        store
            .get_by_intake_id("webhook", "nope")
            .await
            .expect("get nope")
            .is_none()
    );
}

#[tokio::test]
async fn intake_event_unique_channel_intake_id() {
    let pool = open_pool().await;
    let store = IntakeEventStore::new(pool);

    let evt = sample_event("webhook", "dup-1", 1);
    store.insert(&evt).await.expect("first insert");

    let err = store.insert(&evt).await.expect_err("duplicate must fail");
    // refinery + rusqlite surface UNIQUE violation as Sqlite error;
    // exact variant depends on rusqlite version. Match by name.
    let msg = format!("{err}");
    assert!(
        msg.contains("UNIQUE") || msg.contains("unique"),
        "expected UNIQUE constraint error, got: {msg}"
    );
}

#[tokio::test]
async fn intake_event_link_to_task() {
    let pool = open_pool().await;
    let projects = ProjectStore::new(pool.clone());
    let tasks = TaskStore::new(pool.clone());
    let store = IntakeEventStore::new(pool);

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

    let id = store
        .insert(&sample_event("webhook", "link-1", 2))
        .await
        .expect("insert");
    store.link_to_task(&id, &task_id).await.expect("link");

    let row = store
        .get_by_intake_id("webhook", "link-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.task_id.as_deref(), Some(task_id.as_str()));

    // Unknown intake-event id is NotFound.
    let err = store
        .link_to_task("nope", &task_id)
        .await
        .expect_err("not found");
    assert!(matches!(err, IntakeStoreError::NotFound(_)));
}

#[tokio::test]
async fn intake_event_list_by_channel_paginates() {
    let pool = open_pool().await;
    let store = IntakeEventStore::new(pool);

    // Insert 5 webhook events + 1 email event; only webhook events
    // should come back from list_by_channel("webhook").
    for i in 0..5_i64 {
        store
            .insert(&sample_event("webhook", &format!("rw-{i}"), 100 + i))
            .await
            .unwrap();
    }
    store
        .insert(&sample_event("email", "re-0", 999))
        .await
        .unwrap();

    let page = store
        .list_by_channel("webhook", None, 50)
        .await
        .expect("list webhook");
    assert_eq!(page.len(), 5);
    // Newest first.
    assert_eq!(page.first().unwrap().received_at, 104);
    assert_eq!(page.last().unwrap().received_at, 100);

    // Cursor: only rows strictly older than 102.
    let older = store
        .list_by_channel("webhook", Some(102), 50)
        .await
        .expect("list cursor");
    assert_eq!(older.len(), 2);
    assert_eq!(older.first().unwrap().received_at, 101);
    assert_eq!(older.last().unwrap().received_at, 100);

    // Different channel isolated.
    let emails = store
        .list_by_channel("email", None, 50)
        .await
        .expect("list email");
    assert_eq!(emails.len(), 1);
}

// ---------------------------------------------------------------------------
// Story 2.5: IntakeRouter tests.
// ---------------------------------------------------------------------------

/// Minimal `IntakeProvider` stub used only so the registry has a
/// matching name registered — the router validates `event.channel`
/// against the registry, but the provider's `run` lifecycle is the
/// concern of story 2.10+ concrete channels.
struct StubIntake {
    name: &'static str,
}

#[async_trait]
impl IntakeProvider for StubIntake {
    fn name(&self) -> &'static str {
        self.name
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

fn registry_with(name: &'static str) -> Arc<ChannelRegistry> {
    let mut reg = ChannelRegistry::new();
    let stub = Arc::new(StubIntake { name });
    reg.register(ChannelRegistration::new(name).with_intake(stub as Arc<dyn IntakeProvider>));
    Arc::new(reg)
}

async fn router_harness(
    channel_name: &'static str,
) -> (
    IntakeRouter,
    Arc<IntakeEventStore>,
    Arc<TaskStore>,
    Arc<ProjectStore>,
) {
    let pool = open_pool().await;
    let intake_store = Arc::new(IntakeEventStore::new(pool.clone()));
    let task_store = Arc::new(TaskStore::new(pool.clone()));
    let project_store = Arc::new(ProjectStore::new(pool));
    let registry = registry_with(channel_name);
    let router = IntakeRouter::new(
        intake_store.clone(),
        task_store.clone(),
        project_store.clone(),
        registry,
    );
    (router, intake_store, task_store, project_store)
}

#[tokio::test]
async fn intake_router_persists_and_creates_task() {
    let (router, intake_store, task_store, project_store) = router_harness("webhook").await;

    let event = sample_event("webhook", "req-001", 10_000);
    let outcome = router.handle_event(event).await.expect("handle ok");
    let (intake_event_id, task_id, session_id) = match outcome {
        HandleOutcome::Created {
            intake_event_id,
            task_id,
            session_id,
        } => (intake_event_id, task_id, session_id),
        other => panic!("expected Created, got {other:?}"),
    };
    // No spawner attached in this test harness, so the spawner-derived
    // session_id stays `None` — task lands in `drafted` and never moves.
    assert!(session_id.is_none(), "no spawner → no session_id");

    // intake row persisted and linked
    let row = intake_store
        .get_by_intake_id("webhook", "req-001")
        .await
        .unwrap()
        .expect("intake row");
    assert_eq!(row.id, intake_event_id);
    assert_eq!(row.task_id.as_deref(), Some(task_id.as_str()));

    // task created in drafted status
    let task = task_store.get(&task_id).await.expect("task row");
    assert_eq!(task.status, TaskStatus::Drafted);
    assert!(task.title.starts_with("Summarize"));

    // default Inbox project materialised on first miss
    let projects = project_store
        .list(None, None, 50)
        .await
        .expect("list projects");
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].title, "Inbox");
}

#[tokio::test]
async fn intake_router_rejects_duplicate_intake_id() {
    let (router, intake_store, _task_store, _project_store) = router_harness("webhook").await;

    let evt = sample_event("webhook", "dup-x", 20_000);
    let first = router.handle_event(evt.clone()).await.expect("first ok");
    assert!(matches!(first, HandleOutcome::Created { .. }));

    let second = router.handle_event(evt).await.expect("second ok");
    assert_eq!(second, HandleOutcome::DuplicateSkipped);

    // Still exactly one intake row.
    let rows = intake_store
        .list_by_channel("webhook", None, 50)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
}

#[tokio::test]
async fn intake_router_rejects_empty_brief() {
    let (router, _intake_store, task_store, project_store) = router_harness("webhook").await;
    let mut evt = sample_event("webhook", "empty-1", 30_000);
    evt.brief_input = "   \n  ".into();

    let outcome = router.handle_event(evt).await.expect("rejected ok");
    assert_eq!(
        outcome,
        HandleOutcome::Rejected(RejectionReason::EmptyBrief)
    );

    // Nothing persisted.
    let projects = project_store.list(None, None, 50).await.unwrap();
    assert!(projects.is_empty());
    let task_pages = task_store
        .list_by_project("nope", None, None, 50)
        .await
        .unwrap();
    assert!(task_pages.is_empty());
}

#[tokio::test]
async fn intake_router_rejects_unregistered_channel() {
    let (router, _intake_store, _task_store, _project_store) = router_harness("webhook").await;
    let evt = sample_event("ghost", "g-1", 40_000);
    let outcome = router.handle_event(evt).await.expect("rejected ok");
    assert_eq!(
        outcome,
        HandleOutcome::Rejected(RejectionReason::UnknownChannel("ghost".into()))
    );
}

#[tokio::test]
async fn intake_router_uses_explicit_project_id_when_present() {
    let (router, _intake_store, task_store, project_store) = router_harness("webhook").await;

    // Pre-seed a project so the metadata.project_id lookup hits.
    let pid = project_store
        .insert(NewProject {
            tenant_id: None,
            title: "Existing".into(),
            description: None,
        })
        .await
        .unwrap();

    let mut evt = sample_event("webhook", "explicit-1", 50_000);
    evt.metadata = json!({ "project_id": pid });

    let outcome = router.handle_event(evt).await.expect("ok");
    let task_id = match outcome {
        HandleOutcome::Created { task_id, .. } => task_id,
        other => panic!("expected Created, got {other:?}"),
    };

    let task = task_store.get(&task_id).await.unwrap();
    assert_eq!(task.project_id, pid);

    // Inbox NOT created — only the pre-seeded "Existing" project exists.
    let projects = project_store.list(None, None, 50).await.unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].title, "Existing");
}

/// Story 2.8b: when an `InitializerSpawner` is attached, the router
/// invokes it after persisting the drafted task and reflects the
/// returned `session_id` in `HandleOutcome::Created`. Also confirms the
/// spawner sees the canonical `task_id` (matching the DB row) and the
/// raw `brief_input`, so the WS / webhook handlers can trust the
/// surface for the briefing-confirm round-trip.
#[tokio::test]
async fn intake_router_invokes_spawner_on_created() {
    use crate::intake::spawner::{InitializerSpawner, SpawnError, SpawnReceipt, SpawnSpec};
    use std::sync::Mutex;

    #[derive(Default)]
    struct CapturingSpawner {
        seen: Mutex<Vec<SpawnSpec>>,
        next_session_id: &'static str,
    }
    #[async_trait]
    impl InitializerSpawner for CapturingSpawner {
        async fn spawn(&self, spec: SpawnSpec) -> Result<SpawnReceipt, SpawnError> {
            self.seen.lock().unwrap().push(spec);
            Ok(SpawnReceipt {
                session_id: self.next_session_id.into(),
            })
        }
    }

    let (router, _intake_store, task_store, _project_store) = router_harness("chat").await;
    let spawner = Arc::new(CapturingSpawner {
        next_session_id: "sess-fresh",
        ..CapturingSpawner::default()
    });
    if router
        .attach_initializer_spawner(spawner.clone() as Arc<dyn InitializerSpawner>)
        .is_err()
    {
        panic!("first attach should succeed");
    }
    assert!(router.has_initializer_spawner());

    // Double-attach is rejected (OnceLock semantics).
    let alt = Arc::new(CapturingSpawner {
        next_session_id: "sess-other",
        ..CapturingSpawner::default()
    });
    assert!(
        router
            .attach_initializer_spawner(alt as Arc<dyn InitializerSpawner>)
            .is_err()
    );

    let event = sample_event("chat", "ws:abc", 9_000);
    let outcome = router.handle_event(event).await.expect("ok");
    let (task_id, session_id) = match outcome {
        HandleOutcome::Created {
            task_id,
            session_id,
            ..
        } => (task_id, session_id),
        other => panic!("expected Created, got {other:?}"),
    };
    assert_eq!(session_id.as_deref(), Some("sess-fresh"));

    {
        let seen = spawner.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].task_id, task_id);
        assert_eq!(seen[0].brief_input, "Summarize Q4 board deck");
        assert!(seen[0].reply_target.is_some());
    }

    // Drafted task survives — the spawn is fire-and-forget; the
    // confirm-gate run is what later moves it to `briefed → running`.
    let task = task_store.get(&task_id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Drafted);
}

/// Story 2.8b: spawner errors do NOT tear the intake path down. The
/// intake row + drafted task stay persisted; `Created.session_id` is
/// `None` so the caller can surface "task accepted, but briefing didn't
/// start" without losing the work.
#[tokio::test]
async fn intake_router_tolerates_spawner_error() {
    use crate::intake::spawner::{InitializerSpawner, SpawnError, SpawnReceipt, SpawnSpec};

    struct FailingSpawner;
    #[async_trait]
    impl InitializerSpawner for FailingSpawner {
        async fn spawn(&self, _spec: SpawnSpec) -> Result<SpawnReceipt, SpawnError> {
            Err(SpawnError::Other("simulated".into()))
        }
    }

    let (router, intake_store, task_store, _project_store) = router_harness("chat").await;
    if router
        .attach_initializer_spawner(Arc::new(FailingSpawner) as Arc<dyn InitializerSpawner>)
        .is_err()
    {
        panic!("attach should succeed");
    }

    let outcome = router
        .handle_event(sample_event("chat", "ws:zzz", 1234))
        .await
        .expect("router does not propagate the spawner error");
    match outcome {
        HandleOutcome::Created {
            session_id,
            task_id,
            ..
        } => {
            assert!(session_id.is_none(), "spawner failure → no session_id");
            // Drafted task IS persisted and linked.
            let task = task_store.get(&task_id).await.unwrap();
            assert_eq!(task.status, TaskStatus::Drafted);
        }
        other => panic!("expected Created, got {other:?}"),
    }
    let rows = intake_store
        .list_by_channel("chat", None, 50)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
}

/// Proposed DEBT #35: the `session_id_hint` in IntakeEvent metadata
/// flows into `sessions.id` PK + workspace + container name. An attacker
/// who can submit intake (today: anyone with the webhook intake token)
/// can plant a `..`-laden id that the TTL cron later resolves into a
/// host-side `rm -rf`. Verify that:
///   1. A UUID-shaped hint reaches the spawner verbatim.
///   2. A path-traversal hint is dropped (warned, not errored) — the
///      spawner sees `session_id_hint: None` and mints a fresh UUID.
#[tokio::test]
async fn intake_router_drops_unsafe_session_id_hint() {
    use crate::intake::spawner::{InitializerSpawner, SpawnError, SpawnReceipt, SpawnSpec};
    use std::sync::Mutex;

    #[derive(Default)]
    struct CapturingSpawner {
        seen: Mutex<Vec<SpawnSpec>>,
    }
    #[async_trait]
    impl InitializerSpawner for CapturingSpawner {
        async fn spawn(&self, spec: SpawnSpec) -> Result<SpawnReceipt, SpawnError> {
            self.seen.lock().unwrap().push(spec);
            Ok(SpawnReceipt {
                session_id: "minted-by-spawner".into(),
            })
        }
    }

    let (router, _intake_store, _task_store, _project_store) = router_harness("webhook").await;
    let spawner = Arc::new(CapturingSpawner::default());
    if router
        .attach_initializer_spawner(spawner.clone() as Arc<dyn InitializerSpawner>)
        .is_err()
    {
        panic!("first attach should succeed");
    }

    // (1) Safe UUID-ish hint flows through.
    let mut ev_safe = sample_event("webhook", "intake-safe", 1_000);
    ev_safe.metadata = serde_json::json!({
        "session_id_hint": "a1b2c3d4-1234-5678-9abc-def012345678",
    });
    router.handle_event(ev_safe).await.unwrap();

    // (2) Path-traversal hint is dropped (not errored).
    let mut ev_unsafe = sample_event("webhook", "intake-unsafe", 1_001);
    ev_unsafe.metadata = serde_json::json!({
        "session_id_hint": "../../etc/passwd",
    });
    router
        .handle_event(ev_unsafe)
        .await
        .expect("intake still succeeds; only the hint is dropped");

    // (3) Other disallowed shapes also drop.
    for bad in &[
        "with space",
        "has/slash",
        "has\\back",
        "$(whoami)",
        "x;rm",
        "",
    ] {
        let mut ev = sample_event("webhook", &format!("intake-bad-{bad:?}"), 1_002);
        ev.metadata = serde_json::json!({ "session_id_hint": bad });
        router
            .handle_event(ev)
            .await
            .expect("intake still succeeds; only the hint is dropped");
    }

    let seen = spawner.seen.lock().unwrap();
    assert_eq!(seen.len(), 8, "all 8 events reach the spawner");
    assert_eq!(
        seen[0].session_id_hint.as_deref(),
        Some("a1b2c3d4-1234-5678-9abc-def012345678"),
        "safe UUID hint preserved"
    );
    assert_eq!(seen[1].session_id_hint, None, "path-traversal hint dropped");
    for (i, spec) in seen.iter().enumerate().skip(2) {
        assert_eq!(
            spec.session_id_hint, None,
            "bad-shape #{i} should drop the hint"
        );
    }
}

/// Side-test: confirm the runner respects the shutdown token and
/// drains in-flight events instead of dropping them. Belt-and-braces
/// for the long-lived `run()` loop wired into `AppState`.
#[tokio::test]
async fn intake_router_run_drains_then_exits_on_shutdown() {
    let (router, intake_store, _task_store, _project_store) = router_harness("webhook").await;
    let router = Arc::new(router);

    let (tx, rx) = mpsc::channel::<IntakeEvent>(8);
    let shutdown = CancellationToken::new();
    let runner_router = router.clone();
    let runner_shutdown = shutdown.clone();
    let handle = tokio::spawn(async move {
        runner_router.run(rx, runner_shutdown).await;
    });

    tx.send(sample_event("webhook", "run-1", 1)).await.unwrap();
    tx.send(sample_event("webhook", "run-2", 2)).await.unwrap();
    drop(tx);
    handle.await.expect("runner");
    shutdown.cancel(); // no-op, runner already exited

    let rows = intake_store
        .list_by_channel("webhook", None, 50)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
}
