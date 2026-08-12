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
    hidden: bool,
}

#[derive(Debug)]
pub(crate) enum InputDeviceRevealerButtonInput {
    Clicked,
    DefaultDeviceChanged,
    Revealed,
    Hidden,
    /// The rename popover closed — read its entry and persist.
    EditCommitted(String),
    /// Hide-from-cycling eye toggled — immediate, no popover.
    ToggleHidden,
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

            #[name = "content_button"]
            gtk::Button {
                add_css_class: "ok-button-surface",
                set_hexpand: true,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(InputDeviceRevealerButtonInput::Clicked);
                },

                model.content.widget().clone() {},
            },

            #[name = "hide_button"]
            gtk::Button {
                add_css_class: "ok-button-surface",
                add_css_class: "audio-dashboard-icon-button",
                set_valign: gtk::Align::Center,
                #[watch]
                set_icon_name: if model.hidden { "view-conceal-symbolic" } else { "view-reveal-symbolic" },
                #[watch]
                set_tooltip_text: Some(if model.hidden { "Hidden — click to show in cycling" } else { "Visible — click to hide from cycling" }),
                connect_clicked[sender] => move |_| {
                    sender.input(InputDeviceRevealerButtonInput::ToggleHidden);
                },
            },

            #[name = "edit_button"]
            gtk::MenuButton {
                add_css_class: "ok-button-surface",
                add_css_class: "audio-dashboard-icon-button",
                set_icon_name: "document-edit-symbolic",
                set_valign: gtk::Align::Center,
                set_tooltip_text: Some("Rename this device"),

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
                            set_label: "Display name",
                            set_halign: gtk::Align::Start,
                        },

                        #[name = "alias_entry"]
                        gtk::Entry {
                            set_width_chars: 20,
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
        let raw_description = params.input_device.description.get();
        let prefs = lock_prefs().get(&device_name);
        let button_content = RevealerButtonIconLabelModel::builder()
            .launch(RevealerButtonIconLabelInit {
                label: display_alias(&device_name, &raw_description),
                icon_name: "".to_string(),
                secondary_icon_name: "".to_string(),
                subtitle: compute_subtitle(prefs.alias.is_some(), &raw_description),
            })
            .detach();

        let model = InputDeviceRevealerButtonModel {
            input_device: params.input_device,
            content: button_content,
            watcher_token,
            hidden: prefs.hidden,
        };

        model
            .content
            .emit(RevealerButtonIconLabelInput::SetActive(is_current_default(
                &model.input_device,
            )));

        let widgets = view_output!();

        if let Some(popover) = widgets.edit_button.popover() {
            let entry = widgets.alias_entry.clone();
            let show_device_name = device_name.clone();
            popover.connect_show(move |_| {
                let prefs = lock_prefs().get(&show_device_name);
                entry.set_text(prefs.alias.as_deref().unwrap_or(""));
            });

            let entry = widgets.alias_entry.clone();
            let sender = sender.clone();
            popover.connect_closed(move |_| {
                sender.input(InputDeviceRevealerButtonInput::EditCommitted(
                    entry.text().to_string(),
                ));
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
            InputDeviceRevealerButtonInput::Clicked => {
                let device = self.input_device.clone();
                tokio::spawn(async move {
                    let _ = device.set_as_default().await;
                });
            }
            InputDeviceRevealerButtonInput::EditCommitted(alias) => {
                let device_name = self.input_device.name.get();
                let alias = alias.trim();
                let alias = (!alias.is_empty()).then(|| alias.to_string());
                let has_alias = alias.is_some();
                lock_prefs().set_alias(&device_name, alias);
                self.content
                    .emit(RevealerButtonIconLabelInput::SetLabel(display_alias(
                        &device_name,
                        &self.input_device.description.get(),
                    )));
                self.content
                    .emit(RevealerButtonIconLabelInput::SetSubtitle(compute_subtitle(
                        has_alias,
                        &self.input_device.description.get(),
                    )));
            }
            InputDeviceRevealerButtonInput::ToggleHidden => {
                self.hidden = !self.hidden;
                let device_name = self.input_device.name.get();
                lock_prefs().set_hidden(&device_name, self.hidden);
            }
            InputDeviceRevealerButtonInput::DefaultDeviceChanged => {
                let is_default = is_current_default(&self.input_device);
                self.content
                    .emit(RevealerButtonIconLabelInput::SetActive(is_default));
                self.content
                    .emit(RevealerButtonIconLabelInput::SetPrimaryIconName(
                        if is_default {
                            "check-circle-symbolic".to_string()
                        } else {
                            "".to_string()
                        },
                    ));
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
        self.update_view(widgets, sender);
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

fn is_current_default(device: &Arc<InputDevice>) -> bool {
    audio_service()
        .default_input
        .get()
        .map(|d| d.eq(device))
        .unwrap_or(false)
}

/// An un-renamed device already shows its real name as the title, so the
/// subtitle stays empty; a renamed one shows the real name underneath so
/// the alias doesn't hide what it actually is.
fn compute_subtitle(has_alias: bool, raw_description: &str) -> String {
    if has_alias {
        raw_description.to_string()
    } else {
        String::new()
    }
}
