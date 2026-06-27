use clap::Parser;
use mshellctl::app::{Cli, Commands, GameModeAction};
use mshellctl::bus::{bus_command, bus_command_with_arg, bus_command_with_reply};

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
        Commands::HiddenBar { action, name } => {
            bus_command_with_arg("HiddenBar", &(action, name.unwrap_or_default())).await?;
        }
        Commands::Audio { command } => mshellctl::subcommands::audio::execute(command).await?,
        Commands::Bluetooth { command } => {
            mshellctl::subcommands::bluetooth::execute(command).await?
        }
        Commands::Media { command } => mshellctl::subcommands::media::execute(command).await?,
        Commands::Brightness { command } => {
            mshellctl::subcommands::brightness::execute(command).await?
        }
        Commands::Log { command } => mshellctl::subcommands::log::execute(command).await?,
        Commands::Dock { command } => mshellctl::subcommands::dock::execute(command).await?,
        Commands::SetWallpaper { path } => {
            bus_command_with_arg("SetWallpaper", &path.to_string_lossy().as_ref()).await?;
        }
        Commands::Toast {
            title,
            body,
            icon,
            severity,
        } => {
            bus_command_with_arg(
                "Toast",
                &(
                    title,
                    body.unwrap_or_default(),
                    icon.unwrap_or_default(),
                    severity,
                ),
            )
            .await?;
        }
        Commands::Gamemode { action } => match action {
            GameModeAction::Status => {
                let s: String = bus_command_with_reply("GameModeStatus").await?;
                println!("{s}");
            }
            GameModeAction::On => bus_command_with_arg("GameMode", &"on".to_string()).await?,
            GameModeAction::Off => bus_command_with_arg("GameMode", &"off".to_string()).await?,
            GameModeAction::Toggle => {
                bus_command_with_arg("GameMode", &"toggle".to_string()).await?
            }
        },
        Commands::Lock { command } => mshellctl::subcommands::lock::execute(command).await?,
        Commands::Settings { command } => {
            mshellctl::subcommands::settings::execute(command).await?
        }
        Commands::Theme { command } => mshellctl::subcommands::theme::execute(command).await?,
        Commands::Wizard => {
            bus_command("OpenWizard").await?;
        }
        Commands::Wallpaper { command } => {
            mshellctl::subcommands::wallpaper::execute(command).await?
        }
        Commands::Plugin { command } => mshellctl::subcommands::plugin::execute(command).await?,
        Commands::Screenshot { command } => {
            mshellctl::subcommands::screenshot::execute(command).await?
        }
        Commands::ScreenRecord { command } => {
            mshellctl::subcommands::screen_record::execute(command).await?
        }
        Commands::Clipboard { command } => {
            mshellctl::subcommands::clipboard::execute(command).await?
        }
        Commands::Doctor => mshellctl::subcommands::doctor::execute().await?,
        Commands::Completions { shell } => {
            use clap::CommandFactory;
            let mut cmd = Cli::command();
            let name = cmd.get_name().to_string();
            clap_complete::generate(shell, &mut cmd, name, &mut std::io::stdout());
        }
    };

    Ok(())
}
