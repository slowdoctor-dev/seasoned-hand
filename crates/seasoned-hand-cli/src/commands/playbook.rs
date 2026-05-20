//! `seasoned-hand playbook <list|show|delete>`.

use anyhow::{Result, anyhow};
use clap::Subcommand;
use rusqlite::OptionalExtension;
use seasoned_hand_core::db;
use seasoned_hand_core::time::now_micros;
use serde::Serialize;

#[derive(Debug, Subcommand)]
pub enum PlaybookCmd {
    /// List playbooks with status and outcome counters.
    List {
        #[arg(long, default_value_t = false)]
        include_archived: bool,
    },
    /// Show one playbook by id.
    Show { id: String },
    /// Soft-delete a playbook (`status='archived'`).
    Delete { id: String },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct PlaybookRow {
    id: String,
    title: String,
    content: String,
    trigger_keywords: String,
    success_count: i64,
    failure_count: i64,
    status: String,
    version: i64,
    created_at: i64,
    updated_at: i64,
}

pub async fn run(cmd: PlaybookCmd, json: bool) -> Result<()> {
    let pool = db::open(&database_url()).await?;

    match cmd {
        PlaybookCmd::List { include_archived } => {
            let rows = pool
                .with_conn(move |conn| -> Result<Vec<PlaybookRow>> {
                    let sql = if include_archived {
                        "SELECT id, title, content, trigger_keywords, success_count, failure_count, status,
                                version, created_at, updated_at
                         FROM playbooks
                         ORDER BY updated_at DESC, id ASC"
                    } else {
                        "SELECT id, title, content, trigger_keywords, success_count, failure_count, status,
                                version, created_at, updated_at
                         FROM playbooks
                         WHERE status != 'archived'
                         ORDER BY updated_at DESC, id ASC"
                    };
                    let mut stmt = conn.prepare(sql)?;
                    let mapped = stmt.query_map([], |r| {
                        Ok(PlaybookRow {
                            id: r.get(0)?,
                            title: r.get(1)?,
                            content: r.get(2)?,
                            trigger_keywords: r.get(3)?,
                            success_count: r.get(4)?,
                            failure_count: r.get(5)?,
                            status: r.get(6)?,
                            version: r.get(7)?,
                            created_at: r.get(8)?,
                            updated_at: r.get(9)?,
                        })
                    })?;
                    let mut out = Vec::new();
                    for row in mapped {
                        out.push(row?);
                    }
                    Ok(out)
                })
                .await?;

            if json {
                println!("{}", serde_json::to_string_pretty(&rows)?);
            } else {
                for row in rows {
                    println!(
                        "{}\t{}\tstatus={}\tsuccess={}\tfailure={}",
                        row.id, row.title, row.status, row.success_count, row.failure_count
                    );
                }
            }
        }
        PlaybookCmd::Show { id } => {
            let row = pool
                .with_conn(move |conn| -> Result<Option<PlaybookRow>> {
                    let row = conn
                        .query_row(
                            "SELECT id, title, content, trigger_keywords, success_count, failure_count, status,
                                    version, created_at, updated_at
                             FROM playbooks WHERE id = ?",
                            [id],
                            |r| {
                                Ok(PlaybookRow {
                                    id: r.get(0)?,
                                    title: r.get(1)?,
                                    content: r.get(2)?,
                                    trigger_keywords: r.get(3)?,
                                    success_count: r.get(4)?,
                                    failure_count: r.get(5)?,
                                    status: r.get(6)?,
                                    version: r.get(7)?,
                                    created_at: r.get(8)?,
                                    updated_at: r.get(9)?,
                                })
                            },
                        )
                        .optional()?;
                    Ok(row)
                })
                .await?;
            let Some(row) = row else {
                return Err(anyhow!("playbook not found"));
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&row)?);
            } else {
                println!("id: {}", row.id);
                println!("title: {}", row.title);
                println!("status: {}", row.status);
                println!("version: {}", row.version);
                println!("success_count: {}", row.success_count);
                println!("failure_count: {}", row.failure_count);
                println!("trigger_keywords: {}", row.trigger_keywords);
                println!();
                println!("{}", row.content);
            }
        }
        PlaybookCmd::Delete { id } => {
            let now = now_micros();
            let updated = pool
                .with_conn(move |conn| -> Result<bool> {
                    let n = conn.execute(
                        "UPDATE playbooks
                         SET status = 'archived', updated_at = ?
                         WHERE id = ? AND status != 'archived'",
                        rusqlite::params![now, id],
                    )?;
                    Ok(n > 0)
                })
                .await?;
            if !updated {
                return Err(anyhow!("playbook not found or already archived"));
            }
            if json {
                println!("{}", serde_json::json!({"archived": true}));
            } else {
                println!("archived playbook");
            }
        }
    }

    Ok(())
}

fn database_url() -> String {
    std::env::var("SH_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| "sqlite:./data/seasoned-hand.db".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Serial-group key matches the env var the test mutates; the `sop` test
    // uses the same key so they're mutually exclusive under cargo's parallel
    // test runner (Phase 3 REVIEW iter-1 F1).
    #[tokio::test]
    #[serial_test::serial(SH_DATABASE_URL)]
    async fn lifecycle_list_show_delete() {
        let tmp = tempfile::tempdir().unwrap();
        let db_url = format!("sqlite:{}", tmp.path().join("playbook.db").display());
        unsafe { std::env::set_var("SH_DATABASE_URL", &db_url) };

        let pool = db::open(&db_url).await.unwrap();
        pool.with_conn(|conn| {
            conn.execute(
                "INSERT INTO projects (id, tenant_id, status, title, created_at, updated_at)
                 VALUES ('p1', 'legacy-default', 'active', 'P1', 1, 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO tasks (id, project_id, tenant_id, status, title, created_at, updated_at)
                 VALUES ('t1', 'p1', 'legacy-default', 'running', 'Task 1', 1, 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO playbooks (id, tenant_id, title, content_path, schema_version, source_task_id,
                 created_at, updated_at, trigger_keywords, content, success_count, failure_count, avg_duration_ms,
                 avg_tool_calls, status, version)
                 VALUES ('pb-1', NULL, 'Deploy', '', 1, 't1', 1, 1, '[\"deploy\"]',
                 'checklist', 2, 1, NULL, NULL, 'active', 1)",
                [],
            )
            .unwrap();
        })
        .await;

        run(
            PlaybookCmd::List {
                include_archived: false,
            },
            true,
        )
        .await
        .unwrap();
        run(PlaybookCmd::Show { id: "pb-1".into() }, true)
            .await
            .unwrap();
        run(PlaybookCmd::Delete { id: "pb-1".into() }, true)
            .await
            .unwrap();

        let status: String = pool
            .with_conn(|conn| {
                conn.query_row("SELECT status FROM playbooks WHERE id='pb-1'", [], |r| {
                    r.get(0)
                })
                .unwrap()
            })
            .await;
        assert_eq!(status, "archived");

        unsafe { std::env::remove_var("SH_DATABASE_URL") };
    }
}
