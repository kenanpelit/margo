use crate::common_widgets::revealer_button::revealer_button_icon_label::{
    RevealerButtonIconLabelInit, RevealerButtonIconLabelInput, RevealerButtonIconLabelModel,
};
use mshell_common::WatcherToken;
use mshell_services::audio_groups::is_group;
use mshell_services::audio_service;
use mshell_utils::audio::{get_audio_out_icon_device_aware, spawn_default_output_watcher};
use mshell_utils::audio_prefs::{display_alias, lock_prefs};
use relm4::gtk::prelude::*;
use relm4::{
    Component, ComponentController, ComponentParts, ComponentSender, Controller, RelmWidgetExt, gtk,
};
use std::sync::Arc;
use wayle_audio::core::device::output::OutputDevice;

pub(crate) struct OutputDeviceRevealerButtonModel {
    output_device: Arc<OutputDevice>,
    content: Controller<RevealerButtonIconLabelModel>,
    watcher_token: WatcherToken,
    /// True for a margo-created `module-combine-sink` row — shows
    /// "Disband group" in the edit popover instead of just rename.
    is_group: bool,
    hidden: bool,
    is_default: bool,
}

#[derive(Debug)]
pub(crate) enum OutputDeviceRevealerButtonInput {
    Clicked,
    DefaultDeviceChanged,
    Revealed,
    Hidden,
    /// The rename popover closed — read its entry and persist.
    EditCommitted(String),
    /// Hide-from-cycling eye toggled — immediate, no popover.
    ToggleHidden,
    /// "Disband group" clicked — remove this combine-sink.
    Disband,
}

#[derive(Debug)]
pub(crate) enum OutputDeviceRevealerButtonOutput {}

pub(crate) struct OutputDeviceRevealerButtonInit {
    pub output_device: Arc<OutputDevice>,
}

#[derive(Debug)]
pub(crate) enum OutputDeviceRevealerButtonCommandOutput {
    DefaultDeviceChanged,
}

#[relm4::component(pub)]
impl Component for OutputDeviceRevealerButtonModel {
    type CommandOutput = OutputDeviceRevealerButtonCommandOutput;
    type Input = OutputDeviceRevealerButtonInput;
    type Output = OutputDeviceRevealerButtonOutput;
    type Init = OutputDeviceRevealerButtonInit;

    view! {
        #[root]
        gtk::Box {
            set_spacing: 4,

            #[name = "content_button"]
            gtk::Button {
                add_css_class: "audio-dashboard-device-row",
                set_hexpand: true,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(OutputDeviceRevealerButtonInput::Clicked);
                },

                model.content.widget().clone() {},
            },

            #[name = "use_button"]
            gtk::Button {
                add_css_class: "audio-dashboard-use-button",
                set_label: "Use",
                set_valign: gtk::Align::Center,
                #[watch]
                set_visible: !model.is_default,
                connect_clicked[sender] => move |_| {
                    sender.input(OutputDeviceRevealerButtonInput::Clicked);
                },
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
                    sender.input(OutputDeviceRevealerButtonInput::ToggleHidden);
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

                        gtk::Button {
                            add_css_class: "audio-dashboard-action-button",
                            set_label: "Disband group",
                            set_visible: model.is_group,
                            connect_clicked[sender] => move |_| {
                                sender.input(OutputDeviceRevealerButtonInput::Disband);
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

        spawn_default_output_watcher(&sender, Some(token), || {
            OutputDeviceRevealerButtonCommandOutput::DefaultDeviceChanged
        });

        let device_name = params.output_device.name.get();
        let device_is_group = is_group(&device_name);
        let raw_description = params.output_device.description.get();
        let prefs = lock_prefs().get(&device_name);
        let is_default = is_current_default(&params.output_device);
        let button_content = RevealerButtonIconLabelModel::builder()
            .launch(RevealerButtonIconLabelInit {
                label: display_alias(&device_name, &raw_description),
                icon_name: get_audio_out_icon_device_aware(&params.output_device).to_string(),
                secondary_icon_name: "".to_string(),
                subtitle: compute_subtitle(
                    is_default,
                    device_is_group,
                    prefs.alias.is_some(),
                    &raw_description,
                ),
            })
            .detach();
        button_content.emit(RevealerButtonIconLabelInput::SetActive(is_default));

        let model = OutputDeviceRevealerButtonModel {
            output_device: params.output_device,
            content: button_content,
            watcher_token,
            is_group: device_is_group,
            hidden: prefs.hidden,
            is_default,
        };

        let widgets = view_output!();

        widgets
            .content_button
            .set_class_active("active", model.is_default);

        // Pre-fill the popover from stored prefs each time it opens — it
        // isn't reactive (relm4 doesn't watch a plain gtk::Popover), so
        // this is the read side; `connect_closed` below is the write side.
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
                sender.input(OutputDeviceRevealerButtonInput::EditCommitted(
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
            OutputDeviceRevealerButtonInput::Clicked => {
                let device = self.output_device.clone();
                tokio::spawn(async move {
                    if device.set_as_default().await.is_ok() {
                        mshell_utils::audio::migrate_playback_streams_to(&device).await;
                    }
                });
            }
            OutputDeviceRevealerButtonInput::EditCommitted(alias) => {
                let device_name = self.output_device.name.get();
                let alias = alias.trim();
                let alias = (!alias.is_empty()).then(|| alias.to_string());
                let has_alias = alias.is_some();
                lock_prefs().set_alias(&device_name, alias);
                self.content
                    .emit(RevealerButtonIconLabelInput::SetLabel(display_alias(
                        &device_name,
                        &self.output_device.description.get(),
                    )));
                self.content
                    .emit(RevealerButtonIconLabelInput::SetSubtitle(compute_subtitle(
                        self.is_default,
                        self.is_group,
                        has_alias,
                        &self.output_device.description.get(),
                    )));
            }
            OutputDeviceRevealerButtonInput::ToggleHidden => {
                self.hidden = !self.hidden;
                let device_name = self.output_device.name.get();
                lock_prefs().set_hidden(&device_name, self.hidden);
            }
            OutputDeviceRevealerButtonInput::Disband => {
                let device_name = self.output_device.name.get();
                tokio::spawn(async move {
                    if let Some(group) = mshell_services::audio_groups::list_groups()
                        .await
                        .into_iter()
                        .find(|g| g.sink_name == device_name)
                    {
                        mshell_services::audio_groups::disband_group(&group).await;
                    }
                });
            }
            OutputDeviceRevealerButtonInput::DefaultDeviceChanged => {
                self.is_default = is_current_default(&self.output_device);
                widgets
                    .content_button
                    .set_class_active("active", self.is_default);
                let device_name = self.output_device.name.get();
                let has_alias = lock_prefs().get(&device_name).alias.is_some();
                self.content
                    .emit(RevealerButtonIconLabelInput::SetSubtitle(compute_subtitle(
                        self.is_default,
                        self.is_group,
                        has_alias,
                        &self.output_device.description.get(),
                    )));
                self.content
                    .emit(RevealerButtonIconLabelInput::SetActive(self.is_default));
            }
            OutputDeviceRevealerButtonInput::Revealed => {
                let token = self.watcher_token.reset();

                spawn_default_output_watcher(&sender, Some(token), || {
                    OutputDeviceRevealerButtonCommandOutput::DefaultDeviceChanged
                });
            }
            OutputDeviceRevealerButtonInput::Hidden => {
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
            OutputDeviceRevealerButtonCommandOutput::DefaultDeviceChanged => {
                sender.input(OutputDeviceRevealerButtonInput::DefaultDeviceChanged);
            }
        }
    }
}

fn is_current_default(device: &Arc<OutputDevice>) -> bool {
    audio_service()
        .default_output
        .get()
        .map(|d| d.eq(device))
        .unwrap_or(false)
}

/// The active device's subtitle is always "Active" (matches the reference
/// design's status line) regardless of alias/group — that's the single most
/// useful thing to say about the row you're currently listening through. A
/// group row is labelled as such; a renamed hardware device shows its real
/// name underneath so the alias doesn't hide what it actually is; an
/// un-renamed hardware device already shows that name as the title, so the
/// subtitle stays empty rather than repeating it.
fn compute_subtitle(
    is_default: bool,
    is_group: bool,
    has_alias: bool,
    raw_description: &str,
) -> String {
    if is_default {
        "Active".to_string()
    } else if is_group {
        "Combined output".to_string()
    } else if has_alias {
        raw_description.to_string()
    } else {
        String::new()
    }
}
