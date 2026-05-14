use mshell_common::WatcherToken;
use mshell_services::audio_service;
use mshell_utils::audio::{
    get_audio_in_icon, spawn_default_input_watcher, spawn_input_device_volume_mute_watcher,
};
use mshell_utils::hover_scroll::{HoverScrollHandle, attach_hover_scroll};
use relm4::gtk::prelude::WidgetExt;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::sync::Arc;
use wayle_audio::core::device::input::InputDevice;
use wayle_audio::volume::types::Volume;

const STEP_PERCENT: f64 = 5.0;

#[derive(Debug)]
pub(crate) struct AudioInputModel {
    active_device_watcher_token: WatcherToken,
    _scroll: Option<HoverScrollHandle>,
}

#[derive(Debug)]
pub(crate) enum AudioInputInput {
    UpdateDevice(Arc<InputDevice>),
}

#[derive(Debug)]
pub(crate) enum AudioInputOutput {}

pub(crate) struct AudioInputInit {}

#[derive(Debug)]
pub(crate) enum AudioInputCommandOutput {
    DeviceChanged,
    VolumeChanged,
}

#[relm4::component(pub)]
impl Component for AudioInputModel {
    type CommandOutput = AudioInputCommandOutput;
    type Input = AudioInputInput;
    type Output = AudioInputOutput;
    type Init = AudioInputInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["audio-input-bar-widget", "ok-button-surface", "ok-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,
            set_has_tooltip: true,

            #[name="image"]
            gtk::Image {
                set_hexpand: true,
                set_vexpand: true,
                set_halign: gtk::Align::Center,
                set_valign: gtk::Align::Center,
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        spawn_default_input_watcher(&sender, None, || AudioInputCommandOutput::DeviceChanged);

        let scroll = attach_hover_scroll(&root, |_dx, dy, _hovered, _shift| {
            if dy.abs() < 0.5 {
                return;
            }
            let delta = if dy < 0.0 { STEP_PERCENT } else { -STEP_PERCENT };
            adjust_volume(delta);
        });

        let model = AudioInputModel {
            active_device_watcher_token: WatcherToken::new(),
            _scroll: Some(scroll),
        };

        let widgets = view_output!();

        if let Some(device) = audio_service().default_input.get() {
            apply_device_state(&widgets.image, &root, &device);
        }

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            AudioInputInput::UpdateDevice(device) => {
                apply_device_state(&widgets.image, root, &device);
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
            AudioInputCommandOutput::DeviceChanged => {
                let device = audio_service().default_input.get();
                if let Some(audio_device) = device.as_ref() {
                    sender.input(AudioInputInput::UpdateDevice(audio_device.clone()));

                    let token = self.active_device_watcher_token.reset();

                    spawn_input_device_volume_mute_watcher(audio_device, token, &sender, || {
                        AudioInputCommandOutput::VolumeChanged
                    });
                }
            }
            AudioInputCommandOutput::VolumeChanged => {
                if let Some(default_input) = audio_service().default_input.get() {
                    sender.input(AudioInputInput::UpdateDevice(default_input));
                }
            }
        }
    }
}

fn apply_device_state(image: &gtk::Image, root: &gtk::Box, device: &Arc<InputDevice>) {
    image.set_icon_name(Some(get_audio_in_icon(device)));

    let volume = device.volume.get();
    let pct = volume.average_percentage().round() as u32;
    let muted = device.muted.get() || volume.is_muted();

    root.set_tooltip_text(Some(&format!(
        "Microphone: {}",
        if muted { "muted".to_string() } else { format!("{pct}%") }
    )));

    if muted {
        root.add_css_class("muted");
    } else {
        root.remove_css_class("muted");
    }
}

fn adjust_volume(delta_percent: f64) {
    let Some(device) = audio_service().default_input.get() else {
        return;
    };
    let current = device.volume.get();
    let new_pct = (current.average_percentage() + delta_percent).clamp(0.0, 100.0);
    let channels = current.channels().max(1);
    let new_volume = Volume::from_percentage(new_pct, channels);
    tokio::spawn(async move {
        let _ = device.set_volume(new_volume).await;
    });
}
