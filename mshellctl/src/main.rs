use clap::Parser;
use mshellctl::app::{Cli, Commands, GameModeAction};
use mshellctl::bus::shell;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Quit => {
            shell().await?.quit().await?;
        }
        Commands::Inspect => {
            shell().await?.inspect().await?;
        }
        Commands::Menu { command } => mshellctl::subcommands::menu::execute(command).await?,
        Commands::Bar { command } => mshellctl::subcommands::bar::execute(command).await?,
        Commands::HiddenBar { action, name } => {
            shell()
                .await?
                .hidden_bar(&action, &name.unwrap_or_default())
                .await?;
        }
        Commands::CustomRefresh { name } => {
            let n = shell()
                .await?
                .custom_refresh(&name.unwrap_or_default())
                .await?;
            if n == 0 {
                eprintln!("no custom pill is listening");
            }
        }
        Commands::Audio { command } => mshellctl::subcommands::audio::execute(command).await?,
        Commands::Bluetooth { command } => {
            mshellctl::subcommands::bluetooth::execute(command).await?
        }
        Commands::Mtune { command } => mshellctl::subcommands::mtune::execute(command).await?,
        Commands::Media { command } => mshellctl::subcommands::media::execute(command).await?,
        Commands::Brightness { command } => {
            mshellctl::subcommands::brightness::execute(command).await?
        }
        Commands::Log { command } => mshellctl::subcommands::log::execute(command).await?,
        Commands::Dock { command } => mshellctl::subcommands::dock::execute(command).await?,
        Commands::SetWallpaper { path } => {
            // The shell resolves the path against its own cwd, not ours.
            let path = std::path::absolute(&path)?;
            shell()
                .await?
                .set_wallpaper(path.to_string_lossy().as_ref())
                .await?;
        }
        Commands::Toast {
            title,
            body,
            icon,
            severity,
        } => {
            shell()
                .await?
                .toast(
                    &title,
                    &body.unwrap_or_default(),
                    &icon.unwrap_or_default(),
                    &severity,
                )
                .await?;
        }
        Commands::Gamemode { action } => match action {
            GameModeAction::Status => {
                let s: String = shell().await?.game_mode_status().await?;
                println!("{s}");
            }
            GameModeAction::On => shell().await?.game_mode("on").await?,
            GameModeAction::Off => shell().await?.game_mode("off").await?,
            GameModeAction::Toggle => shell().await?.game_mode("toggle").await?,
        },
        Commands::Pomodoro { action } => {
            let status: String = shell().await?.pomodoro(action.verb()).await?;
            println!("{status}");
        }
        Commands::Timer { command } => mshellctl::subcommands::timer::execute(command).await?,
        Commands::Eyecare { action } => {
            let status: String = shell().await?.eyecare(action.verb()).await?;
            println!("{status}");
        }
        Commands::Lock { command } => mshellctl::subcommands::lock::execute(command).await?,
        Commands::Session { command } => mshellctl::subcommands::session::execute(command).await?,
        Commands::Notification { command } => {
            mshellctl::subcommands::notification::execute(command).await?
        }
        Commands::Calendar { command } => {
            mshellctl::subcommands::calendar::execute(command).await?
        }
        Commands::Vpn { command } => mshellctl::subcommands::vpn::execute(command).await?,
        Commands::Power { command } => mshellctl::subcommands::power::execute(command).await?,
        Commands::Layout { command } => mshellctl::subcommands::layout::execute(command).await?,
        Commands::Osk { command } => mshellctl::subcommands::osk::execute(command).await?,
        Commands::Color(args) => mshellctl::subcommands::color::execute(args).await?,
        Commands::Play { command } => mshellctl::subcommands::play::execute(command).await?,
        Commands::Settings { command } => {
            mshellctl::subcommands::settings::execute(command).await?
        }
        Commands::Theme { command } => mshellctl::subcommands::theme::execute(command).await?,
        Commands::Wizard => {
            shell().await?.open_wizard().await?;
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
        Commands::Translate { command } => {
            mshellctl::subcommands::translate::execute(command).await?
        }
        Commands::Qr(args) => mshellctl::subcommands::qr::execute(args).await?,
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
