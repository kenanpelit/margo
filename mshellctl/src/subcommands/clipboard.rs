//! `mshellctl clipboard` — headless access to the shell's clipboard
//! history (the same store behind `mshellctl menu clipboard`). Drives
//! `mshell_clipboard::clipboard_service()` over IPC. One engine, one
//! tool — scriptable + keybind-friendly.
//!
//! ```text
//! mshellctl clipboard list [--json]   # id  category  preview (newest first)
//! mshellctl clipboard copy <id>       # re-copy that entry to the clipboard
//! mshellctl clipboard pin   <id>      # toggle favourite
//! mshellctl clipboard unpin <id>      # (alias of pin — toggles)
//! mshellctl clipboard delete <id>     # remove one entry
//! mshellctl clipboard clear           # drop all non-pinned entries
//! mshellctl clipboard wipe            # drop everything incl. favourites
//! ```

use crate::bus::{bus_command_with_arg, bus_command_with_reply};
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum ClipboardCommands {
    /// List history entries: `id<TAB>category<TAB>preview`, newest first.
    List {
        /// Emit JSON instead of tab-separated lines.
        #[arg(long)]
        json: bool,
    },
    /// Re-copy the entry with this id back onto the clipboard.
    Copy { id: u64 },
    /// Toggle the favourite (pin) flag on this entry.
    Pin { id: u64 },
    /// Toggle the favourite (pin) flag on this entry (alias of `pin`).
    Unpin { id: u64 },
    /// Remove a single entry from history.
    Delete { id: u64 },
    /// Drop all non-pinned entries.
    Clear,
    /// Drop everything, including favourites.
    Wipe,
}

pub async fn execute(command: ClipboardCommands) -> anyhow::Result<()> {
    match command {
        ClipboardCommands::List { json } => {
            let raw: String = bus_command_with_reply("ClipboardList").await?;
            if json {
                // Build a small JSON array without pulling serde_json here:
                // the shell already sanitised tabs/newlines out of previews.
                let items: Vec<String> = raw
                    .lines()
                    .filter_map(|l| {
                        let mut f = l.splitn(3, '\t');
                        let (Some(id), Some(cat), Some(prev)) = (f.next(), f.next(), f.next())
                        else {
                            return None;
                        };
                        Some(format!(
                            "{{\"id\":{id},\"category\":\"{cat}\",\"preview\":\"{}\"}}",
                            prev.replace('\\', "\\\\").replace('"', "\\\"")
                        ))
                    })
                    .collect();
                println!("[{}]", items.join(","));
            } else if !raw.is_empty() {
                println!("{raw}");
            }
        }
        ClipboardCommands::Copy { id } => {
            bus_command_with_arg("ClipboardAction", &format!("copy {id}")).await?;
        }
        ClipboardCommands::Pin { id } | ClipboardCommands::Unpin { id } => {
            bus_command_with_arg("ClipboardAction", &format!("pin {id}")).await?;
        }
        ClipboardCommands::Delete { id } => {
            bus_command_with_arg("ClipboardAction", &format!("delete {id}")).await?;
        }
        ClipboardCommands::Clear => {
            bus_command_with_arg("ClipboardAction", &"clear".to_string()).await?;
        }
        ClipboardCommands::Wipe => {
            bus_command_with_arg("ClipboardAction", &"wipe".to_string()).await?;
        }
    }
    Ok(())
}
