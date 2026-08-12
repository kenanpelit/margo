use crate::common_widgets::revealer_button::revealer_button_icon_label::{
    RevealerButtonIconLabelInit, RevealerButtonIconLabelInput, RevealerButtonIconLabelModel,
};
use mshell_common::WatcherToken;
use mshell_services::audio_service;
use mshell_utils::audio::spawn_default_input_watcher;
use mshell_utils::audio_prefs::{display_alias, lock_prefs};
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};
use std::sync::Arc;
use wayle_audio::core::device::input::InputDevice;

pub(crate) struct InputDeviceRevealerButtonModel {
    input_device: Arc<InputDevice>,
    content: Controller<RevealerButtonIconLabelModel>,
    watcher_token: WatcherToken,
}

#[derive(Debug)]
pub(crate) enum InputDeviceRevealerButtonInput {
    Clicked,
    DefaultDeviceChanged,
    Revealed,
    Hidden,
    /// The edit popover closed — read its entry/switch and persist.
    EditCommitted(String, bool),
}

#[derive(Debug)]
pub(crate) enum InputDeviceRevealerButtonOutput {}

pub(crate) struct InputDeviceRevealerButtonInit {
    pub input_device: Arc<InputDevice>,
}

#[derive(Debug)]
pub(crate) enum InputDeviceRevealerButtonCommandOutput {
    DefaultDeviceChanged,
}

#[relm4::component(pub)]
impl Component for InputDeviceRevealerButtonModel {
    type CommandOutput = InputDeviceRevealerButtonCommandOutput;
    type Input = InputDeviceRevealerButtonInput;
    type Output = InputDeviceRevealerButtonOutput;
    type Init = InputDeviceRevealerButtonInit;

    view! {
        #[root]
        gtk::Box {
            set_spacing: 4,

            gtk::Button {
                add_css_class: "ok-button-surface",
                set_hexpand: true,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(InputDeviceRevealerButtonInput::Clicked);
                },

                model.content.widget().clone() {},
            },

            #[name = "edit_button"]
            gtk::MenuButton {
                add_css_class: "ok-button-surface",
                set_icon_name: "document-edit-symbolic",
                set_valign: gtk::Align::Center,
                set_tooltip_text: Some("Rename or hide this device"),

                #[wrap(Some)]
                set_popover = &gtk::Popover {
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 8,
                        set_margin_start: 8,
                        set_margin_end: 8,
                        set_margin_top: 8,
                        set_margin_bottom: 8,

                        #[name = "alias_entry"]
                        gtk::Entry {
                            set_width_chars: 20,
                        },

                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 8,
                            gtk::Label {
                                set_label: "Hide from cycling",
                                set_hexpand: true,
                                set_xalign: 0.0,
                            },
                            #[name = "hidden_switch"]
                            gtk::Switch {
                                set_valign: gtk::Align::Center,
                            },
                        },
                    },
                },
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut watcher_token = WatcherToken::new();

        let token = watcher_token.reset();

        spawn_default_input_watcher(&sender, Some(token), || {
            InputDeviceRevealerButtonCommandOutput::DefaultDeviceChanged
        });

        let device_name = params.input_device.name.get();
        let button_content = RevealerButtonIconLabelModel::builder()
            .launch(RevealerButtonIconLabelInit {
                label: display_alias(&device_name, &params.input_device.description.get()),
                icon_name: "".to_string(),
                secondary_icon_name: "".to_string(),
            })
            .detach();

        let model = InputDeviceRevealerButtonModel {
            input_device: params.input_device,
            content: button_content,
            watcher_token,
        };

        let widgets = view_output!();

        widgets
            .alias_entry
            .set_placeholder_text(Some(&model.input_device.description.get()));
        if let Some(popover) = widgets.edit_button.popover() {
            let entry = widgets.alias_entry.clone();
            let switch = widgets.hidden_switch.clone();
            let show_device_name = device_name.clone();
            popover.connect_show(move |_| {
                let prefs = lock_prefs().get(&show_device_name);
                entry.set_text(prefs.alias.as_deref().unwrap_or(""));
                switch.set_active(prefs.hidden);
            });

            let entry = widgets.alias_entry.clone();
            let switch = widgets.hidden_switch.clone();
            let sender = sender.clone();
            popover.connect_closed(move |_| {
                sender.input(InputDeviceRevealerButtonInput::EditCommitted(
                    entry.text().to_string(),
                    switch.is_active(),
                ));
            });
        }

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        _widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            InputDeviceRevealerButtonInput::Clicked => {
                let device = self.input_device.clone();
                tokio::spawn(async move {
                    let _ = device.set_as_default().await;
                });
            }
            InputDeviceRevealerButtonInput::EditCommitted(alias, hidden) => {
                let device_name = self.input_device.name.get();
                let alias = alias.trim();
                let alias = (!alias.is_empty()).then(|| alias.to_string());
                let mut prefs = lock_prefs();
                prefs.set_alias(&device_name, alias);
                prefs.set_hidden(&device_name, hidden);
                drop(prefs);
                self.content
                    .emit(RevealerButtonIconLabelInput::SetLabel(display_alias(
                        &device_name,
                        &self.input_device.description.get(),
                    )));
            }
            InputDeviceRevealerButtonInput::DefaultDeviceChanged => {
                let default_device = audio_service().default_input.get();

                if let Some(default_device) = default_device {
                    if default_device.eq(&self.input_device) {
                        self.content
                            .emit(RevealerButtonIconLabelInput::SetPrimaryIconName(
                                "check-circle-symbolic".to_string(),
                            ))
                    } else {
                        self.content
                            .emit(RevealerButtonIconLabelInput::SetPrimaryIconName(
                                "".to_string(),
                            ))
                    }
                } else {
                    self.content
                        .emit(RevealerButtonIconLabelInput::SetPrimaryIconName(
                            "".to_string(),
                        ))
                }
            }
            InputDeviceRevealerButtonInput::Revealed => {
                let token = self.watcher_token.reset();

                spawn_default_input_watcher(&sender, Some(token), || {
                    InputDeviceRevealerButtonCommandOutput::DefaultDeviceChanged
                });
            }
            InputDeviceRevealerButtonInput::Hidden => {
                self.watcher_token.reset();
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
            InputDeviceRevealerButtonCommandOutput::DefaultDeviceChanged => {
                sender.input(InputDeviceRevealerButtonInput::DefaultDeviceChanged);
            }
        }
    }
}
