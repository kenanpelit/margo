//! mgreet — margo's native GTK4 login greeter.
//!
//! Renders a centred login card on EVERY connected monitor via
//! gtk4-layer-shell, hosted under a margo "greeter" compositor instance
//! (cage could never do this — it has no layer-shell). Plain gtk4-rs, no
//! relm4: a login gate must be maximally robust, and fewer layers means
//! fewer ways to abort.
//!
//! The greeter runs NO PAM of its own. It speaks `mlogind-proto` over the socket
//! the session runner leaves on `$MLOGIND_SOCK_FD`, answering the questions PAM
//! actually asks — which is what makes a fingerprint reader prompt once instead
//! of twice, and an OTP module work at all. The shared last-login cache, the
//! power-action F-key footer and a battery indicator are all live. Run
//! `mgreet --preview` under a live margo session for a non-destructive dry-run
//! (no socket, no login, power keys inert); the mlogind orchestrator runs it for
//! real via `[display] host = "gui"`.

mod auth;
mod avatar;
mod background;
mod battery;
mod cache;
mod keyboard;
mod power;
mod sessions;
mod style;
mod ui;

use gtk4 as gtk;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::PathBuf;
use std::rc::Rc;

use mlogind_proto::{Conn, FdTransport};
use sessions::Session;

/// Shared greeter state. The username/password [`gtk::EntryBuffer`]s are shared
/// by every per-monitor window, so typing on any screen updates them all — and
/// they survive a hotplug window rebuild.
pub struct State {
    pub preview: bool,
    pub username: gtk::EntryBuffer,
    pub password: gtk::EntryBuffer,
    pub sessions: Vec<Session>,

    /// The socket to the session runner. Owned here so it outlives every
    /// borrow of `conn`, which only holds its number.
    _sock: Option<OwnedFd>,
    /// The conversation. `None` → dry-run: submit echoes and quits nothing.
    pub conn: Option<RefCell<Conn<FdTransport>>>,
    /// The runner asked something the form could not answer; the password field
    /// now holds the reply rather than a password.
    pub awaiting_prompt: Cell<bool>,
    /// We still hold the password typed at submit, for PAM's first blind prompt.
    pub password_pending: Cell<bool>,
    /// A `Begin` is in flight and PAM has not asked anything yet.
    pub conversing: Cell<bool>,
    /// Every monitor's card, keyed by connector. One conversation, many
    /// monitors — and a hotplug rebuild must not leave the dead ones behind.
    pub cards: RefCell<HashMap<String, ui::CardWidgets>>,
    /// The connector whose card was last submitted from, so a failure can hand
    /// the keyboard back to the screen the user is looking at.
    pub last_submit: RefCell<Option<String>>,

    /// Last-used session name to pre-select (from the shared cache), if any.
    pub initial_session: Option<String>,
    /// Power actions for the F-key footer (from MLOGIND_POWER or a default set).
    pub power: Vec<power::PowerAction>,
    /// The blurred wallpaper the theme sync left for us, if any. Uploaded once
    /// and shared by every per-monitor window.
    pub background: Option<gdk::Texture>,
    /// The face of the user the avatar belongs to, if there is one.
    pub avatar: Option<gdk::Texture>,
    /// Whose face that is. There is one avatar file — the last user to log in —
    /// so it is drawn only while the typed name still matches this.
    pub avatar_owner: Option<String>,
    /// The keymap the greeter's compositor was started with, e.g. `tr(f)`.
    pub layout: Option<String>,
}

impl State {
    /// Is there a session runner listening? Everything destructive is gated on this.
    pub fn real(&self) -> bool {
        self.conn.is_some()
    }
}

/// Adopt the socket the session runner left on `$MLOGIND_SOCK_FD`.
///
/// A missing or unparsable value means nobody is orchestrating us — a UI dry-run.
fn runner_socket() -> Option<OwnedFd> {
    let raw: RawFd = std::env::var("MLOGIND_SOCK_FD").ok()?.parse().ok()?;
    // SAFETY: the runner passed us this descriptor and closed its own copy, so
    // we are its sole owner.
    Some(unsafe { OwnedFd::from_raw_fd(raw) })
}

fn main() -> glib::ExitCode {
    let preview = std::env::args().any(|a| a == "--preview");

    // Pin the GTK theme + dark variant BEFORE GTK initialises. The real greeter
    // runs as root with no dconf/gsettings, so GTK falls back to Adwaita *light*
    // and every node we don't explicitly style (a GtkEntry's inner `text`, the
    // drop-down popover) renders light against our dark palette — while a
    // `--preview` under the user's dark session looked fine and hid it. Forcing
    // it (not just defaulting) also makes the preview a faithful preview.
    // SAFETY: single-threaded here; GTK has not been initialised yet.
    unsafe { std::env::set_var("GTK_THEME", "Adwaita:dark") };

    // Real greeter mode: the session runner left us its end of a SOCK_SEQPACKET
    // pair on MLOGIND_SOCK_FD (atrium's CREDENTIALS_FD idiom — the fd rides
    // across exec, its number arrives in the environment). Without it — or under
    // `--preview` — this is a non-destructive UI dry-run: OnDemand keyboard, no
    // conversation, submit just echoes.
    let sock = if preview { None } else { runner_socket() };

    // Pre-fill the last user + session from the cache the runner shares with the
    // TUI greeter. Read-only: the runner writes it, on a login that succeeded.
    // A greeter has no business writing /var/cache, and under A2 — unprivileged
    // greeter — it will not be able to.
    let (cached_session, cached_user) = match sock
        .as_ref()
        .and(std::env::var_os("MLOGIND_CACHE_PATH").as_ref())
    {
        Some(path) => cache::read(std::path::Path::new(path)),
        None => (None, None),
    };

    let app = gtk::Application::builder()
        .application_id("com.margo.mgreet")
        .build();

    // Register our one flag so `--help` documents it and GApplication accepts it
    // instead of aborting with "Unknown option --preview". The value is still
    // read from argv above (before the GTK main loop) so it's available when the
    // windows are built.
    app.set_option_context_summary(Some(
        "margo's native login greeter — a login card on every connected monitor.",
    ));
    app.add_main_option(
        "preview",
        glib::Char(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Non-destructive dry-run under a live session (no PAM, no hand-off, power keys inert)",
        None,
    );

    // `connect_activate` wants an `Fn`, so the socket has to be taken out from
    // behind a cell rather than moved out of the closure's capture.
    let sock = RefCell::new(sock);
    app.connect_activate(move |app| {
        let Some(display) = gdk::Display::default() else {
            eprintln!("mgreet: no GDK display; cannot start the greeter");
            return;
        };
        let sock = sock.borrow_mut().take();
        let raw = sock.as_ref().map(|fd| fd.as_raw_fd());
        style::install(&display, matugen_css(raw.is_some()).as_deref());

        // Whose avatar `/var/lib/mgreet/avatar` is. In the real greeter that is
        // the last user to log in, which is exactly the name the cache
        // pre-fills. Under `--preview` we are reading our own `~/.face`, so it
        // is us — and the field is seeded with our name, or the preview would
        // show a monogram of the empty string and prove nothing.
        let avatar_owner = if raw.is_some() {
            cached_user.clone()
        } else {
            std::env::var("USER").ok().filter(|u| !u.is_empty())
        };
        let username = gtk::EntryBuffer::new(cached_user.as_deref().or(avatar_owner.as_deref()));

        let state = Rc::new(State {
            preview,
            background: background::load(),
            avatar: avatar::load(raw.is_some()),
            avatar_owner,
            layout: keyboard::layout(),
            username,
            password: gtk::EntryBuffer::new(None::<&str>),
            sessions: sessions::list(),
            // SAFETY: `raw` came from the `OwnedFd` moved in alongside it, and
            // `State` keeps that `OwnedFd` alive for the life of the `Conn`.
            conn: raw.map(|fd| RefCell::new(Conn::new(unsafe { FdTransport::new(fd) }))),
            _sock: sock,
            awaiting_prompt: Cell::new(false),
            password_pending: Cell::new(false),
            conversing: Cell::new(false),
            cards: RefCell::new(HashMap::new()),
            last_submit: RefCell::new(None),
            initial_session: cached_session.clone(),
            power: power::from_env(),
        });

        // Drive the conversation from the GTK main loop. GLib reports IN only
        // when a datagram is queued or the peer hung up, so the `recv` inside
        // never blocks the UI — which is exactly what the old in-process PAM
        // call did.
        if let Some(fd) = raw {
            let app = app.clone();
            let state = state.clone();
            glib::unix_fd_add_local(
                fd,
                glib::IOCondition::IN | glib::IOCondition::HUP,
                move |_, _| ui::on_runner_event(&app, &state),
            );
        }

        let windows: Rc<RefCell<HashMap<String, gtk::Window>>> =
            Rc::new(RefCell::new(HashMap::new()));

        sync_windows(app, &state, &windows);

        // Hotplug: rebuild per-monitor windows when outputs come/go. The shared
        // EntryBuffers persist across a rebuild, so any typed text survives.
        let app2 = app.clone();
        let state2 = state.clone();
        let windows2 = windows.clone();
        display.monitors().connect_items_changed(move |_, _, _, _| {
            // Defer: a freshly-added monitor often has no connector yet.
            let (app3, state3, windows3) = (app2.clone(), state2.clone(), windows2.clone());
            glib::idle_add_local_once(move || sync_windows(&app3, &state3, &windows3));
        });
    });

    app.run()
}

/// Create/destroy per-monitor greeter windows to match the live output list,
/// keyed by connector name (mirrors mshell's monitor reconcile).
fn sync_windows(
    app: &gtk::Application,
    state: &Rc<State>,
    windows: &Rc<RefCell<HashMap<String, gtk::Window>>>,
) {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let model = display.monitors();

    let mut current: Vec<(String, gdk::Monitor)> = Vec::new();
    for i in 0..model.n_items() {
        if let Some(monitor) = model
            .item(i)
            .and_then(|o| o.downcast::<gdk::Monitor>().ok())
            && let Some(connector) = monitor.connector()
        {
            current.push((connector.to_string(), monitor));
        }
    }

    let mut map = windows.borrow_mut();
    let live: Vec<String> = current.iter().map(|(c, _)| c.clone()).collect();
    let stale: Vec<String> = map.keys().filter(|c| !live.contains(c)).cloned().collect();
    for connector in stale {
        if let Some(window) = map.remove(&connector) {
            window.close();
        }
        // Its card went with it; the conversation must not keep writing to
        // widgets nobody can see.
        state.cards.borrow_mut().remove(&connector);
        // Its `Ref` is dropped before the `borrow_mut` below, deliberately: a
        // BorrowMutError here would abort the process that gates the machine.
        let was_last = state.last_submit.borrow().as_deref() == Some(connector.as_str());
        if was_last {
            *state.last_submit.borrow_mut() = None;
        }
    }
    for (connector, monitor) in current {
        map.entry(connector.clone())
            .or_insert_with(|| ui::build_window(app, &monitor, state, &connector));
    }
}

/// The matugen colours to overlay on the baked default palette, if available.
///
/// Keyed on real-greeter mode, NOT on `--preview`: the root greeter has no user
/// to borrow a theme from and reads a synced system path, while *any* run under
/// a live session is a dry-run and should look exactly like the desktop it was
/// launched from. Keying it on `--preview` meant a bare `mgreet` took the root
/// branch, found nothing, and silently rendered the baked Dracula palette —
/// which reads as "the greeter ignores my theme".
fn matugen_css(real_greeter: bool) -> Option<String> {
    if real_greeter {
        // Pre-session, and since A2 not even root: `$HOME` is 0710, so no user
        // cache is reachable. `mlogind`'s theme sync leaves a copy here.
        //
        // This used to read `/etc/mgreet/theme.css`, which nothing has ever
        // written — so the real greeter always rendered the baked Dracula
        // palette, whatever the wallpaper theme was.
        std::fs::read_to_string("/var/lib/mgreet/theme.css").ok()
    } else {
        // Under a live session: reuse the shell's cached theme so the greeter
        // matches the desktop the user just came from.
        let cache = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
        std::fs::read_to_string(cache.join("mshell").join("last_theme.css")).ok()
    }
}
