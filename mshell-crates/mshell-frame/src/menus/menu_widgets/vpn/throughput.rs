//! Live tunnel throughput card for the VPN menu (noctaproton-vpn port).
//!
//! Download / upload rates and a ~2-minute history graph for the VPN
//! tunnel interface (`wg0-mullvad`, picked by
//! `mshell_utils::mvpn::pick_tunnel_interface`), read from
//! `/sys/class/net/<if>/statistics` — the same card, graphs and classes
//! as the Network menu's Activity section, so the two read alike.
//!
//! The 2 s sampler starts on the first reveal and skips its read while
//! the menu is hidden (dropping its baseline so a re-open never shows a
//! bogus spike). When no tunnel exists the card shows a dim "No tunnel"
//! line instead of flat graphs.

use crate::bars::bar_widgets::network::format_speed;
use crate::menus::menu_widgets::network::network_menu_widget::{
    draw_graph, peak_text, push_capped, set_idle,
};
use mshell_utils::mvpn::{
    counter_rate, list_interfaces, pick_tunnel_interface, read_iface_counters,
};
use relm4::gtk::prelude::{BoxExt, DrawingAreaExt, DrawingAreaExtManual, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const INTERVAL: Duration = Duration::from_secs(2);

pub(crate) struct VpnThroughputModel {
    visible: Arc<AtomicBool>,
    started: bool,
    rx_hist: Rc<RefCell<Vec<f64>>>,
    tx_hist: Rc<RefCell<Vec<f64>>>,
}

#[derive(Debug)]
pub(crate) enum VpnThroughputInput {
    ParentRevealChanged(bool),
}

#[derive(Debug)]
pub(crate) enum VpnThroughputCommandOutput {
    /// (interface, rx B/s, tx B/s); `None` interface = no tunnel.
    Sample(Option<String>, u64, u64),
}

#[relm4::component(pub(crate))]
impl Component for VpnThroughputModel {
    type CommandOutput = VpnThroughputCommandOutput;
    type Input = VpnThroughputInput;
    type Output = ();
    type Init = ();

    view! {
        #[root]
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            #[name = "title"]
            gtk::Label {
                add_css_class: "vpn-section-label",
                set_label: "Tunnel traffic",
                set_xalign: 0.0,
            },
            gtk::Box {
                add_css_class: "network-traffic",
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 8,

                #[name = "empty_label"]
                gtk::Label {
                    set_css_classes: &["label-small", "dim-label"],
                    set_label: "No tunnel — connect to see its traffic",
                    set_xalign: 0.0,
                    set_visible: false,
                },

                #[name = "graphs"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 8,

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
                            #[name = "rx_peak"]
                            gtk::Label { add_css_class: "network-traffic-peak" },
                            #[name = "rx_speed"]
                            gtk::Label {
                                set_css_classes: &["network-traffic-speed", "network-rx"],
                                set_label: "0B/s",
                            },
                        },
                        #[name = "rx_area"]
                        gtk::DrawingArea {
                            add_css_class: "network-rx",
                            set_hexpand: true,
                            set_content_height: 48,
                        },
                    },
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
                            #[name = "tx_peak"]
                            gtk::Label { add_css_class: "network-traffic-peak" },
                            #[name = "tx_speed"]
                            gtk::Label {
                                set_css_classes: &["network-traffic-speed", "network-tx"],
                                set_label: "0B/s",
                            },
                        },
                        #[name = "tx_area"]
                        gtk::DrawingArea {
                            add_css_class: "network-tx",
                            set_hexpand: true,
                            set_content_height: 48,
                        },
                    },
                },
            },
        }
    }

    fn init(_: (), root: Self::Root, _sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let model = VpnThroughputModel {
            visible: Arc::new(AtomicBool::new(false)),
            started: false,
            rx_hist: Rc::new(RefCell::new(Vec::new())),
            tx_hist: Rc::new(RefCell::new(Vec::new())),
        };
        let widgets = view_output!();
        let h = model.rx_hist.clone();
        widgets
            .rx_area
            .set_draw_func(move |a, cr, w, ht| draw_graph(a, cr, w, ht, &h.borrow()));
        let h = model.tx_hist.clone();
        widgets
            .tx_area
            .set_draw_func(move |a, cr, w, ht| draw_graph(a, cr, w, ht, &h.borrow()));
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match msg {
            VpnThroughputInput::ParentRevealChanged(v) => {
                self.visible.store(v, Ordering::Relaxed);
                if v && !self.started {
                    self.started = true;
                    start_sampler(&sender, self.visible.clone());
                }
            }
        }
    }

    fn update_cmd_with_view(
        &mut self,
        w: &mut Self::Widgets,
        msg: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        let VpnThroughputCommandOutput::Sample(iface, rx, tx) = msg;
        let up = iface.is_some();
        w.graphs.set_visible(up);
        w.empty_label.set_visible(!up);
        w.title.set_label(&match &iface {
            Some(i) => format!("Tunnel traffic · {i}"),
            None => "Tunnel traffic".to_string(),
        });
        if !up {
            self.rx_hist.borrow_mut().clear();
            self.tx_hist.borrow_mut().clear();
            return;
        }
        push_capped(&self.rx_hist, rx as f64);
        push_capped(&self.tx_hist, tx as f64);
        w.rx_speed.set_label(&format!("{}/s", format_speed(rx)));
        w.tx_speed.set_label(&format!("{}/s", format_speed(tx)));
        set_idle(&w.rx_speed, rx);
        set_idle(&w.tx_speed, tx);
        w.rx_peak.set_label(&peak_text(&self.rx_hist));
        w.tx_peak.set_label(&peak_text(&self.tx_hist));
        w.rx_area.queue_draw();
        w.tx_area.queue_draw();
    }
}

fn start_sampler(sender: &ComponentSender<VpnThroughputModel>, visible: Arc<AtomicBool>) {
    sender.command(move |out, shutdown| async move {
        let shutdown_fut = shutdown.wait();
        tokio::pin!(shutdown_fut);
        // (iface, counters, when) of the previous sample.
        let mut prev: Option<(String, (u64, u64), Instant)> = None;
        let mut first = true;
        loop {
            if !first {
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    _ = tokio::time::sleep(INTERVAL) => {}
                }
            }
            first = false;
            if !visible.load(Ordering::Relaxed) {
                prev = None;
                continue;
            }
            let iface = pick_tunnel_interface(&list_interfaces());
            let Some(name) = iface.clone() else {
                prev = None;
                let _ = out.send(VpnThroughputCommandOutput::Sample(None, 0, 0));
                continue;
            };
            let Some(cur) = read_iface_counters(&name) else {
                prev = None;
                let _ = out.send(VpnThroughputCommandOutput::Sample(None, 0, 0));
                continue;
            };
            let now = Instant::now();
            let (rx, tx) = match &prev {
                Some((p_name, p, at)) if *p_name == name => {
                    let secs = now.duration_since(*at).as_secs_f64();
                    (
                        counter_rate(p.0, cur.0, secs),
                        counter_rate(p.1, cur.1, secs),
                    )
                }
                // First sample (or the tunnel changed): no delta yet.
                _ => (0, 0),
            };
            prev = Some((name, cur, now));
            let _ = out.send(VpnThroughputCommandOutput::Sample(iface, rx, tx));
        }
    });
}
