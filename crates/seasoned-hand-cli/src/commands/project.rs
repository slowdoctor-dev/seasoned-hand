//! `seasoned-hand project <list|create|archive>`.

use anyhow::Result;
use clap::Subcommand;

use crate::client::{ApiClient, into_anyhow};
use crate::format;

#[derive(Debug, Subcommand)]
pub enum ProjectCmd {
    /// List projects (newest-first).
    List,
    /// Create a new project.
    Create {
        title: String,
        #[arg(long)]
        description: Option<String>,
    },
    /// Archive a project (status → archived).
    Archive { id: String },
}

pub async fn run(cmd: ProjectCmd, client: &ApiClient, json: bool) -> Result<()> {
    match cmd {
        ProjectCmd::List => {
            let projects = into_anyhow(client.list_projects().await)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&projects)?);
            } else {
                format::print_projects(&projects);
            }
        }
        ProjectCmd::Create { title, description } => {
            let project = into_anyhow(client.create_project(&title, description.as_deref()).await)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&project)?);
            } else {
                format::print_project(&project);
            }
        }
        ProjectCmd::Archive { id } => {
            into_anyhow(client.archive_project(&id).await)?;
            if json {
                println!("{}", serde_json::json!({"archived": id}));
            } else {
                println!("archived {id}");
            }
        }
    }
    Ok(())
}
