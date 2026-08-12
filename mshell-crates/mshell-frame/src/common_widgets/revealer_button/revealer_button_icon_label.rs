use relm4::gtk;
use relm4::gtk::pango;
use relm4::gtk::prelude::*;
use relm4::prelude::*;
use std::ops::Not;

pub struct RevealerButtonIconLabelModel {
    pub label: String,
    pub icon_name: String,
    pub secondary_icon_name: String,
    /// Small dim line under the label — device port, group member
    /// summary, etc. Empty hides the line entirely, so callers that
    /// don't need it (network rows) render exactly as before.
    pub subtitle: String,
    /// Tints the subtitle line primary + bold (used for an "Active"
    /// status line, matching the row's own tinted background — see
    /// `output_device_revealer_button.rs`). Defaults false; callers that
    /// don't track an active state (network rows) never emit `SetActive`
    /// and the subtitle just never tints.
    pub active: bool,
}

#[derive(Debug)]
pub enum RevealerButtonIconLabelInput {
    #[allow(dead_code)]
    SetSecondaryIconName(String),
    #[allow(dead_code)]
    SetLabel(String),
    #[allow(dead_code)]
    SetSubtitle(String),
    #[allow(dead_code)]
    SetActive(bool),
}

pub struct RevealerButtonIconLabelInit {
    pub label: String,
    pub icon_name: String,
    pub secondary_icon_name: String,
    pub subtitle: String,
}

#[relm4::component(pub)]
impl SimpleComponent for RevealerButtonIconLabelModel {
    type Init = RevealerButtonIconLabelInit;
    type Input = RevealerButtonIconLabelInput;
    type Output = ();

    view! {
        gtk::Box{
            #[name = "image"]
            gtk::Image {
                add_css_class: "revealer-button-icon-label-icon",
                set_margin_end: 12,
                #[watch]
                set_icon_name: Some(model.icon_name.as_str()),
            },

            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_valign: gtk::Align::Center,
                set_spacing: 0,

                #[name = "label"]
                gtk::Label {
                    add_css_class: "label-small",
                    add_css_class: "revealer-button-title",
                    set_halign: gtk::Align::Start,
                    set_hexpand: true,
                    set_ellipsize: pango::EllipsizeMode::End,
                    #[watch]
                    set_label: model.label.as_str(),
                },

                #[name = "subtitle_label"]
                gtk::Label {
                    add_css_class: "revealer-button-subtitle",
                    set_halign: gtk::Align::Start,
                    set_hexpand: true,
                    set_ellipsize: pango::EllipsizeMode::End,
                    #[watch]
                    set_visible: !model.subtitle.is_empty(),
                    #[watch]
                    set_label: model.subtitle.as_str(),
                    #[watch]
                    set_class_active: ("active", model.active),
                },
            },

            #[name = "secondary_image"]
            gtk::Image {
                #[watch]
                set_visible: model.secondary_icon_name.is_empty().not(),
                add_css_class: "revealer-button-icon-label-icon",
                set_margin_start: 12,
                #[watch]
                set_icon_name: Some(model.secondary_icon_name.as_str()),
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = RevealerButtonIconLabelModel {
            label: params.label,
            icon_name: params.icon_name,
            secondary_icon_name: params.secondary_icon_name,
            subtitle: params.subtitle,
            active: false,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, _sender: ComponentSender<Self>) {
        match msg {
            RevealerButtonIconLabelInput::SetSecondaryIconName(icon_name) => {
                self.secondary_icon_name = icon_name;
            }
            RevealerButtonIconLabelInput::SetLabel(label) => {
                self.label = label;
            }
            RevealerButtonIconLabelInput::SetSubtitle(subtitle) => {
                self.subtitle = subtitle;
            }
            RevealerButtonIconLabelInput::SetActive(active) => {
                self.active = active;
            }
        }
    }
}
