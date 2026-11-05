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
//!
//! With `session.reboot_into` on, a "Reboot into…" section below the
//! buttons lists the UEFI boot entries ([`super::reboot_into`]); the
//! focus walk continues into it.

use super::reboot_into::{self, RebootInto, RebootIntoAction};
use mshell_config::config_manager::config_manager;
use mshell_utils::boot_next::{BootList, BootTarget, ListError};
use mshell_utils::session::{SessionAction, run_session_action};
use reactive_graph::traits::GetUntracked;
use relm4::gtk::prelude::{BoxExt, ButtonExt, EventControllerExt, OrientableExt, WidgetExt};
use relm4::gtk::{gdk, glib};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

/// Seconds an armed action counts down before it fires.
const COUNTDOWN_SECS: u8 = 3;

/// What a key-shortcut should do. Kept as a plain `Copy` enum so a
/// single closure factory can emit any of them without owning the
/// component sender per binding.
#[derive(Debug, Clone, Copy)]
enum ShortcutAction {
    Arm(SessionAction),
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
    /// The "Reboot into…" section (hidden unless `session.reboot_into`).
    reboot_into: RebootInto,
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
    /// A click in the "Reboot into…" section.
    RebootInto(RebootIntoAction),
}

#[derive(Debug)]
pub(crate) enum SessionMenuWidgetCommandOutput {
    /// efibootmgr's listing (or why it couldn't be read).
    BootList(Result<BootList, ListError>),
    /// `BootNext` set (`reboot`: then reboot) or cleared.
    BootNextDone {
        result: Result<(), String>,
        reboot: bool,
    },
}

#[derive(Debug)]
pub(crate) enum SessionMenuWidgetOutput {
    CloseMenu,
}

pub(crate) struct SessionMenuWidgetInit {}

#[relm4::component(pub(crate))]
impl Component for SessionMenuWidgetModel {
    type CommandOutput = SessionMenuWidgetCommandOutput;
    type Input = SessionMenuWidgetInput;
    type Output = SessionMenuWidgetOutput;
    type Init = SessionMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "session-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            // ── §12 panel header ──
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,
                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("system-shutdown-symbolic"),
                },
                gtk::Label {
                    add_css_class: "panel-title",
                    set_label: "Session",
                    set_halign: gtk::Align::Start,
                    set_hexpand: true,
                    set_valign: gtk::Align::Center,
                },
            },

            #[local_ref]
            button_row -> gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 6,
                set_homogeneous: true,
            },

            #[local_ref]
            reboot_into_root -> gtk::Box {},

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
        let make_shortcut = |key: gdk::Key, mods: gdk::ModifierType, msg: ShortcutAction| {
            let s = sender.clone();
            gtk::Shortcut::builder()
                .trigger(&gtk::KeyvalTrigger::new(key, mods))
                .action(&gtk::CallbackAction::new(move |_, _| {
                    match msg {
                        ShortcutAction::Arm(action) => {
                            s.input(SessionMenuWidgetInput::Arm(action));
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
        // Capture phase, not Bubble. Tab is GTK4's built-in focus-
        // chain navigation key — buttons consume it in their
        // default handler to jump focus to the next focusable
        // sibling. ShortcutControllers default to Bubble phase,
        // which means our binding runs *after* the button's
        // default handler and the event is already eaten.
        // Capture phase fires the controller before any widget
        // gets a turn, so Tab / Shift+Tab / Ctrl+N / Ctrl+P /
        // Ctrl+J / Ctrl+K all reach us. Number keys 1–5 were
        // already working because buttons don't have a default
        // handler for those.
        sc.set_propagation_phase(gtk::PropagationPhase::Capture);

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
            let Some(action) = SessionAction::ALL.get(i) else {
                continue;
            };
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

        // Esc — cancel any running countdown (frame still closes
        // the menu via its own global ESC shortcut).
        sc.add_shortcut(make_shortcut(
            gdk::Key::Escape,
            gdk::ModifierType::empty(),
            ShortcutAction::Cancel,
        ));

        root.add_controller(sc);

        // Focus-walk keys (Tab / Shift+Tab / Left / Right / Ctrl+N /
        // Ctrl+P / Ctrl+J / Ctrl+K) ship via a separate
        // `EventControllerKey` attached to the **root** widget at
        // Capture phase. First attempt put this on `button_row`
        // which mirrored the Settings sidebar fix exactly, but in
        // a layer-shell menu surface the events never reached the
        // controller — either because focus was parked on a child
        // of `root` outside `button_row`, or because the
        // ShortcutController on root (also Capture phase) drained
        // the event first. Attaching the key controller to root
        // means *any* focused descendant routes Tab through us,
        // and we install it AFTER the ShortcutController so the
        // GTK delivery order respects insertion order at the same
        // phase. Number keys + Escape stay on the ShortcutController
        // above because they have no conflicting default handler.
        {
            let sender_walk = sender.clone();
            let key_controller = gtk::EventControllerKey::new();
            key_controller.set_propagation_phase(gtk::PropagationPhase::Capture);
            key_controller.connect_key_pressed(move |_, keyval, _, modifiers| {
                let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
                let ctrl = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
                let dir = match keyval {
                    gdk::Key::Tab if !shift && !ctrl => 1i32,
                    gdk::Key::Right | gdk::Key::Down if !ctrl => 1i32,
                    gdk::Key::n | gdk::Key::j if ctrl => 1i32,
                    gdk::Key::ISO_Left_Tab => -1i32,
                    gdk::Key::Tab if shift => -1i32,
                    gdk::Key::Left | gdk::Key::Up if !ctrl => -1i32,
                    gdk::Key::p | gdk::Key::k if ctrl => -1i32,
                    _ => return glib::Propagation::Proceed,
                };
                sender_walk.input(if dir > 0 {
                    SessionMenuWidgetInput::FocusNext
                } else {
                    SessionMenuWidgetInput::FocusPrev
                });
                glib::Propagation::Stop
            });
            root.add_controller(key_controller);
        }

        let reboot_into = {
            let s = sender.clone();
            RebootInto::new(move |a| s.input(SessionMenuWidgetInput::RebootInto(a)))
        };
        let reboot_into_root = reboot_into.root().clone();

        let model = SessionMenuWidgetModel {
            buttons,
            focused: 0,
            status: status_label.clone(),
            pending: None,
            generation: 0,
            reboot_into,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
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
            SessionMenuWidgetInput::FocusNext => self.walk_focus(1),
            SessionMenuWidgetInput::FocusPrev => self.walk_focus(-1),
            SessionMenuWidgetInput::RebootInto(action) => {
                self.reboot_into_action(action, &sender);
            }
            SessionMenuWidgetInput::ParentRevealChanged(revealed) => {
                self.reboot_into.reset(
                    revealed
                        && config_manager()
                            .config()
                            .get_untracked()
                            .session
                            .reboot_into,
                );
                if revealed {
                    // A fresh open — drop any stale countdown.
                    self.generation += 1;
                    self.pending = None;
                    self.focused = 0;
                    self.refresh_status();
                    // NB: keyboard mode is owned centrally by the frame's
                    // `sync_keyboard_mode` (Exclusive iff a menu is genuinely
                    // revealed), which is what makes Tab / Ctrl+N reach our
                    // EventControllerKey. We must NOT set it here:
                    // `ParentRevealChanged` rides the menu ScrolledWindow's
                    // `map` signal, which fires at startup even when the
                    // frame never revealed us, and setting Exclusive there
                    // strands the invisible frame layer in keyboard focus.
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
                }
            }
        }
    }
    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            SessionMenuWidgetCommandOutput::BootList(data) => {
                let session = config_manager().config().get_untracked().session;
                self.reboot_into
                    .loaded(data, &session.reboot_into_hidden, session.reboot_into_all);
            }
            SessionMenuWidgetCommandOutput::BootNextDone { result, reboot } => match result {
                // Authorized and set: the reboot is what was asked for, even
                // if the menu closed while the dialog was up.
                Ok(()) if reboot => {
                    run_session_action(SessionAction::Reboot);
                    let _ = sender.output(SessionMenuWidgetOutput::CloseMenu);
                }
                Ok(()) => self.read_boot_entries(&sender),
                Err(e) => self.reboot_into.failed(&e),
            },
        }
    }
}

impl SessionMenuWidgetModel {
    /// Step keyboard focus through the session buttons, then the
    /// "Reboot into…" header, entries and Clear (when shown).
    fn walk_focus(&mut self, dir: i32) {
        let mut chain: Vec<gtk::Widget> = self.buttons.iter().map(|b| b.clone().into()).collect();
        chain.extend(self.reboot_into.focusables());
        if chain.is_empty() {
            return;
        }
        let len = chain.len() as i32;
        let at = chain
            .iter()
            .position(|w| w.has_focus())
            .unwrap_or(self.focused.min(chain.len() - 1)) as i32;
        let next = (at + dir).rem_euclid(len) as usize;
        self.focused = next;
        chain[next].grab_focus();
    }

    fn reboot_into_action(&mut self, action: RebootIntoAction, sender: &ComponentSender<Self>) {
        if self.reboot_into.is_busy() {
            return;
        }
        // A click here is a deliberate choice: drop an armed countdown.
        if self.pending.take().is_some() {
            self.generation += 1;
            self.refresh_status();
        }
        let (target, reboot) = match action {
            RebootIntoAction::Toggle => {
                if self.reboot_into.toggle() {
                    self.read_boot_entries(sender);
                }
                return;
            }
            RebootIntoAction::Pick(num) => (BootTarget::Entry(num), true),
            RebootIntoAction::Clear => (BootTarget::Clear, false),
        };
        self.reboot_into.waiting();
        sender.oneshot_command(async move {
            SessionMenuWidgetCommandOutput::BootNextDone {
                result: reboot_into::apply(target).await,
                reboot,
            }
        });
    }

    fn read_boot_entries(&self, sender: &ComponentSender<Self>) {
        sender.oneshot_command(async {
            SessionMenuWidgetCommandOutput::BootList(reboot_into::read().await)
        });
    }

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
        // Fallible: a frame rebuild mid-countdown drops the component and
        // `input()` on the dead runtime would abort the shell.
        let _ = sender
            .input_sender()
            .send(SessionMenuWidgetInput::Tick(generation));
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
        .css_classes(vec![
            "ok-button-surface",
            "session-button",
            action.css_class(),
        ])
        .hexpand(true)
        .focusable(true)
        .build()
}
