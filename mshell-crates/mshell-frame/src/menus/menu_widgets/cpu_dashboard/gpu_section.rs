//! Overview-tab GPU section: Intel iGPU busy % + frequency per GT (render
//! / media) from sysfs, and the processes using it from DRM fdinfo. Built
//! in the dashboard's own row vocabulary (section label, labelled bar
//! rows like Memory, a compact boxed list for the processes). Hidden on a
//! machine without an `xe` / `i915` GPU. Polls only while the Overview is
//! visible in an open menu; the `/proc` fd sweep runs off the GTK thread.

use super::gpu::{
    ClientKey, DrmClient, DrmFd, GpuProc, GtSample, Igpu, busy_pct, find_drm_fds, find_igpu,
    gpu_procs, read_clients, sample_gt,
};
use super::keyed_rows::KeyedRows;
use super::procs::fmt_kb;
use super::ui;
use crate::menus::reveal_poll::RevealPoll;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
/// Full `/proc/*/fd` re-sweep every N polls; known fds are re-read between.
const RESCAN_EVERY: u32 = 5;
const MAX_PROCS: usize = 5;

/// One poll's raw data, gathered on a blocking thread.
#[derive(Debug, Default)]
pub(crate) struct GpuPollData {
    gts: Vec<Option<GtSample>>,
    fds: Vec<DrmFd>,
    clients: HashMap<ClientKey, DrmClient>,
    names: HashMap<i32, String>,
}

struct GtRow {
    bar: gtk::ProgressBar,
    detail: gtk::Label,
    value: gtk::Label,
}

pub(crate) struct GpuSectionModel {
    poll: RevealPoll,
    in_flight: bool,
    igpu: Option<Igpu>,
    prev_gts: Vec<Option<GtSample>>,
    prev_clients: HashMap<ClientKey, DrmClient>,
    prev_at: Option<Instant>,
    fds: Vec<DrmFd>,
    polls: u32,
    gt_rows: Vec<GtRow>,
    procs: Vec<GpuProc>,
    proc_list: gtk::ListBox,
    proc_rows: KeyedRows<i32, GpuProcRow>,
}

#[derive(Debug)]
pub(crate) enum GpuSectionInput {
    SetActive(bool),
    Poll,
}

#[relm4::component(pub(crate))]
impl Component for GpuSectionModel {
    type CommandOutput = GpuPollData;
    type Input = GpuSectionInput;
    type Output = ();
    type Init = ();

    view! {
        #[root]
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 8,
            set_visible: model.igpu.is_some(),

            gtk::Label {
                add_css_class: "cpu-dashboard-section-label",
                set_label: "GPU",
                set_halign: gtk::Align::Start,
            },
            #[name = "gt_box"]
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 6,
            },
            #[local_ref]
            proc_list -> gtk::ListBox {
                set_css_classes: &["ok-boxed-list", "compact", "cpu-dashboard-list"],
                set_selection_mode: gtk::SelectionMode::None,
                #[watch]
                set_visible: !model.procs.is_empty(),
            },
        }
    }

    fn init(_: (), root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let s = sender.clone();
        let poll = RevealPoll::new(POLL_INTERVAL, move || {
            s.input_sender().send(GpuSectionInput::Poll).is_ok()
        });
        poll.set_revealed(false);
        let proc_list = gtk::ListBox::new();
        let mut model = GpuSectionModel {
            poll,
            in_flight: false,
            igpu: find_igpu(Path::new("/sys/class/drm")),
            prev_gts: Vec::new(),
            prev_clients: HashMap::new(),
            prev_at: None,
            fds: Vec::new(),
            polls: 0,
            gt_rows: Vec::new(),
            procs: Vec::new(),
            proc_list: proc_list.clone(),
            proc_rows: KeyedRows::default(),
        };
        let widgets = view_output!();
        if let Some(gpu) = &model.igpu {
            for gt in &gpu.gts {
                let (row, gt_row) = gt_bar_row(&gt.label);
                widgets.gt_box.append(&row);
                model.gt_rows.push(gt_row);
            }
        }
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match msg {
            GpuSectionInput::SetActive(on) => {
                // No GPU → never poll.
                self.poll.set_revealed(on && self.igpu.is_some());
                if on && self.igpu.is_some() {
                    sender.input(GpuSectionInput::Poll);
                }
            }
            GpuSectionInput::Poll => {
                let Some(gpu) = self.igpu.clone() else { return };
                if self.in_flight {
                    return;
                }
                self.in_flight = true;
                let rescan = self.polls.is_multiple_of(RESCAN_EVERY);
                self.polls = self.polls.wrapping_add(1);
                let known = self.fds.clone();
                sender.oneshot_command(async move {
                    tokio::task::spawn_blocking(move || {
                        let root = Path::new("/proc");
                        let fds = if rescan { find_drm_fds(root) } else { known };
                        let (clients, names) = read_clients(root, &fds);
                        GpuPollData {
                            gts: gpu.gts.iter().map(sample_gt).collect(),
                            fds,
                            clients,
                            names,
                        }
                    })
                    .await
                    .unwrap_or_default()
                });
            }
        }
    }

    fn update_cmd(&mut self, data: GpuPollData, _s: ComponentSender<Self>, _r: &Self::Root) {
        self.in_flight = false;
        let now = Instant::now();
        let wall = self.prev_at.map(|t| now.duration_since(t));
        let wall_ms = wall.map(|d| d.as_millis() as u64).unwrap_or(0);
        for (i, row) in self.gt_rows.iter().enumerate() {
            let Some(Some(s)) = data.gts.get(i) else {
                row.value.set_label("—");
                continue;
            };
            let busy = match self.prev_gts.get(i) {
                Some(Some(p)) => busy_pct(p.idle_ms, s.idle_ms, wall_ms),
                _ => 0.0,
            };
            row.bar.set_fraction(f64::from(busy) / 100.0);
            row.bar.set_css_classes(&[
                "cpu-dashboard-bar",
                if busy >= 90.0 { "danger" } else { "calm" },
            ]);
            row.value.set_label(&format!("{busy:.0}%"));
            row.detail.set_label(&if s.max_mhz > 0 {
                format!(
                    "{:.2} / {:.2} GHz",
                    s.act_mhz as f64 / 1000.0,
                    s.max_mhz as f64 / 1000.0
                )
            } else {
                format!("{:.2} GHz", s.act_mhz as f64 / 1000.0)
            });
        }
        let wall_ns = wall.map(|d| d.as_nanos() as u64).unwrap_or(0);
        self.procs = if self.prev_at.is_some() {
            gpu_procs(&self.prev_clients, &data.clients, &data.names, wall_ns)
        } else {
            Vec::new()
        };
        self.prev_gts = data.gts;
        self.prev_clients = data.clients;
        self.fds = data.fds;
        self.prev_at = Some(now);

        let views: Vec<(i32, GpuProcView)> = self
            .procs
            .iter()
            .take(MAX_PROCS)
            .map(|p| (p.pid, gpu_proc_view(p)))
            .collect();
        self.proc_rows.sync(
            &self.proc_list,
            views,
            |_, _| gpu_proc_row(),
            |r, v| {
                ui::set_title(&r.text, &v.name, &v.subtitle);
                ui::set_text(&r.busy, &v.busy);
                ui::set_text(&r.rss, &v.rss);
            },
            |r| &r.row,
        );
    }
}

/// One iGPU process row's texts.
#[derive(Debug, Clone, PartialEq)]
struct GpuProcView {
    name: String,
    subtitle: String,
    busy: String,
    rss: String,
}

fn gpu_proc_view(p: &GpuProc) -> GpuProcView {
    GpuProcView {
        name: if p.name.is_empty() {
            p.pid.to_string()
        } else {
            p.name.clone()
        },
        subtitle: if p.engine.is_empty() {
            format!("{}", p.pid)
        } else {
            format!("{} · {}", p.pid, p.engine)
        },
        busy: format!("{:.0}%", p.busy),
        rss: fmt_kb(p.resident_kb),
    }
}

/// An iGPU process row, kept across polls and updated in place.
struct GpuProcRow {
    row: gtk::ListBoxRow,
    text: ui::LiveTitle,
    busy: gtk::Label,
    rss: gtk::Label,
}

fn gpu_proc_row() -> GpuProcRow {
    let text = ui::live_title_block();
    let busy = ui::value_label("", 5, "");
    let rss = ui::value_label("", 7, "");
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .build();
    body.append(&text.block);
    body.append(&busy);
    body.append(&rss);
    GpuProcRow {
        row: ui::plain_row(&body),
        text,
        busy,
        rss,
    }
}

/// `Render [=====    ] 0.80 / 2.25 GHz  34%` — the Memory row shape.
fn gt_bar_row(label: &str) -> (gtk::Box, GtRow) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let mut caption = label.to_string();
    if let Some(first) = caption.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    row.append(
        &gtk::Label::builder()
            .label(&caption)
            .css_classes(["cpu-dashboard-stat-caption"])
            .width_chars(6)
            .xalign(0.0)
            .build(),
    );
    let bar = gtk::ProgressBar::builder()
        .hexpand(true)
        .valign(gtk::Align::Center)
        .css_classes(["cpu-dashboard-bar", "calm"])
        .build();
    row.append(&bar);
    let detail = gtk::Label::builder()
        .css_classes(["cpu-dashboard-stat-detail"])
        .build();
    row.append(&detail);
    let value = gtk::Label::builder()
        .label("—")
        .css_classes(["cpu-dashboard-stat-value"])
        .width_chars(4)
        .xalign(1.0)
        .build();
    row.append(&value);
    (row, GtRow { bar, detail, value })
}
