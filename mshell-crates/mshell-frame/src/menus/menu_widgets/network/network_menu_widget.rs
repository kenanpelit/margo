//! Network Console menu widget — content surface for
//! `MenuType::Network`.
//!
//! Layout:
//!   * **Hero** — active-connection summary (icon + name +
//!     IP + connectivity badge).
//!   * **Controls** — Wi-Fi radio toggle Switch + Rescan button.
//!   * **Network list** — scrollable rows of scanned APs, each:
//!     signal-strength icon + SSID + lock glyph (if secured) +
//!     Connect / Connected button.
//!
//! Link / Wi-Fi state is read reactively from `network_service()`
//! (NetworkManager over D-Bus) — no polling. Actions are still
//! unprivileged `nmcli` invocations, but only on a user click:
//!   * `nmcli radio wifi on/off`
//!   * `nmcli device wifi rescan`
//!   * `nmcli device wifi connect <ssid>` (NM prompts via its
//!     own agent if the saved secret is missing — for the MVP
//!     we connect to already-known SSIDs; unknown-SSID password
//!     entry is a follow-up).
//!   * `nmcli connection down <name>` to disconnect.
//! After an action the D-Bus watchers refresh the panel — no
//! re-probe.

use crate::bars::bar_widgets::network::{
    LinkKind, NetworkState, SpeedSample, WifiNetwork, format_speed, read_net_totals,
    read_network_state, wifi_signal_icon,
};
use mshell_common::WatcherToken;
use mshell_utils::network::{
    spawn_available_wifi_networks_watcher, spawn_network_watcher, spawn_wifi_watcher,
    spawn_wired_watcher,
};
use relm4::gtk::glib;
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, DrawingAreaExt, DrawingAreaExtManual, ListBoxRowExt, ObjectExt,
    OrientableExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tracing::warn;

/// Rolling traffic-history window length (samples). At the 2 s poll
/// cadence below that's ~2 minutes of history.
const HISTORY_LEN: usize = 60;
/// Poll cadence for the `/proc/net/dev` throughput sampler.
const SPEED_INTERVAL: Duration = Duration::from_secs(2);
/// Bytes/s below which a direction reads as idle (dims its label).
const IDLE_THRESHOLD: u64 = 3000;

pub(crate) struct NetworkMenuWidgetModel {
    state: NetworkState,
    hero_icon: gtk::Image,
    hero_title: gtk::Label,
    hero_subtitle: gtk::Label,
    connectivity_badge: gtk::Label,
    wifi_switch: gtk::Switch,
    wifi_switch_signal: glib::SignalHandlerId,
    network_list: gtk::ListBox,
    wifi_watcher_token: WatcherToken,
    wired_watcher_token: WatcherToken,
    // ── Traffic graphs (network-indicator port) ──
    rx_area: gtk::DrawingArea,
    tx_area: gtk::DrawingArea,
    rx_speed_label: gtk::Label,
    tx_speed_label: gtk::Label,
    rx_peak_label: gtk::Label,
    tx_peak_label: gtk::Label,
    rx_hist: Rc<RefCell<Vec<f64>>>,
    tx_hist: Rc<RefCell<Vec<f64>>>,
    /// `true` once the throughput sampler has been spawned (first reveal).
    poll_started: bool,
    /// Shared with the sampler; gates the `/proc/net/dev` read so it
    /// only samples while the panel is visible.
    visible: Arc<AtomicBool>,
}

impl std::fmt::Debug for NetworkMenuWidgetModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetworkMenuWidgetModel")
            .field("state", &self.state)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum NetworkMenuWidgetInput {
    SetWifiEnabled(bool),
    Rescan,
    Connect(String),
    Disconnect,
    /// Sent by the host menu on show/hide. The 2 s `/proc/net/dev`
    /// throughput sampler is started lazily on first reveal and skips
    /// its read while hidden, so a menu the user never opens does no
    /// per-second traffic sampling. (The D-Bus state watchers stay
    /// live — they are event-driven, not polled.)
    ParentRevealChanged(bool),
}

#[derive(Debug)]
pub(crate) enum NetworkMenuWidgetOutput {}

pub(crate) struct NetworkMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum NetworkMenuWidgetCommandOutput {
    /// Link / Wi-Fi / AP state changed (a D-Bus watcher fired).
    NetworkChanged,
    /// The Wi-Fi device was (un)plugged — re-arm its sub-watchers.
    WifiChanged,
    /// The wired device was (un)plugged — re-arm its sub-watcher.
    WiredChanged,
    /// A fresh `/proc/net/dev` throughput sample (down/up bytes/s).
    SpeedTick(SpeedSample),
}

#[relm4::component(pub(crate))]
impl Component for NetworkMenuWidgetModel {
    type CommandOutput = NetworkMenuWidgetCommandOutput;
    type Input = NetworkMenuWidgetInput;
    type Output = NetworkMenuWidgetOutput;
    type Init = NetworkMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "network-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            // ── §12 panel header ────────────────────────────────
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,
                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_icon_name: Some("network-wired-symbolic"),
                    set_valign: gtk::Align::Center,
                },
                gtk::Label {
                    add_css_class: "panel-title",
                    set_label: "Network",
                    set_halign: gtk::Align::Start,
                    set_hexpand: true,
                    set_valign: gtk::Align::Center,
                },
            },

            // ── Hero ────────────────────────────────────────────
            gtk::Box {
                add_css_class: "network-hero",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 10,

                #[local_ref]
                hero_icon_widget -> gtk::Image {
                    set_pixel_size: 32,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_hexpand: true,

                    #[local_ref]
                    hero_title_widget -> gtk::Label {
                        add_css_class: "label-large-bold",
                        set_xalign: 0.0,
                    },
                    #[local_ref]
                    hero_subtitle_widget -> gtk::Label {
                        add_css_class: "label-small",
                        set_xalign: 0.0,
                    },
                },

                #[local_ref]
                connectivity_badge_widget -> gtk::Label {
                    add_css_class: "network-badge",
                    set_valign: gtk::Align::Center,
                },
            },

            // ── Traffic (live RX / TX + history graphs) ─────────
            gtk::Box {
                add_css_class: "network-traffic",
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 8,

                // Download (RX).
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 2,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 6,
                        gtk::Image {
                            add_css_class: "network-rx",
                            set_icon_name: Some("network-receive-symbolic"),
                        },
                        gtk::Label {
                            add_css_class: "network-traffic-label",
                            set_label: "Download",
                            set_hexpand: true,
                            set_xalign: 0.0,
                        },
                        #[local_ref]
                        rx_peak_label_widget -> gtk::Label {
                            add_css_class: "network-traffic-peak",
                        },
                        #[local_ref]
                        rx_speed_label_widget -> gtk::Label {
                            set_css_classes: &["network-traffic-speed", "network-rx"],
                        },
                    },
                    #[local_ref]
                    rx_area_widget -> gtk::DrawingArea {
                        add_css_class: "network-rx",
                        set_hexpand: true,
                        set_content_height: 60,
                    },
                },

                // Upload (TX).
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 2,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 6,
                        gtk::Image {
                            add_css_class: "network-tx",
                            set_icon_name: Some("network-transmit-symbolic"),
                        },
                        gtk::Label {
                            add_css_class: "network-traffic-label",
                            set_label: "Upload",
                            set_hexpand: true,
                            set_xalign: 0.0,
                        },
                        #[local_ref]
                        tx_peak_label_widget -> gtk::Label {
                            add_css_class: "network-traffic-peak",
                        },
                        #[local_ref]
                        tx_speed_label_widget -> gtk::Label {
                            set_css_classes: &["network-traffic-speed", "network-tx"],
                        },
                    },
                    #[local_ref]
                    tx_area_widget -> gtk::DrawingArea {
                        add_css_class: "network-tx",
                        set_hexpand: true,
                        set_content_height: 60,
                    },
                },
            },

            gtk::Separator { set_orientation: gtk::Orientation::Horizontal },

            // ── Controls ────────────────────────────────────────
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,

                gtk::Label {
                    add_css_class: "label-medium-bold",
                    set_label: "Wi-Fi",
                    set_hexpand: true,
                    set_xalign: 0.0,
                },

                #[local_ref]
                wifi_switch_widget -> gtk::Switch {
                    set_valign: gtk::Align::Center,
                },

                gtk::Button {
                    set_css_classes: &["ok-button-surface", "ok-button-cell"],
                    set_label: "Rescan",
                    connect_clicked[sender] => move |_| {
                        sender.input(NetworkMenuWidgetInput::Rescan);
                    },
                },
                gtk::Button {
                    set_css_classes: &["ok-button-surface", "ok-button-cell"],
                    set_label: "Disconnect",
                    connect_clicked[sender] => move |_| {
                        sender.input(NetworkMenuWidgetInput::Disconnect);
                    },
                },
            },

            gtk::Separator { set_orientation: gtk::Orientation::Horizontal },

            gtk::Label {
                add_css_class: "label-medium-bold",
                set_label: "Available networks",
                set_xalign: 0.0,
            },

            // ── Network list ────────────────────────────────────
            gtk::ScrolledWindow {
                set_min_content_height: 240,
                set_max_content_height: 420,
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_propagate_natural_height: true,

                #[local_ref]
                network_list_widget -> gtk::ListBox {
                    add_css_class: "network-list",
                    set_selection_mode: gtk::SelectionMode::None,
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let hero_icon_widget = gtk::Image::from_icon_name("network-wireless-offline-symbolic");
        let hero_title_widget = gtk::Label::new(Some("Network"));
        let hero_subtitle_widget = gtk::Label::new(Some("…"));
        let connectivity_badge_widget = gtk::Label::new(Some("unknown"));
        let wifi_switch_widget = gtk::Switch::new();
        let network_list_widget = gtk::ListBox::new();

        // Traffic graphs + live-speed labels.
        let rx_speed_label_widget = gtk::Label::new(Some("0B/s"));
        let tx_speed_label_widget = gtk::Label::new(Some("0B/s"));
        let rx_peak_label_widget = gtk::Label::new(None);
        let tx_peak_label_widget = gtk::Label::new(None);
        let rx_area_widget = gtk::DrawingArea::new();
        let tx_area_widget = gtk::DrawingArea::new();
        let rx_hist: Rc<RefCell<Vec<f64>>> = Rc::new(RefCell::new(Vec::new()));
        let tx_hist: Rc<RefCell<Vec<f64>>> = Rc::new(RefCell::new(Vec::new()));
        {
            let h = rx_hist.clone();
            rx_area_widget
                .set_draw_func(move |a, cr, w, ht| draw_graph(a, cr, w, ht, &h.borrow()));
        }
        {
            let h = tx_hist.clone();
            tx_area_widget
                .set_draw_func(move |a, cr, w, ht| draw_graph(a, cr, w, ht, &h.borrow()));
        }

        // The `/proc/net/dev` throughput sampler is started lazily on
        // first reveal — see `ParentRevealChanged` — so a menu the user
        // never opens does no per-second traffic sampling.

        let toggle_sender = sender.clone();
        let wifi_switch_signal = wifi_switch_widget.connect_state_set(move |_, want_on| {
            toggle_sender.input(NetworkMenuWidgetInput::SetWifiEnabled(want_on));
            glib::Propagation::Stop
        });

        // Reactive — NetworkManager over D-Bus, no polling.
        spawn_network_watcher(
            &sender,
            || NetworkMenuWidgetCommandOutput::NetworkChanged,
            || NetworkMenuWidgetCommandOutput::WifiChanged,
            || NetworkMenuWidgetCommandOutput::WiredChanged,
        );

        let mut model = NetworkMenuWidgetModel {
            state: read_network_state(),
            hero_icon: hero_icon_widget.clone(),
            hero_title: hero_title_widget.clone(),
            hero_subtitle: hero_subtitle_widget.clone(),
            connectivity_badge: connectivity_badge_widget.clone(),
            wifi_switch: wifi_switch_widget.clone(),
            wifi_switch_signal,
            network_list: network_list_widget.clone(),
            wifi_watcher_token: WatcherToken::new(),
            wired_watcher_token: WatcherToken::new(),
            rx_area: rx_area_widget.clone(),
            tx_area: tx_area_widget.clone(),
            rx_speed_label: rx_speed_label_widget.clone(),
            tx_speed_label: tx_speed_label_widget.clone(),
            rx_peak_label: rx_peak_label_widget.clone(),
            tx_peak_label: tx_peak_label_widget.clone(),
            rx_hist,
            tx_hist,
            poll_started: false,
            visible: Arc::new(AtomicBool::new(false)),
        };
        arm_wifi_watchers(&sender, &mut model.wifi_watcher_token);
        arm_wired_watcher(&sender, &mut model.wired_watcher_token);

        let widgets = view_output!();
        sync_view(&model, &sender);

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NetworkMenuWidgetInput::SetWifiEnabled(on) => {
                let arg = if on { "on" } else { "off" };
                run_nmcli(vec!["radio".into(), "wifi".into(), arg.into()], sender.clone());
            }
            NetworkMenuWidgetInput::Rescan => {
                run_nmcli(
                    vec!["device".into(), "wifi".into(), "rescan".into()],
                    sender.clone(),
                );
            }
            NetworkMenuWidgetInput::Connect(ssid) => {
                run_nmcli(
                    vec![
                        "device".into(),
                        "wifi".into(),
                        "connect".into(),
                        ssid,
                    ],
                    sender.clone(),
                );
            }
            NetworkMenuWidgetInput::Disconnect => {
                let name = self.state.active_name.clone();
                if !name.is_empty() {
                    run_nmcli(
                        vec!["connection".into(), "down".into(), name],
                        sender.clone(),
                    );
                }
            }
            NetworkMenuWidgetInput::ParentRevealChanged(visible) => {
                self.visible.store(visible, Ordering::Relaxed);
                if visible && !self.poll_started {
                    self.poll_started = true;
                    start_speed_sampler(&sender, self.visible.clone());
                }
            }
        }
    }

    fn update_cmd_with_view(
        &mut self,
        _widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NetworkMenuWidgetCommandOutput::NetworkChanged => {}
            NetworkMenuWidgetCommandOutput::WifiChanged => {
                arm_wifi_watchers(&sender, &mut self.wifi_watcher_token);
            }
            NetworkMenuWidgetCommandOutput::WiredChanged => {
                arm_wired_watcher(&sender, &mut self.wired_watcher_token);
            }
            NetworkMenuWidgetCommandOutput::SpeedTick(sample) => {
                // Throughput updates are frequent and unrelated to the
                // NM state — repaint just the graphs + labels, never
                // the (expensive) AP-list rebuild below.
                push_capped(&self.rx_hist, sample.down_bps as f64);
                push_capped(&self.tx_hist, sample.up_bps as f64);
                self.rx_speed_label
                    .set_label(&format!("{}/s", format_speed(sample.down_bps)));
                self.tx_speed_label
                    .set_label(&format!("{}/s", format_speed(sample.up_bps)));
                set_idle(&self.rx_speed_label, sample.down_bps);
                set_idle(&self.tx_speed_label, sample.up_bps);
                self.rx_peak_label.set_label(&peak_text(&self.rx_hist));
                self.tx_peak_label.set_label(&peak_text(&self.tx_hist));
                self.rx_area.queue_draw();
                self.tx_area.queue_draw();
                return;
            }
        }
        let state = read_network_state();
        if self.state != state {
            self.state = state;
            sync_view(self, &sender);
        }
    }
}

/// Arm (or re-arm) the Wi-Fi device sub-watchers — enabled /
/// connectivity / ssid / strength plus the scanned-AP list.
/// The top-level `spawn_network_watcher` only re-fires on
/// hot-plug, so this picks up everything in between.
/// Spawn the `/proc/net/dev` throughput sampler. Started lazily on
/// first reveal; while the panel is hidden it drops its baseline and
/// skips the read, so re-revealing warms up cleanly over one interval
/// instead of emitting a bogus accumulated-bytes spike.
fn start_speed_sampler(sender: &ComponentSender<NetworkMenuWidgetModel>, visible: Arc<AtomicBool>) {
    sender.command(move |out, shutdown| async move {
        let shutdown_fut = shutdown.wait();
        tokio::pin!(shutdown_fut);
        let mut prev: Option<(u64, u64)> = None;
        loop {
            tokio::select! {
                () = &mut shutdown_fut => break,
                _ = tokio::time::sleep(SPEED_INTERVAL) => {}
            }
            if !visible.load(Ordering::Relaxed) {
                // Drop the baseline so the first sample after the next
                // reveal measures a fresh 2 s window, not the whole
                // hidden period.
                prev = None;
                continue;
            }
            let cur = read_net_totals().await;
            if let Some(p) = prev {
                let secs = SPEED_INTERVAL.as_secs().max(1);
                let down = cur.0.saturating_sub(p.0) / secs;
                let up = cur.1.saturating_sub(p.1) / secs;
                let _ = out.send(NetworkMenuWidgetCommandOutput::SpeedTick(SpeedSample {
                    down_bps: down,
                    up_bps: up,
                }));
            }
            prev = Some(cur);
        }
    });
}

fn arm_wifi_watchers(
    sender: &ComponentSender<NetworkMenuWidgetModel>,
    token: &mut WatcherToken,
) {
    let t = token.reset();
    spawn_wifi_watcher(sender, t.clone(), || {
        NetworkMenuWidgetCommandOutput::NetworkChanged
    });
    spawn_available_wifi_networks_watcher(sender, t, || {
        NetworkMenuWidgetCommandOutput::NetworkChanged
    });
}

fn arm_wired_watcher(
    sender: &ComponentSender<NetworkMenuWidgetModel>,
    token: &mut WatcherToken,
) {
    let t = token.reset();
    spawn_wired_watcher(sender, t, || NetworkMenuWidgetCommandOutput::NetworkChanged);
}

/// Run `nmcli <args…>` once, on a user action. The panel
/// refresh comes from the D-Bus watchers picking up the
/// NetworkManager state change — no re-probe here.
fn run_nmcli(args: Vec<String>, _sender: ComponentSender<NetworkMenuWidgetModel>) {
    tokio::spawn(async move {
        match tokio::process::Command::new("nmcli")
            .args(&args)
            .status()
            .await
        {
            Ok(s) if s.success() => {}
            Ok(s) => warn!(?s, ?args, "nmcli action returned non-zero"),
            Err(e) => warn!(error = %e, ?args, "nmcli spawn failed"),
        }
    });
}

fn sync_view(model: &NetworkMenuWidgetModel, sender: &ComponentSender<NetworkMenuWidgetModel>) {
    let s = &model.state;

    // Hero.
    let (icon, title, subtitle) = match s.active_kind {
        LinkKind::Wifi => (
            wifi_signal_icon(s.active_signal),
            if s.active_name.is_empty() {
                "Wi-Fi".to_string()
            } else {
                s.active_name.clone()
            },
            format!("{}% signal", s.active_signal),
        ),
        LinkKind::Wired => (
            "network-wired-symbolic",
            if s.active_name.is_empty() {
                "Wired".to_string()
            } else {
                s.active_name.clone()
            },
            "connected".to_string(),
        ),
        LinkKind::None => (
            if s.wifi_enabled {
                "network-wireless-offline-symbolic"
            } else {
                "network-wireless-disabled-symbolic"
            },
            "Not connected".to_string(),
            if s.wifi_enabled {
                "Pick a network below".to_string()
            } else {
                "Wi-Fi is off".to_string()
            },
        ),
    };
    model.hero_icon.set_icon_name(Some(icon));
    model.hero_title.set_label(&title);
    model.hero_subtitle.set_label(&subtitle);

    let (badge_text, badge_class) = match s.connectivity.as_str() {
        "full" => ("Online", "network-badge-online"),
        "limited" | "portal" => ("Limited", "network-badge-limited"),
        "none" => ("Offline", "network-badge-offline"),
        other => (other, "network-badge-unknown"),
    };
    model.connectivity_badge.set_label(badge_text);
    model
        .connectivity_badge
        .set_css_classes(&["network-badge", badge_class]);

    // Wi-Fi switch — block our own handler during the
    // programmatic set so it doesn't loop back into a radio
    // toggle.
    if model.wifi_switch.state() != s.wifi_enabled {
        model.wifi_switch.block_signal(&model.wifi_switch_signal);
        model.wifi_switch.set_state(s.wifi_enabled);
        model.wifi_switch.set_active(s.wifi_enabled);
        model.wifi_switch.unblock_signal(&model.wifi_switch_signal);
    }

    // Network list.
    while let Some(child) = model.network_list.first_child() {
        model.network_list.remove(&child);
    }
    if !s.available {
        model
            .network_list
            .append(&placeholder_row("NetworkManager not available"));
    } else if !s.wifi_enabled {
        model
            .network_list
            .append(&placeholder_row("(Wi-Fi is off)"));
    } else if s.networks.is_empty() {
        model
            .network_list
            .append(&placeholder_row("(no networks found — try Rescan)"));
    } else {
        for net in &s.networks {
            model.network_list.append(&make_network_row(net, sender));
        }
    }
}

fn placeholder_row(text: &str) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);
    let label = gtk::Label::new(Some(text));
    label.add_css_class("label-small");
    label.set_xalign(0.0);
    label.set_margin_top(8);
    label.set_margin_bottom(8);
    label.set_margin_start(12);
    row.set_child(Some(&label));
    row
}

fn make_network_row(
    net: &WifiNetwork,
    sender: &ComponentSender<NetworkMenuWidgetModel>,
) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);
    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_top(5)
        .margin_bottom(5)
        .margin_start(8)
        .margin_end(8)
        .build();

    outer.append(&gtk::Image::from_icon_name(wifi_signal_icon(net.signal)));

    let texts = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .build();
    let name = gtk::Label::new(Some(&net.ssid));
    name.add_css_class("label-medium-bold");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    texts.append(&name);
    let detail = gtk::Label::new(Some(&format!(
        "{}%{}",
        net.signal,
        if net.secured { "  ·  secured" } else { "  ·  open" }
    )));
    detail.add_css_class("label-small");
    detail.set_xalign(0.0);
    texts.append(&detail);
    outer.append(&texts);

    if net.secured {
        let lock = gtk::Image::from_icon_name("system-lock-screen-symbolic");
        lock.add_css_class("network-lock");
        outer.append(&lock);
    }

    let connect = if net.in_use {
        let b = gtk::Button::with_label("Connected");
        b.add_css_class("ok-button-surface");
        b.add_css_class("selected");
        b.set_sensitive(false);
        b
    } else {
        let b = gtk::Button::with_label("Connect");
        b.add_css_class("ok-button-surface");
        let ssid = net.ssid.clone();
        let s = sender.clone();
        b.connect_clicked(move |_| {
            s.input(NetworkMenuWidgetInput::Connect(ssid.clone()));
        });
        b
    };
    connect.set_valign(gtk::Align::Center);
    outer.append(&connect);

    row.set_child(Some(&outer));
    row
}

// ── Traffic-graph helpers (network-indicator port) ──────────────

/// Append a sample to a rolling history, capped at [`HISTORY_LEN`].
fn push_capped(hist: &Rc<RefCell<Vec<f64>>>, value: f64) {
    let mut h = hist.borrow_mut();
    h.push(value);
    let len = h.len();
    if len > HISTORY_LEN {
        h.drain(0..len - HISTORY_LEN);
    }
}

/// "peak X/s" label text from a history window (empty while warming
/// up so the row stays clean).
fn peak_text(hist: &Rc<RefCell<Vec<f64>>>) -> String {
    let peak = hist.borrow().iter().copied().fold(0.0_f64, f64::max);
    if peak < 1.0 {
        String::new()
    } else {
        format!("peak {}/s", format_speed(peak as u64))
    }
}

/// Dim a speed label when its direction is idle (below threshold).
fn set_idle(label: &gtk::Label, bps: u64) {
    if bps >= IDLE_THRESHOLD {
        label.remove_css_class("idle");
    } else {
        label.add_css_class("idle");
    }
}

/// Draw a smooth filled history sparkline. The accent colour comes
/// from the area's resolved CSS `color` (`.network-rx` / `.network-tx`
/// in SCSS), so it tracks the matugen theme automatically. The line is
/// a Catmull-Rom spline, the fill an accent→transparent gradient.
/// Auto-scales to the window's peak.
fn draw_graph(area: &gtk::DrawingArea, cr: &gtk::cairo::Context, w: i32, h: i32, hist: &[f64]) {
    use gtk::cairo::{LineCap, LineJoin, LinearGradient};

    let (w, h) = (w as f64, h as f64);
    let c = area.color();
    let (r, g, b) = (c.red() as f64, c.green() as f64, c.blue() as f64);

    // Faint baseline gridlines at 1/3 + 2/3 height.
    cr.set_line_width(1.0);
    cr.set_source_rgba(r, g, b, 0.08);
    for frac in [0.33_f64, 0.66] {
        let y = h * (1.0 - frac);
        cr.move_to(0.0, y);
        cr.line_to(w, y);
        let _ = cr.stroke();
    }

    if hist.len() < 2 {
        return;
    }
    let max = hist.iter().copied().fold(1.0_f64, f64::max);
    let n = hist.len();
    let step = w / (n as f64 - 1.0);
    // Leave 2 px headroom top + bottom so the stroke + end dot aren't
    // clipped at the peak / baseline.
    let pts: Vec<(f64, f64)> = hist
        .iter()
        .enumerate()
        .map(|(i, v)| (i as f64 * step, h - 2.0 - (v / max) * (h - 4.0)))
        .collect();

    // Gradient-filled area under the smooth curve.
    smooth_path(cr, &pts);
    cr.line_to(w, h);
    cr.line_to(0.0, h);
    cr.close_path();
    let grad = LinearGradient::new(0.0, 0.0, 0.0, h);
    grad.add_color_stop_rgba(0.0, r, g, b, 0.32);
    grad.add_color_stop_rgba(1.0, r, g, b, 0.0);
    let _ = cr.set_source(&grad);
    let _ = cr.fill();

    // Stroke the curve on top, rounded.
    cr.set_line_width(2.0);
    cr.set_line_cap(LineCap::Round);
    cr.set_line_join(LineJoin::Round);
    cr.set_source_rgba(r, g, b, 0.95);
    smooth_path(cr, &pts);
    let _ = cr.stroke();

    // "Now" dot at the leading (right-most) sample.
    if let Some(&(x, y)) = pts.last() {
        cr.set_source_rgba(r, g, b, 1.0);
        cr.arc(x, y, 2.5, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    }
}

/// Emit a Catmull-Rom spline through `pts` as cairo `curve_to`s
/// (move_to first point, then one cubic per segment). Endpoints are
/// clamped so the curve starts/ends exactly on the data.
fn smooth_path(cr: &gtk::cairo::Context, pts: &[(f64, f64)]) {
    let n = pts.len();
    cr.move_to(pts[0].0, pts[0].1);
    for i in 0..n - 1 {
        let p0 = pts[i.saturating_sub(1)];
        let p1 = pts[i];
        let p2 = pts[i + 1];
        let p3 = pts[(i + 2).min(n - 1)];
        let c1 = (p1.0 + (p2.0 - p0.0) / 6.0, p1.1 + (p2.1 - p0.1) / 6.0);
        let c2 = (p2.0 - (p3.0 - p1.0) / 6.0, p2.1 - (p3.1 - p1.1) / 6.0);
        cr.curve_to(c1.0, c1.1, c2.0, c2.1, p2.0, p2.1);
    }
}
