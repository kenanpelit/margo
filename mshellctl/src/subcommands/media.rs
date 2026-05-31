use crate::bus::{bus_command_with_arg, bus_command_with_reply};
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum MediaCommands {
    /// Play/pause — optionally a player name fragment (spotify, browser, vlc…)
    Toggle { player: Option<String> },
    /// Next track on the player (or the active one)
    Next { player: Option<String> },
    /// Previous track on the player (or the active one)
    Prev { player: Option<String> },
    /// The active player: name, state, current track
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Every MPRIS player, with state + which one is active
    List {
        #[arg(long)]
        json: bool,
    },
}

pub async fn execute(command: MediaCommands) -> anyhow::Result<()> {
    match command {
        MediaCommands::Toggle { player } => {
            bus_command_with_arg("MediaToggle", &player.unwrap_or_default()).await?;
        }
        MediaCommands::Next { player } => {
            bus_command_with_arg("MediaNext", &player.unwrap_or_default()).await?;
        }
        MediaCommands::Prev { player } => {
            bus_command_with_arg("MediaPrev", &player.unwrap_or_default()).await?;
        }
        MediaCommands::Status { json } => {
            let method = if json {
                "MediaStatusJson"
            } else {
                "MediaStatusText"
            };
            let out: String = bus_command_with_reply(method).await?;
            println!("{out}");
        }
        MediaCommands::List { json } => {
            let method = if json {
                "MediaListJson"
            } else {
                "MediaListText"
            };
            let out: String = bus_command_with_reply(method).await?;
            println!("{out}");
        }
    }
    Ok(())
}
