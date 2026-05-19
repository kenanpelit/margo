//! Audio Dashboard menu widget — the right-pane content for
//! `MenuType::AudioDashboard`. Mirrors the popover content that
//! used to live inside the `audio_dashboard` bar pill, ported
//! into the layer-shell menu surface so it opens contiguous with
//! the bar like ufw/ndns instead of as a standalone xdg_popup.
//!
//! Two stacked cards:
//!   - OUTPUT: mute toggle + volume slider + device list
//!   - INPUT:  mute toggle + volume slider + device list
//!
//! Volume / mute changes flow back through wayle_audio. The
//! device-row checkmark surfaces which sink/source is the active
//! default — click any row to switch defaults.

use mshell_common::WatcherToken;
use mshell_services::audio_service;
use mshell_utils::audio::{
    get_audio_in_icon, get_audio_out_icon, spawn_default_input_watcher,
    spawn_default_output_watcher, spawn_input_device_volume_mute_watcher,
    spawn_input_devices_watcher, spawn_output_device_volume_mute_watcher,
    spawn_output_devices_watcher,
};
use relm4::gtk::glib;
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, OrientableExt, RangeExt, ScaleExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, RelmWidgetExt, gtk};
use std::sync::Arc;
use wayle_audio::core::device::input::InputDevice;
use wayle_audio::core::device::output::OutputDevice;
use wayle_audio::volume::types::Volume;

struct OutputDeviceRow {
    container: gtk::Button,
    check: gtk::Image,
    device: Arc<OutputDevice>,
}

struct InputDeviceRow {
    container: gtk::Button,
    check: gtk::Image,
    device: Arc<InputDevice>,
}

pub(crate) struct AudioDashboardMenuWidgetModel {
    output_device: Option<Arc<OutputDevice>>,
    input_device: Option<Arc<InputDevice>>,
    output_percent: f64,
    input_percent: f64,
    output_muted: bool,
    input_muted: bool,
    output_icon: String,
    input_icon: String,
    suppress_output_signal: bool,
    suppress_input_signal: bool,
    output_watcher_token: WatcherToken,
    input_watcher_token: WatcherToken,
    _output_devices_token: WatcherToken,
    _input_devices_token: WatcherToken,
    output_rows: Vec<OutputDeviceRow>,
    input_rows: Vec<InputDeviceRow>,
}

impl std::fmt::Debug for AudioDashboardMenuWidgetModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioDashboardMenuWidgetModel")
            .field("output_percent", &self.output_percent)
            .field("input_percent", &self.input_percent)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum AudioDashboardMenuWidgetInput {
    DefaultOutputChanged,
    DefaultInputChanged,
    OutputDevicesChanged,
    InputDevicesChanged,
    OutputVolumeOrMuteChanged,
    InputVolumeOrMuteChanged,
    SetOutputVolume(f64),
    SetInputVolume(f64),
    SetOutputDefault(Arc<OutputDevice>),
    SetInputDefault(Arc<InputDevice>),
    ToggleOutputMute,
    ToggleInputMute,
}

#[derive(Debug)]
pub(crate) enum AudioDashboardMenuWidgetOutput {}

pub(crate) struct AudioDashboardMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum AudioDashboardMenuWidgetCommandOutput {
    DefaultOutputChanged,
    DefaultInputChanged,
    OutputDevicesChanged,
    InputDevicesChanged,
    OutputVolumeOrMuteChanged,
    InputVolumeOrMuteChanged,
}

#[relm4::component(pub(crate))]
impl Component for AudioDashboardMenuWidgetModel {
    type CommandOutput = AudioDashboardMenuWidgetCommandOutput;
    type Input = AudioDashboardMenuWidgetInput;
    type Output = AudioDashboardMenuWidgetOutput;
    type Init = AudioDashboardMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "audio-dashboard-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 14,
            set_margin_all: 16,

            // ── Output ──────────────────────────────────────────
            gtk::Label {
                add_css_class: "audio-dashboard-section-label",
                set_label: "OUTPUT",
                set_halign: gtk::Align::Start,
            },
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 10,

                gtk::Button {
                    add_css_class: "audio-dashboard-mute-button",
                    connect_clicked[sender] => move |_| {
                        sender.input(AudioDashboardMenuWidgetInput::ToggleOutputMute);
                    },
                    gtk::Image {
                        add_css_class: "audio-dashboard-icon",
                        #[watch]
                        set_icon_name: Some(model.output_icon.as_str()),
                    },
                },
                #[name = "out_scale"]
                gtk::Scale {
                    add_css_class: "audio-dashboard-slider",
                    set_hexpand: true,
                    set_range: (0.0, 1.0),
                    set_draw_value: false,
                    connect_value_changed[sender] => move |s| {
                        sender.input(AudioDashboardMenuWidgetInput::SetOutputVolume(s.value()));
                    },
                },
                gtk::Label {
                    add_css_class: "audio-dashboard-value",
                    #[watch]
                    set_label: &format!("{}%", (model.output_percent * 100.0).round() as i32),
                    set_width_chars: 5,
                    set_xalign: 1.0,
                },
            },
            #[name = "out_devices"]
            gtk::Box {
                add_css_class: "audio-dashboard-device-list",
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 2,
            },

            // ── Input ───────────────────────────────────────────
            gtk::Label {
                add_css_class: "audio-dashboard-section-label",
                set_label: "INPUT",
                set_halign: gtk::Align::Start,
            },
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 10,

                gtk::Button {
                    add_css_class: "audio-dashboard-mute-button",
                    connect_clicked[sender] => move |_| {
                        sender.input(AudioDashboardMenuWidgetInput::ToggleInputMute);
                    },
                    gtk::Image {
                        add_css_class: "audio-dashboard-icon",
                        #[watch]
                        set_icon_name: Some(model.input_icon.as_str()),
                    },
                },
                #[name = "in_scale"]
                gtk::Scale {
                    add_css_class: "audio-dashboard-slider",
                    set_hexpand: true,
                    set_range: (0.0, 1.0),
                    set_draw_value: false,
                    connect_value_changed[sender] => move |s| {
                        sender.input(AudioDashboardMenuWidgetInput::SetInputVolume(s.value()));
                    },
                },
                gtk::Label {
                    add_css_class: "audio-dashboard-value",
                    #[watch]
                    set_label: &format!("{}%", (model.input_percent * 100.0).round() as i32),
                    set_width_chars: 5,
                    set_xalign: 1.0,
                },
            },
            #[name = "in_devices"]
            gtk::Box {
                add_css_class: "audio-dashboard-device-list",
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 2,
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        spawn_default_output_watcher(&sender, None, || {
            AudioDashboardMenuWidgetCommandOutput::DefaultOutputChanged
        });
        spawn_default_input_watcher(&sender, None, || {
            AudioDashboardMenuWidgetCommandOutput::DefaultInputChanged
        });
        let mut devices_token = WatcherToken::new();
        spawn_output_devices_watcher(&sender, devices_token.reset(), || {
            AudioDashboardMenuWidgetCommandOutput::OutputDevicesChanged
        });
        let mut input_devices_token = WatcherToken::new();
        spawn_input_devices_watcher(&sender, input_devices_token.reset(), || {
            AudioDashboardMenuWidgetCommandOutput::InputDevicesChanged
        });

        let output_device = audio_service().default_output.get();
        let input_device = audio_service().default_input.get();

        let mut output_watcher_token = WatcherToken::new();
        let mut input_watcher_token = WatcherToken::new();

        if let Some(d) = &output_device {
            spawn_output_device_volume_mute_watcher(
                d,
                output_watcher_token.reset(),
                &sender,
                || AudioDashboardMenuWidgetCommandOutput::OutputVolumeOrMuteChanged,
            );
        }
        if let Some(d) = &input_device {
            spawn_input_device_volume_mute_watcher(
                d,
                input_watcher_token.reset(),
                &sender,
                || AudioDashboardMenuWidgetCommandOutput::InputVolumeOrMuteChanged,
            );
        }

        let (output_percent, output_muted, output_icon) =
            read_output_state(&output_device);
        let (input_percent, input_muted, input_icon) =
            read_input_state(&input_device);

        let model = AudioDashboardMenuWidgetModel {
            output_device,
            input_device,
            output_percent,
            input_percent,
            output_muted,
            input_muted,
            output_icon,
            input_icon,
            suppress_output_signal: false,
            suppress_input_signal: false,
            output_watcher_token,
            input_watcher_token,
            _output_devices_token: devices_token,
            _input_devices_token: input_devices_token,
            output_rows: Vec::new(),
            input_rows: Vec::new(),
        };

        let widgets = view_output!();

        widgets.out_scale.set_value(model.output_percent);
        widgets.in_scale.set_value(model.input_percent);

        let _ = root;
        let mut parts = ComponentParts { model, widgets };

        rebuild_output_list(&mut parts.model, &parts.widgets, &sender);
        rebuild_input_list(&mut parts.model, &parts.widgets, &sender);

        parts
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            AudioDashboardMenuWidgetCommandOutput::DefaultOutputChanged => {
                sender.input(AudioDashboardMenuWidgetInput::DefaultOutputChanged);
            }
            AudioDashboardMenuWidgetCommandOutput::DefaultInputChanged => {
                sender.input(AudioDashboardMenuWidgetInput::DefaultInputChanged);
            }
            AudioDashboardMenuWidgetCommandOutput::OutputDevicesChanged => {
                sender.input(AudioDashboardMenuWidgetInput::OutputDevicesChanged);
            }
            AudioDashboardMenuWidgetCommandOutput::InputDevicesChanged => {
                sender.input(AudioDashboardMenuWidgetInput::InputDevicesChanged);
            }
            AudioDashboardMenuWidgetCommandOutput::OutputVolumeOrMuteChanged => {
                sender.input(AudioDashboardMenuWidgetInput::OutputVolumeOrMuteChanged);
            }
            AudioDashboardMenuWidgetCommandOutput::InputVolumeOrMuteChanged => {
                sender.input(AudioDashboardMenuWidgetInput::InputVolumeOrMuteChanged);
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
            AudioDashboardMenuWidgetInput::DefaultOutputChanged => {
                self.output_device = audio_service().default_output.get();
                if let Some(d) = &self.output_device {
                    spawn_output_device_volume_mute_watcher(
                        d,
                        self.output_watcher_token.reset(),
                        &sender,
                        || AudioDashboardMenuWidgetCommandOutput::OutputVolumeOrMuteChanged,
                    );
                }
                let (p, m, i) = read_output_state(&self.output_device);
                self.output_percent = p;
                self.output_muted = m;
                self.output_icon = i;
                self.suppress_output_signal = true;
                widgets.out_scale.set_value(self.output_percent);
                self.suppress_output_signal = false;
                rebuild_output_list(self, widgets, &sender);
            }
            AudioDashboardMenuWidgetInput::DefaultInputChanged => {
                self.input_device = audio_service().default_input.get();
                if let Some(d) = &self.input_device {
                    spawn_input_device_volume_mute_watcher(
                        d,
                        self.input_watcher_token.reset(),
                        &sender,
                        || AudioDashboardMenuWidgetCommandOutput::InputVolumeOrMuteChanged,
                    );
                }
                let (p, m, i) = read_input_state(&self.input_device);
                self.input_percent = p;
                self.input_muted = m;
                self.input_icon = i;
                self.suppress_input_signal = true;
                widgets.in_scale.set_value(self.input_percent);
                self.suppress_input_signal = false;
                rebuild_input_list(self, widgets, &sender);
            }
            AudioDashboardMenuWidgetInput::OutputDevicesChanged => {
                rebuild_output_list(self, widgets, &sender);
            }
            AudioDashboardMenuWidgetInput::InputDevicesChanged => {
                rebuild_input_list(self, widgets, &sender);
            }
            AudioDashboardMenuWidgetInput::OutputVolumeOrMuteChanged => {
                let (p, m, i) = read_output_state(&self.output_device);
                self.output_percent = p;
                self.output_muted = m;
                self.output_icon = i;
                self.suppress_output_signal = true;
                widgets.out_scale.set_value(self.output_percent);
                self.suppress_output_signal = false;
            }
            AudioDashboardMenuWidgetInput::InputVolumeOrMuteChanged => {
                let (p, m, i) = read_input_state(&self.input_device);
                self.input_percent = p;
                self.input_muted = m;
                self.input_icon = i;
                self.suppress_input_signal = true;
                widgets.in_scale.set_value(self.input_percent);
                self.suppress_input_signal = false;
            }
            AudioDashboardMenuWidgetInput::SetOutputVolume(v) => {
                if self.suppress_output_signal {
                    return;
                }
                self.output_percent = v;
                self.output_icon = output_icon_for(v, self.output_muted);
                if let Some(d) = &self.output_device {
                    let d = d.clone();
                    glib::spawn_future_local(async move {
                        let _ = d.set_volume(Volume::stereo(v, v)).await;
                    });
                }
            }
            AudioDashboardMenuWidgetInput::SetInputVolume(v) => {
                if self.suppress_input_signal {
                    return;
                }
                self.input_percent = v;
                self.input_icon = input_icon_for(v, self.input_muted);
                if let Some(d) = &self.input_device {
                    let d = d.clone();
                    glib::spawn_future_local(async move {
                        let _ = d.set_volume(Volume::stereo(v, v)).await;
                    });
                }
            }
            AudioDashboardMenuWidgetInput::SetOutputDefault(device) => {
                glib::spawn_future_local(async move {
                    let _ = device.set_as_default().await;
                });
            }
            AudioDashboardMenuWidgetInput::SetInputDefault(device) => {
                glib::spawn_future_local(async move {
                    let _ = device.set_as_default().await;
                });
            }
            AudioDashboardMenuWidgetInput::ToggleOutputMute => {
                if let Some(d) = &self.output_device {
                    let d = d.clone();
                    let new_muted = !self.output_muted;
                    glib::spawn_future_local(async move {
                        let _ = d.set_mute(new_muted).await;
                    });
                }
            }
            AudioDashboardMenuWidgetInput::ToggleInputMute => {
                if let Some(d) = &self.input_device {
                    let d = d.clone();
                    let new_muted = !self.input_muted;
                    glib::spawn_future_local(async move {
                        let _ = d.set_mute(new_muted).await;
                    });
                }
            }
        }

        refresh_output_checks(self);
        refresh_input_checks(self);
        self.update_view(widgets, sender);
    }
}

// ── State readers ───────────────────────────────────────────────

fn read_output_state(d: &Option<Arc<OutputDevice>>) -> (f64, bool, String) {
    if let Some(device) = d {
        let muted = device.muted.get();
        let percent = device.volume.get().average();
        (percent, muted, get_audio_out_icon(device).to_string())
    } else {
        (0.0, false, "audio-volume-muted-symbolic".to_string())
    }
}

fn read_input_state(d: &Option<Arc<InputDevice>>) -> (f64, bool, String) {
    if let Some(device) = d {
        let muted = device.muted.get();
        let percent = device.volume.get().average();
        (percent, muted, get_audio_in_icon(device).to_string())
    } else {
        (0.0, false, "microphone-sensitivity-muted-symbolic".to_string())
    }
}

fn output_icon_for(percent: f64, muted: bool) -> String {
    if muted {
        return "audio-volume-muted-symbolic".to_string();
    }
    let pct = (percent * 100.0).round() as u16;
    if pct > 66 {
        "audio-volume-high-symbolic"
    } else if pct > 33 {
        "audio-volume-medium-symbolic"
    } else if pct > 0 {
        "audio-volume-low-symbolic"
    } else {
        "audio-volume-muted-symbolic"
    }
    .to_string()
}

fn input_icon_for(percent: f64, muted: bool) -> String {
    if muted {
        return "microphone-sensitivity-muted-symbolic".to_string();
    }
    let pct = (percent * 100.0).round() as u16;
    if pct > 66 {
        "microphone-sensitivity-high-symbolic"
    } else if pct > 33 {
        "microphone-sensitivity-medium-symbolic"
    } else if pct > 0 {
        "microphone-sensitivity-low-symbolic"
    } else {
        "microphone-sensitivity-muted-symbolic"
    }
    .to_string()
}

// ── Device list builders ───────────────────────────────────────

fn rebuild_output_list(
    model: &mut AudioDashboardMenuWidgetModel,
    widgets: &AudioDashboardMenuWidgetModelWidgets,
    sender: &ComponentSender<AudioDashboardMenuWidgetModel>,
) {
    for row in model.output_rows.drain(..) {
        widgets.out_devices.remove(&row.container);
    }
    let devices = audio_service().output_devices.get();
    for device in devices.iter() {
        let row = build_output_row(device.clone(), sender);
        widgets.out_devices.append(&row.container);
        model.output_rows.push(row);
    }
    refresh_output_checks(model);
}

fn rebuild_input_list(
    model: &mut AudioDashboardMenuWidgetModel,
    widgets: &AudioDashboardMenuWidgetModelWidgets,
    sender: &ComponentSender<AudioDashboardMenuWidgetModel>,
) {
    for row in model.input_rows.drain(..) {
        widgets.in_devices.remove(&row.container);
    }
    let devices = audio_service().input_devices.get();
    for device in devices.iter() {
        let row = build_input_row(device.clone(), sender);
        widgets.in_devices.append(&row.container);
        model.input_rows.push(row);
    }
    refresh_input_checks(model);
}

fn build_output_row(
    device: Arc<OutputDevice>,
    sender: &ComponentSender<AudioDashboardMenuWidgetModel>,
) -> OutputDeviceRow {
    let container = gtk::Button::new();
    container.add_css_class("audio-dashboard-device-row");
    let row_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let check = gtk::Image::from_icon_name("check-symbolic");
    check.add_css_class("audio-dashboard-device-check");
    check.set_visible(false);
    let label = gtk::Label::new(Some(&device.description.get()));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    row_box.append(&check);
    row_box.append(&label);
    container.set_child(Some(&row_box));

    let dev = device.clone();
    let sender_clone = sender.clone();
    container.connect_clicked(move |_| {
        sender_clone.input(AudioDashboardMenuWidgetInput::SetOutputDefault(dev.clone()));
    });

    OutputDeviceRow { container, check, device }
}

fn build_input_row(
    device: Arc<InputDevice>,
    sender: &ComponentSender<AudioDashboardMenuWidgetModel>,
) -> InputDeviceRow {
    let container = gtk::Button::new();
    container.add_css_class("audio-dashboard-device-row");
    let row_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let check = gtk::Image::from_icon_name("check-symbolic");
    check.add_css_class("audio-dashboard-device-check");
    check.set_visible(false);
    let label = gtk::Label::new(Some(&device.description.get()));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    row_box.append(&check);
    row_box.append(&label);
    container.set_child(Some(&row_box));

    let dev = device.clone();
    let sender_clone = sender.clone();
    container.connect_clicked(move |_| {
        sender_clone.input(AudioDashboardMenuWidgetInput::SetInputDefault(dev.clone()));
    });

    InputDeviceRow { container, check, device }
}

fn refresh_output_checks(model: &AudioDashboardMenuWidgetModel) {
    let default = audio_service().default_output.get();
    for row in &model.output_rows {
        let is_default = match &default {
            Some(d) => d.eq(&row.device),
            None => false,
        };
        row.check.set_visible(is_default);
    }
}

fn refresh_input_checks(model: &AudioDashboardMenuWidgetModel) {
    let default = audio_service().default_input.get();
    for row in &model.input_rows {
        let is_default = match &default {
            Some(d) => d.eq(&row.device),
            None => false,
        };
        row.check.set_visible(is_default);
    }
}
