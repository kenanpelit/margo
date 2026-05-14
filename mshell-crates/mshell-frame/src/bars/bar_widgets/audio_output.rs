use mshell_common::WatcherToken;
use mshell_services::audio_service;
use mshell_utils::audio::{
    get_audio_out_icon, spawn_default_output_watcher, spawn_output_device_volume_mute_watcher,
};
use mshell_utils::hover_scroll::{HoverScrollHandle, attach_hover_scroll};
use relm4::gtk::prelude::WidgetExt;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::sync::Arc;
use wayle_audio::core::device::output::OutputDevice;
use wayle_audio::volume::types::Volume;

/// Single scroll tick = ±5 %. Matches `mshellctl audio volume-up/down`
/// (which the keyboard binding uses) so wheel-on-bar and key shortcut
/// are interchangeable.
const STEP_PERCENT: f64 = 5.0;

#[derive(Debug)]
pub(crate) struct AudioOutputModel {
    active_device_watcher_token: WatcherToken,
    _scroll: Option<HoverScrollHandle>,
}

#[derive(Debug)]
pub(crate) enum AudioOutputInput {
    UpdateDevice(Arc<OutputDevice>),
}

#[derive(Debug)]
pub(crate) enum AudioOutputOutput {}

pub(crate) struct AudioOutputInit {}

#[derive(Debug)]
pub(crate) enum AudioOutputCommandOutput {
    DeviceChanged,
    VolumeChanged,
}

#[relm4::component(pub)]
impl Component for AudioOutputModel {
    type CommandOutput = AudioOutputCommandOutput;
    type Input = AudioOutputInput;
    type Output = AudioOutputOutput;
    type Init = AudioOutputInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["audio-output-bar-widget", "ok-button-surface", "ok-bar-widget"],
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
        spawn_default_output_watcher(&sender, None, || AudioOutputCommandOutput::DeviceChanged);

        // Hover-scroll → ±STEP_PERCENT on the default sink. Spawning
        // `set_volume` on a detached tokio task is fine — the bar
        // widget runs on glib's main loop, but `audio_service()` is
        // already pumped from the tokio runtime, so the await is
        // serviced regardless of who fires it.
        let scroll = attach_hover_scroll(&root, |_dx, dy, _hovered, _shift| {
            if dy.abs() < 0.5 {
                return;
            }
            let delta = if dy < 0.0 { STEP_PERCENT } else { -STEP_PERCENT };
            adjust_volume(delta);
        });

        let model = AudioOutputModel {
            active_device_watcher_token: WatcherToken::new(),
            _scroll: Some(scroll),
        };

        let widgets = view_output!();

        if let Some(device) = audio_service().default_output.get() {
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
            AudioOutputInput::UpdateDevice(device) => {
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
            AudioOutputCommandOutput::DeviceChanged => {
                let default_output = audio_service().default_output.get();
                if let Some(audio_device) = default_output {
                    sender.input(AudioOutputInput::UpdateDevice(audio_device.clone()));

                    let token = self.active_device_watcher_token.reset();

                    spawn_output_device_volume_mute_watcher(&audio_device, token, &sender, || {
                        AudioOutputCommandOutput::VolumeChanged
                    });
                }
            }
            AudioOutputCommandOutput::VolumeChanged => {
                if let Some(default_output) = audio_service().default_output.get() {
                    sender.input(AudioOutputInput::UpdateDevice(default_output));
                }
            }
        }
    }
}

fn apply_device_state(image: &gtk::Image, root: &gtk::Box, device: &Arc<OutputDevice>) {
    image.set_icon_name(Some(get_audio_out_icon(device)));

    let volume = device.volume.get();
    let pct = volume.average_percentage().round() as u32;
    let muted = device.muted.get() || volume.is_muted();

    root.set_tooltip_text(Some(&format!(
        "Speaker: {}",
        if muted { "muted".to_string() } else { format!("{pct}%") }
    )));

    // Toggle `.muted` so the SCSS rule in `_audio_widget.scss` can
    // tint the icon red. `set_css_classes` would clobber whatever
    // relm4's view! macro set up at root construction; use the
    // additive add/remove pair instead.
    if muted {
        root.add_css_class("muted");
    } else {
        root.remove_css_class("muted");
    }
}

/// Pump a ±delta-percent volume change through to the default sink.
/// Reads channels from the current `Volume` so multi-channel sinks
/// (stereo, 5.1, etc.) stay balanced. Bails out silently when there
/// is no default sink (rare — fresh boot, before PulseAudio is up).
fn adjust_volume(delta_percent: f64) {
    let Some(device) = audio_service().default_output.get() else {
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
