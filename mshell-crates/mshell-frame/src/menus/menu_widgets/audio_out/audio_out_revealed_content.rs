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
use mshell_services::audio_groups::{create_group, is_group};
use mshell_services::audio_service;
use mshell_utils::audio::{is_hdmi_output, spawn_output_devices_watcher};
use mshell_utils::audio_prefs::{display_alias, is_hidden};
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::RevealerTransitionType;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use wayle_audio::core::device::output::OutputDevice;

pub(crate) struct AudioOutRevealedContentModel {
    devices_dynamic_box_controller: Controller<DynamicBoxModel<Arc<OutputDevice>, String>>,
    watcher_token: WatcherToken,
}

#[derive(Debug)]
pub(crate) enum AudioOutRevealedContentInput {
    UpdateDevices,
    Revealed,
    Hidden,
    /// "Create group" clicked with the checked device names.
    CreateGroup(Vec<String>),
}

#[derive(Debug)]
pub(crate) enum AudioOutRevealedContentOutput {}

pub(crate) struct AudioOutRevealedContentInit {}

#[derive(Debug)]
pub(crate) enum AudioOutRevealedContentCommandOutput {
    DevicesUpdated,
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

        let devices_dynamic_box_factory = DynamicBoxFactory::<Arc<OutputDevice>, String> {
            id: Box::new(|item| item.name.get()),
            create: Box::new(move |item| {
                let device = item.clone();
                let revealer_button = OutputDeviceRevealerButtonModel::builder()
                    .launch(OutputDeviceRevealerButtonInit {
                        output_device: device,
                    })
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
                    .output_devices
                    .get()
                    .into_iter()
                    .filter(|d| !is_group(&d.name.get()))
                    .filter(|d| !is_hidden(&d.name.get()))
                    .collect();

                for device in candidates {
                    let name = device.name.get();
                    let label = display_alias(&name, &device.description.get());
                    let check = gtk::CheckButton::with_label(&label);
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
                let audio = audio_service();
                let hide_hdmi = config_manager()
                    .config()
                    .audio()
                    .hide_hdmi_outputs()
                    .get_untracked();
                let devices: Vec<_> = audio
                    .output_devices
                    .get()
                    .into_iter()
                    .filter(|d| !(hide_hdmi && is_hdmi_output(d)))
                    .collect();
                self.devices_dynamic_box_controller
                    .emit(DynamicBoxInput::SetItems(devices))
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
        }
    }
}
