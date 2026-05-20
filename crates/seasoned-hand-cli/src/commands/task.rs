//! `seasoned-hand task <list|show|new|brief|deliverable|pause|resume|cancel|handoff|provenance>`.

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
    /// Submit a new brief and (by default) block until the deliverable
    /// lands.
    New {
        /// Natural-language brief to hand the Initializer.
        brief: String,
        /// Target project — defaults to the tenant's Inbox project.
        #[arg(long)]
        project: Option<String>,
        /// Don't block on the deliverable. The CLI returns as soon as
        /// the task is created; the deliverable lands in
        /// `~/.seasoned-hand/deliverables/`.
        #[arg(long)]
        detach: bool,
        /// Mark the intake so the Initializer should skip auto-confirm
        /// (the operator drives confirm via `brief confirm`). Phase 2
        /// the metadata flag is recorded but the spawner doesn't yet
        /// honor it — see phase-2/DEBT.md #29.
        #[arg(long = "no-auto-confirm")]
        no_auto_confirm: bool,
        /// On deliverable success, attempt `xdg-open` / `open` on the
        /// rendered content path. Silently no-op if neither binary is
        /// on PATH.
        #[arg(long)]
        open: bool,
    },
    /// Print the structured Brief authored for this task.
    Brief { id: String },
    /// Show or save the latest deliverable's rendered content path.
    Deliverable {
        id: String,
        /// Open the rendered file with xdg-open / open.
        #[arg(long)]
        open: bool,
        /// Copy the provenance manifest to this path.
        #[arg(long)]
        save: Option<std::path::PathBuf>,
    },
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
    /// Transfer task ownership to another user (requires TaskHandoff permission).
    Handoff {
        id: String,
        #[arg(long = "to")]
        to_user_email: String,
        #[arg(long)]
        reason: Option<String>,
        #[arg(long = "expected-updated-at")]
        expected_updated_at: Option<i64>,
    },
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
        TaskCmd::New {
            brief,
            project,
            detach,
            no_auto_confirm,
            open,
        } => {
            let mut metadata = serde_json::json!({});
            if no_auto_confirm && let Some(obj) = metadata.as_object_mut() {
                obj.insert("no_auto_confirm".into(), serde_json::Value::Bool(true));
            }
            if detach {
                let ack = into_anyhow(
                    client
                        .intake_cli_detach(&brief, project.as_deref(), metadata)
                        .await,
                )
                .context("submit detached brief")?;
                if json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "task_id": ack.task_id,
                            "intake_id": ack.intake_id,
                            "detached": true,
                        })
                    );
                } else {
                    println!("task {} queued (intake {})", ack.task_id, ack.intake_id);
                    println!("deliverable will land in ~/.seasoned-hand/deliverables/ when ready");
                }
                return Ok(());
            }
            let max_wait = std::time::Duration::from_secs(
                std::env::var("CLI_INTAKE_MAX_WAIT_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(600),
            );
            let ack = into_anyhow(
                client
                    .intake_cli_blocking(&brief, project.as_deref(), metadata, max_wait)
                    .await,
            )
            .context("submit brief (blocking)")?;
            let Some(deliverable) = ack.deliverable.as_ref() else {
                // wait=true path always returns Some on the happy path;
                // None means the server short-circuited (shouldn't happen
                // under loopback). Print the ack as-is.
                if json {
                    println!("{}", serde_json::to_string_pretty(&ack)?);
                } else {
                    println!("task {} accepted, no deliverable yet", ack.task_id);
                }
                return Ok(());
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&ack)?);
            } else {
                format::print_deliverable_summary(&ack.task_id, deliverable);
            }
            if open {
                open_path(&deliverable.rendered_content_path);
            }
        }
        TaskCmd::Brief { id } => {
            let task = into_anyhow(client.get_task(&id).await)?;
            match task.brief {
                Some(brief) => {
                    println!("{}", serde_json::to_string_pretty(&brief)?);
                }
                None => {
                    println!("(no brief authored yet for task {id})");
                }
            }
        }
        TaskCmd::Deliverable { id, open, save } => {
            // Provenance manifest carries the rendered content path +
            // sha256 + citations. Reuse the existing /v1/tasks/:id/provenance
            // surface — no new route required.
            let manifest = into_anyhow(client.task_provenance(&id).await)?;
            if let Some(path) = save.as_deref() {
                std::fs::write(path, serde_json::to_vec_pretty(&manifest)?)
                    .with_context(|| format!("write provenance to {}", path.display()))?;
                if !json {
                    println!("saved provenance to {}", path.display());
                }
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&manifest)?);
            } else {
                let rendered = manifest
                    .get("deliverable")
                    .and_then(|d| d.get("rendered_content_path"))
                    .and_then(|v| v.as_str());
                match rendered {
                    Some(path) => println!("deliverable at: {path}"),
                    None => println!("no deliverable rendered for task {id}"),
                }
            }
            if open
                && let Some(path) = manifest
                    .get("deliverable")
                    .and_then(|d| d.get("rendered_content_path"))
                    .and_then(|v| v.as_str())
            {
                open_path(path);
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
        TaskCmd::Handoff {
            id,
            to_user_email,
            reason,
            expected_updated_at,
        } => {
            let outcome = into_anyhow(
                client
                    .task_handoff(&id, &to_user_email, reason.as_deref(), expected_updated_at)
                    .await,
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&outcome)?);
            } else {
                println!(
                    "handoff {}: {} -> {} (audit={})",
                    outcome.task_id, outcome.from_user_id, outcome.to_user_id, outcome.audit_log_id
                );
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

fn open_path(path: &str) {
    // Try xdg-open (Linux), then `open` (macOS). Silently no-op when
    // neither resolves — the CLI is happy to run on a headless box.
    for bin in ["xdg-open", "open"] {
        if std::process::Command::new(bin)
            .arg(path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
        {
            return;
        }
    }
}
