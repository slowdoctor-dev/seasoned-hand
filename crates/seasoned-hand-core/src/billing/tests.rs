//! Story 5.12 regression tests for [`NearlineWriter`].
//! refs: /specs/phase-5/stories/story-5.12.md

use super::*;
use crate::db::{self, DbPool};
use crate::events::{EventQuery, EventStore, EventType, sqlite::SqliteEventStore};
use rusqlite::params;

async fn setup() -> DbPool {
    let pool = db::open(":memory:").await.unwrap();
    // V013 already bootstrapped sentinel org + user. Seed two tenants so
    // we can exercise tenant isolation: 'tenant-a' (real org + user) and
    // the sentinel 'legacy-default' that V013 wrote.
    pool.with_conn(|conn| {
        conn.execute(
            "INSERT INTO organizations (id, tenant_id, slug, display_name, status,
                                         created_at, updated_at)
             VALUES ('org-a', 'tenant-a', 'org-a', 'A', 'active', 0, 0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO users (id, tenant_id, email, display_name, status,
                                created_at, updated_at)
             VALUES ('user-a', 'tenant-a', 'a@example.com', 'A', 'active', 0, 0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO organization_memberships (id, tenant_id, organization_id,
                                                   user_id, role, is_primary,
                                                   created_at, updated_at)
             VALUES ('mem-a', 'tenant-a', 'org-a', 'user-a', 'admin', 1, 0, 0)",
            [],
        )?;
        // Real project + task so the join chain resolves.
        conn.execute(
            "INSERT INTO projects (id, tenant_id, title, status, created_at, updated_at)
             VALUES ('proj-a', 'tenant-a', 'P', 'active', 0, 0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO tasks (id, project_id, tenant_id, title, status,
                                created_at, updated_at)
             VALUES ('task-a', 'proj-a', 'tenant-a', 'T', 'Drafted', 0, 0)",
            [],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();
    pool
}

/// Insert a finalized session row with the chosen totals + bucket-month.
/// `updated_at` is interpreted as microseconds (matches `now_micros()`).
#[allow(clippy::too_many_arguments)]
async fn insert_session(
    pool: &DbPool,
    id: &str,
    user_id: Option<&str>,
    task_id: Option<&str>,
    project_id: Option<&str>,
    updated_at: i64,
    cost_cents: i64,
    tool_calls: i64,
    state: &str,
) {
    let id = id.to_string();
    let user_id = user_id.map(str::to_string);
    let task_id = task_id.map(str::to_string);
    let project_id = project_id.map(str::to_string);
    let state = state.to_string();
    pool.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, state,
                                    project_id, user_id, task_id,
                                    cost_cents, tool_calls)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                id, updated_at, updated_at, state, project_id, user_id, task_id, cost_cents,
                tool_calls,
            ],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();
}

/// 2026-05-15 12:00:00 UTC in microseconds — month bucket "202605".
const MAY_2026_MICROS: i64 = 1_778_846_400_000_000;
/// 2026-06-10 12:00:00 UTC in microseconds — month bucket "202606".
const JUN_2026_MICROS: i64 = 1_781_352_000_000_000;

#[tokio::test]
async fn flush_empty_db_writes_zero_rows() {
    let pool = setup().await;
    let writer = NearlineWriter::new(pool);
    let report = writer.flush().await.expect("flush");
    assert_eq!(report.rows_upserted, 0);
}

#[tokio::test]
async fn flush_finalized_session_writes_ledger_row() {
    let pool = setup().await;
    insert_session(
        &pool,
        "sess-1",
        Some("user-a"),
        Some("task-a"),
        Some("proj-a"),
        MAY_2026_MICROS,
        100,
        5,
        "FINISHED",
    )
    .await;
    let writer = NearlineWriter::new(pool.clone());
    let report = writer.flush().await.expect("flush");
    assert_eq!(report.rows_upserted, 1);
    let (org, sessions, tools, cents): (String, i64, i64, i64) = pool
        .with_conn(|conn| {
            conn.query_row(
                "SELECT organization_id, session_count, tool_calls, cost_cents
                 FROM user_cost_ledger
                 WHERE tenant_id = 'tenant-a' AND user_id = 'user-a'
                   AND month_yyyymm = '202605'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
        })
        .await
        .unwrap();
    assert_eq!(org, "org-a");
    assert_eq!(sessions, 1);
    assert_eq!(tools, 5);
    assert_eq!(cents, 100);
}

#[tokio::test]
async fn flush_running_session_is_excluded() {
    // RUNNING sessions are mid-flight; only FINISHED/ERROR count toward
    // the nearline rollup so we don't bill for sessions that may roll
    // back or change cost mid-run.
    let pool = setup().await;
    insert_session(
        &pool,
        "sess-running",
        Some("user-a"),
        Some("task-a"),
        Some("proj-a"),
        MAY_2026_MICROS,
        100,
        5,
        "RUNNING",
    )
    .await;
    let writer = NearlineWriter::new(pool);
    let report = writer.flush().await.expect("flush");
    assert_eq!(report.rows_upserted, 0);
}

#[tokio::test]
async fn flush_is_idempotent_across_reruns() {
    // Re-running the writer against unchanged state must converge to the
    // same totals, not double-count. Idempotency is the contract that
    // lets ops re-run after a partial failure without manual cleanup.
    let pool = setup().await;
    insert_session(
        &pool,
        "sess-1",
        Some("user-a"),
        Some("task-a"),
        Some("proj-a"),
        MAY_2026_MICROS,
        100,
        5,
        "FINISHED",
    )
    .await;
    let writer = NearlineWriter::new(pool.clone());
    writer.flush().await.unwrap();
    writer.flush().await.unwrap();
    let (sessions, cents): (i64, i64) = pool
        .with_conn(|conn| {
            conn.query_row(
                "SELECT session_count, cost_cents FROM user_cost_ledger
                 WHERE user_id = 'user-a' AND month_yyyymm = '202605'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
        .await
        .unwrap();
    assert_eq!(sessions, 1);
    assert_eq!(cents, 100);
}

#[tokio::test]
async fn flush_aggregates_multiple_sessions_per_bucket() {
    let pool = setup().await;
    for (sid, cost) in [("sess-1", 100i64), ("sess-2", 250), ("sess-3", 50)] {
        insert_session(
            &pool,
            sid,
            Some("user-a"),
            Some("task-a"),
            Some("proj-a"),
            MAY_2026_MICROS,
            cost,
            1,
            "FINISHED",
        )
        .await;
    }
    let writer = NearlineWriter::new(pool.clone());
    let report = writer.flush().await.unwrap();
    assert_eq!(report.rows_upserted, 1);
    let (sessions, cents): (i64, i64) = pool
        .with_conn(|conn| {
            conn.query_row(
                "SELECT session_count, cost_cents FROM user_cost_ledger
                 WHERE user_id = 'user-a' AND month_yyyymm = '202605'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
        .await
        .unwrap();
    assert_eq!(sessions, 3);
    assert_eq!(cents, 400);
}

#[tokio::test]
async fn flush_separates_buckets_by_month() {
    let pool = setup().await;
    insert_session(
        &pool,
        "sess-may",
        Some("user-a"),
        Some("task-a"),
        Some("proj-a"),
        MAY_2026_MICROS,
        100,
        2,
        "FINISHED",
    )
    .await;
    insert_session(
        &pool,
        "sess-jun",
        Some("user-a"),
        Some("task-a"),
        Some("proj-a"),
        JUN_2026_MICROS,
        300,
        4,
        "FINISHED",
    )
    .await;
    let writer = NearlineWriter::new(pool.clone());
    let report = writer.flush().await.unwrap();
    assert_eq!(report.rows_upserted, 2);
    let counts: Vec<(String, i64)> = pool
        .with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT month_yyyymm, cost_cents FROM user_cost_ledger
                 WHERE user_id = 'user-a'
                 ORDER BY month_yyyymm ASC",
            )?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok::<Vec<(String, i64)>, rusqlite::Error>(rows)
        })
        .await
        .unwrap();
    assert_eq!(
        counts,
        vec![("202605".to_string(), 100), ("202606".to_string(), 300)]
    );
}

#[tokio::test]
async fn flush_falls_back_to_sentinels_for_unattributable_session() {
    // Sessions predating V013 may have NULL user_id / no task / no
    // project — the writer must still produce a row, attributing to
    // the sentinels seeded by V013 so the ledger never silently drops
    // a finalized session.
    let pool = setup().await;
    insert_session(
        &pool,
        "sess-orphan",
        None,
        None,
        None,
        MAY_2026_MICROS,
        42,
        1,
        "FINISHED",
    )
    .await;
    let writer = NearlineWriter::new(pool.clone());
    let report = writer.flush().await.unwrap();
    assert_eq!(report.rows_upserted, 1);
    let (tenant, org, user, cents): (String, String, String, i64) = pool
        .with_conn(|conn| {
            conn.query_row(
                "SELECT tenant_id, organization_id, user_id, cost_cents
                 FROM user_cost_ledger LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
        })
        .await
        .unwrap();
    assert_eq!(tenant, "legacy-default");
    assert_eq!(org, "org-legacy-default");
    assert_eq!(user, "user-legacy-admin");
    assert_eq!(cents, 42);
}

#[tokio::test]
async fn flush_records_event_watermark() {
    // The watermark is advisory but story 5.13's reconciliation job
    // depends on it being set whenever a flush observes any events.
    let pool = setup().await;
    // Append a session + an event so MAX(events.id) is non-null.
    insert_session(
        &pool,
        "sess-1",
        Some("user-a"),
        Some("task-a"),
        Some("proj-a"),
        MAY_2026_MICROS,
        100,
        1,
        "FINISHED",
    )
    .await;
    pool.with_conn(|conn| {
        conn.execute(
            "INSERT INTO events (session_id, timestamp, type, source, data)
             VALUES ('sess-1', 0, 'Action', 'test', '{}')",
            [],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();
    let writer = NearlineWriter::new(pool.clone());
    let report = writer.flush().await.unwrap();
    assert!(report.high_watermark_event_id.is_some());
    let stored: Option<i64> = pool
        .with_conn(|conn| {
            conn.query_row(
                "SELECT source_high_watermark_event_id FROM user_cost_ledger LIMIT 1",
                [],
                |r| r.get(0),
            )
        })
        .await
        .unwrap();
    assert_eq!(stored, report.high_watermark_event_id);
}

#[tokio::test]
async fn recomputes_match_when_no_drift() {
    let pool = setup().await;
    insert_session(
        &pool,
        "sess-r1",
        Some("user-a"),
        Some("task-a"),
        Some("proj-a"),
        MAY_2026_MICROS,
        150,
        3,
        "FINISHED",
    )
    .await;
    let writer = NearlineWriter::new(pool.clone());
    writer.flush().await.unwrap();

    let events = std::sync::Arc::new(SqliteEventStore::new(pool.clone()));
    let job = ReconciliationJob::new(pool, events);
    let report = job.run("202605").await.unwrap();
    assert_eq!(report.drifted_rows, 0);
    assert!(report.drifts.is_empty());
}

#[tokio::test]
async fn detects_cost_drift() {
    let pool = setup().await;
    insert_session(
        &pool,
        "sess-r2",
        Some("user-a"),
        Some("task-a"),
        Some("proj-a"),
        MAY_2026_MICROS,
        200,
        4,
        "FINISHED",
    )
    .await;
    let writer = NearlineWriter::new(pool.clone());
    writer.flush().await.unwrap();
    pool.with_conn(|conn| {
        conn.execute(
            "UPDATE user_cost_ledger SET cost_cents = 100
             WHERE tenant_id = 'tenant-a' AND user_id = 'user-a' AND month_yyyymm = '202605'",
            [],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();

    let events = std::sync::Arc::new(SqliteEventStore::new(pool.clone()));
    let job = ReconciliationJob::new(pool.clone(), events.clone());
    let report = job.run("202605").await.unwrap();
    assert_eq!(report.drifted_rows, 1);
    let drift = &report.drifts[0];
    assert_eq!(drift.expected_cost_cents, 200);
    assert_eq!(drift.observed_cost_cents, 100);
    assert!((drift.delta_pct - 0.5).abs() < f64::EPSILON);

    let audit_events = events
        .query(
            "audit:tenant-a",
            EventQuery {
                event_type: Some(EventType::Misc),
                after_id: None,
                limit: Some(20),
            },
        )
        .await
        .unwrap();
    assert!(audit_events.iter().any(|e| {
        e.data.get("kind").and_then(|v| v.as_str()) == Some("user_cost_reconciliation_drift")
    }));
}

#[tokio::test]
async fn detects_zero_expected_vs_nonzero_observed() {
    let pool = setup().await;
    pool.with_conn(|conn| {
        conn.execute(
            "INSERT INTO user_cost_ledger
               (id, tenant_id, organization_id, user_id, month_yyyymm, session_count,
                tool_calls, cost_cents, source_low_watermark_event_id,
                source_high_watermark_event_id, reconciled_at, created_at, updated_at)
             VALUES
               ('ucl-manual', 'tenant-a', 'org-a', 'user-a', '202605', 1, 1, 25, NULL, NULL, 0, 0, 0)",
            [],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();
    let events = std::sync::Arc::new(SqliteEventStore::new(pool.clone()));
    let job = ReconciliationJob::new(pool, events);
    let report = job.run("202605").await.unwrap();
    assert_eq!(report.drifted_rows, 1);
    let drift = &report.drifts[0];
    assert_eq!(drift.expected_cost_cents, 0);
    assert_eq!(drift.observed_cost_cents, 25);
    assert!((drift.delta_pct - 1.0).abs() < f64::EPSILON);
}
