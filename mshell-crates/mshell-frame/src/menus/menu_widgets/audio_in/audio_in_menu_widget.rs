//! Microphone section of the audio menus: the input revealer row (mute,
//! volume, device list) plus the **Noise suppression** switch row — an
//! RNNoise virtual source (`mshell_utils::denoise`). The switch state is
//! probed on reveal and whenever the default input changes (turning it on
//! makes the virtual source the default, so the watcher picks that up).
//! Without the LADSPA plugin the row stays visible but insensitive with an
//! install hint.

use crate::common_widgets::revealer_row::revealer_row::{
    RevealerRowInit, RevealerRowInput, RevealerRowModel, RevealerRowOutput,
};
use crate::common_widgets::revealer_row::revealer_row_slider::{
    RevealerRowSliderInit, RevealerRowSliderInput, RevealerRowSliderModel, RevealerRowSliderOutput,
};
use crate::menus::menu_widgets::audio_in::audio_in_revealed_content::{
    AudioInRevealedContentInit, AudioInRevealedContentInput, AudioInRevealedContentModel,
};
use mshell_common::WatcherToken;
use mshell_services::audio_service;
use mshell_utils::audio::{
    get_audio_in_icon, spawn_default_input_watcher, spawn_input_device_volume_mute_watcher,
};
use mshell_utils::denoise::{self, DenoiseStatus};
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};
use std::sync::Arc;
use wayle_audio::core::device::input::InputDevice;
use wayle_audio::volume::types::Volume;

pub(crate) struct AudioInMenuWidgetModel {
    revealer_row: Controller<RevealerRowModel<RevealerRowSliderModel, AudioInRevealedContentModel>>,
    active_device_watcher_token: WatcherToken,
    /// Human-readable name of the active input device, shown above the
    /// row so it's visible without expanding the picker.
    device_name: String,
    denoise: DenoiseStatus,
    denoise_busy: bool,
    denoise_switch: gtk::Switch,
    denoise_signal: gtk::glib::SignalHandlerId,
    denoise_desc: gtk::Label,
}

#[derive(Debug)]
pub(crate) enum AudioInMenuWidgetInput {
    ActionButtonClicked,
    RevealerRowRevealed,
    RevealerRowHidden,
    ParentRevealChanged(bool),
    UpdateDevice(Arc<InputDevice>),
    SetVolume(f64),
    /// Noise-suppression switch flipped by the user.
    SetDenoise(bool),
}

#[derive(Debug)]
pub(crate) enum AudioInMenuWidgetOutput {}

pub(crate) struct AudioInMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum AudioInMenuWidgetCommandOutput {
    DeviceChanged,
    VolumeChanged,
    /// Fresh noise-suppression state; `true` = result of the user's toggle.
    Denoise(DenoiseStatus, bool),
}

#[relm4::component(pub)]
impl Component for AudioInMenuWidgetModel {
    type CommandOutput = AudioInMenuWidgetCommandOutput;
    type Input = AudioInMenuWidgetInput;
    type Output = AudioInMenuWidgetOutput;
    type Init = AudioInMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "audio-in-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 2,

            gtk::Label {
                add_css_class: "audio-device-active-label",
                #[watch]
                set_label: &model.device_name,
                #[watch]
                set_visible: !model.device_name.is_empty(),
                set_halign: gtk::Align::Start,
                set_ellipsize: gtk::pango::EllipsizeMode::End,
            },

            model.revealer_row.widget().clone() {},

            // Noise suppression — the VPN-menu toggle-row shape.
            gtk::Box {
                set_css_classes: &["ok-button-surface", "audio-denoise-row"],
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 10,
                set_margin_top: 6,

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 1,
                    set_hexpand: true,
                    gtk::Label {
                        set_label: "Noise suppression",
                        set_xalign: 0.0,
                    },
                    #[local_ref]
                    denoise_desc_widget -> gtk::Label {
                        set_css_classes: &["label-small", "dim-label"],
                        set_xalign: 0.0,
                        set_wrap: true,
                        set_wrap_mode: gtk::pango::WrapMode::WordChar,
                    },
                },
                #[local_ref]
                denoise_switch_widget -> gtk::Switch {
                    set_valign: gtk::Align::Center,
                    set_tooltip_text: Some("RNNoise virtual microphone for calls"),
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        spawn_default_input_watcher(&sender, None, || {
            AudioInMenuWidgetCommandOutput::DeviceChanged
        });

        let row_content = RevealerRowSliderModel::builder()
            .launch(RevealerRowSliderInit {})
            .forward(sender.input_sender(), |msg| match msg {
                RevealerRowSliderOutput::ValueChanged(value) => {
                    AudioInMenuWidgetInput::SetVolume(value)
                }
            });

        let revealed_content = AudioInRevealedContentModel::builder()
            .launch(AudioInRevealedContentInit {})
            .detach();

        let revealer_row =
            RevealerRowModel::<RevealerRowSliderModel, AudioInRevealedContentModel>::builder()
                .launch(RevealerRowInit {
                    icon_name: "microphone-sensitivity-medium-symbolic".into(),
                    action_button_sensitive: true,
                    content: row_content,
                    revealed_content,
                })
                .forward(sender.input_sender(), |msg| match msg {
                    RevealerRowOutput::ActionButtonClicked => {
                        AudioInMenuWidgetInput::ActionButtonClicked
                    }
                    RevealerRowOutput::Revealed => AudioInMenuWidgetInput::RevealerRowRevealed,
                    RevealerRowOutput::Hidden => AudioInMenuWidgetInput::RevealerRowHidden,
                });

        let denoise_switch_widget = gtk::Switch::new();
        let denoise_desc_widget = gtk::Label::new(Some("…"));
        let s = sender.clone();
        let denoise_signal = denoise_switch_widget.connect_state_set(move |_, on| {
            s.input(AudioInMenuWidgetInput::SetDenoise(on));
            gtk::glib::Propagation::Stop
        });

        let model = AudioInMenuWidgetModel {
            revealer_row,
            active_device_watcher_token: WatcherToken::new(),
            device_name: String::new(),
            denoise: DenoiseStatus::default(),
            denoise_busy: false,
            denoise_switch: denoise_switch_widget.clone(),
            denoise_signal,
            denoise_desc: denoise_desc_widget.clone(),
        };

        // The Audio Dashboard wants the device list visible on open, not
        // behind an extra click — start expanded. The chevron still works
        // to collapse it back for anyone who wants the compact view.
        model.revealer_row.emit(RevealerRowInput::SetRevealed(true));

        let widgets = view_output!();
        sync_denoise(&model);

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
            AudioInMenuWidgetInput::ActionButtonClicked => {
                if let Some(default) = audio_service().and_then(|a| a.default_input.get()) {
                    tokio::spawn(async move {
                        let mute = !default.muted.get();
                        let _ = default.set_mute(mute).await;
                    });
                }
            }
            AudioInMenuWidgetInput::RevealerRowRevealed => {
                self.revealer_row
                    .model()
                    .revealed_content
                    .emit(AudioInRevealedContentInput::Revealed);
            }
            AudioInMenuWidgetInput::RevealerRowHidden => {
                self.revealer_row
                    .model()
                    .revealed_content
                    .emit(AudioInRevealedContentInput::Hidden);
            }
            AudioInMenuWidgetInput::ParentRevealChanged(revealed) => {
                // Re-expand every time the dashboard opens, not just once at
                // component creation — the device list should always start
                // visible, not remember a collapsed state from last time.
                self.revealer_row
                    .emit(RevealerRowInput::SetRevealed(revealed));
                if revealed {
                    probe_denoise(&sender);
                }
            }
            AudioInMenuWidgetInput::SetDenoise(on) => {
                if self.denoise_busy || on == self.denoise.enabled || !self.denoise.available() {
                    sync_denoise(self);
                    return;
                }
                self.denoise_busy = true;
                sync_denoise(self);
                let enabled = self.denoise.enabled;
                sender.oneshot_command(async move {
                    if let Err(e) = denoise::toggle(enabled).await {
                        denoise::toast_failure(&e);
                    }
                    AudioInMenuWidgetCommandOutput::Denoise(denoise::probe().await, true)
                });
            }
            AudioInMenuWidgetInput::UpdateDevice(device) => {
                let desc = device.description.get();
                self.device_name = if desc.is_empty() {
                    device.name.get()
                } else {
                    desc
                };
                self.revealer_row
                    .emit(RevealerRowInput::UpdateActionIconName(
                        get_audio_in_icon(&device).to_string(),
                    ));
                self.revealer_row
                    .model()
                    .content
                    .emit(RevealerRowSliderInput::SetValue(
                        device.volume.get().average(),
                    ))
            }
            AudioInMenuWidgetInput::SetVolume(volume) => {
                if let Some(default) = audio_service().and_then(|a| a.default_input.get()) {
                    tokio::spawn(async move {
                        let _ = default.set_volume(Volume::stereo(volume, volume)).await;
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
            AudioInMenuWidgetCommandOutput::Denoise(status, from_action) => {
                if self.denoise_busy && !from_action {
                    return;
                }
                self.denoise_busy = false;
                self.denoise = status;
                sync_denoise(self);
            }
            AudioInMenuWidgetCommandOutput::DeviceChanged => {
                probe_denoise(&sender);
                let default = audio_service().and_then(|a| a.default_input.get());
                if let Some(audio_device) = default {
                    sender.input(AudioInMenuWidgetInput::UpdateDevice(audio_device.clone()));

                    let token = self.active_device_watcher_token.reset();

                    spawn_input_device_volume_mute_watcher(&audio_device, token, &sender, || {
                        AudioInMenuWidgetCommandOutput::VolumeChanged
                    });
                }
            }
            AudioInMenuWidgetCommandOutput::VolumeChanged => {
                if let Some(default) = audio_service().and_then(|a| a.default_input.get()) {
                    sender.input(AudioInMenuWidgetInput::UpdateDevice(default));
                }
            }
        }
    }
}

fn probe_denoise(sender: &ComponentSender<AudioInMenuWidgetModel>) {
    sender.oneshot_command(async {
        AudioInMenuWidgetCommandOutput::Denoise(denoise::probe().await, false)
    });
}

fn sync_denoise(model: &AudioInMenuWidgetModel) {
    let st = &model.denoise;
    let desc = if model.denoise_busy {
        "Working…".to_string()
    } else if let Some(hint) = st.hint() {
        hint
    } else if st.enabled {
        match &st.mic {
            Some(m) => format!("RNNoise on {}", short_source(m)),
            None => "RNNoise on".to_string(),
        }
    } else {
        "Filters background noise for calls (RNNoise)".to_string()
    };
    model.denoise_desc.set_label(&desc);
    let sw = &model.denoise_switch;
    sw.block_signal(&model.denoise_signal);
    sw.set_state(st.enabled);
    sw.set_active(st.enabled);
    sw.unblock_signal(&model.denoise_signal);
    sw.set_sensitive(st.available() && !model.denoise_busy);
}

/// Friendly name of the physical source behind the filter: its
/// PipeWire description when known, else the raw node name.
fn short_source(name: &str) -> String {
    audio_service()
        .and_then(|a| {
            a.input_devices
                .get()
                .into_iter()
                .find(|d| d.name.get() == name)
                .map(|d| d.description.get())
        })
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| name.to_string())
}
