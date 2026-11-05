//! Drives tab of the CPU dashboard: NVMe inventory + live temperature
//! from sysfs (no root), and full SMART health (health verdict, wear,
//! spare, error counters, writes, power-on hours) from `smartctl -j -a`.
//! NVMe SMART needs root, so the full read is an explicit button that
//! runs every device behind ONE pkexec prompt; nothing privileged runs
//! on its own. Polls sysfs every 5 s only while the tab is visible.

use super::drives::{
    Health, SmartRead, SmartReport, SysDrive, assess, parse_scan, parse_smart_stream,
    privileged_argv, read_sys_drives,
};
use super::ui;
use crate::menus::reveal_poll::RevealPoll;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::Path;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// SMART state of the tab as a whole.
#[derive(Debug, Clone, PartialEq)]
enum SmartState {
    /// Not tried yet.
    Idle,
    Reading,
    /// `smartctl` missing.
    NoTool,
    /// Unprivileged read refused: offer the pkexec button.
    NeedsRoot,
    Loaded(Vec<SmartRead>),
    Failed(String),
}

pub(crate) struct DrivesTabModel {
    poll: RevealPoll,
    drives: Vec<SysDrive>,
    devices: Vec<String>,
    smart: SmartState,
    tried_unprivileged: bool,
    list: gtk::Box,
}

#[derive(Debug)]
pub(crate) enum DrivesTabInput {
    SetActive(bool),
    Poll,
    ReadSmartAdmin,
}

#[derive(Debug)]
pub(crate) enum DrivesTabCmd {
    Sys(Vec<SysDrive>),
    /// `(devices from --scan, unprivileged reads)`; `None` = no smartctl.
    Unprivileged(Option<(Vec<String>, Vec<SmartRead>)>),
    Admin(Result<Vec<SmartRead>, String>),
}

async fn smartctl(args: &[&str]) -> Option<String> {
    let out = tokio::process::Command::new("smartctl")
        .args(args)
        .output()
        .await
        .ok()?;
    // smartctl's exit status is a bitmask; the JSON on stdout is the
    // source of truth either way.
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

async fn unprivileged_read() -> Option<(Vec<String>, Vec<SmartRead>)> {
    let devices = parse_scan(&smartctl(&["-j", "--scan"]).await?);
    let mut reads = Vec::new();
    for d in &devices {
        if let Some(out) = smartctl(&["-j", "-a", d]).await {
            reads.extend(parse_smart_stream(&out));
        }
    }
    Some((devices, reads))
}

async fn admin_read(devices: Vec<String>) -> Result<Vec<SmartRead>, String> {
    let argv = privileged_argv(&devices).ok_or("No drives to read")?;
    if !mshell_privileged::installed() {
        return Err(mshell_privileged::MISSING.to_string());
    }
    let out = tokio::process::Command::new("pkexec")
        .args(&argv)
        .output()
        .await
        .map_err(|e| format!("pkexec unavailable: {e}"))?;
    let code = out.status.code();
    if matches!(code, Some(126 | 127)) {
        super::pkexec_outcome(code, "")?;
    }
    let reads = parse_smart_stream(&String::from_utf8_lossy(&out.stdout));
    if reads.is_empty() {
        super::pkexec_outcome(code, &String::from_utf8_lossy(&out.stderr))?;
        return Err("smartctl returned no data".to_string());
    }
    Ok(reads)
}

#[relm4::component(pub(crate))]
impl Component for DrivesTabModel {
    type CommandOutput = DrivesTabCmd;
    type Input = DrivesTabInput;
    type Output = ();
    type Init = ();

    view! {
        #[root]
        gtk::Box {
            add_css_class: "cpu-dashboard-tab",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            #[name = "fill_list"]
            gtk::ScrolledWindow {
                set_min_content_height: 0,
                set_max_content_height: 420,
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_propagate_natural_height: true,
                #[local_ref]
                list -> gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 8,
                },
            },

            // §17 no-permission state: neutral explanation + the action.
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 8,
                #[watch]
                set_visible: matches!(
                    model.smart,
                    SmartState::NeedsRoot | SmartState::Failed(_) | SmartState::Reading
                ),
                gtk::Label {
                    set_css_classes: &["label-small", "dim-label"],
                    set_xalign: 0.0,
                    set_wrap: true,
                    #[watch]
                    set_label: &match &model.smart {
                        SmartState::Failed(e) => e.clone(),
                        SmartState::Reading => "Reading SMART data…".to_string(),
                        _ => "Full SMART health (wear, spare, error counters) needs \
                              administrator access for NVMe drives.".to_string(),
                    },
                },
                gtk::Button {
                    set_css_classes: &["ok-button-surface", "label-small"],
                    set_halign: gtk::Align::Start,
                    set_label: "Read SMART health…",
                    #[watch]
                    set_sensitive: model.smart != SmartState::Reading,
                    connect_clicked => DrivesTabInput::ReadSmartAdmin,
                },
            },

            gtk::Label {
                set_css_classes: &["label-small", "dim-label"],
                set_xalign: 0.0,
                set_wrap: true,
                #[watch]
                set_visible: model.smart == SmartState::NoTool,
                set_label: "smartctl is not installed (package smartmontools) — \
                            showing temperatures only.",
            },
        }
    }

    fn init(_: (), root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let s = sender.clone();
        let poll = RevealPoll::new(POLL_INTERVAL, move || {
            s.input_sender().send(DrivesTabInput::Poll).is_ok()
        });
        poll.set_revealed(false);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let model = DrivesTabModel {
            poll,
            drives: Vec::new(),
            devices: Vec::new(),
            smart: SmartState::Idle,
            tried_unprivileged: false,
            list: list.clone(),
        };
        let widgets = view_output!();
        crate::menus::list_fit::mark_menu_list(&widgets.fill_list, 420);
        model.render();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match msg {
            DrivesTabInput::SetActive(on) => {
                self.poll.set_revealed(on);
                if on {
                    sender.input(DrivesTabInput::Poll);
                    if !self.tried_unprivileged {
                        self.tried_unprivileged = true;
                        self.smart = SmartState::Reading;
                        sender.oneshot_command(async {
                            DrivesTabCmd::Unprivileged(unprivileged_read().await)
                        });
                    }
                }
            }
            DrivesTabInput::Poll => {
                sender.oneshot_command(async {
                    let d = tokio::task::spawn_blocking(|| {
                        read_sys_drives(Path::new("/sys/class/nvme"))
                    })
                    .await
                    .unwrap_or_default();
                    DrivesTabCmd::Sys(d)
                });
            }
            DrivesTabInput::ReadSmartAdmin => {
                if self.smart == SmartState::Reading {
                    return;
                }
                self.smart = SmartState::Reading;
                let devices = if self.devices.is_empty() {
                    self.drives.iter().map(|d| d.dev.clone()).collect()
                } else {
                    self.devices.clone()
                };
                sender
                    .oneshot_command(async move { DrivesTabCmd::Admin(admin_read(devices).await) });
            }
        }
    }

    fn update_cmd(&mut self, msg: Self::CommandOutput, _s: ComponentSender<Self>, _r: &Self::Root) {
        match msg {
            DrivesTabCmd::Sys(d) => {
                // Same drives at the same temperatures: nothing to redraw.
                if !ui::replace_if_changed(&mut self.drives, d) {
                    return;
                }
            }
            DrivesTabCmd::Unprivileged(None) => self.smart = SmartState::NoTool,
            DrivesTabCmd::Unprivileged(Some((devices, reads))) => {
                self.devices = devices;
                let needs_root =
                    reads.is_empty() || reads.iter().any(|r| matches!(r, SmartRead::NeedsRoot(_)));
                self.smart = if needs_root {
                    SmartState::NeedsRoot
                } else {
                    SmartState::Loaded(reads)
                };
            }
            DrivesTabCmd::Admin(Ok(reads)) => self.smart = SmartState::Loaded(reads),
            DrivesTabCmd::Admin(Err(e)) => self.smart = SmartState::Failed(e),
        }
        self.render();
    }
}

fn temp_class(c: i64) -> &'static str {
    // Same amber / red tiers as the rest of the dashboard's temperatures.
    if c >= 85 {
        "metric-critical"
    } else if c >= 70 {
        "metric-warning"
    } else {
        ""
    }
}

fn detail_row(text: &str) -> gtk::ListBoxRow {
    let l = gtk::Label::builder()
        .label(text)
        .css_classes(["cpu-dashboard-stat-detail"])
        .xalign(0.0)
        .wrap(true)
        .build();
    ui::plain_row(&l)
}

/// `Some("3%")`-style helper: drops rows for counters the drive omits.
fn opt<T: std::fmt::Display>(v: Option<T>, suffix: &str) -> Option<String> {
    v.map(|v| format!("{v}{suffix}"))
}

fn report_rows(list: &gtk::ListBox, r: &SmartReport) {
    let health = assess(r);
    let (title, why) = match &health {
        Health::Good => ("Healthy".to_string(), String::new()),
        Health::Warn(w) => ("Needs attention".to_string(), w.join(" · ")),
        Health::Bad(w) => ("Failing".to_string(), w.join(" · ")),
    };
    let verdict = gtk::Label::builder()
        .label(&title)
        .valign(gtk::Align::Center)
        .build();
    if health.class().is_empty() {
        verdict.set_css_classes(&["cpu-dashboard-stat-value"]);
    } else {
        verdict.set_css_classes(&["cpu-dashboard-stat-value", health.class()]);
    }
    list.append(&ui::plain_row(&ui::row_body(
        "SMART health",
        &why,
        &[verdict.into()],
    )));
    let line = |parts: Vec<Option<String>>| -> Option<String> {
        let p: Vec<String> = parts.into_iter().flatten().collect();
        (!p.is_empty()).then(|| p.join(" · "))
    };
    let rows = [
        line(vec![
            opt(r.wear_pct, "% worn").map(|s| format!("Wear {s}")),
            opt(r.spare_pct, "% spare"),
            r.written_tb.map(|t| format!("{t:.1} TB written")),
        ]),
        line(vec![
            opt(r.power_on_hours, " h powered on"),
            opt(r.unsafe_shutdowns, " unsafe shutdowns"),
        ]),
        line(vec![
            opt(r.media_errors, " media errors"),
            opt(r.error_log_entries, " error-log entries"),
            opt(r.bad_sectors, " bad sectors"),
        ]),
    ];
    for text in rows.into_iter().flatten() {
        list.append(&detail_row(&text));
    }
}

impl DrivesTabModel {
    fn render(&self) {
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        let reports: Vec<&SmartRead> = match &self.smart {
            SmartState::Loaded(r) => r.iter().collect(),
            _ => Vec::new(),
        };
        let report_for = |dev: &str| {
            reports.iter().find_map(|r| match r {
                SmartRead::Report(rep) if rep.dev == dev => Some(rep),
                _ => None,
            })
        };
        let error_for = |dev: &str| {
            reports.iter().find_map(|r| match r {
                SmartRead::Error(e) if e.contains(dev) => Some(e.clone()),
                _ => None,
            })
        };
        // sysfs NVMe controllers first, then any SMART-only device (SATA).
        let mut devs: Vec<(String, String, Option<i64>)> = self
            .drives
            .iter()
            .map(|d| (d.dev.clone(), d.model.clone(), d.temp_c.map(i64::from)))
            .collect();
        for r in &reports {
            if let SmartRead::Report(rep) = r
                && !devs.iter().any(|(d, _, _)| *d == rep.dev)
            {
                devs.push((rep.dev.clone(), rep.model.clone(), rep.temp_c));
            }
        }
        if devs.is_empty() {
            let list = gtk::ListBox::new();
            list.set_css_classes(&["ok-boxed-list", "compact", "cpu-dashboard-list"]);
            list.append(&ui::placeholder_row("No NVMe drives found"));
            self.list.append(&list);
            return;
        }
        for (dev, model, sys_temp) in devs {
            let list = gtk::ListBox::new();
            list.set_css_classes(&["ok-boxed-list", "compact", "cpu-dashboard-list"]);
            list.set_selection_mode(gtk::SelectionMode::None);
            let rep = report_for(&dev);
            let temp = sys_temp.or(rep.and_then(|r| r.temp_c));
            let temp_label = gtk::Label::builder()
                .label(temp.map(|t| format!("{t}°C")).unwrap_or_else(|| "—".into()))
                .valign(gtk::Align::Center)
                .build();
            temp_label.set_css_classes(&match temp.map(temp_class).filter(|c| !c.is_empty()) {
                Some(c) => vec!["cpu-dashboard-stat-value", c],
                None => vec!["cpu-dashboard-stat-value"],
            });
            let title = if model.is_empty() { dev.clone() } else { model };
            list.append(&ui::plain_row(&ui::row_body(
                &title,
                &dev,
                &[temp_label.into()],
            )));
            if let Some(r) = rep {
                report_rows(&list, r);
            } else if let Some(e) = error_for(&dev) {
                list.append(&detail_row(&e));
            }
            self.list.append(&list);
        }
    }
}
