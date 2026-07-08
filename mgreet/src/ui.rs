//! The per-monitor greeter window: a fullscreen layer-shell surface with a
//! centred login card. Username/password entries share the [`State`] buffers,
//! so input on any monitor is reflected on all of them.

use gtk4 as gtk;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::rc::Rc;

use crate::State;

/// Build (and present) a greeter window pinned to `monitor`, filling it.
pub fn build_window(
    app: &gtk::Application,
    monitor: &gdk::Monitor,
    state: &Rc<State>,
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
    // Real greeter owns the keyboard exclusively; the preview (run under a live
    // session) uses OnDemand so a test run can never trap the user's input.
    window.set_keyboard_mode(if state.preview {
        KeyboardMode::OnDemand
    } else {
        KeyboardMode::Exclusive
    });
    window.set_decorated(false);

    // Dim scrim behind a centred card (Overlay stacks card over scrim).
    let scrim = gtk::Box::new(gtk::Orientation::Vertical, 0);
    scrim.add_css_class("mgreet-scrim");
    scrim.set_hexpand(true);
    scrim.set_vexpand(true);

    let card = build_card(state);
    card.set_halign(gtk::Align::Center);
    card.set_valign(gtk::Align::Center);

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&scrim));
    overlay.add_overlay(&card);
    window.set_child(Some(&overlay));

    // Escape quits the greeter (preview convenience; real mode gets a power menu).
    let key = gtk::EventControllerKey::new();
    let app_weak = app.downgrade();
    key.connect_key_pressed(move |_, keyval, _, _| {
        if keyval == gdk::Key::Escape {
            if let Some(app) = app_weak.upgrade() {
                app.quit();
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(key);

    window.present();
    window
}

fn build_card(state: &Rc<State>) -> gtk::Box {
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

    card.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    // ── Username ──
    card.append(&caption("User"));
    let username = gtk::Entry::with_buffer(&state.username);
    username.add_css_class("mgreet-field");
    username.set_placeholder_text(Some("Username"));
    username.set_hexpand(true);
    card.append(&username);

    // ── Password ──
    card.append(&caption("Password"));
    let password = gtk::Entry::with_buffer(&state.password);
    password.add_css_class("mgreet-field");
    password.set_placeholder_text(Some("Password"));
    password.set_visibility(false);
    password.set_input_purpose(gtk::InputPurpose::Password);
    password.set_hexpand(true);
    card.append(&password);

    // ── Session picker ──
    let session_names: Vec<&str> = state.sessions.iter().map(|s| s.name.as_str()).collect();
    let sessions = if session_names.is_empty() {
        gtk::DropDown::from_strings(&["No sessions"])
    } else {
        gtk::DropDown::from_strings(&session_names)
    };
    sessions.add_css_class("mgreet-session");
    sessions.set_halign(gtk::Align::Start);
    let session_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    session_row.append(&caption("Session"));
    session_row.append(&sessions);
    card.append(&session_row);

    // ── Status line ──
    let status = label(&["mgreet-status"]);
    card.append(&status);

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

/// Phase 1 stub: validate the form is filled and echo intent. Real PAM auth +
/// the mlogind credential hand-off replace the body next phase.
fn submit_login(state: &Rc<State>, status: &gtk::Label, sessions: &gtk::DropDown) {
    let user = state.username.text().to_string();
    let pass = zeroize::Zeroizing::new(state.password.text().to_string());
    let session = state
        .sessions
        .get(sessions.selected() as usize)
        .map(|s| s.name.clone())
        .unwrap_or_default();

    if user.is_empty() {
        set_status(status, "Enter a username", true);
        return;
    }
    set_status(status, &format!("Authenticating {user}…"), false);
    eprintln!(
        "[mgreet] (preview) would authenticate user={user:?} session={session:?} pass_len={}",
        pass.len()
    );
}

fn set_status(label: &gtk::Label, text: &str, error: bool) {
    label.set_text(text);
    if error {
        label.add_css_class("error");
    } else {
        label.remove_css_class("error");
    }
}

fn label(classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::new(None);
    l.set_xalign(0.0);
    for c in classes {
        l.add_css_class(c);
    }
    l
}

fn caption(text: &str) -> gtk::Label {
    let l = label(&["mgreet-caption"]);
    l.set_text(text);
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
