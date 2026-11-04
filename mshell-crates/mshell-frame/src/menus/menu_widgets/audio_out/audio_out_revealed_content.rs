//! Output device list of the audio-out revealer row: the connected sinks
//! (rename / hide / make default rows), the "Group outputs" popover, and
//! **Bluetooth handoff** rows — paired but disconnected Bluetooth audio
//! devices (`mshell_utils::bt_audio`). Picking one connects it and makes
//! its sink the default output once PipeWire creates it.

use crate::menus::menu_widgets::audio_out::output_device_revealer_button::{
    OutputDeviceRevealerButtonInit, OutputDeviceRevealerButtonInput,
    OutputDeviceRevealerButtonModel,
};
use mshell_common::WatcherToken;
use mshell_common::dynamic_box::dynamic_box::{
    DynamicBoxFactory, DynamicBoxInit, DynamicBoxInput, DynamicBoxModel,
};
use mshell_common::dynamic_box::generic_widget_controller::{
    GenericWidgetController, GenericWidgetControllerExtSafe,
};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{AudioConfigStoreFields, ConfigStoreFields};
use mshell_config::toast::{ToastEvent, ToastSeverity, push_toast};
use mshell_services::audio_groups::{create_group, is_group};
use mshell_services::audio_service;
use mshell_utils::audio::{is_hdmi_output, spawn_output_devices_watcher};
use mshell_utils::audio_prefs::{display_alias, is_hidden};
use mshell_utils::bluetooth::spawn_bluetooth_devices_watcher;
use mshell_utils::bt_audio::{BtAudioDevice, current_candidates};
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::RevealerTransitionType;
use relm4::gtk::pango;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use wayle_audio::core::device::output::OutputDevice;

pub(crate) struct AudioOutRevealedContentModel {
    devices_dynamic_box_controller: Controller<DynamicBoxModel<Arc<OutputDevice>, String>>,
    watcher_token: WatcherToken,
    /// MACs with a handoff connect in flight (row shows "Connecting…").
    connecting: Vec<String>,
}

#[derive(Debug)]
pub(crate) enum AudioOutRevealedContentInput {
    UpdateDevices,
    Revealed,
    Hidden,
    /// "Create group" clicked with the checked device names.
    CreateGroup(Vec<String>),
    /// A disconnected Bluetooth output row was picked.
    Handoff(String),
}

#[derive(Debug)]
pub(crate) enum AudioOutRevealedContentOutput {}

pub(crate) struct AudioOutRevealedContentInit {}

#[derive(Debug)]
pub(crate) enum AudioOutRevealedContentCommandOutput {
    DevicesUpdated,
    /// A handoff connect finished (MAC, success).
    HandoffDone(String, bool),
}

#[relm4::component(pub)]
impl Component for AudioOutRevealedContentModel {
    type CommandOutput = AudioOutRevealedContentCommandOutput;
    type Input = AudioOutRevealedContentInput;
    type Output = AudioOutRevealedContentOutput;
    type Init = AudioOutRevealedContentInit;

    view! {
        #[root]
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 4,

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_halign: gtk::Align::End,

                #[name = "group_button"]
                gtk::MenuButton {
                    add_css_class: "audio-dashboard-group-trigger",
                    set_label: "Group outputs",
                    set_tooltip_text: Some("Play through two or more outputs at once"),

                    #[wrap(Some)]
                    set_popover = &gtk::Popover {
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_spacing: 8,
                            set_margin_start: 8,
                            set_margin_end: 8,
                            set_margin_top: 8,
                            set_margin_bottom: 8,

                            gtk::Label {
                                add_css_class: "label-small",
                                set_label: "Select at least two outputs to play through both at once",
                                set_halign: gtk::Align::Start,
                                set_max_width_chars: 28,
                                set_wrap: true,
                            },

                            #[name = "group_checklist_box"]
                            gtk::Box {
                                set_orientation: gtk::Orientation::Vertical,
                                set_spacing: 2,
                                // Fixed width so a long device description
                                // can't balloon the popover past the panel
                                // edge — device labels ellipsize inside it
                                // instead (see connect_show below).
                                set_width_request: 260,
                            },

                            #[name = "create_group_button"]
                            gtk::Button {
                                add_css_class: "audio-dashboard-action-button",
                                set_label: "Create group",
                                set_sensitive: false,
                            },
                        },
                    },
                },
            },

            model.devices_dynamic_box_controller.widget().clone() {},

            // Paired, disconnected Bluetooth outputs (rebuilt on updates).
            #[name = "bt_handoff_box"]
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 0,
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut watcher_token = WatcherToken::new();

        let token = watcher_token.reset();

        spawn_output_devices_watcher(&sender, token, || {
            AudioOutRevealedContentCommandOutput::DevicesUpdated
        });
        // New/removed paired devices change the handoff rows too.
        spawn_bluetooth_devices_watcher(&sender, || {
            AudioOutRevealedContentCommandOutput::DevicesUpdated
        });

        let devices_dynamic_box_factory = DynamicBoxFactory::<Arc<OutputDevice>, String> {
            id: Box::new(|item| item.name.get()),
            create: Box::new(move |item| {
                let device = item.clone();
                let revealer_button = OutputDeviceRevealerButtonModel::builder()
                    .launch(OutputDeviceRevealerButtonInit { device })
                    .detach();

                Box::new(revealer_button) as Box<dyn GenericWidgetController>
            }),
            update: None,
        };

        let devices_dynamic_box_controller: Controller<DynamicBoxModel<Arc<OutputDevice>, String>> =
            DynamicBoxModel::builder()
                .launch(DynamicBoxInit {
                    factory: devices_dynamic_box_factory,
                    orientation: gtk::Orientation::Vertical,
                    spacing: 0,
                    transition_type: RevealerTransitionType::SlideDown,
                    transition_duration_ms: 200,
                    reverse: false,
                    retain_entries: false,
                    allow_drag_and_drop: false,
                })
                .detach();

        let model = AudioOutRevealedContentModel {
            devices_dynamic_box_controller,
            watcher_token: WatcherToken::new(),
            connecting: Vec::new(),
        };

        let widgets = view_output!();

        // Checkbox rows are rebuilt fresh each time the popover opens (the
        // output list can change between opens); `checked_rows` is the
        // shared read side the Create button consults, avoiding a
        // GTK-widget-tree downcast walk.
        let checked_rows: Rc<RefCell<Vec<(gtk::CheckButton, String)>>> =
            Rc::new(RefCell::new(Vec::new()));

        if let Some(popover) = widgets.group_button.popover() {
            let checklist_box = widgets.group_checklist_box.clone();
            let create_button = widgets.create_group_button.clone();
            let checked_rows_show = checked_rows.clone();
            popover.connect_show(move |_| {
                while let Some(child) = checklist_box.first_child() {
                    checklist_box.remove(&child);
                }
                checked_rows_show.borrow_mut().clear();
                create_button.set_sensitive(false);

                let candidates: Vec<Arc<OutputDevice>> = audio_service()
                    .map(|a| a.output_devices.get())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|d| mshell_utils::audio::output_connected(d))
                    .filter(|d| !is_group(&d.name.get()))
                    .filter(|d| !is_hidden(&d.name.get()))
                    .collect();

                for device in candidates {
                    let name = device.name.get();
                    let label = display_alias(&name, &device.description.get());
                    let check = gtk::CheckButton::new();
                    let check_label = gtk::Label::new(Some(&label));
                    check_label.set_halign(gtk::Align::Start);
                    check_label.set_ellipsize(pango::EllipsizeMode::End);
                    check_label.set_max_width_chars(26);
                    check.set_child(Some(&check_label));
                    checklist_box.append(&check);
                    checked_rows_show.borrow_mut().push((check.clone(), name));

                    let create_button = create_button.clone();
                    let checked_rows = checked_rows_show.clone();
                    check.connect_toggled(move |_| {
                        let checked = checked_rows
                            .borrow()
                            .iter()
                            .filter(|(c, _)| c.is_active())
                            .count();
                        create_button.set_sensitive(checked >= 2);
                    });
                }
            });

            let sender_create = sender.clone();
            let checked_rows_create = checked_rows;
            widgets.create_group_button.connect_clicked(move |_| {
                let names: Vec<String> = checked_rows_create
                    .borrow()
                    .iter()
                    .filter(|(c, _)| c.is_active())
                    .map(|(_, name)| name.clone())
                    .collect();
                sender_create.input(AudioOutRevealedContentInput::CreateGroup(names));
                popover.popdown();
            });
        }

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
            AudioOutRevealedContentInput::UpdateDevices => {
                let Some(audio) = audio_service() else {
                    return;
                };
                let hide_hdmi = config_manager()
                    .config()
                    .audio()
                    .hide_hdmi_outputs()
                    .get_untracked();
                let all = audio.output_devices.get();
                let sink_names: Vec<String> = all.iter().map(|d| d.name.get()).collect();
                let devices: Vec<_> = all
                    .into_iter()
                    .filter(|d| mshell_utils::audio::output_connected(d))
                    .filter(|d| !(hide_hdmi && is_hdmi_output(d)))
                    .collect();
                self.devices_dynamic_box_controller
                    .emit(DynamicBoxInput::SetItems(devices));
                rebuild_bt_handoff(
                    &widgets.bt_handoff_box,
                    &current_candidates(&sink_names),
                    &self.connecting,
                    &sender,
                );
            }
            AudioOutRevealedContentInput::Revealed => {
                let token = self.watcher_token.reset();

                spawn_output_devices_watcher(&sender, token, || {
                    AudioOutRevealedContentCommandOutput::DevicesUpdated
                });

                self.devices_dynamic_box_controller
                    .model()
                    .for_each_entry(|_, entry| {
                        if let Some(ctrl) = entry
                            .controller
                            .as_ref()
                            .downcast_ref::<Controller<OutputDeviceRevealerButtonModel>>()
                        {
                            ctrl.emit(OutputDeviceRevealerButtonInput::Revealed);
                        }
                    });
            }
            AudioOutRevealedContentInput::Hidden => {
                self.watcher_token.reset();

                self.devices_dynamic_box_controller
                    .model()
                    .for_each_entry(|_, entry| {
                        if let Some(ctrl) = entry
                            .controller
                            .as_ref()
                            .downcast_ref::<Controller<OutputDeviceRevealerButtonModel>>()
                        {
                            ctrl.emit(OutputDeviceRevealerButtonInput::Hidden);
                        }
                    });
            }
            AudioOutRevealedContentInput::Handoff(mac) => {
                if !self.connecting.contains(&mac) {
                    self.connecting.push(mac.clone());
                    sender.input(AudioOutRevealedContentInput::UpdateDevices);
                    sender.oneshot_command(async move {
                        let ok = mshell_services::bluetooth::connect_for_output(&mac).await;
                        AudioOutRevealedContentCommandOutput::HandoffDone(mac, ok)
                    });
                }
            }
            AudioOutRevealedContentInput::CreateGroup(names) => {
                tokio::spawn(async move {
                    create_group(&names).await;
                });
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
            AudioOutRevealedContentCommandOutput::DevicesUpdated => {
                sender.input(AudioOutRevealedContentInput::UpdateDevices);
            }
            AudioOutRevealedContentCommandOutput::HandoffDone(mac, ok) => {
                self.connecting.retain(|m| m != &mac);
                if !ok {
                    push_toast(ToastEvent {
                        icon: "bluetooth-disabled-symbolic".to_string(),
                        title: "Bluetooth output".to_string(),
                        body: Some("Couldn't connect the device".to_string()),
                        severity: ToastSeverity::Warn,
                    });
                }
                sender.input(AudioOutRevealedContentInput::UpdateDevices);
            }
        }
    }
}

/// Rebuild the Bluetooth handoff rows: the device-row button shape of the
/// sink rows above (icon + name + dim status), plus a trailing "Connect"
/// in the same slot and style as the sink rows' "Use".
fn rebuild_bt_handoff(
    container: &gtk::Box,
    candidates: &[BtAudioDevice],
    connecting: &[String],
    sender: &ComponentSender<AudioOutRevealedContentModel>,
) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    for dev in candidates {
        let busy = connecting.contains(&dev.mac);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);

        let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        // bluez icon names (`audio-headset`) as symbolic when the theme
        // has them — the same fallback the Bluetooth menu uses.
        let themed = dev
            .icon
            .as_deref()
            .map(|i| format!("{i}-symbolic"))
            .filter(|i| {
                gtk::gdk::Display::default()
                    .is_some_and(|d| gtk::IconTheme::for_display(&d).has_icon(i))
            })
            .unwrap_or_else(|| "bluetooth-active-symbolic".to_string());
        let icon = gtk::Image::from_icon_name(&themed);
        content.append(&icon);
        let texts = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let title = gtk::Label::new(Some(&dev.alias));
        title.set_xalign(0.0);
        title.set_ellipsize(pango::EllipsizeMode::End);
        let status = gtk::Label::new(Some(if busy {
            "Connecting…"
        } else {
            "Bluetooth · not connected"
        }));
        status.set_css_classes(&["label-small", "dim-label"]);
        status.set_xalign(0.0);
        texts.append(&title);
        texts.append(&status);
        content.append(&texts);

        let button = gtk::Button::new();
        button.add_css_class("audio-dashboard-device-row");
        button.set_hexpand(true);
        button.set_child(Some(&content));
        button.set_tooltip_text(Some("Connect and use as the output"));
        button.set_sensitive(!busy);

        let connect = gtk::Button::with_label("Connect");
        connect.add_css_class("audio-dashboard-use-button");
        connect.set_valign(gtk::Align::Center);
        connect.set_sensitive(!busy);

        for b in [&button, &connect] {
            let s = sender.clone();
            let mac = dev.mac.clone();
            b.connect_clicked(move |_| s.input(AudioOutRevealedContentInput::Handoff(mac.clone())));
        }
        row.append(&button);
        row.append(&connect);
        container.append(&row);
    }
    container.set_visible(!candidates.is_empty());
}
