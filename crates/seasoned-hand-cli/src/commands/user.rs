use anyhow::Result;
use clap::Subcommand;

use crate::client::{ApiClient, into_anyhow};

#[derive(Debug, Subcommand)]
pub enum UserCmd {
    /// Invite a user into an organization.
    Invite {
        email: String,
        #[arg(long = "org")]
        org_slug: String,
        #[arg(long)]
        role: String,
    },
    /// List users in an organization.
    List {
        #[arg(long = "org")]
        org_slug: String,
    },
}

pub async fn run(cmd: UserCmd, client: &ApiClient, json: bool) -> Result<()> {
    match cmd {
        UserCmd::Invite {
            email,
            org_slug,
            role,
        } => {
            let out = into_anyhow(client.invite_user(&org_slug, &email, &role).await)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                println!("user_id: {}", out.user_id);
                println!("display_name: {}", out.display_name);
                println!("login_token: {}", out.login_token);
            }
        }
        UserCmd::List { org_slug } => {
            let rows = into_anyhow(client.list_org_users(&org_slug).await)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rows)?);
            } else if rows.is_empty() {
                println!("(no users)");
            } else {
                println!(
                    "{:<28}  {:<28}  {:<10}  {:<8}",
                    "USER_ID", "EMAIL", "ROLE", "STATUS"
                );
                for row in rows {
                    println!(
                        "{:<28}  {:<28}  {:<10}  {:<8}",
                        row.user_id, row.email, row.role, row.status
                    );
                }
            }
        }
    }
    Ok(())
}
