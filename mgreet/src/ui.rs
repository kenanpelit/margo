//! The per-monitor greeter window: a fullscreen layer-shell surface with a
//! centred login card. Username/password entries share the [`State`] buffers,
//! so input on any monitor is reflected on all of them.

use gtk4 as gtk;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use mlogind_proto::Request;
use std::rc::Rc;
use zeroize::Zeroizing;

use crate::State;

/// Build (and present) a greeter window pinned to `monitor`, filling it.
pub fn build_window(
    app: &gtk::Application,
    monitor: &gdk::Monitor,
    state: &Rc<State>,
    connector: &str,
) -> gtk::Window {
    let window = gtk::Window::new();
    window.add_css_class("mgreet-root");
    app.add_window(&window);

    // Fullscreen layer-shell surface on THIS monitor that owns the keyboard.
    window.init_layer_shell();
    window.set_monitor(Some(monitor));
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("mgreet"));
    window.set_exclusive_zone(-1);
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(edge, true);
    }
    // Real greeter owns the keyboard exclusively; the preview / dry-run (run
    // under a live session) uses OnDemand so a test run can never trap input.
    window.set_keyboard_mode(if state.real() {
        KeyboardMode::Exclusive
    } else {
        KeyboardMode::OnDemand
    });
    window.set_decorated(false);

    // Size the window to the monitor explicitly. Anchoring all four edges should
    // make the compositor fill the output on its own, but a bare 4-anchor layer
    // surface was landing at the wrong output's width on a multi-monitor greeter
    // (the external panel ended up laptop-wide); seeding GTK's allocation with the
    // real per-output geometry makes coverage deterministic.
    let geo = monitor.geometry();
    window.set_default_size(geo.width().max(1), geo.height().max(1));

    // Opaque backdrop; a centred card floats over it (Overlay stacks children).
    let scrim = gtk::Box::new(gtk::Orientation::Vertical, 0);
    scrim.add_css_class("mgreet-scrim");
    scrim.set_hexpand(true);
    scrim.set_vexpand(true);

    let card = build_card(state, connector);
    card.set_halign(gtk::Align::Center);
    card.set_valign(gtk::Align::Center);

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&scrim));
    overlay.add_overlay(&card);

    // Battery indicator, top-right (laptops only — None on a desktop).
    if let Some(battery) = build_battery() {
        overlay.add_overlay(&battery);
    }

    // Power-action footer (F-key chips), bottom-centre.
    if !state.power.is_empty() {
        let footer = build_power_footer(&state.power);
        footer.set_halign(gtk::Align::Center);
        footer.set_valign(gtk::Align::End);
        footer.set_margin_bottom(22);
        overlay.add_overlay(&footer);
    }

    window.set_child(Some(&overlay));

    // Keyboard: the power F-keys anywhere (real greeter ONLY — a preview run
    // under the live session must never poweroff the machine), plus Escape to
    // quit in preview. Matching by GTK key name ("F1"…) mirrors the TUI.
    let key = gtk::EventControllerKey::new();
    let app_weak = app.downgrade();
    let allow_escape_quit = !state.real();
    let state_keys = state.clone();
    key.connect_key_pressed(move |_, keyval, _, _| {
        if let Some(name) = keyval.name()
            && let Some(action) = state_keys.power.iter().find(|a| a.key == name.as_str())
        {
            // Not mid-conversation: a Power frame would land in the runner's PAM
            // conversation callback, where it is not an answer to the prompt PAM
            // is holding open.
            if state_keys.real() && !state_keys.conversing.get() {
                let index = action.index;
                if !send(&state_keys, &Request::Power { index }) {
                    lost(&state_keys);
                }
                // The runner always replies; its Info/Error lands in the fd
                // watcher and reaches every monitor's status line from there.
            }
            return glib::Propagation::Stop;
        }
        if keyval == gdk::Key::Escape {
            if allow_escape_quit && let Some(app) = app_weak.upgrade() {
                app.quit();
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(key);

    // `set_visible(true)` (not `present()`) matches mshell-frame's proven
    // multi-monitor 4-anchor layer surface: present() adds toplevel raise/focus
    // semantics a layer surface shouldn't need.
    window.set_visible(true);
    window
}

fn build_card(state: &Rc<State>, connector: &str) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 14);
    card.add_css_class("mgreet-card");

    // ── Clock / greeting / date ──
    let greeting = label(&["mgreet-greeting"]);
    let clock = label(&["mgreet-clock"]);
    let date = label(&["mgreet-date"]);
    card.append(&greeting);
    card.append(&clock);
    card.append(&date);
    update_clock(&greeting, &clock, &date);
    {
        let (g, c, d) = (greeting.clone(), clock.clone(), date.clone());
        glib::timeout_add_seconds_local(1, move || {
            update_clock(&g, &c, &d);
            glib::ControlFlow::Continue
        });
    }

    // Hostname — a small orienting touch (which machine you're logging into).
    if let Some(host) = hostname() {
        let host_label = label(&["mgreet-host"]);
        host_label.set_text(&host);
        card.append(&host_label);
    }

    card.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    // ── Username ──
    // No visible caption on any of the three rows: each control already names
    // itself (the entries via their placeholder, the drop-down via the selected
    // session), so a caption above it just says the same word twice. The name is
    // still exposed to assistive tech, which can't read a placeholder.
    let username = gtk::Entry::with_buffer(&state.username);
    username.add_css_class("mgreet-field");
    username.set_placeholder_text(Some("Username"));
    username.set_hexpand(true);
    username.update_property(&[gtk::accessible::Property::Label("Username")]);
    card.append(&username);

    // ── Password ──
    let password = gtk::Entry::with_buffer(&state.password);
    password.add_css_class("mgreet-field");
    password.set_placeholder_text(Some("Password"));
    password.set_visibility(false);
    password.set_input_purpose(gtk::InputPurpose::Password);
    password.set_hexpand(true);
    password.update_property(&[gtk::accessible::Property::Label("Password")]);

    // ── Caps Lock warning ── (critical for password entry). Updated from the
    // modifier state on each keystroke in the password field. Grouped tightly
    // with the field so it reads as belonging to the one it warns about.
    let password_group = gtk::Box::new(gtk::Orientation::Vertical, 6);
    password_group.append(&password);
    let caps = label(&["mgreet-caps"]);
    caps.set_text("\u{2191} Caps Lock is on");
    caps.set_visible(false);
    password_group.append(&caps);
    card.append(&password_group);
    {
        let caps_ctrl = gtk::EventControllerKey::new();
        let caps_p = caps.clone();
        caps_ctrl.connect_key_pressed(move |_, _, _, mods| {
            caps_p.set_visible(mods.contains(gdk::ModifierType::LOCK_MASK));
            glib::Propagation::Proceed
        });
        let caps_r = caps.clone();
        caps_ctrl.connect_key_released(move |_, _, _, mods| {
            caps_r.set_visible(mods.contains(gdk::ModifierType::LOCK_MASK));
        });
        password.add_controller(caps_ctrl);
    }

    // ── Session picker ──
    let session_names: Vec<&str> = state.sessions.iter().map(|s| s.name.as_str()).collect();
    let sessions = if session_names.is_empty() {
        gtk::DropDown::from_strings(&["No sessions"])
    } else {
        gtk::DropDown::from_strings(&session_names)
    };
    sessions.add_css_class("mgreet-session");
    // Full width, like the two fields above it. (GtkDropDown's internal
    // `button_stack` is hexpand, so the label stays left and the arrow rides the
    // right edge, and GtkDropDown size-requests its popover to the button width.)
    sessions.set_hexpand(true);
    sessions.update_property(&[gtk::accessible::Property::Label("Session")]);
    // Pre-select the last-used session (from the shared mlogind cache).
    if let Some(idx) =
        crate::sessions::select_index(&state.sessions, state.initial_session.as_deref())
    {
        sessions.set_selected(idx);
    }
    card.append(&sessions);

    // ── Status line ──
    let status = label(&["mgreet-status"]);
    card.append(&status);
    // One conversation, many monitors: the runner's prompts and errors have to
    // land on every screen, not only the one the user happened to submit from.
    state
        .status
        .borrow_mut()
        .insert(connector.to_string(), status.clone());

    // ── Log-in button ──
    let login = gtk::Button::with_label("Log in");
    login.add_css_class("mgreet-login");
    card.append(&login);

    // Submit: the button, or Enter in the password field.
    let submit: Rc<dyn Fn()> = {
        let state = state.clone();
        let status = status.clone();
        let sessions = sessions.clone();
        Rc::new(move || submit_login(&state, &status, &sessions))
    };
    {
        let submit = submit.clone();
        login.connect_clicked(move |_| submit());
    }
    {
        let submit = submit.clone();
        password.connect_activate(move |_| submit());
    }
    {
        // Enter in the username field advances to the password field.
        let password = password.clone();
        username.connect_activate(move |_| {
            password.grab_focus();
        });
    }

    // Focus the empty field once the window is shown.
    {
        let username = username.clone();
        let password = password.clone();
        let has_user = !state.username.text().is_empty();
        glib::idle_add_local_once(move || {
            if has_user {
                password.grab_focus();
            } else {
                username.grab_focus();
            }
        });
    }

    card
}

/// The machine hostname (from /etc/hostname), if set.
fn hostname() -> Option<String> {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Top-right battery indicator (icon + percent), refreshed every 30 s. `None`
/// on a host with no battery, so desktops show nothing.
fn build_battery() -> Option<gtk::Widget> {
    crate::battery::read()?; // gate: no battery → no indicator

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.add_css_class("mgreet-battery");
    row.set_halign(gtk::Align::End);
    row.set_valign(gtk::Align::Start);
    row.set_margin_top(16);
    row.set_margin_end(20);

    let icon = label(&["mgreet-battery-icon"]);
    let pct = label(&["mgreet-battery-pct"]);
    row.append(&icon);
    row.append(&pct);

    let refresh = move || {
        if let Some(b) = crate::battery::read() {
            icon.set_text(crate::battery::icon(&b));
            pct.set_text(&format!("{}%", b.percent));
        }
    };
    refresh();
    glib::timeout_add_seconds_local(30, move || {
        refresh();
        glib::ControlFlow::Continue
    });

    Some(row.upcast())
}

/// A centred row of `[F1] Shutdown  [F2] Reboot …` chips for the power actions.
fn build_power_footer(actions: &[crate::power::PowerAction]) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 18);
    row.add_css_class("mgreet-power");
    for action in actions {
        let chip = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        chip.add_css_class("mgreet-power-chip");
        let key = gtk::Label::new(Some(&format!("[{}]", action.key)));
        key.add_css_class("mgreet-power-key");
        let hint = gtk::Label::new(Some(&action.hint));
        hint.add_css_class("mgreet-power-hint");
        chip.append(&key);
        chip.append(&hint);
        row.append(&chip);
    }
    row
}

/// Open a conversation with the session runner, answer a prompt it is holding
/// open, or — with no runner — echo intent.
///
/// Nothing here blocks. `Begin` and `Response` are one small datagram each; the
/// runner's replies arrive later, in [`on_runner_event`], from the GTK main
/// loop. That is the difference from the old greeter, whose in-process
/// `pam_authenticate` froze the UI for the length of the PAM stack — which is
/// why there was never an "Authenticating…" frame to paint.
fn submit_login(state: &Rc<State>, status: &gtk::Label, sessions: &gtk::DropDown) {
    let user = state.username.text().to_string();
    let session = state
        .sessions
        .get(sessions.selected() as usize)
        .map(|s| s.name.clone())
        .unwrap_or_default();

    match crate::auth::decide_submit(
        &user,
        &session,
        state.awaiting_prompt.get(),
        state.conversing.get(),
        state.real(),
    ) {
        crate::auth::Submit::Reject(msg) => set_status(status, msg, true),
        crate::auth::Submit::Preview(msg) => {
            set_status(status, &msg, false);
            // Length only — never the password itself.
            eprintln!(
                "[mgreet] (preview) would authenticate user={user:?} session={session:?} pass_len={}",
                state.password.text().len()
            );
        }
        // Enter pressed again while PAM is still thinking. Say nothing new.
        crate::auth::Submit::Busy => {}
        crate::auth::Submit::Begin => {
            state.password_pending.set(true);
            state.conversing.set(true);
            set_status(status, "Verifying credentials", false);
            if !send(state, &Request::Begin { user, session }) {
                lost(state);
            }
        }
        crate::auth::Submit::Answer => {
            state.awaiting_prompt.set(false);
            let answer = take_secret(state);
            if !send(state, &Request::Response { secret: answer }) {
                lost(state);
            }
        }
    }
}

/// One frame arrived on the runner's socket. Read it and act.
///
/// Returns [`glib::ControlFlow::Break`] once the conversation is over, which
/// removes the source: after `Success` the application is quitting, and after a
/// hangup there is nothing left to read.
pub fn on_runner_event(app: &gtk::Application, state: &Rc<State>) -> glib::ControlFlow {
    let Some(conn) = state.conn.as_ref() else {
        return glib::ControlFlow::Break;
    };

    let event = match conn.borrow_mut().recv_event() {
        Ok(Some(event)) => event,
        // A clean EOF and a broken frame mean the same thing to the user.
        Ok(None) => {
            lost(state);
            return glib::ControlFlow::Break;
        }
        Err(err) => {
            eprintln!("[mgreet] protocol error: {err}");
            lost(state);
            return glib::ControlFlow::Break;
        }
    };

    match crate::auth::decide_event(event, state.password_pending.get()) {
        crate::auth::Action::AnswerWithPassword => {
            let secret = take_secret(state);
            if !send(state, &Request::Response { secret }) {
                lost(state);
                return glib::ControlFlow::Break;
            }
        }
        // A question the form cannot answer: an OTP, a new password after
        // expiry, a second factor. The password field becomes its answer box.
        crate::auth::Action::AskUser(text) => {
            state.password.set_text("");
            state.awaiting_prompt.set(true);
            broadcast(state, &text, false);
        }
        crate::auth::Action::Note(text) => broadcast(state, &text, false),
        crate::auth::Action::Warn(text) => broadcast(state, &text, true),
        crate::auth::Action::Done => {
            // Quit so the greeter compositor exits and the runner — which has
            // been holding the PAM handle all along — opens the session.
            app.quit();
            return glib::ControlFlow::Break;
        }
        crate::auth::Action::Failed(reason) => {
            state.password.set_text("");
            state.awaiting_prompt.set(false);
            state.password_pending.set(false);
            state.conversing.set(false);
            broadcast(state, &reason, true);
        }
    }
    glib::ControlFlow::Continue
}

/// Lift the password field's contents out as a scrubbing buffer and blank the
/// field. This is a root process: freed heap survives in core dumps and swap.
fn take_secret(state: &Rc<State>) -> Zeroizing<Vec<u8>> {
    let text = Zeroizing::new(state.password.text().to_string());
    state.password.set_text("");
    state.password_pending.set(false);
    Zeroizing::new(text.as_bytes().to_vec())
}

/// `false` if the socket broke. The caller tells the user.
fn send(state: &Rc<State>, request: &Request) -> bool {
    let Some(conn) = state.conn.as_ref() else {
        return false;
    };
    match conn.borrow_mut().send_request(request) {
        Ok(()) => true,
        Err(err) => {
            eprintln!("[mgreet] could not reach the session runner: {err}");
            false
        }
    }
}

/// Show `text` on every monitor's status line.
fn broadcast(state: &Rc<State>, text: &str, error: bool) {
    for label in state.status.borrow().values() {
        set_status(label, text, error);
    }
}

/// The runner is gone. Say so and stop taking input for a login that cannot
/// happen; the orchestrator will notice its child died and start a fresh one.
fn lost(state: &Rc<State>) {
    state.password.set_text("");
    state.awaiting_prompt.set(false);
    state.password_pending.set(false);
    state.conversing.set(false);
    broadcast(state, "Lost the session runner. Check the logs", true);
}

fn set_status(label: &gtk::Label, text: &str, error: bool) {
    label.set_text(text);
    if error {
        label.add_css_class("error");
    } else {
        label.remove_css_class("error");
    }
}

/// A greeter label. Stacked in the card these centre on their own line — the
/// greeting/clock/date/hostname header, the field captions, the Caps Lock
/// warning, the status line. Stated explicitly (0.5 is also GTK's default) so it
/// reads as the layout decision it is rather than an oversight. Two things stay
/// left: the text *inside* the entries and the drop-down, which are those
/// widgets' own inner nodes and never come through here. (The battery labels sit
/// in a horizontal row at natural width, so xalign doesn't reach them either.)
fn label(classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::new(None);
    l.set_xalign(0.5);
    for c in classes {
        l.add_css_class(c);
    }
    l
}

fn update_clock(greeting: &gtk::Label, clock: &gtk::Label, date: &gtk::Label) {
    let Ok(now) = glib::DateTime::now_local() else {
        return;
    };
    let g = match now.hour() {
        5..=11 => "Good morning",
        12..=16 => "Good afternoon",
        17..=20 => "Good evening",
        _ => "Good night",
    };
    greeting.set_text(g);
    if let Ok(t) = now.format("%H:%M") {
        clock.set_text(&t);
    }
    if let Ok(d) = now.format("%A, %e %B") {
        date.set_text(d.trim());
    }
}
