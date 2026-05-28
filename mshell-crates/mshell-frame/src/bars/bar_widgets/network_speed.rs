//! Network-speed bar pill.
//!
//! Render-only — shows the aggregate download / upload throughput
//! (`↓ rate  ↑ rate`) across all real interfaces, sampled from
//! `/proc/net/dev` on a fixed interval. Loopback (`lo`) is excluded.
//! The rate is the byte delta between samples divided by the elapsed
//! time, so it reflects live traffic. (Inspired by VibePanel's
//! network-speed widget.)

use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

/// Sample cadence. A speed read is a cheap `/proc/net/dev` parse.
const REFRESH_INTERVAL: Duration = Duration::from_secs(1);

pub(crate) struct NetworkSpeedModel {
    down: String,
    up: String,
}

#[derive(Debug)]
pub(crate) enum NetworkSpeedInput {}

#[derive(Debug)]
pub(crate) enum NetworkSpeedOutput {}

pub(crate) struct NetworkSpeedInit {}

#[derive(Debug)]
pub(crate) enum NetworkSpeedCommandOutput {
    /// New (download, upload) rates, already formatted.
    Tick(String, String),
}

#[relm4::component(pub)]
impl Component for NetworkSpeedModel {
    type CommandOutput = NetworkSpeedCommandOutput;
    type Input = NetworkSpeedInput;
    type Output = NetworkSpeedOutput;
    type Init = NetworkSpeedInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Box {
            set_css_classes: &["network-speed-bar-widget", "ok-button-surface", "ok-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,
            set_tooltip_text: Some("Network throughput — download / upload per second"),

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,
                set_halign: gtk::Align::Center,
                set_valign: gtk::Align::Center,
                set_hexpand: true,
                set_vexpand: true,

                gtk::Label {
                    add_css_class: "network-speed-down",
                    #[watch]
                    set_label: &format!("\u{2193} {}", model.down),
                },
                gtk::Label {
                    add_css_class: "network-speed-up",
                    #[watch]
                    set_label: &format!("\u{2191} {}", model.up),
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        _root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Sample /proc/net/dev on a fixed interval; the first sample only
        // primes the baseline (no rate yet), each later sample emits the
        // delta-per-second.
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut prev: Option<(u64, u64)> = None;
            loop {
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    _ = tokio::time::sleep(REFRESH_INTERVAL) => {}
                }
                let Some((rx, tx)) = read_net_totals() else {
                    continue;
                };
                if let Some((prx, ptx)) = prev {
                    let secs = REFRESH_INTERVAL.as_secs_f64();
                    let down = rx.saturating_sub(prx) as f64 / secs;
                    let up = tx.saturating_sub(ptx) as f64 / secs;
                    let _ = out.send(NetworkSpeedCommandOutput::Tick(
                        fmt_rate(down),
                        fmt_rate(up),
                    ));
                }
                prev = Some((rx, tx));
            }
        });

        let model = NetworkSpeedModel {
            down: "—".to_string(),
            up: "—".to_string(),
        };

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        _widgets: &mut Self::Widgets,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {}
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NetworkSpeedCommandOutput::Tick(down, up) => {
                self.down = down;
                self.up = up;
            }
        }
        self.update_view(widgets, sender);
    }
}

/// Sum received + transmitted bytes across all non-loopback interfaces
/// from `/proc/net/dev`. Returns `(rx_total, tx_total)`.
fn read_net_totals() -> Option<(u64, u64)> {
    let content = std::fs::read_to_string("/proc/net/dev").ok()?;
    Some(parse_net_dev(&content))
}

/// Parse `/proc/net/dev` contents into `(rx_total, tx_total)` across all
/// non-loopback interfaces. Split out from the file read so it's testable.
fn parse_net_dev(content: &str) -> (u64, u64) {
    let mut rx_total = 0u64;
    let mut tx_total = 0u64;
    // Skip the two header lines; each data line is "iface: rx ... tx ...".
    for line in content.lines().skip(2) {
        let Some((iface, rest)) = line.split_once(':') else {
            continue;
        };
        let iface = iface.trim();
        if iface == "lo" {
            continue;
        }
        let fields: Vec<&str> = rest.split_whitespace().collect();
        // Field 0 = rx bytes, field 8 = tx bytes (kernel column order).
        if let (Some(rx), Some(tx)) = (
            fields.first().and_then(|s| s.parse::<u64>().ok()),
            fields.get(8).and_then(|s| s.parse::<u64>().ok()),
        ) {
            rx_total += rx;
            tx_total += tx;
        }
    }
    (rx_total, tx_total)
}

/// Human-readable per-second rate: `B`, `K`, or `M` (binary units),
/// kept short so the pill width stays stable.
fn fmt_rate(bytes_per_sec: f64) -> String {
    const K: f64 = 1024.0;
    const M: f64 = K * K;
    if bytes_per_sec >= M {
        format!("{:.1}M", bytes_per_sec / M)
    } else if bytes_per_sec >= K {
        format!("{:.0}K", bytes_per_sec / K)
    } else {
        format!("{:.0}B", bytes_per_sec)
    }
}

#[cfg(test)]
mod tests {
    use super::{fmt_rate, parse_net_dev};

    #[test]
    fn fmt_rate_units() {
        assert_eq!(fmt_rate(0.0), "0B");
        assert_eq!(fmt_rate(512.0), "512B");
        assert_eq!(fmt_rate(1024.0), "1K");
        assert_eq!(fmt_rate(1536.0), "2K"); // rounds
        assert_eq!(fmt_rate(1024.0 * 1024.0), "1.0M");
        assert_eq!(fmt_rate(1024.0 * 1024.0 * 2.5), "2.5M");
    }

    #[test]
    fn parse_net_dev_sums_non_loopback() {
        let content = "\
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets
    lo: 1000       10    0    0    0     0          0         0   2000      20
  eth0: 5000       50    0    0    0     0          0         0   7000      70
  wlan0: 100        1    0    0    0     0          0         0    200       2
";
        // lo excluded; eth0 + wlan0 summed: rx = 5000+100, tx = 7000+200.
        assert_eq!(parse_net_dev(content), (5100, 7200));
    }

    #[test]
    fn parse_net_dev_handles_empty_and_garbage() {
        assert_eq!(parse_net_dev(""), (0, 0));
        assert_eq!(parse_net_dev("header1\nheader2\n"), (0, 0));
        // Malformed lines are skipped, not panicked on.
        assert_eq!(parse_net_dev("h1\nh2\nnonsense line\n"), (0, 0));
    }
}
