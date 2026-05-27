//! Control Center tile grid — 2-column grid of toggle/info tiles.
//!
//! Layout (col 0 / col 1):
//!   [Wi-Fi      ]  [Bluetooth   ]
//!   [Audio Out  ]  [Mic         ]
//!   [Keep Awake ]  [Color Picker]
//!   [Do Not Disturb    (wide)  ]
//!   [Dark Mode  ]  [Night Light ]
//!   [Disk       ]  [Battery     ]
//!
//! Wi-Fi, Bluetooth, Audio Out, Mic, and Battery tiles are *expandable*:
//! clicking them emits `ControlCenterTilesOutput::ExpandPage` so the parent
//! `ControlCenterMenuWidgetModel` can switch the Stack to the matching detail
//! sub-page.
//!
//! Dark Mode and Night Light are `.small` (icon-only).
//! Do Not Disturb is `.wide` (spans 2 columns).
//! Battery tile is hidden when no battery is present.
//!
//! All stateful tiles subscribe to their respective service watchers and
//! start those watchers lazily on the first `Reveal(true)`.

use crate::menus::menu_widgets::control_center::tile::{
    TileWidget, build_small_tile, build_tile,
};
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    ConfigStoreFields, MatugenStoreFields, ThemeStoreFields,
};
use mshell_config::schema::themes::MatugenMode;
use mshell_idle::inhibitor::IdleInhibitor;
use mshell_services::{
    audio_service, battery_service, bluetooth_service, line_power_service, network_service,
    notification_service,
};
use mshell_utils::battery::{
    get_battery_icon, get_charging_battery_icon, spawn_battery_online_watcher,
    spawn_battery_watcher,
};
use mshell_utils::idle::spawn_idle_inhibitor_watcher;
use mshell_utils::notifications::spawn_dnd_watcher;
use mshell_utils::picker::spawn_color_picker;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk;
use relm4::gtk::prelude::{ButtonExt, GridExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender};
use std::time::Duration;
use tracing::warn;
use wayle_battery::types::DeviceState;

const POLL_INTERVAL: Duration = Duration::from_secs(5);
const STARTUP_DELAY: Duration = Duration::from_millis(200);
const POST_TOGGLE_DELAY: Duration = Duration::from_millis(150);

// ── Model ─────────────────────────────────────────────────────────────────────

pub(crate) struct ControlCenterTilesModel {
    // Keep Awake
    keep_awake: bool,
    // DND
    dnd: bool,
    // Dark Mode
    dark: MatugenMode,
    // Night Light
    night_light: bool,
    // Disk
    disk: DiskUsage,
    // Battery
    battery: BatterySnapshot,
    // Wi-Fi subtitle
    wifi_subtitle: String,
    // Bluetooth subtitle
    bt_subtitle: String,
    // Audio out subtitle
    audio_out_subtitle: String,
    // Mic subtitle
    mic_subtitle: String,
    // Lazy-start guard — watchers only start on first reveal
    watchers_started: bool,
    _effects: EffectScope,
}

#[derive(Debug, Clone)]
#[derive(Default)]
pub(crate) struct DiskUsage {
    used_bytes: u64,
    total_bytes: u64,
}

impl DiskUsage {
    fn format(&self) -> String {
        let used = bytes_to_gib(self.used_bytes);
        let total = bytes_to_gib(self.total_bytes);
        let pct = if self.total_bytes > 0 {
            (self.used_bytes as f64 / self.total_bytes as f64 * 100.0).round() as u64
        } else {
            0
        };
        format!("{used:.1}G / {total:.1}G ({pct}%)")
    }
}

#[derive(Debug, Clone)]
struct BatterySnapshot {
    present: bool,
    percent: u8,
    state: DeviceState,
    on_ac: bool,
}

impl Default for BatterySnapshot {
    fn default() -> Self {
        Self {
            present: false,
            percent: 0,
            state: DeviceState::Unknown,
            on_ac: false,
        }
    }
}

/// Which detail sub-page to open (emitted to the parent menu widget).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DetailPage {
    Wifi,
    Bluetooth,
    AudioOut,
    Mic,
    Battery,
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
    ClickNightLight,
    ClickColorPicker,

    /// Reactive dark-mode update from EffectScope.
    DarkModeChanged(MatugenMode),

    /// Re-read live subtitles for the expandable tiles.
    RefreshSubtitles,
}

#[derive(Debug)]
pub(crate) enum ControlCenterTilesOutput {
    /// User clicked an expandable tile; switch to this detail page.
    ExpandPage(DetailPage),
}

pub(crate) struct ControlCenterTilesInit {}

#[derive(Debug)]
pub(crate) enum ControlCenterTilesCommandOutput {
    KeepAwakeChanged,
    DndChanged,
    NightLightChanged(bool),
    BatteryChanged,
    DiskRefreshed(DiskUsage),
    SubtitlesRefreshed,
}

// ── Widgets struct (manual — we hold the tile handles) ────────────────────────

pub(crate) struct ControlCenterTilesWidgets {
    // Expandable tiles
    tile_wifi: TileWidget,
    tile_bluetooth: TileWidget,
    tile_audio_out: TileWidget,
    tile_mic: TileWidget,
    // Toggle / info tiles
    tile_keep_awake: TileWidget,
    // Held alive so the click handler isn't dropped; not updated by apply_visuals
    // (color picker has no state).
    #[allow(dead_code)]
    tile_color_picker: TileWidget,
    tile_dnd: TileWidget,
    tile_dark_mode: TileWidget,
    tile_night_light: TileWidget,
    tile_disk: TileWidget,
    tile_battery: TileWidget,
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
        grid.set_row_spacing(8);
        grid.set_column_spacing(8);
        grid.set_hexpand(true);
        grid
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // ── Expandable tiles (top section) ──────────────────────────────────
        let tile_wifi = build_tile("network-wireless-symbolic", "Wi-Fi", "…");
        let tile_bluetooth = build_tile("bluetooth-active-symbolic", "Bluetooth", "…");
        let tile_audio_out = build_tile("audio-speakers-symbolic", "Audio Out", "…");
        let tile_mic = build_tile("audio-input-microphone-symbolic", "Mic", "…");

        // Mark expandable tiles with a chevron-styled hint (`.expandable`)
        for tw in [&tile_wifi, &tile_bluetooth, &tile_audio_out, &tile_mic] {
            tw.button.add_css_class("expandable");
        }

        // ── Toggle / info tiles ─────────────────────────────────────────────
        let tile_keep_awake = build_tile("eye-symbolic", "Keep Awake", "Off");
        let tile_color_picker = build_tile("color-select-symbolic", "Color Picker", "Pick a colour");
        let tile_dnd = build_tile(
            "notification-symbolic",
            "Do Not Disturb",
            "All notifications",
        );
        let tile_dark_mode = build_small_tile("weather-clear-night-symbolic");
        let tile_night_light = build_small_tile("nightlight-symbolic");
        let tile_disk = build_tile("drive-harddisk-symbolic", "Disk", "");
        let tile_battery = build_tile("battery-level-50-symbolic", "Battery", "");

        // Mark the DND tile as wide
        tile_dnd.button.add_css_class("wide");
        // Battery tile is also expandable
        tile_battery.button.add_css_class("expandable");

        // Attach tiles to the grid:
        //   Row 0: [Wi-Fi (0,0)] [Bluetooth (1,0)]
        //   Row 1: [Audio Out (0,1)] [Mic (1,1)]
        //   Row 2: [Keep Awake (0,2)] [Color Picker (1,2)]
        //   Row 3: [DND (0,3) span 2]
        //   Row 4: [Dark Mode (0,4)] [Night Light (1,4)]
        //   Row 5: [Disk (0,5)] [Battery (1,5)]
        root.attach(&tile_wifi.button, 0, 0, 1, 1);
        root.attach(&tile_bluetooth.button, 1, 0, 1, 1);
        root.attach(&tile_audio_out.button, 0, 1, 1, 1);
        root.attach(&tile_mic.button, 1, 1, 1, 1);
        root.attach(&tile_keep_awake.button, 0, 2, 1, 1);
        root.attach(&tile_color_picker.button, 1, 2, 1, 1);
        root.attach(&tile_dnd.button, 0, 3, 2, 1);
        root.attach(&tile_dark_mode.button, 0, 4, 1, 1);
        root.attach(&tile_night_light.button, 1, 4, 1, 1);
        root.attach(&tile_disk.button, 0, 5, 1, 1);
        root.attach(&tile_battery.button, 1, 5, 1, 1);

        // Wire expandable tile clicks → outputs
        {
            let s = sender.clone();
            tile_wifi.button.connect_clicked(move |_| {
                s.output(ControlCenterTilesOutput::ExpandPage(DetailPage::Wifi))
                    .ok();
            });
        }
        {
            let s = sender.clone();
            tile_bluetooth.button.connect_clicked(move |_| {
                s.output(ControlCenterTilesOutput::ExpandPage(DetailPage::Bluetooth))
                    .ok();
            });
        }
        {
            let s = sender.clone();
            tile_audio_out.button.connect_clicked(move |_| {
                s.output(ControlCenterTilesOutput::ExpandPage(DetailPage::AudioOut))
                    .ok();
            });
        }
        {
            let s = sender.clone();
            tile_mic.button.connect_clicked(move |_| {
                s.output(ControlCenterTilesOutput::ExpandPage(DetailPage::Mic))
                    .ok();
            });
        }
        {
            let s = sender.clone();
            tile_battery.button.connect_clicked(move |_| {
                s.output(ControlCenterTilesOutput::ExpandPage(DetailPage::Battery))
                    .ok();
            });
        }

        // Wire toggle tile clicks
        {
            let s = sender.clone();
            tile_keep_awake
                .button
                .connect_clicked(move |_| s.input(ControlCenterTilesInput::ClickKeepAwake));
        }
        {
            let s = sender.clone();
            tile_dnd
                .button
                .connect_clicked(move |_| s.input(ControlCenterTilesInput::ClickDnd));
        }
        {
            let s = sender.clone();
            tile_dark_mode
                .button
                .connect_clicked(move |_| s.input(ControlCenterTilesInput::ClickDarkMode));
        }
        {
            let s = sender.clone();
            tile_night_light
                .button
                .connect_clicked(move |_| s.input(ControlCenterTilesInput::ClickNightLight));
        }
        {
            let s = sender.clone();
            tile_color_picker
                .button
                .connect_clicked(move |_| s.input(ControlCenterTilesInput::ClickColorPicker));
        }
        // Disk tile is info-only; no click handler needed.

        // Reactive dark-mode effect (always active — cheap config-store watch)
        let mut effects = EffectScope::new();
        {
            let s = sender.clone();
            effects.push(move |_| {
                let mode = config_manager()
                    .config()
                    .theme()
                    .matugen()
                    .mode()
                    .get();
                s.input(ControlCenterTilesInput::DarkModeChanged(mode));
            });
        }

        // Snapshot initial state
        let keep_awake = IdleInhibitor::global().get();
        let dnd = notification_service().dnd.get();
        let dark = config_manager()
            .config()
            .theme()
            .matugen()
            .mode()
            .get_untracked();
        let disk = read_disk_usage();
        let battery = read_battery();

        let model = ControlCenterTilesModel {
            keep_awake,
            dnd,
            dark,
            night_light: false,
            disk,
            battery,
            wifi_subtitle: read_wifi_subtitle(),
            bt_subtitle: read_bt_subtitle(),
            audio_out_subtitle: read_audio_out_subtitle(),
            mic_subtitle: read_mic_subtitle(),
            watchers_started: false,
            _effects: effects,
        };

        let widgets = ControlCenterTilesWidgets {
            tile_wifi,
            tile_bluetooth,
            tile_audio_out,
            tile_mic,
            tile_keep_awake,
            tile_color_picker,
            tile_dnd,
            tile_dark_mode,
            tile_night_light,
            tile_disk,
            tile_battery,
        };

        // Apply initial visual state
        apply_visuals(&model, &widgets);

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            ControlCenterTilesInput::Reveal(true) => {
                if !self.watchers_started {
                    self.watchers_started = true;

                    // Keep Awake watcher
                    spawn_idle_inhibitor_watcher(&sender, || {
                        ControlCenterTilesCommandOutput::KeepAwakeChanged
                    });

                    // DND watcher
                    spawn_dnd_watcher(&sender, || ControlCenterTilesCommandOutput::DndChanged);

                    // Battery watchers
                    spawn_battery_watcher(&sender, || ControlCenterTilesCommandOutput::BatteryChanged);
                    spawn_battery_online_watcher(&sender, || ControlCenterTilesCommandOutput::BatteryChanged);

                    // Night Light poller
                    sender.command(|out, shutdown| async move {
                        let shutdown_fut = shutdown.wait();
                        tokio::pin!(shutdown_fut);
                        let mut first = true;
                        loop {
                            let delay = if first { STARTUP_DELAY } else { POLL_INTERVAL };
                            first = false;
                            tokio::select! {
                                () = &mut shutdown_fut => break,
                                _ = tokio::time::sleep(delay) => {}
                            }
                            if let Some(enabled) = probe_twilight_enabled().await {
                                let _ = out.send(ControlCenterTilesCommandOutput::NightLightChanged(enabled));
                            }
                        }
                    });

                    // Disk refresh (once on reveal, then every 30s)
                    sender.command(|out, shutdown| async move {
                        let shutdown_fut = shutdown.wait();
                        tokio::pin!(shutdown_fut);
                        let mut first = true;
                        loop {
                            let delay = if first {
                                Duration::ZERO
                            } else {
                                Duration::from_secs(30)
                            };
                            first = false;
                            tokio::select! {
                                () = &mut shutdown_fut => break,
                                _ = tokio::time::sleep(delay) => {}
                            }
                            let usage = read_disk_usage();
                            let _ = out.send(ControlCenterTilesCommandOutput::DiskRefreshed(usage));
                        }
                    });

                    // Subtitle poller for Wi-Fi / BT / audio tiles (every 5s)
                    sender.command(|out, shutdown| async move {
                        let shutdown_fut = shutdown.wait();
                        tokio::pin!(shutdown_fut);
                        let mut first = true;
                        loop {
                            let delay = if first { STARTUP_DELAY } else { POLL_INTERVAL };
                            first = false;
                            tokio::select! {
                                () = &mut shutdown_fut => break,
                                _ = tokio::time::sleep(delay) => {}
                            }
                            let _ = out.send(ControlCenterTilesCommandOutput::SubtitlesRefreshed);
                        }
                    });
                }

                // Re-snapshot fast values on each reveal
                self.keep_awake = IdleInhibitor::global().get();
                self.dnd = notification_service().dnd.get();
                self.dark = config_manager()
                    .config()
                    .theme()
                    .matugen()
                    .mode()
                    .get_untracked();
                self.battery = read_battery();
                self.disk = read_disk_usage();
                self.wifi_subtitle = read_wifi_subtitle();
                self.bt_subtitle = read_bt_subtitle();
                self.audio_out_subtitle = read_audio_out_subtitle();
                self.mic_subtitle = read_mic_subtitle();
            }

            ControlCenterTilesInput::Reveal(false) => {}

            ControlCenterTilesInput::RefreshSubtitles => {
                self.wifi_subtitle = read_wifi_subtitle();
                self.bt_subtitle = read_bt_subtitle();
                self.audio_out_subtitle = read_audio_out_subtitle();
                self.mic_subtitle = read_mic_subtitle();
            }

            ControlCenterTilesInput::ClickKeepAwake => {
                tokio::spawn(async move {
                    let inhibitor = IdleInhibitor::global();
                    let _ = inhibitor.toggle().await;
                });
            }

            ControlCenterTilesInput::ClickDnd => {
                let service = notification_service();
                let current = service.dnd.get();
                service.set_dnd(!current);
            }

            ControlCenterTilesInput::ClickDarkMode => {
                config_manager().update_config(|config| {
                    config.theme.matugen.mode = match config.theme.matugen.mode {
                        MatugenMode::Light => MatugenMode::Dark,
                        MatugenMode::Dark => MatugenMode::Light,
                    };
                });
            }

            ControlCenterTilesInput::ClickNightLight => {
                sender.command(|out, _shutdown| async move {
                    run_twilight_toggle().await;
                    tokio::time::sleep(POST_TOGGLE_DELAY).await;
                    if let Some(enabled) = probe_twilight_enabled().await {
                        let _ = out.send(ControlCenterTilesCommandOutput::NightLightChanged(enabled));
                    }
                });
            }

            ControlCenterTilesInput::ClickColorPicker => {
                spawn_color_picker(300);
            }

            ControlCenterTilesInput::DarkModeChanged(mode) => {
                self.dark = mode;
            }
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
                self.dnd = notification_service().dnd.get();
            }
            ControlCenterTilesCommandOutput::NightLightChanged(enabled) => {
                self.night_light = enabled;
            }
            ControlCenterTilesCommandOutput::BatteryChanged => {
                self.battery = read_battery();
            }
            ControlCenterTilesCommandOutput::DiskRefreshed(usage) => {
                self.disk = usage;
            }
            ControlCenterTilesCommandOutput::SubtitlesRefreshed => {
                sender.input(ControlCenterTilesInput::RefreshSubtitles);
            }
        }

        apply_visuals(self, widgets);
    }
}

// ── Visual updater ─────────────────────────────────────────────────────────────

fn apply_visuals(model: &ControlCenterTilesModel, w: &ControlCenterTilesWidgets) {
    // Wi-Fi
    w.tile_wifi.set_subtitle(&model.wifi_subtitle);

    // Bluetooth
    w.tile_bluetooth.set_subtitle(&model.bt_subtitle);

    // Audio Out
    w.tile_audio_out.set_subtitle(&model.audio_out_subtitle);

    // Mic
    w.tile_mic.set_subtitle(&model.mic_subtitle);

    // Keep Awake
    w.tile_keep_awake.set_active(model.keep_awake);
    w.tile_keep_awake
        .set_subtitle(if model.keep_awake { "On" } else { "Off" });
    w.tile_keep_awake.set_icon(if model.keep_awake {
        "eye-symbolic"
    } else {
        "eye-off-symbolic"
    });

    // DND
    w.tile_dnd.set_active(model.dnd);
    w.tile_dnd.set_subtitle(if model.dnd {
        "Silenced"
    } else {
        "All notifications"
    });
    w.tile_dnd.set_icon(if model.dnd {
        "notification-disabled-symbolic"
    } else {
        "notification-symbolic"
    });

    // Dark Mode
    let is_dark = model.dark == MatugenMode::Dark;
    w.tile_dark_mode.set_active(is_dark);
    w.tile_dark_mode.set_icon(match model.dark {
        MatugenMode::Dark => "weather-clear-symbolic",
        MatugenMode::Light => "weather-clear-night-symbolic",
    });

    // Night Light
    w.tile_night_light.set_active(model.night_light);
    w.tile_night_light.set_icon(if model.night_light {
        "nightlight-symbolic"
    } else {
        "nightlight-disabled-symbolic"
    });

    // Disk
    w.tile_disk.set_subtitle(&model.disk.format());

    // Battery
    let bat = &model.battery;
    w.tile_battery.set_visible(bat.present);
    if bat.present {
        let on_ac = bat.on_ac
            || bat.state == DeviceState::Charging
            || bat.state == DeviceState::FullyCharged;
        let icon = if on_ac {
            get_charging_battery_icon(bat.percent as f64)
        } else {
            get_battery_icon(bat.percent as f64)
        };
        w.tile_battery.set_icon(icon);
        let status = match bat.state {
            DeviceState::Charging => "Charging",
            DeviceState::Discharging => "Discharging",
            DeviceState::FullyCharged => "Full",
            DeviceState::Empty => "Empty",
            DeviceState::PendingCharge | DeviceState::PendingDischarge => "Not charging",
            DeviceState::Unknown => "",
        };
        let subtitle = if status.is_empty() {
            format!("{}%", bat.percent)
        } else {
            format!("{}% • {}", bat.percent, status)
        };
        w.tile_battery.set_subtitle(&subtitle);
    }
}

// ── Service helpers ────────────────────────────────────────────────────────────

fn read_battery() -> BatterySnapshot {
    let service = battery_service();
    let dev = &service.device;
    let present = dev.is_present.get();
    if !present {
        return BatterySnapshot::default();
    }
    let percent = dev.percentage.get().round().clamp(0.0, 100.0) as u8;
    let state = dev.state.get();
    let on_ac = line_power_service()
        .map(|s| s.device.online.get())
        .unwrap_or(false);
    BatterySnapshot {
        present,
        percent,
        state,
        on_ac,
    }
}

/// Read `/` filesystem usage via `libc::statvfs`. Returns zeros on failure.
fn read_disk_usage() -> DiskUsage {
    use std::ffi::CString;
    use std::mem::MaybeUninit;

    let path = CString::new("/").unwrap();
    let mut stat: MaybeUninit<libc::statvfs64> = MaybeUninit::uninit();
    // SAFETY: path is a valid C string; stat is written by the syscall.
    let rc = unsafe { libc::statvfs64(path.as_ptr(), stat.as_mut_ptr()) };
    if rc != 0 {
        return DiskUsage::default();
    }
    // SAFETY: statvfs64 returned 0 → stat is fully initialised.
    let s = unsafe { stat.assume_init() };
    let block = s.f_frsize;
    let total = s.f_blocks * block;
    let avail = s.f_bavail * block; // unprivileged available blocks
    let used = total.saturating_sub(avail);
    DiskUsage {
        used_bytes: used,
        total_bytes: total,
    }
}

fn bytes_to_gib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0 * 1024.0)
}

// ── Night Light helpers (mirrors night_light.rs) ──────────────────────────────

async fn probe_twilight_enabled() -> Option<bool> {
    let out = tokio::process::Command::new("mctl")
        .args(["twilight", "status", "--json"])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    v.get("enabled")?.as_bool()
}

async fn run_twilight_toggle() {
    match tokio::process::Command::new("mctl")
        .args(["twilight", "toggle"])
        .status()
        .await
    {
        Ok(s) if s.success() => {}
        Ok(s) => warn!(?s, "mctl twilight toggle returned non-zero"),
        Err(e) => warn!(error = %e, "mctl twilight toggle spawn failed"),
    }
}

// ── Expandable-tile subtitle helpers ─────────────────────────────────────────

fn read_wifi_subtitle() -> String {
    let network = network_service();
    if let Some(wifi) = network.wifi.get() {
        if let Some(ssid) = wifi.ssid.get() {
            return ssid;
        }
        if wifi.enabled.get() {
            return "Not connected".to_string();
        }
        return "Off".to_string();
    }
    "Unavailable".to_string()
}

fn read_bt_subtitle() -> String {
    let bt = bluetooth_service();
    if !bt.available.get() {
        return "Unavailable".to_string();
    }
    if !bt.enabled.get() {
        return "Off".to_string();
    }
    let devices = bt.devices.get();
    let connected: Vec<_> = devices.iter().filter(|d| d.connected.get()).collect();
    match connected.len() {
        0 => "On · no devices".to_string(),
        1 => connected[0].alias.get(),
        n => format!("{n} connected"),
    }
}

fn read_audio_out_subtitle() -> String {
    let audio = audio_service();
    if let Some(dev) = audio.default_output.get() {
        return dev.description.get();
    }
    "No device".to_string()
}

fn read_mic_subtitle() -> String {
    let audio = audio_service();
    if let Some(dev) = audio.default_input.get() {
        return dev.description.get();
    }
    "No device".to_string()
}
