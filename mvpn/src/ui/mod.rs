//! `mvpn menu` — the GTK4 layer-shell control panel.
//!
//! Plain gtk4 (no relm4 macro): a layer-shell window anchored top-right, themed
//! from the matugen palette cache. All `mullvad`-touching work runs on a worker
//! thread and is delivered back to the GTK main loop over an async-channel, so
//! the panel never blocks on a subprocess (no frozen UI, no main-loop `recv`).
//!
//! The layout deliberately mirrors the in-shell native VPN menu
//! (`mshellctl menu vpn`): a Mullvad / Blocky / Default mode selector on top,
//! a Random / Fastest / Add relay-action row, Lockdown / Auto-connect / Quantum
//! switches, an anti-censorship dropdown, then collapsing Favourites + Locations.

mod theme;

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;

use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::engine::{actions, blocky, diag, favorites, obf, relays, slot, status};

/// A consistent snapshot of everything the panel shows, built off-thread.
struct Snapshot {
    status: status::Status,
    favs: Vec<favorites::Fav>,
    obf_mode: String,
    device: String,
    expiry: String,
    lockdown: bool,
    autoconnect: bool,
    quantum: bool,
    blocky: bool,
    /// Only set by the leak-test button; `None` leaves the footer untouched.
    leak: Option<String>,
}

impl Snapshot {
    fn build(leak: Option<String>) -> Self {
        Snapshot {
            status: status::query(),
            favs: favorites::load(),
            obf_mode: obf::current(),
            device: slot::current_device(),
            expiry: status::account_expiry(),
            lockdown: status::setting_on("lockdown-mode"),
            autoconnect: status::setting_on("auto-connect"),
            quantum: actions::quantum_on(),
            blocky: blocky::is_active(),
            leak,
        }
    }
}

type Tx = async_channel::Sender<Snapshot>;

/// Run an engine op (if any) on a worker thread, then push a fresh snapshot.
fn kick(tx: &Tx, op: impl FnOnce() + Send + 'static) {
    let tx = tx.clone();
    std::thread::spawn(move || {
        op();
        let _ = tx.send_blocking(Snapshot::build(None));
    });
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

    // Guards the snapshot→widget write so programmatic `set_active`/`set_selected`
    // don't re-trigger the switch/dropdown handlers (a feedback loop).
    let updating = Rc::new(RefCell::new(false));

    // Worker threads push fresh snapshots; the main loop drains them.
    let (tx, rx) = async_channel::unbounded::<Snapshot>();

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
    header.append(&title);
    header.append(&badge);
    header.append(&refresh_btn);
    root.append(&header);

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

    // ── Mode selector: Mullvad / Blocky / Default ─────────────────────
    let mode_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    mode_row.set_homogeneous(true);
    let mullvad_btn = mode_btn("Mullvad");
    let blocky_btn = mode_btn("Blocky");
    let default_btn = mode_btn("Default");
    mode_row.append(&mullvad_btn);
    mode_row.append(&blocky_btn);
    mode_row.append(&default_btn);
    root.append(&mode_row);

    // ── Relay actions: Random / Fastest / Add ─────────────────────────
    let actions_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    actions_row.set_homogeneous(true);
    let random_btn = mode_btn("Random");
    let fastest_btn = mode_btn("Fastest");
    let add_btn = mode_btn("Add");
    actions_row.append(&random_btn);
    actions_row.append(&fastest_btn);
    actions_row.append(&add_btn);
    root.append(&actions_row);

    // ── Toggles ───────────────────────────────────────────────────────
    let lockdown_sw = toggle_row(&root, "Lockdown mode", "Block traffic when the VPN drops");
    let autoconnect_sw = toggle_row(&root, "Auto-connect", "Bring the tunnel up on daemon start");
    let quantum_sw = toggle_row(&root, "Quantum-resistant", "Post-quantum key exchange");

    // ── Anti-censorship (obfuscation) ─────────────────────────────────
    let obf_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    obf_row.add_css_class("mvpn-card");
    let obf_texts = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    obf_texts.set_hexpand(true);
    let obf_title = gtk4::Label::new(Some("Anti-censorship"));
    obf_title.set_halign(gtk4::Align::Start);
    let obf_desc = gtk4::Label::new(Some("Obfuscate the tunnel on hostile networks"));
    obf_desc.add_css_class("mvpn-dim");
    obf_desc.set_halign(gtk4::Align::Start);
    obf_texts.append(&obf_title);
    obf_texts.append(&obf_desc);
    let obf_drop = gtk4::DropDown::from_strings(obf::CYCLE);
    obf_drop.set_valign(gtk4::Align::Center);
    obf_row.append(&obf_texts);
    obf_row.append(&obf_drop);
    root.append(&obf_row);

    // ── Favorites ─────────────────────────────────────────────────────
    root.append(&section_label("Favorites"));
    let fav_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    root.append(&scroller(&fav_box, 130));

    // ── Country search ────────────────────────────────────────────────
    root.append(&section_label("Locations"));
    let search = gtk4::SearchEntry::new();
    search.add_css_class("mvpn-search");
    root.append(&search);
    let country_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    let country_scroll = scroller(&country_box, 200);
    country_scroll.set_vexpand(true);
    root.append(&country_scroll);

    // ── Footer ────────────────────────────────────────────────────────
    let footer = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let device_lbl = gtk4::Label::new(Some(""));
    device_lbl.add_css_class("mvpn-dim");
    device_lbl.set_halign(gtk4::Align::Start);
    device_lbl.set_hexpand(true);
    let test_btn = gtk4::Button::with_label("Leak test");
    test_btn.add_css_class("mvpn-action");
    footer.append(&device_lbl);
    footer.append(&test_btn);
    root.append(&footer);

    window.set_child(Some(&root));

    let countries: Rc<RefCell<Vec<relays::Country>>> = Rc::new(RefCell::new(Vec::new()));

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

    // ── Leak test (separate channel, doesn't disturb the rest) ────────
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
                let _ = tx3.send_blocking(Snapshot::build(Some(s)));
            });
        });
    }

    // ── Wire switches ─────────────────────────────────────────────────
    {
        let (tx2, up) = (tx.clone(), updating.clone());
        lockdown_sw.connect_state_set(move |_, on| {
            if !*up.borrow() {
                kick(&tx2, move || {
                    actions::set_lockdown(on);
                });
            }
            glib::Propagation::Proceed
        });
    }
    {
        let (tx2, up) = (tx.clone(), updating.clone());
        autoconnect_sw.connect_state_set(move |_, on| {
            if !*up.borrow() {
                kick(&tx2, move || {
                    actions::set_autoconnect(on);
                });
            }
            glib::Propagation::Proceed
        });
    }
    {
        let (tx2, up) = (tx.clone(), updating.clone());
        quantum_sw.connect_state_set(move |_, on| {
            if !*up.borrow() {
                kick(&tx2, move || {
                    // No direct setter — flip only when it differs.
                    if on != actions::quantum_on() {
                        actions::toggle_quantum();
                    }
                });
            }
            glib::Propagation::Proceed
        });
    }

    // ── Wire anti-censorship dropdown ─────────────────────────────────
    {
        let (tx2, up) = (tx.clone(), updating.clone());
        obf_drop.connect_selected_notify(move |d| {
            if *up.borrow() {
                return;
            }
            let idx = d.selected() as usize;
            if let Some(mode) = obf::CYCLE.get(idx) {
                let mode = mode.to_string();
                kick(&tx2, move || {
                    obf::set(&mode);
                });
            }
        });
    }

    // ── Wire country search ───────────────────────────────────────────
    {
        let (cs, cb, tx2) = (countries.clone(), country_box.clone(), tx.clone());
        search.connect_search_changed(move |e| {
            rebuild_countries(&cb, &cs.borrow(), &e.text(), &tx2);
        });
    }

    // ── Country catalog: load once off-thread, deliver over a channel ──
    let (ctx, crx) = async_channel::unbounded::<Vec<relays::Country>>();
    {
        std::thread::spawn(move || {
            let _ = ctx.send_blocking(relays::countries());
        });
        let (cs, cb, tx2, se) = (
            countries.clone(),
            country_box.clone(),
            tx.clone(),
            search.clone(),
        );
        glib::spawn_future_local(async move {
            if let Ok(list) = crx.recv().await {
                *cs.borrow_mut() = list;
                rebuild_countries(&cb, &cs.borrow(), &se.text(), &tx2);
            }
        });
    }

    // ── Receive snapshots on the main loop, update widgets ────────────
    {
        let up = updating.clone();
        let tx2 = tx.clone();
        let (badge, relay_lbl, where_lbl) = (badge.clone(), relay_lbl.clone(), where_lbl.clone());
        let (mullvad_btn, blocky_btn, default_btn) =
            (mullvad_btn.clone(), blocky_btn.clone(), default_btn.clone());
        let (lockdown_sw2, autoconnect_sw2, quantum_sw2, obf_drop2) = (
            lockdown_sw.clone(),
            autoconnect_sw.clone(),
            quantum_sw.clone(),
            obf_drop.clone(),
        );
        let (fav_box2, device_lbl2) = (fav_box.clone(), device_lbl.clone());
        glib::spawn_future_local(async move {
            while let Ok(s) = rx.recv().await {
                *up.borrow_mut() = true;
                let connected = s.status.connected;
                badge.set_text(if connected {
                    "Active"
                } else if s.status.connecting {
                    "…"
                } else {
                    "Inactive"
                });
                badge.remove_css_class("ok");
                if connected {
                    badge.add_css_class("ok");
                }
                relay_lbl.set_text(if s.status.relay.is_empty() {
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
                where_lbl.set_text(&sub);

                // Highlight the active mode: Blocky wins (it implies a DNS
                // override on top), else Mullvad when connected, else Default.
                for b in [&mullvad_btn, &blocky_btn, &default_btn] {
                    b.remove_css_class("selected");
                }
                let active = if s.blocky {
                    &blocky_btn
                } else if connected {
                    &mullvad_btn
                } else {
                    &default_btn
                };
                active.add_css_class("selected");

                lockdown_sw2.set_active(s.lockdown);
                autoconnect_sw2.set_active(s.autoconnect);
                quantum_sw2.set_active(s.quantum);
                if let Some(i) = obf::CYCLE.iter().position(|m| *m == s.obf_mode) {
                    obf_drop2.set_selected(i as u32);
                }

                if let Some(leak) = &s.leak {
                    device_lbl2.set_text(leak);
                } else {
                    let exp = if s.expiry.is_empty() || s.expiry == "—" {
                        String::new()
                    } else {
                        format!(" · expires {}", s.expiry)
                    };
                    if s.device.is_empty() {
                        device_lbl2.set_text(exp.trim_start_matches(" · "));
                    } else {
                        device_lbl2.set_text(&format!("Device: {}{}", s.device, exp));
                    }
                }
                rebuild_favs(&fav_box2, &s.favs, &tx2);
                *up.borrow_mut() = false;
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

// ── Small builders ────────────────────────────────────────────────────

fn wire(btn: &gtk4::Button, tx: &Tx, op: fn()) {
    let tx2 = tx.clone();
    btn.connect_clicked(move |_| kick(&tx2, op));
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

fn scroller(child: &impl IsA<gtk4::Widget>, min_h: i32) -> gtk4::ScrolledWindow {
    let s = gtk4::ScrolledWindow::new();
    s.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    s.set_min_content_height(min_h);
    s.set_child(Some(child));
    s.add_css_class("mvpn-card");
    s
}

fn toggle_row(parent: &gtk4::Box, title: &str, desc: &str) -> gtk4::Switch {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    row.add_css_class("mvpn-card");
    let texts = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    texts.set_hexpand(true);
    let t = gtk4::Label::new(Some(title));
    t.set_halign(gtk4::Align::Start);
    let d = gtk4::Label::new(Some(desc));
    d.add_css_class("mvpn-dim");
    d.set_halign(gtk4::Align::Start);
    texts.append(&t);
    texts.append(&d);
    let sw = gtk4::Switch::new();
    sw.set_valign(gtk4::Align::Center);
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
/// the caller (favorites connect by relay id, countries by location).
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
        let l = gtk4::Label::new(Some("No favorites yet — connect, then ‘Add’."));
        l.add_css_class("mvpn-dim");
        b.append(&l);
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

fn rebuild_countries(b: &gtk4::Box, all: &[relays::Country], needle: &str, tx: &Tx) {
    clear_box(b);
    let needle = needle.to_lowercase();
    for c in all.iter().filter(|c| {
        needle.is_empty() || c.name.to_lowercase().contains(&needle) || c.code.contains(&needle)
    }) {
        let row = relay_row(
            &c.name,
            &c.code.to_uppercase(),
            &format!("{} relays", c.relays),
        );
        let (tx2, code) = (tx.clone(), c.code.clone());
        row.connect_clicked(move |_| {
            let code = code.clone();
            kick(&tx2, move || {
                actions::set_location(&code, None);
            });
        });
        b.append(&row);
    }
}
