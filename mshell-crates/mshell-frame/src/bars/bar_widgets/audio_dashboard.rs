//! Audio Dashboard bar pill — single pill that surfaces both
//! default output (sink) and default input (source) volumes,
//! plus a click-to-open Popover with sliders + device pickers.
//!
//! Bar cluster:
//!
//!   🔊 42%  ·  🎙 5%
//!
//! Right-click cycles the visible slots:
//!
//!   Both → OutputOnly → InputOnly → Both → …
//!
//! Click pops a panel with:
//!   - Output slider (0..1, drag to change volume) + percent + mute
//!   - Output device list (click a row to make it default)
//!   - Input slider + percent + mute
//!   - Input device list
//!
//! Reuses the same wayle_audio subscriptions the standalone
//! AudioOutput / AudioInput menu widgets and the CompactAudio
//! menu tile use — no extra data plumbing.

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
    BoxExt, ButtonExt, GestureSingleExt, OrientableExt, PopoverExt, RangeExt, ScaleExt,
    WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, RelmWidgetExt, gtk};
use std::sync::Arc;
use wayle_audio::core::device::input::InputDevice;
use wayle_audio::core::device::output::OutputDevice;
use wayle_audio::volume::types::Volume;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DisplayMode {
    Both,
    OutputOnly,
    InputOnly,
}

impl DisplayMode {
    fn next(self) -> Self {
        match self {
            DisplayMode::Both => DisplayMode::OutputOnly,
            DisplayMode::OutputOnly => DisplayMode::InputOnly,
            DisplayMode::InputOnly => DisplayMode::Both,
        }
    }
}

/// One row inside an audio device list — owns its widget so we
/// can refresh the checkmark + label when the default device
/// changes without rebuilding the whole list.
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

pub(crate) struct AudioDashboardModel {
    output_device: Option<Arc<OutputDevice>>,
    input_device: Option<Arc<InputDevice>>,
    output_percent: f64,
    input_percent: f64,
    output_muted: bool,
    input_muted: bool,
    output_icon: String,
    input_icon: String,
    mode: DisplayMode,
    /// Suppress the slider value-changed handler while we push a
    /// remote update — same anti-bounce trick the CompactAudio
    /// menu widget uses.
    suppress_output_signal: bool,
    suppress_input_signal: bool,
    /// Cancels per-device watchers when the default device flips
    /// to a different device so the old volume watcher stops
    /// firing into our channel.
    output_watcher_token: WatcherToken,
    input_watcher_token: WatcherToken,
    /// Device-list watcher tokens kept alive for the widget's
    /// lifetime — drop cancels the watchers automatically. Two
    /// separate tokens because each spawn needs its own.
    _output_devices_token: WatcherToken,
    _input_devices_token: WatcherToken,
    /// Lazily-grown rows in the popover device pickers.
    output_rows: Vec<OutputDeviceRow>,
    input_rows: Vec<InputDeviceRow>,
}

#[derive(Debug)]
pub(crate) enum AudioDashboardInput {
    ToggleMenu,
    CycleMode,
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
pub(crate) enum AudioDashboardOutput {}

pub(crate) struct AudioDashboardInit {}

#[derive(Debug)]
pub(crate) enum AudioDashboardCommandOutput {
    DefaultOutputChanged,
    DefaultInputChanged,
    OutputDevicesChanged,
    InputDevicesChanged,
    OutputVolumeOrMuteChanged,
    InputVolumeOrMuteChanged,
}

#[relm4::component(pub)]
impl Component for AudioDashboardModel {
    type CommandOutput = AudioDashboardCommandOutput;
    type Input = AudioDashboardInput;
    type Output = AudioDashboardOutput;
    type Init = AudioDashboardInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["ok-button-surface", "ok-bar-widget", "audio-dashboard-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,
            set_has_tooltip: true,

            #[name = "button"]
            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| {
                    sender.input(AudioDashboardInput::ToggleMenu);
                },

                gtk::Box {
                    add_css_class: "audio-dashboard-bar-cluster",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 6,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

                    // ── Output slot ─────────────────────────────
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 4,
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        #[watch]
                        set_visible: matches!(
                            model.mode,
                            DisplayMode::Both | DisplayMode::OutputOnly,
                        ) && model.output_device.is_some(),

                        gtk::Image {
                            add_css_class: "audio-dashboard-icon",
                            #[watch]
                            set_icon_name: Some(model.output_icon.as_str()),
                        },
                        gtk::Label {
                            add_css_class: "audio-dashboard-bar-label",
                            #[watch]
                            set_label: &format!("{}%", (model.output_percent * 100.0).round() as i32),
                        },
                    },

                    gtk::Label {
                        add_css_class: "audio-dashboard-bar-sep",
                        set_label: "·",
                        #[watch]
                        set_visible: matches!(model.mode, DisplayMode::Both)
                            && model.output_device.is_some()
                            && model.input_device.is_some(),
                    },

                    // ── Input slot ──────────────────────────────
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 4,
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        #[watch]
                        set_visible: matches!(
                            model.mode,
                            DisplayMode::Both | DisplayMode::InputOnly,
                        ) && model.input_device.is_some(),

                        gtk::Image {
                            add_css_class: "audio-dashboard-icon",
                            #[watch]
                            set_icon_name: Some(model.input_icon.as_str()),
                        },
                        gtk::Label {
                            add_css_class: "audio-dashboard-bar-label",
                            #[watch]
                            set_label: &format!("{}%", (model.input_percent * 100.0).round() as i32),
                        },
                    },
                },
            },

            // ── Popover ────────────────────────────────────────
            #[name = "popover"]
            gtk::Popover {
                set_position: gtk::PositionType::Bottom,
                set_has_arrow: false,
                set_autohide: true,
                add_css_class: "audio-dashboard-menu",

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 14,
                    set_margin_all: 16,
                    set_width_request: 380,

                    // ── Output section ──────────────────────────
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
                                sender.input(AudioDashboardInput::ToggleOutputMute);
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
                                sender.input(AudioDashboardInput::SetOutputVolume(s.value()));
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

                    // ── Input section ──────────────────────────
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
                                sender.input(AudioDashboardInput::ToggleInputMute);
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
                                sender.input(AudioDashboardInput::SetInputVolume(s.value()));
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
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Top-level: default device + device-list watchers. The
        // default watchers don't need a cancellation token (they
        // live for the widget's lifetime); the device-list ones
        // do, so create a one-shot token now and never reset.
        spawn_default_output_watcher(&sender, None, || {
            AudioDashboardCommandOutput::DefaultOutputChanged
        });
        spawn_default_input_watcher(&sender, None, || {
            AudioDashboardCommandOutput::DefaultInputChanged
        });
        // device-list watchers want a CancellationToken — reset()
        // returns one and stores its sibling inside the token so
        // dropping the WatcherToken on widget shutdown cancels.
        let mut devices_token = WatcherToken::new();
        spawn_output_devices_watcher(&sender, devices_token.reset(), || {
            AudioDashboardCommandOutput::OutputDevicesChanged
        });
        // Reset a second time would cancel the output-list watcher;
        // use a SEPARATE token for the input-list watcher instead.
        let mut input_devices_token = WatcherToken::new();
        spawn_input_devices_watcher(&sender, input_devices_token.reset(), || {
            AudioDashboardCommandOutput::InputDevicesChanged
        });

        let output_device = audio_service().default_output.get();
        let input_device = audio_service().default_input.get();

        let mut output_watcher_token = WatcherToken::new();
        let mut input_watcher_token = WatcherToken::new();

        // Per-device volume/mute watchers — re-spawned every time
        // the default device flips (see update_cmd handlers).
        if let Some(d) = &output_device {
            spawn_output_device_volume_mute_watcher(
                d,
                output_watcher_token.reset(),
                &sender,
                || AudioDashboardCommandOutput::OutputVolumeOrMuteChanged,
            );
        }
        if let Some(d) = &input_device {
            spawn_input_device_volume_mute_watcher(
                d,
                input_watcher_token.reset(),
                &sender,
                || AudioDashboardCommandOutput::InputVolumeOrMuteChanged,
            );
        }

        let (output_percent, output_muted, output_icon) =
            read_output_state(&output_device);
        let (input_percent, input_muted, input_icon) =
            read_input_state(&input_device);

        let model = AudioDashboardModel {
            output_device,
            input_device,
            output_percent,
            input_percent,
            output_muted,
            input_muted,
            output_icon,
            input_icon,
            mode: DisplayMode::Both,
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

        // Prime sliders + populate device lists once.
        widgets.out_scale.set_value(model.output_percent);
        widgets.in_scale.set_value(model.input_percent);

        // Right-click on the pill cycles display mode (Both /
        // OutputOnly / InputOnly).
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
        let sender_clone = sender.clone();
        gesture.connect_pressed(move |_, _, _, _| {
            sender_clone.input(AudioDashboardInput::CycleMode);
        });
        widgets.button.add_controller(gesture);

        let _ = root;
        let mut parts = ComponentParts { model, widgets };

        // Populate device lists post-construction (need access to
        // both model and widgets at the same time).
        rebuild_output_list(&mut parts.model, &parts.widgets, &sender);
        rebuild_input_list(&mut parts.model, &parts.widgets, &sender);
        apply_tooltip(&parts.model, &parts.widgets);

        parts
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            AudioDashboardCommandOutput::DefaultOutputChanged => {
                sender.input(AudioDashboardInput::DefaultOutputChanged);
            }
            AudioDashboardCommandOutput::DefaultInputChanged => {
                sender.input(AudioDashboardInput::DefaultInputChanged);
            }
            AudioDashboardCommandOutput::OutputDevicesChanged => {
                sender.input(AudioDashboardInput::OutputDevicesChanged);
            }
            AudioDashboardCommandOutput::InputDevicesChanged => {
                sender.input(AudioDashboardInput::InputDevicesChanged);
            }
            AudioDashboardCommandOutput::OutputVolumeOrMuteChanged => {
                sender.input(AudioDashboardInput::OutputVolumeOrMuteChanged);
            }
            AudioDashboardCommandOutput::InputVolumeOrMuteChanged => {
                sender.input(AudioDashboardInput::InputVolumeOrMuteChanged);
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
            AudioDashboardInput::ToggleMenu => {
                if widgets.popover.is_visible() {
                    widgets.popover.popdown();
                } else {
                    widgets.popover.popup();
                }
            }
            AudioDashboardInput::CycleMode => {
                self.mode = self.mode.next();
            }
            AudioDashboardInput::DefaultOutputChanged => {
                self.output_device = audio_service().default_output.get();
                // Re-spawn the per-device watcher against the new
                // active output.
                if let Some(d) = &self.output_device {
                    spawn_output_device_volume_mute_watcher(
                        d,
                        self.output_watcher_token.reset(),
                        &sender,
                        || AudioDashboardCommandOutput::OutputVolumeOrMuteChanged,
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
            AudioDashboardInput::DefaultInputChanged => {
                self.input_device = audio_service().default_input.get();
                if let Some(d) = &self.input_device {
                    spawn_input_device_volume_mute_watcher(
                        d,
                        self.input_watcher_token.reset(),
                        &sender,
                        || AudioDashboardCommandOutput::InputVolumeOrMuteChanged,
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
            AudioDashboardInput::OutputDevicesChanged => {
                rebuild_output_list(self, widgets, &sender);
            }
            AudioDashboardInput::InputDevicesChanged => {
                rebuild_input_list(self, widgets, &sender);
            }
            AudioDashboardInput::OutputVolumeOrMuteChanged => {
                let (p, m, i) = read_output_state(&self.output_device);
                self.output_percent = p;
                self.output_muted = m;
                self.output_icon = i;
                self.suppress_output_signal = true;
                widgets.out_scale.set_value(self.output_percent);
                self.suppress_output_signal = false;
            }
            AudioDashboardInput::InputVolumeOrMuteChanged => {
                let (p, m, i) = read_input_state(&self.input_device);
                self.input_percent = p;
                self.input_muted = m;
                self.input_icon = i;
                self.suppress_input_signal = true;
                widgets.in_scale.set_value(self.input_percent);
                self.suppress_input_signal = false;
            }
            AudioDashboardInput::SetOutputVolume(v) => {
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
            AudioDashboardInput::SetInputVolume(v) => {
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
            AudioDashboardInput::SetOutputDefault(device) => {
                glib::spawn_future_local(async move {
                    let _ = device.set_as_default().await;
                });
            }
            AudioDashboardInput::SetInputDefault(device) => {
                glib::spawn_future_local(async move {
                    let _ = device.set_as_default().await;
                });
            }
            AudioDashboardInput::ToggleOutputMute => {
                if let Some(d) = &self.output_device {
                    let d = d.clone();
                    let new_muted = !self.output_muted;
                    glib::spawn_future_local(async move {
                        let _ = d.set_mute(new_muted).await;
                    });
                }
            }
            AudioDashboardInput::ToggleInputMute => {
                if let Some(d) = &self.input_device {
                    let d = d.clone();
                    let new_muted = !self.input_muted;
                    glib::spawn_future_local(async move {
                        let _ = d.set_mute(new_muted).await;
                    });
                }
            }
        }

        // Refresh the default-device checkmark on every list
        // mutation so a click on a row immediately reflects.
        refresh_output_checks(self);
        refresh_input_checks(self);
        apply_tooltip(self, widgets);
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
    model: &mut AudioDashboardModel,
    widgets: &AudioDashboardModelWidgets,
    sender: &ComponentSender<AudioDashboardModel>,
) {
    // Drop existing rows.
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
    model: &mut AudioDashboardModel,
    widgets: &AudioDashboardModelWidgets,
    sender: &ComponentSender<AudioDashboardModel>,
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
    sender: &ComponentSender<AudioDashboardModel>,
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
        sender_clone.input(AudioDashboardInput::SetOutputDefault(dev.clone()));
    });

    OutputDeviceRow { container, check, device }
}

fn build_input_row(
    device: Arc<InputDevice>,
    sender: &ComponentSender<AudioDashboardModel>,
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
        sender_clone.input(AudioDashboardInput::SetInputDefault(dev.clone()));
    });

    InputDeviceRow { container, check, device }
}

fn refresh_output_checks(model: &AudioDashboardModel) {
    let default = audio_service().default_output.get();
    for row in &model.output_rows {
        let is_default = match &default {
            Some(d) => d.eq(&row.device),
            None => false,
        };
        row.check.set_visible(is_default);
    }
}

fn refresh_input_checks(model: &AudioDashboardModel) {
    let default = audio_service().default_input.get();
    for row in &model.input_rows {
        let is_default = match &default {
            Some(d) => d.eq(&row.device),
            None => false,
        };
        row.check.set_visible(is_default);
    }
}

// ── Tooltip ────────────────────────────────────────────────────

fn apply_tooltip(model: &AudioDashboardModel, widgets: &AudioDashboardModelWidgets) {
    let out_line = if let Some(d) = &model.output_device {
        format!(
            "Output: {} ({}%{})",
            d.description.get(),
            (model.output_percent * 100.0).round() as i32,
            if model.output_muted { ", muted" } else { "" },
        )
    } else {
        "Output: none".to_string()
    };
    let in_line = if let Some(d) = &model.input_device {
        format!(
            "Input: {} ({}%{})",
            d.description.get(),
            (model.input_percent * 100.0).round() as i32,
            if model.input_muted { ", muted" } else { "" },
        )
    } else {
        "Input: none".to_string()
    };
    let tooltip = format!(
        "{out_line}\n{in_line}\n\nClick: open mixer\nRight-click: cycle display mode"
    );
    widgets
        .button
        .parent()
        .map(|p| p.set_tooltip_text(Some(&tooltip)));
}
