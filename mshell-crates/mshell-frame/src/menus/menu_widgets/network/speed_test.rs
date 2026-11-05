//! Speed-test card for the Network menu (speedtest-meter port).
//!
//! A ring gauge sweeping the live Mbit/s of the current phase, the phase
//! + server/ISP line, and the result stats (download / upload / ping /
//! jitter / loss). The engine is `mshell_utils::speedtest`: Ookla
//! `speedtest` when installed (live JSON progress, jitter, loss), else
//! `speedtest-cli` with the gauge fed from `/proc/net/dev`.
//!
//! Nothing runs until the user presses *Run* — no polling. The backend
//! probe happens once, on the first reveal. *Cancel* aborts the task,
//! which drops the child (`kill_on_drop`).
//!
//! Gauge colours come from CSS (`.network-rx` / `.network-tx` on the
//! drawing area, read via `widget.color()`), never from Rust literals.

use mshell_utils::speedtest::{
    self, Backend, Phase, SpeedEvent, SpeedResult, format_mbps, gauge_scale,
};
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, DrawingAreaExt, DrawingAreaExtManual, OrientableExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::cell::Cell;
use std::rc::Rc;

pub(crate) struct SpeedTestModel {
    /// `None` until probed; `Some(None)` = no backend installed.
    backend: Option<Option<Backend>>,
    probed: bool,
    running: Option<tokio::task::JoinHandle<()>>,
    phase: Phase,
    live_mbps: f64,
    /// Highest live reading this run — sets the gauge full-scale.
    peak_mbps: f64,
    ping_ms: Option<f64>,
    jitter_ms: Option<f64>,
    server: String,
    result: Option<SpeedResult>,
    error: Option<String>,
    /// Shared with the gauge draw func: (fraction 0..=1).
    gauge: Rc<Cell<f64>>,
}

#[derive(Debug)]
pub(crate) enum SpeedTestInput {
    /// Host menu shown/hidden — the first reveal probes the backend.
    ParentRevealChanged(bool),
    RunOrCancel,
    Event(SpeedEvent),
}

#[derive(Debug)]
pub(crate) enum SpeedTestCommandOutput {
    Backend(Option<Backend>),
}

#[relm4::component(pub(crate))]
impl Component for SpeedTestModel {
    type CommandOutput = SpeedTestCommandOutput;
    type Input = SpeedTestInput;
    type Output = ();
    type Init = ();

    view! {
        #[root]
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            gtk::Label {
                add_css_class: "network-section-label",
                set_label: "Speed test",
                set_xalign: 0.0,
            },
            gtk::Box {
                set_css_classes: &["network-traffic", "network-speedtest"],
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 8,

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,

                    #[name = "gauge_area"]
                    gtk::DrawingArea {
                        add_css_class: "network-speedtest-gauge",
                        set_content_width: 72,
                        set_content_height: 72,
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        set_valign: gtk::Align::Center,
                        set_spacing: 2,

                        #[name = "value_label"]
                        gtk::Label {
                            add_css_class: "network-speedtest-value",
                            set_xalign: 0.0,
                        },
                        #[name = "phase_label"]
                        gtk::Label {
                            add_css_class: "label-small",
                            set_xalign: 0.0,
                        },
                        #[name = "server_label"]
                        gtk::Label {
                            add_css_class: "label-small",
                            set_xalign: 0.0,
                            set_ellipsize: gtk::pango::EllipsizeMode::End,
                        },
                    },
                },

                #[name = "stats_box"]
                gtk::Box {
                    add_css_class: "network-speedtest-stats",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_homogeneous: true,
                    set_spacing: 4,
                },

                #[name = "run_button"]
                gtk::Button {
                    set_css_classes: &["ok-button-surface", "network-action-btn"],
                    set_hexpand: true,
                    set_label: "Run speed test",
                    connect_clicked[sender] => move |_| {
                        sender.input(SpeedTestInput::RunOrCancel);
                    },
                },
            },
        }
    }

    fn init(_: (), root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let model = SpeedTestModel {
            backend: None,
            probed: false,
            running: None,
            phase: Phase::default(),
            live_mbps: 0.0,
            peak_mbps: 0.0,
            ping_ms: None,
            jitter_ms: None,
            server: String::new(),
            result: None,
            error: None,
            gauge: Rc::new(Cell::new(0.0)),
        };
        let widgets = view_output!();
        let g = model.gauge.clone();
        widgets
            .gauge_area
            .set_draw_func(move |a, cr, w, h| draw_gauge(a, cr, w, h, g.get()));
        sync(&model, &widgets);
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        msg: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match msg {
            SpeedTestInput::ParentRevealChanged(visible) => {
                if visible && !self.probed {
                    self.probed = true;
                    sender.oneshot_command(async {
                        SpeedTestCommandOutput::Backend(speedtest::detect_backend().await)
                    });
                }
                return;
            }
            SpeedTestInput::RunOrCancel => {
                if let Some(h) = self.running.take() {
                    h.abort();
                    self.error = Some("Cancelled".to_string());
                } else if let Some(Some(backend)) = self.backend {
                    self.reset_run();
                    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
                    let s = sender.clone();
                    self.running = Some(tokio::spawn(async move {
                        let fwd = async move {
                            while let Some(ev) = rx.recv().await {
                                s.input(SpeedTestInput::Event(ev));
                            }
                        };
                        tokio::join!(speedtest::run(backend, tx), fwd);
                    }));
                }
            }
            SpeedTestInput::Event(ev) => self.apply(ev),
        }
        sync(self, widgets);
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        msg: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match msg {
            SpeedTestCommandOutput::Backend(b) => self.backend = Some(b),
        }
        sync(self, widgets);
    }
}

impl SpeedTestModel {
    fn reset_run(&mut self) {
        self.phase = Phase::Connecting;
        self.live_mbps = 0.0;
        self.peak_mbps = 0.0;
        self.ping_ms = None;
        self.jitter_ms = None;
        self.server.clear();
        self.result = None;
        self.error = None;
    }

    fn apply(&mut self, ev: SpeedEvent) {
        match ev {
            SpeedEvent::Started { server, isp } => {
                self.server = [server, isp]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · ");
            }
            SpeedEvent::Ping {
                latency_ms,
                jitter_ms,
            } => {
                self.phase = Phase::Ping;
                self.ping_ms = Some(latency_ms);
                self.jitter_ms = jitter_ms;
            }
            SpeedEvent::Live { phase, mbps, .. } => {
                if phase != self.phase {
                    // New direction — the gauge restarts from 0 on its own scale.
                    self.peak_mbps = 0.0;
                }
                self.phase = phase;
                self.live_mbps = mbps;
                self.peak_mbps = self.peak_mbps.max(mbps);
            }
            SpeedEvent::Done(r) => {
                self.running = None;
                if self.server.is_empty() {
                    self.server = [r.server_line(), r.isp.clone()]
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(" · ");
                }
                self.result = Some(r);
            }
            SpeedEvent::Failed(e) => {
                self.running = None;
                self.error = Some(e);
            }
        }
    }
}

fn opt(v: Option<f64>, f: impl Fn(f64) -> String) -> String {
    v.map(f).unwrap_or_else(|| "—".to_string())
}

fn stat(caption: &str, value: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let c = gtk::Label::new(Some(caption));
    c.add_css_class("network-traffic-label");
    let v = gtk::Label::new(Some(value));
    v.add_css_class("network-traffic-speed");
    b.append(&c);
    b.append(&v);
    b
}

fn sync(m: &SpeedTestModel, w: &SpeedTestModelWidgets) {
    let running = m.running.is_some();
    let no_backend = matches!(m.backend, Some(None));

    // Gauge: live reading while running, the final download after.
    let (fraction, value, rx) = if running {
        let scale = gauge_scale(m.peak_mbps);
        (
            (m.live_mbps / scale).clamp(0.0, 1.0),
            if matches!(m.phase, Phase::Download | Phase::Upload) {
                format_mbps(m.live_mbps)
            } else {
                opt(m.ping_ms, |p| format!("{p:.0} ms"))
            },
            m.phase != Phase::Upload,
        )
    } else if let Some(r) = &m.result {
        let d = r.download_mbps.unwrap_or(0.0);
        (
            (d / gauge_scale(d)).clamp(0.0, 1.0),
            opt(r.download_mbps, format_mbps),
            true,
        )
    } else {
        (0.0, "—".to_string(), true)
    };
    m.gauge.set(fraction);
    w.gauge_area.set_css_classes(&[
        "network-speedtest-gauge",
        if rx { "network-rx" } else { "network-tx" },
    ]);
    w.gauge_area.queue_draw();
    w.value_label.set_label(&value);

    let phase = if no_backend {
        "Install speedtest-cli (or ookla-speedtest-bin from the AUR)".to_string()
    } else if m.backend.is_none() {
        "…".to_string()
    } else if running {
        m.phase.label().to_string()
    } else if let Some(e) = &m.error {
        e.clone()
    } else if m.result.is_some() {
        "Done".to_string()
    } else {
        "Measures the active route — over Mullvad that is the tunnel".to_string()
    };
    w.phase_label.set_label(&phase);
    w.server_label.set_label(&m.server);
    w.server_label.set_visible(!m.server.is_empty());

    // Stats row (rebuilt — five labels, only on events).
    while let Some(c) = w.stats_box.first_child() {
        w.stats_box.remove(&c);
    }
    let r = m.result.clone().unwrap_or_default();
    let ping = r.ping_ms.or(m.ping_ms);
    let jitter = r.jitter_ms.or(m.jitter_ms);
    for (cap, val) in [
        ("Down", opt(r.download_mbps, |v| format!("{v:.1}"))),
        ("Up", opt(r.upload_mbps, |v| format!("{v:.1}"))),
        ("Ping", opt(ping, |v| format!("{v:.0} ms"))),
        ("Jitter", opt(jitter, |v| format!("{v:.1} ms"))),
        ("Loss", opt(r.packet_loss, |v| format!("{v:.1}%"))),
    ] {
        w.stats_box.append(&stat(cap, &val));
    }

    w.run_button
        .set_label(if running { "Cancel" } else { "Run speed test" });
    w.run_button
        .set_sensitive(running || matches!(m.backend, Some(Some(_))));
}

/// 270° ring: a faint track plus the value arc, both in the area's CSS
/// colour (`.network-rx` / `.network-tx`).
fn draw_gauge(area: &gtk::DrawingArea, cr: &gtk::cairo::Context, w: i32, h: i32, frac: f64) {
    use std::f64::consts::PI;
    let (w, h) = (w as f64, h as f64);
    let c = area.color();
    let (r, g, b) = (c.red() as f64, c.green() as f64, c.blue() as f64);
    let line = 6.0;
    let radius = (w.min(h) - line) / 2.0;
    let (cx, cy) = (w / 2.0, h / 2.0);
    let start = 0.75 * PI;
    let sweep = 1.5 * PI;
    cr.set_line_width(line);
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    cr.set_source_rgba(r, g, b, 0.15);
    cr.arc(cx, cy, radius, start, start + sweep);
    let _ = cr.stroke();
    if frac > 0.0 {
        cr.set_source_rgba(r, g, b, 1.0);
        cr.arc(cx, cy, radius, start, start + sweep * frac.clamp(0.0, 1.0));
        let _ = cr.stroke();
    }
}

#[path = "speed_test_tests.rs"]
#[cfg(test)]
mod speed_test_tests;
