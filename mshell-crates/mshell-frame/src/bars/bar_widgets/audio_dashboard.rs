//! Audio Dashboard bar pill — render-only chip surfacing both
//! default output (sink) and default input (source) volumes.
//!
//! Bar cluster:
//!
//!   🔊 42%  ·  🎙 5%
//!
//! Interactions:
//!   - Left-click  → emits Clicked; frame opens MenuType::AudioDashboard
//!     (the layer-shell menu that hosts sliders + device pickers)
//!   - Right-click → cycles visible slots: Both / OutputOnly / InputOnly
//!   - Scroll      → adjusts the default output volume in ±5 % steps
//!
//! Scroll handling targets the OUTPUT device specifically (the most
//! common volume action); use the menu for input/mic adjustments.

use mshell_services::audio_service;
use mshell_utils::audio::{
    get_audio_in_icon, get_audio_out_icon, spawn_default_input_watcher,
    spawn_default_output_watcher, spawn_input_device_volume_mute_watcher,
    spawn_output_device_volume_mute_watcher,
};
use mshell_common::WatcherToken;
use relm4::gtk::glib;
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, GestureSingleExt, OrientableExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
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
    output_watcher_token: WatcherToken,
    input_watcher_token: WatcherToken,
}

#[derive(Debug)]
pub(crate) enum AudioDashboardInput {
    Clicked,
    CycleMode,
    ScrollOutputVolume(f64),
    DefaultOutputChanged,
    DefaultInputChanged,
    OutputVolumeOrMuteChanged,
    InputVolumeOrMuteChanged,
}

#[derive(Debug)]
pub(crate) enum AudioDashboardOutput {
    /// The pill was left-clicked. Frame opens MenuType::AudioDashboard.
    Clicked,
}

pub(crate) struct AudioDashboardInit {}

#[derive(Debug)]
pub(crate) enum AudioDashboardCommandOutput {
    DefaultOutputChanged,
    DefaultInputChanged,
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
                    sender.input(AudioDashboardInput::Clicked);
                },

                gtk::Box {
                    add_css_class: "audio-dashboard-bar-cluster",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 6,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

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
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        spawn_default_output_watcher(&sender, None, || {
            AudioDashboardCommandOutput::DefaultOutputChanged
        });
        spawn_default_input_watcher(&sender, None, || {
            AudioDashboardCommandOutput::DefaultInputChanged
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

        let (output_percent, output_muted, output_icon) = read_output_state(&output_device);
        let (input_percent, input_muted, input_icon) = read_input_state(&input_device);

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
            output_watcher_token,
            input_watcher_token,
        };

        let widgets = view_output!();

        // Right-click → cycle display mode.
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
        let sender_clone = sender.clone();
        gesture.connect_pressed(move |_, _, _, _| {
            sender_clone.input(AudioDashboardInput::CycleMode);
        });
        widgets.button.add_controller(gesture);

        // Scroll wheel → adjust output volume in 5 % steps. Two-
        // finger scroll on trackpads emits the same events.
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let sender_clone = sender.clone();
        scroll.connect_scroll(move |_, _dx, dy| {
            // dy > 0 means scroll DOWN — reduce volume; dy < 0
            // scroll UP, raise volume. 0.05 step matches what
            // most desktop audio toolbars use.
            let delta = -dy * 0.05;
            sender_clone.input(AudioDashboardInput::ScrollOutputVolume(delta));
            glib::Propagation::Stop
        });
        widgets.button.add_controller(scroll);

        apply_tooltip(&model, &root);
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
            AudioDashboardCommandOutput::DefaultOutputChanged => {
                sender.input(AudioDashboardInput::DefaultOutputChanged);
            }
            AudioDashboardCommandOutput::DefaultInputChanged => {
                sender.input(AudioDashboardInput::DefaultInputChanged);
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
        root: &Self::Root,
    ) {
        match message {
            AudioDashboardInput::Clicked => {
                let _ = sender.output(AudioDashboardOutput::Clicked);
            }
            AudioDashboardInput::CycleMode => {
                self.mode = self.mode.next();
            }
            AudioDashboardInput::ScrollOutputVolume(delta) => {
                if let Some(d) = &self.output_device {
                    let new_v = (self.output_percent + delta).clamp(0.0, 1.0);
                    self.output_percent = new_v;
                    self.output_icon = output_icon_for(new_v, self.output_muted);
                    let d = d.clone();
                    glib::spawn_future_local(async move {
                        let _ = d.set_volume(Volume::stereo(new_v, new_v)).await;
                    });
                }
            }
            AudioDashboardInput::DefaultOutputChanged => {
                self.output_device = audio_service().default_output.get();
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
            }
            AudioDashboardInput::OutputVolumeOrMuteChanged => {
                let (p, m, i) = read_output_state(&self.output_device);
                self.output_percent = p;
                self.output_muted = m;
                self.output_icon = i;
            }
            AudioDashboardInput::InputVolumeOrMuteChanged => {
                let (p, m, i) = read_input_state(&self.input_device);
                self.input_percent = p;
                self.input_muted = m;
                self.input_icon = i;
            }
        }

        apply_tooltip(self, root);
        self.update_view(widgets, sender);
    }
}

fn read_output_state(d: &Option<Arc<OutputDevice>>) -> (f64, bool, String) {
    if let Some(device) = d {
        (
            device.volume.get().average(),
            device.muted.get(),
            get_audio_out_icon(device).to_string(),
        )
    } else {
        (0.0, false, "audio-volume-muted-symbolic".to_string())
    }
}

fn read_input_state(d: &Option<Arc<InputDevice>>) -> (f64, bool, String) {
    if let Some(device) = d {
        (
            device.volume.get().average(),
            device.muted.get(),
            get_audio_in_icon(device).to_string(),
        )
    } else {
        (0.0, false, "microphone-sensitivity-muted-symbolic".to_string())
    }
}

fn output_icon_for(percent: f64, muted: bool) -> String {
    if muted {
        return "audio-volume-muted-symbolic".to_string();
    }
    let pct = (percent * 100.0).round() as u16;
    if pct > 66 { "audio-volume-high-symbolic" }
    else if pct > 33 { "audio-volume-medium-symbolic" }
    else if pct > 0 { "audio-volume-low-symbolic" }
    else { "audio-volume-muted-symbolic" }.to_string()
}

fn apply_tooltip(model: &AudioDashboardModel, root: &gtk::Box) {
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
    root.set_tooltip_text(Some(&format!(
        "{out_line}\n{in_line}\n\nClick: open mixer\nScroll: ± volume\nRight-click: cycle display"
    )));
}
