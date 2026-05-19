//! `seasoned-hand curator <status|run|review ...>`.

use anyhow::{Result, anyhow};
use clap::Subcommand;
use rusqlite::OptionalExtension;
use seasoned_hand_core::db;
use serde::Serialize;

#[derive(Debug, Subcommand)]
pub enum CuratorCmd {
    /// Show queue and decision counters.
    Status {
        #[arg(long)]
        project: Option<String>,
    },
    /// Trigger a manual run request marker (or preview with --dry-run).
    Run {
        #[arg(long)]
        project: String,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
    },
    /// Review queue operations.
    #[command(subcommand)]
    Review(CuratorReviewCmd),
}

#[derive(Debug, Subcommand)]
pub enum CuratorReviewCmd {
    /// List review queue rows.
    List {
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        state: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Approve a queued decision.
    Approve {
        queue_id: String,
        #[arg(long)]
        reviewer: Option<String>,
        #[arg(long)]
        note: Option<String>,
    },
    /// Reject a queued decision.
    Reject {
        queue_id: String,
        #[arg(long)]
        reviewer: Option<String>,
        #[arg(long)]
        note: Option<String>,
    },
    /// Suppress a queued decision for ttl-days.
    Suppress {
        queue_id: String,
        #[arg(long, default_value_t = 30)]
        ttl_days: u32,
        #[arg(long)]
        reviewer: Option<String>,
        #[arg(long)]
        note: Option<String>,
    },
}

#[derive(Debug, Serialize)]
struct CuratorStatus {
    project: Option<String>,
    queue_pending: i64,
    queue_suppressed: i64,
    decisions_applied: i64,
    decisions_queued_review: i64,
}

#[derive(Debug, Serialize)]
struct QueueRow {
    id: String,
    decision_id: String,
    project_id: String,
    queue_reason: String,
    severity: String,
    state: String,
    reviewer: Option<String>,
    reviewer_note: Option<String>,
    created_at: i64,
    resolved_at: Option<i64>,
}

pub async fn run(cmd: CuratorCmd, json: bool) -> Result<()> {
    let pool = db::open(&database_url()).await?;
    match cmd {
        CuratorCmd::Status { project } => {
            let project_for_db = project.clone();
            let status = pool
                .with_conn(move |conn| -> Result<CuratorStatus> {
                    let queue_pending: i64 = conn.query_row(
                        "SELECT COUNT(*) FROM curator_review_queue WHERE state='pending'
                         AND (?1 IS NULL OR project_id = ?1)",
                        [project_for_db.clone()],
                        |r| r.get(0),
                    )?;
                    let queue_suppressed: i64 = conn.query_row(
                        "SELECT COUNT(*) FROM curator_review_queue WHERE state='suppressed'
                         AND (?1 IS NULL OR project_id = ?1)",
                        [project_for_db.clone()],
                        |r| r.get(0),
                    )?;
                    let decisions_applied: i64 = conn.query_row(
                        "SELECT COUNT(*) FROM curator_decisions WHERE status='applied'
                         AND (?1 IS NULL OR project_id = ?1)",
                        [project_for_db.clone()],
                        |r| r.get(0),
                    )?;
                    let decisions_queued_review: i64 = conn.query_row(
                        "SELECT COUNT(*) FROM curator_decisions WHERE status='queued_review'
                         AND (?1 IS NULL OR project_id = ?1)",
                        [project_for_db.clone()],
                        |r| r.get(0),
                    )?;
                    Ok(CuratorStatus {
                        project: project_for_db,
                        queue_pending,
                        queue_suppressed,
                        decisions_applied,
                        decisions_queued_review,
                    })
                })
                .await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                println!(
                    "queue_pending={} queue_suppressed={} applied={} queued_review={}",
                    status.queue_pending,
                    status.queue_suppressed,
                    status.decisions_applied,
                    status.decisions_queued_review
                );
            }
        }
        CuratorCmd::Run { project, dry_run } => {
            let project_for_db = project.clone();
            let out = pool
                .with_conn(move |conn| -> Result<serde_json::Value> {
                    let pending: i64 = conn.query_row(
                        "SELECT COUNT(*) FROM curator_review_queue WHERE project_id=?1 AND state='pending'",
                        [project_for_db.clone()],
                        |r| r.get(0),
                    )?;
                    if !dry_run {
                        let now = now_micros();
                        conn.execute(
                            "INSERT INTO session_search_index (event_id, session_id, timestamp, event_type, source, searchable_text)
                             VALUES (?1, ?2, ?3, 'Misc', 'curator_cli', ?4)",
                            rusqlite::params![
                                next_event_id(conn)?,
                                format!("curator:{project_for_db}"),
                                now,
                                format!("manual_run_requested project={project_for_db}")
                            ],
                        )?;
                    }
                    Ok(serde_json::json!({
                        "project": project_for_db,
                        "dry_run": dry_run,
                        "pending_review": pending
                    }))
                })
                .await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else if dry_run {
                println!("dry-run: {}", out);
            } else {
                println!("queued manual curator run request: {}", out);
            }
        }
        CuratorCmd::Review(review_cmd) => match review_cmd {
            CuratorReviewCmd::List {
                project,
                state,
                limit,
            } => {
                let rows = pool
                    .with_conn(move |conn| -> Result<Vec<QueueRow>> {
                        let mut stmt = conn.prepare(
                            "SELECT id, decision_id, project_id, queue_reason, severity, state, reviewer, reviewer_note, created_at, resolved_at
                             FROM curator_review_queue
                             WHERE (?1 IS NULL OR project_id = ?1)
                               AND (?2 IS NULL OR state = ?2)
                             ORDER BY created_at DESC, id ASC
                             LIMIT ?3",
                        )?;
                        let mut q = stmt.query(rusqlite::params![
                            project,
                            state,
                            i64::try_from(limit.max(1)).unwrap_or(50)
                        ])?;
                        let mut out = Vec::new();
                        while let Some(row) = q.next()? {
                            out.push(QueueRow {
                                id: row.get(0)?,
                                decision_id: row.get(1)?,
                                project_id: row.get(2)?,
                                queue_reason: row.get(3)?,
                                severity: row.get(4)?,
                                state: row.get(5)?,
                                reviewer: row.get(6)?,
                                reviewer_note: row.get(7)?,
                                created_at: row.get(8)?,
                                resolved_at: row.get(9)?,
                            });
                        }
                        Ok(out)
                    })
                    .await?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&rows)?);
                } else {
                    for row in rows {
                        println!(
                            "{}\t{}\t{}\t{}\t{}",
                            row.id, row.project_id, row.state, row.severity, row.queue_reason
                        );
                    }
                }
            }
            CuratorReviewCmd::Approve {
                queue_id,
                reviewer,
                note,
            } => {
                transition(
                    &pool, &queue_id, "approved", "applied", reviewer, note, None,
                )
                .await?;
                print_transition(json, "approved")?;
            }
            CuratorReviewCmd::Reject {
                queue_id,
                reviewer,
                note,
            } => {
                transition(
                    &pool, &queue_id, "rejected", "rejected", reviewer, note, None,
                )
                .await?;
                print_transition(json, "rejected")?;
            }
            CuratorReviewCmd::Suppress {
                queue_id,
                ttl_days,
                reviewer,
                note,
            } => {
                transition(
                    &pool,
                    &queue_id,
                    "suppressed",
                    "suppressed",
                    reviewer,
                    note,
                    Some(ttl_days),
                )
                .await?;
                print_transition(json, "suppressed")?;
            }
        },
    }
    Ok(())
}

async fn transition(
    pool: &db::DbPool,
    queue_id: &str,
    queue_state: &str,
    decision_status: &str,
    reviewer: Option<String>,
    note: Option<String>,
    suppress_ttl_days: Option<u32>,
) -> Result<()> {
    let queue_id = queue_id.to_string();
    let queue_state = queue_state.to_string();
    let decision_status = decision_status.to_string();
    pool.with_conn(move |conn| -> Result<()> {
        conn.execute_batch("BEGIN IMMEDIATE;")?;
        let now = now_micros();
        let row: Option<(String, String)> = conn
            .query_row(
                "SELECT decision_id, state FROM curator_review_queue WHERE id=?1",
                [queue_id.clone()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((decision_id, state)) = row else {
            conn.execute_batch("ROLLBACK;")?;
            return Err(anyhow!("queue item not found"));
        };
        if state != "pending" {
            conn.execute_batch("ROLLBACK;")?;
            return Err(anyhow!("queue item is not pending"));
        }
        let note_out = if queue_state == "suppressed" {
            let ttl = suppress_ttl_days.unwrap_or(30);
            let until = now.saturating_add(i64::from(ttl) * 86_400_000_000_i64);
            format!("suppress_until={until};{}", note.unwrap_or_default())
        } else {
            note.unwrap_or_default()
        };
        conn.execute(
            "UPDATE curator_review_queue
             SET state=?1, reviewer=?2, reviewer_note=?3, resolved_at=?4
             WHERE id=?5",
            rusqlite::params![queue_state, reviewer, note_out, now, queue_id],
        )?;
        conn.execute(
            "UPDATE curator_decisions SET status=?1 WHERE id=?2",
            rusqlite::params![decision_status, decision_id],
        )?;
        conn.execute_batch("COMMIT;")?;
        Ok(())
    })
    .await
}

fn print_transition(json: bool, state: &str) -> Result<()> {
    if json {
        println!("{}", serde_json::json!({"ok": true, "state": state}));
    } else {
        println!("updated queue state -> {state}");
    }
    Ok(())
}

fn database_url() -> String {
    std::env::var("SH_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| "sqlite:./data/seasoned-hand.db".to_string())
}

fn now_micros() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => (d.as_secs() as i64) * 1_000_000 + (d.subsec_micros() as i64),
        Err(_) => 0,
    }
}

fn next_event_id(conn: &rusqlite::Connection) -> Result<i64> {
    let next: i64 = conn.query_row(
        "SELECT COALESCE(MAX(event_id), 0) + 1 FROM session_search_index",
        [],
        |r| r.get(0),
    )?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[serial_test::serial(SH_DATABASE_URL)]
    async fn review_transition_flow() {
        let tmp = tempfile::tempdir().expect("tmp");
        let db_url = format!("sqlite:{}", tmp.path().join("curator-review.db").display());
        unsafe { std::env::set_var("SH_DATABASE_URL", &db_url) };
        let pool = db::open(&db_url).await.expect("db");
        pool.with_conn(|conn| {
            conn.execute(
                "INSERT INTO curator_decisions (
                    id, tenant_id, project_id, cycle_id, decision_type, subject_kind, subject_id, confidence,
                    rationale_json, evidence_json, status, failure_category, created_at
                ) VALUES (
                    'cd-1', NULL, 'p1', 'cycle-1', 'merge', 'revision', 'rev-1', 0.4,
                    '{}', '{}', 'queued_review', NULL, 1
                )",
                [],
            )
            .expect("insert decision");
            conn.execute(
                "INSERT INTO curator_review_queue (
                    id, tenant_id, decision_id, project_id, queue_reason, severity, state, reviewer, reviewer_note, resolved_at, created_at
                ) VALUES (
                    'rq-1', NULL, 'cd-1', 'p1', 'low_confidence', 'high', 'pending', NULL, NULL, NULL, 1
                )",
                [],
            )
            .expect("insert queue");
        })
        .await;

        run(
            CuratorCmd::Review(CuratorReviewCmd::Approve {
                queue_id: "rq-1".into(),
                reviewer: Some("ops".into()),
                note: Some("looks good".into()),
            }),
            true,
        )
        .await
        .expect("approve");

        let status: String = pool
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT status FROM curator_decisions WHERE id='cd-1'",
                    [],
                    |r| r.get(0),
                )
                .expect("decision status")
            })
            .await;
        assert_eq!(status, "applied");

        run(
            CuratorCmd::Review(CuratorReviewCmd::List {
                project: Some("p1".into()),
                state: Some("approved".into()),
                limit: 10,
            }),
            true,
        )
        .await
        .expect("list");

        unsafe { std::env::remove_var("SH_DATABASE_URL") };
    }
}
