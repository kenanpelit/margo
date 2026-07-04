//! Generic-VPN detail menu — content surface for
//! `MenuType::VpnIndicator`.
//!
//! One card per active generic tunnel (OpenVPN / WireGuard, Mullvad
//! excluded) showing the interface **name**, its **type**, the local
//! tunnel **IP(s)** (IPv4 + IPv6 when present), and live **RX/TX
//! throughput**. Everything is gathered from local sources only
//! (`/sys/class/net` + `getifaddrs`) — there is **no** network call.
//!
//! Polls lazily: the sampler starts on first reveal and only reads
//! while the panel is visible, so a menu the user never opens does no
//! background work (DESIGN.md §13.4). Successive `statistics/{rx,tx}_bytes`
//! samples are turned into a per-second rate.

use crate::bars::bar_widgets::vpn_indicator::{VpnInterface, gather_interfaces};
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Sample cadence while the panel is open. Short enough that the
/// throughput readout feels live, long enough to stay calm.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(2);

/// The live widgets of one interface card — kept on the model so a
/// refresh only touches text/visibility when the interface set is
/// unchanged (no per-tick teardown, DESIGN.md §13.4).
struct IfaceRow {
    name: String,
    ipv4_value: gtk::Label,
    ipv6_row: gtk::Box,
    ipv6_value: gtk::Label,
    throughput_value: gtk::Label,
}

pub(crate) struct VpnIndicatorMenuWidgetModel {
    /// Container the per-interface cards live in.
    details_box: gtk::Box,
    /// Shown when no generic tunnel is up.
    empty_label: gtk::Label,
    /// Live interface cards, in display order.
    rows: Vec<IfaceRow>,
    /// Previous `(rx_bytes, tx_bytes, sampled_at)` per interface, used
    /// to derive throughput from successive samples.
    prev: HashMap<String, (u64, u64, Instant)>,
    /// `true` once the sampler has been spawned (on first reveal).
    poll_started: bool,
    /// Shared with the sampler; gates the local read so it only runs
    /// while the panel is actually visible.
    visible: Arc<AtomicBool>,
}

impl std::fmt::Debug for VpnIndicatorMenuWidgetModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VpnIndicatorMenuWidgetModel")
            .field("rows", &self.rows.len())
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum VpnIndicatorMenuWidgetInput {
    RefreshNow,
    /// Sent by the host menu when it is shown/hidden. The sampler is
    /// started lazily on first reveal, so a menu the user never opens
    /// does no background work.
    ParentRevealChanged(bool),
}

#[derive(Debug)]
pub(crate) enum VpnIndicatorMenuWidgetOutput {}

pub(crate) struct VpnIndicatorMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum VpnIndicatorMenuWidgetCommandOutput {
    Refreshed(Vec<VpnInterface>),
}

#[relm4::component(pub(crate))]
impl Component for VpnIndicatorMenuWidgetModel {
    type CommandOutput = VpnIndicatorMenuWidgetCommandOutput;
    type Input = VpnIndicatorMenuWidgetInput;
    type Output = VpnIndicatorMenuWidgetOutput;
    type Init = VpnIndicatorMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "vpn-indicator-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            // ── §12 panel header ────────────────────────────────
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,
                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_icon_name: Some("network-vpn-symbolic"),
                    set_valign: gtk::Align::Center,
                },
                gtk::Label {
                    add_css_class: "panel-title",
                    set_label: "VPN Tunnels",
                    set_halign: gtk::Align::Start,
                    set_hexpand: true,
                    set_valign: gtk::Align::Center,
                },
            },

            // ── Empty state ─────────────────────────────────────
            #[local_ref]
            empty_label -> gtk::Label {
                add_css_class: "vpn-indicator-empty",
                set_label: "No active VPN tunnels.",
                set_halign: gtk::Align::Start,
                set_xalign: 0.0,
                set_wrap: true,
            },

            // ── Per-interface cards ─────────────────────────────
            #[local_ref]
            details_box -> gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 8,
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let details_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let empty_label = gtk::Label::new(Some("No active VPN tunnels."));

        // The sampler is *not* started here — it spawns lazily on first
        // reveal (see `ParentRevealChanged`), so a menu the user never
        // opens does no background sysfs reads.
        let model = VpnIndicatorMenuWidgetModel {
            details_box: details_box.clone(),
            empty_label: empty_label.clone(),
            rows: Vec::new(),
            prev: HashMap::new(),
            poll_started: false,
            visible: Arc::new(AtomicBool::new(false)),
        };

        let widgets = view_output!();
        model.empty_label.set_visible(true);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            VpnIndicatorMenuWidgetInput::RefreshNow => {
                sender.command(|out, _shutdown| async move {
                    let ifaces = gather_interfaces();
                    let _ = out.send(VpnIndicatorMenuWidgetCommandOutput::Refreshed(ifaces));
                });
            }
            VpnIndicatorMenuWidgetInput::ParentRevealChanged(visible) => {
                self.visible.store(visible, Ordering::Relaxed);
                if visible {
                    if !self.poll_started {
                        self.poll_started = true;
                        start_sampling(&sender, self.visible.clone());
                    }
                    // Fresh detail the moment the panel opens, without
                    // waiting for the next interval tick.
                    sender.input(VpnIndicatorMenuWidgetInput::RefreshNow);
                } else {
                    // Drop the baselines so the next reveal measures a
                    // fresh window rather than the whole hidden period.
                    self.prev.clear();
                }
            }
        }
    }

    fn update_cmd_with_view(
        &mut self,
        _widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            VpnIndicatorMenuWidgetCommandOutput::Refreshed(ifaces) => {
                self.sync(ifaces);
            }
        }
    }
}

impl VpnIndicatorMenuWidgetModel {
    fn sync(&mut self, ifaces: Vec<VpnInterface>) {
        self.empty_label.set_visible(ifaces.is_empty());

        // Rebuild the card structure only when the interface *set*
        // changes; otherwise update the value labels in place so a
        // per-tick throughput refresh doesn't tear down the cards.
        let names_now: Vec<&str> = ifaces.iter().map(|i| i.name.as_str()).collect();
        let names_rows: Vec<&str> = self.rows.iter().map(|r| r.name.as_str()).collect();
        if names_now != names_rows {
            while let Some(child) = self.details_box.first_child() {
                self.details_box.remove(&child);
            }
            self.rows.clear();
            for iface in &ifaces {
                let (card, row) = build_iface_card(iface);
                self.details_box.append(&card);
                self.rows.push(row);
            }
        }

        let now = Instant::now();
        for iface in &ifaces {
            let throughput = match self.prev.get(&iface.name) {
                Some((prx, ptx, at)) => {
                    let elapsed = now.duration_since(*at).as_secs_f64().max(0.001);
                    let rx = iface.rx_bytes.saturating_sub(*prx) as f64 / elapsed;
                    let tx = iface.tx_bytes.saturating_sub(*ptx) as f64 / elapsed;
                    format!("↓ {}   ↑ {}", human_rate(rx), human_rate(tx))
                }
                None => "…".to_string(),
            };
            self.prev
                .insert(iface.name.clone(), (iface.rx_bytes, iface.tx_bytes, now));

            if let Some(row) = self.rows.iter().find(|r| r.name == iface.name) {
                row.ipv4_value.set_label(&join_or_dash(&iface.ipv4));
                let has_v6 = !iface.ipv6.is_empty();
                row.ipv6_row.set_visible(has_v6);
                if has_v6 {
                    row.ipv6_value.set_label(&iface.ipv6.join("\n"));
                }
                row.throughput_value.set_label(&throughput);
            }
        }

        // Forget baselines for interfaces that dropped.
        self.prev
            .retain(|name, _| ifaces.iter().any(|i| &i.name == name));
    }
}

/// Spawn the perpetual sampler. Started lazily on first reveal; while
/// the panel is hidden the loop only does a cheap timer wake — no read.
fn start_sampling(sender: &ComponentSender<VpnIndicatorMenuWidgetModel>, visible: Arc<AtomicBool>) {
    sender.command(move |out, shutdown| async move {
        let shutdown_fut = shutdown.wait();
        tokio::pin!(shutdown_fut);
        loop {
            tokio::select! {
                () = &mut shutdown_fut => break,
                _ = tokio::time::sleep(SAMPLE_INTERVAL) => {}
            }
            if visible.load(Ordering::Relaxed) {
                let ifaces = gather_interfaces();
                let _ = out.send(VpnIndicatorMenuWidgetCommandOutput::Refreshed(ifaces));
            }
        }
    });
}

/// Build one interface card + its live label handles.
fn build_iface_card(iface: &VpnInterface) -> (gtk::Box, IfaceRow) {
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .css_classes(vec!["vpn-indicator-iface-card"])
        .build();

    // Header: icon + name (hexpand) + type badge.
    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    let icon = gtk::Image::from_icon_name("network-vpn-symbolic");
    icon.add_css_class("vpn-indicator-iface-icon");
    icon.set_valign(gtk::Align::Center);
    header.append(&icon);
    let name = gtk::Label::new(Some(&iface.name));
    name.add_css_class("vpn-indicator-iface-name");
    name.set_xalign(0.0);
    name.set_hexpand(true);
    name.set_selectable(true);
    header.append(&name);
    let badge = gtk::Label::new(Some(iface.kind.label()));
    badge.add_css_class("vpn-indicator-badge");
    badge.set_valign(gtk::Align::Center);
    header.append(&badge);
    card.append(&header);

    // Detail rows.
    let (ipv4_row, ipv4_value) = make_detail_row("IPv4");
    card.append(&ipv4_row);
    let (ipv6_row, ipv6_value) = make_detail_row("IPv6");
    card.append(&ipv6_row);
    let (throughput_row, throughput_value) = make_detail_row("Throughput");
    card.append(&throughput_row);

    let row = IfaceRow {
        name: iface.name.clone(),
        ipv4_value,
        ipv6_row,
        ipv6_value,
        throughput_value,
    };
    (card, row)
}

fn make_detail_row(caption: &str) -> (gtk::Box, gtk::Label) {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .css_classes(vec!["vpn-indicator-detail-row"])
        .build();
    let cap = gtk::Label::new(Some(caption));
    cap.add_css_class("label-small");
    cap.set_xalign(0.0);
    cap.set_valign(gtk::Align::Start);
    cap.set_width_request(96);
    row.append(&cap);
    let value = gtk::Label::new(Some("—"));
    value.add_css_class("label-small-bold");
    value.set_xalign(0.0);
    value.set_hexpand(true);
    value.set_selectable(true);
    value.set_wrap(true);
    value.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    row.append(&value);
    (row, value)
}

fn join_or_dash(items: &[String]) -> String {
    if items.is_empty() {
        "—".to_string()
    } else {
        items.join("\n")
    }
}

/// Human-readable throughput (`B/s` … `GB/s`).
fn human_rate(bytes_per_sec: f64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes_per_sec.max(0.0);
    if b >= GB {
        format!("{:.1} GB/s", b / GB)
    } else if b >= MB {
        format!("{:.1} MB/s", b / MB)
    } else if b >= KB {
        format!("{:.0} KB/s", b / KB)
    } else {
        format!("{b:.0} B/s")
    }
}
