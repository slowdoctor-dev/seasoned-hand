//! `seasoned-hand inbox` — list pending briefings across all projects.

use anyhow::Result;
use clap::Args;

use crate::client::{ApiClient, into_anyhow};
use crate::format;

#[derive(Debug, Args)]
pub struct InboxCmd {
    /// Filter to a single project.
    #[arg(long)]
    pub project: Option<String>,
}

pub async fn run(cmd: InboxCmd, client: &ApiClient, json: bool) -> Result<()> {
    let entries = into_anyhow(client.list_inbox(cmd.project.as_deref()).await)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
    } else {
        format::print_inbox(&entries);
    }
    Ok(())
}
