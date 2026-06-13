//! `mshellctl theme` — inspect and switch the shell's colour scheme from
//! the terminal. This is the same scheme picker as Settings → Theme →
//! Color Scheme; setting one here writes the reactive config store and
//! re-themes live — no `mctl reload`, no restart.
//!
//! ```text
//! mshellctl theme list [--names-only]   # name<TAB>label, one per scheme
//! mshellctl theme get                   # the scheme currently in use
//! mshellctl theme set <name>            # switch live (case/-_ insensitive)
//! ```
//!
//! `set` matching is forgiving: `eventide`, `Eventide`, `tokyo-night`,
//! `tokyo_night`, `TokyoNight` and `"Tokyo Night"` all resolve.

use crate::bus::{bus_command_with_arg_reply, bus_command_with_reply};
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum ThemeCommands {
    /// List every built-in colour scheme as `name<TAB>label`, one per line.
    List {
        /// Print only the canonical names (one per line), dropping labels —
        /// handy for piping into a picker like `fzf`.
        #[arg(long)]
        names_only: bool,
    },
    /// Print the canonical name of the colour scheme currently in use.
    Get,
    /// Switch to a colour scheme by name and apply it live. Name matching
    /// ignores case and `-`/`_`/space separators (e.g. `eventide`,
    /// `tokyo-night`). Use `theme list` to see the valid names.
    Set { name: String },
}

pub async fn execute(command: ThemeCommands) -> anyhow::Result<()> {
    match command {
        ThemeCommands::List { names_only } => {
            let raw: String = bus_command_with_reply("ThemeList").await?;
            if names_only {
                for line in raw.lines() {
                    let name = line.split_once('\t').map(|(n, _)| n).unwrap_or(line);
                    println!("{name}");
                }
            } else if !raw.is_empty() {
                println!("{raw}");
            }
        }
        ThemeCommands::Get => {
            let current: String = bus_command_with_reply("ThemeGet").await?;
            if !current.is_empty() {
                println!("{current}");
            }
        }
        ThemeCommands::Set { name } => {
            // The shell replies with an empty string on success, or an error
            // message for an unknown name — surface it as a non-zero exit.
            let err: String = bus_command_with_arg_reply("ThemeSet", &name).await?;
            if !err.is_empty() {
                anyhow::bail!("{err}");
            }
        }
    }
    Ok(())
}
