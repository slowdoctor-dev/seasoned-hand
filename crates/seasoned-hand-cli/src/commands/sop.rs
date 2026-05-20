//! `seasoned-hand sop <create|edit|list|show|delete>`.

use crate::client::ApiClient;
use anyhow::{Result, anyhow};
use clap::Subcommand;
use rusqlite::{OptionalExtension, params};
use seasoned_hand_core::db;
use seasoned_hand_core::sharing::sop::SopShareService;
use seasoned_hand_core::time::now_micros;
use serde::Serialize;

#[derive(Debug, Subcommand)]
pub enum SopCmd {
    /// Create a new SOP row.
    Create {
        id: String,
        title: String,
        content: String,
        #[arg(long, default_value_t = true)]
        enforced: bool,
    },
    /// Edit an existing SOP; increments version by 1.
    Edit {
        id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        content: Option<String>,
        #[arg(long)]
        enforced: Option<bool>,
    },
    /// List SOPs ordered by updated_at desc.
    List,
    /// Show one SOP by id.
    Show { id: String },
    /// Hard-delete one SOP by id.
    Delete { id: String },
    /// Share an SOP with a user.
    Share {
        sop_id: String,
        #[arg(long = "user")]
        user_email: String,
        #[arg(long)]
        permission: String,
    },
    /// Remove SOP sharing from a user.
    Unshare {
        sop_id: String,
        #[arg(long = "user")]
        user_email: String,
    },
    /// List share rows for an SOP.
    Shares { sop_id: String },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct SopRow {
    id: String,
    title: String,
    content: String,
    version: i64,
    enforced: bool,
    created_at: i64,
    updated_at: i64,
}

pub async fn run(cmd: SopCmd, client: &ApiClient, json: bool) -> Result<()> {
    let pool = db::open(&database_url()).await?;

    match cmd {
        SopCmd::Create {
            id,
            title,
            content,
            enforced,
        } => {
            let now = now_micros();
            let sop_id = id.clone();
            pool.with_conn(move |conn| -> Result<()> {
                conn.execute(
                    "INSERT INTO sops (id, title, content, version, enforced, created_at, updated_at)
                     VALUES (?, ?, ?, 1, ?, ?, ?)",
                    params![id, title, content, bool_to_i64(enforced), now, now],
                )?;
                Ok(())
            })
            .await?;
            let tenant_id =
                std::env::var("SH_TENANT_ID").unwrap_or_else(|_| "legacy-default".to_string());
            let owner_user_id = std::env::var("SH_ACTOR_USER_ID")
                .unwrap_or_else(|_| "user-cli-operator".to_string());
            SopShareService::new(pool.clone())
                .ensure_default_owner(&tenant_id, &sop_id, &owner_user_id)
                .await?;
            if json {
                println!("{}", serde_json::json!({"created": true}));
            } else {
                println!("created sop");
            }
        }
        SopCmd::Edit {
            id,
            title,
            content,
            enforced,
        } => {
            let now = now_micros();
            let updated = pool
                .with_conn(move |conn| -> Result<bool> {
                    let current: Option<SopRow> = conn
                        .query_row(
                            "SELECT id, title, content, version, enforced, created_at, updated_at
                             FROM sops WHERE id = ?",
                            [id.as_str()],
                            |r| {
                                Ok(SopRow {
                                    id: r.get(0)?,
                                    title: r.get(1)?,
                                    content: r.get(2)?,
                                    version: r.get(3)?,
                                    enforced: r.get::<_, i64>(4)? != 0,
                                    created_at: r.get(5)?,
                                    updated_at: r.get(6)?,
                                })
                            },
                        )
                        .optional()?;

                    let Some(current) = current else {
                        return Ok(false);
                    };

                    let next_title = title.unwrap_or(current.title);
                    let next_content = content.unwrap_or(current.content);
                    let next_enforced = enforced.unwrap_or(current.enforced);
                    let next_version = current.version + 1;

                    conn.execute(
                        "UPDATE sops
                         SET title = ?, content = ?, enforced = ?, version = ?, updated_at = ?
                         WHERE id = ?",
                        params![
                            next_title,
                            next_content,
                            bool_to_i64(next_enforced),
                            next_version,
                            now,
                            id
                        ],
                    )?;
                    Ok(true)
                })
                .await?;

            if !updated {
                return Err(anyhow!("sop not found"));
            }
            if json {
                println!("{}", serde_json::json!({"updated": true}));
            } else {
                println!("updated sop");
            }
        }
        SopCmd::List => {
            let rows = pool
                .with_conn(|conn| -> Result<Vec<SopRow>> {
                    let mut stmt = conn.prepare(
                        "SELECT id, title, content, version, enforced, created_at, updated_at
                         FROM sops ORDER BY updated_at DESC, id ASC",
                    )?;
                    let mapped = stmt.query_map([], |r| {
                        Ok(SopRow {
                            id: r.get(0)?,
                            title: r.get(1)?,
                            content: r.get(2)?,
                            version: r.get(3)?,
                            enforced: r.get::<_, i64>(4)? != 0,
                            created_at: r.get(5)?,
                            updated_at: r.get(6)?,
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
                        "{}\t{}\tv{}\tenforced={}",
                        row.id, row.title, row.version, row.enforced
                    );
                }
            }
        }
        SopCmd::Show { id } => {
            let row = pool
                .with_conn(move |conn| -> Result<Option<SopRow>> {
                    let row = conn
                        .query_row(
                            "SELECT id, title, content, version, enforced, created_at, updated_at
                             FROM sops WHERE id = ?",
                            [id],
                            |r| {
                                Ok(SopRow {
                                    id: r.get(0)?,
                                    title: r.get(1)?,
                                    content: r.get(2)?,
                                    version: r.get(3)?,
                                    enforced: r.get::<_, i64>(4)? != 0,
                                    created_at: r.get(5)?,
                                    updated_at: r.get(6)?,
                                })
                            },
                        )
                        .optional()?;
                    Ok(row)
                })
                .await?;

            let Some(row) = row else {
                return Err(anyhow!("sop not found"));
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&row)?);
            } else {
                println!("id: {}", row.id);
                println!("title: {}", row.title);
                println!("version: {}", row.version);
                println!("enforced: {}", row.enforced);
                println!();
                println!("{}", row.content);
            }
        }
        SopCmd::Delete { id } => {
            let deleted = pool
                .with_conn(move |conn| -> Result<bool> {
                    let n = conn.execute("DELETE FROM sops WHERE id = ?", [id])?;
                    Ok(n > 0)
                })
                .await?;
            if !deleted {
                return Err(anyhow!("sop not found"));
            }
            if json {
                println!("{}", serde_json::json!({"deleted": true}));
            } else {
                println!("deleted sop");
            }
        }
        SopCmd::Share {
            sop_id,
            user_email,
            permission,
        } => {
            let row = client.sop_share(&sop_id, &user_email, &permission).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&row)?);
            } else {
                println!(
                    "shared {} -> {} ({})",
                    row.sop_id,
                    row.subject_email.unwrap_or(row.subject_id),
                    row.permission
                );
            }
        }
        SopCmd::Unshare { sop_id, user_email } => {
            client.sop_unshare(&sop_id, &user_email).await?;
            if json {
                println!("{}", serde_json::json!({"unshared": true}));
            } else {
                println!("unshared {sop_id} from {user_email}");
            }
        }
        SopCmd::Shares { sop_id } => {
            let rows = client.sop_list_shares(&sop_id).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rows)?);
            } else {
                for row in rows {
                    println!(
                        "{}\t{}\t{}",
                        row.subject_email.unwrap_or(row.subject_id),
                        row.permission,
                        row.granted_by_user_id
                    );
                }
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

fn bool_to_i64(v: bool) -> i64 {
    if v { 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Serial-group key matches the env var this test mutates; the `playbook`
    // test uses the same key so they're mutually exclusive under cargo's
    // parallel test runner (Phase 3 REVIEW iter-1 F1).
    #[tokio::test]
    #[serial_test::serial(SH_DATABASE_URL)]
    async fn create_edit_list_show_delete_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let db_url = format!("sqlite:{}", tmp.path().join("sop.db").display());
        // SAFETY: tests in this module run as one process; we set and remove env
        // around this test's calls only.
        unsafe { std::env::set_var("SH_DATABASE_URL", &db_url) };
        unsafe { std::env::set_var("SH_TENANT_ID", "legacy-default") };
        unsafe { std::env::set_var("SH_ACTOR_USER_ID", "user-cli-operator") };

        let pool = db::open(&db_url).await.unwrap();
        pool.with_conn(|conn| {
            conn.execute(
                "INSERT OR IGNORE INTO organizations (id, tenant_id, slug, display_name, status, created_at, updated_at)
                 VALUES ('org-legacy-default', 'legacy-default', 'legacy', 'Legacy Org', 'active', 1, 1)",
                [],
            )?;
            conn.execute(
                "INSERT OR IGNORE INTO users (id, tenant_id, email, display_name, status, created_at, updated_at)
                 VALUES ('user-cli-operator', 'legacy-default', 'cli@example.test', 'CLI Operator', 'active', 1, 1)",
                [],
            )?;
            Ok::<(), rusqlite::Error>(())
        })
        .await
        .unwrap();

        let client = ApiClient::new("http://127.0.0.1:3000");
        run(
            SopCmd::Create {
                id: "sop-1".into(),
                title: "Deploy".into(),
                content: "Use checklist".into(),
                enforced: true,
            },
            &client,
            true,
        )
        .await
        .unwrap();

        run(
            SopCmd::Edit {
                id: "sop-1".into(),
                title: Some("Deploy v2".into()),
                content: Some("Use checklist and verify".into()),
                enforced: Some(false),
            },
            &client,
            true,
        )
        .await
        .unwrap();

        let row = pool
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT title, content, version, enforced FROM sops WHERE id='sop-1'",
                    [],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, i64>(2)?,
                            r.get::<_, i64>(3)?,
                        ))
                    },
                )
                .unwrap()
            })
            .await;
        assert_eq!(row.0, "Deploy v2");
        assert_eq!(row.1, "Use checklist and verify");
        assert_eq!(row.2, 2);
        assert_eq!(row.3, 0);

        run(SopCmd::List, &client, true).await.unwrap();
        run(SopCmd::Show { id: "sop-1".into() }, &client, true)
            .await
            .unwrap();
        run(SopCmd::Delete { id: "sop-1".into() }, &client, true)
            .await
            .unwrap();

        let exists = pool
            .with_conn(|conn| {
                conn.query_row("SELECT COUNT(*) FROM sops WHERE id='sop-1'", [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap()
            })
            .await;
        assert_eq!(exists, 0);
        unsafe {
            std::env::remove_var("SH_DATABASE_URL");
            std::env::remove_var("SH_TENANT_ID");
            std::env::remove_var("SH_ACTOR_USER_ID");
        };
    }
}
