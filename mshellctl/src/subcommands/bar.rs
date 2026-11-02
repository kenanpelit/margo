use crate::bus::shell;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum BarCommands {
    /// Toggle the top bar
    Top,
    /// Toggle the bottom bar
    Bottom,
    /// Toggle every bar (top + bottom)
    #[command(visible_alias = "toggle")]
    ToggleAll {
        /// Exclude bars that are hidden by default
        #[arg(short = 'x', long = "exclude")]
        exclude_hidden_by_default: bool,
    },
    /// Reveal every bar (top + bottom)
    #[command(visible_aliases = ["show", "show-all", "reveal"])]
    RevealAll {
        /// Exclude bars that are hidden by default
        #[arg(short = 'x', long = "exclude")]
        exclude_hidden_by_default: bool,
    },
    /// Hide every bar (top + bottom)
    #[command(visible_alias = "hide")]
    HideAll {
        /// Exclude bars that are hidden by default
        #[arg(short = 'x', long = "exclude")]
        exclude_hidden_by_default: bool,
    },
}

pub async fn execute(command: BarCommands) -> anyhow::Result<()> {
    match command {
        BarCommands::Top => {
            shell().await?.bar_toggle_top().await?;
        }
        BarCommands::Bottom => {
            shell().await?.bar_toggle_bottom().await?;
        }
        BarCommands::ToggleAll {
            exclude_hidden_by_default,
        } => {
            shell()
                .await?
                .bar_toggle_all(exclude_hidden_by_default)
                .await?;
        }
        BarCommands::RevealAll {
            exclude_hidden_by_default,
        } => {
            shell()
                .await?
                .bar_reveal_all(exclude_hidden_by_default)
                .await?;
        }
        BarCommands::HideAll {
            exclude_hidden_by_default,
        } => {
            shell()
                .await?
                .bar_hide_all(exclude_hidden_by_default)
                .await?;
        }
    }
    Ok(())
}
