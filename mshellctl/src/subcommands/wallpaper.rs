use crate::bus::bus_command_with_arg;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum WallpaperCommands {
    /// Switch to the next wallpaper in the directory
    Next,
    /// Switch to the previous wallpaper in the directory
    Prev,
    /// Switch to a random wallpaper from the directory
    Random,
}

pub async fn execute(command: WallpaperCommands) -> anyhow::Result<()> {
    let direction = match command {
        WallpaperCommands::Next => "next",
        WallpaperCommands::Prev => "previous",
        WallpaperCommands::Random => "random",
    };
    bus_command_with_arg("WallpaperCycle", &direction).await?;
    Ok(())
}
