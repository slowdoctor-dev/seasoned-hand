//! `seasoned-hand brief <confirm|edit|cancel>` — drive the briefing
//! confirm gate for a task.
//!
//! Phase 2 aliases `briefing_id := task_id` (see
//! `/v1/inbox` doc-comment in the server). `seasoned-hand inbox` lists
//! the available briefing ids.

use anyhow::{Context, Result};
use clap::Subcommand;
use seasoned_hand_core::agent::init::briefing::PartialBrief;

use crate::client::{ApiClient, BriefingConfirmRequest, into_anyhow};

#[derive(Debug, Subcommand)]
pub enum BriefCmd {
    /// Approve the current Brief as-authored.
    Confirm { id: String },
    /// Edit the current Brief.
    Edit {
        id: String,
        /// Open the Brief in $EDITOR (default `vi`) and apply diffs as
        /// a PartialBrief overlay. Required for now — non-`--editor`
        /// edit will arrive in a follow-up once a brief CLI form lands.
        #[arg(long)]
        editor: bool,
    },
    /// Cancel the briefing (task → cancelled).
    Cancel { id: String },
}

pub async fn run(cmd: BriefCmd, client: &ApiClient, json: bool) -> Result<()> {
    match cmd {
        BriefCmd::Confirm { id } => {
            into_anyhow(
                client
                    .briefing_confirm(&id, BriefingConfirmRequest::Confirm)
                    .await,
            )
            .with_context(|| format!("confirm briefing {id}"))?;
            ack(json, &id, "confirm");
        }
        BriefCmd::Cancel { id } => {
            into_anyhow(
                client
                    .briefing_confirm(&id, BriefingConfirmRequest::Cancel)
                    .await,
            )
            .with_context(|| format!("cancel briefing {id}"))?;
            ack(json, &id, "cancel");
        }
        BriefCmd::Edit { id, editor } => {
            if !editor {
                anyhow::bail!(
                    "brief edit currently requires --editor (opens $EDITOR on the current Brief JSON)"
                );
            }
            let task = into_anyhow(client.get_task(&id).await)?;
            let current = task.brief.unwrap_or_else(|| serde_json::json!({}));
            let edited = open_in_editor(&current)?;
            if edited == current {
                println!("no changes detected — aborting");
                return Ok(());
            }
            let edits: PartialBrief =
                serde_json::from_value(edited).context("re-parse edited brief as PartialBrief")?;
            into_anyhow(
                client
                    .briefing_confirm(&id, BriefingConfirmRequest::Edit { edits })
                    .await,
            )
            .with_context(|| format!("submit edits for briefing {id}"))?;
            ack(json, &id, "edit");
        }
    }
    Ok(())
}

fn ack(json: bool, id: &str, action: &str) {
    if json {
        println!(
            "{}",
            serde_json::json!({
                "briefing_id": id,
                "action": action,
                "ok": true,
            })
        );
    } else {
        println!("{action} → briefing {id} accepted");
    }
}

fn open_in_editor(current: &serde_json::Value) -> Result<serde_json::Value> {
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".into());
    let dir = std::env::temp_dir();
    let path = dir.join(format!("seasoned-hand-brief-{}.json", uuid::Uuid::new_v4()));
    std::fs::write(&path, serde_json::to_vec_pretty(current)?)
        .with_context(|| format!("write tmpfile {}", path.display()))?;
    let status = std::process::Command::new(&editor)
        .arg(&path)
        .status()
        .with_context(|| format!("spawn editor {editor}"))?;
    if !status.success() {
        // Best-effort cleanup; leaving the file behind is harmless.
        let _ = std::fs::remove_file(&path);
        anyhow::bail!("editor exited with status {status:?}");
    }
    let bytes = std::fs::read(&path).with_context(|| format!("read tmpfile {}", path.display()))?;
    let _ = std::fs::remove_file(&path);
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).context("re-parse edited brief json")?;
    Ok(value)
}
