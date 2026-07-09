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
    // Real greeter owns the keyboard exclusively; the preview / dry-run (run
    // under a live session) uses OnDemand so a test run can never trap input.
    window.set_keyboard_mode(if state.greeter.is_some() {
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

    let card = build_card(app, state);
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
    let allow_escape_quit = state.greeter.is_none();
    let power = state.power.clone();
    let power_live = state.greeter.is_some();
    key.connect_key_pressed(move |_, keyval, _, _| {
        if let Some(name) = keyval.name()
            && let Some(action) = power.iter().find(|a| a.key == name.as_str())
        {
            if power_live {
                crate::power::run(action);
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

fn build_card(app: &gtk::Application, state: &Rc<State>) -> gtk::Box {
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
    if let Some(want) = state.initial_session.as_deref()
        && let Some(idx) = state.sessions.iter().position(|s| s.name == want)
    {
        sessions.set_selected(idx as u32);
    }
    card.append(&sessions);

    // ── Status line ──
    let status = label(&["mgreet-status"]);
    card.append(&status);

    // ── Log-in button ──
    let login = gtk::Button::with_label("Log in");
    login.add_css_class("mgreet-login");
    card.append(&login);

    // Submit: the button, or Enter in the password field.
    let submit: Rc<dyn Fn()> = {
        let app = app.clone();
        let state = state.clone();
        let status = status.clone();
        let sessions = sessions.clone();
        Rc::new(move || submit_login(&app, &state, &status, &sessions))
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

/// Validate the login, then either hand it off to the orchestrator (real mode)
/// or echo intent (preview). On a wrong password in real mode the greeter stays
/// up and clears the field — it never tears the compositor down for a typo.
fn submit_login(
    app: &gtk::Application,
    state: &Rc<State>,
    status: &gtk::Label,
    sessions: &gtk::DropDown,
) {
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

    let Some(greeter) = state.greeter.as_ref() else {
        // Preview / dry-run: no PAM, no hand-off, never quit.
        set_status(status, &format!("(preview) {user} · {session}"), false);
        eprintln!(
            "[mgreet] (preview) would authenticate user={user:?} session={session:?} pass_len={}",
            pass.len()
        );
        return;
    };

    if session.is_empty() {
        set_status(status, "No login session available", true);
        return;
    }

    set_status(status, &format!("Authenticating {user}…"), false);
    match crate::auth::validate(&user, &pass, &greeter.pam_service) {
        // Validated: hand the credentials to the orchestrator, then quit so the
        // greeter compositor exits and it launches the session.
        Ok(()) => match crate::handoff::write(&greeter.result_path, &user, &session, &pass) {
            Ok(()) => {
                // Remember this login for next time (shared with the TUI greeter).
                if let Some(cache_path) = greeter.cache_path.as_deref() {
                    crate::cache::write(cache_path, &session, &user);
                }
                app.quit()
            }
            Err(e) => set_status(status, &format!("Login hand-off failed: {e}"), true),
        },
        Err(msg) => {
            state.password.set_text("");
            set_status(status, &msg, true);
        }
    }
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
