use crate::bus::shell;
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
        /// Pre-select the named category tab (e.g. "Apps",
        /// "Actions"). Unknown names fall back to "All".
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
    /// Toggle the Privacy menu (mic / camera / screen-share access log)
    Privacy,
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
    /// Toggle the Margo Layout Switcher menu (pick the current tag's tiling
    /// layout — same picker as the bar's Margo Layout Switcher pill)
    #[command(name = "margo-layout")]
    MargoLayout,
    /// Toggle the Weather menu (current + hourly + daily, all in one)
    Weather,
    /// Toggle the keybind cheatsheet menu (searchable shortcut list)
    Keybinds,
    /// Toggle the Alarm Clock menu (alarms list + add/edit + stopwatch)
    AlarmClock,
    /// Toggle the Control Center menu (system preferences + quick controls)
    #[command(name = "control-center")]
    ControlCenter,
    /// Toggle the SSH Sessions menu (searchable ~/.ssh/config host list)
    SshSessions,
    /// Toggle the USB Drives menu (removable drives: mount, unlock, safe removal)
    Usb,
    /// Toggle the Discord voice menu (call controls + participants)
    DiscordVoice,
    /// Toggle the Crashes menu (your core dumps + this boot's journal errors)
    Crashes,
    /// Toggle the DNS / VPN menu (Mullvad controls + a collapsible DNS section)
    Vpn,
    /// Toggle the DNS / VPN menu (alias for `vpn`)
    Dns,
    /// Toggle the AI assistant chat menu
    Ai,
    /// Toggle the Podman menu
    Podman,
    /// Toggle the GitHub menu
    Github,
    /// Toggle the Host Monitor menu (pinged hosts: up / down, latency)
    HostMonitor,
    /// Toggle the AI Agents menu
    Agents,
    /// Toggle the QR code menu (share text / Wi-Fi with a phone)
    Qr,
    /// Toggle the Notes Hub menu
    Notes,
    /// Toggle the Public IP menu
    Ip,
    /// Toggle the generic-VPN detail menu (active OpenVPN / WireGuard tunnels)
    VpnIndicator,
    /// Toggle the Network Console menu
    Network,
    /// Toggle the Power Profile (power) menu
    Power,
    /// Toggle the Media Player menu
    MediaPlayer,
    /// Toggle the Tune (mtune folder-first music player) menu
    Mtune,
    /// Toggle the Lyrics menu (synced lyrics of the now-playing track)
    Lyrics,
    /// Toggle the session / power menu, or run a session action
    Session {
        #[command(subcommand)]
        action: Option<SessionAction>,
    },
    /// Toggle the mdash dashboard menu — greeting header, calendar +
    /// weather, the quick-settings card stack, and a menu-shortcut grid.
    Mdash,
    /// Toggle an installed plugin's panel/menu by key — e.g.
    /// `menu plugin assistant` or `menu plugin mullvad`. Generic: works for
    /// any plugin that ships a panel or a `[[widget.menu]]`, with no
    /// per-plugin code (the key matches the plugin id or a widget key).
    Plugin {
        /// The plugin key (its id, or a widget key).
        key: String,
    },
    /// Toggle the Translate menu (manual language lookup; the
    /// select-or-copy + keybind flow is headless — see `mshellctl
    /// translate`)
    Translate,
    /// Toggle the Color Picker menu (recent colours + HEX / RGB / HSL
    /// editor)
    ColorPicker,
    /// Close all open menus
    CloseAll,
}

pub async fn execute(command: MenuCommands) -> anyhow::Result<()> {
    match command {
        MenuCommands::AppLauncher { tab, list_tabs } => {
            if list_tabs {
                let tabs: Vec<String> = shell().await?.list_app_launcher_tabs().await?;
                for t in tabs {
                    println!("{t}");
                }
            } else if let Some(name) = tab {
                shell().await?.app_launcher_tab(&name).await?;
            } else {
                shell().await?.app_launcher().await?;
            }
        }
        MenuCommands::Clipboard => {
            shell().await?.clipboard().await?;
        }
        MenuCommands::Clock => {
            shell().await?.clock().await?;
        }
        MenuCommands::Notifications { action } => match action {
            None => {
                shell().await?.notifications().await?;
            }
            Some(NotificationAction::Clears) => {
                shell().await?.notifications_clear_all().await?;
            }
            Some(NotificationAction::Read) => {
                shell().await?.notifications_read_popups().await?;
            }
            Some(NotificationAction::Dnd { state }) => match state {
                Some(DndState::On) => shell().await?.notification_dnd_on().await?,
                Some(DndState::Off) => shell().await?.notification_dnd_off().await?,
                None => shell().await?.notification_dnd_toggle().await?,
            },
            Some(NotificationAction::Count) => {
                let count: u32 = shell().await?.notification_count().await?;
                println!("{count}");
            }
        },
        MenuCommands::Screenshot => {
            shell().await?.screenshot().await?;
        }
        MenuCommands::Wallpaper => {
            shell().await?.wallpaper().await?;
        }
        MenuCommands::Ufw => {
            shell().await?.ufw().await?;
        }
        MenuCommands::Privacy => {
            shell().await?.privacy().await?;
        }
        MenuCommands::Bluetooth => {
            shell().await?.bluetooth().await?;
        }
        MenuCommands::CpuDashboard => {
            shell().await?.cpu_dashboard().await?;
        }
        MenuCommands::AudioDashboard => {
            shell().await?.audio_dashboard().await?;
        }
        MenuCommands::SystemUpdate => {
            shell().await?.system_update().await?;
        }
        MenuCommands::Valent => {
            shell().await?.valent().await?;
        }
        MenuCommands::KeepAwake => {
            shell().await?.keep_awake().await?;
        }
        MenuCommands::Twilight => {
            shell().await?.twilight().await?;
        }
        MenuCommands::MargoLayout => {
            shell().await?.margo_layout_menu().await?;
        }
        MenuCommands::Weather => {
            shell().await?.weather().await?;
        }
        MenuCommands::Keybinds => {
            shell().await?.keybinds().await?;
        }
        MenuCommands::AlarmClock => {
            shell().await?.alarm_clock().await?;
        }
        MenuCommands::ControlCenter => {
            shell().await?.control_center().await?;
        }
        MenuCommands::Crashes => {
            shell().await?.crashes().await?;
        }
        MenuCommands::SshSessions => {
            shell().await?.ssh_sessions().await?;
        }
        MenuCommands::Usb => {
            shell().await?.usb().await?;
        }
        MenuCommands::DiscordVoice => {
            shell().await?.discord_voice().await?;
        }
        MenuCommands::Vpn => {
            shell().await?.vpn().await?;
        }
        MenuCommands::Dns => {
            shell().await?.dns().await?;
        }
        MenuCommands::Translate => {
            shell().await?.translate_menu().await?;
        }
        MenuCommands::ColorPicker => {
            shell().await?.color_picker().await?;
        }
        MenuCommands::Ai => {
            shell().await?.ai().await?;
        }
        MenuCommands::Podman => {
            shell().await?.podman().await?;
        }
        MenuCommands::Github => {
            shell().await?.github().await?;
        }
        MenuCommands::HostMonitor => {
            shell().await?.host_monitor().await?;
        }
        MenuCommands::Agents => {
            shell().await?.agents().await?;
        }
        MenuCommands::Qr => {
            shell().await?.qr_menu().await?;
        }
        MenuCommands::Notes => {
            shell().await?.notes().await?;
        }
        MenuCommands::Ip => {
            shell().await?.ip().await?;
        }
        MenuCommands::VpnIndicator => {
            shell().await?.vpn_indicator().await?;
        }
        MenuCommands::Network => {
            shell().await?.network().await?;
        }
        MenuCommands::Power => {
            shell().await?.power().await?;
        }
        MenuCommands::MediaPlayer => {
            shell().await?.media_player().await?;
        }
        MenuCommands::Mtune => {
            shell().await?.mtune().await?;
        }
        MenuCommands::Lyrics => {
            shell().await?.lyrics().await?;
        }
        MenuCommands::Session { action } => match action {
            None => {
                shell().await?.session().await?;
            }
            Some(SessionAction::Lock) => {
                shell().await?.session_lock().await?;
            }
            Some(SessionAction::Logout) => {
                shell().await?.session_logout().await?;
            }
            Some(SessionAction::Suspend) => {
                shell().await?.session_suspend().await?;
            }
            Some(SessionAction::Reboot) => {
                shell().await?.session_reboot().await?;
            }
            Some(SessionAction::Shutdown) => {
                shell().await?.session_shutdown().await?;
            }
        },
        MenuCommands::Mdash => {
            shell().await?.mdash().await?;
        }
        MenuCommands::Plugin { key } => {
            shell().await?.plugin_menu(&key).await?;
        }
        MenuCommands::CloseAll => {
            shell().await?.close_all_menus().await?;
        }
    }
    Ok(())
}
