//! Audio Route — bar pill that flips the whole default audio path
//! (default input/mic **and** default output/speaker together) between
//! the built-in device port and a headset/external port, via
//! `wayle_audio`'s `set_port`. Native port of the DMS `Audio Port
//! Switcher` plugin, generalized to route both sides at once.
//!
//! One click: if anything is currently on the headset → send everything
//! back to built-in; otherwise route everything to the headset. The
//! glyph + label reflect the current route. The pill hides itself unless
//! at least one side is routable (a headset port is present + available),
//! so a machine with no combo jack never sees it.

use mshell_common::WatcherToken;
use mshell_services::audio_service;
use mshell_utils::audio::{
    spawn_default_input_watcher, spawn_default_output_watcher, spawn_input_device_ports_watcher,
    spawn_output_device_ports_watcher,
};
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
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
    in_dev: Option<Arc<InputDevice>>,
    out_dev: Option<Arc<OutputDevice>>,
    in_route: Option<Route>,
    out_route: Option<Route>,
    in_ports_token: WatcherToken,
    out_ports_token: WatcherToken,
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
    /// Recompute both routes from live device state and repaint the pill.
    fn reload(&mut self) {
        self.in_route = self
            .in_dev
            .as_ref()
            .and_then(|d| resolve_route(&d.ports.get(), d.active_port.get().as_deref()));
        self.out_route = self
            .out_dev
            .as_ref()
            .and_then(|d| resolve_route(&d.ports.get(), d.active_port.get().as_deref()));

        let in_ok = self.in_route.as_ref().is_some_and(|r| r.switchable);
        let out_ok = self.out_route.as_ref().is_some_and(|r| r.switchable);
        let visible = in_ok || out_ok;
        let on_headset = self.on_headset();

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

    /// True when a routable side is currently on its headset port.
    fn on_headset(&self) -> bool {
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

    /// Flip both routable devices to the opposite of the current route.
    fn toggle(&self) {
        let to_headset = !self.on_headset();
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
}
