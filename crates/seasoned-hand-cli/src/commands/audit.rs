use anyhow::Result;
use clap::Subcommand;

use crate::client::{ApiClient, into_anyhow};

#[derive(Debug, Subcommand)]
pub enum AuditCmd {
    /// List audit rows (tenant-scoped).
    List {
        #[arg(long)]
        actor: Option<String>,
        #[arg(long)]
        action: Option<String>,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long = "task")]
        task_id: Option<String>,
    },
}

pub async fn run(cmd: AuditCmd, client: &ApiClient, json: bool) -> Result<()> {
    match cmd {
        AuditCmd::List {
            actor,
            action,
            since,
            limit,
            task_id,
        } => {
            let since_micros = since.as_deref().map(parse_since_to_micros).transpose()?;
            let rows = into_anyhow(
                client
                    .list_audit(
                        actor.as_deref(),
                        action.as_deref(),
                        since_micros,
                        limit,
                        task_id.as_deref(),
                    )
                    .await,
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rows)?);
            } else if rows.is_empty() {
                println!("(no audit rows)");
            } else {
                println!(
                    "{:<26}  {:<16}  {:<16}  {:<38}",
                    "CREATED_AT", "ACTION", "ACTOR", "RESOURCE"
                );
                for r in rows {
                    println!(
                        "{:<26}  {:<16}  {:<16}  {:<38}",
                        r.created_at,
                        r.action,
                        r.actor_user_id,
                        format!("{}:{}", r.resource_type, r.resource_id)
                    );
                }
            }
        }
    }
    Ok(())
}

fn parse_since_to_micros(value: &str) -> Result<i64> {
    value.parse::<i64>().map_err(Into::into)
}
