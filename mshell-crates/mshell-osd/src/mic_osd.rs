//! Microphone OSD — the input-side twin of [`crate::volume_osd`].
//!
//! Pops the same bottom-centre pill when the default audio *source*
//! (microphone) mute/level changes — e.g. the `XF86AudioMicMute` key
//! routed through `mshellctl audio mic-mute`. The icon flips to the
//! muted-mic glyph via [`get_audio_in_icon`], giving the toggle the same
//! visual feedback the volume keys already get. Watches the device
//! reactively, so it reflects changes from any source, not just our keys.

use gtk4::gdk;
use gtk4::prelude::{BoxExt, GtkWindowExt, OrientableExt, RangeExt, WidgetExt};
use gtk4_layer_shell::{Layer, LayerShell};
use mshell_common::WatcherToken;
use mshell_services::audio_service;
use mshell_utils::audio::{
    get_audio_in_icon, spawn_default_input_watcher, spawn_input_device_volume_mute_watcher,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::sync::Arc;
use wayle_audio::core::device::input::InputDevice;

#[derive(Debug)]
pub struct MicOsdModel {
    active_device_watcher_token: WatcherToken,
    hide_token: WatcherToken,
    icon_name: String,
    slider_value: f64,
    value_label: String,
    shown_count: u16,
}

#[derive(Debug)]
pub enum MicOsdInput {
    UpdateDevice(Arc<InputDevice>),
    Show,
    Hide,
}

#[derive(Debug)]
pub enum MicOsdOutput {}

pub struct MicOsdInit {
    pub monitor: gdk::Monitor,
}

#[derive(Debug)]
pub enum MicOsdCommandOutput {
    DeviceChanged,
    MuteChanged,
    Hide,
}

#[relm4::component(pub)]
impl Component for MicOsdModel {
    type CommandOutput = MicOsdCommandOutput;
    type Input = MicOsdInput;
    type Output = MicOsdOutput;
    type Init = MicOsdInit;

    view! {
        #[root]
        gtk::Window {
            set_css_classes: &["osd-window", "window-opacity"],
            set_decorated: false,
            set_visible: false,
            set_default_height: 1,

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,

                gtk::Image {
                    add_css_class: "osd-icon",
                    #[watch]
                    set_icon_name: Some(model.icon_name.as_str()),
                },

                gtk::Scale {
                    add_css_class: "ok-progress-bar",
                    set_hexpand: true,
                    set_can_focus: false,
                    set_focus_on_click: false,
                    set_range: (0.0, 1.0),
                    #[watch]
                    set_value: model.slider_value,
                },

                gtk::Label {
                    add_css_class: "osd-value",
                    set_width_chars: 4,
                    set_xalign: 1.0,
                    #[watch]
                    set_label: &model.value_label,
                }
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        root.init_layer_shell();
        root.set_monitor(Some(&params.monitor));
        root.set_namespace(Some("mshell-osd"));
        root.set_layer(Layer::Overlay);
        root.set_exclusive_zone(0);
        let (position, distance) = crate::osd_geometry::read();
        crate::osd_geometry::apply(&root, &position, distance);

        spawn_default_input_watcher(&sender, None, || MicOsdCommandOutput::DeviceChanged);

        let model = MicOsdModel {
            active_device_watcher_token: WatcherToken::new(),
            hide_token: WatcherToken::new(),
            icon_name: "audio-input-microphone-symbolic".to_string(),
            slider_value: 0.0,
            value_label: "0%".to_string(),
            shown_count: 0,
        };

        let widgets = view_output!();

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
            MicOsdInput::UpdateDevice(device) => {
                self.icon_name = get_audio_in_icon(&device).to_string();
                if device.muted.get() {
                    self.slider_value = 0.0;
                    self.value_label = "Off".to_string();
                } else {
                    let volume = device.volume.get();
                    self.slider_value = volume.average();
                    self.value_label = format!("{}%", volume.average_percentage().round() as i32);
                }

                let token = self.hide_token.reset();
                sender.command(|out, shutdown| {
                    shutdown
                        .register(async move {
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                            if !token.is_cancelled() {
                                out.send(MicOsdCommandOutput::Hide).ok();
                            }
                        })
                        .drop_on_shutdown()
                });
            }
            MicOsdInput::Show => {
                if self.shown_count > 1 {
                    root.set_visible(true);
                } else {
                    self.shown_count += 1;
                }
            }
            MicOsdInput::Hide => {
                root.set_visible(false);
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
            MicOsdCommandOutput::DeviceChanged => {
                if let Some(input_device) = audio_service().default_input.get() {
                    sender.input(MicOsdInput::UpdateDevice(input_device.clone()));

                    let token = self.active_device_watcher_token.reset();

                    spawn_input_device_volume_mute_watcher(&input_device, token, &sender, || {
                        MicOsdCommandOutput::MuteChanged
                    });
                }
            }
            MicOsdCommandOutput::MuteChanged => {
                if let Some(default_input) = audio_service().default_input.get() {
                    sender.input(MicOsdInput::UpdateDevice(default_input));
                    sender.input(MicOsdInput::Show);
                }
            }
            MicOsdCommandOutput::Hide => {
                sender.input(MicOsdInput::Hide);
            }
        }
    }
}
