use crate::bus::shell;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum LockCommands {
    /// Lock the active session
    Activate,
    /// Check if the session is lock. Prints "locked" or "unlocked"
    Check,
}

pub async fn execute(command: Option<LockCommands>) -> anyhow::Result<()> {
    match command {
        // Bare `mshellctl lock` (no subcommand) locks — the common case.
        None | Some(LockCommands::Activate) => {
            shell().await?.lock().await?;
        }
        Some(LockCommands::Check) => {
            let locked: bool = shell().await?.check_lock().await?;
            println!("{}", if locked { "locked" } else { "unlocked" });
        }
    }
    Ok(())
}
