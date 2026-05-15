//! Session menu widget — the power-menu surface for
//! `MenuType::Session`. Five buttons: Lock / Logout / Suspend /
//! Reboot / Shutdown. Each runs the command from the `[session]`
//! config block, or a built-in `systemctl …` / session-lock
//! default when that field is left empty.
//!
//! Keyboard:
//!   * `1`–`5` arm the matching action behind a 3-second
//!     countdown; the status line shows the tick. Any key while
//!     a countdown runs (or `Escape`) cancels it.
//!   * `Tab` / `Shift+Tab`, `Ctrl+N` / `Ctrl+P`, `Ctrl+J` /
//!     `Ctrl+K` step focus between the buttons.
//!   * `Space` / `Enter` activates the focused button at once
//!     (no countdown — a deliberate click is taken at its word).

use gtk4_layer_shell::{KeyboardMode, LayerShell};
use mshell_utils::session::{SessionAction, run_session_action};
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::gtk::{gdk, glib};
use relm4::{Component, ComponentParts, ComponentSender, RelmWidgetExt, gtk};
use std::time::Duration;

/// Seconds an armed action counts down before it fires.
const COUNTDOWN_SECS: u8 = 3;

/// What a key-shortcut should do. Kept as a plain `Copy` enum so a
/// single closure factory can emit any of them without owning the
/// component sender per binding.
#[derive(Debug, Clone, Copy)]
enum ShortcutAction {
    Arm(SessionAction),
    FocusNext,
    FocusPrev,
    Cancel,
}

pub(crate) struct SessionMenuWidgetModel {
    /// The five action buttons, in display order — kept so the
    /// keyboard handler can walk focus between them.
    buttons: Vec<gtk::Button>,
    /// Index of the button keyboard focus currently sits on.
    focused: usize,
    /// Status line — shows the live countdown, empty otherwise.
    status: gtk::Label,
    /// Armed action + seconds left, or `None` when idle.
    pending: Option<(SessionAction, u8)>,
    /// Bumped on every arm / cancel so stale countdown ticks
    /// from a previous run are ignored.
    generation: u64,
}

#[derive(Debug)]
pub(crate) enum SessionMenuWidgetInput {
    /// Run an action immediately (button click).
    Activate(SessionAction),
    /// Arm an action behind the countdown (number key).
    Arm(SessionAction),
    /// One countdown tick — carries the generation it belongs to.
    Tick(u64),
    /// Cancel a running countdown.
    Cancel,
    FocusNext,
    FocusPrev,
    /// The parent menu was shown / hidden. On show we land
    /// keyboard focus on the first button (after a short delay —
    /// the layer-shell surface only gains keyboard focus once
    /// `sync_keyboard_mode`'s debounce has applied).
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

            #[local_ref]
            status_label -> gtk::Label {
                add_css_class: "session-status",
                set_xalign: 0.0,
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

        let status_label = gtk::Label::new(None);

        // All keyboard handling lives here — number keys arm the
        // countdown, Tab / Ctrl+N walk focus, Escape cancels.
        //
        // **Why a `ShortcutController` (not `EventControllerKey`).**
        // Two earlier attempts to wire keys via `EventControllerKey`
        // failed:
        //
        //   * default `Bubble` phase: every button is
        //     `focusable(true)`, so GTK4's built-in Tab handler
        //     moved focus and *consumed* the event before it
        //     could bubble up to our controller.
        //   * `Capture` phase: didn't fire reliably either —
        //     focus path / mapping timing seemed to leave the
        //     controller dormant on a freshly-revealed menu.
        //
        // `ShortcutController` (the same path the frame uses for
        // its ESC handler) sidesteps both issues: each shortcut is
        // a `KeyvalTrigger` matched against the keymap before any
        // widget gets a turn at the event. As long as the layer
        // surface holds keyboard focus, the binding fires.
        let make_shortcut =
            |key: gdk::Key, mods: gdk::ModifierType, msg: ShortcutAction| {
                let s = sender.clone();
                gtk::Shortcut::builder()
                    .trigger(&gtk::KeyvalTrigger::new(key, mods))
                    .action(&gtk::CallbackAction::new(move |_, _| {
                        match msg {
                            ShortcutAction::Arm(action) => {
                                s.input(SessionMenuWidgetInput::Arm(action));
                            }
                            ShortcutAction::FocusNext => {
                                s.input(SessionMenuWidgetInput::FocusNext);
                            }
                            ShortcutAction::FocusPrev => {
                                s.input(SessionMenuWidgetInput::FocusPrev);
                            }
                            ShortcutAction::Cancel => {
                                s.input(SessionMenuWidgetInput::Cancel);
                            }
                        }
                        glib::Propagation::Stop
                    }))
                    .build()
            };

        let sc = gtk::ShortcutController::new();
        sc.set_scope(gtk::ShortcutScope::Local);

        // 1–5 (and keypad) — arm the matching action.
        for (i, (a, b)) in [
            (gdk::Key::_1, gdk::Key::KP_1),
            (gdk::Key::_2, gdk::Key::KP_2),
            (gdk::Key::_3, gdk::Key::KP_3),
            (gdk::Key::_4, gdk::Key::KP_4),
            (gdk::Key::_5, gdk::Key::KP_5),
        ]
        .into_iter()
        .enumerate()
        {
            let Some(action) = SessionAction::ALL.get(i) else { continue };
            sc.add_shortcut(make_shortcut(
                a,
                gdk::ModifierType::empty(),
                ShortcutAction::Arm(*action),
            ));
            sc.add_shortcut(make_shortcut(
                b,
                gdk::ModifierType::empty(),
                ShortcutAction::Arm(*action),
            ));
        }

        // Focus walk forward — Tab, Ctrl+N, Ctrl+J.
        sc.add_shortcut(make_shortcut(
            gdk::Key::Tab,
            gdk::ModifierType::empty(),
            ShortcutAction::FocusNext,
        ));
        for key in [gdk::Key::n, gdk::Key::j] {
            sc.add_shortcut(make_shortcut(
                key,
                gdk::ModifierType::CONTROL_MASK,
                ShortcutAction::FocusNext,
            ));
        }

        // Focus walk back — Shift+Tab (ISO_Left_Tab), Ctrl+P, Ctrl+K.
        sc.add_shortcut(make_shortcut(
            gdk::Key::ISO_Left_Tab,
            gdk::ModifierType::SHIFT_MASK,
            ShortcutAction::FocusPrev,
        ));
        sc.add_shortcut(make_shortcut(
            gdk::Key::Tab,
            gdk::ModifierType::SHIFT_MASK,
            ShortcutAction::FocusPrev,
        ));
        for key in [gdk::Key::p, gdk::Key::k] {
            sc.add_shortcut(make_shortcut(
                key,
                gdk::ModifierType::CONTROL_MASK,
                ShortcutAction::FocusPrev,
            ));
        }

        // Esc — cancel any running countdown (frame still closes
        // the menu via its own global ESC shortcut).
        sc.add_shortcut(make_shortcut(
            gdk::Key::Escape,
            gdk::ModifierType::empty(),
            ShortcutAction::Cancel,
        ));

        root.add_controller(sc);

        let model = SessionMenuWidgetModel {
            buttons,
            focused: 0,
            status: status_label.clone(),
            pending: None,
            generation: 0,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            SessionMenuWidgetInput::Activate(action) => {
                run_session_action(action);
                let _ = sender.output(SessionMenuWidgetOutput::CloseMenu);
            }
            SessionMenuWidgetInput::Arm(action) => {
                // (Re-)arm: bump the generation so any in-flight
                // tick from a previous countdown is ignored.
                self.generation += 1;
                self.pending = Some((action, COUNTDOWN_SECS));
                self.refresh_status();
                schedule_tick(&sender, self.generation);
            }
            SessionMenuWidgetInput::Tick(generation) => {
                if generation != self.generation {
                    return; // stale tick from a cancelled run
                }
                let Some((action, remaining)) = self.pending else {
                    return;
                };
                let remaining = remaining.saturating_sub(1);
                if remaining == 0 {
                    self.pending = None;
                    self.refresh_status();
                    run_session_action(action);
                    let _ = sender.output(SessionMenuWidgetOutput::CloseMenu);
                } else {
                    self.pending = Some((action, remaining));
                    self.refresh_status();
                    schedule_tick(&sender, generation);
                }
            }
            SessionMenuWidgetInput::Cancel => {
                if self.pending.is_some() {
                    self.generation += 1;
                    self.pending = None;
                    self.refresh_status();
                }
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
                if revealed {
                    // A fresh open — drop any stale countdown.
                    self.generation += 1;
                    self.pending = None;
                    self.focused = 0;
                    self.refresh_status();
                    // Mirror app_launcher: flip the host layer-shell
                    // surface to Exclusive keyboard mode so Tab /
                    // Ctrl+N / Ctrl+P actually reach the GTK
                    // EventControllerKey we wired up. Without this
                    // the layer surface stays on KeyboardMode::None
                    // (the default for menu surfaces) and only the
                    // menu's number-key arming works because that
                    // path travels via a different route.
                    if let Some(window) = root.toplevel_window() {
                        window.set_keyboard_mode(KeyboardMode::Exclusive);
                    }
                    if let Some(first) = self.buttons.first().cloned() {
                        // The layer-shell surface only takes keyboard
                        // focus after `sync_keyboard_mode`'s ~90 ms
                        // debounce; grabbing focus synchronously sets
                        // the window's focus pointer but it doesn't
                        // stick. Re-grab once the surface is actually
                        // keyboard-focused.
                        glib::timeout_add_local_once(Duration::from_millis(160), move || {
                            first.grab_focus();
                        });
                    }
                } else {
                    self.generation += 1;
                    self.pending = None;
                    self.refresh_status();
                    // Release the exclusive grab when the menu hides.
                    if let Some(window) = root.toplevel_window() {
                        window.set_keyboard_mode(KeyboardMode::None);
                    }
                }
            }
        }
    }
}

impl SessionMenuWidgetModel {
    /// Repaint the status line from `pending`.
    fn refresh_status(&self) {
        match self.pending {
            Some((action, remaining)) => {
                self.status.set_label(&format!(
                    "{} in {remaining}…  (Esc / any key to cancel)",
                    action.label(),
                ));
            }
            None => self.status.set_label(""),
        }
    }
}

/// Queue one countdown tick a second out, tagged with the
/// generation it belongs to so a cancelled run can ignore it.
fn schedule_tick(sender: &ComponentSender<SessionMenuWidgetModel>, generation: u64) {
    let sender = sender.clone();
    glib::timeout_add_local_once(Duration::from_secs(1), move || {
        sender.input(SessionMenuWidgetInput::Tick(generation));
    });
}

/// One session button — a vertical icon + label tile, carrying
/// the action's colour-state class (`.session-reboot` etc.). The
/// label leads with the `1`–`5` shortcut number.
fn make_session_button(action: SessionAction) -> gtk::Button {
    let shortcut = match action {
        SessionAction::Lock => "1",
        SessionAction::Logout => "2",
        SessionAction::Suspend => "3",
        SessionAction::Reboot => "4",
        SessionAction::Shutdown => "5",
    };
    let inner = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .halign(gtk::Align::Center)
        .build();
    let img = gtk::Image::from_icon_name(action.icon());
    img.set_pixel_size(24);
    inner.append(&img);
    let label = gtk::Label::new(Some(&format!("{shortcut}  {}", action.label())));
    label.add_css_class("label-small-bold");
    inner.append(&label);
    gtk::Button::builder()
        .child(&inner)
        .css_classes(vec!["ok-button-surface", "session-button", action.css_class()])
        .hexpand(true)
        .focusable(true)
        .build()
}
