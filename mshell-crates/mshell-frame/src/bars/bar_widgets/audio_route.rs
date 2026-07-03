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
    mic_follow_target, next_index, out_is_headset, routable_outputs, spawn_default_output_watcher,
    spawn_output_devices_watcher,
};
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

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
        if switch_mic && let Some(mic) = mic_follow_target(to_headset) {
            tokio::spawn(async move {
                let _ = mic.set_as_default().await;
            });
        }
    }
}
