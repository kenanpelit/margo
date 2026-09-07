//! Settings → Guide.
//!
//! A browsable, searchable reference for what margo can do — dispatch
//! actions, config keys, a feature tour, and companion-tool `--help`
//! output. Every tab derives from an existing single-source-of-truth
//! (`mctl::actions::ACTIONS`, `margo/src/config.example.conf`,
//! `docs/features.md`) rather than hand-copied text, so it can't drift
//! the way prose docs elsewhere in this project have drifted before.
//! See `docs/superpowers/specs/2026-09-07-settings-guide-page-design.md`.

use crate::guide::actions_tab;
use crate::guide::config_tab;
use crate::guide::features_tab;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug, Default)]
pub(crate) struct GuideSettingsModel {}

#[derive(Debug)]
pub(crate) enum GuideSettingsInput {}

#[derive(Debug)]
pub(crate) enum GuideSettingsOutput {}

pub(crate) struct GuideSettingsInit {}

#[derive(Debug)]
pub(crate) enum GuideSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for GuideSettingsModel {
    type CommandOutput = GuideSettingsCommandOutput;
    type Input = GuideSettingsInput;
    type Output = GuideSettingsOutput;
    type Init = GuideSettingsInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "settings-page",
            set_orientation: gtk::Orientation::Vertical,
            set_hexpand: true,
            set_vexpand: true,
            set_spacing: 16,

            gtk::Box {
                add_css_class: "settings-hero",
                set_orientation: gtk::Orientation::Horizontal,
                set_halign: gtk::Align::Start,
                set_spacing: 16,
                gtk::Image {
                    add_css_class: "settings-hero-icon",
                    set_icon_name: Some("help-faq-symbolic"),
                    set_valign: gtk::Align::Center,
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_valign: gtk::Align::Center,
                    gtk::Label {
                        add_css_class: "settings-hero-title",
                        set_label: "Guide",
                        set_halign: gtk::Align::Start,
                    },
                    gtk::Label {
                        add_css_class: "settings-hero-subtitle",
                        set_label: "Browse everything margo can do.",
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                        set_wrap: true,
                    },
                },
            },

            #[name = "stack"]
            gtk::Stack {
                set_vexpand: true,
                set_transition_type: gtk::StackTransitionType::Crossfade,
            },

            gtk::StackSwitcher {
                set_stack: Some(&stack),
                set_halign: gtk::Align::Center,
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = GuideSettingsModel::default();
        let widgets = view_output!();
        widgets
            .stack
            .add_titled(&actions_tab::build(), Some("actions"), "Actions");
        widgets
            .stack
            .add_titled(&config_tab::build(), Some("config"), "Settings");
        widgets
            .stack
            .add_titled(&features_tab::build(), Some("features"), "Features");
        let _ = sender;
        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        _message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
    }
}
