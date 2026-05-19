//! `seasoned-hand init` — bootstrap `~/.seasoned-hand/{deliverables,config}`.
//!
//! Idempotent: re-running just confirms the paths exist. No file is
//! ever overwritten — operators editing `config/notify.toml` won't have
//! their settings clobbered.

use std::path::PathBuf;

use anyhow::{Context, Result};

pub fn run(json: bool) -> Result<()> {
    let home = resolve_home()?;
    let root = home.join(".seasoned-hand");
    let deliverables = root.join("deliverables");
    let config = root.join("config");
    for dir in [&root, &deliverables, &config] {
        std::fs::create_dir_all(dir).with_context(|| format!("create dir {}", dir.display()))?;
    }
    if json {
        println!(
            "{}",
            serde_json::json!({
                "root": root.display().to_string(),
                "deliverables": deliverables.display().to_string(),
                "config": config.display().to_string(),
            })
        );
    } else {
        println!("ready:");
        println!("  root         {}", root.display());
        println!("  deliverables {}", deliverables.display());
        println!("  config       {}", config.display());
    }
    Ok(())
}

fn resolve_home() -> Result<PathBuf> {
    // Honor $HOME first so tests can sandbox to a tmpdir without
    // touching the developer's actual home. Windows runtimes that
    // don't set HOME fall to USERPROFILE.
    if let Ok(home) = std::env::var("HOME")
        && !home.is_empty()
    {
        return Ok(PathBuf::from(home));
    }
    if let Ok(profile) = std::env::var("USERPROFILE")
        && !profile.is_empty()
    {
        return Ok(PathBuf::from(profile));
    }
    anyhow::bail!("could not resolve home directory (HOME / USERPROFILE both unset)")
}
