//! Session menu widget — the power-menu surface for
//! `MenuType::Session`. Five buttons: Lock / Logout / Suspend /
//! Reboot / Shutdown. Each runs the command from the `[session]`
//! config block, or a built-in `systemctl …` / session-lock
//! default when that field is left empty.

use mshell_utils::session::{SessionAction, run_session_action};
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct SessionMenuWidgetModel {}

#[derive(Debug)]
pub(crate) enum SessionMenuWidgetInput {
    Activate(SessionAction),
}

#[derive(Debug)]
pub(crate) enum SessionMenuWidgetOutput {
    CloseMenu,
}

pub(crate) struct SessionMenuWidgetInit {}

#[relm4::component(pub(crate))]
impl Component for SessionMenuWidgetModel {
    type CommandOutput = ();
    type Input = SessionMenuWidgetInput;
    type Output = SessionMenuWidgetOutput;
    type Init = SessionMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "session-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            gtk::Label {
                add_css_class: "label-large-bold",
                set_label: "Session",
                set_xalign: 0.0,
            },

            #[local_ref]
            button_row -> gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 6,
                set_homogeneous: true,
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let button_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        for action in SessionAction::ALL {
            let btn = make_session_button(action);
            let s = sender.clone();
            btn.connect_clicked(move |_| s.input(SessionMenuWidgetInput::Activate(action)));
            button_row.append(&btn);
        }

        let model = SessionMenuWidgetModel {};
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            SessionMenuWidgetInput::Activate(action) => {
                run_session_action(action);
                let _ = sender.output(SessionMenuWidgetOutput::CloseMenu);
            }
        }
    }
}

/// One session button — a vertical icon + label tile, carrying
/// the action's colour-state class (`.session-reboot` etc.).
fn make_session_button(action: SessionAction) -> gtk::Button {
    let inner = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .halign(gtk::Align::Center)
        .build();
    let img = gtk::Image::from_icon_name(action.icon());
    img.set_pixel_size(24);
    inner.append(&img);
    let label = gtk::Label::new(Some(action.label()));
    label.add_css_class("label-small-bold");
    inner.append(&label);
    gtk::Button::builder()
        .child(&inner)
        .css_classes(vec!["ok-button-surface", "session-button", action.css_class()])
        .hexpand(true)
        .build()
}
