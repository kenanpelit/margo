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
//! `set` matching is forgiving: `kenp`, `Kenp`, `tokyo-night`,
//! `tokyo_night`, `TokyoNight` and `"Tokyo Night"` all resolve.

use crate::bus::shell;
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
    /// ignores case and `-`/`_`/space separators (e.g. `kenp`,
    /// `tokyo-night`). Use `theme list` to see the valid names.
    Set { name: String },
}

pub async fn execute(command: ThemeCommands) -> anyhow::Result<()> {
    match command {
        ThemeCommands::List { names_only } => {
            let raw: String = shell().await?.theme_list().await?;
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
            let current: String = shell().await?.theme_get().await?;
            if !current.is_empty() {
                println!("{current}");
            }
        }
        ThemeCommands::Set { name } => {
            // An unknown name comes back as a D-Bus error → non-zero exit.
            shell().await?.theme_set(&name).await?;
        }
    }
    Ok(())
}
