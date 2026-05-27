//! Control Center menu widget — the panel content for
//! `MenuType::ControlCenter`. A minimal scaffold panel for Task 1;
//! later tasks will fill the body with controls.

use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct ControlCenterMenuWidgetModel {}

#[derive(Debug)]
pub(crate) enum ControlCenterMenuWidgetInput {
    ParentRevealChanged(bool),
}

pub(crate) struct ControlCenterMenuWidgetInit {}

#[relm4::component(pub(crate))]
impl Component for ControlCenterMenuWidgetModel {
    type CommandOutput = ();
    type Input = ControlCenterMenuWidgetInput;
    type Output = ();
    type Init = ControlCenterMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "control-center-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 16,

            // ── §12 panel header ──
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,
                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("preferences-system-symbolic"),
                },
                gtk::Label {
                    add_css_class: "panel-title",
                    set_label: "Control Center",
                    set_hexpand: true,
                    set_halign: gtk::Align::Start,
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = ControlCenterMenuWidgetModel {};
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            ControlCenterMenuWidgetInput::ParentRevealChanged(_revealed) => {
                // Future tasks will lazy-start pollers here when revealed.
            }
        }
    }
}
