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

/// How long `.shake` stays on the card. Must outlast the keyframe in
/// `style.scss`, or the class is pulled while the animation is still running.
const SHAKE_MS: u64 = 420;

/// The avatar circle, in logical pixels. Also its corner radius — `.mgreet-avatar`
/// is a pill, and a square pill is a circle.
const AVATAR_PX: i32 = 84;

/// Every control on one monitor's card.
///
/// A greeter draws the same card on every output, but a failed login has to
/// shake all of them, a busy conversation has to lock all of them, and a hotplug
/// rebuild must not leave a dead one behind. One handle per monitor, keyed by
/// connector.
pub struct CardWidgets {
    pub card: gtk::Box,
    pub status: gtk::Label,
    pub username: gtk::Entry,
    pub password: gtk::Entry,
    pub sessions: gtk::DropDown,
    pub login: gtk::Button,
    pub login_label: gtk::Label,
    pub spinner: gtk::Spinner,
}

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

    let widgets = build_card(state, connector);
    let card = widgets.card.clone();
    card.set_halign(gtk::Align::Center);
    card.set_valign(gtk::Align::Center);
    state
        .cards
        .borrow_mut()
        .insert(connector.to_string(), widgets);

    let overlay = gtk::Overlay::new();
    build_backdrop(&overlay, &window, state.background.as_ref());
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

/// Put the wallpaper under the card, or the flat scrim when there is none.
///
/// The overlay's *child* is the bottom layer; everything added afterwards stacks
/// above it. So the picture goes in as the child and the dim rides over it,
/// under the card.
///
/// The dim is its own widget rather than a translucent colour on the scrim
/// because GTK's colour functions and CSS custom properties do not obviously
/// compose — `alpha(var(--bg), .55)` is not something a login screen should rest
/// on. `opacity` on a box is unambiguous, and the colour is still an M3 surface
/// token, so re-theming re-tints the dim without re-baking the image.
fn build_backdrop(overlay: &gtk::Overlay, window: &gtk::Window, background: Option<&gdk::Texture>) {
    let Some(texture) = background else {
        // Opaque, deliberately: the host compositor renders its own wallpaper
        // behind this layer surface and a greeter must never let it through.
        let scrim = gtk::Box::new(gtk::Orientation::Vertical, 0);
        scrim.add_css_class("mgreet-scrim");
        scrim.set_hexpand(true);
        scrim.set_vexpand(true);
        overlay.set_child(Some(&scrim));
        return;
    };

    window.add_css_class("has-background");

    let picture = gtk::Picture::for_paintable(texture);
    // `Cover` crops rather than letterboxes, so a 960 px landscape backdrop
    // fills a portrait monitor too. `can_shrink` keeps the picture's natural
    // size from forcing the window larger than the output.
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.set_can_shrink(true);
    picture.set_can_target(false);
    overlay.set_child(Some(&picture));

    let dim = gtk::Box::new(gtk::Orientation::Vertical, 0);
    dim.add_css_class("mgreet-dim");
    dim.set_can_target(false);
    overlay.add_overlay(&dim);
}

fn build_card(state: &Rc<State>, connector: &str) -> CardWidgets {
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

    // ── Avatar ──
    card.append(&build_avatar(state));

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

    // ── The row under the password ──
    //
    // Two things that only matter to the field above them, so they live with it:
    // the Caps Lock warning (left, appears on demand) and the keyboard layout
    // (right, always). The layout badge keeps the row a fixed height, so Caps
    // Lock coming on no longer nudges the card.
    //
    // A login screen that will not take your password is a bad place to find out
    // the machine booted `us` while your keyboard is Turkish-F.
    let password_group = gtk::Box::new(gtk::Orientation::Vertical, 6);
    password_group.append(&password);

    let meta = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let caps = label(&["mgreet-caps"]);
    caps.set_text("\u{2191} Caps Lock is on");
    caps.set_visible(false);
    caps.set_hexpand(true);
    caps.set_xalign(0.0);
    meta.append(&caps);
    if let Some(layout) = state.layout.as_deref() {
        let badge = gtk::Label::new(Some(&layout.to_uppercase()));
        badge.add_css_class("mgreet-kbd");
        badge.set_halign(gtk::Align::End);
        badge.update_property(&[gtk::accessible::Property::Label(&format!(
            "Keyboard layout {layout}"
        ))]);
        meta.append(&badge);
    }
    password_group.append(&meta);
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
    // A drop-down with one entry is a label that asks to be clicked. It stays
    // built either way — `submit_login` reads `selected()`, which is 0 — but the
    // card only shows it when there is a choice to make, or when there is
    // nothing to choose at all and the user needs to know why.
    if state.sessions.len() != 1 {
        card.append(&sessions);
    }

    // ── Status line ──
    // One conversation, many monitors: the runner's prompts and errors have to
    // land on every screen, not only the one the user happened to submit from.
    let status = label(&["mgreet-status"]);
    card.append(&status);

    // ── Log-in button ──
    // A box rather than a plain label, so the spinner can appear beside the text
    // while PAM is thinking instead of the button silently doing nothing.
    let login = gtk::Button::new();
    login.add_css_class("mgreet-login");
    let login_content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    login_content.set_halign(gtk::Align::Center);
    let spinner = gtk::Spinner::new();
    spinner.set_visible(false);
    let login_label = gtk::Label::new(Some("Log in"));
    login_content.append(&spinner);
    login_content.append(&login_label);
    login.set_child(Some(&login_content));
    card.append(&login);

    // Submit: the button, or Enter in the password field.
    let submit: Rc<dyn Fn()> = {
        let state = state.clone();
        let connector = connector.to_string();
        Rc::new(move || submit_login(&state, &connector))
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

    CardWidgets {
        card,
        status,
        username,
        password,
        sessions,
        login,
        login_label,
        spinner,
    }
}

/// The circle above the username field: the user's face, or their initial.
///
/// The picture is only shown while the typed name is the one the avatar belongs
/// to — there is a single `/var/lib/mgreet/avatar`, left by the last user to log
/// in. Type someone else's name and it becomes their monogram; clear the field
/// and the circle disappears rather than sitting there empty.
///
/// GTK's CSS `overflow` is ignored on a `GtkBox`, so the clip is set in code and
/// only the corner radius comes from the stylesheet.
fn build_avatar(state: &Rc<State>) -> gtk::Widget {
    let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
    frame.add_css_class("mgreet-avatar");
    frame.set_overflow(gtk::Overflow::Hidden);
    frame.set_size_request(AVATAR_PX, AVATAR_PX);
    frame.set_halign(gtk::Align::Center);

    let stack = gtk::Stack::new();
    stack.set_hexpand(true);
    stack.set_vexpand(true);

    let monogram = gtk::Label::new(None);
    monogram.add_css_class("mgreet-monogram");
    stack.add_named(&monogram, Some("monogram"));

    if let Some(texture) = state.avatar.as_ref() {
        let picture = gtk::Picture::for_paintable(texture);
        picture.set_content_fit(gtk::ContentFit::Cover);
        picture.set_can_shrink(true);
        picture.set_can_target(false);
        stack.add_named(&picture, Some("picture"));
    }
    frame.append(&stack);

    // Captured by value, not through `state`: the handler outlives this call and
    // hangs off the buffer that `State` owns, so borrowing `State` back into it
    // would be a cycle the greeter never breaks.
    let owner = state.avatar_owner.clone();
    let has_picture = state.avatar.is_some();
    let refresh: Rc<dyn Fn(&str)> = {
        let (frame, stack, monogram) = (frame.clone(), stack.clone(), monogram.clone());
        Rc::new(move |name: &str| {
            if has_picture && owner.as_deref() == Some(name) {
                stack.set_visible_child_name("picture");
                frame.set_visible(true);
            } else if let Some(initial) = crate::avatar::monogram(name) {
                monogram.set_text(&initial);
                stack.set_visible_child_name("monogram");
                frame.set_visible(true);
            } else {
                frame.set_visible(false);
            }
        })
    };

    refresh(&state.username.text());
    {
        let refresh = refresh.clone();
        state
            .username
            .connect_text_notify(move |buffer| refresh(&buffer.text()));
    }

    frame.upcast()
}

/// Lock the card while PAM is thinking, unlock it when PAM asks a question.
///
/// Busy means a `Begin` (or an answer) is in flight and nothing has been asked
/// back yet: there is nothing to type, and a second Enter would arrive in a
/// conversation callback that is not waiting for one. The moment the runner
/// forwards a real prompt — an OTP, a new password after expiry — the fields
/// come back, because that prompt is exactly what the user must now answer.
fn refresh_busy(state: &Rc<State>) {
    let busy = state.conversing.get() && !state.awaiting_prompt.get();
    for card in state.cards.borrow().values() {
        card.username.set_sensitive(!busy);
        card.password.set_sensitive(!busy);
        card.sessions.set_sensitive(!busy);
        card.login.set_sensitive(!busy);
        card.spinner.set_visible(busy);
        if busy {
            card.spinner.start();
        } else {
            card.spinner.stop();
        }
        card.login_label
            .set_text(if busy { "Verifying…" } else { "Log in" });
    }
}

/// Shake every card. The keyframe lives in `style.scss`; the class is pulled
/// again once it has run, so the next failure can retrigger it.
///
/// Two failures inside `SHAKE_MS` would coalesce into one shake — GTK settles
/// style once per frame, so removing and re-adding the class in the same tick is
/// not a restart. PAM takes the better part of a second to say no, so this is a
/// race nobody can lose.
fn shake(state: &Rc<State>) {
    for card in state.cards.borrow().values() {
        card.card.add_css_class("shake");
        let weak = card.card.downgrade();
        glib::timeout_add_local_once(std::time::Duration::from_millis(SHAKE_MS), move || {
            if let Some(card) = weak.upgrade() {
                card.remove_css_class("shake");
            }
        });
    }
}

/// Put the caret back where the password goes, on the monitor the user is at.
///
/// Not every monitor: `grab_focus` on each card in turn would leave the keyboard
/// wherever the iteration happened to end, which on a two-monitor greeter is a
/// coin flip. Disabling the entry (see `refresh_busy`) drops focus, so something
/// has to put it back.
fn focus_password(state: &Rc<State>) {
    let Some(connector) = state.last_submit.borrow().clone() else {
        return;
    };
    if let Some(card) = state.cards.borrow().get(&connector) {
        card.password.grab_focus();
    }
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
fn submit_login(state: &Rc<State>, connector: &str) {
    let user = state.username.text().to_string();
    let (session, status) = {
        let cards = state.cards.borrow();
        let Some(card) = cards.get(connector) else {
            return; // the monitor went away between the click and the handler
        };
        let session = state
            .sessions
            .get(card.sessions.selected() as usize)
            .map(|s| s.name.clone())
            .unwrap_or_default();
        (session, card.status.clone())
    };

    // Remember which screen asked, so a failure can hand the keyboard back to
    // the password field the user is actually looking at.
    *state.last_submit.borrow_mut() = Some(connector.to_string());

    match crate::auth::decide_submit(
        &user,
        &session,
        state.awaiting_prompt.get(),
        state.conversing.get(),
        state.real(),
    ) {
        crate::auth::Submit::Reject(msg) => set_status(&status, msg, true),
        crate::auth::Submit::Preview(msg) => {
            set_status(&status, &msg, false);
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
            refresh_busy(state);
            broadcast(state, "Verifying credentials", false);
            if !send(state, &Request::Begin { user, session }) {
                lost(state);
            }
        }
        crate::auth::Submit::Answer => {
            state.awaiting_prompt.set(false);
            refresh_busy(state);
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
            refresh_busy(state);
            focus_password(state);
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
            refresh_busy(state);
            broadcast(state, &reason, true);
            shake(state);
            focus_password(state);
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
    for card in state.cards.borrow().values() {
        set_status(&card.status, text, error);
    }
}

/// The runner is gone. Say so and stop taking input for a login that cannot
/// happen; the orchestrator will notice its child died and start a fresh one.
fn lost(state: &Rc<State>) {
    state.password.set_text("");
    state.awaiting_prompt.set(false);
    state.password_pending.set(false);
    state.conversing.set(false);
    // Unlock, or the card stays greyed out forever behind a message that says to
    // go read a log the user cannot reach from a login screen.
    refresh_busy(state);
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
