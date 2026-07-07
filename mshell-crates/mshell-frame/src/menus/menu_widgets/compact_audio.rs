//! Dashboard "Compact Audio" tile — minimal Volume + Mic sliders
//! in one card.
//!
//!   🔊 Volume               42%
//!   ────────────────━━━━━━━━
//!   🎙 Mic                    5%
//!   ───
//!
//! Replaces the standalone AudioOutput + AudioInput pair in the
//! dashboard's right column when the user wants tighter density.
//! Drops the revealer-row chrome those widgets carry by default
//! and surfaces just the two sliders + percentage readouts.

use mshell_services::audio_service;
use mshell_utils::audio::{
    get_audio_in_icon, get_audio_out_icon, spawn_default_input_watcher,
    spawn_default_output_watcher,
};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, OrientableExt, RangeExt, ScaleExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::sync::Arc;
use wayle_audio::core::device::input::InputDevice;
use wayle_audio::core::device::output::OutputDevice;
use wayle_audio::volume::types::Volume;

pub(crate) struct CompactAudioModel {
    output_device: Option<Arc<OutputDevice>>,
    input_device: Option<Arc<InputDevice>>,
    output_percent: f64,
    input_percent: f64,
    output_icon: String,
    input_icon: String,
    /// Block the slider value-changed handler while we set the
    /// value from a remote update, so we don't bounce it back.
    suppress_output_signal: bool,
    suppress_input_signal: bool,
}

#[derive(Debug)]
pub(crate) enum CompactAudioInput {
    Refresh,
    SetOutputVolume(f64),
    SetInputVolume(f64),
}

#[derive(Debug)]
pub(crate) enum CompactAudioOutput {}

pub(crate) struct CompactAudioInit {}

#[derive(Debug)]
pub(crate) enum CompactAudioCommandOutput {
    OutputChanged,
    InputChanged,
}

#[relm4::component(pub)]
impl Component for CompactAudioModel {
    type CommandOutput = CompactAudioCommandOutput;
    type Input = CompactAudioInput;
    type Output = CompactAudioOutput;
    type Init = CompactAudioInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "compact-audio-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_hexpand: true,
            set_spacing: 8,

            // Card title — matches the Overview / Connectivity tiles
            // so the right column's cards share one titled rhythm.
            gtk::Label {
                add_css_class: "compact-audio-header",
                set_label: "Sound",
                set_halign: gtk::Align::Start,
            },

            // ── Volume row ──────────────────────────────────────
            gtk::Box {
                add_css_class: "compact-audio-row",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 10,

                #[name = "out_icon"]
                gtk::Image {
                    add_css_class: "compact-audio-icon",
                    #[watch]
                    set_icon_name: Some(model.output_icon.as_str()),
                },
                gtk::Label {
                    add_css_class: "compact-audio-caption",
                    set_label: "Volume",
                    set_halign: gtk::Align::Start,
                },
                #[name = "out_scale"]
                gtk::Scale {
                    add_css_class: "compact-audio-slider",
                    set_hexpand: true,
                    set_range: (0.0, 1.0),
                    set_draw_value: false,
                    connect_value_changed[sender] => move |scale| {
                        let v = scale.value();
                        sender.input(CompactAudioInput::SetOutputVolume(v));
                    },
                },
                gtk::Label {
                    add_css_class: "compact-audio-value",
                    #[watch]
                    set_label: &format!("{}%", (model.output_percent * 100.0).round() as i32),
                    set_width_chars: 4,
                    set_xalign: 1.0,
                },
            },

            // ── Mic row ─────────────────────────────────────────
            gtk::Box {
                add_css_class: "compact-audio-row",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 10,

                #[name = "in_icon"]
                gtk::Image {
                    add_css_class: "compact-audio-icon",
                    #[watch]
                    set_icon_name: Some(model.input_icon.as_str()),
                },
                gtk::Label {
                    add_css_class: "compact-audio-caption",
                    set_label: "Mic",
                    set_halign: gtk::Align::Start,
                },
                #[name = "in_scale"]
                gtk::Scale {
                    add_css_class: "compact-audio-slider",
                    set_hexpand: true,
                    set_range: (0.0, 1.0),
                    set_draw_value: false,
                    connect_value_changed[sender] => move |scale| {
                        let v = scale.value();
                        sender.input(CompactAudioInput::SetInputVolume(v));
                    },
                },
                gtk::Label {
                    add_css_class: "compact-audio-value",
                    #[watch]
                    set_label: &format!("{}%", (model.input_percent * 100.0).round() as i32),
                    set_width_chars: 4,
                    set_xalign: 1.0,
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        spawn_default_output_watcher(&sender, None, || CompactAudioCommandOutput::OutputChanged);
        spawn_default_input_watcher(&sender, None, || CompactAudioCommandOutput::InputChanged);

        let output_device = audio_service().default_output.get();
        let input_device = audio_service().default_input.get();

        let output_percent = output_device
            .as_ref()
            .map(|d| d.volume.get().average())
            .unwrap_or(0.0);
        let input_percent = input_device
            .as_ref()
            .map(|d| d.volume.get().average())
            .unwrap_or(0.0);

        let output_icon = output_device
            .as_ref()
            .map(|d| get_audio_out_icon(d).to_string())
            .unwrap_or_else(|| "audio-volume-muted-symbolic".to_string());
        let input_icon = input_device
            .as_ref()
            .map(|d| get_audio_in_icon(d).to_string())
            .unwrap_or_else(|| "microphone-sensitivity-muted-symbolic".to_string());

        let model = CompactAudioModel {
            output_device,
            input_device,
            output_percent,
            input_percent,
            output_icon,
            input_icon,
            suppress_output_signal: false,
            suppress_input_signal: false,
        };

        let widgets = view_output!();

        // Prime sliders without bouncing back into update.
        widgets.out_scale.set_value(model.output_percent);
        widgets.in_scale.set_value(model.input_percent);

        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            CompactAudioCommandOutput::OutputChanged => {
                sender.input(CompactAudioInput::Refresh);
            }
            CompactAudioCommandOutput::InputChanged => {
                sender.input(CompactAudioInput::Refresh);
            }
        }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            CompactAudioInput::Refresh => {
                self.output_device = audio_service().default_output.get();
                self.input_device = audio_service().default_input.get();

                if let Some(d) = &self.output_device {
                    self.output_percent = d.volume.get().average();
                    self.output_icon = get_audio_out_icon(d).to_string();
                    // Block our own change-handler before pushing
                    // the new value into the slider.
                    self.suppress_output_signal = true;
                    widgets.out_scale.set_value(self.output_percent);
                    self.suppress_output_signal = false;
                }
                if let Some(d) = &self.input_device {
                    self.input_percent = d.volume.get().average();
                    self.input_icon = get_audio_in_icon(d).to_string();
                    self.suppress_input_signal = true;
                    widgets.in_scale.set_value(self.input_percent);
                    self.suppress_input_signal = false;
                }
            }
            CompactAudioInput::SetOutputVolume(v) => {
                if self.suppress_output_signal {
                    return;
                }
                // Optimistic local update — the wayle write below
                // is async and the watcher round-trip can take long
                // enough that the % label visibly lags behind the
                // dragging finger. Update the model up front so
                // update_view repaints the label immediately; the
                // watcher will reconcile if the device clamps/maps.
                self.output_percent = v;
                if let Some(d) = &self.output_device {
                    let d = d.clone();
                    glib::spawn_future_local(async move {
                        let _ = d.set_volume(Volume::stereo(v, v)).await;
                    });
                }
            }
            CompactAudioInput::SetInputVolume(v) => {
                if self.suppress_input_signal {
                    return;
                }
                self.input_percent = v;
                if let Some(d) = &self.input_device {
                    let d = d.clone();
                    glib::spawn_future_local(async move {
                        let _ = d.set_volume(Volume::stereo(v, v)).await;
                    });
                }
            }
        }

        self.update_view(widgets, sender);
    }
}
