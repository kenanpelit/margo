use crate::bus::shell;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum MediaCommands {
    /// Play/pause — optionally a target: a player name fragment (spotify,
    /// browser, vlc…) or `mpd`/`mpc` for native MPD. Omit for the
    /// best-scoring active player across every backend.
    Toggle { player: Option<String> },
    /// Next track on the target player (or the best-scoring active one)
    Next { player: Option<String> },
    /// Previous track on the target player (or the best-scoring active one)
    Prev { player: Option<String> },
    /// The player a target-less command would act on: name, state, track
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Every player — MPRIS + native MPD — with state + which one is active
    List {
        #[arg(long)]
        json: bool,
    },
}

pub async fn execute(command: MediaCommands) -> anyhow::Result<()> {
    match command {
        MediaCommands::Toggle { player } => {
            shell()
                .await?
                .media_toggle(&player.unwrap_or_default())
                .await?;
        }
        MediaCommands::Next { player } => {
            shell()
                .await?
                .media_next(&player.unwrap_or_default())
                .await?;
        }
        MediaCommands::Prev { player } => {
            shell()
                .await?
                .media_prev(&player.unwrap_or_default())
                .await?;
        }
        MediaCommands::Status { json } => {
            let shell = shell().await?;
            let out = if json {
                shell.media_status_json().await?
            } else {
                shell.media_status_text().await?
            };
            println!("{out}");
        }
        MediaCommands::List { json } => {
            let shell = shell().await?;
            let out = if json {
                shell.media_list_json().await?
            } else {
                shell.media_list_text().await?
            };
            println!("{out}");
        }
    }
    Ok(())
}
