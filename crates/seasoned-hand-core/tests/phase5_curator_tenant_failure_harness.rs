//! Phase 5 story 5.28 (part 2) — `phase5_curator_tenant_failure_harness`.
//!
//! Asserts all three F-5.14 failure categories emit the right Misc
//! event with the right `failure_category` discriminator:
//!
//! 1. `curator_cycle_refused` + `failure_category: "tenant_unresolved"`
//!    when the worker's `AuthContext` carries an empty tenant_id.
//! 2. `curator_decision_quarantined` + `failure_category: "tenant_unresolved"`
//!    when a decision row references a revision with empty tenant_id.
//! 3. `curator_decision_quarantined` + `failure_category: "cross_tenant_ref"`
//!    when a decision references a revision in a different tenant.
//!
//! For (2) and (3), the harness uses a synthetic `CuratorCycleExecutor`
//! that returns a `CuratorCycleResult` with pre-seeded quarantines —
//! this exercises the worker's quarantine-event emission code path
//! deterministically without depending on the production executor's
//! exact decision-construction sequence.
//!
//! refs: /specs/phase-5/stories/story-5.28.md
//! refs: /specs/phase-5/architecture.md §15 harness 7
//! refs: /specs/phase-5/requirements.md F-5.14

use async_trait::async_trait;
use rusqlite::params;
use seasoned_hand_core::curator::{
    BacklogProbe, CuratorConfig, CuratorCycleExecutor, CuratorCycleResult, CuratorFailureCategory,
    CuratorQuarantineRecord, CuratorTrigger, CuratorWorkerError, NoopCycleExecutor,
    ProductionCuratorWorker,
};
use seasoned_hand_core::db::{self, DbPool};
use seasoned_hand_core::events::sqlite::SqliteEventStore;
use seasoned_hand_core::events::{EventQuery, EventStore, EventType};
use std::sync::Arc;

struct StaticBacklog;

#[async_trait]
impl BacklogProbe for StaticBacklog {
    async fn pending_count(&self, _project_id: &str) -> Result<u32, CuratorWorkerError> {
        Ok(0)
    }
}

/// Executor that pre-seeds the cycle result with a quarantine of the
/// requested category. Lets the harness exercise the
/// `curator_decision_quarantined` emission path without needing the
/// production executor's decision-construction prereqs.
struct QuarantinedExecutor {
    category: CuratorFailureCategory,
    detail: &'static str,
}

#[async_trait]
impl CuratorCycleExecutor for QuarantinedExecutor {
    async fn execute(
        &self,
        project_id: &str,
        _trigger: CuratorTrigger,
        _backlog_count: u32,
    ) -> Result<CuratorCycleResult, CuratorWorkerError> {
        Ok(CuratorCycleResult {
            cycle_id: format!("cycle-test-{}", uuid::Uuid::new_v4()),
            project_id: project_id.to_string(),
            decisions_total: 1,
            queued_for_review: 0,
            failures: 1,
            elapsed_ms: 0,
            quarantines: vec![CuratorQuarantineRecord {
                decision_id: "dec-test-1".to_string(),
                failure_category: self.category,
                retry_count: 0,
                detail: self.detail.to_string(),
            }],
            budget_circuit_open: false,
            budget_month_tokens: 0,
            budget_pct_of_total: 0.0,
            retrospective_refused_reason: None,
        })
    }
}

async fn seed_project(db: &DbPool, id: &str, tenant_id: &str) {
    let id = id.to_string();
    let tenant_id = tenant_id.to_string();
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO projects (id, tenant_id, title, description, status,
                                    created_at, updated_at)
             VALUES (?1, ?2, 'p', NULL, 'active', 1, 1)",
            params![id, tenant_id],
        )
        .unwrap();
    })
    .await;
}

async fn run_worker_and_get_misc_events(
    db: DbPool,
    events: Arc<SqliteEventStore>,
    project_id: &str,
    executor: Arc<dyn CuratorCycleExecutor>,
) -> Result<Vec<seasoned_hand_core::events::Event>, CuratorWorkerError> {
    let config = CuratorConfig {
        enabled: true,
        project_id: project_id.to_string(),
        org_aggregation_enabled: false,
        ..CuratorConfig::default()
    };
    let worker = ProductionCuratorWorker::new(
        config,
        db,
        events.clone(),
        Arc::new(StaticBacklog),
        executor,
    );
    // `run_once` may return Err on the cycle-refused path; either way
    // the Misc events get appended. Swallow and return the events.
    let _ = worker.run_once(CuratorTrigger::Manual, 0).await;
    let session_id = format!("curator:{project_id}");
    let evs = events
        .query(
            &session_id,
            EventQuery {
                event_type: Some(EventType::Misc),
                after_id: None,
                limit: Some(50),
            },
        )
        .await
        .expect("query Misc events");
    Ok(evs)
}

#[tokio::test]
async fn phase5_curator_tenant_failure_harness_cycle_refused_on_empty_tenant() {
    // Category 1: project carries empty tenant_id → worker refuses
    // the cycle before invoking the executor.
    let db = db::open(":memory:").await.expect("open db");
    seed_project(&db, "proj-empty", "").await;
    let events = Arc::new(SqliteEventStore::new(db.clone()));
    let evs = run_worker_and_get_misc_events(db, events, "proj-empty", Arc::new(NoopCycleExecutor))
        .await
        .expect("worker run");
    assert!(
        evs.iter().any(|e| {
            e.data.get("kind").and_then(|v| v.as_str()) == Some("curator_cycle_refused")
                && e.data.get("failure_category").and_then(|v| v.as_str())
                    == Some("tenant_unresolved")
        }),
        "curator_cycle_refused + tenant_unresolved must emit when worker tenant is empty; got events: {evs:?}"
    );
}

#[tokio::test]
async fn phase5_curator_tenant_failure_harness_quarantine_tenant_unresolved() {
    // Category 2: a decision came back from the executor flagged with
    // TenantUnresolved. The worker emits curator_decision_quarantined
    // with the matching failure_category.
    let db = db::open(":memory:").await.expect("open db");
    seed_project(&db, "proj-a", "tenant-a").await;
    let events = Arc::new(SqliteEventStore::new(db.clone()));
    let executor = Arc::new(QuarantinedExecutor {
        category: CuratorFailureCategory::TenantUnresolved,
        detail: "tenant_unresolved: revision rev-x has empty tenant_id",
    });
    let evs = run_worker_and_get_misc_events(db, events, "proj-a", executor)
        .await
        .expect("worker run");
    assert!(
        evs.iter().any(|e| {
            e.data.get("kind").and_then(|v| v.as_str()) == Some("curator_decision_quarantined")
                && e.data.get("failure_category").and_then(|v| v.as_str())
                    == Some("tenant_unresolved")
        }),
        "curator_decision_quarantined + tenant_unresolved must emit; got events: {evs:?}"
    );
}

#[tokio::test]
async fn phase5_curator_tenant_failure_harness_quarantine_cross_tenant_ref() {
    // Category 3: a decision's target tenant ≠ worker tenant. Worker
    // emits curator_decision_quarantined with cross_tenant_ref.
    let db = db::open(":memory:").await.expect("open db");
    seed_project(&db, "proj-a", "tenant-a").await;
    let events = Arc::new(SqliteEventStore::new(db.clone()));
    let executor = Arc::new(QuarantinedExecutor {
        category: CuratorFailureCategory::CrossTenantRef,
        detail: "cross_tenant_ref: revision rev-b tenant tenant-b != worker tenant tenant-a",
    });
    let evs = run_worker_and_get_misc_events(db, events, "proj-a", executor)
        .await
        .expect("worker run");
    assert!(
        evs.iter().any(|e| {
            e.data.get("kind").and_then(|v| v.as_str()) == Some("curator_decision_quarantined")
                && e.data.get("failure_category").and_then(|v| v.as_str())
                    == Some("cross_tenant_ref")
        }),
        "curator_decision_quarantined + cross_tenant_ref must emit; got events: {evs:?}"
    );
}
