use crate::bus::bus_command;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum NotificationAction {
    /// Clear every notification — history and on-screen popups
    Clears,
    /// Dismiss the on-screen popups, keeping them in history
    Read,
}

#[derive(Subcommand, Debug)]
pub enum SessionAction {
    /// Lock the session
    Lock,
    /// Log out of the session
    Logout,
    /// Suspend the system
    Suspend,
    /// Reboot the system
    Reboot,
    /// Power off the system
    Shutdown,
}

#[derive(Subcommand, Debug)]
pub enum MenuCommands {
    /// Toggle the app launcher menu
    AppLauncher,
    /// Toggle the clipboard menu
    Clipboard,
    /// Toggle the clock menu
    Clock,
    /// Toggle the notifications menu, or act on notifications
    Notifications {
        #[command(subcommand)]
        action: Option<NotificationAction>,
    },
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
    /// Toggle the Power Profile (npower) menu
    Npower,
    /// Toggle the Media Player menu
    MediaPlayer,
    /// Toggle the session / power menu, or run a session action
    Session {
        #[command(subcommand)]
        action: Option<SessionAction>,
    },
    /// Toggle the combined dashboard menu (clock + weather +
    /// quick settings, all in one panel)
    Dashboard,
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
        MenuCommands::Notifications { action } => match action {
            None => {
                bus_command("Notifications").await?;
            }
            Some(NotificationAction::Clears) => {
                bus_command("NotificationsClearAll").await?;
            }
            Some(NotificationAction::Read) => {
                bus_command("NotificationsReadPopups").await?;
            }
        },
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
        MenuCommands::Npower => {
            bus_command("Npower").await?;
        }
        MenuCommands::MediaPlayer => {
            bus_command("MediaPlayer").await?;
        }
        MenuCommands::Session { action } => match action {
            None => {
                bus_command("Session").await?;
            }
            Some(SessionAction::Lock) => {
                bus_command("SessionLock").await?;
            }
            Some(SessionAction::Logout) => {
                bus_command("SessionLogout").await?;
            }
            Some(SessionAction::Suspend) => {
                bus_command("SessionSuspend").await?;
            }
            Some(SessionAction::Reboot) => {
                bus_command("SessionReboot").await?;
            }
            Some(SessionAction::Shutdown) => {
                bus_command("SessionShutdown").await?;
            }
        },
        MenuCommands::Dashboard => {
            bus_command("Dashboard").await?;
        }
        MenuCommands::CloseAll => {
            bus_command("CloseAllMenus").await?;
        }
    }
    Ok(())
}
