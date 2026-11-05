//! The Control Center tile registry: one row per tile, shared by the grid
//! (mshell-frame `control_center/tiles`) and the Settings tile list.
//!
//! A tile's config id is its `ControlCenterConfig` field name (also the
//! string stored in `tile_order` / `wide_tiles`). Adding a tile means a new
//! `bool` field on `ControlCenterConfig` plus one row here; the grid and
//! Settings read visibility through these accessors instead of each
//! matching every field by hand.

use crate::config_manager::config_manager;
use crate::schema::config::{
    ConfigStoreFields, ControlCenterConfig, ControlCenterConfigStoreFields,
};
use reactive_graph::prelude::{Get, GetUntracked};

/// One Control Center tile as far as the config is concerned.
pub struct CcTile {
    /// Config id (`tile_order` / `wide_tiles` string, config field name).
    pub id: &'static str,
    /// Row name in Settings → Control Center → Tiles.
    pub label: &'static str,
    /// Current visibility, read without subscribing.
    pub visible: fn() -> bool,
    /// Current visibility, read inside a reactive effect (subscribes to it).
    pub track_visible: fn() -> bool,
    /// Write the visibility into a config being updated.
    pub set_visible: fn(&mut ControlCenterConfig, bool),
}

macro_rules! cc_tiles {
    ($($field:ident => $label:literal),* $(,)?) => {
        /// Every tile in canonical (default grid) order.
        pub const CONTROL_CENTER_TILES: &[CcTile] = &[$(CcTile {
            id: stringify!($field),
            label: $label,
            visible: || config_manager().config().control_center().$field().get_untracked(),
            track_visible: || config_manager().config().control_center().$field().get(),
            set_visible: |c, visible| c.$field = visible,
        }),*];
    };
}

cc_tiles! {
    wifi => "Wi-Fi",
    bluetooth => "Bluetooth",
    audio_out => "Audio Out",
    mic => "Microphone",
    vpn => "VPN",
    valent => "Valent",
    battery => "Battery",
    keep_awake => "Keep Awake",
    dnd => "Do Not Disturb",
    airplane_mode => "Airplane Mode",
    dark_mode => "Dark Mode",
    night_light => "Twilight (Night Light)",
    color_picker => "Color Picker",
    disk => "Disk",
    ufw => "Firewall (UFW)",
    podman => "Podman",
    mirror => "Mirror Display",
    mic_denoise => "Noise Suppression",
    hotspot => "Hotspot",
    thinkpad_led => "ThinkPad Logo LED",
    custom_shortcut => "Custom Shortcut",
    internal_keyboard => "Internal Keyboard",
}

/// The registry row for a config id; `None` for an unknown id (e.g. a
/// tile removed in a later release but still listed in `tile_order`).
pub fn tile(id: &str) -> Option<&'static CcTile> {
    CONTROL_CENTER_TILES.iter().find(|t| t.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `bool` field of `ControlCenterConfig` except the `show_*`
    /// section switches is a tile visibility flag: the registry must list
    /// each exactly once and nothing else, so a new tile can't be missing
    /// from the grid or the Settings list.
    #[test]
    fn registry_covers_every_visibility_field() {
        let value = serde_json::to_value(ControlCenterConfig::default()).unwrap();
        let mut fields: Vec<&str> = value
            .as_object()
            .unwrap()
            .iter()
            .filter(|(k, v)| v.is_boolean() && !k.starts_with("show_"))
            .map(|(k, _)| k.as_str())
            .collect();
        fields.sort_unstable();
        let mut ids: Vec<&str> = CONTROL_CENTER_TILES.iter().map(|t| t.id).collect();
        ids.sort_unstable();
        assert_eq!(ids, fields);
    }

    #[test]
    fn set_visible_writes_its_own_field() {
        for t in CONTROL_CENTER_TILES {
            let mut c = ControlCenterConfig::default();
            (t.set_visible)(&mut c, false);
            let value = serde_json::to_value(&c).unwrap();
            assert_eq!(value[t.id], serde_json::Value::Bool(false), "{}", t.id);
            (t.set_visible)(&mut c, true);
            let value = serde_json::to_value(&c).unwrap();
            assert_eq!(value[t.id], serde_json::Value::Bool(true), "{}", t.id);
        }
        assert!(tile("mic_denoise").is_some());
        assert!(tile("bogus").is_none());
    }
}
