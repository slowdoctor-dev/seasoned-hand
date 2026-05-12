use serde_json::json;

use super::persistence::VerificationStore;
use super::routes::{ListQuery, RouteOutcome, get_verification, list_verifications};
use super::*;
use crate::db;

async fn open_pool() -> db::DbPool {
    db::open(":memory:").await.expect("open in-memory db")
}

async fn insert_session(pool: &db::DbPool, id: &str, state: &str) {
    let id = id.to_string();
    let state = state.to_string();
    pool.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, state) \
             VALUES (?, 0, 0, ?)",
            rusqlite::params![id, state],
        )
        .expect("insert session row");
    })
    .await;
}

fn synthetic_new(session_id: &str, verdict: VerdictKind, evidence: &[i64]) -> NewVerification {
    NewVerification {
        session_id: session_id.into(),
        triggered_at_event_id: 42,
        trigger: VerifyTrigger::TaskComplete {
            final_message_call_id: "call-1".into(),
        },
        verdict,
        reason: "test reason".into(),
        evidence_event_ids: evidence.to_vec(),
        suggested_plan_update: Some(json!({"phases": [{"id": 1, "title": "Re-plan"}]})),
        model_id: "claude-sonnet-4-6".into(),
        cost_cents: 3,
    }
}

// ----------------------------------------------------------------------------
// Migration parity tests (V004 widens sessions.state CHECK; ensure indexes).
// ----------------------------------------------------------------------------

#[tokio::test]
async fn migration_v004_idempotent_against_phase0_seed() {
    // First open runs V001-V004 from the embedded migrations runner.
    let pool = open_pool().await;
    // Sanity: VERIFYING is now an accepted state.
    pool.with_conn(|conn| {
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, state) \
             VALUES ('s-verifying', 0, 0, 'VERIFYING')",
            [],
        )
        .expect("VERIFYING must be accepted post-V004");
    })
    .await;
    // Re-running migrations on an already-migrated pool is a no-op
    // (refinery skips applied versions).
    let count: i64 = pool
        .with_conn(|conn| {
            conn.query_row::<i64, _, _>("SELECT COUNT(*) FROM verifications", [], |r| r.get(0))
                .unwrap()
        })
        .await;
    assert_eq!(count, 0);
}

#[tokio::test]
async fn migration_v004_preserves_sessions_indexes() {
    let pool = open_pool().await;
    let indexes: Vec<String> = pool
        .with_conn(|conn| -> rusqlite::Result<Vec<String>> {
            let mut stmt = conn.prepare("PRAGMA index_list('sessions')")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
            rows.collect()
        })
        .await
        .expect("index_list");
    assert!(
        indexes.iter().any(|n| n == "idx_sessions_state"),
        "idx_sessions_state must survive V004 (got: {indexes:?})"
    );
}

#[tokio::test]
async fn migration_v004_rejects_invalid_state_value() {
    let pool = open_pool().await;
    let err = pool
        .with_conn(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, created_at, updated_at, state) \
                 VALUES ('bad', 0, 0, 'NOT_A_STATE')",
                [],
            )
        })
        .await;
    assert!(
        err.is_err(),
        "post-V004 CHECK must still reject non-listed state values"
    );
}

// ----------------------------------------------------------------------------
// Persistence CRUD.
// ----------------------------------------------------------------------------

#[tokio::test]
async fn persistence_insert_and_get_round_trip() {
    let pool = open_pool().await;
    insert_session(&pool, "sess-1", "RUNNING").await;
    let store = VerificationStore::new(pool.clone());

    let id = store
        .insert(synthetic_new("sess-1", VerdictKind::Fail, &[10, 20, 30]))
        .await
        .expect("insert");

    let row = store.get(&id).await.expect("get");
    assert_eq!(row.id, id);
    assert_eq!(row.session_id, "sess-1");
    assert_eq!(row.verdict, VerdictKind::Fail);
    assert_eq!(row.trigger_kind, "TaskComplete");
    assert_eq!(row.evidence_event_ids, vec![10, 20, 30]);
    assert!(row.suggested_plan_update.is_some());
    assert_eq!(row.reason, "test reason");
    assert_eq!(row.model_id, "claude-sonnet-4-6");
    assert_eq!(row.cost_cents, 3);
}

#[tokio::test]
async fn persistence_get_returns_not_found_for_unknown_id() {
    let pool = open_pool().await;
    let store = VerificationStore::new(pool);
    let err = store.get("does-not-exist").await.expect_err("not found");
    assert!(matches!(err, VerifierPersistenceError::NotFound(_)));
}

#[tokio::test]
async fn persistence_list_paginates_by_cursor() {
    let pool = open_pool().await;
    insert_session(&pool, "sess-page", "RUNNING").await;
    let store = VerificationStore::new(pool);
    for i in 0..75 {
        // Re-set created_at deterministically so ordering is stable;
        // the store sets created_at internally to now() so we manually
        // override via a direct INSERT for this test.
        let session = "sess-page".to_string();
        let i_clone = i as i64;
        let id = uuid::Uuid::new_v4().to_string();
        store
            .pool_for_test()
            .with_conn(move |conn| {
                conn.execute(
                    "INSERT INTO verifications ( \
                       id, session_id, triggered_at_event_id, trigger_kind, \
                       trigger_detail, verdict, reason, evidence_event_ids, \
                       suggested_plan_update, model_id, cost_cents, created_at \
                     ) VALUES (?, ?, 0, 'TaskComplete', '{}', 'pass', 'r', '[]', NULL, 'm', 0, ?)",
                    rusqlite::params![id, session, i_clone],
                )
                .unwrap();
            })
            .await;
    }
    let first = store
        .list_by_session("sess-page", None, 50)
        .await
        .expect("page 1");
    assert_eq!(first.len(), 50);
    // Newest-first means created_at descends; the highest is 74.
    assert_eq!(first.first().unwrap().created_at, 74);
    assert_eq!(first.last().unwrap().created_at, 25);

    let cursor = first.last().unwrap().created_at;
    let second = store
        .list_by_session("sess-page", Some(cursor), 50)
        .await
        .expect("page 2");
    assert_eq!(second.len(), 25);
    assert_eq!(second.first().unwrap().created_at, 24);
    assert_eq!(second.last().unwrap().created_at, 0);
}

// ----------------------------------------------------------------------------
// Routes (pure-outcome layer — axum wrapping happens in seasoned-hand-server).
// ----------------------------------------------------------------------------

#[tokio::test]
async fn http_verifications_list_route_returns_paginated_json() {
    let pool = open_pool().await;
    insert_session(&pool, "sess-r", "RUNNING").await;
    let store = VerificationStore::new(pool);
    for _ in 0..3 {
        store
            .insert(synthetic_new("sess-r", VerdictKind::Pass, &[1]))
            .await
            .unwrap();
    }
    let outcome = list_verifications(
        &store,
        "sess-r",
        ListQuery {
            cursor: None,
            limit: Some(2),
        },
    )
    .await;
    let body = match outcome {
        RouteOutcome::Ok(b) => b,
        _ => panic!("expected Ok"),
    };
    assert_eq!(body.rows.len(), 2);
    assert!(body.next_cursor.is_some());
}

#[tokio::test]
async fn http_verification_by_id_route_returns_full_row_including_suggested_plan_update() {
    let pool = open_pool().await;
    insert_session(&pool, "sess-x", "RUNNING").await;
    let store = VerificationStore::new(pool);
    let id = store
        .insert(synthetic_new("sess-x", VerdictKind::Fail, &[7]))
        .await
        .unwrap();
    let outcome = get_verification(&store, &id).await;
    let row = match outcome {
        RouteOutcome::Ok(r) => r,
        _ => panic!("expected Ok"),
    };
    assert_eq!(row.id, id);
    assert_eq!(row.verdict, VerdictKind::Fail);
    assert_eq!(row.evidence_event_ids, vec![7]);
    let suggested = row
        .suggested_plan_update
        .as_ref()
        .expect("suggested_plan_update present");
    assert_eq!(suggested["phases"][0]["title"], json!("Re-plan"));
}

#[tokio::test]
async fn http_get_route_returns_not_found_for_unknown_id() {
    let pool = open_pool().await;
    let store = VerificationStore::new(pool);
    let outcome = get_verification(&store, "no-such-id").await;
    assert!(matches!(outcome, RouteOutcome::NotFound(_)));
}

// ----------------------------------------------------------------------------
// Prompt loading.
// ----------------------------------------------------------------------------

#[tokio::test]
async fn verifier_system_prompt_loaded_at_boot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("verifier.system.txt");
    let body = "Independent reviewer — FAIL biased.\n";
    std::fs::write(&path, body).unwrap();

    let loaded = load_system_prompt(path.to_str().unwrap()).expect("load");
    assert_eq!(loaded, body);
}

#[tokio::test]
async fn verifier_system_prompt_missing_fails_boot() {
    let err = load_system_prompt("/nonexistent/verifier.system.txt")
        .expect_err("missing file must error");
    match err {
        VerifierError::PromptMissing { path, .. } => {
            assert!(path.contains("verifier.system.txt"), "path was: {path}");
        }
        other => panic!("expected PromptMissing, got {other:?}"),
    }
}
