//! Story 5.9 regression tests.
//! refs: /specs/phase-5/stories/story-5.9.md

use super::*;
use crate::auth::{AuthContext, Role};
use crate::db::{self, DbPool};
use crate::events::sqlite::SqliteEventStore;
use crate::project::TaskStatus;
use rusqlite::params;

const TENANT: &str = "tenant-test";

async fn setup() -> (DbPool, TaskHandoffService) {
    let pool = db::open(":memory:").await.unwrap();
    let events = std::sync::Arc::new(SqliteEventStore::new(pool.clone()));
    let service = TaskHandoffService::new(pool.clone(), events);
    (pool, service)
}

async fn seed_world(pool: &DbPool, task_status: TaskStatus) -> (String, String) {
    let task_id = format!("t-{}", uuid::Uuid::new_v4());
    let from_user = "user-from".to_string();
    let to_user = "user-to".to_string();
    let task_id_clone = task_id.clone();
    let from_clone = from_user.clone();
    let to_clone = to_user.clone();
    let status = task_status.as_db_str().to_string();
    pool.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO organizations (id, tenant_id, slug, display_name, status, created_at, updated_at)
             VALUES ('org-test', ?, 'org-test', 'Test Org', 'active', 0, 0)",
            params![TENANT],
        )?;
        conn.execute(
            "INSERT INTO users (id, tenant_id, email, display_name, status, created_at, updated_at)
             VALUES (?, ?, 'from@example.com', 'X', 'active', 0, 0)",
            params![from_clone, TENANT],
        )?;
        conn.execute(
            "INSERT INTO users (id, tenant_id, email, display_name, status, created_at, updated_at)
             VALUES (?, ?, 'to@example.com', 'X', 'active', 0, 0)",
            params![to_clone, TENANT],
        )?;
        conn.execute(
            "INSERT INTO users (id, tenant_id, email, display_name, status, created_at, updated_at)
             VALUES ('user-admin', ?, 'admin@example.com', 'A', 'active', 0, 0)",
            params![TENANT],
        )?;
        conn.execute(
            "INSERT INTO projects (id, tenant_id, title, description, status, created_at, updated_at)
             VALUES ('proj-1', ?, 'P', NULL, 'active', 0, 0)",
            params![TENANT],
        )?;
        conn.execute(
            "INSERT INTO tasks (id, project_id, tenant_id, title, brief, status, expected_due_at,
                                completed_at, failure_reason, parent_task_id, schedule,
                                skill_attached_event_id, created_at, updated_at, owner_user_id)
             VALUES (?, 'proj-1', ?, 'Task', NULL, ?, NULL, NULL, NULL, NULL, NULL, NULL, 0, 100, 'user-from')",
            params![task_id_clone, TENANT, status],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();
    (task_id, to_user)
}

fn admin_ctx() -> AuthContext {
    AuthContext {
        tenant_id: TENANT.into(),
        organization_id: "org-test".into(),
        actor_user_id: "user-admin".into(),
        org_role: Role::Admin,
        project_override_role: None,
    }
}

#[tokio::test]
async fn handoff_succeeds_from_drafted_state() {
    let (pool, service) = setup().await;
    let (task_id, _to_user) = seed_world(&pool, TaskStatus::Drafted).await;
    let out = service
        .handoff(
            &admin_ctx(),
            HandoffRequest {
                task_id: task_id.clone(),
                to_user_email: "to@example.com".into(),
                reason: Some("vacation cover".into()),
                expected_updated_at: None,
            },
        )
        .await
        .expect("handoff");
    assert_eq!(out.to_user_id, "user-to");
    assert_eq!(out.from_user_id, "user-from");
    // audit_log row exists with the right action.
    let id = out.audit_log_id.clone();
    let row: (String, String, String, Option<String>) = pool
        .with_conn(move |conn| {
            conn.query_row(
                "SELECT action, resource_type, actor_user_id, target_user_id
                 FROM audit_log WHERE id = ?",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
        })
        .await
        .expect("audit_log read");
    assert_eq!(row.0, "task.handoff");
    assert_eq!(row.1, "task");
    assert_eq!(row.2, "user-admin");
    assert_eq!(row.3.as_deref(), Some("user-to"));
}

#[tokio::test]
async fn handoff_rejects_running_state() {
    let (pool, service) = setup().await;
    let (task_id, _) = seed_world(&pool, TaskStatus::Running).await;
    let err = service
        .handoff(
            &admin_ctx(),
            HandoffRequest {
                task_id,
                to_user_email: "to@example.com".into(),
                reason: None,
                expected_updated_at: None,
            },
        )
        .await
        .expect_err("running must pause first");
    assert!(matches!(err, HandoffError::MustPauseFirst(_)));
}

#[tokio::test]
async fn handoff_rejects_terminal_states() {
    for terminal in [
        TaskStatus::Completed,
        TaskStatus::Failed,
        TaskStatus::Cancelled,
    ] {
        let (pool, service) = setup().await;
        let (task_id, _) = seed_world(&pool, terminal).await;
        let err = service
            .handoff(
                &admin_ctx(),
                HandoffRequest {
                    task_id,
                    to_user_email: "to@example.com".into(),
                    reason: None,
                    expected_updated_at: None,
                },
            )
            .await
            .expect_err("terminal must reject");
        assert!(matches!(err, HandoffError::TerminalState(_)));
    }
}

#[tokio::test]
async fn handoff_rejects_stale_revision() {
    let (pool, service) = setup().await;
    let (task_id, _) = seed_world(&pool, TaskStatus::Paused).await;
    let err = service
        .handoff(
            &admin_ctx(),
            HandoffRequest {
                task_id,
                to_user_email: "to@example.com".into(),
                reason: None,
                expected_updated_at: Some(999), // not 100
            },
        )
        .await
        .expect_err("stale revision must reject");
    assert!(matches!(err, HandoffError::StaleRevision { .. }));
}

#[tokio::test]
async fn handoff_viewer_role_denied_by_policy() {
    let (pool, service) = setup().await;
    let (task_id, _) = seed_world(&pool, TaskStatus::Drafted).await;
    let viewer = AuthContext {
        tenant_id: TENANT.into(),
        organization_id: "org-test".into(),
        actor_user_id: "user-viewer".into(),
        org_role: Role::Viewer,
        project_override_role: None,
    };
    let err = service
        .handoff(
            &viewer,
            HandoffRequest {
                task_id,
                to_user_email: "to@example.com".into(),
                reason: None,
                expected_updated_at: None,
            },
        )
        .await
        .expect_err("viewer must be denied");
    assert!(matches!(err, HandoffError::Auth(_)));
}

#[tokio::test]
async fn can_handoff_reports_state_gate() {
    let (pool, service) = setup().await;
    let (task_id, _) = seed_world(&pool, TaskStatus::Drafted).await;
    assert!(service.can_handoff(&task_id).await.unwrap());

    // Mutate to running; can_handoff must report false (caller must pause).
    let tid = task_id.clone();
    pool.with_conn(move |conn| {
        conn.execute(
            "UPDATE tasks SET status = 'running' WHERE id = ?",
            params![tid],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();
    assert!(!service.can_handoff(&task_id).await.unwrap());

    // Unknown id → false.
    assert!(!service.can_handoff("t-bogus").await.unwrap());
}
