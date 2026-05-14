use crate::bus::bus_command;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum MenuCommands {
    /// Toggle the app launcher menu
    AppLauncher,
    /// Toggle the clipboard menu
    Clipboard,
    /// Toggle the clock menu
    Clock,
    /// Toggle the notifications menu
    Notifications,
    /// Toggle the quick settings menu
    QuickSettings,
    /// Toggle the screenshot menu
    Screenshot,
    /// Toggle the wallpaper menu
    Wallpaper,
    /// Toggle the UFW firewall menu
    Nufw,
    /// Toggle the DNS / VPN menu
    Ndns,
    /// Toggle the Podman menu
    Npodman,
    /// Toggle the Notes Hub menu
    Nnotes,
    /// Toggle the Public IP menu
    Nip,
    /// Toggle the Network Console menu
    Nnetwork,
    /// Close all open menus
    CloseAll,
}

pub async fn execute(command: MenuCommands) -> anyhow::Result<()> {
    match command {
        MenuCommands::QuickSettings => {
            bus_command("QuickSettings").await?;
        }
        MenuCommands::AppLauncher => {
            bus_command("AppLauncher").await?;
        }
        MenuCommands::Clipboard => {
            bus_command("Clipboard").await?;
        }
        MenuCommands::Clock => {
            bus_command("Clock").await?;
        }
        MenuCommands::Notifications => {
            bus_command("Notifications").await?;
        }
        MenuCommands::Screenshot => {
            bus_command("Screenshot").await?;
        }
        MenuCommands::Wallpaper => {
            bus_command("Wallpaper").await?;
        }
        MenuCommands::Nufw => {
            bus_command("Nufw").await?;
        }
        MenuCommands::Ndns => {
            bus_command("Ndns").await?;
        }
        MenuCommands::Npodman => {
            bus_command("Npodman").await?;
        }
        MenuCommands::Nnotes => {
            bus_command("Nnotes").await?;
        }
        MenuCommands::Nip => {
            bus_command("Nip").await?;
        }
        MenuCommands::Nnetwork => {
            bus_command("Nnetwork").await?;
        }
        MenuCommands::CloseAll => {
            bus_command("CloseAllMenus").await?;
        }
    }
    Ok(())
}
