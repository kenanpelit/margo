//! Bluetooth audio handoff for the output picker (audio-switcher port).
//!
//! A paired Bluetooth headset that is currently disconnected has no
//! PipeWire sink at all, so it never shows up in the output list and
//! "switch to my headphones" meant a detour through the Bluetooth menu.
//! The audio-out list now also offers those devices; picking one
//! connects it (adapter powered on if needed) and makes its sink the
//! default output once PipeWire creates it
//! (`mshell_services::bluetooth::connect_for_output`).
//!
//! This module only decides *which* devices to offer — pure and tested.

use std::sync::Arc;
use wayle_bluetooth::core::device::Device;

/// The bits of a bluez device the decision needs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BtAudioDevice {
    pub mac: String,
    pub alias: String,
    pub icon: Option<String>,
    pub uuids: Vec<String>,
    pub paired: bool,
    pub connected: bool,
}

/// Bluetooth service UUIDs (16-bit, lower-case) of profiles that carry
/// audio *to* the device: A2DP sink, HFP hands-free, HSP headset and LE
/// Audio (ASCS).
const AUDIO_SINK_UUIDS: [&str; 4] = ["0000110b", "0000111e", "00001108", "0000184e"];

/// Can this device play audio? bluez's icon (`audio-headset`,
/// `audio-headphones`, `audio-card`, …) or one of the sink profiles.
pub fn is_audio_sink(icon: Option<&str>, uuids: &[String]) -> bool {
    if icon.is_some_and(|i| i.starts_with("audio-")) {
        return true;
    }
    uuids.iter().any(|u| {
        let u = u.to_ascii_lowercase();
        AUDIO_SINK_UUIDS.iter().any(|p| u.starts_with(p))
    })
}

/// A PipeWire sink for this MAC already exists (bluez nodes embed the
/// address underscored: `bluez_output.00_7F_1D_22_4C_18.1`).
pub fn sink_present(mac: &str, sink_names: &[String]) -> bool {
    let needle = mac.trim().to_ascii_uppercase().replace([':', '-'], "_");
    !needle.is_empty()
        && sink_names
            .iter()
            .any(|n| n.to_ascii_uppercase().contains(&needle))
}

/// Paired, disconnected, audio-capable devices with no sink yet — the
/// rows to offer below the connected outputs. Sorted by name so the list
/// doesn't jump around as bluez updates properties.
pub fn handoff_candidates(devices: &[BtAudioDevice], sink_names: &[String]) -> Vec<BtAudioDevice> {
    let mut out: Vec<BtAudioDevice> = devices
        .iter()
        .filter(|d| d.paired && !d.connected)
        .filter(|d| is_audio_sink(d.icon.as_deref(), &d.uuids))
        .filter(|d| !sink_present(&d.mac, sink_names))
        .cloned()
        .collect();
    out.sort_by(|a, b| {
        a.alias
            .to_lowercase()
            .cmp(&b.alias.to_lowercase())
            .then_with(|| a.mac.cmp(&b.mac))
    });
    out
}

/// Snapshot a wayle bluez device.
pub fn from_device(d: &Arc<Device>) -> BtAudioDevice {
    let alias = d.alias.get();
    BtAudioDevice {
        mac: d.address.get(),
        alias: if alias.is_empty() {
            d.name.get().unwrap_or_default()
        } else {
            alias
        },
        icon: d.icon.get(),
        uuids: d.uuids.get().unwrap_or_default(),
        paired: d.paired.get(),
        connected: d.connected.get(),
    }
}

/// Current candidates from the live services (empty without Bluetooth).
pub fn current_candidates(sink_names: &[String]) -> Vec<BtAudioDevice> {
    let devices: Vec<BtAudioDevice> = mshell_services::bluetooth_service()
        .map(|bt| bt.devices.get())
        .unwrap_or_default()
        .iter()
        .map(from_device)
        .collect();
    handoff_candidates(&devices, sink_names)
}

#[cfg(test)]
mod tests {
    use super::*;

    // UUIDs as bluez reports them for the owner's SL4 headset
    // (`bluetoothctl info`), plus a mouse (HID only).
    fn headset(mac: &str, alias: &str, connected: bool) -> BtAudioDevice {
        BtAudioDevice {
            mac: mac.into(),
            alias: alias.into(),
            icon: Some("audio-headset".into()),
            uuids: vec![
                "00001101-0000-1000-8000-00805f9b34fb".into(),
                "0000110b-0000-1000-8000-00805f9b34fb".into(),
                "0000110e-0000-1000-8000-00805f9b34fb".into(),
            ],
            paired: true,
            connected,
        }
    }

    #[test]
    fn audio_capability() {
        assert!(is_audio_sink(Some("audio-headphones"), &[]));
        assert!(is_audio_sink(
            None,
            &["0000110B-0000-1000-8000-00805F9B34FB".to_string()]
        ));
        assert!(is_audio_sink(
            None,
            &["0000111e-0000-1000-8000-00805f9b34fb".to_string()]
        ));
        // A mouse: HID only.
        assert!(!is_audio_sink(
            Some("input-mouse"),
            &["00001124-0000-1000-8000-00805f9b34fb".to_string()]
        ));
        assert!(!is_audio_sink(None, &[]));
    }

    #[test]
    fn sink_match_uses_underscored_mac() {
        // Real sink name from this machine's `pactl list short sinks`.
        let sinks = vec!["bluez_output.00_7F_1D_22_4C_18.1".to_string()];
        assert!(sink_present("00:7f:1d:22:4c:18", &sinks));
        assert!(!sink_present("00:7F:1D:22:4C:19", &sinks));
        assert!(!sink_present("", &sinks));
    }

    #[test]
    fn offers_only_paired_disconnected_audio_without_sink() {
        let sinks = vec!["bluez_output.00_7F_1D_22_4C_18.1".to_string()];
        let mut mouse = headset("AA:AA:AA:AA:AA:01", "Mouse", false);
        mouse.icon = Some("input-mouse".into());
        mouse.uuids = vec!["00001124-0000-1000-8000-00805f9b34fb".into()];
        let mut unpaired = headset("AA:AA:AA:AA:AA:02", "Stranger", false);
        unpaired.paired = false;
        let devices = vec![
            headset("00:7F:1D:22:4C:18", "SL4", false), // sink still present
            headset("AA:AA:AA:AA:AA:03", "slp5", false),
            headset("AA:AA:AA:AA:AA:04", "SLP4", false),
            headset("AA:AA:AA:AA:AA:05", "Connected", true),
            mouse,
            unpaired,
        ];
        let c = handoff_candidates(&devices, &sinks);
        let names: Vec<&str> = c.iter().map(|d| d.alias.as_str()).collect();
        assert_eq!(names, vec!["SLP4", "slp5"]);
        assert!(handoff_candidates(&[], &sinks).is_empty());
    }
}
