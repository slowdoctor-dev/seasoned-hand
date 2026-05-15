//! `seasoned-hand channel <list|test|logs>`.
//!
//! `logs` is intentionally stubbed in 2.21b — the server has no
//! per-channel structured log feed yet; tracking as phase-2/DEBT.md #30.

use anyhow::Result;
use clap::Subcommand;

use crate::client::{ApiClient, into_anyhow};

#[derive(Debug, Subcommand)]
pub enum ChannelCmd {
    /// List registered channels.
    List,
    /// Send the canned test event through a channel.
    Test {
        name: String,
        /// Channel role to exercise (intake / delivery / notify).
        #[arg(long)]
        role: Option<String>,
    },
    /// Tail structured logs for a channel. Not yet implemented — see
    /// phase-2/DEBT.md #30 for the WS subscription work.
    Logs {
        name: String,
        #[arg(long)]
        tail: bool,
    },
}

pub async fn run(cmd: ChannelCmd, client: &ApiClient, json: bool) -> Result<()> {
    match cmd {
        ChannelCmd::List => {
            let snapshot = into_anyhow(client.list_channels().await)?;
            println!("{}", serde_json::to_string_pretty(&snapshot)?);
        }
        ChannelCmd::Test { name, role } => {
            let _ = json;
            let outcome = into_anyhow(client.channel_test(&name, role.as_deref()).await)?;
            // Channel test bodies are already structured JSON; pretty-print
            // regardless of --json so the operator can pipe to jq either way.
            println!("{}", serde_json::to_string_pretty(&outcome)?);
        }
        ChannelCmd::Logs { name, tail } => {
            let _ = (name, tail);
            anyhow::bail!(
                "channel logs is not yet implemented — see phase-2/DEBT.md #30 for the WS-subscription follow-up"
            );
        }
    }
    Ok(())
}
