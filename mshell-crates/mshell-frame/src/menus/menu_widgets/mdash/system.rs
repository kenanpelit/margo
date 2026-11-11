//! The **System** tab: CPU, memory, temperature and disk with small
//! sparklines, battery + the power-profile selector, network throughput,
//! pending updates and the Claude / Codex limits. Each card opens its
//! detailed menu. The data comes from the shared services / polls (see
//! the dashboard's live feeds), only while this tab is on screen.

use super::card::{Card, GAP, empty_label, value_label};
use super::layout::{self, Slot};
use super::sparkline::Sparkline;
use crate::frame::MenuId;
use crate::menus::menu_widgets::control_center::power_profile::{
    ControlCenterPowerProfileInit, ControlCenterPowerProfileModel,
};
use mshell_config::schema::dashboard::{DashboardCard, DashboardTabConfig};
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ButtonExt, WidgetExt};
use relm4::{Component, ComponentController, Controller};
use std::rc::Rc;

/// Temperature ladder (DESIGN §2), the CPU dashboard's thresholds.
pub(crate) fn temp_severity(celsius: f64) -> &'static str {
    if celsius >= 85.0 {
        "danger"
    } else if celsius >= 75.0 {
        "warn"
    } else {
        "calm"
    }
}

/// Usage ladder for memory / disk / limits.
pub(crate) fn usage_severity(percent: f64) -> &'static str {
    if percent >= 90.0 {
        "danger"
    } else if percent >= 75.0 {
        "warn"
    } else {
        "calm"
    }
}

/// "9.8 GB" (decimal units, one decimal under 100).
pub(crate) fn bytes_short(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1000.0 && i < UNITS.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 || v >= 100.0 {
        format!("{v:.0} {}", UNITS[i])
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// "1.2 MB/s".
pub(crate) fn rate(bytes_per_sec: u64) -> String {
    format!("{}/s", bytes_short(bytes_per_sec))
}

/// The battery card's second line: "3 h 12 min left" on battery, "Full in
/// 45 min" while charging (UPower's estimates, seconds; 0 = unknown).
pub(crate) fn battery_time_line(charging: bool, time_to_empty: i64, time_to_full: i64) -> String {
    let (secs, template): (i64, fn(&str) -> String) = if charging {
        (time_to_full, |t| format!("Full in {t}"))
    } else {
        (time_to_empty, |t| format!("{t} left"))
    };
    if secs <= 0 {
        return if charging {
            "Charging".to_string()
        } else {
            "Estimating time left…".to_string()
        };
    }
    let mins = (secs + 30) / 60;
    let text = match (mins / 60, mins % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    };
    template(&text)
}

/// One limits row: "Claude · 5h", 42 %.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AiRow {
    pub(crate) label: String,
    pub(crate) percent: f64,
}

/// The pending updates: total + the per-source breakdown line.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct UpdatesView {
    pub(crate) total: usize,
    pub(crate) detail: String,
    /// What pacman will download for the repo part (`None` = unknown).
    pub(crate) download: Option<u64>,
}

/// The updates card's third line.
pub(crate) fn download_line(u: &UpdatesView) -> String {
    match (u.total, u.download) {
        (0, _) => "Nothing to download".to_string(),
        (_, Some(b)) if b > 0 => format!("{} to download", bytes_short(b)),
        (_, Some(_)) => "Already downloaded".to_string(),
        (_, None) => "Download size not known yet".to_string(),
    }
}

/// A metric card: big value + dim detail + sparkline (or a bar).
struct Metric {
    card: Card,
    value: gtk::Label,
    detail: gtk::Label,
    spark: Option<Sparkline>,
}

impl Metric {
    fn new(
        icon: &str,
        title: &str,
        spark: Option<Sparkline>,
        target: MenuId,
        open: Rc<dyn Fn(MenuId)>,
    ) -> Self {
        let card = Card::new(icon, title).opens(title, move || open(target));
        card.root.add_css_class("mdash-metric");
        let value = value_label("mdash-metric-value");
        card.body.append(&value);
        let detail = value_label("mdash-dim");
        card.body.append(&detail);
        if let Some(s) = &spark {
            // Fixed height (`.mdash-sparkline`), right under the text.
            s.area.set_vexpand(false);
            card.body.append(&s.area);
        }
        Self {
            card,
            value,
            detail,
            spark,
        }
    }

    fn set(&self, value: &str, detail: &str, severity: Option<&str>) {
        self.value.set_label(value);
        self.detail.set_label(detail);
        for class in ["calm", "warn", "danger"] {
            self.value.remove_css_class(class);
        }
        if let Some(sev) = severity {
            self.value.add_css_class(sev);
        }
    }

    fn push(&self, v: f64) {
        if let Some(s) = &self.spark {
            s.push(v);
        }
    }
}

pub(crate) struct SystemTab {
    pub(crate) root: gtk::Box,
    cpu: Metric,
    memory: Metric,
    temp: Metric,
    disk: Metric,
    disk_bar: gtk::LevelBar,
    disk_used: gtk::Label,
    battery_card: Card,
    battery_value: gtk::Label,
    battery_detail: gtk::Label,
    battery_time: gtk::Label,
    pub(crate) power_profile: Controller<ControlCenterPowerProfileModel>,
    network: Metric,
    updates_card: Card,
    updates_value: gtk::Label,
    updates_detail: gtk::Label,
    updates_download: gtk::Label,
    updates_button: gtk::Button,
    ai_card: Card,
    ai_rows: gtk::Box,
    ai_empty: gtk::Label,
}

impl SystemTab {
    pub(crate) fn new(cfg: &DashboardTabConfig, open: impl Fn(MenuId) + 'static) -> Self {
        let open: Rc<dyn Fn(MenuId)> = Rc::new(open);
        let root = gtk::Box::new(gtk::Orientation::Vertical, GAP);
        root.add_css_class("mdash-system");

        let cpu_menu = MenuId::CpuDashboard;
        let cpu = Metric::new(
            "cpu-symbolic",
            "CPU",
            Some(Sparkline::new(Some(100.0))),
            cpu_menu,
            open.clone(),
        );
        let memory = Metric::new(
            "memory-symbolic",
            "Memory",
            Some(Sparkline::new(Some(100.0))),
            cpu_menu,
            open.clone(),
        );
        let temp = Metric::new(
            "temperature-symbolic",
            "Temperature",
            Some(Sparkline::new(Some(100.0))),
            cpu_menu,
            open.clone(),
        );
        let disk = Metric::new(
            "drive-harddisk-symbolic",
            "Disk",
            None,
            cpu_menu,
            open.clone(),
        );
        // The disk's visual sits in the same fixed-height slot as the
        // sparklines beside it: the used / total line over a usage bar.
        let disk_visual = gtk::Box::new(gtk::Orientation::Vertical, 6);
        disk_visual.add_css_class("mdash-metric-visual");
        let disk_used = value_label("mdash-dim");
        disk_used.set_valign(gtk::Align::End);
        disk_used.set_vexpand(true);
        disk_visual.append(&disk_used);
        let disk_bar = gtk::LevelBar::new();
        disk_bar.add_css_class("mdash-bar");
        disk_visual.append(&disk_bar);
        disk.card.body.append(&disk_visual);

        let battery_card = {
            let open = open.clone();
            Card::new("battery-good-symbolic", "Battery & power")
                .opens("Power", move || open(MenuId::Power))
        };
        let battery_line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let battery_value = value_label("mdash-metric-value");
        battery_line.append(&battery_value);
        let battery_detail = value_label("mdash-dim");
        battery_detail.set_valign(gtk::Align::Center);
        battery_line.append(&battery_detail);
        battery_card.body.append(&battery_line);
        let battery_time = value_label("mdash-dim");
        battery_card.body.append(&battery_time);
        let power_profile = ControlCenterPowerProfileModel::builder()
            .launch(ControlCenterPowerProfileInit {})
            .detach();
        battery_card.body.append(power_profile.widget());

        let network = Metric::new(
            "network-transmit-receive-symbolic",
            "Network",
            Some(Sparkline::new(None)),
            MenuId::Network,
            open.clone(),
        );

        let updates_card = {
            let open = open.clone();
            Card::new("software-update-available-symbolic", "Updates")
                .opens("Updates", move || open(MenuId::SystemUpdate))
        };
        // Count + sources on the left, the action inline on the right.
        let updates_line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let updates_text = gtk::Box::new(gtk::Orientation::Vertical, 4);
        updates_text.set_hexpand(true);
        let updates_value = value_label("mdash-metric-value");
        updates_text.append(&updates_value);
        let updates_detail = value_label("mdash-dim");
        updates_text.append(&updates_detail);
        let updates_download = value_label("mdash-dim");
        updates_text.append(&updates_download);
        updates_line.append(&updates_text);
        let updates_button = gtk::Button::with_label("Review & update");
        updates_button.add_css_class("ok-button-primary");
        updates_button.add_css_class("mdash-inline-button");
        updates_button.set_valign(gtk::Align::Center);
        {
            let open = open.clone();
            updates_button.connect_clicked(move |_| open(MenuId::SystemUpdate));
        }
        updates_line.append(&updates_button);
        updates_card.body.append(&updates_line);

        let ai_card = {
            let open = open.clone();
            Card::new("ai-agents-symbolic", "AI usage")
                .opens("AI agents", move || open(MenuId::Agents))
        };
        let ai_rows = gtk::Box::new(gtk::Orientation::Vertical, 8);
        ai_card.body.append(&ai_rows);
        let ai_empty = empty_label("No Claude or Codex sign-in found");
        ai_card.body.append(&ai_empty);

        let tab = Self {
            root,
            cpu,
            memory,
            temp,
            disk,
            disk_bar,
            disk_used,
            battery_card,
            battery_value,
            battery_detail,
            battery_time,
            power_profile,
            network,
            updates_card,
            updates_value,
            updates_detail,
            updates_download,
            updates_button,
            ai_card,
            ai_rows,
            ai_empty,
        };
        // One homogeneous row per layout row: the cards of a row share its
        // height, and every card keeps its natural content (no card
        // vexpands, nothing is pushed to a card's bottom).
        let mut rows: Vec<gtk::Box> = Vec::new();
        for Slot { card, row, .. } in layout::system(|c| cfg.shows(c)) {
            let widget = match card {
                DashboardCard::Cpu => &tab.cpu.card.root,
                DashboardCard::Memory => &tab.memory.card.root,
                DashboardCard::Temperature => &tab.temp.card.root,
                DashboardCard::Disk => &tab.disk.card.root,
                DashboardCard::Battery => &tab.battery_card.root,
                DashboardCard::Network => &tab.network.card.root,
                DashboardCard::Updates => &tab.updates_card.root,
                DashboardCard::AiUsage => &tab.ai_card.root,
                _ => continue,
            };
            let row = row as usize;
            while rows.len() <= row {
                let b = gtk::Box::new(gtk::Orientation::Horizontal, GAP);
                b.set_homogeneous(true);
                b.add_css_class("mdash-system-row");
                tab.root.append(&b);
                rows.push(b);
            }
            rows[row].append(widget);
        }
        tab.apply_ai(&[]);
        tab.apply_updates(None);
        tab.apply_battery(None);
        tab
    }

    pub(crate) fn apply_cpu(&self, percent: f64, freq_mhz: Option<u64>) {
        let detail = freq_mhz
            .filter(|f| *f > 0)
            .map(|f| format!("{:.1} GHz", f as f64 / 1000.0))
            .unwrap_or_default();
        self.cpu.set(
            &format!("{percent:.0}%"),
            &detail,
            Some(usage_severity(percent)),
        );
        self.cpu.push(percent);
    }

    pub(crate) fn apply_memory(&self, used: u64, total: u64, percent: f64) {
        self.memory.set(
            &format!("{percent:.0}%"),
            &format!("{} of {}", bytes_short(used), bytes_short(total)),
            Some(usage_severity(percent)),
        );
        self.memory.push(percent);
    }

    pub(crate) fn apply_temp(&self, celsius: Option<f64>) {
        match celsius {
            Some(c) => {
                self.temp
                    .set(&format!("{c:.0}°C"), "CPU package", Some(temp_severity(c)));
                self.temp.push(c);
            }
            None => self.temp.set("—", "No sensor", None),
        }
    }

    /// The root filesystem (or the fullest one when `/` is unknown).
    pub(crate) fn apply_disk(&self, mount: &str, used: u64, total: u64) {
        let pct = if total > 0 {
            used as f64 / total as f64 * 100.0
        } else {
            0.0
        };
        self.disk.set(
            &format!("{pct:.0}%"),
            &format!("{mount} · {} free", bytes_short(total.saturating_sub(used))),
            Some(usage_severity(pct)),
        );
        self.disk_used.set_label(&format!(
            "{} of {} used",
            bytes_short(used),
            bytes_short(total)
        ));
        self.disk_bar.set_value((pct / 100.0).clamp(0.0, 1.0));
    }

    pub(crate) fn apply_net(&self, rx_per_sec: u64, tx_per_sec: u64) {
        self.network.set(
            &format!("↓ {}", rate(rx_per_sec)),
            &format!("↑ {}", rate(tx_per_sec)),
            None,
        );
        self.network.push(rx_per_sec as f64);
    }

    /// `None` = no battery: the card is about the power profile only.
    /// `time` is the second line ([`battery_time_line`]).
    pub(crate) fn apply_battery(&self, battery: Option<(u8, &str, String)>) {
        match battery {
            Some((pct, state, time)) => {
                self.battery_card.set_title("Battery & power");
                self.battery_value.set_label(&format!("{pct}%"));
                self.battery_detail.set_label(state);
                self.battery_time.set_label(&time);
            }
            None => {
                self.battery_card.set_title("Power");
                self.battery_value.set_label("AC");
                self.battery_detail.set_label("Plugged in");
                self.battery_time.set_label("No battery");
            }
        }
    }

    /// `None` = not known yet.
    pub(crate) fn apply_updates(&self, updates: Option<&UpdatesView>) {
        match updates {
            None => {
                self.updates_value.set_label("—");
                self.updates_detail.set_label("Checking…");
                self.updates_download.set_label("Repo, AUR and Flatpak");
                self.updates_button.set_visible(false);
            }
            Some(u) if u.total == 0 => {
                self.updates_value.set_label("Up to date");
                self.updates_detail.set_label(&u.detail);
                self.updates_download.set_label(&download_line(u));
                self.updates_button.set_visible(false);
            }
            Some(u) => {
                self.updates_value
                    .set_label(&format!("{} pending", u.total));
                self.updates_detail.set_label(&u.detail);
                self.updates_download.set_label(&download_line(u));
                self.updates_button.set_visible(true);
            }
        }
    }

    pub(crate) fn apply_ai(&self, rows: &[AiRow]) {
        while let Some(child) = self.ai_rows.first_child() {
            self.ai_rows.remove(&child);
        }
        self.ai_empty.set_visible(rows.is_empty());
        self.ai_rows.set_visible(!rows.is_empty());
        for r in rows.iter().take(4) {
            // One line per window: label · bar · percent.
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let label = gtk::Label::new(Some(&r.label));
            label.add_css_class("mdash-ai-label");
            label.set_xalign(0.0);
            // The longest default window name ("Claude · weekly") whole.
            label.set_width_chars(16);
            label.set_max_width_chars(16);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            row.append(&label);
            let bar = gtk::LevelBar::new();
            bar.add_css_class("mdash-bar");
            bar.add_css_class(usage_severity(r.percent));
            bar.set_hexpand(true);
            bar.set_valign(gtk::Align::Center);
            bar.set_value((r.percent / 100.0).clamp(0.0, 1.0));
            row.append(&bar);
            let pct = gtk::Label::new(Some(&format!("{:.0}%", r.percent)));
            pct.add_css_class("mdash-ai-percent");
            pct.add_css_class(usage_severity(r.percent));
            pct.set_xalign(1.0);
            pct.set_width_chars(4);
            row.append(&pct);
            self.ai_rows.append(&row);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_and_rate_formats() {
        assert_eq!(bytes_short(512), "512 B");
        assert_eq!(bytes_short(9_800_000_000), "9.8 GB");
        assert_eq!(bytes_short(476_000_000_000), "476 GB");
        assert_eq!(rate(1_234_000), "1.2 MB/s");
        assert_eq!(rate(0), "0 B/s");
    }

    #[test]
    fn download_line_says_what_is_known() {
        let u = |total, download| UpdatesView {
            total,
            detail: String::new(),
            download,
        };
        assert_eq!(
            download_line(&u(7, Some(182_000_000))),
            "182 MB to download"
        );
        assert_eq!(download_line(&u(7, Some(0))), "Already downloaded");
        assert_eq!(download_line(&u(7, None)), "Download size not known yet");
        assert_eq!(download_line(&u(0, None)), "Nothing to download");
    }

    #[test]
    fn battery_time_reads_naturally() {
        assert_eq!(battery_time_line(false, 11_520, 0), "3 h 12 min left");
        assert_eq!(battery_time_line(false, 7_200, 0), "2 h left");
        assert_eq!(battery_time_line(true, 0, 2_700), "Full in 45 min");
        assert_eq!(battery_time_line(false, 0, 0), "Estimating time left…");
        assert_eq!(battery_time_line(true, 0, 0), "Charging");
    }

    #[test]
    fn severity_ladders() {
        assert_eq!(temp_severity(58.0), "calm");
        assert_eq!(temp_severity(78.0), "warn");
        assert_eq!(temp_severity(90.0), "danger");
        assert_eq!(usage_severity(40.0), "calm");
        assert_eq!(usage_severity(80.0), "warn");
        assert_eq!(usage_severity(95.0), "danger");
    }
}
