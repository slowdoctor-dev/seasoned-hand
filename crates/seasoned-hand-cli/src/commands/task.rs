//! `seasoned-hand task <list|show|pause|resume|cancel|provenance>`.

use anyhow::{Context, Result};
use clap::Subcommand;

use crate::client::{ApiClient, into_anyhow};
use crate::format;

#[derive(Debug, Subcommand)]
pub enum TaskCmd {
    /// List tasks for a project.
    List {
        #[arg(long)]
        project: String,
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Show a single task.
    Show { id: String },
    /// Pause a running task. Durable (default) records sandbox, workspace,
    /// and event-cursor metadata so resume can rebuild even after a
    /// server restart.
    Pause {
        id: String,
        /// Disable durable pause (transient suspend only).
        #[arg(long = "non-durable")]
        non_durable: bool,
    },
    /// Resume a paused task.
    Resume { id: String },
    /// Cancel a task (state-machine widened to Drafted/Briefed/Confirmed/Running/Paused → Cancelled).
    Cancel { id: String },
    /// Print the latest deliverable's provenance manifest.
    Provenance { id: String },
}

pub async fn run(cmd: TaskCmd, client: &ApiClient, json: bool) -> Result<()> {
    match cmd {
        TaskCmd::List {
            project,
            status,
            limit,
        } => {
            let tasks = into_anyhow(client.list_tasks(&project, status.as_deref(), limit).await)
                .with_context(|| format!("list tasks for project {project}"))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&tasks)?);
            } else {
                format::print_tasks(&tasks);
            }
        }
        TaskCmd::Show { id } => {
            let task = into_anyhow(client.get_task(&id).await)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&task)?);
            } else {
                format::print_task(&task);
            }
        }
        TaskCmd::Pause { id, non_durable } => {
            into_anyhow(client.pause_task(&id, !non_durable).await)?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({"task_id": id, "paused": true, "durable": !non_durable})
                );
            } else {
                println!("paused {id} (durable={})", !non_durable);
            }
        }
        TaskCmd::Resume { id } => {
            into_anyhow(client.resume_task(&id).await)?;
            if json {
                println!("{}", serde_json::json!({"task_id": id, "resumed": true}));
            } else {
                println!("resumed {id}");
            }
        }
        TaskCmd::Cancel { id } => {
            into_anyhow(client.cancel_task(&id).await)?;
            if json {
                println!("{}", serde_json::json!({"task_id": id, "cancelled": true}));
            } else {
                println!("cancelled {id}");
            }
        }
        TaskCmd::Provenance { id } => {
            let manifest = into_anyhow(client.task_provenance(&id).await)?;
            // Provenance is structured JSON; pretty-print regardless of
            // --json so the operator can paste into a viewer either way.
            println!("{}", serde_json::to_string_pretty(&manifest)?);
        }
    }
    Ok(())
}
