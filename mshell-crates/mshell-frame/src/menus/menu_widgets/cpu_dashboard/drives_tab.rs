//! Drives tab of the CPU dashboard: NVMe inventory + live temperature
//! from sysfs (no root), and full SMART health (health verdict, wear,
//! spare, error counters, writes, power-on hours) from `smartctl -j -a`.
//! NVMe SMART needs root, so the full read is an explicit button that
//! runs every device behind ONE pkexec prompt; nothing privileged runs
//! on its own. Polls sysfs every 5 s only while the tab is visible.
//!
//! Once SMART is loaded, each drive that supports it offers a short or
//! long **self-test** (`mshell-privileged smart-selftest`, one prompt):
//! the helper's progress stream drives a progress bar, and the finished
//! test's result and fresh SMART data replace the drive's rows (plus a
//! notification — a long test can take hours). Every full read also goes
//! to the drive-health monitor ([`crate::drive_health`]: alerts, history),
//! whose recorded history draws a per-drive temperature (and wear) chart.

use super::drive_chart;
use super::drives::{
    Health, Limits, SmartRead, SmartReport, SysDrive, assess, parse_scan, parse_smart_stream,
    parse_smart_value, privileged_argv, read_sys_drives,
};
use super::selftest::{self, LastTest, SelfTest, Step, Tracker};
use super::ui;
use crate::drive_health::{self, history::History, scan::drive_id};
use crate::menus::reveal_poll::RevealPoll;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::collections::HashMap;
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

/// A self-test this tab started and is following.
#[derive(Debug, Clone, PartialEq)]
struct Run {
    test: SelfTest,
    /// The drive accepted it (after the password prompt).
    started: bool,
    done_pct: Option<u8>,
}

pub(crate) struct DrivesTabModel {
    poll: RevealPoll,
    drives: Vec<SysDrive>,
    devices: Vec<String>,
    smart: SmartState,
    tried_unprivileged: bool,
    /// Self-tests in flight, by device.
    runs: HashMap<String, Run>,
    /// Why the last self-test start failed, by device.
    run_errors: HashMap<String, String>,
    history: History,
    list: gtk::Box,
}

#[derive(Debug)]
pub(crate) enum DrivesTabInput {
    SetActive(bool),
    Poll,
    ReadSmartAdmin,
    StartSelfTest(String, SelfTest),
}

/// What the self-test stream reports.
#[derive(Debug)]
pub(crate) enum RunEvent {
    Started,
    Progress(Option<u8>),
    Done(Option<LastTest>, Box<SmartRead>),
    Failed(String),
}

#[derive(Debug)]
pub(crate) enum DrivesTabCmd {
    Sys(Vec<SysDrive>),
    /// `(devices from --scan, unprivileged reads)`; `None` = no smartctl.
    Unprivileged(Option<(Vec<String>, Vec<SmartRead>)>),
    Admin(Result<Vec<SmartRead>, String>),
    History(History),
    Run(String, RunEvent),
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

/// Start `test` on `dev` through the helper and follow its stream until
/// the test is done (then the pipe is dropped, which ends the helper).
async fn run_self_test(
    dev: String,
    test: SelfTest,
    before: Option<Option<LastTest>>,
    out: relm4::Sender<DrivesTabCmd>,
) {
    use tokio::io::AsyncBufReadExt;
    let send = |e: RunEvent| {
        let _ = out.send(DrivesTabCmd::Run(dev.clone(), e));
    };
    let Some(argv) = selftest::start_argv(&dev, test) else {
        return send(RunEvent::Failed(format!("{dev} is not a drive path")));
    };
    if !mshell_privileged::installed() {
        return send(RunEvent::Failed(mshell_privileged::MISSING.to_string()));
    }
    let child = tokio::process::Command::new("pkexec")
        .args(&argv)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return send(RunEvent::Failed(format!("pkexec unavailable: {e}"))),
    };
    let Some(stdout) = child.stdout.take() else {
        return send(RunEvent::Failed("no output from the helper".to_string()));
    };
    let mut lines = tokio::io::BufReader::new(stdout).lines();
    let mut tracker = Tracker::new(before);
    while let Ok(Some(line)) = lines.next_line().await {
        match tracker.step(&line) {
            Step::Started => send(RunEvent::Started),
            Step::Progress(p) => send(RunEvent::Progress(p)),
            Step::Waiting => {}
            Step::Refused(why) => return send(RunEvent::Failed(why)),
            Step::Done(last, doc) => {
                // Dropping the reader closes the pipe; the root helper
                // stops at its next report. tokio reaps the child.
                return send(RunEvent::Done(last, Box::new(parse_smart_value(&doc))));
            }
        }
    }
    // The stream ended before the test did: dismissed prompt, helper
    // error, or the watch limit.
    let why = match child.wait_with_output().await {
        Ok(o) => super::pkexec_outcome(o.status.code(), &String::from_utf8_lossy(&o.stderr))
            .err()
            .unwrap_or_else(|| "The self-test watch ended early".to_string()),
        Err(e) => e.to_string(),
    };
    send(RunEvent::Failed(why));
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
                        _ => "Full SMART health (wear, spare, error counters, self-tests) \
                              needs administrator access for NVMe drives.".to_string(),
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
            runs: HashMap::new(),
            run_errors: HashMap::new(),
            history: History::default(),
            list: list.clone(),
        };
        let widgets = view_output!();
        crate::menus::list_fit::mark_menu_list(&widgets.fill_list, 420);
        model.render(&sender);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match msg {
            DrivesTabInput::SetActive(on) => {
                self.poll.set_revealed(on);
                if on {
                    sender.input(DrivesTabInput::Poll);
                    sender.oneshot_command(async {
                        let h = tokio::task::spawn_blocking(drive_health::history_snapshot)
                            .await
                            .unwrap_or_default();
                        DrivesTabCmd::History(h)
                    });
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
            DrivesTabInput::StartSelfTest(dev, test) => {
                if self.runs.contains_key(&dev) {
                    return;
                }
                self.run_errors.remove(&dev);
                self.runs.insert(
                    dev.clone(),
                    Run {
                        test,
                        started: false,
                        done_pct: None,
                    },
                );
                let before = self.report_for(&dev).map(|r| r.self_test.last.clone());
                sender.command(move |out, shutdown| {
                    shutdown
                        .register(run_self_test(dev, test, before, out))
                        .drop_on_shutdown()
                });
                self.render(&sender);
            }
        }
    }

    fn update_cmd(
        &mut self,
        msg: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _r: &Self::Root,
    ) {
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
                    hand_to_monitor(&reads);
                    SmartState::Loaded(reads)
                };
            }
            DrivesTabCmd::Admin(Ok(reads)) => {
                hand_to_monitor(&reads);
                self.smart = SmartState::Loaded(reads);
            }
            DrivesTabCmd::Admin(Err(e)) => self.smart = SmartState::Failed(e),
            DrivesTabCmd::History(h) => {
                if !ui::replace_if_changed(&mut self.history, h) {
                    return;
                }
            }
            DrivesTabCmd::Run(dev, event) => self.on_run(dev, event),
        }
        self.render(&sender);
    }
}

/// Give full SMART reports to the drive-health monitor (alerts, history).
fn hand_to_monitor(reads: &[SmartRead]) {
    let reports: Vec<SmartReport> = reads
        .iter()
        .filter_map(|r| match r {
            SmartRead::Report(rep) => Some(rep.clone()),
            _ => None,
        })
        .collect();
    drive_health::smart_read(reports);
}

fn test_name(test: SelfTest) -> &'static str {
    match test {
        SelfTest::Short => "Short",
        SelfTest::Long => "Long",
    }
}

/// The finished-test notification: `(summary, body, critical)`.
fn finished_notice(name: &str, test: SelfTest, last: Option<&LastTest>) -> (String, String, bool) {
    let summary = format!("{} self-test finished: {name}", test_name(test));
    match last {
        Some(l) => (summary, l.result.clone(), !l.passed),
        None => (summary, "The drive logged no result".to_string(), false),
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

fn report_rows(list: &gtk::ListBox, r: &SmartReport, limits: &Limits) {
    let health = assess(r, limits);
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

/// The self-test row's subtitle.
fn self_test_subtitle(r: &SmartReport, run: Option<&Run>, error: Option<&String>) -> String {
    if let Some(run) = run {
        let kind = test_name(run.test);
        return match (run.started, run.done_pct) {
            (false, _) => format!("Starting the {} test…", kind.to_lowercase()),
            (true, Some(p)) => format!("{kind} test running · {p}% done"),
            (true, None) => format!("{kind} test running"),
        };
    }
    if let Some(running) = &r.self_test.running {
        return match running.done_pct {
            Some(p) => format!("{} test running · {p}% done", running.kind),
            None => format!("{} test running", running.kind),
        };
    }
    let mut s = match &r.self_test.last {
        Some(l) => {
            let at = l.hours.map(|h| format!(" · at {h} h")).unwrap_or_default();
            format!("Last: {} · {}{at}", l.kind, l.result)
        }
        None => "No self-test logged".to_string(),
    };
    if let Some(e) = error {
        s.push_str(" · ");
        s.push_str(e);
    }
    s
}

impl DrivesTabModel {
    fn report_for(&self, dev: &str) -> Option<&SmartReport> {
        match &self.smart {
            SmartState::Loaded(reads) => reads.iter().find_map(|r| match r {
                SmartRead::Report(rep) if rep.dev == dev => Some(rep),
                _ => None,
            }),
            _ => None,
        }
    }

    fn on_run(&mut self, dev: String, event: RunEvent) {
        match event {
            RunEvent::Started => {
                if let Some(run) = self.runs.get_mut(&dev) {
                    run.started = true;
                }
            }
            RunEvent::Progress(p) => {
                if let Some(run) = self.runs.get_mut(&dev) {
                    run.started = true;
                    run.done_pct = p.or(run.done_pct);
                }
            }
            RunEvent::Failed(why) => {
                self.runs.remove(&dev);
                self.run_errors.insert(dev, why);
            }
            RunEvent::Done(last, read) => {
                let test = self.runs.remove(&dev).map(|r| r.test);
                let name = self
                    .report_for(&dev)
                    .map(|r| r.model.clone())
                    .filter(|m| !m.is_empty())
                    .unwrap_or_else(|| dev.clone());
                if let Some(test) = test {
                    use mshell_process::desktop::{Notice, Urgency, notify};
                    let (summary, body, critical) = finished_notice(&name, test, last.as_ref());
                    notify(
                        Notice::new("Drive health", summary, body)
                            .icon("drive-harddisk-symbolic")
                            .urgency(if critical {
                                Urgency::Critical
                            } else {
                                Urgency::Normal
                            }),
                    );
                }
                // The last progress read is a full SMART read: show it.
                if let SmartRead::Report(rep) = read.as_ref() {
                    hand_to_monitor(std::slice::from_ref(read.as_ref()));
                    if let SmartState::Loaded(reads) = &mut self.smart {
                        let slot = reads
                            .iter_mut()
                            .find(|r| matches!(r, SmartRead::Report(old) if old.dev == rep.dev));
                        match slot {
                            Some(slot) => *slot = SmartRead::Report(rep.clone()),
                            None => reads.push(SmartRead::Report(rep.clone())),
                        }
                    }
                }
            }
        }
    }

    fn self_test_row(&self, list: &gtk::ListBox, r: &SmartReport, sender: &ComponentSender<Self>) {
        let run = self.runs.get(&r.dev);
        let subtitle = self_test_subtitle(r, run, self.run_errors.get(&r.dev));
        let mut trailing: Vec<gtk::Widget> = Vec::new();
        if let Some(run) = run {
            let bar = gtk::ProgressBar::builder()
                .width_request(90)
                .valign(gtk::Align::Center)
                .css_classes(["cpu-dashboard-bar", "calm"])
                .build();
            match run.done_pct {
                Some(p) => bar.set_fraction(f64::from(p) / 100.0),
                None => bar.pulse(),
            }
            trailing.push(bar.into());
        } else if r.self_test.running.is_none() {
            for (test, tip) in [
                (
                    SelfTest::Short,
                    "Start a short SMART self-test (a few minutes; asks for your password)",
                ),
                (
                    SelfTest::Long,
                    "Start a long SMART self-test: reads the whole drive (minutes on NVMe, \
                     hours on a hard disk; asks for your password)",
                ),
            ] {
                let b = gtk::Button::with_label(test_name(test));
                b.set_css_classes(&["ok-button-surface", "label-small"]);
                b.set_valign(gtk::Align::Center);
                b.set_tooltip_text(Some(tip));
                let (s, dev) = (sender.clone(), r.dev.clone());
                b.connect_clicked(move |_| {
                    s.input(DrivesTabInput::StartSelfTest(dev.clone(), test));
                });
                trailing.push(b.into());
            }
        }
        list.append(&ui::plain_row(&ui::row_body(
            "Self-test",
            &subtitle,
            &trailing,
        )));
    }

    /// The drive's trend charts from the drive-health history.
    fn history_rows(&self, list: &gtk::ListBox, id: &str, limits: &Limits) {
        let temps = self.history.series(id, |s| s.temp.map(|t| t as f64));
        if temps.len() >= 2 {
            list.append(&detail_row(&format!(
                "Temperature · {}",
                drive_chart::caption(&temps, "°C")
            )));
            let chart = drive_chart::chart(temps, Some(limits.temp_warn_c as f64));
            list.append(&ui::plain_row(&chart));
        }
        let wear = self.history.series(id, |s| s.wear.map(|w| w as f64));
        let moved = wear.first().map(|f| f.1) != wear.last().map(|l| l.1);
        if wear.len() >= 2 && moved {
            list.append(&detail_row(&format!(
                "Wear · {}",
                drive_chart::caption(&wear, "%")
            )));
            list.append(&ui::plain_row(&drive_chart::chart(
                wear,
                Some(limits.wear_warn_pct as f64),
            )));
        }
    }

    fn render(&self, sender: &ComponentSender<Self>) {
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        let limits = Limits::current();
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
        // sysfs NVMe controllers first, then any SMART-only device (SATA):
        // (dev, model, temperature, history id).
        let mut devs: Vec<(String, String, Option<i64>, String)> = self
            .drives
            .iter()
            .map(|d| {
                (
                    d.dev.clone(),
                    d.model.clone(),
                    d.temp_c.map(i64::from),
                    drive_id(&d.serial, &d.dev),
                )
            })
            .collect();
        for r in &reports {
            if let SmartRead::Report(rep) = r
                && !devs.iter().any(|(d, ..)| *d == rep.dev)
            {
                devs.push((
                    rep.dev.clone(),
                    rep.model.clone(),
                    rep.temp_c,
                    drive_id(&rep.serial, &rep.dev),
                ));
            }
        }
        if devs.is_empty() {
            let list = gtk::ListBox::new();
            list.set_css_classes(&["ok-boxed-list", "compact", "cpu-dashboard-list"]);
            list.append(&ui::placeholder_row("No NVMe drives found"));
            self.list.append(&list);
            return;
        }
        for (dev, model, sys_temp, id) in devs {
            let list = gtk::ListBox::new();
            list.set_css_classes(&["ok-boxed-list", "compact", "cpu-dashboard-list"]);
            list.set_selection_mode(gtk::SelectionMode::None);
            let rep = report_for(&dev);
            let temp = sys_temp.or(rep.and_then(|r| r.temp_c));
            let temp_label = gtk::Label::builder()
                .label(temp.map(|t| format!("{t}°C")).unwrap_or_else(|| "—".into()))
                .valign(gtk::Align::Center)
                .build();
            let class = temp.map(|t| limits.temp_class(t)).filter(|c| !c.is_empty());
            temp_label.set_css_classes(&match class {
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
                report_rows(&list, r, &limits);
                if r.self_test_supported {
                    self.self_test_row(&list, r, sender);
                }
            } else if let Some(e) = error_for(&dev) {
                list.append(&detail_row(&e));
            }
            self.history_rows(&list, &id, &limits);
            self.list.append(&list);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menus::menu_widgets::cpu_dashboard::selftest::{Running, SelfTestInfo};

    fn rep() -> SmartReport {
        SmartReport {
            dev: "/dev/nvme0".into(),
            self_test_supported: true,
            self_test: SelfTestInfo {
                running: None,
                last: Some(LastTest {
                    kind: "Short".into(),
                    result: "Completed without error".into(),
                    passed: true,
                    hours: Some(2961),
                }),
            },
            ..SmartReport::default()
        }
    }

    #[test]
    fn self_test_subtitles() {
        let r = rep();
        assert_eq!(
            self_test_subtitle(&r, None, None),
            "Last: Short · Completed without error · at 2961 h"
        );
        let err = "Authorization was cancelled".to_string();
        assert!(
            self_test_subtitle(&r, None, Some(&err)).ends_with(" · Authorization was cancelled")
        );
        let mut run = Run {
            test: SelfTest::Long,
            started: false,
            done_pct: None,
        };
        assert_eq!(
            self_test_subtitle(&r, Some(&run), None),
            "Starting the long test…"
        );
        run.started = true;
        assert_eq!(
            self_test_subtitle(&r, Some(&run), None),
            "Long test running"
        );
        run.done_pct = Some(40);
        assert_eq!(
            self_test_subtitle(&r, Some(&run), None),
            "Long test running · 40% done"
        );
        // A test the firmware runs that this tab didn't start.
        let mut busy = rep();
        busy.self_test.running = Some(Running {
            kind: "Extended".into(),
            done_pct: Some(10),
        });
        assert_eq!(
            self_test_subtitle(&busy, None, None),
            "Extended test running · 10% done"
        );
        busy.self_test = SelfTestInfo::default();
        assert_eq!(self_test_subtitle(&busy, None, None), "No self-test logged");
    }

    #[test]
    fn finished_notices() {
        let ok = rep().self_test.last;
        let (s, b, crit) = finished_notice("Disk", SelfTest::Short, ok.as_ref());
        assert_eq!(s, "Short self-test finished: Disk");
        assert_eq!(b, "Completed without error");
        assert!(!crit);
        let bad = LastTest {
            kind: "Extended".into(),
            result: "Completed: failed segments".into(),
            passed: false,
            hours: None,
        };
        assert!(finished_notice("Disk", SelfTest::Long, Some(&bad)).2);
        assert_eq!(
            finished_notice("Disk", SelfTest::Long, None).1,
            "The drive logged no result"
        );
    }
}
