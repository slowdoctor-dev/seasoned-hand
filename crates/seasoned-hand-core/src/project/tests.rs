use serde_json::json;

use super::project::{NewProject, ProjectError, ProjectPatch, ProjectStatus, ProjectStore};
use super::task::{NewTask, TaskError, TaskStatus, TaskStore, legal_transitions};
use crate::db;

async fn open_pool() -> db::DbPool {
    db::open(":memory:").await.expect("open in-memory db")
}

#[tokio::test]
async fn project_store_crud() {
    let pool = open_pool().await;
    let store = ProjectStore::new(pool);

    let id = store
        .insert(NewProject {
            tenant_id: None,
            title: "Q4 Planning".into(),
            description: Some("Draft Q4 goals".into()),
        })
        .await
        .expect("insert");

    let row = store.get(&id).await.expect("get");
    assert_eq!(row.id, id);
    assert_eq!(row.title, "Q4 Planning");
    assert_eq!(row.description.as_deref(), Some("Draft Q4 goals"));
    assert_eq!(row.status, ProjectStatus::Active);
    assert!(row.tenant_id.is_none());

    // Patch: rename only.
    store
        .patch(
            &id,
            ProjectPatch {
                title: Some("Q4 Strategy".into()),
                description: None,
            },
        )
        .await
        .expect("patch");
    let row = store.get(&id).await.expect("get");
    assert_eq!(row.title, "Q4 Strategy");
    assert_eq!(row.description.as_deref(), Some("Draft Q4 goals"));

    // set_status: archive.
    store
        .set_status(&id, ProjectStatus::Archived)
        .await
        .expect("set_status");
    let row = store.get(&id).await.expect("get");
    assert_eq!(row.status, ProjectStatus::Archived);

    // Unknown id is NotFound.
    let err = store.get("nope").await.expect_err("not found");
    assert!(matches!(err, ProjectError::NotFound(_)));
    let err = store
        .set_status("nope", ProjectStatus::Archived)
        .await
        .expect_err("not found");
    assert!(matches!(err, ProjectError::NotFound(_)));
}

#[tokio::test]
async fn project_list_filters_by_status() {
    let pool = open_pool().await;
    let store = ProjectStore::new(pool);

    let active_a = store
        .insert(NewProject {
            tenant_id: None,
            title: "Active A".into(),
            description: None,
        })
        .await
        .unwrap();
    let active_b = store
        .insert(NewProject {
            tenant_id: None,
            title: "Active B".into(),
            description: None,
        })
        .await
        .unwrap();
    let archived = store
        .insert(NewProject {
            tenant_id: None,
            title: "Archived".into(),
            description: None,
        })
        .await
        .unwrap();
    store
        .set_status(&archived, ProjectStatus::Archived)
        .await
        .unwrap();

    let active = store
        .list(Some(ProjectStatus::Active), None, 50)
        .await
        .expect("list active");
    let ids: Vec<_> = active.iter().map(|p| p.id.clone()).collect();
    assert_eq!(active.len(), 2);
    assert!(ids.contains(&active_a));
    assert!(ids.contains(&active_b));
    assert!(!ids.contains(&archived));

    let archived_only = store
        .list(Some(ProjectStatus::Archived), None, 50)
        .await
        .expect("list archived");
    assert_eq!(archived_only.len(), 1);
    assert_eq!(archived_only[0].id, archived);

    let all = store.list(None, None, 50).await.expect("list all");
    assert_eq!(all.len(), 3);
}

#[tokio::test]
async fn task_store_crud() {
    let pool = open_pool().await;
    let projects = ProjectStore::new(pool.clone());
    let tasks = TaskStore::new(pool);

    let project_id = projects
        .insert(NewProject {
            tenant_id: None,
            title: "Parent".into(),
            description: None,
        })
        .await
        .unwrap();

    let task_id = tasks
        .insert(NewTask {
            project_id: project_id.clone(),
            tenant_id: None,
            title: "Write Q4 summary".into(),
            expected_due_at: Some(1_700_000_000_000_000),
        })
        .await
        .expect("insert");

    let row = tasks.get(&task_id).await.expect("get");
    assert_eq!(row.id, task_id);
    assert_eq!(row.project_id, project_id);
    assert_eq!(row.title, "Write Q4 summary");
    assert_eq!(row.status, TaskStatus::Drafted);
    assert!(row.brief.is_none());
    assert_eq!(row.expected_due_at, Some(1_700_000_000_000_000));
    assert!(row.parent_task_id.is_none());
    assert!(row.schedule.is_none());
    assert!(row.skill_attached_event_id.is_none());

    // set_brief stores JSON without moving the state machine.
    let brief = json!({ "goal": "Summarize", "phases": [] });
    tasks.set_brief(&task_id, &brief).await.expect("set_brief");
    let row = tasks.get(&task_id).await.unwrap();
    assert_eq!(row.brief.as_ref().unwrap(), &brief);
    assert_eq!(row.status, TaskStatus::Drafted);

    // set_completed: drafted → completed is illegal, but running →
    // completed should work via the right sequence.
    let err = tasks.set_completed(&task_id).await.expect_err("illegal");
    assert!(matches!(err, TaskError::IllegalTransition { .. }));

    for to in [
        TaskStatus::Briefed,
        TaskStatus::Confirmed,
        TaskStatus::Running,
    ] {
        tasks.set_status(&task_id, to).await.expect("legal advance");
    }
    tasks.set_completed(&task_id).await.expect("set_completed");
    let row = tasks.get(&task_id).await.unwrap();
    assert_eq!(row.status, TaskStatus::Completed);
    assert!(row.completed_at.is_some());

    // set_failure on a completed task is illegal.
    let err = tasks
        .set_failure(&task_id, "boom")
        .await
        .expect_err("illegal");
    assert!(matches!(err, TaskError::IllegalTransition { .. }));

    // Unknown id is NotFound.
    let err = tasks.get("nope").await.expect_err("not found");
    assert!(matches!(err, TaskError::NotFound(_)));
}

#[tokio::test]
async fn task_state_machine_legal_transitions() {
    let pool = open_pool().await;
    let projects = ProjectStore::new(pool.clone());
    let tasks = TaskStore::new(pool);

    let project_id = projects
        .insert(NewProject {
            tenant_id: None,
            title: "P".into(),
            description: None,
        })
        .await
        .unwrap();

    // Happy path through running ⇄ paused → completed.
    let task_id = tasks
        .insert(NewTask {
            project_id: project_id.clone(),
            tenant_id: None,
            title: "Happy".into(),
            expected_due_at: None,
        })
        .await
        .unwrap();
    for to in [
        TaskStatus::Briefed,
        TaskStatus::Confirmed,
        TaskStatus::Running,
        TaskStatus::Paused,
        TaskStatus::Running,
        TaskStatus::Paused,
        TaskStatus::Completed,
    ] {
        tasks.set_status(&task_id, to).await.expect("legal");
    }
    assert_eq!(
        tasks.get(&task_id).await.unwrap().status,
        TaskStatus::Completed
    );

    // Pin the explicit transition table — protects against an accidental
    // re-shuffle that silently widens or narrows the state machine.
    assert_eq!(
        legal_transitions(TaskStatus::Drafted),
        &[TaskStatus::Briefed, TaskStatus::Cancelled]
    );
    assert_eq!(
        legal_transitions(TaskStatus::Briefed),
        &[TaskStatus::Confirmed, TaskStatus::Cancelled]
    );
    assert_eq!(
        legal_transitions(TaskStatus::Confirmed),
        &[TaskStatus::Running, TaskStatus::Cancelled]
    );
    assert_eq!(
        legal_transitions(TaskStatus::Running),
        &[
            TaskStatus::Paused,
            TaskStatus::Completed,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
        ]
    );
    assert_eq!(
        legal_transitions(TaskStatus::Paused),
        &[
            TaskStatus::Running,
            TaskStatus::Completed,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
        ]
    );
    for terminal in [
        TaskStatus::Completed,
        TaskStatus::Failed,
        TaskStatus::Cancelled,
    ] {
        assert!(legal_transitions(terminal).is_empty());
        assert!(terminal.is_terminal());
    }
}

#[tokio::test]
async fn task_state_machine_rejects_illegal_transitions() {
    let pool = open_pool().await;
    let projects = ProjectStore::new(pool.clone());
    let tasks = TaskStore::new(pool);

    let project_id = projects
        .insert(NewProject {
            tenant_id: None,
            title: "P".into(),
            description: None,
        })
        .await
        .unwrap();

    // Drafted → Running is illegal (must go through Briefed → Confirmed).
    let t1 = tasks
        .insert(NewTask {
            project_id: project_id.clone(),
            tenant_id: None,
            title: "Skip ahead".into(),
            expected_due_at: None,
        })
        .await
        .unwrap();
    let err = tasks
        .set_status(&t1, TaskStatus::Running)
        .await
        .expect_err("illegal");
    match err {
        TaskError::IllegalTransition { from, to } => {
            assert_eq!(from, TaskStatus::Drafted);
            assert_eq!(to, TaskStatus::Running);
        }
        other => panic!("expected IllegalTransition, got {other:?}"),
    }
    // Row stays put.
    assert_eq!(tasks.get(&t1).await.unwrap().status, TaskStatus::Drafted);

    // Briefed → Drafted is illegal (no reverse path).
    let t2 = tasks
        .insert(NewTask {
            project_id: project_id.clone(),
            tenant_id: None,
            title: "Reverse".into(),
            expected_due_at: None,
        })
        .await
        .unwrap();
    tasks.set_status(&t2, TaskStatus::Briefed).await.unwrap();
    let err = tasks
        .set_status(&t2, TaskStatus::Drafted)
        .await
        .expect_err("illegal reverse");
    assert!(matches!(err, TaskError::IllegalTransition { .. }));

    // Terminal states reject every move.
    let t3 = tasks
        .insert(NewTask {
            project_id,
            tenant_id: None,
            title: "Terminal".into(),
            expected_due_at: None,
        })
        .await
        .unwrap();
    for to in [
        TaskStatus::Briefed,
        TaskStatus::Confirmed,
        TaskStatus::Running,
    ] {
        tasks.set_status(&t3, to).await.unwrap();
    }
    tasks
        .set_failure(&t3, "explicit failure")
        .await
        .expect("set_failure");
    assert_eq!(tasks.get(&t3).await.unwrap().status, TaskStatus::Failed);
    for to in [
        TaskStatus::Drafted,
        TaskStatus::Briefed,
        TaskStatus::Confirmed,
        TaskStatus::Running,
        TaskStatus::Paused,
        TaskStatus::Completed,
        TaskStatus::Cancelled,
    ] {
        let err = tasks.set_status(&t3, to).await.expect_err("from terminal");
        assert!(matches!(err, TaskError::IllegalTransition { .. }));
    }
}

#[tokio::test]
async fn task_list_paginates_newest_first() {
    let pool = open_pool().await;
    let projects = ProjectStore::new(pool.clone());
    let tasks = TaskStore::new(pool);

    let project_id = projects
        .insert(NewProject {
            tenant_id: None,
            title: "P".into(),
            description: None,
        })
        .await
        .unwrap();

    // Seed 75 task rows with deterministic created_at values so the
    // cursor assertion is stable (mirrors the verifier-store test).
    for i in 0..75_i64 {
        let pid = project_id.clone();
        let id = uuid::Uuid::new_v4().to_string();
        tasks
            .pool_for_test()
            .with_conn(move |conn| {
                conn.execute(
                    "INSERT INTO tasks ( \
                       id, project_id, tenant_id, title, brief, status, \
                       expected_due_at, completed_at, failure_reason, \
                       parent_task_id, schedule, skill_attached_event_id, \
                       created_at, updated_at \
                     ) VALUES (?, ?, NULL, 't', NULL, 'drafted', NULL, NULL, NULL, \
                              NULL, NULL, NULL, ?, ?)",
                    rusqlite::params![id, pid, i, i],
                )
                .unwrap();
            })
            .await;
    }

    let first = tasks
        .list_by_project(&project_id, None, None, 50)
        .await
        .expect("page 1");
    assert_eq!(first.len(), 50);
    assert_eq!(first.first().unwrap().created_at, 74);
    assert_eq!(first.last().unwrap().created_at, 25);

    let cursor = first.last().unwrap().created_at;
    let second = tasks
        .list_by_project(&project_id, None, Some(cursor), 50)
        .await
        .expect("page 2");
    assert_eq!(second.len(), 25);
    assert_eq!(second.first().unwrap().created_at, 24);
    assert_eq!(second.last().unwrap().created_at, 0);

    // Status filter narrows: flip one row to Briefed, confirm filter.
    let briefed_id = first.first().unwrap().id.clone();
    tasks
        .set_status(&briefed_id, TaskStatus::Briefed)
        .await
        .unwrap();
    let only_briefed = tasks
        .list_by_project(&project_id, Some(TaskStatus::Briefed), None, 50)
        .await
        .expect("briefed only");
    assert_eq!(only_briefed.len(), 1);
    assert_eq!(only_briefed[0].id, briefed_id);
}
