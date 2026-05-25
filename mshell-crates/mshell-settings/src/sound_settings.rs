//! Settings → Sound.
//!
//! Output + input device selection, volume, and mute — backed by the same
//! reactive `wayle_audio` service the bar's Audio Dashboard uses. The
//! controls are built imperatively and re-synced from the service via
//! watchers (default-device, device-list, and per-device volume/mute), with
//! their signal handlers blocked during programmatic updates so the live
//! refresh never feeds back into a write loop.

use crate::row::Row;
use mshell_common::WatcherToken;
use mshell_services::audio_service;
use mshell_utils::audio::{
    spawn_default_input_watcher, spawn_default_output_watcher,
    spawn_input_device_volume_mute_watcher, spawn_input_devices_watcher,
    spawn_output_device_volume_mute_watcher, spawn_output_devices_watcher,
};
use relm4::gtk::glib::SignalHandlerId;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::sync::Arc;
use wayle_audio::core::device::input::InputDevice;
use wayle_audio::core::device::output::OutputDevice;
use wayle_audio::volume::types::Volume;

pub(crate) struct SoundSettingsModel {
    out_devices: Vec<Arc<OutputDevice>>,
    in_devices: Vec<Arc<InputDevice>>,
    out_dd: gtk::DropDown,
    out_dd_handler: SignalHandlerId,
    out_model: gtk::StringList,
    out_scale: gtk::Scale,
    out_scale_handler: SignalHandlerId,
    out_mute: gtk::Switch,
    out_mute_handler: SignalHandlerId,
    in_dd: gtk::DropDown,
    in_dd_handler: SignalHandlerId,
    in_model: gtk::StringList,
    in_scale: gtk::Scale,
    in_scale_handler: SignalHandlerId,
    in_mute: gtk::Switch,
    in_mute_handler: SignalHandlerId,
    /// Cancels the per-device volume/mute watcher when the default switches.
    out_vol_token: WatcherToken,
    in_vol_token: WatcherToken,
    _out_devices_token: WatcherToken,
    _in_devices_token: WatcherToken,
}

impl std::fmt::Debug for SoundSettingsModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SoundSettingsModel").finish()
    }
}

#[derive(Debug)]
pub(crate) enum SoundSettingsInput {
    SetOutputDevice(u32),
    SetOutputVolume(f64),
    SetOutputMute(bool),
    SetInputDevice(u32),
    SetInputVolume(f64),
    SetInputMute(bool),
}

#[derive(Debug)]
pub(crate) enum SoundSettingsOutput {}

pub(crate) struct SoundSettingsInit {}

#[derive(Debug)]
pub(crate) enum SoundSettingsCommandOutput {
    OutDefaultChanged,
    OutVolMuteChanged,
    OutDevicesChanged,
    InDefaultChanged,
    InVolMuteChanged,
    InDevicesChanged,
}

#[relm4::component(pub)]
impl Component for SoundSettingsModel {
    type CommandOutput = SoundSettingsCommandOutput;
    type Input = SoundSettingsInput;
    type Output = SoundSettingsOutput;
    type Init = SoundSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_propagate_natural_height: false,
            set_propagate_natural_width: false,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                gtk::Box {
                    add_css_class: "settings-hero",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Start,
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("audio-volume-high-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Sound",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Output + input device, volume, and mute. Live with the rest of the system.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Output",
                    set_halign: gtk::Align::Start,
                },
                #[template]
                Row {
                    #[template_child] title { set_label: "Device" },
                    #[template_child] desc { set_label: "Where sound plays." },
                    #[local_ref] out_dd_w -> gtk::DropDown {},
                },
                #[template]
                Row {
                    #[template_child] title { set_label: "Volume" },
                    #[template_child] desc { set_label: "Output level." },
                    #[local_ref] out_scale_w -> gtk::Scale {},
                },
                #[template]
                Row {
                    #[template_child] title { set_label: "Mute" },
                    #[template_child] desc { set_label: "Silence the output." },
                    #[local_ref] out_mute_w -> gtk::Switch {},
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Input",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 12,
                },
                #[template]
                Row {
                    #[template_child] title { set_label: "Device" },
                    #[template_child] desc { set_label: "Microphone / capture source." },
                    #[local_ref] in_dd_w -> gtk::DropDown {},
                },
                #[template]
                Row {
                    #[template_child] title { set_label: "Volume" },
                    #[template_child] desc { set_label: "Input level." },
                    #[local_ref] in_scale_w -> gtk::Scale {},
                },
                #[template]
                Row {
                    #[template_child] title { set_label: "Mute" },
                    #[template_child] desc { set_label: "Silence the input." },
                    #[local_ref] in_mute_w -> gtk::Switch {},
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let svc = audio_service();
        let out_devices = svc.output_devices.get();
        let in_devices = svc.input_devices.get();
        let default_out = svc.default_output.get();
        let default_in = svc.default_input.get();

        // ── Output controls ──
        let out_model = string_list(out_devices.iter().map(|d| d.name.get()));
        let out_dd = gtk::DropDown::builder().model(&out_model).build();
        out_dd.set_width_request(240);
        out_dd.set_valign(gtk::Align::Center);
        if let Some(i) = default_out
            .as_ref()
            .and_then(|d| device_index(&out_devices, &d.name.get()))
        {
            out_dd.set_selected(i);
        }
        let out_dd_handler = out_dd.connect_selected_notify({
            let s = sender.clone();
            move |dd| s.input(SoundSettingsInput::SetOutputDevice(dd.selected()))
        });

        let out_scale = volume_scale();
        out_scale.set_value(default_out.as_ref().map(|d| d.volume.get().average()).unwrap_or(0.0));
        let out_scale_handler = out_scale.connect_value_changed({
            let s = sender.clone();
            move |sc| s.input(SoundSettingsInput::SetOutputVolume(sc.value()))
        });

        let out_mute = gtk::Switch::builder().valign(gtk::Align::Center).build();
        out_mute.set_active(default_out.as_ref().map(|d| d.muted.get()).unwrap_or(false));
        let out_mute_handler = out_mute.connect_active_notify({
            let s = sender.clone();
            move |sw| s.input(SoundSettingsInput::SetOutputMute(sw.is_active()))
        });

        // ── Input controls ──
        let in_model = string_list(in_devices.iter().map(|d| d.name.get()));
        let in_dd = gtk::DropDown::builder().model(&in_model).build();
        in_dd.set_width_request(240);
        in_dd.set_valign(gtk::Align::Center);
        if let Some(i) = default_in
            .as_ref()
            .and_then(|d| device_index_in(&in_devices, &d.name.get()))
        {
            in_dd.set_selected(i);
        }
        let in_dd_handler = in_dd.connect_selected_notify({
            let s = sender.clone();
            move |dd| s.input(SoundSettingsInput::SetInputDevice(dd.selected()))
        });

        let in_scale = volume_scale();
        in_scale.set_value(default_in.as_ref().map(|d| d.volume.get().average()).unwrap_or(0.0));
        let in_scale_handler = in_scale.connect_value_changed({
            let s = sender.clone();
            move |sc| s.input(SoundSettingsInput::SetInputVolume(sc.value()))
        });

        let in_mute = gtk::Switch::builder().valign(gtk::Align::Center).build();
        in_mute.set_active(default_in.as_ref().map(|d| d.muted.get()).unwrap_or(false));
        let in_mute_handler = in_mute.connect_active_notify({
            let s = sender.clone();
            move |sw| s.input(SoundSettingsInput::SetInputMute(sw.is_active()))
        });

        // ── Watchers: default device, device list, and the active device's
        //    volume/mute (re-pointed when the default changes). ──
        let mut out_vol_token = WatcherToken::new();
        let mut in_vol_token = WatcherToken::new();
        let mut out_devices_token = WatcherToken::new();
        let mut in_devices_token = WatcherToken::new();
        spawn_default_output_watcher(&sender, None, || {
            SoundSettingsCommandOutput::OutDefaultChanged
        });
        spawn_default_input_watcher(&sender, None, || {
            SoundSettingsCommandOutput::InDefaultChanged
        });
        spawn_output_devices_watcher(&sender, out_devices_token.reset(), || {
            SoundSettingsCommandOutput::OutDevicesChanged
        });
        spawn_input_devices_watcher(&sender, in_devices_token.reset(), || {
            SoundSettingsCommandOutput::InDevicesChanged
        });
        if let Some(d) = &default_out {
            spawn_output_device_volume_mute_watcher(d, out_vol_token.reset(), &sender, || {
                SoundSettingsCommandOutput::OutVolMuteChanged
            });
        }
        if let Some(d) = &default_in {
            spawn_input_device_volume_mute_watcher(d, in_vol_token.reset(), &sender, || {
                SoundSettingsCommandOutput::InVolMuteChanged
            });
        }

        let model = SoundSettingsModel {
            out_devices,
            in_devices,
            out_dd: out_dd.clone(),
            out_dd_handler,
            out_model,
            out_scale: out_scale.clone(),
            out_scale_handler,
            out_mute: out_mute.clone(),
            out_mute_handler,
            in_dd: in_dd.clone(),
            in_dd_handler,
            in_model,
            in_scale: in_scale.clone(),
            in_scale_handler,
            in_mute: in_mute.clone(),
            in_mute_handler,
            out_vol_token,
            in_vol_token,
            _out_devices_token: out_devices_token,
            _in_devices_token: in_devices_token,
        };

        let out_dd_w = &out_dd;
        let out_scale_w = &out_scale;
        let out_mute_w = &out_mute;
        let in_dd_w = &in_dd;
        let in_scale_w = &in_scale;
        let in_mute_w = &in_mute;
        let widgets = view_output!();
        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            SoundSettingsInput::SetOutputDevice(idx) => {
                if let Some(d) = self.out_devices.get(idx as usize).cloned() {
                    tokio::spawn(async move {
                        let _ = d.set_as_default().await;
                    });
                }
            }
            SoundSettingsInput::SetOutputVolume(v) => {
                if let Some(d) = audio_service().default_output.get() {
                    tokio::spawn(async move {
                        let _ = d.set_volume(Volume::stereo(v, v)).await;
                    });
                }
            }
            SoundSettingsInput::SetOutputMute(m) => {
                if let Some(d) = audio_service().default_output.get() {
                    tokio::spawn(async move {
                        let _ = d.set_mute(m).await;
                    });
                }
            }
            SoundSettingsInput::SetInputDevice(idx) => {
                if let Some(d) = self.in_devices.get(idx as usize).cloned() {
                    tokio::spawn(async move {
                        let _ = d.set_as_default().await;
                    });
                }
            }
            SoundSettingsInput::SetInputVolume(v) => {
                if let Some(d) = audio_service().default_input.get() {
                    tokio::spawn(async move {
                        let _ = d.set_volume(Volume::stereo(v, v)).await;
                    });
                }
            }
            SoundSettingsInput::SetInputMute(m) => {
                if let Some(d) = audio_service().default_input.get() {
                    tokio::spawn(async move {
                        let _ = d.set_mute(m).await;
                    });
                }
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
            SoundSettingsCommandOutput::OutDefaultChanged => {
                if let Some(d) = audio_service().default_output.get() {
                    let token = self.out_vol_token.reset();
                    spawn_output_device_volume_mute_watcher(&d, token, &sender, || {
                        SoundSettingsCommandOutput::OutVolMuteChanged
                    });
                }
                self.sync_output();
            }
            SoundSettingsCommandOutput::OutVolMuteChanged => self.sync_output(),
            SoundSettingsCommandOutput::OutDevicesChanged => {
                self.out_devices = audio_service().output_devices.get();
                splice(&self.out_model, self.out_devices.iter().map(|d| d.name.get()));
                self.sync_output();
            }
            SoundSettingsCommandOutput::InDefaultChanged => {
                if let Some(d) = audio_service().default_input.get() {
                    let token = self.in_vol_token.reset();
                    spawn_input_device_volume_mute_watcher(&d, token, &sender, || {
                        SoundSettingsCommandOutput::InVolMuteChanged
                    });
                }
                self.sync_input();
            }
            SoundSettingsCommandOutput::InVolMuteChanged => self.sync_input(),
            SoundSettingsCommandOutput::InDevicesChanged => {
                self.in_devices = audio_service().input_devices.get();
                splice(&self.in_model, self.in_devices.iter().map(|d| d.name.get()));
                self.sync_input();
            }
        }
    }
}

impl SoundSettingsModel {
    /// Re-sync the output widgets from the service, blocking their handlers so
    /// the programmatic set never re-fires a write.
    fn sync_output(&self) {
        let default = audio_service().default_output.get();
        let idx = default
            .as_ref()
            .and_then(|d| device_index(&self.out_devices, &d.name.get()));
        block(&self.out_dd, &self.out_dd_handler, || {
            if let Some(i) = idx {
                self.out_dd.set_selected(i);
            }
        });
        let vol = default.as_ref().map(|d| d.volume.get().average()).unwrap_or(0.0);
        block(&self.out_scale, &self.out_scale_handler, || self.out_scale.set_value(vol));
        let muted = default.as_ref().map(|d| d.muted.get()).unwrap_or(false);
        block(&self.out_mute, &self.out_mute_handler, || self.out_mute.set_active(muted));
    }

    fn sync_input(&self) {
        let default = audio_service().default_input.get();
        let idx = default
            .as_ref()
            .and_then(|d| device_index_in(&self.in_devices, &d.name.get()));
        block(&self.in_dd, &self.in_dd_handler, || {
            if let Some(i) = idx {
                self.in_dd.set_selected(i);
            }
        });
        let vol = default.as_ref().map(|d| d.volume.get().average()).unwrap_or(0.0);
        block(&self.in_scale, &self.in_scale_handler, || self.in_scale.set_value(vol));
        let muted = default.as_ref().map(|d| d.muted.get()).unwrap_or(false);
        block(&self.in_mute, &self.in_mute_handler, || self.in_mute.set_active(muted));
    }
}

/// Run `f` with `widget`'s `handler` blocked, so a programmatic property set
/// doesn't bounce back through its `connect_*` closure.
fn block<W: IsA<gtk::glib::Object>>(widget: &W, handler: &SignalHandlerId, f: impl FnOnce()) {
    widget.block_signal(handler);
    f();
    widget.unblock_signal(handler);
}

fn string_list(names: impl Iterator<Item = String>) -> gtk::StringList {
    let v: Vec<String> = names.collect();
    let refs: Vec<&str> = v.iter().map(|s| s.as_str()).collect();
    gtk::StringList::new(&refs)
}

fn splice(model: &gtk::StringList, names: impl Iterator<Item = String>) {
    let v: Vec<String> = names.collect();
    let refs: Vec<&str> = v.iter().map(|s| s.as_str()).collect();
    model.splice(0, model.n_items(), &refs);
}

fn volume_scale() -> gtk::Scale {
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.01);
    scale.set_width_request(240);
    scale.set_valign(gtk::Align::Center);
    scale.set_draw_value(true);
    scale.set_format_value_func(|_, v| format!("{:.0}%", v * 100.0));
    scale
}

fn device_index(devices: &[Arc<OutputDevice>], name: &str) -> Option<u32> {
    devices.iter().position(|d| d.name.get() == name).map(|i| i as u32)
}

fn device_index_in(devices: &[Arc<InputDevice>], name: &str) -> Option<u32> {
    devices.iter().position(|d| d.name.get() == name).map(|i| i as u32)
}
