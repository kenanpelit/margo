use crate::bus::{bus_command, bus_command_with_arg, bus_command_with_reply};
use clap::Subcommand;

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum DndState {
    On,
    Off,
}

#[derive(Subcommand, Debug)]
pub enum NotificationAction {
    /// Clear every notification — history and on-screen popups
    Clears,
    /// Dismiss the on-screen popups, keeping them in history
    Read,
    /// Toggle Do Not Disturb, or set it explicitly with `on` / `off`
    Dnd {
        #[arg(value_enum)]
        state: Option<DndState>,
    },
    /// Print the current notification count (history)
    Count,
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
    /// Toggle the app launcher menu, optionally pre-selecting a
    /// category tab. With no flags this is a pure toggle. Use
    /// `--list-tabs` to discover valid tab names, `--tab <name>`
    /// to jump straight to a tab.
    AppLauncher {
        /// Pre-select the named category tab (e.g. "Run",
        /// "Insert"). Unknown names fall back to "All".
        #[arg(long)]
        tab: Option<String>,
        /// List the known category tab names and exit without
        /// opening the launcher.
        #[arg(long, conflicts_with = "tab")]
        list_tabs: bool,
    },
    /// Toggle the clipboard menu
    Clipboard,
    /// Toggle the clock menu
    Clock,
    /// Toggle the notifications menu, or act on notifications
    Notifications {
        #[command(subcommand)]
        action: Option<NotificationAction>,
    },
    /// Toggle the screenshot menu
    Screenshot,
    /// Toggle the wallpaper menu
    Wallpaper,
    /// Toggle the UFW firewall menu
    Ufw,
    /// Toggle the Bluetooth menu (connect/disconnect/scan/pair)
    Bluetooth,
    /// Toggle the CPU Dashboard menu (CPU, temp, RAM, load)
    CpuDashboard,
    /// Toggle the Audio Dashboard menu (output + input mixer)
    AudioDashboard,
    /// Toggle the System Updates menu (repo / AUR / Flatpak)
    SystemUpdate,
    /// Toggle the Valent Connect menu (phone status + actions)
    Valent,
    /// Toggle the Keep Awake menu (duration grid + countdown)
    KeepAwake,
    /// Toggle the Twilight menu (toggle + temperature + mode + presets)
    Twilight,
    /// Toggle the keybind cheatsheet menu (searchable shortcut list)
    Keybinds,
    /// Toggle the DNS / VPN menu
    Dns,
    /// Toggle the Podman menu
    Podman,
    /// Toggle the Notes Hub menu
    Notes,
    /// Toggle the Public IP menu
    Ip,
    /// Toggle the Network Console menu
    Network,
    /// Toggle the Power Profile (power) menu
    Power,
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
        MenuCommands::AppLauncher { tab, list_tabs } => {
            if list_tabs {
                let tabs: Vec<String> =
                    bus_command_with_reply("ListAppLauncherTabs").await?;
                for t in tabs {
                    println!("{t}");
                }
            } else if let Some(name) = tab {
                bus_command_with_arg("AppLauncherTab", &name).await?;
            } else {
                bus_command("AppLauncher").await?;
            }
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
            Some(NotificationAction::Dnd { state }) => match state {
                Some(DndState::On) => bus_command("NotificationDndOn").await?,
                Some(DndState::Off) => bus_command("NotificationDndOff").await?,
                None => bus_command("NotificationDndToggle").await?,
            },
            Some(NotificationAction::Count) => {
                let count: u32 = bus_command_with_reply("NotificationCount").await?;
                println!("{count}");
            }
        },
        MenuCommands::Screenshot => {
            bus_command("Screenshot").await?;
        }
        MenuCommands::Wallpaper => {
            bus_command("Wallpaper").await?;
        }
        MenuCommands::Ufw => {
            bus_command("Ufw").await?;
        }
        MenuCommands::Bluetooth => {
            bus_command("Bluetooth").await?;
        }
        MenuCommands::CpuDashboard => {
            bus_command("CpuDashboard").await?;
        }
        MenuCommands::AudioDashboard => {
            bus_command("AudioDashboard").await?;
        }
        MenuCommands::SystemUpdate => {
            bus_command("SystemUpdate").await?;
        }
        MenuCommands::Valent => {
            bus_command("Valent").await?;
        }
        MenuCommands::KeepAwake => {
            bus_command("KeepAwake").await?;
        }
        MenuCommands::Twilight => {
            bus_command("Twilight").await?;
        }
        MenuCommands::Keybinds => {
            bus_command("Keybinds").await?;
        }
        MenuCommands::Dns => {
            bus_command("Dns").await?;
        }
        MenuCommands::Podman => {
            bus_command("Podman").await?;
        }
        MenuCommands::Notes => {
            bus_command("Notes").await?;
        }
        MenuCommands::Ip => {
            bus_command("Ip").await?;
        }
        MenuCommands::Network => {
            bus_command("Network").await?;
        }
        MenuCommands::Power => {
            bus_command("Power").await?;
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
