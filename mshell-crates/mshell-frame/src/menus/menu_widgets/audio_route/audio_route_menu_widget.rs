//! Audio Route picker rendered as an in-frame menu widget.
//!
//! This is the right-click surface for the Audio Route bar pill
//! (`bar_widgets/audio_route.rs`). Where the pill's left-click *cycles*
//! through outputs, this menu lists every routable output at once so the
//! user can jump straight to one. HDMI / DisplayPort sinks are filtered out
//! (they're monitors, not something you "route" audio to) — the same
//! `is_hdmi_output` rule the pill uses.
//!
//! Content: a vertical boxed list of output rows (description + a check
//! glyph on the current default). The list is rebuilt on any device add /
//! remove and whenever the default output flips, so it always mirrors live
//! PipeWire state. Clicking a row makes that output the default and closes
//! the drawer.

use mshell_common::WatcherToken;
use mshell_services::audio_service;
use mshell_utils::audio::{
    routable_outputs, spawn_default_output_watcher, spawn_output_devices_watcher,
};
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::sync::Arc;
use wayle_audio::core::device::output::OutputDevice;

pub(crate) struct AudioRouteMenuWidgetModel {
    /// The row container. Shares its underlying GTK widget with the view's
    /// `#[local_ref]` box, so [`refresh`](Self::refresh) can rebuild the rows
    /// in place whenever devices or the default output change.
    list: gtk::Box,
    /// Keeps the output-device-list watcher alive for the widget's lifetime
    /// (dropping the token cancels the watch).
    _out_devices_token: WatcherToken,
}

#[derive(Debug)]
pub(crate) enum AudioRouteMenuWidgetInput {
    /// Rebuild the row list (a device or the default output changed).
    Refresh,
    /// A row was clicked — make this output the default.
    Select(Arc<OutputDevice>),
}

#[derive(Debug)]
pub(crate) enum AudioRouteMenuWidgetOutput {
    /// Collapse the host menu after a pick.
    CloseMenu,
}

#[derive(Debug)]
pub(crate) enum AudioRouteMenuWidgetCommandOutput {
    Refresh,
}

pub(crate) struct AudioRouteMenuWidgetInit {}

#[relm4::component(pub(crate))]
impl Component for AudioRouteMenuWidgetModel {
    type CommandOutput = AudioRouteMenuWidgetCommandOutput;
    type Input = AudioRouteMenuWidgetInput;
    type Output = AudioRouteMenuWidgetOutput;
    type Init = AudioRouteMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "audio-route-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 8,

            // ── §12 panel header ──
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,
                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("audio-volume-high-symbolic"),
                },
                gtk::Label {
                    add_css_class: "panel-title",
                    set_label: "Audio Route",
                    set_halign: gtk::Align::Start,
                    set_hexpand: true,
                    set_valign: gtk::Align::Center,
                },
            },

            #[local_ref]
            list_box -> gtk::Box {
                add_css_class: "boxed-list",
                set_orientation: gtk::Orientation::Vertical,
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let list_box = gtk::Box::new(gtk::Orientation::Vertical, 0);

        // Rebuild whenever the default output flips (permanent watch) or a
        // device appears / drops (e.g. the Bluetooth headset connecting).
        spawn_default_output_watcher(&sender, None, || AudioRouteMenuWidgetCommandOutput::Refresh);
        let mut out_devices_token = WatcherToken::new();
        spawn_output_devices_watcher(&sender, out_devices_token.reset(), || {
            AudioRouteMenuWidgetCommandOutput::Refresh
        });

        let model = AudioRouteMenuWidgetModel {
            list: list_box.clone(),
            _out_devices_token: out_devices_token,
        };
        let widgets = view_output!();
        model.refresh(&sender);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            AudioRouteMenuWidgetInput::Refresh => self.refresh(&sender),
            AudioRouteMenuWidgetInput::Select(device) => {
                tokio::spawn(async move {
                    let _ = device.set_as_default().await;
                });
                let _ = sender.output(AudioRouteMenuWidgetOutput::CloseMenu);
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
            AudioRouteMenuWidgetCommandOutput::Refresh => {
                sender.input(AudioRouteMenuWidgetInput::Refresh)
            }
        }
    }
}

impl AudioRouteMenuWidgetModel {
    /// Rebuild the row list from live PipeWire state: one button per routable
    /// output (HDMI/DisplayPort filtered out), name-sorted for a stable order,
    /// with a check glyph marking the current default.
    fn refresh(&self, sender: &ComponentSender<Self>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }

        let outputs = routable_outputs();
        let default_name = audio_service().default_output.get().map(|d| d.name.get());

        for device in outputs {
            let name = device.name.get();
            let desc = device.description.get();
            let label_text = if desc.is_empty() { name.clone() } else { desc };
            let is_default = default_name.as_deref() == Some(name.as_str());

            let row = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(12)
                .build();

            let label = gtk::Label::new(Some(&label_text));
            label.add_css_class("audio-route-menu-label");
            label.set_xalign(0.0);
            label.set_hexpand(true);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            row.append(&label);

            let check = gtk::Image::from_icon_name("object-select-symbolic");
            check.add_css_class("audio-route-menu-check");
            check.set_visible(is_default);
            row.append(&check);

            let btn = gtk::Button::builder()
                .child(&row)
                .css_classes(["ok-button-surface"])
                .build();

            let s = sender.clone();
            let dev = device.clone();
            btn.connect_clicked(move |_| {
                s.input(AudioRouteMenuWidgetInput::Select(dev.clone()));
            });

            self.list.append(&btn);
        }
    }
}
