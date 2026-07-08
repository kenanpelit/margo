//! mgreet — margo's native GTK4 login greeter.
//!
//! Renders a centred login card on EVERY connected monitor via
//! gtk4-layer-shell, hosted under a margo "greeter" compositor instance
//! (cage could never do this — it has no layer-shell). Plain gtk4-rs, no
//! relm4: a login gate must be maximally robust, and fewer layers means
//! fewer ways to abort.
//!
//! Phase 1 (this file): the multi-monitor UI. Run `mgreet --preview` under a
//! live margo session to see the login card appear on all outputs. Real PAM
//! auth + the mlogind orchestrator hand-off + margo greeter-mode land next.

mod auth;
mod handoff;
mod sessions;
mod style;
mod ui;

use gtk4 as gtk;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use sessions::Session;

/// Real-greeter parameters: where to write the credential hand-off and which
/// PAM service to authenticate against. `None` → preview / dry-run: no PAM, no
/// hand-off, submit never quits and never touches the session.
#[derive(Clone)]
pub struct Greeter {
    pub result_path: PathBuf,
    pub pam_service: String,
}

/// Shared greeter state. The username/password [`gtk::EntryBuffer`]s are shared
/// by every per-monitor window, so typing on any screen updates them all — and
/// they survive a hotplug window rebuild.
pub struct State {
    pub preview: bool,
    pub username: gtk::EntryBuffer,
    pub password: gtk::EntryBuffer,
    pub sessions: Vec<Session>,
    pub greeter: Option<Greeter>,
}

fn main() -> glib::ExitCode {
    let preview = std::env::args().any(|a| a == "--preview");

    // Real greeter mode: the mlogind orchestrator exports MLOGIND_RESULT_PATH
    // (the one-shot credential hand-off) and MLOGIND_PAM_SERVICE. Without the
    // hand-off path — or under `--preview` — this is a non-destructive UI
    // dry-run: OnDemand keyboard, no PAM, submit just echoes.
    let greeter = if preview {
        None
    } else {
        std::env::var_os("MLOGIND_RESULT_PATH").map(|path| Greeter {
            result_path: PathBuf::from(path),
            pam_service: std::env::var("MLOGIND_PAM_SERVICE")
                .unwrap_or_else(|_| "login".to_string()),
        })
    };

    let app = gtk::Application::builder()
        .application_id("com.margo.mgreet")
        .build();

    app.connect_activate(move |app| {
        let Some(display) = gdk::Display::default() else {
            eprintln!("mgreet: no GDK display; cannot start the greeter");
            return;
        };
        style::install(&display, matugen_css(preview).as_deref());

        let state = Rc::new(State {
            preview,
            username: gtk::EntryBuffer::new(None::<&str>),
            password: gtk::EntryBuffer::new(None::<&str>),
            sessions: sessions::list(),
            greeter: greeter.clone(),
        });

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
    }
    for (connector, monitor) in current {
        map.entry(connector)
            .or_insert_with(|| ui::build_window(app, &monitor, state));
    }
}

/// The matugen colours to overlay on the baked default palette, if available.
fn matugen_css(preview: bool) -> Option<String> {
    if preview {
        // Under a live session: reuse the shell's cached theme so the greeter
        // matches the desktop the user just came from.
        let cache = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
        std::fs::read_to_string(cache.join("mshell").join("last_theme.css")).ok()
    } else {
        // Real greeter (root, pre-session): a synced system path (later phase).
        std::fs::read_to_string("/etc/mgreet/theme.css").ok()
    }
}
