//! Control Center menu widget — the panel content for
//! `MenuType::ControlCenter`. Embeds the header (Task 2); later tasks
//! will fill the body with sliders, toggles, and tiles.

use crate::menus::menu_widgets::control_center::header::{
    ControlCenterHeaderInit, ControlCenterHeaderInput, ControlCenterHeaderModel,
    ControlCenterHeaderOutput,
};
use crate::menus::menu_widgets::control_center::sliders::{
    ControlCenterSlidersInit, ControlCenterSlidersModel,
};
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};

pub(crate) struct ControlCenterMenuWidgetModel {
    header: Controller<ControlCenterHeaderModel>,
    sliders: relm4::Controller<ControlCenterSlidersModel>,
    /// Whether edit-mode is active (inert until Task 6).
    edit_mode: bool,
}

#[derive(Debug)]
pub(crate) enum ControlCenterMenuWidgetInput {
    ParentRevealChanged(bool),
    /// Forwarded from header output; inert until Task 6.
    ToggleEdit,
    /// Header SessionPower icon → ask the frame to open the session menu.
    RequestSessionMenu,
    /// Forwarded from header Lock/Settings outputs (handled in the header).
    _HeaderActionHandled,
}

#[derive(Debug)]
pub(crate) enum ControlCenterMenuWidgetOutput {
    /// Open the session / power menu (the header power icon).
    ToggleSessionMenu,
}

pub(crate) struct ControlCenterMenuWidgetInit {}

#[relm4::component(pub(crate))]
impl Component for ControlCenterMenuWidgetModel {
    type CommandOutput = ();
    type Input = ControlCenterMenuWidgetInput;
    type Output = ControlCenterMenuWidgetOutput;
    type Init = ControlCenterMenuWidgetInit;

    view! {
        #[root]
        #[name = "root_box"]
        gtk::Box {
            add_css_class: "control-center-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 16,
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Build the header component and forward its outputs.
        let header = ControlCenterHeaderModel::builder()
            .launch(ControlCenterHeaderInit {})
            .forward(sender.input_sender(), |msg| match msg {
                // Lock and Settings are already handled inside the header
                // (lock_session() / open_settings() called directly). No
                // further action needed at the menu-widget level.
                ControlCenterHeaderOutput::Lock => {
                    ControlCenterMenuWidgetInput::_HeaderActionHandled
                }
                ControlCenterHeaderOutput::SessionPower => {
                    ControlCenterMenuWidgetInput::RequestSessionMenu
                }
                ControlCenterHeaderOutput::Settings => {
                    ControlCenterMenuWidgetInput::_HeaderActionHandled
                }
                ControlCenterHeaderOutput::ToggleEdit => {
                    ControlCenterMenuWidgetInput::ToggleEdit
                }
            });

        // Build the sliders component (volume + brightness).
        let sliders = ControlCenterSlidersModel::builder()
            .launch(ControlCenterSlidersInit {})
            .detach();

        let model = ControlCenterMenuWidgetModel {
            header,
            sliders,
            edit_mode: false,
        };

        let widgets = view_output!();

        // Prepend the header widget at the top of the root box.
        widgets.root_box.prepend(model.header.widget());
        // Append the sliders row directly after the header.
        widgets.root_box.append(model.sliders.widget());

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            ControlCenterMenuWidgetInput::RequestSessionMenu => {
                sender
                    .output(ControlCenterMenuWidgetOutput::ToggleSessionMenu)
                    .ok();
            }
            ControlCenterMenuWidgetInput::ParentRevealChanged(revealed) => {
                if revealed {
                    // Refresh uptime whenever the menu is opened.
                    self.header
                        .sender()
                        .send(ControlCenterHeaderInput::RecomputeUptime)
                        .ok();
                }
            }
            ControlCenterMenuWidgetInput::ToggleEdit => {
                // Toggle the stored state; Task 6 will act on it.
                self.edit_mode = !self.edit_mode;
            }
            ControlCenterMenuWidgetInput::_HeaderActionHandled => {
                // Lock/Settings/SessionPower already handled in the header;
                // nothing to do at this level.
            }
        }
    }
}
