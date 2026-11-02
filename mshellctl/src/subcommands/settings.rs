use crate::bus::shell;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum SettingsCommands {
    /// Open the settings window
    Open,
    /// Close the settings window
    Close,
}

pub async fn execute(command: SettingsCommands) -> anyhow::Result<()> {
    match command {
        SettingsCommands::Open => {
            shell().await?.open_settings().await?;
        }
        SettingsCommands::Close => {
            shell().await?.close_settings().await?;
        }
    }
    Ok(())
}
