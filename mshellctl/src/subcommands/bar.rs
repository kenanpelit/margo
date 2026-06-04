use crate::bus::{bus_command, bus_command_with_arg};
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
            bus_command("BarToggleTop").await?;
        }
        BarCommands::Bottom => {
            bus_command("BarToggleBottom").await?;
        }
        BarCommands::ToggleAll {
            exclude_hidden_by_default,
        } => {
            bus_command_with_arg("BarToggleAll", &exclude_hidden_by_default).await?;
        }
        BarCommands::RevealAll {
            exclude_hidden_by_default,
        } => {
            bus_command_with_arg("BarRevealAll", &exclude_hidden_by_default).await?;
        }
        BarCommands::HideAll {
            exclude_hidden_by_default,
        } => {
            bus_command_with_arg("BarHideAll", &exclude_hidden_by_default).await?;
        }
    }
    Ok(())
}
