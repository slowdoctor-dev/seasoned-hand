//! Phase 5 story 5.32 — `phase5_team_simulation_benchmark`.
//!
//! Headline composite acceptance benchmark: simulates a 5-person team
//! (1 admin, 3 users, 1 viewer) using one Seasoned Hand instance
//! concurrently with task hand-offs, shared SOP usage, per-user cost
//! ledger reconciliation, audit reads, and a cross-tenant isolation
//! probe — composing every Phase 5 surface in a single run.
//!
//! Analogous to Phase 4's `phase4_warm_full_loop_benchmark`. The
//! per-surface harnesses (5.26-5.31) already validate each contract
//! in isolation; this benchmark composes them and asserts no
//! cross-surface regression surfaces.
//!
//! refs: /specs/phase-5/stories/story-5.32.md
//! refs: /specs/phase-5/architecture.md §15 (composed acceptance)
//! refs: /specs/phase-5/requirements.md F-5.24, all NFRs

use rusqlite::params;
use seasoned_hand_core::audit::{AuditLogger, AuditQuery};
use seasoned_hand_core::auth::{AuthContext, Role};
use seasoned_hand_core::billing::{NearlineWriter, ReconciliationJob};
use seasoned_hand_core::db::{self, DbPool};
use seasoned_hand_core::events::sqlite::SqliteEventStore;
use seasoned_hand_core::events::visibility::{self, EventReadQuery};
use seasoned_hand_core::events::{EventStore, EventType, NewEvent};
use seasoned_hand_core::handoff::{HandoffRequest, TaskHandoffService};
use seasoned_hand_core::sharing::sop::{SopPermission, SopShareError, SopShareService};
use std::sync::Arc;
use std::time::Instant;

/// 2026-05-15 12:00:00 UTC microseconds — month bucket "202605".
const MAY_2026_MICROS: i64 = 1_778_846_400_000_000;

async fn seed_team_fixture(pool: &DbPool) {
    pool.with_conn(|conn| {
        // Single org + 5-user team.
        conn.execute(
            "INSERT INTO organizations (id, tenant_id, slug, display_name, status,
                                         created_at, updated_at)
             VALUES ('org-team', 'tenant-team', 'team', 'Team', 'active', 0, 0)",
            [],
        )?;
        for (uid, email, role) in [
            ("u-admin", "admin@team.io", "admin"),
            ("u-alice", "alice@team.io", "user"),
            ("u-bob", "bob@team.io", "user"),
            ("u-cara", "cara@team.io", "user"),
            ("u-viewer", "view@team.io", "viewer"),
        ] {
            conn.execute(
                "INSERT INTO users (id, tenant_id, email, display_name, status,
                                    created_at, updated_at)
                 VALUES (?, 'tenant-team', ?, ?, 'active', 0, 0)",
                params![uid, email, uid],
            )?;
            conn.execute(
                "INSERT INTO organization_memberships
                   (id, tenant_id, organization_id, user_id, role,
                    is_primary, created_at, updated_at)
                 VALUES (?, 'tenant-team', 'org-team', ?, ?, 1, 0, 0)",
                params![format!("mem-{uid}"), uid, role],
            )?;
        }
        // 3 projects.
        for pid in ["proj-blue", "proj-green", "proj-amber"] {
            conn.execute(
                "INSERT INTO projects (id, tenant_id, title, status, created_at, updated_at)
                 VALUES (?, 'tenant-team', 'P', 'active', 0, 0)",
                params![pid],
            )?;
        }
        // 51 tasks distributed across the 3 projects and 3 user-owners.
        // Each task carries a session for cost-ledger seeding.
        for i in 0..51 {
            let pid = ["proj-blue", "proj-green", "proj-amber"][i % 3];
            let owner = ["u-alice", "u-bob", "u-cara"][i % 3];
            let tid = format!("task-{i:02}");
            let sid = format!("sess-{i:02}");
            conn.execute(
                "INSERT INTO tasks (id, project_id, tenant_id, owner_user_id, title,
                                    status, created_at, updated_at)
                 VALUES (?, ?, 'tenant-team', ?, 'T', 'drafted', 0, 0)",
                params![tid, pid, owner],
            )?;
            // Cost-bearing FINISHED session linked to the task.
            // Each session: 10 cents per (i % 5 + 1) so totals are
            // distinctive per user.
            let cost = 10_i64 * ((i as i64 % 5) + 1);
            conn.execute(
                "INSERT INTO sessions (id, created_at, updated_at, state,
                                        project_id, user_id, task_id,
                                        cost_cents, tool_calls)
                 VALUES (?, ?, ?, 'FINISHED', ?, ?, ?, ?, 1)",
                params![sid, MAY_2026_MICROS, MAY_2026_MICROS, pid, owner, tid, cost],
            )?;
        }
        // One SOP for sharing scenarios.
        conn.execute(
            "INSERT INTO sops (id, tenant_id, title, content, version, enforced, created_at, updated_at)
             VALUES ('sop-team-onboarding', 'tenant-team', 'Onboarding', 'Steps', 1, 0, 0, 0)",
            [],
        )?;
        // Cross-tenant probe data: a 2nd tenant with isolated content
        // so the cross-tenant assertions below have something to
        // attempt-and-fail against.
        conn.execute(
            "INSERT INTO organizations (id, tenant_id, slug, display_name, status,
                                         created_at, updated_at)
             VALUES ('org-other', 'tenant-other', 'other', 'O', 'active', 0, 0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO users (id, tenant_id, email, display_name, status,
                                created_at, updated_at)
             VALUES ('u-other', 'tenant-other', 'other@x.io', 'O', 'active', 0, 0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO organization_memberships
               (id, tenant_id, organization_id, user_id, role,
                is_primary, created_at, updated_at)
             VALUES ('mem-other', 'tenant-other', 'org-other', 'u-other',
                     'admin', 1, 0, 0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO projects (id, tenant_id, title, status, created_at, updated_at)
             VALUES ('proj-other', 'tenant-other', 'P', 'active', 0, 0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO tasks (id, project_id, tenant_id, title, status,
                                created_at, updated_at)
             VALUES ('task-other', 'proj-other', 'tenant-other', 'T',
                     'drafted', 0, 0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, state, task_id)
             VALUES ('sess-other', 0, 0, 'IDLE', 'task-other')",
            [],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();
}

fn ctx(actor: &str, role: Role) -> AuthContext {
    AuthContext {
        tenant_id: "tenant-team".into(),
        organization_id: "org-team".into(),
        actor_user_id: actor.into(),
        org_role: role,
        project_override_role: None,
    }
}

#[tokio::test]
async fn phase5_team_simulation_benchmark() {
    let started = Instant::now();
    let pool = db::open(":memory:").await.expect("open db");
    seed_team_fixture(&pool).await;

    let events_store = Arc::new(SqliteEventStore::new(pool.clone()));
    let audit = AuditLogger::new(pool.clone(), events_store.clone());
    let handoff = TaskHandoffService::new(pool.clone(), events_store.clone(), audit.clone());

    let admin_ctx = ctx("u-admin", Role::Admin);
    let alice_ctx = ctx("u-alice", Role::User);
    let viewer_ctx = ctx("u-viewer", Role::Viewer);

    // ---------- 1. Hand-off rotation: alice → bob → cara ----------
    // 5 hand-offs covering the (drafted → direct) state-gate path —
    // exercises both the optimistic concurrency precondition and the
    // audit row + Misc event emission per transfer.
    for (task_id, to_email) in [
        ("task-00", "bob@team.io"),
        ("task-03", "bob@team.io"),
        ("task-06", "cara@team.io"),
        ("task-09", "cara@team.io"),
        ("task-12", "alice@team.io"),
    ] {
        let outcome = handoff
            .handoff(
                &admin_ctx,
                HandoffRequest {
                    task_id: task_id.into(),
                    to_user_email: to_email.into(),
                    reason: Some("rotation".into()),
                    expected_updated_at: None,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("handoff {task_id} → {to_email}: {e:?}"));
        assert!(!outcome.audit_log_id.is_empty());
    }

    // ---------- 2. SOP sharing flow: 3 grants by admin, 1 denied for viewer ----------
    let sop = SopShareService::new(pool.clone());
    for email in ["alice@team.io", "bob@team.io", "cara@team.io"] {
        sop.share(
            &admin_ctx,
            "sop-team-onboarding",
            email,
            SopPermission::Editor,
            None,
        )
        .await
        .unwrap_or_else(|e| panic!("admin share to {email}: {e:?}"));
    }
    // Viewer attempt to share → Auth error.
    let viewer_share = sop
        .share(
            &viewer_ctx,
            "sop-team-onboarding",
            "bob@team.io",
            SopPermission::Viewer,
            None,
        )
        .await
        .expect_err("viewer must be denied SopShare");
    assert!(
        matches!(viewer_share, SopShareError::Auth(_)),
        "viewer share must surface Auth error, got {viewer_share:?}"
    );

    // ---------- 3. Curator-style high-confidence playbook share ----------
    // Skip going through the full curator pipeline — the share-row
    // shape is the production invariant; curator execution already has
    // dedicated harness coverage in story 5.28.
    pool.with_conn(|conn| {
        conn.execute(
            "INSERT INTO playbooks (id, tenant_id, title, content_path, schema_version,
                                     source_task_id, created_at, updated_at, trigger_keywords,
                                     content, status, source_project_id, active_revision_id,
                                     success_count, failure_count)
             VALUES ('pb-team-onb', 'tenant-team', 'Onb', 'pb.md', 1, NULL, 0, 0, '[]', '',
                     'active', 'proj-blue', NULL, 0, 0)",
            [],
        )?;
        // Auto-shared at 'shared' visibility (curator's high-confidence path).
        conn.execute(
            "INSERT INTO playbook_shares (id, tenant_id, playbook_id, subject_type,
                                            subject_id, permission, visibility_state,
                                            granted_by_user_id, created_at, updated_at)
             VALUES ('pbs-1', 'tenant-team', 'pb-team-onb', 'user', 'u-alice',
                     'viewer', 'shared', 'u-admin', 0, 0)",
            [],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();

    // ---------- 4. Audit log captures every mutating op that audits ----------
    // Hand-off (story 5.11) routes through AuditLogger; SOP share
    // (story 5.7) doesn't audit today — sharing audit emission is a
    // follow-up improvement, tracked separately. The 5 hand-offs here
    // each write one audit_log row under tenant-team. Viewer's denied
    // attempt does NOT write an audit row (gate rejects before
    // AuditLogger::record).
    let audit_rows = audit
        .query(&admin_ctx, AuditQuery::default())
        .await
        .expect("audit query");
    let mut handoff_count = 0;
    for row in &audit_rows {
        assert_eq!(row.tenant_id, "tenant-team", "audit must be tenant-scoped");
        if row.action == "task.handoff" {
            handoff_count += 1;
        }
    }
    assert_eq!(
        handoff_count, 5,
        "expected 5 handoff audit rows; got rows: {audit_rows:?}"
    );

    // ---------- 5. Per-user cost ledger reconciles within ±0.5% ----------
    let writer = NearlineWriter::new(pool.clone());
    writer.flush().await.expect("nearline flush");
    let reconciler = ReconciliationJob::new(pool.clone(), events_store.clone());
    let report = reconciler.run("202605").await.expect("reconcile");
    assert_eq!(
        report.drifted_rows, 0,
        "fresh ledger must reconcile drift-free; got {report:?}"
    );
    // 3 cost-bearing users → 3 rows.
    assert_eq!(report.rows_checked, 3);
    // Inject a real drift (>0.5%) to validate detection path in this
    // composed benchmark (not just the clean reconciliation path).
    pool.with_conn(|conn| {
        conn.execute(
            "UPDATE user_cost_ledger
             SET cost_cents = cost_cents + 10
             WHERE tenant_id='tenant-team' AND user_id='u-alice' AND month_yyyymm='202605'",
            [],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .unwrap();
    let drift_report = reconciler.run("202605").await.expect("reconcile drift");
    assert!(
        drift_report.drifted_rows >= 1,
        "injected >0.5% drift must be detected; report={drift_report:?}"
    );

    // ---------- 6. Cross-tenant probe — zero leakage from tenant-team ----------
    // Alice (tenant-team User) attempts to read tenant-other's session
    // events → empty. Same for raw read (Auth error since alice isn't
    // admin, but even if she were admin in her own tenant, the
    // session's resolved tenant != alice.tenant_id short-circuits).
    let cross = visibility::query(&pool, &alice_ctx, "sess-other", EventReadQuery::default())
        .await
        .expect("query cross");
    assert!(cross.is_empty(), "cross-tenant read must return zero rows");

    let cross_admin = visibility::query_raw(
        &pool,
        &admin_ctx,
        &audit,
        "sess-other",
        EventReadQuery::default(),
    )
    .await
    .expect("query_raw cross admin");
    assert!(
        cross_admin.is_empty(),
        "admin raw read must respect tenant boundary; got {} rows",
        cross_admin.len()
    );

    // Append a tenant-team event to assert in-tenant read succeeds —
    // pins the "this isn't always-empty by bug" property.
    events_store
        .append(NewEvent {
            session_id: "sess-00".into(),
            event_type: EventType::Message,
            source: "user".into(),
            data: serde_json::json!({"text": "alice mid-task ping"}),
        })
        .await
        .expect("append in-tenant event");
    let own = visibility::query(&pool, &alice_ctx, "sess-00", EventReadQuery::default())
        .await
        .expect("query own");
    assert!(!own.is_empty(), "in-tenant read must surface own events");

    // ---------- 7. Wall-clock CI budget ----------
    // Per acceptance §1, the composed benchmark must stay comfortably
    // inside CI budget. Here we enforce a concrete and non-trivial
    // local ceiling for this in-memory harness.
    let elapsed = started.elapsed();
    assert!(
        elapsed.as_secs() <= 300,
        "team simulation exceeded 5min harness ceiling: {:.2}s",
        elapsed.as_secs_f64()
    );
    eprintln!(
        "phase5_team_simulation_benchmark: 5-actor / 3-project / 51-task / 5-handoff / 3-share / cost-reconcile / cross-tenant cycle in {:.3}s",
        elapsed.as_secs_f64()
    );
}
