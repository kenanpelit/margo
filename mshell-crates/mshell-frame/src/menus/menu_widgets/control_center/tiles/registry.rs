//! The Control Center tile registry: one [`TileSpec`] row per tile
//! (icon, title, initial subtitle, what a left/right click does), in
//! canonical grid order. The config side of each tile (id string,
//! visibility flag, Settings label) lives in
//! [`mshell_config::control_center_tiles`], in the same order.
//!
//! Adding a tile: a `bool` field on `ControlCenterConfig` + its row in
//! `CONTROL_CENTER_TILES`, a `TileId` variant + its row in [`TILES`] (same
//! position), then its live state in the model, `visuals.rs` and (if it
//! probes something) `pollers.rs` / `tile_probes.rs`.

use super::{ControlCenterTilesInput, DetailPage};
use mshell_config::control_center_tiles::{CONTROL_CENTER_TILES, CcTile};
use std::collections::HashSet;

/// Identifies a tile. Declared in canonical order: `TileId as usize` is
/// the tile's index in [`TILES`] and `CONTROL_CENTER_TILES`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TileId {
    Wifi,
    Bluetooth,
    AudioOut,
    Mic,
    Vpn,
    Valent,
    Battery,
    KeepAwake,
    Dnd,
    AirplaneMode,
    DarkMode,
    NightLight,
    ColorPicker,
    Disk,
    Ufw,
    Podman,
    Mirror,
    MicDenoise,
    Hotspot,
    ThinkpadLed,
    CustomShortcut,
    InternalKeyboard,
}

/// What a primary (left) click on a tile does.
pub(crate) enum PrimaryClick {
    /// Open this detail sub-page (the tile is built with a chevron).
    Expand(DetailPage),
    /// Send this input to the tiles component.
    Input(fn() -> ControlCenterTilesInput),
    /// Info-only tile, no click handler.
    None,
}

/// Static description of one tile.
pub(crate) struct TileSpec {
    pub(crate) id: TileId,
    pub(crate) icon: &'static str,
    pub(crate) title: &'static str,
    /// Subtitle shown until the first state update.
    pub(crate) subtitle: &'static str,
    pub(crate) click: PrimaryClick,
    /// Secondary (right) click quick toggle, if any.
    pub(crate) secondary: Option<fn() -> ControlCenterTilesInput>,
}

const fn spec(
    id: TileId,
    icon: &'static str,
    title: &'static str,
    subtitle: &'static str,
    click: PrimaryClick,
    secondary: Option<fn() -> ControlCenterTilesInput>,
) -> TileSpec {
    TileSpec {
        id,
        icon,
        title,
        subtitle,
        click,
        secondary,
    }
}

use ControlCenterTilesInput as In;
use PrimaryClick::{Expand, Input};

/// Every tile in canonical (default) order.
pub(crate) const TILES: &[TileSpec] = &[
    spec(
        TileId::Wifi,
        "network-wireless-symbolic",
        "Wi-Fi",
        "…",
        Expand(DetailPage::Wifi),
        None,
    ),
    // Bluetooth / Twilight / Keep Awake / Firewall / Podman: left-click
    // opens the detail page, right-click quick-toggles.
    spec(
        TileId::Bluetooth,
        "bluetooth-active-symbolic",
        "Bluetooth",
        "…",
        Expand(DetailPage::Bluetooth),
        Some(|| In::ToggleBluetooth),
    ),
    spec(
        TileId::AudioOut,
        "audio-speakers-symbolic",
        "Audio Out",
        "…",
        Expand(DetailPage::AudioOut),
        None,
    ),
    spec(
        TileId::Mic,
        "audio-input-microphone-symbolic",
        "Mic",
        "…",
        Expand(DetailPage::Mic),
        None,
    ),
    spec(
        TileId::Vpn,
        "network-vpn-symbolic",
        "VPN",
        "…",
        Expand(DetailPage::Vpn),
        None,
    ),
    spec(
        TileId::Valent,
        "phone-symbolic",
        "Valent",
        "…",
        Expand(DetailPage::Valent),
        None,
    ),
    spec(
        TileId::Battery,
        "battery-level-50-symbolic",
        "Battery",
        "",
        Expand(DetailPage::Battery),
        None,
    ),
    spec(
        TileId::KeepAwake,
        "eye-symbolic",
        "Keep Awake",
        "Off",
        Expand(DetailPage::KeepAwake),
        Some(|| In::ClickKeepAwake),
    ),
    spec(
        TileId::Dnd,
        "notification-symbolic",
        "Do Not Disturb",
        "All notifications",
        Input(|| In::ClickDnd),
        None,
    ),
    spec(
        TileId::AirplaneMode,
        "airplane-mode-symbolic",
        "Airplane Mode",
        "Off",
        Input(|| In::ClickAirplaneMode),
        None,
    ),
    spec(
        TileId::DarkMode,
        "weather-clear-night-symbolic",
        "Dark Mode",
        "Off",
        Input(|| In::ClickDarkMode),
        None,
    ),
    // Twilight expands to the full detail page (mode selector,
    // temperature slider, presets); clicking it never toggles.
    spec(
        TileId::NightLight,
        "night-light-symbolic",
        "Twilight",
        "…",
        Expand(DetailPage::Twilight),
        Some(|| In::ToggleTwilight),
    ),
    spec(
        TileId::ColorPicker,
        "color-select-symbolic",
        "Color Picker",
        "Pick a colour",
        Input(|| In::ClickColorPicker),
        None,
    ),
    spec(
        TileId::Disk,
        "drive-harddisk-symbolic",
        "Disk",
        "",
        PrimaryClick::None,
        None,
    ),
    spec(
        TileId::Ufw,
        "shield-check-symbolic",
        "Firewall",
        "…",
        Expand(DetailPage::Ufw),
        Some(|| In::ToggleUfw),
    ),
    spec(
        TileId::Podman,
        "package-symbolic",
        "Podman",
        "…",
        Expand(DetailPage::Podman),
        Some(|| In::StopPodman),
    ),
    spec(
        TileId::Mirror,
        "video-display-symbolic",
        "Mirror",
        "Off",
        Input(|| In::ClickMirror),
        None,
    ),
    spec(
        TileId::MicDenoise,
        "microphone-sensitivity-high-symbolic",
        "Noise Suppression",
        "Off",
        Input(|| In::ToggleDenoise),
        None,
    ),
    spec(
        TileId::Hotspot,
        "network-wireless-hotspot-symbolic",
        "Hotspot",
        "Off",
        Input(|| In::ToggleHotspot),
        None,
    ),
    // The lid logo's red dot; shown only on ThinkPads that have it.
    spec(
        TileId::ThinkpadLed,
        "media-record-symbolic",
        "Logo LED",
        "…",
        Input(|| In::ToggleLed),
        None,
    ),
    // The user's own tile: label, icon and command from config.
    spec(
        TileId::CustomShortcut,
        "utilities-terminal-symbolic",
        "Custom",
        "…",
        Input(|| In::ClickCustom),
        None,
    ),
    // Click wiring + live state: `grid/internal_keyboard.rs`.
    spec(
        TileId::InternalKeyboard,
        "input-keyboard-symbolic",
        "Internal Keyboard",
        "On",
        PrimaryClick::None,
        None,
    ),
];

impl TileId {
    /// The tile's config row (id string, visibility accessors).
    pub(crate) fn config(self) -> &'static CcTile {
        &CONTROL_CENTER_TILES[self as usize]
    }

    /// Canonical string id used in `tile_order` config.
    pub(crate) fn as_str(self) -> &'static str {
        self.config().id
    }

    /// Parse a tile-id string back to a `TileId`. Unknown strings → `None`.
    pub(crate) fn from_str(s: &str) -> Option<Self> {
        Self::all().find(|id| id.as_str() == s)
    }

    /// All tile ids in canonical (default) order.
    pub(crate) fn all() -> impl ExactSizeIterator<Item = TileId> {
        TILES.iter().map(|t| t.id)
    }
}

/// Compute the ordered sequence of `TileId`s from a `tile_order` config vec.
/// - Ids listed in `tile_order` appear first (in that order); unknown ids are
///   skipped.
/// - Ids not present in `tile_order` are appended at the end in canonical
///   order so a newly-added tile never silently vanishes.
pub(super) fn ordered_tile_ids(tile_order: &[String]) -> Vec<TileId> {
    let mut seen = HashSet::new();
    let mut result = Vec::with_capacity(TILES.len());

    for s in tile_order {
        if let Some(id) = TileId::from_str(s)
            && seen.insert(id.as_str())
        {
            result.push(id);
        }
    }
    // Append any canonical ids not yet covered.
    for id in TileId::all() {
        if seen.insert(id.as_str()) {
            result.push(id);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `TileId as usize` indexes both tables: each row sits at its id's
    /// position and the frame and config registries list the same tiles.
    #[test]
    fn registry_rows_line_up_with_their_ids_and_the_config() {
        assert_eq!(TILES.len(), CONTROL_CENTER_TILES.len());
        for (i, t) in TILES.iter().enumerate() {
            assert_eq!(t.id as usize, i, "{:?} is out of place", t.id);
        }
        let keys: Vec<&str> = TileId::all().map(TileId::as_str).collect();
        assert_eq!(
            keys,
            [
                "wifi",
                "bluetooth",
                "audio_out",
                "mic",
                "vpn",
                "valent",
                "battery",
                "keep_awake",
                "dnd",
                "airplane_mode",
                "dark_mode",
                "night_light",
                "color_picker",
                "disk",
                "ufw",
                "podman",
                "mirror",
                "mic_denoise",
                "hotspot",
                "thinkpad_led",
                "custom_shortcut",
                "internal_keyboard",
            ]
        );
    }

    #[test]
    fn every_id_round_trips() {
        for id in TileId::all() {
            assert_eq!(TileId::from_str(id.as_str()), Some(id));
        }
        assert_eq!(TileId::from_str("bogus"), None);
    }

    #[test]
    fn mirror_tile_id_round_trips() {
        assert_eq!(TileId::from_str("mirror"), Some(TileId::Mirror));
        assert_eq!(TileId::Mirror.as_str(), "mirror");
    }

    #[test]
    fn ordered_tile_ids_listed_first_then_canonical_remainder() {
        let order = vec!["bluetooth".to_string(), "wifi".to_string()];
        let ids = ordered_tile_ids(&order);
        assert_eq!(ids[0], TileId::Bluetooth);
        assert_eq!(ids[1], TileId::Wifi);
        // Every canonical id present exactly once — none lost, none duplicated.
        assert_eq!(ids.len(), TileId::all().len());
    }

    #[test]
    fn ordered_tile_ids_ignores_unknown_and_dedupes() {
        let order = vec!["wifi".to_string(), "bogus".to_string(), "wifi".to_string()];
        let ids = ordered_tile_ids(&order);
        assert_eq!(ids[0], TileId::Wifi);
        assert_eq!(ids.len(), TileId::all().len());
        assert_eq!(ids.iter().filter(|&&i| i == TileId::Wifi).count(), 1);
    }
}
