//! `mshellctl log …` — control the shell's file logging.
//!
//! `level` / `enable` / `disable` retune the *running* shell live (no
//! restart); they round-trip through the `LogLevel` / `LogEnabled` D-Bus
//! methods and print the shell's reply. `path` / `open` are local conveniences
//! over the shared log dir.

use crate::bus::bus_command_with_arg_reply;
use clap::Subcommand;
use margo_logging::logs_dir;

#[derive(Subcommand, Debug)]
pub enum LogCommands {
    /// Set the file-log level live: error | warn | info | debug | trace.
    Level {
        /// One of error, warn, info, debug, trace.
        level: String,
    },
    /// Turn shell file logging on, live.
    Enable,
    /// Turn shell file logging off, live.
    Disable,
    /// Print the log directory and the current shell-session file.
    Path,
    /// Open the log directory in the default file manager.
    Open,
}

pub async fn execute(command: LogCommands) -> anyhow::Result<()> {
    match command {
        LogCommands::Level { level } => {
            let reply: String = bus_command_with_arg_reply("LogLevel", &level).await?;
            println!("{reply}");
        }
        LogCommands::Enable => {
            let reply: String = bus_command_with_arg_reply("LogEnabled", &true).await?;
            println!("{reply}");
        }
        LogCommands::Disable => {
            let reply: String = bus_command_with_arg_reply("LogEnabled", &false).await?;
            println!("{reply}");
        }
        LogCommands::Path => {
            let dir = logs_dir();
            println!("dir:     {}", dir.display());
            println!("current: {}", dir.join("mshell-latest.log").display());
        }
        LogCommands::Open => {
            let dir = logs_dir();
            let _ = std::fs::create_dir_all(&dir);
            std::process::Command::new("xdg-open")
                .arg(&dir)
                .spawn()
                .map(|_| ())
                .unwrap_or_else(|e| eprintln!("could not open {}: {e}", dir.display()));
        }
    }
    Ok(())
}
