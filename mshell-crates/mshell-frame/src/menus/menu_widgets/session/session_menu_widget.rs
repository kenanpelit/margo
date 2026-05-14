//! Session menu widget — the power-menu surface for
//! `MenuType::Session`. Five buttons: Lock / Logout / Suspend /
//! Reboot / Shutdown. Each runs the command from the `[session]`
//! config block, or a built-in `systemctl …` / session-lock
//! default when that field is left empty.
//!
//! Keyboard: Tab / Shift+Tab, Ctrl+N / Ctrl+P and the vim-style
//! Ctrl+J / Ctrl+K all step focus between the buttons (wrapping
//! at the ends); Space / Enter activates the focused one.

use mshell_utils::session::{SessionAction, run_session_action};
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::gtk::{gdk, glib};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct SessionMenuWidgetModel {
    /// The five action buttons, in display order — kept so the
    /// keyboard handler can walk focus between them.
    buttons: Vec<gtk::Button>,
    /// Index of the button keyboard focus currently sits on.
    focused: usize,
}

#[derive(Debug)]
pub(crate) enum SessionMenuWidgetInput {
    Activate(SessionAction),
    FocusNext,
    FocusPrev,
    /// The parent menu was shown / hidden. On show we land
    /// keyboard focus on the first button so Tab / Ctrl+N have a
    /// starting point — a focus grab at `init` time is a no-op
    /// because the menu surface isn't realized yet.
    ParentRevealChanged(bool),
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
        let mut buttons = Vec::with_capacity(SessionAction::ALL.len());
        for action in SessionAction::ALL {
            let btn = make_session_button(action);
            let s = sender.clone();
            btn.connect_clicked(move |_| s.input(SessionMenuWidgetInput::Activate(action)));
            button_row.append(&btn);
            buttons.push(btn);
        }

        // Tab / Shift+Tab, Ctrl+N/P and Ctrl+J/K all step focus
        // between the buttons; Tab is intercepted here too so it
        // wraps at the ends instead of escaping the menu.
        let key_controller = gtk::EventControllerKey::new();
        let sender_clone = sender.clone();
        key_controller.connect_key_pressed(move |_, key, _, modifier| {
            let ctrl = modifier.contains(gdk::ModifierType::CONTROL_MASK);
            let shift = modifier.contains(gdk::ModifierType::SHIFT_MASK);
            let is_next = (matches!(key, gdk::Key::Tab) && !shift)
                || (ctrl && matches!(key, gdk::Key::n | gdk::Key::N | gdk::Key::j | gdk::Key::J));
            let is_prev = matches!(key, gdk::Key::ISO_Left_Tab)
                || (matches!(key, gdk::Key::Tab) && shift)
                || (ctrl && matches!(key, gdk::Key::p | gdk::Key::P | gdk::Key::k | gdk::Key::K));
            if is_next {
                sender_clone.input(SessionMenuWidgetInput::FocusNext);
                glib::Propagation::Stop
            } else if is_prev {
                sender_clone.input(SessionMenuWidgetInput::FocusPrev);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        root.add_controller(key_controller);

        let model = SessionMenuWidgetModel { buttons, focused: 0 };
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
            SessionMenuWidgetInput::FocusNext => {
                if !self.buttons.is_empty() {
                    self.focused = (self.focused + 1) % self.buttons.len();
                    self.buttons[self.focused].grab_focus();
                }
            }
            SessionMenuWidgetInput::FocusPrev => {
                if !self.buttons.is_empty() {
                    let len = self.buttons.len();
                    self.focused = (self.focused + len - 1) % len;
                    self.buttons[self.focused].grab_focus();
                }
            }
            SessionMenuWidgetInput::ParentRevealChanged(revealed) => {
                if revealed
                    && let Some(first) = self.buttons.first()
                {
                    self.focused = 0;
                    first.grab_focus();
                }
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
