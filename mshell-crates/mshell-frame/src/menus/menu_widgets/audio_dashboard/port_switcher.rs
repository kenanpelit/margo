//! Output/input port (route) switcher — QSAP's profile/port toggle.
//!
//! Renders the active default device's ports as chips (e.g. Speakers ↔
//! Headphones, the laptop's internal mic ↔ a headset mic) and routes
//! via `wayle_audio`'s `set_port`. Distinct from the device picker:
//! that switches *between* sinks/sources, this switches the route
//! *within* the current device. Hidden unless the device exposes ≥2
//! ports, so single-port hosts see no extra chrome.

use mshell_common::WatcherToken;
use mshell_services::audio_service;
use mshell_utils::audio::{
    spawn_default_input_watcher, spawn_default_output_watcher, spawn_input_device_ports_watcher,
    spawn_output_device_ports_watcher,
};
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::sync::Arc;
use wayle_audio::core::device::input::InputDevice;
use wayle_audio::core::device::output::OutputDevice;
use wayle_audio::types::device::DevicePort;

pub(crate) struct PortSwitcherModel {
    recording: bool,
    out_dev: Option<Arc<OutputDevice>>,
    in_dev: Option<Arc<InputDevice>>,
    ports: Vec<DevicePort>,
    active: Option<String>,
    ports_token: WatcherToken,
    has_ports: bool,
}

#[derive(Debug)]
pub(crate) enum PortSwitcherInput {
    DeviceChanged,
    PortsChanged,
    SelectPort(String),
}

#[derive(Debug)]
pub(crate) enum PortSwitcherOutput {}

pub(crate) struct PortSwitcherInit {
    /// `true` → input (mic) device ports; `false` → output device ports.
    pub recording: bool,
}

#[derive(Debug)]
pub(crate) enum PortSwitcherCommandOutput {
    DeviceChanged,
    PortsChanged,
}

#[relm4::component(pub)]
impl Component for PortSwitcherModel {
    type CommandOutput = PortSwitcherCommandOutput;
    type Input = PortSwitcherInput;
    type Output = PortSwitcherOutput;
    type Init = PortSwitcherInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "audio-port-section",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 4,
            #[watch]
            set_visible: model.has_ports,

            gtk::Label {
                add_css_class: "label-small-bold-variant",
                set_halign: gtk::Align::Start,
                set_label: if model.recording { "INPUT" } else { "OUTPUT" },
            },

            #[name = "chips_box"]
            gtk::Box {
                add_css_class: "audio-port-row",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 6,
                set_halign: gtk::Align::Start,
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        if params.recording {
            spawn_default_input_watcher(&sender, None, || PortSwitcherCommandOutput::DeviceChanged);
        } else {
            spawn_default_output_watcher(&sender, None, || {
                PortSwitcherCommandOutput::DeviceChanged
            });
        }

        let model = PortSwitcherModel {
            recording: params.recording,
            out_dev: None,
            in_dev: None,
            ports: Vec::new(),
            active: None,
            ports_token: WatcherToken::new(),
            has_ports: false,
        };

        let widgets = view_output!();

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
            PortSwitcherInput::DeviceChanged => {
                let token = self.ports_token.reset();
                if self.recording {
                    self.in_dev = audio_service().default_input.get();
                    if let Some(dev) = &self.in_dev {
                        spawn_input_device_ports_watcher(dev, token, &sender, || {
                            PortSwitcherCommandOutput::PortsChanged
                        });
                    }
                } else {
                    self.out_dev = audio_service().default_output.get();
                    if let Some(dev) = &self.out_dev {
                        spawn_output_device_ports_watcher(dev, token, &sender, || {
                            PortSwitcherCommandOutput::PortsChanged
                        });
                    }
                }
                self.reload_ports(&widgets.chips_box, &sender);
            }
            PortSwitcherInput::PortsChanged => {
                self.reload_ports(&widgets.chips_box, &sender);
            }
            PortSwitcherInput::SelectPort(name) => {
                if self.recording {
                    if let Some(dev) = self.in_dev.clone() {
                        tokio::spawn(async move {
                            let _ = dev.set_port(name).await;
                        });
                    }
                } else if let Some(dev) = self.out_dev.clone() {
                    tokio::spawn(async move {
                        let _ = dev.set_port(name).await;
                    });
                }
            }
        }

        self.update_view(widgets, sender);
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            PortSwitcherCommandOutput::DeviceChanged => {
                sender.input(PortSwitcherInput::DeviceChanged);
            }
            PortSwitcherCommandOutput::PortsChanged => {
                sender.input(PortSwitcherInput::PortsChanged);
            }
        }
    }
}

impl PortSwitcherModel {
    fn reload_ports(&mut self, chips_box: &gtk::Box, sender: &ComponentSender<Self>) {
        if self.recording {
            self.ports = self
                .in_dev
                .as_ref()
                .map(|d| d.ports.get())
                .unwrap_or_default();
            self.active = self.in_dev.as_ref().and_then(|d| d.active_port.get());
        } else {
            self.ports = self
                .out_dev
                .as_ref()
                .map(|d| d.ports.get())
                .unwrap_or_default();
            self.active = self.out_dev.as_ref().and_then(|d| d.active_port.get());
        }

        // Only worth showing when there's an actual choice to make.
        self.has_ports = self.ports.len() >= 2;

        while let Some(child) = chips_box.first_child() {
            chips_box.remove(&child);
        }
        if !self.has_ports {
            return;
        }

        for port in &self.ports {
            let chip = gtk::Button::builder()
                .label(port.description.as_str())
                .css_classes(["audio-port-chip"])
                .sensitive(port.available)
                .build();
            if self.active.as_deref() == Some(port.name.as_str()) {
                chip.add_css_class("selected");
            }
            let sender = sender.clone();
            let name = port.name.clone();
            chip.connect_clicked(move |_| {
                sender.input(PortSwitcherInput::SelectPort(name.clone()));
            });
            chips_box.append(&chip);
        }
    }
}
