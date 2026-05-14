use clap::Parser;
use mshellctl::app::{Cli, Commands};
use mshellctl::bus::{bus_command, bus_command_with_arg};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Quit => {
            bus_command("Quit").await?;
        }
        Commands::Inspect => {
            bus_command("Inspect").await?;
        }
        Commands::Menu { command } => mshellctl::subcommands::menu::execute(command).await?,
        Commands::Bar { command } => mshellctl::subcommands::bar::execute(command).await?,
        Commands::Audio { command } => mshellctl::subcommands::audio::execute(command).await?,
        Commands::Brightness { command } => {
            mshellctl::subcommands::brightness::execute(command).await?
        }
        Commands::SetWallpaper { path } => {
            bus_command_with_arg("SetWallpaper", &path.to_string_lossy().as_ref()).await?;
        }
        Commands::Lock { command } => mshellctl::subcommands::lock::execute(command).await?,
        Commands::Settings { command } => {
            mshellctl::subcommands::settings::execute(command).await?
        }
        Commands::Wallpaper { command } => {
            mshellctl::subcommands::wallpaper::execute(command).await?
        }
    };

    Ok(())
}
