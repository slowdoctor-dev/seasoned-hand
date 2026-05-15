//! `seasoned-hand server [args...]` — exec the `seasoned-hand-server`
//! binary.
//!
//! Side-by-side install assumption: the server binary must be on PATH
//! (cargo install lays it alongside the CLI). On Unix we use
//! `CommandExt::exec` so the current process is replaced — no fork, no
//! extra wrapper-shell shim above the server's signal handling. On
//! Windows we fall back to `Command::status()` and propagate the exit
//! code.

use anyhow::Result;

pub fn run(args: Vec<String>) -> Result<std::process::ExitCode> {
    let bin = "seasoned-hand-server";
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let mut cmd = std::process::Command::new(bin);
        cmd.args(&args);
        let err = cmd.exec();
        anyhow::bail!("failed to exec `{bin}`: {err}");
    }
    #[cfg(not(unix))]
    {
        let status = std::process::Command::new(bin).args(&args).status()?;
        let code = status.code().unwrap_or(1);
        Ok(std::process::ExitCode::from(
            u8::try_from(code).unwrap_or(1),
        ))
    }
}
