//! `mvpn menu` — the GTK4 layer-shell control panel.
//!
//! Plain gtk4 (no relm4 macro): a layer-shell window anchored top-right, themed
//! from the matugen palette cache. All `mullvad`-touching work runs on a worker
//! thread and is delivered back to the GTK main loop over an async-channel, so
//! the panel never blocks on a subprocess (no frozen UI, no main-loop `recv`).
//!
//! The layout deliberately mirrors the in-shell native VPN menu
//! (`mshellctl menu vpn`): an error header saying what is wrong with Mullvad
//! and the fix, a Mullvad / Blocky / Default mode selector, Random / Fastest /
//! My country / Add relay actions, Lockdown / Auto-connect / Quantum /
//! Multihop switches with the multihop entry country and city, an
//! anti-censorship dropdown, Favourites, Locations (countries opening to their
//! cities) and an Account section.
//!
//! The newer actions run this very binary (`mvpn <args>`, secrets on stdin)
//! with the argv the shell menu uses, decided by the shared `mvpn_proto`
//! helpers (`health::Fix`, `account`, `multihop`, `locations`), so both
//! surfaces behave the same and print the same sentences.

mod account;
mod locations;
mod theme;

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;

use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use mvpn_proto::account::{AccountAction, AccountState, Click, outcome};
use mvpn_proto::health::{ACCOUNT_URL, Account, Device, Fix, Problem};
use mvpn_proto::multihop::{self as entry_pick, Entry};

use crate::engine::relays::Location;
use crate::engine::{actions, blocky, diag, favorites, health, multihop, obf, relays, status, sys};

/// How long a first click on Log out / Revoke stays armed.
const CONFIRM_WINDOW: Duration = Duration::from_secs(5);

/// A consistent snapshot of everything the panel shows, built off-thread.
struct Snapshot {
    status: status::Status,
    favs: Vec<favorites::Fav>,
    obf_mode: String,
    lockdown: bool,
    autoconnect: bool,
    quantum: bool,
    blocky: bool,
    multihop: bool,
    entry: Entry,
    account: Account,
    problem: Option<Problem>,
    /// Only set by the leak-test button; `None` leaves the footer untouched.
    leak: Option<String>,
}

impl Snapshot {
    fn build(leak: Option<String>) -> Self {
        let h = health::gather();
        let relay = multihop::relay_settings();
        Snapshot {
            status: status::query(),
            favs: favorites::load(),
            obf_mode: obf::current(),
            lockdown: status::setting_on("lockdown-mode"),
            autoconnect: status::setting_on("auto-connect"),
            quantum: actions::quantum_on(),
            blocky: blocky::is_active(),
            multihop: relay.multihop,
            entry: Entry::parse(&relay.entry.short()),
            problem: health::diagnose(&h),
            account: h.account,
            leak,
        }
    }
}

/// What worker threads hand back to the main loop.
enum Msg {
    Snap(Box<Snapshot>),
    Locations(Vec<Location>),
    Account {
        account: Account,
        devices: Result<Vec<Device>, String>,
    },
    /// An Account-section `mvpn account …` run finished.
    AccountDone(sys::Output),
    /// Any other `mvpn` run failed: the sentence to show.
    Failed(String),
}

type Tx = async_channel::Sender<Msg>;

/// Run an engine op (if any) on a worker thread, then push a fresh snapshot.
fn kick(tx: &Tx, op: impl FnOnce() + Send + 'static) {
    let tx = tx.clone();
    std::thread::spawn(move || {
        op();
        let _ = tx.send_blocking(Msg::Snap(Box::new(Snapshot::build(None))));
    });
}

/// This binary, to run `mvpn <args>` exactly as the shell menu does. A
/// binary replaced by an upgrade reads as `… (deleted)`: then `mvpn`.
fn mvpn_exe() -> String {
    std::env::current_exe()
        .ok()
        .filter(|p| p.exists())
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_else(|| "mvpn".into())
}

/// Run `mvpn <args>` on a worker thread (`stdin`, a secret, piped in), then
/// report and push a fresh snapshot.
fn run_mvpn(tx: &Tx, args: Vec<String>, stdin: Option<String>, account: bool) {
    let tx = tx.clone();
    std::thread::spawn(move || {
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = sys::run(&mvpn_exe(), &argv, stdin.as_deref());
        if account {
            let _ = tx.send_blocking(Msg::AccountDone(out));
        } else if !out.success {
            let _ = tx.send_blocking(Msg::Failed(outcome(false, &out.stdout, &out.stderr)));
        }
        let _ = tx.send_blocking(Msg::Snap(Box::new(Snapshot::build(None))));
    });
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| s.to_string()).collect()
}

pub fn run() -> bool {
    // No GtkApplication: GtkApplication's window management interferes with
    // gtk4-layer-shell (the surface ends up a normal xdg-toplevel "popup"),
    // even with a plain gtk::Window. Drive a raw GLib main loop directly —
    // the canonical standalone layer-shell pattern.
    if gtk4::init().is_err() {
        eprintln!("mvpn: GTK init failed");
        return false;
    }
    let main_loop = glib::MainLoop::new(None, false);
    build_ui(&main_loop);
    main_loop.run();
    true
}

/// Everything the main-loop handlers touch.
struct Panel {
    tx: Tx,
    /// Guards the snapshot→widget write so programmatic `set_active` /
    /// `set_selected` don't re-trigger the switch/dropdown handlers.
    updating: Cell<bool>,
    // Header + hero.
    badge: gtk4::Label,
    relay_lbl: gtk4::Label,
    where_lbl: gtk4::Label,
    problem_box: gtk4::Box,
    problem_msg: gtk4::Label,
    problem_icon: gtk4::Image,
    problem_fixes: gtk4::Box,
    shown_problem: RefCell<Option<Problem>>,
    // Controls.
    mode_btns: [gtk4::Button; 3],
    lockdown_sw: gtk4::Switch,
    autoconnect_sw: gtk4::Switch,
    quantum_sw: gtk4::Switch,
    multihop_sw: gtk4::Switch,
    entry_row: gtk4::Box,
    entry_country: gtk4::DropDown,
    entry_city: gtk4::DropDown,
    obf_drop: gtk4::DropDown,
    // Lists.
    fav_box: gtk4::Box,
    search: gtk4::SearchEntry,
    country_box: gtk4::Box,
    account_exp: gtk4::Expander,
    account_box: gtk4::Box,
    notice: gtk4::Label,
    device_lbl: gtk4::Label,
    // State.
    countries: RefCell<Vec<Location>>,
    countries_loaded: Cell<bool>,
    expanded: RefCell<HashSet<String>>,
    entry: RefCell<Entry>,
    account: RefCell<AccountState>,
    arm: Cell<u32>,
}

fn build_ui(main_loop: &glib::MainLoop) {
    let palette = theme::load();
    let provider = gtk4::CssProvider::new();
    provider.load_from_string(&theme::css(&palette));
    if let Some(display) = gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    // A plain gtk::Window driven by a raw main loop — no GtkApplication.
    let window = gtk4::Window::new();
    window.add_css_class("mvpn");
    window.set_default_size(420, 760);

    window.init_layer_shell();
    window.set_namespace(Some("mvpn"));
    window.set_layer(Layer::Overlay);
    window.set_exclusive_zone(0);
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Right, true);
    window.set_margin(Edge::Top, 8);
    window.set_margin(Edge::Right, 8);
    // Exclusive keyboard while open so Esc closes immediately (no click needed).
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    {
        let ml = main_loop.clone();
        window.connect_close_request(move |_| {
            ml.quit();
            glib::Propagation::Proceed
        });
    }

    let key = gtk4::EventControllerKey::new();
    let win_for_key = window.clone();
    key.connect_key_pressed(move |_, k, _, _| {
        if k == gdk::Key::Escape {
            win_for_key.close();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(key);

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    root.add_css_class("mvpn-root");

    // Worker threads push messages; the main loop drains them.
    let (tx, rx) = async_channel::unbounded::<Msg>();

    // ── Header ────────────────────────────────────────────────────────
    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let title = gtk4::Label::new(Some("Mullvad VPN"));
    title.add_css_class("mvpn-title");
    title.set_hexpand(true);
    title.set_halign(gtk4::Align::Start);
    let badge = gtk4::Label::new(Some("…"));
    badge.add_css_class("mvpn-badge");
    let refresh_btn = gtk4::Button::from_icon_name("view-refresh-symbolic");
    refresh_btn.add_css_class("mvpn-action");
    a11y_name(&refresh_btn, "Refresh");
    header.append(&title);
    header.append(&badge);
    header.append(&refresh_btn);
    root.append(&header);

    // ── Error header: what is wrong with Mullvad, and the fix ─────────
    let problem_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    problem_box.add_css_class("mvpn-problem");
    problem_box.set_visible(false);
    let problem_head = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
    let problem_icon = gtk4::Image::new();
    problem_icon.set_valign(gtk4::Align::Start);
    let problem_msg = gtk4::Label::new(None);
    problem_msg.set_xalign(0.0);
    problem_msg.set_hexpand(true);
    problem_msg.set_wrap(true);
    problem_msg.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
    problem_head.append(&problem_icon);
    problem_head.append(&problem_msg);
    let problem_fixes = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    problem_box.append(&problem_head);
    problem_box.append(&problem_fixes);
    root.append(&problem_box);

    // ── Hero (current relay + location) ───────────────────────────────
    let hero = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    hero.add_css_class("mvpn-hero");
    let relay_lbl = gtk4::Label::new(Some("—"));
    relay_lbl.add_css_class("mvpn-relay");
    relay_lbl.set_halign(gtk4::Align::Start);
    let where_lbl = gtk4::Label::new(Some(""));
    where_lbl.add_css_class("mvpn-dim");
    where_lbl.set_halign(gtk4::Align::Start);
    hero.append(&relay_lbl);
    hero.append(&where_lbl);
    root.append(&hero);

    // Everything below scrolls as one (the window's height bounds it).
    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    let body_scroll = gtk4::ScrolledWindow::new();
    body_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    body_scroll.set_vexpand(true);
    // A floor so the panel opens tall enough to use (the lists used to
    // carry their own minimum heights).
    body_scroll.set_min_content_height(520);
    body_scroll.set_child(Some(&body));
    root.append(&body_scroll);

    // ── Mode selector: Mullvad / Blocky / Default ─────────────────────
    let mode_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    mode_row.set_homogeneous(true);
    let mullvad_btn = mode_btn("Mullvad");
    let blocky_btn = mode_btn("Blocky");
    let default_btn = mode_btn("Default");
    mode_row.append(&mullvad_btn);
    mode_row.append(&blocky_btn);
    mode_row.append(&default_btn);
    body.append(&mode_row);

    // ── Relay actions: Random / Fastest / My country / Add ────────────
    let actions_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    actions_row.set_homogeneous(true);
    let random_btn = mode_btn("Random");
    let fastest_btn = mode_btn("Fastest");
    let my_country_btn = mode_btn("My country");
    my_country_btn.set_tooltip_text(Some(
        "Connect in the country you are in, or the nearest one with relays",
    ));
    let add_btn = mode_btn("Add");
    actions_row.append(&random_btn);
    actions_row.append(&fastest_btn);
    actions_row.append(&my_country_btn);
    actions_row.append(&add_btn);
    body.append(&actions_row);

    // ── Toggles ───────────────────────────────────────────────────────
    let lockdown_sw = toggle_row(&body, "Lockdown mode", "Block traffic when the VPN drops");
    let autoconnect_sw = toggle_row(&body, "Auto-connect", "Bring the tunnel up on daemon start");
    let quantum_sw = toggle_row(&body, "Quantum-resistant", "Post-quantum key exchange");
    let multihop_sw = toggle_row(&body, "Multihop", "Enter through a second relay");

    // ── Multihop entry: country, then optionally a city ───────────────
    let entry_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    entry_row.add_css_class("mvpn-card");
    let entry_lbl = gtk4::Label::new(Some("Entry"));
    entry_lbl.set_hexpand(true);
    entry_lbl.set_halign(gtk4::Align::Start);
    let entry_country = gtk4::DropDown::from_strings(&["Any country"]);
    let entry_city = gtk4::DropDown::from_strings(&["Any city"]);
    a11y_name(&entry_country, "Multihop entry country");
    a11y_name(&entry_city, "Multihop entry city");
    entry_row.append(&entry_lbl);
    entry_row.append(&entry_country);
    entry_row.append(&entry_city);
    entry_row.set_sensitive(false);
    body.append(&entry_row);

    // ── Anti-censorship (obfuscation) ─────────────────────────────────
    let obf_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    obf_row.add_css_class("mvpn-card");
    let obf_texts = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    obf_texts.set_hexpand(true);
    let obf_title = gtk4::Label::new(Some("Anti-censorship"));
    obf_title.set_halign(gtk4::Align::Start);
    let obf_desc = dim("Obfuscate the tunnel on hostile networks");
    obf_texts.append(&obf_title);
    obf_texts.append(&obf_desc);
    let obf_drop = gtk4::DropDown::from_strings(obf::CYCLE);
    obf_drop.set_valign(gtk4::Align::Center);
    obf_row.append(&obf_texts);
    obf_row.append(&obf_drop);
    body.append(&obf_row);

    // ── Favorites ─────────────────────────────────────────────────────
    body.append(&section_label("Favorites"));
    let fav_box = list_box();
    body.append(&fav_box);

    // ── Locations: countries opening to their cities ──────────────────
    body.append(&section_label("Locations"));
    let search = gtk4::SearchEntry::new();
    search.add_css_class("mvpn-search");
    body.append(&search);
    let country_box = list_box();
    body.append(&country_box);

    // ── Account ───────────────────────────────────────────────────────
    let account_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    let account_exp = gtk4::Expander::new(Some("Account"));
    account_exp.add_css_class("mvpn-title");
    account_exp.set_child(Some(&account_box));
    body.append(&account_exp);

    // ── Footer ────────────────────────────────────────────────────────
    let notice = dim("");
    notice.add_css_class("mvpn-error");
    notice.set_visible(false);
    root.append(&notice);
    let footer = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let device_lbl = dim("");
    device_lbl.set_hexpand(true);
    let test_btn = gtk4::Button::with_label("Leak test");
    test_btn.add_css_class("mvpn-action");
    footer.append(&device_lbl);
    footer.append(&test_btn);
    root.append(&footer);

    window.set_child(Some(&root));

    let p = Rc::new(Panel {
        tx: tx.clone(),
        updating: Cell::new(false),
        badge,
        relay_lbl,
        where_lbl,
        problem_box,
        problem_msg,
        problem_icon,
        problem_fixes,
        shown_problem: RefCell::new(None),
        mode_btns: [mullvad_btn.clone(), blocky_btn.clone(), default_btn.clone()],
        lockdown_sw,
        autoconnect_sw,
        quantum_sw,
        multihop_sw,
        entry_row,
        entry_country,
        entry_city,
        obf_drop,
        fav_box,
        search,
        country_box,
        account_exp,
        account_box,
        notice,
        device_lbl,
        countries: RefCell::new(Vec::new()),
        countries_loaded: Cell::new(false),
        expanded: RefCell::new(HashSet::new()),
        entry: RefCell::new(Entry::default()),
        account: RefCell::new(AccountState::default()),
        arm: Cell::new(0),
    });

    // ── Wire mode selector ────────────────────────────────────────────
    // Mullvad = bring the tunnel up; Blocky = start the DNS guard; Default =
    // drop the tunnel and stop blocky (back to the plain system path).
    wire(&mullvad_btn, &tx, || {
        actions::connect();
    });
    wire(&blocky_btn, &tx, || {
        blocky::ensure();
        blocky::start();
    });
    wire(&default_btn, &tx, || {
        actions::disconnect();
        blocky::stop();
    });

    // ── Wire relay actions ────────────────────────────────────────────
    wire(&random_btn, &tx, || {
        actions::random("", "", relays::Ownership::Any);
    });
    wire(&fastest_btn, &tx, || {
        favorites::fastest("", 10, 3, 2, true);
    });
    wire(&add_btn, &tx, || {
        favorites::add_current();
    });
    wire(&refresh_btn, &tx, || {});
    {
        let p = p.clone();
        my_country_btn.connect_clicked(move |_| p.run(strings(&["my-country"])));
    }

    // ── Leak test (its own thread, doesn't disturb the rest) ──────────
    {
        let tx2 = tx.clone();
        test_btn.connect_clicked(move |_| {
            let tx3 = tx2.clone();
            std::thread::spawn(move || {
                let r = diag::leak_test();
                let s = if !r.connected {
                    format!("○ not connected ({})", r.exit_ip)
                } else if r.mullvad_exit {
                    format!("✔ secure · {}", r.exit_ip)
                } else {
                    format!("✘ LEAK · {}", r.exit_ip)
                };
                let _ = tx3.send_blocking(Msg::Snap(Box::new(Snapshot::build(Some(s)))));
            });
        });
    }

    // ── Wire switches ─────────────────────────────────────────────────
    p.on_switch(&p.lockdown_sw, |on| {
        actions::set_lockdown(on);
    });
    p.on_switch(&p.autoconnect_sw, |on| {
        actions::set_autoconnect(on);
    });
    p.on_switch(&p.quantum_sw, |on| {
        // No direct setter — flip only when it differs.
        if on != actions::quantum_on() {
            actions::toggle_quantum();
        }
    });
    {
        let p2 = p.clone();
        p.multihop_sw.connect_state_set(move |_, on| {
            if !p2.updating.get() {
                p2.run(strings(&["multihop", if on { "on" } else { "off" }]));
            }
            glib::Propagation::Proceed
        });
    }

    // ── Wire multihop entry pickers ───────────────────────────────────
    {
        let p2 = p.clone();
        p.entry_country.connect_selected_notify(move |d| {
            if p2.updating.get() {
                return;
            }
            let args = entry_pick::entry_args(&p2.countries.borrow(), d.selected(), 0);
            p2.fill_cities(d.selected(), 0);
            p2.run(args);
        });
    }
    {
        let p2 = p.clone();
        p.entry_city.connect_selected_notify(move |d| {
            if p2.updating.get() {
                return;
            }
            let country = p2.entry_country.selected();
            p2.run(entry_pick::entry_args(
                &p2.countries.borrow(),
                country,
                d.selected(),
            ));
        });
    }

    // ── Wire anti-censorship dropdown ─────────────────────────────────
    {
        let p2 = p.clone();
        p.obf_drop.connect_selected_notify(move |d| {
            if p2.updating.get() {
                return;
            }
            if let Some(mode) = obf::CYCLE.get(d.selected() as usize) {
                let mode = mode.to_string();
                kick(&p2.tx, move || {
                    obf::set(&mode);
                });
            }
        });
    }

    // ── Wire location search + account section ────────────────────────
    {
        let p2 = p.clone();
        p.search
            .connect_search_changed(move |_| p2.rebuild_locations());
    }
    {
        let p2 = p.clone();
        p.account_exp.connect_expanded_notify(move |e| {
            if e.is_expanded() {
                p2.account.borrow_mut().notice = None;
                p2.load_account();
            }
        });
    }

    // ── Relay locations: load once off-thread ─────────────────────────
    {
        let tx2 = tx.clone();
        std::thread::spawn(move || {
            let _ = tx2.send_blocking(Msg::Locations(relays::locations()));
        });
    }
    p.rebuild_locations();

    // ── Receive messages on the main loop, update widgets ─────────────
    {
        let p = p.clone();
        glib::spawn_future_local(async move {
            while let Ok(msg) = rx.recv().await {
                p.handle(msg);
            }
        });
    }

    // Initial load + periodic refresh.
    kick(&tx, || {});
    let tx_timer = tx.clone();
    glib::timeout_add_seconds_local(5, move || {
        kick(&tx_timer, || {});
        glib::ControlFlow::Continue
    });

    window.present();
}

impl Panel {
    /// Run `mvpn <args>` (not an Account action), clearing an old failure.
    fn run(&self, args: Vec<String>) {
        self.notice.set_visible(false);
        run_mvpn(&self.tx, args, None, false);
    }

    fn on_switch(self: &Rc<Self>, sw: &gtk4::Switch, op: fn(bool)) {
        let p = self.clone();
        sw.connect_state_set(move |_, on| {
            if !p.updating.get() {
                kick(&p.tx, move || op(on));
            }
            glib::Propagation::Proceed
        });
    }

    fn handle(self: &Rc<Self>, msg: Msg) {
        match msg {
            Msg::Snap(s) => self.apply(&s),
            Msg::Locations(list) => {
                *self.countries.borrow_mut() = list;
                self.countries_loaded.set(true);
                self.rebuild_locations();
                self.show_entry(true);
            }
            Msg::Account { account, devices } => {
                {
                    let mut st = self.account.borrow_mut();
                    st.account = Some(account);
                    st.devices = Some(devices);
                }
                self.rebuild_account();
            }
            Msg::AccountDone(out) => {
                self.account
                    .borrow_mut()
                    .finish(out.success, &out.stdout, &out.stderr);
                self.rebuild_account();
                self.load_account();
            }
            Msg::Failed(text) => {
                self.notice.set_text(&text);
                self.notice.set_visible(true);
            }
        }
    }

    /// Write a snapshot into the widgets.
    fn apply(self: &Rc<Self>, s: &Snapshot) {
        self.updating.set(true);
        let connected = s.status.connected;
        self.badge.set_text(if connected {
            "Active"
        } else if s.status.connecting {
            "…"
        } else {
            "Inactive"
        });
        self.badge.remove_css_class("ok");
        if connected {
            self.badge.add_css_class("ok");
        }
        self.relay_lbl.set_text(if s.status.relay.is_empty() {
            "Not connected"
        } else {
            &s.status.relay
        });
        let mut sub = String::new();
        if !s.status.country.is_empty() {
            sub = if s.status.city.is_empty() {
                s.status.country.clone()
            } else {
                format!("{}, {}", s.status.city, s.status.country)
            };
        }
        if !s.status.tunnel_type.is_empty() {
            sub = format!("{sub} · {}", s.status.tunnel_type);
        }
        self.where_lbl.set_text(&sub);

        // Highlight the active mode: Blocky wins (it implies a DNS override
        // on top), else Mullvad when connected, else Default.
        for b in &self.mode_btns {
            b.remove_css_class("selected");
        }
        let active = if s.blocky {
            1
        } else if connected {
            0
        } else {
            2
        };
        self.mode_btns[active].add_css_class("selected");

        self.lockdown_sw.set_active(s.lockdown);
        self.autoconnect_sw.set_active(s.autoconnect);
        self.quantum_sw.set_active(s.quantum);
        self.multihop_sw.set_active(s.multihop);
        self.entry_row.set_sensitive(s.multihop);
        if let Some(i) = obf::CYCLE.iter().position(|m| *m == s.obf_mode) {
            self.obf_drop.set_selected(i as u32);
        }
        self.updating.set(false);

        if *self.entry.borrow() != s.entry {
            *self.entry.borrow_mut() = s.entry.clone();
            self.show_entry(false);
        }

        if let Some(leak) = &s.leak {
            self.device_lbl.set_text(leak);
        } else {
            self.device_lbl.set_text(&match &s.account {
                Account::LoggedIn { device, expiry, .. } if !expiry.is_empty() => {
                    format!("Device: {device} · paid until {expiry}")
                }
                Account::LoggedIn { device, .. } => format!("Device: {device}"),
                _ => String::new(),
            });
        }
        // A login / logout elsewhere: the open section follows.
        if self.account_exp.is_expanded()
            && let Some(shown) = &self.account.borrow().account
            && std::mem::discriminant(shown) != std::mem::discriminant(&s.account)
            && s.account != Account::Unknown
        {
            self.load_account();
        }
        self.show_problem(s.problem.clone());
        rebuild_favs(&self.fav_box, &s.favs, &self.tx);
    }

    /// Show the error header for `problem` (hidden for `None`); unchanged,
    /// it rebuilds nothing.
    fn show_problem(self: &Rc<Self>, problem: Option<Problem>) {
        if *self.shown_problem.borrow() == problem {
            return;
        }
        let Some(pr) = &problem else {
            self.problem_box.set_visible(false);
            *self.shown_problem.borrow_mut() = None;
            return;
        };
        for class in ["calm", "warn", "danger"] {
            self.problem_box.remove_css_class(class);
        }
        self.problem_box.add_css_class(pr.severity.class());
        self.problem_icon.set_icon_name(Some(pr.severity.icon()));
        self.problem_msg.set_text(&pr.message);
        clear_box(&self.problem_fixes);
        for &fix in &pr.fixes {
            let b = gtk4::Button::with_label(fix.label());
            b.add_css_class("mvpn-action");
            let p = self.clone();
            b.connect_clicked(move |_| p.fix(fix));
            self.problem_fixes.append(&b);
        }
        self.problem_fixes.set_visible(!pr.fixes.is_empty());
        self.problem_box.set_visible(true);
        *self.shown_problem.borrow_mut() = problem;
    }

    fn fix(self: &Rc<Self>, fix: Fix) {
        match fix {
            Fix::ShowAccount => self.account_exp.set_expanded(true),
            // Try the saved number (env / pass); the section shows why not.
            Fix::Login => {
                self.account_exp.set_expanded(true);
                self.account_click(AccountAction::Login(String::new()));
            }
            Fix::OpenAccountPage => self.open_account_page(),
            other => {
                if let Some(args) = other.mvpn_args() {
                    self.run(strings(args));
                }
            }
        }
    }

    fn open_account_page(&self) {
        let notice = self.notice.clone();
        match std::process::Command::new("xdg-open")
            .arg(ACCOUNT_URL)
            .spawn()
        {
            // Reap it off the GTK thread.
            Ok(mut child) => {
                std::thread::spawn(move || child.wait());
            }
            Err(_) => {
                notice.set_text(&format!("Open {ACCOUNT_URL} in a browser (no xdg-open)."));
                notice.set_visible(true);
            }
        }
    }

    fn rebuild_locations(self: &Rc<Self>) {
        let p = self.clone();
        locations::rebuild(
            &self.country_box,
            &self.countries.borrow(),
            self.countries_loaded.get(),
            &self.search.text(),
            &self.expanded.borrow(),
            Rc::new(move |click| match click {
                locations::Click::Toggle(code) => {
                    {
                        let mut open = p.expanded.borrow_mut();
                        if !open.remove(&code) {
                            open.insert(code);
                        }
                    }
                    // Rebuilding drops the clicked row: do it after the click.
                    let p = p.clone();
                    glib::idle_add_local_once(move || p.rebuild_locations());
                }
                locations::Click::Connect(args) => p.run(args),
            }),
        );
    }

    /// Point the entry dropdowns at the current entry; `refill` reloads
    /// the country list too (the relay list arrived).
    fn show_entry(&self, refill: bool) {
        let countries = self.countries.borrow();
        let (country, city) = entry_pick::selection(&countries, &self.entry.borrow());
        self.updating.set(true);
        if refill {
            let labels = entry_pick::country_labels(&countries);
            let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
            self.entry_country
                .set_model(Some(&gtk4::StringList::new(&labels)));
        }
        self.entry_country.set_selected(country);
        self.updating.set(false);
        drop(countries);
        self.fill_cities(country, city);
    }

    /// Fill the city dropdown for the country at `country` and select `city`.
    fn fill_cities(&self, country: u32, city: u32) {
        let labels = entry_pick::city_labels(&self.countries.borrow(), country);
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        self.updating.set(true);
        self.entry_city
            .set_model(Some(&gtk4::StringList::new(&labels)));
        self.entry_city.set_selected(city);
        self.entry_city.set_sensitive(labels.len() > 1);
        self.updating.set(false);
    }

    /// Read the Account section's data off-thread.
    fn load_account(&self) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let account = crate::engine::account::query();
            let devices = match &account {
                Account::LoggedIn { .. } => crate::engine::account::devices(),
                _ => Ok(Vec::new()),
            };
            let _ = tx.send_blocking(Msg::Account { account, devices });
        });
    }

    fn rebuild_account(self: &Rc<Self>) {
        let p = self.clone();
        account::rebuild(
            &self.account_box,
            &self.account.borrow(),
            // Rebuilding drops the clicked button: act after the click.
            Rc::new(move |action| {
                let p = p.clone();
                glib::idle_add_local_once(move || p.account_click(action));
            }),
        );
    }

    /// Run, arm or open an Account-section click (`mvpn_proto::account`).
    fn account_click(self: &Rc<Self>, action: AccountAction) {
        let click = self.account.borrow_mut().click(action);
        match click {
            Click::Run { args, stdin } => {
                self.rebuild_account();
                run_mvpn(&self.tx, args, stdin, true);
            }
            Click::Armed => {
                self.arm.set(self.arm.get() + 1);
                let arm = self.arm.get();
                self.rebuild_account();
                let p = self.clone();
                glib::timeout_add_local_once(CONFIRM_WINDOW, move || {
                    if p.arm.get() == arm && p.account.borrow_mut().disarm() {
                        p.rebuild_account();
                    }
                });
            }
            Click::OpenPage => self.open_account_page(),
            Click::None => {}
        }
    }
}

// ── Small builders ────────────────────────────────────────────────────

fn wire(btn: &gtk4::Button, tx: &Tx, op: fn()) {
    let tx2 = tx.clone();
    btn.connect_clicked(move |_| kick(&tx2, op));
}

fn a11y_name(w: &impl IsA<gtk4::Accessible>, name: &str) {
    w.update_property(&[gtk4::accessible::Property::Label(name)]);
}

fn mode_btn(label: &str) -> gtk4::Button {
    let b = gtk4::Button::with_label(label);
    b.add_css_class("mvpn-action");
    b.add_css_class("mvpn-mode");
    b
}

fn section_label(text: &str) -> gtk4::Label {
    let l = gtk4::Label::new(Some(text));
    l.add_css_class("mvpn-title");
    l.set_halign(gtk4::Align::Start);
    l
}

/// A dimmed, wrapping, start-aligned line.
fn dim(text: &str) -> gtk4::Label {
    let l = gtk4::Label::new(Some(text));
    l.add_css_class("mvpn-dim");
    l.set_halign(gtk4::Align::Start);
    l.set_xalign(0.0);
    l.set_wrap(true);
    l
}

fn list_box() -> gtk4::Box {
    let b = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    b.add_css_class("mvpn-card");
    b
}

fn toggle_row(parent: &gtk4::Box, title: &str, desc: &str) -> gtk4::Switch {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    row.add_css_class("mvpn-card");
    let texts = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    texts.set_hexpand(true);
    let t = gtk4::Label::new(Some(title));
    t.set_halign(gtk4::Align::Start);
    texts.append(&t);
    texts.append(&dim(desc));
    let sw = gtk4::Switch::new();
    sw.set_valign(gtk4::Align::Center);
    a11y_name(&sw, title);
    row.append(&texts);
    row.append(&sw);
    parent.append(&row);
    sw
}

fn clear_box(b: &gtk4::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

/// A clickable row button: `[KEY] main … trailing`. The click op is wired by
/// the caller (favorites connect by relay id, locations by country / city).
fn relay_row(main: &str, key: &str, trailing: &str) -> gtk4::Button {
    let b = gtk4::Button::new();
    b.add_css_class("mvpn-action");
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
    if !key.is_empty() {
        let k = gtk4::Label::new(Some(key));
        k.add_css_class("mvpn-key");
        row.append(&k);
    }
    let m = gtk4::Label::new(Some(main));
    m.set_halign(gtk4::Align::Start);
    m.set_hexpand(true);
    m.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    row.append(&m);
    if !trailing.is_empty() {
        let t = gtk4::Label::new(Some(trailing));
        t.add_css_class("mvpn-ping");
        row.append(&t);
    }
    b.set_child(Some(&row));
    b
}

fn rebuild_favs(b: &gtk4::Box, favs: &[favorites::Fav], tx: &Tx) {
    clear_box(b);
    if favs.is_empty() {
        b.append(&dim("No favorites yet — connect, then ‘Add’."));
        return;
    }
    for f in favs {
        let ping = f.ping.map(|p| format!("{p:.0} ms")).unwrap_or_default();
        let row = relay_row(&f.relay, "", &ping);
        let (tx2, relay) = (tx.clone(), f.relay.clone());
        row.connect_clicked(move |_| {
            let relay = relay.clone();
            kick(&tx2, move || {
                actions::set_relay(&relay);
            });
        });
        b.append(&row);
    }
}
