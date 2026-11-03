//! One audio device row in the audio-in / audio-out menus: the device button
//! (click = make default), a "Use" shortcut, the hide-from-cycling eye and the
//! rename popover. Generic over the device side ([`AudioDeviceKind`]) so the
//! sink and source lists share one row; the per-side bits live in
//! `audio_out::output_device_revealer_button` and
//! `audio_in::input_device_revealer_button`.

use crate::common_widgets::revealer_button::revealer_button_icon_label::{
    RevealerButtonIconLabelInit, RevealerButtonIconLabelInput, RevealerButtonIconLabelModel,
};
use mshell_common::WatcherToken;
use mshell_utils::audio_prefs::{display_alias, lock_prefs};
use relm4::gtk::prelude::*;
use relm4::{
    Component, ComponentController, ComponentParts, ComponentSender, Controller, RelmWidgetExt, gtk,
};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// The side of the audio graph a row belongs to (sinks or sources).
pub(crate) trait AudioDeviceKind: Sized + 'static {
    type Device: Send + Sync + 'static;

    /// The PipeWire/Pulse node name — the key for stored prefs.
    fn name(device: &Self::Device) -> String;
    fn description(device: &Self::Device) -> String;
    fn icon(device: &Arc<Self::Device>) -> &'static str;
    /// Whether `device` is the current default of its side.
    fn is_default(device: &Arc<Self::Device>) -> bool;
    /// Watch the side's default device; `token` cancels the watch.
    fn spawn_default_watcher(
        sender: &ComponentSender<DeviceRevealerButtonModel<Self>>,
        token: CancellationToken,
    );
    /// Make `device` the default (async; errors are dropped like any other
    /// click that the audio server refuses).
    fn make_default(device: Arc<Self::Device>);
    /// True for a margo-created combine-sink group (output side only).
    fn is_group(_name: &str) -> bool {
        false
    }
    /// Remove the group named `name` (only reachable when [`Self::is_group`]).
    fn disband(_name: String) {}
}

pub(crate) struct DeviceRevealerButtonModel<K: AudioDeviceKind> {
    device: Arc<K::Device>,
    content: Controller<RevealerButtonIconLabelModel>,
    watcher_token: WatcherToken,
    /// True for a margo-created `module-combine-sink` row — shows
    /// "Disband group" in the edit popover instead of just rename.
    is_group: bool,
    hidden: bool,
    is_default: bool,
}

#[derive(Debug)]
pub(crate) enum DeviceRevealerButtonInput {
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
pub(crate) enum DeviceRevealerButtonOutput {}

pub(crate) struct DeviceRevealerButtonInit<K: AudioDeviceKind> {
    pub device: Arc<K::Device>,
}

#[derive(Debug)]
pub(crate) enum DeviceRevealerButtonCommandOutput {
    DefaultDeviceChanged,
}

#[relm4::component(pub(crate))]
impl<K: AudioDeviceKind> Component for DeviceRevealerButtonModel<K> {
    type CommandOutput = DeviceRevealerButtonCommandOutput;
    type Input = DeviceRevealerButtonInput;
    type Output = DeviceRevealerButtonOutput;
    type Init = DeviceRevealerButtonInit<K>;

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
                    sender.input(DeviceRevealerButtonInput::Clicked);
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
                    sender.input(DeviceRevealerButtonInput::Clicked);
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
                    sender.input(DeviceRevealerButtonInput::ToggleHidden);
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
                                sender.input(DeviceRevealerButtonInput::Disband);
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
        K::spawn_default_watcher(&sender, watcher_token.reset());

        let device_name = K::name(&params.device);
        let device_is_group = K::is_group(&device_name);
        let raw_description = K::description(&params.device);
        let prefs = lock_prefs().get(&device_name);
        let is_default = K::is_default(&params.device);
        let button_content = RevealerButtonIconLabelModel::builder()
            .launch(RevealerButtonIconLabelInit {
                label: display_alias(&device_name, &raw_description),
                icon_name: K::icon(&params.device).to_string(),
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

        let model = DeviceRevealerButtonModel {
            device: params.device,
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
                sender.input(DeviceRevealerButtonInput::EditCommitted(
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
            DeviceRevealerButtonInput::Clicked => {
                K::make_default(self.device.clone());
            }
            DeviceRevealerButtonInput::EditCommitted(alias) => {
                let device_name = K::name(&self.device);
                let alias = alias.trim();
                let alias = (!alias.is_empty()).then(|| alias.to_string());
                let has_alias = alias.is_some();
                lock_prefs().set_alias(&device_name, alias);
                let description = K::description(&self.device);
                self.content
                    .emit(RevealerButtonIconLabelInput::SetLabel(display_alias(
                        &device_name,
                        &description,
                    )));
                self.content
                    .emit(RevealerButtonIconLabelInput::SetSubtitle(compute_subtitle(
                        self.is_default,
                        self.is_group,
                        has_alias,
                        &description,
                    )));
            }
            DeviceRevealerButtonInput::ToggleHidden => {
                self.hidden = !self.hidden;
                lock_prefs().set_hidden(&K::name(&self.device), self.hidden);
            }
            DeviceRevealerButtonInput::Disband => {
                if self.is_group {
                    K::disband(K::name(&self.device));
                }
            }
            DeviceRevealerButtonInput::DefaultDeviceChanged => {
                self.is_default = K::is_default(&self.device);
                widgets
                    .content_button
                    .set_class_active("active", self.is_default);
                let has_alias = lock_prefs().get(&K::name(&self.device)).alias.is_some();
                self.content
                    .emit(RevealerButtonIconLabelInput::SetSubtitle(compute_subtitle(
                        self.is_default,
                        self.is_group,
                        has_alias,
                        &K::description(&self.device),
                    )));
                self.content
                    .emit(RevealerButtonIconLabelInput::SetActive(self.is_default));
            }
            DeviceRevealerButtonInput::Revealed => {
                K::spawn_default_watcher(&sender, self.watcher_token.reset());
            }
            DeviceRevealerButtonInput::Hidden => {
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
            DeviceRevealerButtonCommandOutput::DefaultDeviceChanged => {
                sender.input(DeviceRevealerButtonInput::DefaultDeviceChanged);
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::compute_subtitle;

    #[test]
    fn the_default_device_reads_active_whatever_else_is_true() {
        assert_eq!(compute_subtitle(true, false, false, "Speakers"), "Active");
        assert_eq!(compute_subtitle(true, true, true, "Speakers"), "Active");
    }

    #[test]
    fn a_group_row_is_labelled_as_combined_output() {
        assert_eq!(
            compute_subtitle(false, true, true, "Combined"),
            "Combined output"
        );
    }

    #[test]
    fn a_renamed_device_shows_its_real_name_underneath() {
        assert_eq!(compute_subtitle(false, false, true, "USB Mic"), "USB Mic");
    }

    #[test]
    fn an_unrenamed_device_has_no_subtitle() {
        assert_eq!(compute_subtitle(false, false, false, "USB Mic"), "");
    }
}
