//! Control Center tile grid — 2-column grid of toggle/info tiles.
//!
//! The grid order is driven by `ControlCenterConfig::tile_order`. Wide tiles
//! (dnd, valent) span both columns. Tiles not present in `tile_order` append
//! at the end in canonical order so nothing silently disappears after an
//! upgrade.
//!
//! Wi-Fi, Bluetooth, Audio Out, Mic, Battery, VPN, and Valent tiles are
//! *expandable*: clicking them emits `ControlCenterTilesOutput::ExpandPage`
//! so the parent `ControlCenterMenuWidgetModel` can switch the Stack to
//! the matching detail sub-page.
//!
//! Dark Mode and Twilight are full labeled tiles (not small).
//! Do Not Disturb is `.wide` (spans 2 columns).
//! Battery tile is hidden when no battery is present.
//!
//! All stateful tiles subscribe to their respective service watchers and
//! start those watchers lazily on the first `Reveal(true)`.

use crate::menus::menu_widgets::control_center::tile::{
    TileWidget, build_tile,
};
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    ConfigStoreFields, ControlCenterConfigStoreFields, MatugenStoreFields, ThemeStoreFields,
};
use mshell_config::schema::themes::MatugenMode;
use mshell_idle::inhibitor::IdleInhibitor;
use mshell_services::{
    audio_service, battery_service, bluetooth_service, line_power_service, network_service,
    notification_service, power_profile_service,
};
use mshell_utils::battery::{
    get_battery_icon, get_charging_battery_icon, spawn_battery_online_watcher,
    spawn_battery_watcher,
};
use mshell_utils::idle::spawn_idle_inhibitor_watcher;
use mshell_utils::notifications::spawn_dnd_watcher;
use mshell_utils::picker::spawn_color_picker;
use mshell_utils::power_profile::get_power_profile_label;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk;
use relm4::gtk::prelude::{ButtonExt, CheckButtonExt, GridExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender};
use std::time::Duration;
use tracing::warn;
use wayle_battery::types::DeviceState;
use wayle_power_profiles::types::profile::PowerProfile;

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
    // Twilight (Night Light)
    night_light: bool,
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
    // Lazy-start guard — watchers only start on first reveal
    watchers_started: bool,
    // Edit mode — when true, all tiles visible + per-tile visibility toggles shown
    edit_mode: bool,
    // Current tile order — mirrors config; used to detect changes that need
    // a grid rebuild.
    tile_order: Vec<String>,
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
    Vpn,
    Valent,
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
    ClickAirplaneMode,

    /// Reactive dark-mode update from EffectScope.
    DarkModeChanged(MatugenMode),

    /// Re-read live subtitles for the expandable tiles.
    RefreshSubtitles,

    /// Enter or exit edit mode (forwarded from the pencil-icon toggle).
    SetEditMode(bool),

    /// Edit-overlay checkbox toggled for a tile — write to config.
    EditTileVisibility(TileId, bool),

    /// Config-driven tile order changed — detach and reattach all grid
    /// children in the new order. Also fires on visibility changes so
    /// hidden gaps are eliminated in normal mode.
    RebuildGrid(Vec<String>),
}

/// Identifies a tile for the edit-overlay toggle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TileId {
    Wifi,
    Bluetooth,
    AudioOut,
    Mic,
    Battery,
    KeepAwake,
    Dnd,
    DarkMode,
    NightLight,
    ColorPicker,
    Disk,
    AirplaneMode,
    Vpn,
    Valent,
}

impl TileId {
    /// Canonical string id used in `tile_order` config.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            TileId::Wifi => "wifi",
            TileId::Bluetooth => "bluetooth",
            TileId::AudioOut => "audio_out",
            TileId::Mic => "mic",
            TileId::Battery => "battery",
            TileId::KeepAwake => "keep_awake",
            TileId::Dnd => "dnd",
            TileId::DarkMode => "dark_mode",
            TileId::NightLight => "night_light",
            TileId::ColorPicker => "color_picker",
            TileId::Disk => "disk",
            TileId::AirplaneMode => "airplane_mode",
            TileId::Vpn => "vpn",
            TileId::Valent => "valent",
        }
    }

    /// Parse a tile-id string back to a `TileId`. Unknown strings → `None`.
    pub(crate) fn from_str(s: &str) -> Option<Self> {
        match s {
            "wifi" => Some(TileId::Wifi),
            "bluetooth" => Some(TileId::Bluetooth),
            "audio_out" => Some(TileId::AudioOut),
            "mic" => Some(TileId::Mic),
            "battery" => Some(TileId::Battery),
            "keep_awake" => Some(TileId::KeepAwake),
            "dnd" => Some(TileId::Dnd),
            "dark_mode" => Some(TileId::DarkMode),
            "night_light" => Some(TileId::NightLight),
            "color_picker" => Some(TileId::ColorPicker),
            "disk" => Some(TileId::Disk),
            "airplane_mode" => Some(TileId::AirplaneMode),
            "vpn" => Some(TileId::Vpn),
            "valent" => Some(TileId::Valent),
            _ => None,
        }
    }

    /// All tile ids in canonical (default) order.
    pub(crate) fn all() -> &'static [TileId] {
        &[
            TileId::Wifi,
            TileId::Bluetooth,
            TileId::AudioOut,
            TileId::Mic,
            TileId::Vpn,
            TileId::Valent,
            TileId::Battery,
            TileId::KeepAwake,
            TileId::Dnd,
            TileId::AirplaneMode,
            TileId::DarkMode,
            TileId::NightLight,
            TileId::ColorPicker,
            TileId::Disk,
        ]
    }

    /// True for tiles that span the full 2-column width.
    pub(crate) fn is_wide(self) -> bool {
        matches!(self, TileId::Dnd | TileId::Valent)
    }
}

/// Compute the ordered sequence of `TileId`s from a `tile_order` config vec.
/// - Ids listed in `tile_order` appear first (in that order); unknown ids are
///   skipped.
/// - Ids not present in `tile_order` are appended at the end in canonical
///   order so a newly-added tile never silently vanishes.
fn ordered_tile_ids(tile_order: &[String]) -> Vec<TileId> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::with_capacity(TileId::all().len());

    for s in tile_order {
        if let Some(id) = TileId::from_str(s)
            && seen.insert(id.as_str())
        {
            result.push(id);
        }
    }
    // Append any canonical ids not yet covered.
    for &id in TileId::all() {
        if seen.insert(id.as_str()) {
            result.push(id);
        }
    }
    result
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
    /// Async valent probe finished — (subtitle, is_connected).
    ValentStateRefreshed(String, bool),
}

// ── Widgets struct (manual — we hold the tile handles) ────────────────────────

pub(crate) struct ControlCenterTilesWidgets {
    // Expandable tiles
    tile_wifi: TileWidget,
    tile_bluetooth: TileWidget,
    tile_audio_out: TileWidget,
    tile_mic: TileWidget,
    tile_vpn: TileWidget,
    tile_valent: TileWidget,
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
    tile_airplane_mode: TileWidget,
    // Edit-mode overlay wrappers (gtk::Overlay containing the tile button +
    // a corner CheckButton). One per tile — wrapped after the tile buttons
    // are attached to the grid.
    overlay_wifi: gtk::Overlay,
    overlay_bluetooth: gtk::Overlay,
    overlay_audio_out: gtk::Overlay,
    overlay_mic: gtk::Overlay,
    overlay_battery: gtk::Overlay,
    overlay_keep_awake: gtk::Overlay,
    overlay_color_picker: gtk::Overlay,
    overlay_dnd: gtk::Overlay,
    overlay_dark_mode: gtk::Overlay,
    overlay_night_light: gtk::Overlay,
    overlay_disk: gtk::Overlay,
    overlay_airplane_mode: gtk::Overlay,
    overlay_vpn: gtk::Overlay,
    overlay_valent: gtk::Overlay,
    // The CheckButton references — needed to update their state in apply_visuals.
    check_wifi: gtk::CheckButton,
    check_bluetooth: gtk::CheckButton,
    check_audio_out: gtk::CheckButton,
    check_mic: gtk::CheckButton,
    check_battery: gtk::CheckButton,
    check_keep_awake: gtk::CheckButton,
    check_color_picker: gtk::CheckButton,
    check_dnd: gtk::CheckButton,
    check_dark_mode: gtk::CheckButton,
    check_night_light: gtk::CheckButton,
    check_disk: gtk::CheckButton,
    check_airplane_mode: gtk::CheckButton,
    check_vpn: gtk::CheckButton,
    check_valent: gtk::CheckButton,
}

impl ControlCenterTilesWidgets {
    /// Return a reference to the overlay wrapper for a given `TileId`.
    fn overlay_for(&self, id: TileId) -> &gtk::Overlay {
        match id {
            TileId::Wifi => &self.overlay_wifi,
            TileId::Bluetooth => &self.overlay_bluetooth,
            TileId::AudioOut => &self.overlay_audio_out,
            TileId::Mic => &self.overlay_mic,
            TileId::Battery => &self.overlay_battery,
            TileId::KeepAwake => &self.overlay_keep_awake,
            TileId::Dnd => &self.overlay_dnd,
            TileId::DarkMode => &self.overlay_dark_mode,
            TileId::NightLight => &self.overlay_night_light,
            TileId::ColorPicker => &self.overlay_color_picker,
            TileId::Disk => &self.overlay_disk,
            TileId::AirplaneMode => &self.overlay_airplane_mode,
            TileId::Vpn => &self.overlay_vpn,
            TileId::Valent => &self.overlay_valent,
        }
    }

    /// Detach all tile overlays from `grid`, then reattach them in the order
    /// described by `tile_order`. Wide tiles (dnd, valent) span 2 columns.
    /// Each non-wide tile occupies one cell; we fill left-to-right, creating
    /// a new row whenever the current column would overflow or a wide tile
    /// needs a fresh row.
    fn rebuild_grid(&self, grid: &gtk::Grid, tile_order: &[String]) {
        use relm4::gtk::prelude::GridExt;

        // Remove every tile overlay currently in the grid.
        for &id in TileId::all() {
            let overlay = self.overlay_for(id);
            grid.remove(overlay);
        }

        // Re-attach in config order, appending unknowns at the end.
        let ids = ordered_tile_ids(tile_order);
        let mut col: i32 = 0;
        let mut row: i32 = 0;
        for id in ids {
            let overlay = self.overlay_for(id);
            if id.is_wide() {
                // Wide tiles always start at column 0 of a fresh row.
                if col != 0 {
                    row += 1;
                }
                grid.attach(overlay, 0, row, 2, 1);
                row += 1;
                col = 0;
            } else {
                grid.attach(overlay, col, row, 1, 1);
                col += 1;
                if col >= 2 {
                    col = 0;
                    row += 1;
                }
            }
        }
    }
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
        let tile_vpn = build_tile("network-vpn-symbolic", "VPN", "…");
        let tile_valent = build_tile("phone-symbolic", "Valent", "…");

        // Mark expandable tiles with a chevron-styled hint (`.expandable`)
        for tw in [&tile_wifi, &tile_bluetooth, &tile_audio_out, &tile_mic, &tile_vpn, &tile_valent] {
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
        let tile_dark_mode = build_tile("weather-clear-night-symbolic", "Dark Mode", "Off");
        let tile_night_light = build_tile("night-light-symbolic", "Twilight", "Off");
        let tile_disk = build_tile("drive-harddisk-symbolic", "Disk", "");
        let tile_battery = build_tile("battery-level-50-symbolic", "Battery", "");
        let tile_airplane_mode = build_tile("airplane-mode-symbolic", "Airplane Mode", "Off");

        // Mark the DND tile as wide
        tile_dnd.button.add_css_class("wide");
        // Battery tile is also expandable
        tile_battery.button.add_css_class("expandable");

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
        {
            let s = sender.clone();
            tile_vpn.button.connect_clicked(move |_| {
                s.output(ControlCenterTilesOutput::ExpandPage(DetailPage::Vpn))
                    .ok();
            });
        }
        {
            let s = sender.clone();
            tile_valent.button.connect_clicked(move |_| {
                s.output(ControlCenterTilesOutput::ExpandPage(DetailPage::Valent))
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
        {
            let s = sender.clone();
            tile_airplane_mode
                .button
                .connect_clicked(move |_| s.input(ControlCenterTilesInput::ClickAirplaneMode));
        }
        // Disk tile is info-only; no click handler needed.

        // ── Edit-mode overlays ───────────────────────────────────────────────
        let (overlay_wifi, check_wifi) =
            build_edit_overlay(&tile_wifi.button, &sender, TileId::Wifi);
        let (overlay_bluetooth, check_bluetooth) =
            build_edit_overlay(&tile_bluetooth.button, &sender, TileId::Bluetooth);
        let (overlay_audio_out, check_audio_out) =
            build_edit_overlay(&tile_audio_out.button, &sender, TileId::AudioOut);
        let (overlay_mic, check_mic) =
            build_edit_overlay(&tile_mic.button, &sender, TileId::Mic);
        let (overlay_battery, check_battery) =
            build_edit_overlay(&tile_battery.button, &sender, TileId::Battery);
        let (overlay_keep_awake, check_keep_awake) =
            build_edit_overlay(&tile_keep_awake.button, &sender, TileId::KeepAwake);
        let (overlay_color_picker, check_color_picker) =
            build_edit_overlay(&tile_color_picker.button, &sender, TileId::ColorPicker);
        let (overlay_dnd, check_dnd) =
            build_edit_overlay(&tile_dnd.button, &sender, TileId::Dnd);
        let (overlay_dark_mode, check_dark_mode) =
            build_edit_overlay(&tile_dark_mode.button, &sender, TileId::DarkMode);
        let (overlay_night_light, check_night_light) =
            build_edit_overlay(&tile_night_light.button, &sender, TileId::NightLight);
        let (overlay_disk, check_disk) =
            build_edit_overlay(&tile_disk.button, &sender, TileId::Disk);
        let (overlay_airplane_mode, check_airplane_mode) =
            build_edit_overlay(&tile_airplane_mode.button, &sender, TileId::AirplaneMode);
        let (overlay_vpn, check_vpn) =
            build_edit_overlay(&tile_vpn.button, &sender, TileId::Vpn);
        let (overlay_valent, check_valent) =
            build_edit_overlay(&tile_valent.button, &sender, TileId::Valent);

        // Mark valent as wide (spans 2 cols). DND is marked wide just above.
        // Both need the CSS class before being attached to the grid.
        tile_valent.button.add_css_class("wide");

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

        // Reactive tile-order / visibility effect — fires whenever tile_order or
        // any per-tile visibility bool changes in config. Sends `RebuildGrid` so
        // the grid re-renders in the new order with hidden tiles removed from the
        // layout in normal mode.
        {
            let s = sender.clone();
            effects.push(move |_| {
                // Touch every visibility bool + tile_order to subscribe to
                // their changes. Each call creates a fresh Subfield so we
                // call config_manager() once per field (matches the pattern
                // used in apply_visuals).
                let cm = config_manager();
                let _ = cm.config().control_center().wifi().get();
                let _ = cm.config().control_center().bluetooth().get();
                let _ = cm.config().control_center().audio_out().get();
                let _ = cm.config().control_center().mic().get();
                let _ = cm.config().control_center().battery().get();
                let _ = cm.config().control_center().keep_awake().get();
                let _ = cm.config().control_center().dnd().get();
                let _ = cm.config().control_center().dark_mode().get();
                let _ = cm.config().control_center().night_light().get();
                let _ = cm.config().control_center().color_picker().get();
                let _ = cm.config().control_center().disk().get();
                let _ = cm.config().control_center().airplane_mode().get();
                let _ = cm.config().control_center().vpn().get();
                let _ = cm.config().control_center().valent().get();
                let order = cm.config().control_center().tile_order().get();
                s.input(ControlCenterTilesInput::RebuildGrid(order));
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
        let power_profile = power_profile_service().power_profiles.active_profile.get();
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

        let model = ControlCenterTilesModel {
            keep_awake,
            dnd,
            dark,
            night_light: false,
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
            watchers_started: false,
            edit_mode: false,
            tile_order: initial_tile_order.clone(),
            _effects: effects,
        };

        let widgets = ControlCenterTilesWidgets {
            tile_wifi,
            tile_bluetooth,
            tile_audio_out,
            tile_mic,
            tile_vpn,
            tile_valent,
            tile_keep_awake,
            tile_color_picker,
            tile_dnd,
            tile_dark_mode,
            tile_night_light,
            tile_disk,
            tile_battery,
            tile_airplane_mode,
            overlay_wifi,
            overlay_bluetooth,
            overlay_audio_out,
            overlay_mic,
            overlay_battery,
            overlay_keep_awake,
            overlay_color_picker,
            overlay_dnd,
            overlay_dark_mode,
            overlay_night_light,
            overlay_disk,
            overlay_airplane_mode,
            overlay_vpn,
            overlay_valent,
            check_wifi,
            check_bluetooth,
            check_audio_out,
            check_mic,
            check_battery,
            check_keep_awake,
            check_color_picker,
            check_dnd,
            check_dark_mode,
            check_night_light,
            check_disk,
            check_airplane_mode,
            check_vpn,
            check_valent,
        };

        // Attach all overlays to the grid in config order.
        widgets.rebuild_grid(&root, &initial_tile_order);

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

                    // Subtitle poller for Wi-Fi / BT / VPN / audio tiles (every 5s)
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

                    // Valent state poller (every 15s — heavier async probe)
                    sender.command(|out, shutdown| async move {
                        use crate::valent;
                        let shutdown_fut = shutdown.wait();
                        tokio::pin!(shutdown_fut);
                        let mut first = true;
                        loop {
                            let delay = if first {
                                STARTUP_DELAY
                            } else {
                                Duration::from_secs(15)
                            };
                            first = false;
                            tokio::select! {
                                () = &mut shutdown_fut => break,
                                _ = tokio::time::sleep(delay) => {}
                            }
                            let report = valent::probe().await;
                            let (subtitle, connected) = valent_state_from_report(&report);
                            let _ = out.send(ControlCenterTilesCommandOutput::ValentStateRefreshed(subtitle, connected));
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
                self.power_profile = power_profile_service().power_profiles.active_profile.get();
                self.disk = read_disk_usage();
                (self.wifi_subtitle, self.wifi_connected) = read_wifi_state();
                (self.bt_subtitle, self.bt_connected) = read_bt_state();
                self.audio_out_subtitle = read_audio_out_subtitle();
                self.mic_subtitle = read_mic_subtitle();
                (self.vpn_subtitle, self.vpn_connected) = read_vpn_state();
                (self.airplane_mode, self.airplane_available) = read_airplane_state();
            }

            ControlCenterTilesInput::Reveal(false) => {}

            ControlCenterTilesInput::RefreshSubtitles => {
                (self.wifi_subtitle, self.wifi_connected) = read_wifi_state();
                (self.bt_subtitle, self.bt_connected) = read_bt_state();
                self.audio_out_subtitle = read_audio_out_subtitle();
                self.mic_subtitle = read_mic_subtitle();
                (self.vpn_subtitle, self.vpn_connected) = read_vpn_state();
                (self.airplane_mode, self.airplane_available) = read_airplane_state();
                self.power_profile = power_profile_service().power_profiles.active_profile.get();
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

            ControlCenterTilesInput::ClickAirplaneMode => {
                // Airplane mode = disable Wi-Fi (toggle wifi enabled state)
                if let Some(wifi) = network_service().wifi.get() {
                    let new_enabled = !wifi.enabled.get();
                    tokio::spawn(async move {
                        let _ = wifi.set_enabled(new_enabled).await;
                    });
                }
            }

            ControlCenterTilesInput::DarkModeChanged(mode) => {
                self.dark = mode;
            }

            ControlCenterTilesInput::SetEditMode(on) => {
                self.edit_mode = on;
            }

            ControlCenterTilesInput::EditTileVisibility(tile, visible) => {
                config_manager().update_config(|c| match tile {
                    TileId::Wifi => c.control_center.wifi = visible,
                    TileId::Bluetooth => c.control_center.bluetooth = visible,
                    TileId::AudioOut => c.control_center.audio_out = visible,
                    TileId::Mic => c.control_center.mic = visible,
                    TileId::Battery => c.control_center.battery = visible,
                    TileId::KeepAwake => c.control_center.keep_awake = visible,
                    TileId::Dnd => c.control_center.dnd = visible,
                    TileId::DarkMode => c.control_center.dark_mode = visible,
                    TileId::NightLight => c.control_center.night_light = visible,
                    TileId::ColorPicker => c.control_center.color_picker = visible,
                    TileId::Disk => c.control_center.disk = visible,
                    TileId::AirplaneMode => c.control_center.airplane_mode = visible,
                    TileId::Vpn => c.control_center.vpn = visible,
                    TileId::Valent => c.control_center.valent = visible,
                });
                // The RebuildGrid effect will fire automatically from the config
                // store change. No explicit rebuild needed here.
            }

            ControlCenterTilesInput::RebuildGrid(order) => {
                // Always rebuild — order or visibility bools may have changed.
                self.tile_order = order.clone();
                widgets.rebuild_grid(root, &order);
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
                self.power_profile = power_profile_service().power_profiles.active_profile.get();
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
        }

        apply_visuals(self, widgets);
    }
}

// ── Visual updater ─────────────────────────────────────────────────────────────

fn apply_visuals(model: &ControlCenterTilesModel, w: &ControlCenterTilesWidgets) {
    // Read per-tile config bools — each call creates a fresh Subfield, so
    // we call config_manager() once per field rather than chaining off `cc`.
    let cfg_wifi =
        config_manager().config().control_center().wifi().get_untracked();
    let cfg_bluetooth =
        config_manager().config().control_center().bluetooth().get_untracked();
    let cfg_audio_out =
        config_manager().config().control_center().audio_out().get_untracked();
    let cfg_mic =
        config_manager().config().control_center().mic().get_untracked();
    let cfg_battery =
        config_manager().config().control_center().battery().get_untracked();
    let cfg_keep_awake =
        config_manager().config().control_center().keep_awake().get_untracked();
    let cfg_dnd =
        config_manager().config().control_center().dnd().get_untracked();
    let cfg_dark_mode =
        config_manager().config().control_center().dark_mode().get_untracked();
    let cfg_night_light =
        config_manager().config().control_center().night_light().get_untracked();
    let cfg_color_picker =
        config_manager().config().control_center().color_picker().get_untracked();
    let cfg_disk =
        config_manager().config().control_center().disk().get_untracked();
    let cfg_airplane_mode =
        config_manager().config().control_center().airplane_mode().get_untracked();
    let cfg_vpn =
        config_manager().config().control_center().vpn().get_untracked();
    let cfg_valent =
        config_manager().config().control_center().valent().get_untracked();

    let edit = model.edit_mode;

    // Helper: a tile's overlay is visible always; the tile button visibility
    // depends on edit mode (all visible) vs normal mode (only enabled tiles).
    // In edit mode, disabled tiles get the `.disabled` class for opacity dimming.
    let update_tile_visibility = |overlay: &gtk::Overlay,
                                   tile_btn: &gtk::Button,
                                   check: &gtk::CheckButton,
                                   enabled: bool| {
        if edit {
            overlay.set_visible(true);
            tile_btn.set_visible(true);
            if enabled {
                tile_btn.remove_css_class("disabled");
            } else {
                tile_btn.add_css_class("disabled");
            }
            check.set_visible(true);
            check.set_active(enabled);
        } else {
            overlay.set_visible(enabled);
            tile_btn.set_visible(true);
            tile_btn.remove_css_class("disabled");
            check.set_visible(false);
        }
    };

    update_tile_visibility(&w.overlay_wifi, &w.tile_wifi.button, &w.check_wifi, cfg_wifi);
    update_tile_visibility(&w.overlay_bluetooth, &w.tile_bluetooth.button, &w.check_bluetooth, cfg_bluetooth);
    update_tile_visibility(&w.overlay_audio_out, &w.tile_audio_out.button, &w.check_audio_out, cfg_audio_out);
    update_tile_visibility(&w.overlay_mic, &w.tile_mic.button, &w.check_mic, cfg_mic);
    update_tile_visibility(&w.overlay_keep_awake, &w.tile_keep_awake.button, &w.check_keep_awake, cfg_keep_awake);
    update_tile_visibility(&w.overlay_color_picker, &w.tile_color_picker.button, &w.check_color_picker, cfg_color_picker);
    update_tile_visibility(&w.overlay_dnd, &w.tile_dnd.button, &w.check_dnd, cfg_dnd);
    update_tile_visibility(&w.overlay_dark_mode, &w.tile_dark_mode.button, &w.check_dark_mode, cfg_dark_mode);
    update_tile_visibility(&w.overlay_night_light, &w.tile_night_light.button, &w.check_night_light, cfg_night_light);
    update_tile_visibility(&w.overlay_disk, &w.tile_disk.button, &w.check_disk, cfg_disk);
    update_tile_visibility(&w.overlay_airplane_mode, &w.tile_airplane_mode.button, &w.check_airplane_mode, cfg_airplane_mode);
    update_tile_visibility(&w.overlay_vpn, &w.tile_vpn.button, &w.check_vpn, cfg_vpn);
    update_tile_visibility(&w.overlay_valent, &w.tile_valent.button, &w.check_valent, cfg_valent);

    // Battery tile: additionally hidden when no battery present (normal mode)
    let bat = &model.battery;
    let battery_effective = cfg_battery && bat.present;
    if edit {
        w.overlay_battery.set_visible(true);
        w.tile_battery.button.set_visible(true);
        if cfg_battery {
            w.tile_battery.button.remove_css_class("disabled");
        } else {
            w.tile_battery.button.add_css_class("disabled");
        }
        w.check_battery.set_visible(true);
        w.check_battery.set_active(cfg_battery);
    } else {
        w.overlay_battery.set_visible(battery_effective);
        w.tile_battery.button.set_visible(true);
        w.tile_battery.button.remove_css_class("disabled");
        w.check_battery.set_visible(false);
    }

    // Wi-Fi — active when connected to a network (has an SSID)
    w.tile_wifi.set_subtitle(&model.wifi_subtitle);
    w.tile_wifi.set_active(model.wifi_connected);

    // Bluetooth — active when a device is connected
    w.tile_bluetooth.set_subtitle(&model.bt_subtitle);
    w.tile_bluetooth.set_active(model.bt_connected);

    // Audio Out
    w.tile_audio_out.set_subtitle(&model.audio_out_subtitle);

    // Mic
    w.tile_mic.set_subtitle(&model.mic_subtitle);

    // VPN — active when VPN is connected
    w.tile_vpn.set_subtitle(&model.vpn_subtitle);
    w.tile_vpn.set_active(model.vpn_connected);

    // Valent — active when a device is reachable + paired
    w.tile_valent.set_subtitle(&model.valent_subtitle);
    w.tile_valent.set_active(model.valent_connected);

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

    // Dark Mode — now a full labeled tile
    let is_dark = model.dark == MatugenMode::Dark;
    w.tile_dark_mode.set_active(is_dark);
    w.tile_dark_mode.set_subtitle(if is_dark { "On" } else { "Off" });
    w.tile_dark_mode.set_icon(match model.dark {
        MatugenMode::Dark => "weather-clear-symbolic",
        MatugenMode::Light => "weather-clear-night-symbolic",
    });

    // Twilight — full labeled tile
    w.tile_night_light.set_active(model.night_light);
    w.tile_night_light.set_subtitle(if model.night_light { "On" } else { "Off" });
    w.tile_night_light.set_icon(if model.night_light {
        "night-light-symbolic"
    } else {
        "night-light-disabled-symbolic"
    });

    // Airplane Mode
    w.tile_airplane_mode.set_active(model.airplane_mode);
    w.tile_airplane_mode.set_subtitle(if model.airplane_mode { "On" } else { "Off" });

    // Disk
    w.tile_disk.set_subtitle(&model.disk.format());

    // Battery content update (when present) — include active power profile
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
        let profile_label = get_power_profile_label(&model.power_profile);
        let subtitle = if status.is_empty() {
            format!("{profile_label} · {}%", bat.percent)
        } else {
            format!("{profile_label} · {}% · {}", bat.percent, status)
        };
        w.tile_battery.set_subtitle(&subtitle);
    }
}

// ── Edit-overlay builder ───────────────────────────────────────────────────────

fn build_edit_overlay(
    tile_btn: &gtk::Button,
    sender: &ComponentSender<ControlCenterTilesModel>,
    tile_id: TileId,
) -> (gtk::Overlay, gtk::CheckButton) {
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(tile_btn));

    let check = gtk::CheckButton::new();
    check.add_css_class("control-center-edit-check");
    check.set_halign(gtk::Align::End);
    check.set_valign(gtk::Align::Start);
    check.set_visible(false);
    check.set_can_focus(false);

    overlay.add_overlay(&check);

    let s = sender.clone();
    check.connect_toggled(move |cb| {
        let active = cb.is_active();
        s.input(ControlCenterTilesInput::EditTileVisibility(tile_id, active));
    });

    (overlay, check)
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

fn read_disk_usage() -> DiskUsage {
    use std::ffi::CString;
    use std::mem::MaybeUninit;

    let path = CString::new("/").unwrap();
    let mut stat: MaybeUninit<libc::statvfs64> = MaybeUninit::uninit();
    let rc = unsafe { libc::statvfs64(path.as_ptr(), stat.as_mut_ptr()) };
    if rc != 0 {
        return DiskUsage::default();
    }
    let s = unsafe { stat.assume_init() };
    let block = s.f_frsize;
    let total = s.f_blocks * block;
    let avail = s.f_bavail * block;
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

// ── Expandable-tile subtitle / state helpers ─────────────────────────────────

/// Returns (subtitle, is_connected). Connected = has an SSID.
fn read_wifi_state() -> (String, bool) {
    let network = network_service();
    if let Some(wifi) = network.wifi.get() {
        if let Some(ssid) = wifi.ssid.get() {
            return (ssid, true);
        }
        if wifi.enabled.get() {
            return ("Not connected".to_string(), false);
        }
        return ("Off".to_string(), false);
    }
    ("Unavailable".to_string(), false)
}

/// Returns (subtitle, is_connected). Connected = at least one device connected.
fn read_bt_state() -> (String, bool) {
    let bt = bluetooth_service();
    if !bt.available.get() {
        return ("Unavailable".to_string(), false);
    }
    if !bt.enabled.get() {
        return ("Off".to_string(), false);
    }
    let devices = bt.devices.get();
    let connected: Vec<_> = devices.iter().filter(|d| d.connected.get()).collect();
    match connected.len() {
        0 => ("On · no devices".to_string(), false),
        1 => (connected[0].alias.get(), true),
        n => (format!("{n} connected"), true),
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

/// Returns (subtitle, is_connected). Connected = a VPN tunnel interface is up.
/// Vendor-neutral: detects OpenVPN (`tun*`), WireGuard (`wg*`, incl. Mullvad's
/// `wg*-mullvad`), NetworkManager VPNs, and PPP-based VPNs by scanning
/// `/sys/class/net` — no VPN-specific CLI. Cheap (a directory read).
fn read_vpn_state() -> (String, bool) {
    match vpn_interface() {
        Some(iface) => (format!("Connected · {iface}"), true),
        None => ("Off".to_string(), false),
    }
}

/// Name of the first VPN tunnel interface present, if any.
fn vpn_interface() -> Option<String> {
    let mut names: Vec<String> = std::fs::read_dir("/sys/class/net")
        .ok()?
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| {
            n.starts_with("tun")
                || n.starts_with("wg")
                || n.starts_with("wireguard")
                || n.starts_with("ppp")
        })
        .collect();
    names.sort();
    names.into_iter().next()
}

/// Compute (subtitle, is_connected) from a ValentReport.
fn valent_state_from_report(report: &crate::valent::ValentReport) -> (String, bool) {
    if !report.daemon_available {
        return ("Unavailable".to_string(), false);
    }
    // Find a connected device: reachable + paired
    let connected: Vec<_> = report.devices.iter()
        .filter(|d| d.reachable && d.paired)
        .collect();
    match connected.len() {
        0 => {
            if report.devices.is_empty() {
                ("No devices".to_string(), false)
            } else {
                ("Not reachable".to_string(), false)
            }
        }
        1 => (connected[0].name.clone(), true),
        n => (format!("{n} connected"), true),
    }
}

fn read_airplane_state() -> (bool, bool) {
    // Airplane mode = Wi-Fi is disabled. Returns (is_airplane_on, is_wifi_available).
    let network = network_service();
    if let Some(wifi) = network.wifi.get() {
        let enabled = wifi.enabled.get();
        // Airplane mode is "on" when Wi-Fi is disabled
        (!enabled, true)
    } else {
        (false, false)
    }
}
