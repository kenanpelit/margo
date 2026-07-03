//! Audio Route — bar pill that switches the default audio **output**.
//!
//! * **Left-click** cycles to the next real output device (Bluetooth / USB /
//!   analog speakers …), wrapping around. HDMI / DisplayPort sinks are skipped
//!   (they're monitors, not something you "route" your audio to) — same idea as
//!   the `audio.hide_hdmi_outputs` filter.
//! * **Right-click** opens the Audio Route frame menu to pick any output
//!   directly (see `menus/menu_widgets/audio_route/`).
//!
//! Optionally (Settings → Widgets → Audio Route → "Switch the microphone too")
//! the default microphone follows the output across the headset boundary: land
//! on a headset and the mic hops to the headset's mic; land on a built-in
//! output and it hops back to a built-in mic. Headset detection uses PipeWire's
//! structured, machine-portable metadata (`device.form_factor` / `icon_name` /
//! `bus`), never a hardcoded device-name list.

use mshell_common::WatcherToken;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{AudioConfigStoreFields, ConfigStoreFields};
use mshell_services::audio_service;
use mshell_utils::audio::{
    is_hdmi_output, spawn_default_output_watcher, spawn_output_devices_watcher,
};
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::collections::HashMap;
use std::sync::Arc;
use wayle_audio::core::device::input::InputDevice;
use wayle_audio::core::device::output::OutputDevice;

/// Name-only headset keywords — the LAST-resort fallback in [`is_headset`],
/// used only when a device advertises no structured PipeWire metadata.
/// Deliberately excludes "hdmi" / "line-out": those are their own devices but
/// are NOT wearable headsets, so they must never read as one.
const DEVICE_HEADSET_KEYS: &[&str] = &["headset", "headphone", "earphone", "earbud", "airpod"];

/// Classify a device as a headset, preferring PipeWire's **structured,
/// machine-portable** metadata over any name guessing (name-only matching was
/// fragile across machines). Signals, most authoritative first:
///
///   1. `device.form_factor` — the PulseAudio/PipeWire standard. BlueZ sets
///      `headset`/`headphone`; ALSA UCM sets `speaker`/`internal`. Decides when
///      present, no guessing.
///   2. `device.icon_name` — `audio-headset*` / `audio-headphones` ⇒ headset;
///      `audio-speakers` / `video-display` / `audio-card-analog` ⇒ explicitly
///      NOT (the negative match stops speakers/HDMI reading as a headset).
///   3. `device.bus == "bluetooth"` ⇒ a Bluetooth audio output (a rare BT
///      *speaker* still carries `device.form_factor = "speaker"`, caught above).
///   4. Last resort — substring match on the node name + description.
///
/// Only used to pick the mic's side + the pill's glyph; it never gates which
/// outputs you can cycle to, so a misclassification can't hide a device.
fn is_headset(props: &HashMap<String, String>, name: &str, desc: &str) -> bool {
    // 1. form_factor — authoritative when present.
    if let Some(ff) = props.get("device.form_factor") {
        return matches!(
            ff.to_ascii_lowercase().as_str(),
            "headset" | "headphone" | "hands-free" | "handset" | "earpiece"
        );
    }
    // 2. icon_name — positive then negative.
    if let Some(icon) = props
        .get("device.icon_name")
        .map(|s| s.to_ascii_lowercase())
    {
        if icon.contains("headset") || icon.contains("headphone") || icon.contains("earbud") {
            return true;
        }
        if icon.contains("speaker")
            || icon.contains("video-display")
            || icon.contains("card-analog")
            || icon.contains("hdmi")
        {
            return false;
        }
    }
    // 3. Bluetooth bus.
    if props
        .get("device.bus")
        .is_some_and(|b| b.eq_ignore_ascii_case("bluetooth"))
    {
        return true;
    }
    // 4. Name/description keyword heuristic (last resort).
    is_headset_name(name, desc)
}

/// Name-only headset heuristic — the final fallback in [`is_headset`]. Pure +
/// unit-testable.
pub(crate) fn is_headset_name(name: &str, desc: &str) -> bool {
    let hay = format!(
        "{} {}",
        name.to_ascii_lowercase(),
        desc.to_ascii_lowercase()
    );
    DEVICE_HEADSET_KEYS.iter().any(|k| hay.contains(k))
}

/// Device-level headset test for an output device (structured props first).
fn out_is_headset(d: &OutputDevice) -> bool {
    is_headset(&d.properties.get(), &d.name.get(), &d.description.get())
}

/// Device-level headset test for an input device (structured props first).
fn in_is_headset(d: &InputDevice) -> bool {
    is_headset(&d.properties.get(), &d.name.get(), &d.description.get())
}

/// The routable output devices — every sink except HDMI/DisplayPort — in a
/// deterministic (name-sorted) order so the left-click cycle is stable across
/// reloads.
fn routable_outputs() -> Vec<Arc<OutputDevice>> {
    let mut outs: Vec<_> = audio_service()
        .output_devices
        .get()
        .into_iter()
        .filter(|d| !is_hdmi_output(d))
        .collect();
    outs.sort_by_key(|d| d.name.get());
    outs
}

/// Index of the next device to cycle to: one past `current` (wrapping), or 0
/// when the current default isn't in the list (e.g. it's an HDMI sink). Pure +
/// unit-testable.
fn next_index(names: &[String], current: Option<&str>) -> usize {
    if names.is_empty() {
        return 0;
    }
    match current.and_then(|c| names.iter().position(|n| n == c)) {
        Some(i) => (i + 1) % names.len(),
        None => 0,
    }
}

pub(crate) struct AudioRouteModel {
    orientation: Orientation,
    root_box: gtk::Box,
    icon: gtk::Image,
    /// Keeps the output-device-list watcher alive for the widget's lifetime.
    _out_devices_token: WatcherToken,
}

#[derive(Debug)]
pub(crate) enum AudioRouteInput {
    /// Repaint (a device or the default output changed).
    Refresh,
    /// Left-click — switch to the next routable output.
    CycleNext,
    /// Right-click — ask the frame to open the Audio Route picker menu.
    OpenMenu,
}

#[derive(Debug)]
pub(crate) enum AudioRouteOutput {
    /// The bar forwards this to the frame to toggle the Audio Route menu.
    OpenMenu,
}

#[derive(Debug)]
pub(crate) enum AudioRouteCommandOutput {
    Refresh,
}

pub(crate) struct AudioRouteInit {
    pub(crate) orientation: Orientation,
}

#[relm4::component(pub)]
impl Component for AudioRouteModel {
    type CommandOutput = AudioRouteCommandOutput;
    type Input = AudioRouteInput;
    type Output = AudioRouteOutput;
    type Init = AudioRouteInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "audio-route-bar-widget",
            set_hexpand: model.orientation == Orientation::Vertical,
            set_vexpand: model.orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,
            set_visible: false,

            gtk::Button {
                set_css_classes: &["ok-button-surface", "ok-bar-widget"],
                set_hexpand: false,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(AudioRouteInput::CycleNext);
                },
                add_controller = gtk::GestureClick::builder()
                    .button(gtk::gdk::BUTTON_SECONDARY)
                    .build() {
                    connect_pressed[sender] => move |_, _, _, _| {
                        sender.input(AudioRouteInput::OpenMenu);
                    },
                },

                #[local_ref]
                icon_widget -> gtk::Image {
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("audio-volume-high-symbolic"),
                },
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let icon_widget = gtk::Image::new();

        // Repaint whenever the default output flips or a device appears/drops
        // (e.g. the Bluetooth/USB headset connecting) so the pill shows up and
        // tracks the live output without a manual refresh.
        spawn_default_output_watcher(&sender, None, || AudioRouteCommandOutput::Refresh);
        let mut out_devices_token = WatcherToken::new();
        spawn_output_devices_watcher(&sender, out_devices_token.reset(), || {
            AudioRouteCommandOutput::Refresh
        });

        let model = AudioRouteModel {
            orientation: params.orientation,
            root_box: root.clone(),
            icon: icon_widget.clone(),
            _out_devices_token: out_devices_token,
        };
        let widgets = view_output!();
        model.refresh();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            AudioRouteInput::Refresh => self.refresh(),
            AudioRouteInput::CycleNext => self.cycle_next(),
            AudioRouteInput::OpenMenu => {
                let _ = sender.output(AudioRouteOutput::OpenMenu);
            }
        }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            AudioRouteCommandOutput::Refresh => sender.input(AudioRouteInput::Refresh),
        }
    }
}

impl AudioRouteModel {
    /// Repaint: show the pill only when there are ≥2 routable outputs (something
    /// to cycle between), and reflect the current output in the glyph + tooltip.
    fn refresh(&self) {
        let outputs = routable_outputs();
        let visible = outputs.len() >= 2;
        self.root_box.set_visible(visible);
        if !visible {
            return;
        }

        let default_out = audio_service().default_output.get();
        let on_headset = default_out.as_ref().is_some_and(|d| out_is_headset(d));
        if on_headset {
            self.icon.set_icon_name(Some("audio-headset-symbolic"));
        } else {
            self.icon.set_icon_name(Some("audio-volume-high-symbolic"));
        }

        let desc = default_out
            .as_ref()
            .map(|d| d.description.get())
            .unwrap_or_default();
        self.root_box.set_tooltip_text(Some(&format!(
            "Output: {desc}\nClick: next output · Right-click: pick"
        )));
    }

    /// Switch the default output to the next routable device (wrapping). If
    /// "switch the microphone too" is on, the mic follows across the headset
    /// boundary.
    fn cycle_next(&self) {
        let outputs = routable_outputs();
        if outputs.len() < 2 {
            return;
        }
        let names: Vec<String> = outputs.iter().map(|d| d.name.get()).collect();
        let current = audio_service().default_output.get().map(|d| d.name.get());
        let idx = next_index(&names, current.as_deref());
        let target = outputs[idx].clone();
        let to_headset = out_is_headset(&target);

        let dev = target.clone();
        tokio::spawn(async move {
            let _ = dev.set_as_default().await;
        });

        let switch_mic = config_manager()
            .config()
            .audio()
            .route_switch_microphone()
            .get_untracked();
        if switch_mic {
            follow_mic(to_headset);
        }
    }
}

/// Move the default microphone to match the output's headset side — but only
/// when it isn't already there, and only if a suitable input exists. Landing on
/// a headset moves the mic to the headset mic; landing on a built-in output
/// moves it back to a built-in mic.
fn follow_mic(to_headset: bool) {
    let audio = audio_service();
    let already = audio
        .default_input
        .get()
        .as_ref()
        .is_some_and(|d| in_is_headset(d));
    if already == to_headset {
        return;
    }
    let inputs = audio.input_devices.get();
    let target = if to_headset {
        inputs.iter().find(|d| in_is_headset(d)).cloned()
    } else {
        inputs.iter().find(|d| !in_is_headset(d)).cloned()
    };
    if let Some(dev) = target {
        tokio::spawn(async move {
            let _ = dev.set_as_default().await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn headset_form_factor_is_authoritative() {
        // A BlueZ headset advertises device.form_factor=headset — decisive.
        let bt = props(&[
            ("device.form_factor", "headset"),
            ("device.bus", "bluetooth"),
        ]);
        assert!(is_headset(&bt, "bluez_output.F4_9D", "SLP4"));

        // A Bluetooth *speaker* carries form_factor=speaker → NOT a headset.
        let bt_speaker = props(&[
            ("device.form_factor", "speaker"),
            ("device.bus", "bluetooth"),
        ]);
        assert!(!is_headset(
            &bt_speaker,
            "bluez_output.SOUNDCORE",
            "Speaker"
        ));
    }

    #[test]
    fn headset_icon_name_positive_and_negative() {
        assert!(is_headset(
            &props(&[("device.icon_name", "audio-headset")]),
            "alsa_output.usb-Headset",
            "USB Headset"
        ));
        // HDMI / speakers / analog card must NOT read as headsets.
        assert!(!is_headset(
            &props(&[("device.icon_name", "video-display")]),
            "alsa_output.pci.HiFi__HDMI1__sink",
            "HDMI 1"
        ));
        assert!(!is_headset(
            &props(&[("device.icon_name", "audio-speakers")]),
            "alsa_output.pci.HiFi__Speaker__sink",
            "Speaker"
        ));
        assert!(!is_headset(
            &props(&[
                ("device.icon_name", "audio-card-analog"),
                ("device.bus", "usb")
            ]),
            "alsa_output.usb-Logitech_Z205",
            "Logitech Z205"
        ));
    }

    #[test]
    fn headset_bluetooth_bus_without_form_factor() {
        assert!(is_headset(
            &props(&[("device.bus", "bluetooth")]),
            "bluez_output.xx",
            ""
        ));
    }

    #[test]
    fn headset_name_fallback_when_no_metadata() {
        assert!(is_headset(
            &props(&[]),
            "alsa_output.usb-X",
            "Gaming Headset"
        ));
        assert!(!is_headset(
            &props(&[]),
            "alsa_output.usb-Z205",
            "Logitech Z205"
        ));
        assert!(is_headset_name("x", "My Headphones"));
        assert!(!is_headset_name("alsa_output.hdmi", "HDMI Audio"));
    }

    #[test]
    fn cycle_next_index_wraps() {
        let names = vec![
            "alsa_output.pci.Speaker".to_string(),
            "alsa_output.usb-Logitech".to_string(),
            "bluez_output.SLP4".to_string(),
        ];
        // Middle → next.
        assert_eq!(next_index(&names, Some("alsa_output.usb-Logitech")), 2);
        // Last → wraps to first.
        assert_eq!(next_index(&names, Some("bluez_output.SLP4")), 0);
        // Current not in list (e.g. on HDMI) → starts at first.
        assert_eq!(next_index(&names, Some("alsa_output.pci.HDMI1")), 0);
        // No current → first.
        assert_eq!(next_index(&names, None), 0);
        // Empty list → 0 (guarded, no modulo-by-zero).
        assert_eq!(next_index(&[], Some("x")), 0);
    }
}
