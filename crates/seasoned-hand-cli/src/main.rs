//! `seasoned-hand` — CLI binary, story 2.21a (project + task surface).
//!
//! Phase 2 ships the OS-layer non-negotiable: every UI action has a
//! CLI equivalent so the web frontend becomes one of several frontends
//! rather than THE frontend. Story 2.21a is the first slice — project
//! list/create/archive + task list/show/pause/resume/cancel/provenance.
//!
//! 2.21b will add `task new`, brief/inbox subcommands, init, and the
//! `server` shell-through.
//!
//! refs: /specs/phase-2/stories/story-2.21.md
//! refs: /specs/phase-2/stories/story-2.21b.md

use std::io::IsTerminal;
use std::process::ExitCode;

use clap::Parser;

mod client;
mod commands;
mod format;

#[derive(Debug, Parser)]
#[command(
    name = "seasoned-hand",
    version,
    about = "Seasoned Hand operator CLI",
    long_about = None,
)]
struct Cli {
    /// Base URL of the seasoned-hand HTTP server.
    #[arg(
        long,
        global = true,
        env = "SH_SERVER_URL",
        default_value = "http://127.0.0.1:3000"
    )]
    server: String,

    /// Disable ANSI colors regardless of TTY detection.
    #[arg(long, global = true)]
    no_color: bool,

    /// Emit raw JSON for list/show subcommands. Implies --no-color.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, clap::Subcommand)]
enum Commands {
    /// Project management — `seasoned-hand project <list|create|archive>`.
    #[command(subcommand)]
    Project(commands::project::ProjectCmd),
    /// Task lifecycle — `seasoned-hand task <list|show|pause|resume|cancel|provenance>`.
    #[command(subcommand)]
    Task(commands::task::TaskCmd),
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    // Auto-disable colors when not a TTY OR when JSON output is on OR
    // when the user passed --no-color. `colored` already auto-detects
    // TTY but we make the override explicit so piped output is clean.
    let use_color = !cli.no_color && !cli.json && std::io::stdout().is_terminal();
    colored::control::set_override(use_color);

    let client = client::ApiClient::new(&cli.server);

    let result = match cli.command {
        Commands::Project(cmd) => commands::project::run(cmd, &client, cli.json).await,
        Commands::Task(cmd) => commands::task::run(cmd, &client, cli.json).await,
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // `{:#}` walks the anyhow chain so callers see the source
            // (e.g. `server returned 404: task_not_found`) not just
            // the outermost context.
            eprintln!("error: {err:#}");
            ExitCode::from(1)
        }
    }
}
