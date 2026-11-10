//! Control Center tile grid — 2-column grid of toggle/info tiles.
//!
//! The grid order is driven by `ControlCenterConfig::tile_order`. Wide tiles
//! (dnd, valent) span both columns. Tiles not present in `tile_order` append
//! at the end in canonical order so nothing silently disappears after an
//! upgrade.
//!
//! Wi-Fi, Bluetooth, Audio Out, Mic, Battery, VPN, Valent, Twilight,
//! Keep Awake, Firewall (UFW), and Podman tiles are *expandable*:
//! left-clicking them emits `ControlCenterTilesOutput::ExpandPage` so the
//! parent `ControlCenterMenuWidgetModel` can switch the Stack to the
//! matching detail sub-page. The Twilight tile shows the active profile
//! (mode + temperature) as its subtitle; Podman shows the running
//! machine name(s) (or a container summary); Firewall shows Active/Inactive.
//!
//! Bluetooth, Twilight, Keep Awake, Firewall, and Podman additionally
//! support a **secondary (right) click** that quick-toggles their state
//! without leaving the grid (left-click opens the detail page; right-click
//! flips on/off — UFW enable/disable via pkexec, Podman stops the running
//! machine(s)).
//!
//! Dark Mode is a full labeled toggle tile. Mirror toggles margo's output
//! mirroring (laptop panel onto the external display) and shows the
//! running `source → target` pair.
//! Do Not Disturb is `.wide` (spans 2 columns).
//! Battery tile is hidden when no battery is present.
//!
//! All stateful tiles subscribe to their respective service watchers and
//! start those watchers lazily on the first `Reveal(true)`.
//!
//! Layout: `tiles/registry.rs` lists every tile (icon, title, clicks) and
//! is the place to start when adding one; `tiles/grid.rs` builds the
//! widgets from it and lays out the grid; `tiles/visuals.rs` renders the
//! model onto them; `tiles/pollers.rs` starts the watchers and
//! `tiles/tile_probes.rs` reads the live system state.

mod custom;
pub(crate) mod dash_layout;
#[cfg(test)]
pub(crate) mod fixture;
mod grid;
mod led;
pub(crate) mod led_auto;
mod pollers;
mod registry;
pub(crate) mod tile_probes;
mod visuals;

pub(crate) use grid::{ControlCenterTilesInit, ControlCenterTilesWidgets, TilesLayout};
pub(crate) use registry::TileId;

use crate::bars::bar_widgets::podman::{PODMAN_POLL, PodmanSummary};
use crate::menus::reveal_poll::RevealGate;
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    ConfigStoreFields, ControlCenterConfigStoreFields, MatugenStoreFields, ThemeStoreFields,
};
use mshell_config::schema::themes::MatugenMode;
use mshell_idle::inhibitor::IdleInhibitor;
use mshell_services::{
    bluetooth_service, network_service, notification_service, power_profile_service,
};
use mshell_utils::denoise::{self, DenoiseStatus};
use mshell_utils::hotspot::{self, HotspotStatus};
use mshell_utils::picker::spawn_color_picker;
use mshell_utils::shared_poll::PollSubscription;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk;
use relm4::gtk::prelude::{GridExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender};
use std::collections::HashSet;
use std::time::Duration;
use tile_probes::{
    BatterySnapshot, DiskUsage, read_airplane_state, read_audio_out_subtitle, read_battery,
    read_bt_state, read_disk_usage, read_mic_subtitle, read_podman_tile_state, read_ufw_tile_state,
    read_vpn_state, read_wifi_state, stop_podman_machines, toggle_mirror_and_probe,
    toggle_twilight_and_probe,
};
use visuals::apply_visuals;
use wayle_power_profiles::types::profile::PowerProfile;

// ── Model ─────────────────────────────────────────────────────────────────────

pub(crate) struct ControlCenterTilesModel {
    // Keep Awake
    keep_awake: bool,
    // DND
    dnd: bool,
    // Dark Mode
    dark: MatugenMode,
    // Twilight — enabled flag (drives the active tint) + the live profile
    // line shown as the tile subtitle (e.g. "Schedule · 3500K").
    twilight_enabled: bool,
    twilight_subtitle: String,
    // Airplane Mode
    airplane_mode: bool,
    airplane_available: bool,
    // Disk
    disk: DiskUsage,
    // Battery
    battery: BatterySnapshot,
    // Power profile
    power_profile: PowerProfile,
    // Wi-Fi subtitle + connected state
    wifi_subtitle: String,
    wifi_connected: bool,
    // Bluetooth subtitle + connected state
    bt_subtitle: String,
    bt_connected: bool,
    // Audio out subtitle
    audio_out_subtitle: String,
    // Mic subtitle
    mic_subtitle: String,
    // VPN subtitle + connected state
    vpn_subtitle: String,
    vpn_connected: bool,
    // Valent subtitle + connected state
    valent_subtitle: String,
    valent_connected: bool,
    // UFW firewall subtitle + active (enabled) state
    ufw_subtitle: String,
    ufw_active: bool,
    // Podman subtitle (active machine names / container summary) + active
    podman_subtitle: String,
    podman_active: bool,
    // Output mirroring subtitle (`eDP-1 → DP-3` / Off) + active state
    mirror_subtitle: String,
    mirror_active: bool,
    // Wi-Fi hotspot (NetworkManager AP) — status + clients, and a busy
    // flag so a second click while nmcli is starting the AP is ignored.
    hotspot: HotspotStatus,
    hotspot_busy: bool,
    // Mic noise suppression (RNNoise filter-chain) + in-flight flag.
    denoise: DenoiseStatus,
    denoise_busy: bool,
    // ThinkPad lid logo LED (tpacpi::lid_logo_dot) + its in-flight write.
    led: led::LedState,
    // The user-defined tile (label / icon / state from config + command).
    custom: custom::CustomState,
    // Lazy-start guard — watchers only start on first reveal
    watchers_started: bool,
    // Shared with the pollers (twilight / ufw / podman / disk / valent):
    // they wait on it with no timer armed while the panel is hidden, so a
    // closed control center neither wakes nor spawns `mctl` / `systemctl`
    // / `podman`.
    revealed: RevealGate,
    // The Podman and Valent tiles read the shared pill polls (one probe
    // loop for the whole shell, `podman events`-driven for Podman) instead
    // of probing on their own; active while the panel is revealed.
    podman_poll: PollSubscription<PodmanSummary>,
    valent_poll: PollSubscription<crate::valent::ValentReport>,
    // Edit mode — when true, all tiles visible + per-tile visibility toggles shown
    edit_mode: bool,
    // Current tile order — mirrors config; used to detect changes that need
    // a grid rebuild.
    tile_order: Vec<String>,
    // Current wide tiles — mirrors config; tiles in this set span 2 columns.
    wide_tiles: HashSet<String>,
    _effects: EffectScope,
}

/// Which detail sub-page to open (emitted to the parent menu widget).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DetailPage {
    Wifi,
    Bluetooth,
    AudioOut,
    Mic,
    Battery,
    Vpn,
    Valent,
    Twilight,
    KeepAwake,
    Ufw,
    Podman,
}

// ── Messages ──────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub(crate) enum ControlCenterTilesInput {
    /// Menu revealed or hidden.
    Reveal(bool),

    // Tile clicks — toggles
    ClickKeepAwake,
    ClickDnd,
    ClickDarkMode,
    ClickColorPicker,
    ClickAirplaneMode,
    /// Toggle output mirroring (`toggle_mirror`: laptop → external).
    ClickMirror,
    /// Secondary-click quick toggles on the expandable tiles (left-click
    /// opens the detail page, right-click flips the state).
    ToggleTwilight,
    ToggleBluetooth,
    /// Right-click on the Firewall tile → enable/disable UFW (pkexec).
    ToggleUfw,
    /// Right-click on the Podman tile → stop the running machine(s).
    StopPodman,
    /// Hotspot tile click → start or stop the access point.
    ToggleHotspot,
    /// Noise-suppression tile click → RNNoise virtual source on/off.
    ToggleDenoise,
    /// ThinkPad LED tile click → lid logo LED on/off.
    ToggleLed,
    /// Custom tile click → run its command.
    ClickCustom,

    /// Reactive dark-mode update from EffectScope.
    DarkModeChanged(MatugenMode),

    /// Re-read live subtitles for the expandable tiles.
    RefreshSubtitles,

    /// Enter or exit edit mode (forwarded from the pencil-icon toggle).
    SetEditMode(bool),

    /// Edit-overlay checkbox toggled for a tile — write to config.
    EditTileVisibility(TileId, bool),

    /// Config-driven tile order or wide_tiles changed — detach and reattach
    /// all grid children in the new order / column spans. Also fires on
    /// visibility changes so hidden gaps are eliminated in normal mode.
    RebuildGrid(Vec<String>, Vec<String>),
    /// Render fixture: replace the live state with canned values.
    #[cfg(test)]
    Fixture(Box<fixture::TilesFixture>),
}

#[derive(Debug)]
pub(crate) enum ControlCenterTilesOutput {
    /// User clicked an expandable tile; switch to this detail page.
    ExpandPage(DetailPage),
    /// mdash: open this menu (closes the dashboard).
    OpenMenu(crate::frame::MenuId),
}

#[derive(Debug)]
pub(crate) enum ControlCenterTilesCommandOutput {
    KeepAwakeChanged,
    DndChanged,
    /// Twilight poll result — (enabled, profile-line subtitle).
    TwilightChanged(bool, String),
    BatteryChanged,
    DiskRefreshed(DiskUsage),
    SubtitlesRefreshed,
    /// Async valent probe finished — (subtitle, is_connected).
    ValentStateRefreshed(String, bool),
    /// Async UFW probe finished — (subtitle, is_active).
    UfwRefreshed(String, bool),
    /// Async Podman probe finished — (subtitle, is_active).
    PodmanRefreshed(String, bool),
    /// Mirror probe finished — (subtitle, is_active).
    MirrorRefreshed(String, bool),
    /// Hotspot probe finished (after a poll or a start/stop).
    HotspotRefreshed(HotspotStatus),
    /// Noise-suppression probe finished (poll or after a toggle).
    DenoiseRefreshed(DenoiseStatus),
    /// ThinkPad LED re-read after a toggle (with its error, if any).
    LedRefreshed(led::LedState),
    /// Custom tile re-read (config + state command).
    CustomRefreshed(custom::CustomState),
}

// ── Component ─────────────────────────────────────────────────────────────────

impl Component for ControlCenterTilesModel {
    type CommandOutput = ControlCenterTilesCommandOutput;
    type Input = ControlCenterTilesInput;
    type Output = ControlCenterTilesOutput;
    type Init = ControlCenterTilesInit;
    type Root = gtk::Grid;
    type Widgets = ControlCenterTilesWidgets;

    fn init_root() -> Self::Root {
        let grid = gtk::Grid::new();
        grid.add_css_class("control-center-grid");
        grid.set_column_homogeneous(true);
        // Roomier inter-tile gaps for a GNOME-like, less cramped grid.
        grid.set_row_spacing(12);
        grid.set_column_spacing(12);
        grid.set_hexpand(true);
        grid
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Build every tile from the registry and wire its clicks + its
        // edit-mode overlay. Wide tile CSS is applied by rebuild_grid from
        // the `wide_tiles` config.
        let widgets = ControlCenterTilesWidgets::build(&sender, params.layout, &root, params.live);

        // Reactive dark-mode effect (always active — cheap config-store watch)
        let mut effects = EffectScope::new();
        {
            let s = sender.clone();
            effects.push(move |_| {
                let mode = config_manager().config().theme().matugen().mode().get();
                s.input(ControlCenterTilesInput::DarkModeChanged(mode));
            });
        }

        // Reactive tile-order / visibility / wide_tiles effect — fires whenever
        // tile_order, wide_tiles, or any per-tile visibility bool changes in
        // config. Sends `RebuildGrid` so the grid re-renders in the new order
        // with correct spans and hidden tiles removed in normal mode.
        {
            let s = sender.clone();
            effects.push(move |_| {
                // Touch every visibility bool + tile_order + wide_tiles to
                // subscribe to their changes.
                let cm = config_manager();
                for id in TileId::all() {
                    let _ = (id.config().track_visible)();
                }

                let order = cm.config().control_center().tile_order().get();
                let wide = cm.config().control_center().wide_tiles().get();
                s.input(ControlCenterTilesInput::RebuildGrid(order, wide));
            });
        }

        // Snapshot initial state
        let keep_awake = IdleInhibitor::global().get();
        let dnd = notification_service().map(|s| s.dnd.get()).unwrap_or(false);
        let dark = config_manager()
            .config()
            .theme()
            .matugen()
            .mode()
            .get_untracked();
        let disk = read_disk_usage();
        let battery = read_battery();
        let power_profile = power_profile_service()
            .map(|s| s.power_profiles.active_profile.get())
            .unwrap_or(PowerProfile::Unknown);
        let (wifi_subtitle, wifi_connected) = read_wifi_state();
        let (bt_subtitle, bt_connected) = read_bt_state();
        let (vpn_subtitle, vpn_connected) = read_vpn_state();
        let (valent_subtitle, valent_connected) = (String::from("…"), false);

        // Airplane mode initial state (wifi disabled = airplane on)
        let (airplane_mode, airplane_available) = read_airplane_state();

        let initial_tile_order = config_manager()
            .config()
            .control_center()
            .tile_order()
            .get_untracked();
        let initial_wide_tiles: HashSet<String> = config_manager()
            .config()
            .control_center()
            .wide_tiles()
            .get_untracked()
            .into_iter()
            .collect();

        let model = ControlCenterTilesModel {
            keep_awake,
            dnd,
            dark,
            twilight_enabled: false,
            twilight_subtitle: String::from("…"),
            airplane_mode,
            airplane_available,
            disk,
            battery,
            power_profile,
            wifi_subtitle,
            wifi_connected,
            bt_subtitle,
            bt_connected,
            audio_out_subtitle: read_audio_out_subtitle(),
            mic_subtitle: read_mic_subtitle(),
            vpn_subtitle,
            vpn_connected,
            valent_subtitle,
            valent_connected,
            ufw_subtitle: String::from("…"),
            ufw_active: false,
            podman_subtitle: String::from("…"),
            podman_active: false,
            mirror_subtitle: String::from("…"),
            mirror_active: false,
            hotspot: HotspotStatus::default(),
            hotspot_busy: false,
            denoise: DenoiseStatus::default(),
            denoise_busy: false,
            led: led::probe(),
            custom: custom::CustomState::default(),
            revealed: RevealGate::new(false),
            podman_poll: PODMAN_POLL.subscribe(),
            valent_poll: crate::valent::VALENT_POLL.subscribe(),
            watchers_started: false,
            edit_mode: false,
            tile_order: initial_tile_order.clone(),
            wide_tiles: initial_wide_tiles.clone(),
            _effects: effects,
        };

        // Attach all overlays to the grid in config order with correct spans.
        widgets.rebuild_grid(&root, &initial_tile_order, &initial_wide_tiles);

        // Apply initial visual state
        apply_visuals(&model, &widgets);

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            ControlCenterTilesInput::Reveal(true) => {
                self.revealed.set(true);
                self.podman_poll.set_active(true);
                self.valent_poll.set_active(true);
                if !self.watchers_started {
                    self.watchers_started = true;
                    pollers::start_watchers(self, &sender);
                }

                // Re-snapshot fast values on each reveal
                self.keep_awake = IdleInhibitor::global().get();
                self.dnd = notification_service().map(|s| s.dnd.get()).unwrap_or(false);
                self.dark = config_manager()
                    .config()
                    .theme()
                    .matugen()
                    .mode()
                    .get_untracked();
                self.battery = read_battery();
                self.power_profile = power_profile_service()
                    .map(|s| s.power_profiles.active_profile.get())
                    .unwrap_or(PowerProfile::Unknown);
                self.disk = read_disk_usage();
                (self.wifi_subtitle, self.wifi_connected) = read_wifi_state();
                (self.bt_subtitle, self.bt_connected) = read_bt_state();
                self.audio_out_subtitle = read_audio_out_subtitle();
                self.mic_subtitle = read_mic_subtitle();
                (self.vpn_subtitle, self.vpn_connected) = read_vpn_state();
                (self.airplane_mode, self.airplane_available) = read_airplane_state();
                if !self.led.busy {
                    self.led = led::probe();
                }
                custom::refresh(&sender, false);
            }

            ControlCenterTilesInput::Reveal(false) => {
                self.revealed.set(false);
                self.podman_poll.set_active(false);
                self.valent_poll.set_active(false);
            }

            ControlCenterTilesInput::RefreshSubtitles => {
                (self.wifi_subtitle, self.wifi_connected) = read_wifi_state();
                (self.bt_subtitle, self.bt_connected) = read_bt_state();
                self.audio_out_subtitle = read_audio_out_subtitle();
                self.mic_subtitle = read_mic_subtitle();
                (self.vpn_subtitle, self.vpn_connected) = read_vpn_state();
                (self.airplane_mode, self.airplane_available) = read_airplane_state();
                self.power_profile = power_profile_service()
                    .map(|s| s.power_profiles.active_profile.get())
                    .unwrap_or(PowerProfile::Unknown);
            }

            ControlCenterTilesInput::ClickKeepAwake => {
                tokio::spawn(async move {
                    let inhibitor = IdleInhibitor::global();
                    let _ = inhibitor.toggle().await;
                });
            }

            ControlCenterTilesInput::ClickDnd => {
                if let Some(service) = notification_service() {
                    let current = service.dnd.get();
                    service.set_dnd(!current);
                }
            }

            ControlCenterTilesInput::ClickDarkMode => {
                config_manager().update_config(|config| {
                    config.theme.matugen.mode = match config.theme.matugen.mode {
                        MatugenMode::Light => MatugenMode::Dark,
                        MatugenMode::Dark => MatugenMode::Light,
                    };
                });
            }

            ControlCenterTilesInput::ClickColorPicker => {
                spawn_color_picker(300);
            }

            ControlCenterTilesInput::ClickAirplaneMode => {
                // Airplane mode = disable Wi-Fi (toggle wifi enabled state)
                if let Some(wifi) = network_service().and_then(|n| n.wifi.get()) {
                    let new_enabled = !wifi.enabled.get();
                    tokio::spawn(async move {
                        let _ = wifi.set_enabled(new_enabled).await;
                    });
                }
            }

            ControlCenterTilesInput::ClickMirror => {
                // Toggle, then re-probe so the tile shows the new pair (or
                // why margo refused, e.g. no second display).
                sender.command(|out, _shutdown| async move {
                    let (subtitle, active) = toggle_mirror_and_probe().await;
                    let _ = out.send(ControlCenterTilesCommandOutput::MirrorRefreshed(
                        subtitle, active,
                    ));
                });
            }

            ControlCenterTilesInput::ToggleTwilight => {
                // Right-click quick toggle: flip the filter, then re-probe
                // so the subtitle (active profile) updates promptly.
                sender.command(|out, _shutdown| async move {
                    if let Some((enabled, subtitle)) = toggle_twilight_and_probe().await {
                        let _ = out.send(ControlCenterTilesCommandOutput::TwilightChanged(
                            enabled, subtitle,
                        ));
                    }
                });
            }

            ControlCenterTilesInput::ToggleBluetooth => {
                // Right-click quick toggle: flip the adapter power. The 5s
                // subtitle poller picks up the new state. No-op without an
                // adapter.
                if let Some(bt) = bluetooth_service() {
                    let enabled = bt.enabled.get();
                    tokio::spawn(async move {
                        if enabled {
                            let _ = bt.disable().await;
                        } else {
                            let _ = bt.enable().await;
                        }
                    });
                }
            }

            ControlCenterTilesInput::ToggleUfw => {
                // Right-click quick toggle: turn UFW on/off through the
                // audited root helper (one polkit prompt), then re-probe.
                use mshell_privileged::ufw::{Switch, UfwOp};
                let op = UfwOp::Switch(if self.ufw_active {
                    Switch::Off
                } else {
                    Switch::On
                });
                sender.command(move |out, _shutdown| async move {
                    if let Err(e) = crate::bars::bar_widgets::ufw::service::run(op).await
                        && e != "Authorization was cancelled"
                    {
                        crate::bars::bar_widgets::ufw::service::report(
                            "Couldn't switch the firewall",
                            Err(e),
                        );
                    }
                    let (subtitle, active) = read_ufw_tile_state().await;
                    let _ = out.send(ControlCenterTilesCommandOutput::UfwRefreshed(
                        subtitle, active,
                    ));
                });
            }

            ControlCenterTilesInput::StopPodman => {
                // Right-click quick action: stop any running machine(s),
                // then re-probe the tile state.
                sender.command(move |out, _shutdown| async move {
                    stop_podman_machines().await;
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    let (subtitle, active) = read_podman_tile_state().await;
                    let _ = out.send(ControlCenterTilesCommandOutput::PodmanRefreshed(
                        subtitle, active,
                    ));
                });
            }

            ControlCenterTilesInput::ToggleHotspot => {
                if self.hotspot_busy || !self.hotspot.available {
                    return;
                }
                self.hotspot_busy = true;
                let active = self.hotspot.active.clone();
                let wifi_iface = self.hotspot.wifi_interface.clone();
                sender.command(move |out, _shutdown| async move {
                    let _ = hotspot::toggle(active, wifi_iface).await;
                    let _ = out.send(ControlCenterTilesCommandOutput::HotspotRefreshed(
                        hotspot::probe(&hotspot::configured_interface()).await,
                    ));
                });
            }

            ControlCenterTilesInput::ToggleDenoise => {
                if self.denoise_busy || !self.denoise.available() {
                    return;
                }
                self.denoise_busy = true;
                let enabled = self.denoise.enabled;
                sender.command(move |out, _shutdown| async move {
                    if let Err(e) = denoise::toggle(enabled).await {
                        denoise::toast_failure(&e);
                    }
                    let _ = out.send(ControlCenterTilesCommandOutput::DenoiseRefreshed(
                        denoise::probe().await,
                    ));
                });
            }

            ControlCenterTilesInput::ToggleLed => led::toggle(self, &sender),
            ControlCenterTilesInput::ClickCustom => custom::click(&sender),

            ControlCenterTilesInput::DarkModeChanged(mode) => {
                self.dark = mode;
            }

            ControlCenterTilesInput::SetEditMode(on) => {
                self.edit_mode = on;
            }

            ControlCenterTilesInput::EditTileVisibility(tile, visible) => {
                let set_visible = tile.config().set_visible;
                config_manager().update_config(|c| set_visible(&mut c.control_center, visible));

                // The RebuildGrid effect will fire automatically from the config
                // store change. No explicit rebuild needed here.
            }

            ControlCenterTilesInput::RebuildGrid(order, wide) => {
                // Always rebuild — order, visibility, or wide_tiles may have changed.
                self.tile_order = order.clone();
                self.wide_tiles = wide.iter().cloned().collect();
                widgets.rebuild_grid(root, &order, &self.wide_tiles);
            }
            #[cfg(test)]
            ControlCenterTilesInput::Fixture(f) => f.apply(self),
        }

        apply_visuals(self, widgets);
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            ControlCenterTilesCommandOutput::KeepAwakeChanged => {
                self.keep_awake = IdleInhibitor::global().get();
            }
            ControlCenterTilesCommandOutput::DndChanged => {
                self.dnd = notification_service().map(|s| s.dnd.get()).unwrap_or(false);
            }
            ControlCenterTilesCommandOutput::TwilightChanged(enabled, subtitle) => {
                self.twilight_enabled = enabled;
                self.twilight_subtitle = subtitle;
            }
            ControlCenterTilesCommandOutput::BatteryChanged => {
                self.battery = read_battery();
                self.power_profile = power_profile_service()
                    .map(|s| s.power_profiles.active_profile.get())
                    .unwrap_or(PowerProfile::Unknown);
            }
            ControlCenterTilesCommandOutput::DiskRefreshed(usage) => {
                self.disk = usage;
            }
            ControlCenterTilesCommandOutput::SubtitlesRefreshed => {
                sender.input(ControlCenterTilesInput::RefreshSubtitles);
            }
            ControlCenterTilesCommandOutput::ValentStateRefreshed(subtitle, connected) => {
                self.valent_subtitle = subtitle;
                self.valent_connected = connected;
            }
            ControlCenterTilesCommandOutput::UfwRefreshed(subtitle, active) => {
                self.ufw_subtitle = subtitle;
                self.ufw_active = active;
            }
            ControlCenterTilesCommandOutput::PodmanRefreshed(subtitle, active) => {
                self.podman_subtitle = subtitle;
                self.podman_active = active;
            }
            ControlCenterTilesCommandOutput::MirrorRefreshed(subtitle, active) => {
                self.mirror_subtitle = subtitle;
                self.mirror_active = active;
            }
            ControlCenterTilesCommandOutput::HotspotRefreshed(status) => {
                self.hotspot = status;
                self.hotspot_busy = false;
            }
            ControlCenterTilesCommandOutput::DenoiseRefreshed(status) => {
                self.denoise = status;
                self.denoise_busy = false;
            }
            ControlCenterTilesCommandOutput::LedRefreshed(state) => self.led = state,
            ControlCenterTilesCommandOutput::CustomRefreshed(state) => self.custom = state,
        }

        apply_visuals(self, widgets);
    }
}
