//! Phase 5 story 5.30 — `phase5_user_cost_reconciliation_harness`.
//!
//! Verifies NFR-5.4: monthly per-user cost totals reconcile to source
//! rows within ±0.5%. The harness seeds N sessions across M users
//! with known `cost_cents` totals, runs the NearlineWriter to populate
//! `user_cost_ledger`, then runs the ReconciliationJob and asserts:
//!
//! 1. **Clean state**: a freshly-flushed ledger reconciles with
//!    `drifted_rows == 0`.
//! 2. **Drift detected**: corrupting one ledger row by ≥0.5% (the
//!    NFR-5.4 threshold) triggers a drift finding AND emits a
//!    `Misc{kind:"user_cost_reconciliation_drift"}` event with the
//!    expected fields.
//! 3. **Drift floor**: corrupting a row by less than 0.5% does NOT
//!    trigger a drift finding (pins the threshold against accidental
//!    tightening).
//!
//! refs: /specs/phase-5/stories/story-5.30.md
//! refs: /specs/phase-5/architecture.md §15 harness 5, §9
//! refs: /specs/phase-5/requirements.md F-5.10, NFR-5.4

use rusqlite::params;
use seasoned_hand_core::billing::{NearlineWriter, ReconciliationJob};
use seasoned_hand_core::db::{self, DbPool};
use seasoned_hand_core::events::sqlite::SqliteEventStore;
use seasoned_hand_core::events::{EventQuery, EventStore, EventType};
use std::sync::Arc;

/// 2026-05-15 12:00:00 UTC in microseconds — month bucket "202605".
const MAY_2026_MICROS: i64 = 1_778_846_400_000_000;

async fn seed_corpus(pool: &DbPool) {
    pool.with_conn(|conn| {
        conn.execute(
            "INSERT INTO organizations (id, tenant_id, slug, display_name, status,
                                         created_at, updated_at)
             VALUES ('org-a', 'tenant-a', 'a', 'A', 'active', 0, 0)",
            [],
        )?;
        // 3 users, each with their own primary membership (so the
        // NearlineWriter's organization_memberships join resolves).
        for uid in ["user-alpha", "user-bravo", "user-charlie"] {
            conn.execute(
                "INSERT INTO users (id, tenant_id, email, display_name, status,
                                    created_at, updated_at)
                 VALUES (?, 'tenant-a', ?, 'X', 'active', 0, 0)",
                params![uid, format!("{uid}@x.io")],
            )?;
            conn.execute(
                "INSERT INTO organization_memberships
                   (id, tenant_id, organization_id, user_id, role,
                    is_primary, created_at, updated_at)
                 VALUES (?, 'tenant-a', 'org-a', ?, 'admin', 1, 0, 0)",
                params![format!("mem-{uid}"), uid],
            )?;
        }
        // Project + task common to all sessions.
        conn.execute(
            "INSERT INTO projects (id, tenant_id, title, status, created_at, updated_at)
             VALUES ('proj-a', 'tenant-a', 'P', 'active', 0, 0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO tasks (id, project_id, tenant_id, owner_user_id, title,
                                status, created_at, updated_at)
             VALUES ('task-a', 'proj-a', 'tenant-a', NULL, 'T', 'drafted', 0, 0)",
            [],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();
}

async fn insert_session(pool: &DbPool, id: &str, user: &str, cost: i64, tool_calls: i64) {
    let id = id.to_string();
    let user = user.to_string();
    pool.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, state,
                                    project_id, user_id, task_id,
                                    cost_cents, tool_calls)
             VALUES (?, ?, ?, 'FINISHED', 'proj-a', ?, 'task-a', ?, ?)",
            params![id, MAY_2026_MICROS, MAY_2026_MICROS, user, cost, tool_calls],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn phase5_user_cost_reconciliation_harness() {
    let pool = db::open(":memory:").await.expect("open db");
    seed_corpus(&pool).await;
    let events = Arc::new(SqliteEventStore::new(pool.clone()));

    // Seed 9 sessions across 3 users with known totals per user:
    //   user-alpha  : 100 + 200 + 300 = 600 cents, 3 sessions, 1+2+3 = 6 calls
    //   user-bravo  : 50 + 150 + 250  = 450 cents, 3 sessions, 1+1+1 = 3 calls
    //   user-charlie: 10 + 20 + 30    = 60 cents,  3 sessions, 1+1+1 = 3 calls
    for (i, (sid, user, cost, calls)) in [
        ("sess-a1", "user-alpha", 100i64, 1i64),
        ("sess-a2", "user-alpha", 200, 2),
        ("sess-a3", "user-alpha", 300, 3),
        ("sess-b1", "user-bravo", 50, 1),
        ("sess-b2", "user-bravo", 150, 1),
        ("sess-b3", "user-bravo", 250, 1),
        ("sess-c1", "user-charlie", 10, 1),
        ("sess-c2", "user-charlie", 20, 1),
        ("sess-c3", "user-charlie", 30, 1),
    ]
    .iter()
    .enumerate()
    {
        let _ = i;
        insert_session(&pool, sid, user, *cost, *calls).await;
    }

    // ---------- 1. Clean state: flush → reconcile → 0 drift ----------
    let writer = NearlineWriter::new(pool.clone());
    writer.flush().await.expect("nearline flush");

    let reconciler = ReconciliationJob::new(pool.clone(), events.clone());
    let report = reconciler.run("202605").await.expect("reconcile");
    assert_eq!(
        report.drifted_rows, 0,
        "freshly-flushed ledger must reconcile with zero drift; got {report:?}"
    );
    assert_eq!(report.rows_checked, 3, "3 users → 3 rows checked");

    // Spot-check ledger totals match source: user-alpha = 600 cents.
    let (alpha_cents, alpha_count): (i64, i64) = pool
        .with_conn(|conn| {
            conn.query_row(
                "SELECT cost_cents, session_count FROM user_cost_ledger
                 WHERE user_id = 'user-alpha' AND month_yyyymm = '202605'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
        .await
        .unwrap();
    assert_eq!(alpha_cents, 600);
    assert_eq!(alpha_count, 3);

    // ---------- 2. Inject drift > 0.5% threshold → drift detected ----------
    // Corrupt user-alpha's ledger row by 10 cents (~1.67% of 600).
    pool.with_conn(|conn| {
        conn.execute(
            "UPDATE user_cost_ledger
             SET cost_cents = cost_cents + 10
             WHERE user_id = 'user-alpha' AND month_yyyymm = '202605'",
            [],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();

    let drifted_report = reconciler.run("202605").await.expect("reconcile drift");
    assert_eq!(drifted_report.drifted_rows, 1);
    let finding = &drifted_report.drifts[0];
    assert_eq!(finding.user_id, "user-alpha");
    assert_eq!(finding.expected_cost_cents, 600);
    assert_eq!(finding.observed_cost_cents, 610);
    // delta_pct uses (|expected - observed| / expected) — for 10/600 ≈ 0.0167.
    assert!(
        (finding.delta_pct - 10.0 / 600.0).abs() < 1e-9,
        "delta_pct must be precise: got {}",
        finding.delta_pct
    );

    // Drift Misc event landed on the synthesized `audit:tenant-a` session.
    let evs = events
        .query(
            "audit:tenant-a",
            EventQuery {
                event_type: Some(EventType::Misc),
                after_id: None,
                limit: Some(50),
            },
        )
        .await
        .expect("query Misc");
    assert!(
        evs.iter().any(|e| {
            e.data.get("kind").and_then(|v| v.as_str()) == Some("user_cost_reconciliation_drift")
                && e.data.get("user_id").and_then(|v| v.as_str()) == Some("user-alpha")
                && e.data.get("expected_cost_cents").and_then(|v| v.as_i64()) == Some(600)
                && e.data.get("observed_cost_cents").and_then(|v| v.as_i64()) == Some(610)
        }),
        "user_cost_reconciliation_drift Misc event must carry full finding fields; got: {evs:?}"
    );

    // ---------- 3. Drift floor: < 0.5% must NOT trigger ----------
    // Reset alpha to clean state then introduce a 2-cent corruption
    // (2/600 ≈ 0.33% — under the 0.5% threshold).
    pool.with_conn(|conn| {
        conn.execute(
            "UPDATE user_cost_ledger
             SET cost_cents = 602
             WHERE user_id = 'user-alpha' AND month_yyyymm = '202605'",
            [],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();
    let sub_threshold_report = reconciler
        .run("202605")
        .await
        .expect("reconcile sub-threshold");
    assert_eq!(
        sub_threshold_report.drifted_rows, 0,
        "drift under 0.5% must NOT trigger a finding (NFR-5.4 floor); got {:?}",
        sub_threshold_report
    );
}
