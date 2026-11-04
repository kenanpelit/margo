//! Menu config: every menu surface, their default widget layouts.

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Menus {
    pub clock_menu: Menu,
    pub clipboard_menu: Menu,
    pub notification_menu: Menu,
    pub screenshot_menu: Menu,
    pub app_launcher_menu: Menu,
    pub wallpaper_menu: Menu,
    pub screenshare_menu: ScreenshareMenu,
    pub ufw_menu: Menu,
    pub dns_menu: Menu,
    pub vpn_menu: Menu,
    pub ai_menu: Menu,
    pub podman_menu: Menu,
    /// `github` bar pill's panel. Default-on-missing so profiles
    /// predating it still parse.
    #[serde(default = "default_github_menu")]
    pub github_menu: Menu,
    /// `agents` bar pill's panel. Default-on-missing so profiles
    /// predating it still parse.
    #[serde(default = "default_agents_menu")]
    pub agents_menu: Menu,
    pub notes_menu: Menu,
    pub ip_menu: Menu,
    pub network_menu: Menu,
    pub power_menu: Menu,
    // Default-on-missing so older user YAML predating these menu
    // types still parses cleanly.
    #[serde(default = "default_bluetooth_menu")]
    pub bluetooth_menu: Menu,
    #[serde(default = "default_cpu_dashboard_menu")]
    pub cpu_dashboard_menu: Menu,
    #[serde(default = "default_audio_dashboard_menu")]
    pub audio_dashboard_menu: Menu,
    #[serde(default = "default_system_update_menu")]
    pub system_update_menu: Menu,
    #[serde(default = "default_valent_menu")]
    pub valent_menu: Menu,
    #[serde(default = "default_keep_awake_menu")]
    pub keep_awake_menu: Menu,
    #[serde(default = "default_weather_menu")]
    pub weather_menu: Menu,
    #[serde(default = "default_twilight_menu")]
    pub twilight_menu: Menu,
    #[serde(default = "default_keybinds_menu")]
    pub keybinds_menu: Menu,
    #[serde(default = "default_alarmclock_menu")]
    pub alarmclock_menu: Menu,
    #[serde(default = "default_dock_menu")]
    pub dock_menu: Menu,
    #[serde(default = "default_control_center_menu")]
    pub control_center_menu: Menu,
    #[serde(default = "default_ssh_menu")]
    pub ssh_menu: Menu,
    #[serde(default = "default_usb_menu")]
    pub usb_menu: Menu,
    #[serde(default = "default_discord_voice_menu")]
    pub discord_voice_menu: Menu,
    #[serde(default = "default_crashes_menu")]
    pub crashes_menu: Menu,
    #[serde(default = "default_privacy_menu")]
    pub privacy_menu: Menu,
    /// Generic-VPN detail menu opened by the `vpn_indicator` bar pill.
    /// Default-on-missing so older YAML predating this menu still parses.
    #[serde(default = "default_vpn_indicator_menu")]
    pub vpn_indicator_menu: Menu,
    /// `translate` bar pill's panel. Default-on-missing so profiles
    /// predating it still parse.
    #[serde(default = "default_translate_menu")]
    pub translate_menu: Menu,
    pub media_player_menu: Menu,
    /// Dedicated `mtune` (folder-first music player) menu. Default-on-missing
    /// so profiles predating it still parse.
    #[serde(default = "default_mtune_menu")]
    pub mtune_menu: Menu,
    pub lyrics_menu: Menu,
    pub session_menu: Menu,
    /// Settings panel — embeds in the frame's menu stack instead
    /// of launching a separate `gtk::Window` toplevel.
    pub settings_menu: Menu,
    /// `mdash` — the dashboard menu. A time-aware greeting header,
    /// the two-pane calendar/weather + intel/connectivity/audio/system/
    /// media body, and a two-row action grid (toggles + a menu-shortcut
    /// grid + power actions). Opened by `mshellctl menu mdash` or the
    /// Mdash bar pill.
    /// Default-on-missing so older YAML parses. `skip_serializing_if`
    /// keeps it OUT of the written profile while it equals the code
    /// default, so future default changes win instead of a stale baked
    /// copy overriding them (it's only persisted once edited in Settings).
    #[serde(
        default = "default_mdash_menu",
        skip_serializing_if = "mdash_menu_is_default"
    )]
    pub mdash_menu: Menu,
    /// Margo layout switcher — replaces the legacy bar-popover
    /// variant. Anchored to whichever side the user pins it to;
    /// content is a single `MargoLayout` widget rendering a
    /// vertical list of compositor layouts.
    pub margo_layout_menu: Menu,
    /// First-class surface for plugin-provided WASM panels (mplugins WASM
    /// tier). One menu hosts whichever plugin panel is opened (by key), so an
    /// installed plugin's panel is as position/size-configurable as any
    /// built-in menu. Default-on-missing so older YAML parses.
    #[serde(default = "default_plugin_panel_menu")]
    pub plugin_panel_menu: Menu,
    pub left_menu_expansion_type: VerticalMenuExpansion,
    pub right_menu_expansion_type: VerticalMenuExpansion,
}

pub(super) fn default_plugin_panel_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        // Content is the injected WASM panel, not config widgets.
        widgets: vec![],
        minimum_width: 420,
        maximum_height: 560,
    }
}

/// `mdash` — a compact, polished dashboard. A balanced two-pane body sized
/// to fit on screen, a time-aware greeting header, and a two-row action
/// grid where the power actions sit apart from the toggles so a stray
/// click can't reach Shutdown.
///
/// What it intentionally does NOT do: cram in the full Notes hub / inline
/// notification center / system-update panel. Those are tall, full-feature
/// surfaces — stacking them here turned the panel into a giant scroll that
/// pushed the quick actions off-screen and stretched the media tile. They
/// live in their own menus (`mshellctl menu notifications`, etc.); the
/// OverviewIntel tile still surfaces the counts at a glance.
///
/// `fill: false` on the columns keeps tiles at their natural height (top
/// aligned) instead of stretching the last tile.
/// True when `mdash_menu` is still the untouched code default, so the
/// profile serializer can omit it (see the field's `skip_serializing_if`).
pub(super) fn mdash_menu_is_default(m: &Menu) -> bool {
    *m == default_mdash_menu()
}

pub(super) fn default_mdash_menu() -> Menu {
    Menu {
        position: Position::Top,
        widgets: vec![
            // Greeting header (replaces the static "Dashboard" title).
            MenuWidget::PanelHeader(PanelHeaderConfig {
                title: "Dashboard".to_string(),
                greeting: true,
            }),
            MenuWidget::Spacer(SpacerConfig { size: 8 }),
            // ── 2-col body ──
            MenuWidget::Container(ContainerConfig {
                widgets: vec![
                    // Left = "today at a glance".
                    MenuWidget::Container(ContainerConfig {
                        widgets: vec![MenuWidget::CalendarGrid, MenuWidget::Weather],
                        spacing: 10,
                        orientation: Orientation::Vertical,
                        minimum_width: 400,
                        homogeneous: false,
                        fill: false,
                    }),
                    // Right = intel + controls + media.
                    MenuWidget::Container(ContainerConfig {
                        widgets: vec![
                            MenuWidget::OverviewIntel,
                            MenuWidget::Connectivity,
                            MenuWidget::CompactAudio,
                            MenuWidget::SystemStatus,
                            MenuWidget::MediaPlayer,
                        ],
                        spacing: 8,
                        orientation: Orientation::Vertical,
                        minimum_width: 400,
                        homogeneous: false,
                        fill: false,
                    }),
                ],
                spacing: 12,
                orientation: Orientation::Horizontal,
                minimum_width: 0,
                homogeneous: true,
                fill: false,
            }),
            // ── Bottom action grid: everything in two dense rows (~12
            //    per row) — toggles, menu shortcuts, and the power
            //    actions. Each shortcut opens its menu and closes mdash.
            //    A "Quick Actions" heading titles the grid so the
            //    bottom half reads with the same rhythm as the cards.
            MenuWidget::Spacer(SpacerConfig { size: 10 }),
            MenuWidget::SectionLabel(SectionLabelConfig {
                text: "Quick Actions".to_string(),
            }),
            MenuWidget::Spacer(SpacerConfig { size: 4 }),
            MenuWidget::QuickActions(QuickActionsConfig {
                widgets: vec![
                    QuickActionWidget::AirplaneMode,
                    QuickActionWidget::Nightlight,
                    QuickActionWidget::ColorPicker,
                    QuickActionWidget::Wallpaper,
                    QuickActionWidget::Screenshot,
                    QuickActionWidget::Settings,
                    QuickActionWidget::Network,
                    QuickActionWidget::Bluetooth,
                    QuickActionWidget::CpuDashboard,
                    QuickActionWidget::AudioDashboard,
                    QuickActionWidget::Vpn,
                    QuickActionWidget::ControlCenter,
                ],
            }),
            MenuWidget::Spacer(SpacerConfig { size: 6 }),
            MenuWidget::QuickActions(QuickActionsConfig {
                widgets: vec![
                    QuickActionWidget::Twilight,
                    QuickActionWidget::Keybinds,
                    QuickActionWidget::Dns,
                    QuickActionWidget::Power,
                    QuickActionWidget::Session,
                    QuickActionWidget::Ip,
                    QuickActionWidget::AlarmClock,
                    QuickActionWidget::SystemUpdate,
                    QuickActionWidget::Logout,
                    QuickActionWidget::Lock,
                    QuickActionWidget::Reboot,
                    QuickActionWidget::Shutdown,
                ],
            }),
        ],
        minimum_width: 860,
        maximum_height: 0,
    }
}

pub(super) fn default_bluetooth_menu() -> Menu {
    Menu {
        position: Position::Top,
        widgets: vec![MenuWidget::Bluetooth],
        minimum_width: 400,
        maximum_height: 0,
    }
}

pub(super) fn default_agents_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        widgets: vec![MenuWidget::Agents],
        minimum_width: 460,
        maximum_height: 0,
    }
}

pub(super) fn default_github_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        widgets: vec![MenuWidget::Github],
        minimum_width: 460,
        maximum_height: 0,
    }
}

pub(super) fn default_translate_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        widgets: vec![MenuWidget::Translate],
        minimum_width: 420,
        maximum_height: 0,
    }
}

pub(super) fn default_vpn_indicator_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        widgets: vec![MenuWidget::VpnIndicator],
        minimum_width: 380,
        maximum_height: 0,
    }
}

pub(super) fn default_mtune_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        widgets: vec![MenuWidget::Mtune],
        minimum_width: 500,
        maximum_height: 760,
    }
}

pub(super) fn default_system_update_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        widgets: vec![MenuWidget::SystemUpdate],
        minimum_width: 460,
        maximum_height: 620,
    }
}

pub(super) fn default_valent_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        widgets: vec![MenuWidget::Valent],
        minimum_width: 460,
        maximum_height: 620,
    }
}

pub(super) fn default_keep_awake_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        widgets: vec![MenuWidget::KeepAwake],
        minimum_width: 320,
        maximum_height: 0,
    }
}

pub(super) fn default_weather_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        widgets: vec![MenuWidget::Weather],
        minimum_width: 380,
        maximum_height: 0,
    }
}

pub(super) fn default_twilight_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        widgets: vec![MenuWidget::Twilight],
        minimum_width: 360,
        maximum_height: 0,
    }
}

pub(super) fn default_keybinds_menu() -> Menu {
    Menu {
        position: Position::Top,
        widgets: vec![MenuWidget::Keybinds],
        // Wide enough for the two-column "combo | description" rows
        // without wrapping common labels; capped height so the long
        // shortcut list scrolls instead of overflowing the screen.
        minimum_width: 720,
        maximum_height: 720,
    }
}

pub(super) fn default_alarmclock_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        widgets: vec![MenuWidget::AlarmClock],
        // Roomy enough for the alarm rows (time + repeat-day chips +
        // delete) and the stopwatch hero; capped height so a long
        // alarm list scrolls instead of overflowing the screen.
        minimum_width: 420,
        maximum_height: 640,
    }
}

pub(super) fn default_dock_menu() -> Menu {
    Menu {
        position: Position::Bottom,
        widgets: vec![MenuWidget::MargoDock],
        minimum_width: 0,
        maximum_height: 0,
    }
}

pub(super) fn default_control_center_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        widgets: vec![MenuWidget::ControlCenter],
        minimum_width: 460,
        maximum_height: 720,
    }
}

pub(super) fn default_usb_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        widgets: vec![MenuWidget::Usb],
        minimum_width: 380,
        maximum_height: 0,
    }
}

pub(super) fn default_crashes_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        widgets: vec![MenuWidget::Crashes],
        minimum_width: 460,
        maximum_height: 640,
    }
}

pub(super) fn default_ssh_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        widgets: vec![MenuWidget::SshSessions],
        // Roomy enough for "host + user@hostname:port" rows; capped
        // height so a large ~/.ssh/config scrolls.
        minimum_width: 460,
        maximum_height: 720,
    }
}

/// Discord voice panel: header, call controls and a participant list
/// with per-user volume sliders.
pub(super) fn default_discord_voice_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        widgets: vec![MenuWidget::DiscordVoice],
        minimum_width: 420,
        maximum_height: 640,
    }
}

pub(super) fn default_privacy_menu() -> Menu {
    Menu {
        position: Position::TopRight,
        widgets: vec![MenuWidget::Privacy],
        // Roomy enough for the "in use now" rows + the access-log list
        // (icon + app + time + started/stopped); capped so a long log
        // scrolls instead of overflowing the screen.
        minimum_width: 380,
        maximum_height: 560,
    }
}

pub(super) fn default_cpu_dashboard_menu() -> Menu {
    Menu {
        position: Position::Top,
        widgets: vec![MenuWidget::CpuDashboard],
        minimum_width: 380,
        maximum_height: 0,
    }
}

pub(super) fn default_audio_dashboard_menu() -> Menu {
    Menu {
        position: Position::Top,
        widgets: vec![MenuWidget::AudioDashboard],
        minimum_width: 400,
        maximum_height: 0,
    }
}

impl Default for Menus {
    fn default() -> Self {
        Self {
            clock_menu: Menu {
                position: Position::Top,
                widgets: vec![
                    MenuWidget::Calendar,
                    MenuWidget::Spacer(SpacerConfig { size: 20 }),
                    MenuWidget::Weather,
                ],
                minimum_width: 410,
                maximum_height: 0,
            },
            clipboard_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Clipboard],
                // Golden-ratio proportions (φ ≈ 1.618): the tabbed
                // ListBox reads best as a portrait golden rectangle
                // (890/550 ≈ φ) that is also the golden section of a
                // 1440-tall screen (890/1440 ≈ 1/φ). Tune in Settings
                // → Widgets → Clipboard for other monitor heights.
                minimum_width: 550,
                maximum_height: 890,
            },
            notification_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Notifications],
                minimum_width: 410,
                maximum_height: 0,
            },
            screenshot_menu: Menu {
                position: Position::TopRight,
                widgets: vec![
                    MenuWidget::Screenshots,
                    MenuWidget::Divider,
                    MenuWidget::ScreenRecording,
                ],
                minimum_width: 410,
                maximum_height: 0,
            },
            app_launcher_menu: Menu {
                position: Position::TopLeft,
                widgets: vec![MenuWidget::AppLauncher],
                minimum_width: 800,
                maximum_height: 1080,
            },
            wallpaper_menu: Menu {
                position: Position::Top,
                widgets: vec![MenuWidget::ThemePicker, MenuWidget::Wallpaper],
                minimum_width: 1200,
                maximum_height: 0,
            },
            screenshare_menu: ScreenshareMenu {
                position: Position::TopRight,
            },
            ufw_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Ufw],
                minimum_width: 410,
                maximum_height: 0,
            },
            dns_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Dns],
                minimum_width: 420,
                maximum_height: 0,
            },
            vpn_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Vpn],
                minimum_width: 430,
                maximum_height: 0,
            },
            ai_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Ai],
                minimum_width: 460,
                maximum_height: 560,
            },
            agents_menu: default_agents_menu(),
            github_menu: default_github_menu(),
            podman_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Podman],
                minimum_width: 540,
                maximum_height: 0,
            },
            notes_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Notes],
                minimum_width: 480,
                maximum_height: 0,
            },
            ip_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Ip],
                minimum_width: 380,
                maximum_height: 0,
            },
            network_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Network],
                minimum_width: 460,
                maximum_height: 0,
            },
            power_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Power],
                minimum_width: 360,
                maximum_height: 0,
            },
            bluetooth_menu: default_bluetooth_menu(),
            cpu_dashboard_menu: default_cpu_dashboard_menu(),
            audio_dashboard_menu: default_audio_dashboard_menu(),
            system_update_menu: default_system_update_menu(),
            valent_menu: default_valent_menu(),
            keep_awake_menu: default_keep_awake_menu(),
            weather_menu: default_weather_menu(),
            twilight_menu: default_twilight_menu(),
            keybinds_menu: default_keybinds_menu(),
            alarmclock_menu: default_alarmclock_menu(),
            dock_menu: default_dock_menu(),
            control_center_menu: default_control_center_menu(),
            ssh_menu: default_ssh_menu(),
            usb_menu: default_usb_menu(),
            discord_voice_menu: default_discord_voice_menu(),
            crashes_menu: default_crashes_menu(),
            privacy_menu: default_privacy_menu(),
            vpn_indicator_menu: default_vpn_indicator_menu(),
            translate_menu: default_translate_menu(),
            plugin_panel_menu: default_plugin_panel_menu(),
            media_player_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::MediaPlayer],
                minimum_width: 380,
                maximum_height: 0,
            },
            mtune_menu: default_mtune_menu(),
            lyrics_menu: Menu {
                position: Position::TopRight,
                widgets: vec![MenuWidget::Lyrics],
                minimum_width: 420,
                maximum_height: 560,
            },
            session_menu: Menu {
                position: Position::Top,
                widgets: vec![MenuWidget::Session],
                minimum_width: 420,
                maximum_height: 0,
            },
            settings_menu: Menu {
                // Settings is a wide / tall panel. Top anchor with
                // a generous width — same family as the wallpaper
                // and notifications menus. `widgets` is empty
                // because the settings panel renders itself; the
                // generic MenuWidget pipeline isn't used.
                position: Position::Top,
                widgets: vec![],
                minimum_width: 780,
                maximum_height: 0,
            },
            mdash_menu: default_mdash_menu(),
            margo_layout_menu: Menu {
                // Replaces the legacy `gtk::PopoverMenu` that
                // opened as a separate `xdg_popup` window. Lives
                // in the frame's menu stack now so it slides out
                // contiguous with the bar like every other menu.
                // The single `MargoLayout` widget renders the
                // full list of compositor layouts with the
                // currently-active row highlighted.
                position: Position::Top,
                widgets: vec![MenuWidget::MargoLayout],
                minimum_width: 280,
                maximum_height: 0,
            },
            left_menu_expansion_type: VerticalMenuExpansion::AlwaysExpanded,
            right_menu_expansion_type: VerticalMenuExpansion::AlwaysExpanded,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Menu {
    pub position: Position,
    pub widgets: Vec<MenuWidget>,
    pub minimum_width: i32,
    /// Maximum content height in pixels. 0 = no cap (legacy
    /// "grow to fit children" behaviour). When > 0, the menu's
    /// outer ScrolledWindow caps the visible height at this value
    /// and the inner content scrolls vertically — useful for
    /// menus with long, scrollable result lists (app launcher,
    /// clipboard history…) so the panel doesn't grow taller than
    /// the user's monitor.
    pub maximum_height: i32,
}

impl Default for Menu {
    fn default() -> Self {
        Self {
            position: Position::Left,
            widgets: Vec::new(),
            minimum_width: 410,
            maximum_height: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct ScreenshareMenu {
    pub position: Position,
}

impl Default for ScreenshareMenu {
    fn default() -> Self {
        Self {
            position: Position::Left,
        }
    }
}
