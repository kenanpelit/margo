use crate::bus::shell;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum BrightnessCommands {
    /// Increase the brightness by 5 percent
    Up,
    /// Decrease the brightness by 5 percent
    Down,
}

pub async fn execute(command: BrightnessCommands) -> anyhow::Result<()> {
    match command {
        BrightnessCommands::Up => {
            shell().await?.brightness_up().await?;
        }
        BrightnessCommands::Down => {
            shell().await?.brightness_down().await?;
        }
    }
    Ok(())
}
