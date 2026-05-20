use std::sync::Arc;

use super::{
    BacklogProbe, CuratorConfig, CuratorFailureCategory, CuratorTrigger, CuratorWorkerError,
    NoopCycleExecutor, ProductionCuratorWorker, project_tenant_id, validate_decision_scope,
};
use crate::db::open;
use crate::events::{EventQuery, EventStore, EventType, sqlite::SqliteEventStore};

struct StaticBacklog;

#[async_trait::async_trait]
impl BacklogProbe for StaticBacklog {
    async fn pending_count(&self, _project_id: &str) -> Result<u32, CuratorWorkerError> {
        Ok(0)
    }
}

async fn seed_project(db: &crate::db::DbPool, id: &str, tenant_id: &str) {
    let id = id.to_string();
    let tenant_id = tenant_id.to_string();
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO projects (id, tenant_id, title, description, status, created_at, updated_at)
             VALUES (?1, ?2, 'p', NULL, 'active', 1, 1)",
            rusqlite::params![id, tenant_id],
        )
        .unwrap();
    })
    .await;
}

async fn seed_revision(
    db: &crate::db::DbPool,
    revision_id: &str,
    playbook_id: &str,
    project_id: &str,
    tenant_id: &str,
) {
    let revision_id = revision_id.to_string();
    let playbook_id = playbook_id.to_string();
    let project_id = project_id.to_string();
    let tenant_id = tenant_id.to_string();
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO playbooks
             (id, tenant_id, title, content_path, schema_version, source_task_id, created_at, updated_at,
              trigger_keywords, content, status, source_project_id, active_revision_id, success_count, failure_count)
             VALUES (?1, ?2, 't', '', 1, NULL, 1, 1, '[]', '{}', 'active', ?3, ?4, 0, 0)",
            rusqlite::params![playbook_id, tenant_id, project_id, revision_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO playbook_revisions
             (id, tenant_id, playbook_id, revision_no, parent_revision_id, title, trigger_keywords,
              content, source_task_id, source_project_id, author_type, change_kind, confidence,
              created_at, superseded_at)
             VALUES (?1, ?2, ?3, 1, NULL, 'r', '[]', '{}', NULL, ?4, 'extractor', 'extract',
                     1.0, 1, NULL)",
            rusqlite::params![revision_id, tenant_id, playbook_id, project_id],
        )
        .unwrap();
    })
    .await;
}

#[tokio::test]
async fn cross_tenant_decision_target_is_quarantined() {
    let db = open(":memory:").await.unwrap();
    seed_project(&db, "proj-a", "tenant-a").await;
    seed_revision(&db, "rev-a-1", "pb-a", "proj-a", "tenant-a").await;
    seed_revision(&db, "rev-b-1", "pb-b", "proj-a", "tenant-b").await;

    db.with_conn(|conn| {
        let worker_tenant = project_tenant_id(conn, "proj-a").unwrap().unwrap();
        let err = validate_decision_scope(
            conn,
            "proj-a",
            &worker_tenant,
            "rev-b-1",
            &["rev-a-1".to_string(), "rev-b-1".to_string()],
        )
        .unwrap()
        .expect("cross-tenant revision should be rejected");
        assert_eq!(err.failure_category, CuratorFailureCategory::CrossTenantRef);
        assert!(err.detail.contains("cross_tenant_ref"));
    })
    .await;
}

#[tokio::test]
async fn worker_tenant_unresolved_emits_curator_cycle_refused() {
    let db = open(":memory:").await.unwrap();
    seed_project(&db, "proj-empty", "").await;
    let events = Arc::new(SqliteEventStore::new(db.clone()));

    let worker = ProductionCuratorWorker::new(
        CuratorConfig {
            enabled: true,
            project_id: "proj-empty".to_string(),
            org_aggregation_enabled: false,
            ..CuratorConfig::default()
        },
        db,
        events.clone(),
        Arc::new(StaticBacklog),
        Arc::new(NoopCycleExecutor),
    );

    let err = worker
        .run_once(CuratorTrigger::Manual, 0)
        .await
        .expect_err("missing worker tenant must refuse cycle");
    assert!(err.to_string().contains("tenant_unresolved"));

    let session_id = "curator:proj-empty";
    let events = events
        .query(
            session_id,
            EventQuery {
                event_type: Some(EventType::Misc),
                after_id: None,
                limit: Some(20),
            },
        )
        .await
        .unwrap();
    assert!(events.iter().any(|event| {
        event.data["kind"] == "curator_cycle_refused"
            && event.data["failure_category"] == "tenant_unresolved"
    }));
}

#[tokio::test]
async fn same_tenant_decision_scope_passes() {
    let db = open(":memory:").await.unwrap();
    seed_project(&db, "proj-a", "tenant-a").await;
    seed_revision(&db, "rev-a-1", "pb-a", "proj-a", "tenant-a").await;

    db.with_conn(|conn| {
        let worker_tenant = project_tenant_id(conn, "proj-a").unwrap().unwrap();
        let err = validate_decision_scope(
            conn,
            "proj-a",
            &worker_tenant,
            "rev-a-1",
            &["rev-a-1".to_string()],
        )
        .unwrap();
        assert!(err.is_none(), "same-tenant revision must pass");
    })
    .await;
}
