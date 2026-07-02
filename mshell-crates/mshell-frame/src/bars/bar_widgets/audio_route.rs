//! Audio Route — bar pill that flips the whole default audio path
//! (default input/mic **and** default output/speaker together) between
//! the built-in device port and a headset/external port, via
//! `wayle_audio`'s `set_port`. Native port of the DMS `Audio Port
//! Switcher` plugin, generalized to route both sides at once.
//!
//! One click: if the audio is currently on the headset → send everything
//! back to built-in; otherwise route everything to the headset. The glyph
//! + label reflect the current route.
//!
//! Two mechanisms, picked automatically per machine:
//!   * **Device-level** (the common case): a headset that shows up as its
//!     own PipeWire device — Bluetooth, USB, or a separate analog card — is
//!     switched by moving the *default* sink (and, unless disabled in
//!     Settings → Widgets → Audio Route, the default source) between it and
//!     the built-in device. "Built-in" is whatever non-headset device you
//!     were last on (so a click returns you to your USB speakers, not a
//!     keyword guess).
//!   * **Port-level** (combo-jack laptops): when the internal codec exposes
//!     the speaker and headphone as two *ports* on one device, the active
//!     port is flipped via `set_port` instead.
//!
//! The pill shows whenever either mechanism has somewhere to switch, and
//! hides on a machine with a single output and no headset — where routing
//! is meaningless.

use mshell_common::WatcherToken;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{AudioConfigStoreFields, ConfigStoreFields};
use mshell_services::audio_service;
use mshell_utils::audio::{
    spawn_default_input_watcher, spawn_default_output_watcher, spawn_input_device_ports_watcher,
    spawn_input_devices_watcher, spawn_output_device_ports_watcher, spawn_output_devices_watcher,
};
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::collections::HashMap;
use std::sync::Arc;
use wayle_audio::core::device::input::InputDevice;
use wayle_audio::core::device::output::OutputDevice;
use wayle_audio::types::device::DevicePort;

/// Substrings (lower-cased name + description) marking a headset/external
/// route, and the built-in route. First match per device wins.
const HEADSET_KEYS: &[&str] = &[
    "headset",
    "headphone",
    "bluez",
    "bluetooth",
    "hdmi",
    "external",
    "line-out",
    "lineout",
];
const INTERNAL_KEYS: &[&str] = &[
    "internal", "speaker", "builtin", "built-in", "front", "onboard",
];

/// Name-only headset keywords — the LAST-resort fallback, used only when a
/// device advertises no structured PipeWire metadata (see [`is_headset`]).
/// Deliberately excludes "hdmi" / "line-out": those are their own devices but
/// are NOT wearable headsets, so they must never be picked as the switch
/// target.
const DEVICE_HEADSET_KEYS: &[&str] = &["headset", "headphone", "earphone", "earbud", "airpod"];

/// Classify a device as a headset we can route the whole audio path to,
/// preferring PipeWire's **structured, machine-portable** metadata over any
/// name guessing (that's what made the old keyword-only match fragile across
/// machines). Signals, most authoritative first:
///
///   1. `device.form_factor` — the PulseAudio/PipeWire standard. BlueZ sets
///      `headset`/`headphone`; ALSA UCM sets `speaker`/`internal`. When the
///      device advertises it, it decides — no guessing.
///   2. `device.icon_name` — `audio-headset*` / `audio-headphones` ⇒ headset;
///      `audio-speakers` / `video-display` / `audio-card-analog` ⇒ explicitly
///      NOT (this negative match is what stops HDMI/speakers being mistaken
///      for a headset).
///   3. `device.bus == "bluetooth"` ⇒ a Bluetooth audio output — treat as a
///      headset (the overwhelmingly common case; a rare BT *speaker* still
///      carries `device.form_factor = "speaker"` and is caught by step 1).
///   4. Last resort — a substring match on the node name + description, for
///      exotic devices that expose none of the above.
///
/// Fails safe: anything it can't confidently call a headset is treated as a
/// built-in, so the pill never silently routes audio to the wrong device.
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

/// Name-only headset heuristic — the final fallback in [`is_headset`] for
/// devices that expose no structured metadata. Pure + unit-testable.
pub(crate) fn is_headset_name(name: &str, desc: &str) -> bool {
    let hay = format!(
        "{} {}",
        name.to_ascii_lowercase(),
        desc.to_ascii_lowercase()
    );
    DEVICE_HEADSET_KEYS.iter().any(|k| hay.contains(k))
}

/// A resolved two-way route for one device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Route {
    pub(crate) internal: String,
    pub(crate) headset: String,
    /// Active port is currently the headset port.
    pub(crate) on_headset: bool,
    /// There is somewhere to switch (headset port available, or we are
    /// already on it and can switch back).
    pub(crate) switchable: bool,
}

/// Resolve a device's (built-in, headset) port pair + current state from
/// its full port list and active port. Pure + unit-testable. `None` when
/// the device has no distinguishable two-way route.
pub(crate) fn resolve_route(ports: &[DevicePort], active: Option<&str>) -> Option<Route> {
    let mut internal: Option<&str> = None;
    let mut headset: Option<&str> = None;
    for p in ports {
        let hay = format!(
            "{} {}",
            p.name.to_ascii_lowercase(),
            p.description.to_ascii_lowercase()
        );
        if headset.is_none() && HEADSET_KEYS.iter().any(|k| hay.contains(k)) {
            headset = Some(p.name.as_str());
        } else if internal.is_none() && INTERNAL_KEYS.iter().any(|k| hay.contains(k)) {
            internal = Some(p.name.as_str());
        }
    }
    // Fallback: no keyword split but exactly two ports → flip [0] ↔ [1].
    if (internal.is_none() || headset.is_none()) && ports.len() == 2 {
        internal = Some(ports[0].name.as_str());
        headset = Some(ports[1].name.as_str());
    }
    let (internal, headset) = match (internal, headset) {
        (Some(i), Some(h)) if i != h => (i.to_string(), h.to_string()),
        _ => return None,
    };
    let on_headset = active == Some(headset.as_str());
    let headset_available = ports.iter().any(|p| p.name == headset && p.available);
    Some(Route {
        on_headset,
        switchable: on_headset || headset_available,
        internal,
        headset,
    })
}

pub(crate) struct AudioRouteModel {
    orientation: Orientation,
    root_box: gtk::Box,
    icon: gtk::Image,
    label: gtk::Label,
    // ── Port-level (combo-jack) state: routes on the current default devices.
    in_dev: Option<Arc<InputDevice>>,
    out_dev: Option<Arc<OutputDevice>>,
    in_route: Option<Route>,
    out_route: Option<Route>,
    in_ports_token: WatcherToken,
    out_ports_token: WatcherToken,
    // ── Device-level state: resolved each reload from the live device lists,
    //    so the click handler can act without re-scanning.
    device_headset_out: Option<Arc<OutputDevice>>,
    device_builtin_out: Option<Arc<OutputDevice>>,
    device_headset_in: Option<Arc<InputDevice>>,
    device_builtin_in: Option<Arc<InputDevice>>,
    device_on_headset: bool,
    device_usable: bool,
    /// The non-headset default we were last on — the exact device a "back to
    /// built-in" click restores (e.g. the USB speakers), not a keyword guess.
    last_builtin_out: Option<Arc<OutputDevice>>,
    last_builtin_in: Option<Arc<InputDevice>>,
    /// Keep the device-list watchers alive for the widget's lifetime.
    _out_devices_token: WatcherToken,
    _in_devices_token: WatcherToken,
}

#[derive(Debug)]
pub(crate) enum AudioRouteInput {
    InDeviceChanged,
    OutDeviceChanged,
    Reload,
    Clicked,
}

#[derive(Debug)]
pub(crate) enum AudioRouteOutput {}

#[derive(Debug)]
pub(crate) enum AudioRouteCommandOutput {
    InDeviceChanged,
    OutDeviceChanged,
    Reload,
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
                    sender.input(AudioRouteInput::Clicked);
                },

                gtk::Box {
                    set_spacing: 6,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    #[local_ref]
                    icon_widget -> gtk::Image {
                        set_icon_name: Some("audio-volume-high-symbolic"),
                    },
                    #[local_ref]
                    label_widget -> gtk::Label {
                        add_css_class: "audio-route-label",
                    },
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
        let label_widget = gtk::Label::new(None);

        spawn_default_input_watcher(&sender, None, || AudioRouteCommandOutput::InDeviceChanged);
        spawn_default_output_watcher(&sender, None, || AudioRouteCommandOutput::OutDeviceChanged);

        // Device-level: repaint whenever a device appears/disappears (e.g. the
        // Bluetooth/USB headset connecting or dropping) so the pill shows up
        // and flips its route without a manual refresh.
        let mut out_devices_token = WatcherToken::new();
        let mut in_devices_token = WatcherToken::new();
        spawn_output_devices_watcher(&sender, out_devices_token.reset(), || {
            AudioRouteCommandOutput::Reload
        });
        spawn_input_devices_watcher(&sender, in_devices_token.reset(), || {
            AudioRouteCommandOutput::Reload
        });

        let model = AudioRouteModel {
            orientation: params.orientation,
            root_box: root.clone(),
            icon: icon_widget.clone(),
            label: label_widget.clone(),
            in_dev: None,
            out_dev: None,
            in_route: None,
            out_route: None,
            in_ports_token: WatcherToken::new(),
            out_ports_token: WatcherToken::new(),
            device_headset_out: None,
            device_builtin_out: None,
            device_headset_in: None,
            device_builtin_in: None,
            device_on_headset: false,
            device_usable: false,
            last_builtin_out: None,
            last_builtin_in: None,
            _out_devices_token: out_devices_token,
            _in_devices_token: in_devices_token,
        };
        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            AudioRouteInput::InDeviceChanged => {
                let token = self.in_ports_token.reset();
                self.in_dev = audio_service().default_input.get();
                if let Some(dev) = &self.in_dev {
                    spawn_input_device_ports_watcher(dev, token, &sender, || {
                        AudioRouteCommandOutput::Reload
                    });
                }
                self.reload();
            }
            AudioRouteInput::OutDeviceChanged => {
                let token = self.out_ports_token.reset();
                self.out_dev = audio_service().default_output.get();
                if let Some(dev) = &self.out_dev {
                    spawn_output_device_ports_watcher(dev, token, &sender, || {
                        AudioRouteCommandOutput::Reload
                    });
                }
                self.reload();
            }
            AudioRouteInput::Reload => self.reload(),
            AudioRouteInput::Clicked => self.toggle(),
        }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            AudioRouteCommandOutput::InDeviceChanged => {
                sender.input(AudioRouteInput::InDeviceChanged);
            }
            AudioRouteCommandOutput::OutDeviceChanged => {
                sender.input(AudioRouteInput::OutDeviceChanged);
            }
            AudioRouteCommandOutput::Reload => sender.input(AudioRouteInput::Reload),
        }
    }
}

impl AudioRouteModel {
    /// Recompute the device-level and port-level routes from live audio state
    /// and repaint the pill.
    fn reload(&mut self) {
        self.recompute_device_route();

        // Port-level (combo-jack) routes on the CURRENT default devices — the
        // fallback for laptops whose internal codec exposes speaker + headphone
        // as two ports on one device (no separate headset *device* to switch).
        self.in_route = self
            .in_dev
            .as_ref()
            .and_then(|d| resolve_route(&d.ports.get(), d.active_port.get().as_deref()));
        self.out_route = self
            .out_dev
            .as_ref()
            .and_then(|d| resolve_route(&d.ports.get(), d.active_port.get().as_deref()));

        // Show if EITHER mechanism has somewhere to switch. Device-level wins
        // when a separate headset device exists; otherwise the combo-jack
        // fallback drives it. A machine with a single output and no headset
        // resolves neither and stays hidden.
        let visible = self.device_usable || self.in_route.is_some() || self.out_route.is_some();
        let on_headset = self.device_on_headset || self.port_on_headset();

        self.root_box.set_visible(visible);
        if !visible {
            return;
        }
        if on_headset {
            self.icon.set_icon_name(Some("audio-headset-symbolic"));
            self.label.set_label("Headset");
            self.root_box
                .set_tooltip_text(Some("Audio on headset — click for built-in"));
        } else {
            self.icon.set_icon_name(Some("audio-volume-high-symbolic"));
            self.label.set_label("Built-in");
            self.root_box
                .set_tooltip_text(Some("Audio on built-in — click for headset"));
        }
    }

    /// Resolve the device-level route (headset ↔ built-in *devices*) from the
    /// live device lists + defaults, remembering the current built-in default
    /// so a "back to built-in" click restores exactly it.
    fn recompute_device_route(&mut self) {
        let audio = audio_service();
        let out_devices = audio.output_devices.get();
        let in_devices = audio.input_devices.get();
        let default_out = audio.default_output.get();
        let default_in = audio.default_input.get();

        // Remember the built-in (non-headset) default we're on, so switching
        // back returns to that exact device rather than a keyword guess.
        if let Some(d) = default_out.as_ref().filter(|d| !out_is_headset(d)) {
            self.last_builtin_out = Some(d.clone());
        }
        if let Some(d) = default_in.as_ref().filter(|d| !in_is_headset(d)) {
            self.last_builtin_in = Some(d.clone());
        }

        let headset_out = out_devices.iter().find(|d| out_is_headset(d)).cloned();
        let headset_in = in_devices.iter().find(|d| in_is_headset(d)).cloned();
        // Built-in target: the remembered non-headset default if it's still
        // present, else the first non-headset device.
        let builtin_out = self
            .last_builtin_out
            .clone()
            .filter(|d| out_devices.iter().any(|x| x.name.get() == d.name.get()))
            .or_else(|| out_devices.iter().find(|d| !out_is_headset(d)).cloned());
        let builtin_in = self
            .last_builtin_in
            .clone()
            .filter(|d| in_devices.iter().any(|x| x.name.get() == d.name.get()))
            .or_else(|| in_devices.iter().find(|d| !in_is_headset(d)).cloned());

        self.device_on_headset = default_out.as_ref().is_some_and(|d| out_is_headset(d))
            || default_in.as_ref().is_some_and(|d| in_is_headset(d));
        // Usable only when we can both reach a headset and get back to a
        // built-in.
        self.device_usable = headset_out.is_some() && builtin_out.is_some();
        self.device_headset_out = headset_out;
        self.device_headset_in = headset_in;
        self.device_builtin_out = builtin_out;
        self.device_builtin_in = builtin_in;
    }

    /// Port-level: true when a routable combo-jack side is on its headset port.
    fn port_on_headset(&self) -> bool {
        let in_h = self
            .in_route
            .as_ref()
            .is_some_and(|r| r.switchable && r.on_headset);
        let out_h = self
            .out_route
            .as_ref()
            .is_some_and(|r| r.switchable && r.on_headset);
        in_h || out_h
    }

    /// Flip the whole audio path to the opposite of the current route.
    fn toggle(&self) {
        let to_headset = !(self.device_on_headset || self.port_on_headset());

        // Device-level takes precedence: move the default sink (and, unless the
        // user turned the mic off in Settings, the default source) between the
        // built-in and headset devices.
        if self.device_usable {
            let switch_mic = config_manager()
                .config()
                .audio()
                .route_switch_microphone()
                .get_untracked();
            let out_target = if to_headset {
                self.device_headset_out.clone()
            } else {
                self.device_builtin_out.clone()
            };
            let in_target = if to_headset {
                self.device_headset_in.clone()
            } else {
                self.device_builtin_in.clone()
            };
            if let Some(dev) = out_target {
                tokio::spawn(async move {
                    let _ = dev.set_as_default().await;
                });
            }
            if switch_mic && let Some(dev) = in_target {
                tokio::spawn(async move {
                    let _ = dev.set_as_default().await;
                });
            }
            return;
        }

        // Port-level (combo-jack) fallback: flip the active port on each side.
        if let (Some(dev), Some(name)) =
            (self.in_dev.clone(), target_port(&self.in_route, to_headset))
        {
            tokio::spawn(async move {
                let _ = dev.set_port(name).await;
            });
        }
        if let (Some(dev), Some(name)) = (
            self.out_dev.clone(),
            target_port(&self.out_route, to_headset),
        ) {
            tokio::spawn(async move {
                let _ = dev.set_port(name).await;
            });
        }
    }
}

/// Device-level headset test for an output device (structured props first).
fn out_is_headset(d: &OutputDevice) -> bool {
    is_headset(&d.properties.get(), &d.name.get(), &d.description.get())
}

/// Device-level headset test for an input device (structured props first).
fn in_is_headset(d: &InputDevice) -> bool {
    is_headset(&d.properties.get(), &d.name.get(), &d.description.get())
}

/// The port to route a device to for `to_headset`, or `None` when the
/// device isn't switchable.
fn target_port(route: &Option<Route>, to_headset: bool) -> Option<String> {
    route.as_ref().filter(|r| r.switchable).map(|r| {
        if to_headset {
            r.headset.clone()
        } else {
            r.internal.clone()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(name: &str, desc: &str, available: bool) -> DevicePort {
        DevicePort {
            name: name.to_string(),
            description: desc.to_string(),
            priority: 0,
            available,
        }
    }

    #[test]
    fn resolves_combo_jack_mic() {
        let ports = vec![
            port("analog-input-internal-mic", "Internal Microphone", true),
            port("analog-input-headset-mic", "Headset Microphone", true),
        ];
        let r = resolve_route(&ports, Some("analog-input-internal-mic")).unwrap();
        assert_eq!(r.internal, "analog-input-internal-mic");
        assert_eq!(r.headset, "analog-input-headset-mic");
        assert!(!r.on_headset);
        assert!(r.switchable); // headset port available
    }

    #[test]
    fn resolves_output_speaker_vs_headphones() {
        let ports = vec![
            port("analog-output-speaker", "Speakers", true),
            port("analog-output-headphones", "Headphones", true),
        ];
        let r = resolve_route(&ports, Some("analog-output-headphones")).unwrap();
        assert_eq!(r.internal, "analog-output-speaker");
        assert_eq!(r.headset, "analog-output-headphones");
        assert!(r.on_headset);
        assert!(r.switchable);
    }

    #[test]
    fn not_switchable_when_headset_unplugged() {
        // Headset port present but unavailable, and we're on internal.
        let ports = vec![
            port("analog-output-speaker", "Speakers", true),
            port("analog-output-headphones", "Headphones", false),
        ];
        let r = resolve_route(&ports, Some("analog-output-speaker")).unwrap();
        assert!(!r.on_headset);
        assert!(!r.switchable); // nothing to switch to
    }

    #[test]
    fn none_when_single_port() {
        let ports = vec![port("analog-output-speaker", "Speakers", true)];
        assert_eq!(resolve_route(&ports, Some("analog-output-speaker")), None);
    }

    #[test]
    fn fallback_two_unclassified_ports() {
        let ports = vec![
            port("port-a", "Output A", true),
            port("port-b", "Output B", true),
        ];
        let r = resolve_route(&ports, Some("port-a")).unwrap();
        assert_eq!(r.internal, "port-a");
        assert_eq!(r.headset, "port-b");
        assert!(!r.on_headset);
    }

    fn props(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn headset_form_factor_is_authoritative() {
        // A BlueZ headset advertises device.form_factor=headset — decisive,
        // no name guessing (this is what makes it portable across machines).
        let bt = props(&[
            ("device.form_factor", "headset"),
            ("device.bus", "bluetooth"),
        ]);
        assert!(is_headset(&bt, "bluez_output.F4_9D", "WH-1000XM4"));

        // A Bluetooth *speaker* carries form_factor=speaker → NOT a headset,
        // even though it's on the bluetooth bus and the name is unhelpful.
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
        // HDMI / speakers / analog card must NOT read as headsets — this is
        // exactly the false-positive the old keyword match risked.
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
        // No form_factor, no icon — a bluetooth-bus output is still a headset.
        assert!(is_headset(
            &props(&[("device.bus", "bluetooth")]),
            "bluez_output.xx",
            ""
        ));
    }

    #[test]
    fn headset_name_fallback_when_no_metadata() {
        // Empty props → last-resort name heuristic.
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
        // The pure fallback in isolation.
        assert!(is_headset_name("x", "My Headphones"));
        assert!(!is_headset_name("alsa_output.hdmi", "HDMI Audio"));
    }
}
