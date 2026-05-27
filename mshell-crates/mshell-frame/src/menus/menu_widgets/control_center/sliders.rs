//! Control Center sliders row — Volume + Brightness.
//!
//! Two horizontal rows stacked in a vertical box:
//!
//!   🔊 ──────━━━━━━────── 42%
//!   ☀  ──────━━━━━────── 65%
//!
//! The brightness row is hidden when no backlight device is present.
//! Volume uses the 0.0–1.0 scale from compact_audio (mirrors that widget's
//! math exactly).  Brightness uses 0.0–1.0 fraction of the Percentage.

use mshell_services::{audio_service, brightness_service};
use mshell_utils::audio::{get_audio_out_icon, spawn_default_output_watcher};
use mshell_utils::brightness::{get_brightness_icon, spawn_brightness_watcher};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, OrientableExt, RangeExt, ScaleExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::sync::Arc;
use wayle_audio::core::device::output::OutputDevice;
use wayle_audio::volume::types::Volume;
use wayle_brightness::BacklightDevice;
use wayle_brightness::types::Percentage;

// ── Model ─────────────────────────────────────────────────────────────────────

pub(crate) struct ControlCenterSlidersModel {
    // Volume state
    output_device: Option<Arc<OutputDevice>>,
    output_percent: f64, // 0.0–1.0  (mirrors compact_audio convention)
    output_icon: String,
    suppress_output_signal: bool,

    // Brightness state
    brightness_device: Option<Arc<BacklightDevice>>,
    brightness_fraction: f64, // 0.0–1.0
    brightness_icon: String,
    suppress_brightness_signal: bool,
    has_backlight: bool,
}

// ── Messages ──────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub(crate) enum ControlCenterSlidersInput {
    Refresh,
    SetOutputVolume(f64),
    ToggleMute,
    SetBrightness(f64),
}

#[derive(Debug)]
pub(crate) enum ControlCenterSlidersOutput {}

pub(crate) struct ControlCenterSlidersInit {}

#[derive(Debug)]
pub(crate) enum ControlCenterSlidersCommandOutput {
    OutputChanged,
    BrightnessChanged,
}

// ── Component ─────────────────────────────────────────────────────────────────

#[relm4::component(pub(crate))]
impl Component for ControlCenterSlidersModel {
    type CommandOutput = ControlCenterSlidersCommandOutput;
    type Input = ControlCenterSlidersInput;
    type Output = ControlCenterSlidersOutput;
    type Init = ControlCenterSlidersInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "control-center-sliders",
            set_orientation: gtk::Orientation::Vertical,
            set_hexpand: true,
            set_spacing: 8,

            // ── Volume row ────────────────────────────────────────
            gtk::Box {
                add_css_class: "control-center-slider-row",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 10,

                #[name = "vol_icon"]
                gtk::Image {
                    add_css_class: "control-center-slider-icon",
                    #[watch]
                    set_icon_name: Some(model.output_icon.as_str()),
                },
                // The mute-toggle GestureClick is attached to vol_icon in init().

                #[name = "vol_scale"]
                gtk::Scale {
                    add_css_class: "control-center-slider",
                    set_hexpand: true,
                    set_range: (0.0, 1.0),
                    set_draw_value: false,
                    connect_value_changed[sender] => move |scale| {
                        let v = scale.value();
                        sender.input(ControlCenterSlidersInput::SetOutputVolume(v));
                    },
                },
                gtk::Label {
                    add_css_class: "control-center-slider-value",
                    #[watch]
                    set_label: &format!("{}%", (model.output_percent * 100.0).round() as i32),
                    set_width_chars: 4,
                    set_xalign: 1.0,
                },
            },

            // ── Brightness row ────────────────────────────────────
            #[name = "brightness_row"]
            gtk::Box {
                add_css_class: "control-center-slider-row",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 10,
                #[watch]
                set_visible: model.has_backlight,

                gtk::Image {
                    add_css_class: "control-center-slider-icon",
                    #[watch]
                    set_icon_name: Some(model.brightness_icon.as_str()),
                },

                #[name = "bright_scale"]
                gtk::Scale {
                    add_css_class: "control-center-slider",
                    set_hexpand: true,
                    set_range: (0.0, 1.0),
                    set_draw_value: false,
                    connect_value_changed[sender] => move |scale| {
                        let v = scale.value();
                        sender.input(ControlCenterSlidersInput::SetBrightness(v));
                    },
                },
                gtk::Label {
                    add_css_class: "control-center-slider-value",
                    #[watch]
                    set_label: &format!("{}%", (model.brightness_fraction * 100.0).round() as i32),
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
        // Spawn watchers for default audio output and brightness.
        spawn_default_output_watcher(
            &sender,
            None,
            || ControlCenterSlidersCommandOutput::OutputChanged,
        );
        spawn_brightness_watcher(
            &sender,
            || ControlCenterSlidersCommandOutput::BrightnessChanged,
        );

        // Snapshot initial state.
        let output_device = audio_service().default_output.get();
        let output_percent = output_device
            .as_ref()
            .map(|d| d.volume.get().average())
            .unwrap_or(0.0);
        let output_icon = output_device
            .as_ref()
            .map(|d| get_audio_out_icon(d).to_string())
            .unwrap_or_else(|| "audio-volume-muted-symbolic".to_string());

        let has_backlight = brightness_service().is_some();
        let brightness_device = brightness_service()
            .as_ref()
            .and_then(|s| s.primary.get());
        let brightness_fraction = brightness_device
            .as_ref()
            .map(|d| d.percentage().fraction())
            .unwrap_or(0.0);
        let brightness_icon =
            get_brightness_icon(brightness_fraction * 100.0).to_string();

        let model = ControlCenterSlidersModel {
            output_device,
            output_percent,
            output_icon,
            suppress_output_signal: false,
            brightness_device,
            brightness_fraction,
            brightness_icon,
            suppress_brightness_signal: false,
            has_backlight,
        };

        let widgets = view_output!();

        // Prime sliders without triggering change callbacks.
        widgets.vol_scale.set_value(model.output_percent);
        widgets.bright_scale.set_value(model.brightness_fraction);

        // Attach click gesture to volume icon for mute toggle.
        let click = gtk::GestureClick::new();
        let mute_sender = sender.clone();
        click.connect_released(move |_, _, _, _| {
            mute_sender.input(ControlCenterSlidersInput::ToggleMute);
        });
        widgets.vol_icon.add_controller(click);

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
            ControlCenterSlidersCommandOutput::OutputChanged => {
                sender.input(ControlCenterSlidersInput::Refresh);
            }
            ControlCenterSlidersCommandOutput::BrightnessChanged => {
                sender.input(ControlCenterSlidersInput::Refresh);
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
            ControlCenterSlidersInput::Refresh => {
                // Refresh volume.
                self.output_device = audio_service().default_output.get();
                if let Some(d) = &self.output_device {
                    self.output_percent = d.volume.get().average();
                    self.output_icon = get_audio_out_icon(d).to_string();
                    self.suppress_output_signal = true;
                    widgets.vol_scale.set_value(self.output_percent);
                    self.suppress_output_signal = false;
                }

                // Refresh brightness.
                self.brightness_device = brightness_service()
                    .as_ref()
                    .and_then(|s| s.primary.get());
                if let Some(d) = &self.brightness_device {
                    self.brightness_fraction = d.percentage().fraction();
                    self.brightness_icon =
                        get_brightness_icon(self.brightness_fraction * 100.0).to_string();
                    self.suppress_brightness_signal = true;
                    widgets.bright_scale.set_value(self.brightness_fraction);
                    self.suppress_brightness_signal = false;
                }
            }
            ControlCenterSlidersInput::SetOutputVolume(v) => {
                if self.suppress_output_signal {
                    return;
                }
                // Optimistic update so the label tracks the drag immediately.
                self.output_percent = v;
                if let Some(d) = &self.output_device {
                    let d = d.clone();
                    glib::spawn_future_local(async move {
                        let _ = d.set_volume(Volume::stereo(v, v)).await;
                    });
                }
            }
            ControlCenterSlidersInput::ToggleMute => {
                if let Some(d) = &self.output_device {
                    let d = d.clone();
                    let currently_muted = d.muted.get();
                    glib::spawn_future_local(async move {
                        let _ = d.set_mute(!currently_muted).await;
                    });
                }
            }
            ControlCenterSlidersInput::SetBrightness(v) => {
                if self.suppress_brightness_signal {
                    return;
                }
                // Optimistic update.
                self.brightness_fraction = v;
                self.brightness_icon =
                    get_brightness_icon(self.brightness_fraction * 100.0).to_string();
                if let Some(d) = &self.brightness_device {
                    let d = d.clone();
                    glib::spawn_future_local(async move {
                        let _ = d.set_percentage(Percentage::from_fraction(v)).await;
                    });
                }
            }
        }

        self.update_view(widgets, sender);
    }
}
