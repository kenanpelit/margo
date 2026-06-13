//! Catwalk — a CPU-reactive animated cat bar pill.
//!
//! Below `bars.widgets.catwalk.minimum_threshold` CPU busy% the cat idles;
//! above it the cat walks, the frame rate scaling up with load. Click opens
//! the CPU dashboard.
//!
//! Two sprite sets ship bundled (selected by `bars.widgets.catwalk.style`):
//! the original **noctalia** cat (4 walk + 4 idle frames) and **RunCat** —
//! the classic macOS running cat (5 run frames + a sleeping idle pose), via
//! CatWalk by Driglu4it, originally RunCat by Kyome. Frames are written to a
//! cache dir at first run so `GtkImage` can load them scalably.
//!
//! The walk cadence uses the RunCat easing — `5000/√(cpu+35) − 400` ms per
//! frame, floored at 30 ms — so the cat eases smoothly from a saunter to a
//! full zoom. An optional CPU-% readout (`display`) sits beside the cat,
//! severity-coloured (calm → warn → danger), and the tooltip narrates the
//! cat's mood ("Cat is ZOOMING!! · 94%").

use crate::bars::bar_widgets::sysstat::read_cpu_stat_pub;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, CatStyle, CatwalkConfig, CatwalkConfigStoreFields,
    CatwalkDisplay, ConfigStoreFields,
};
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const NOCTALIA_IDLE: [(&str, &[u8]); 4] = [
    (
        "idle-0.svg",
        include_bytes!("catwalk_icons/my-idle-0-symbolic.svg"),
    ),
    (
        "idle-1.svg",
        include_bytes!("catwalk_icons/my-idle-1-symbolic.svg"),
    ),
    (
        "idle-2.svg",
        include_bytes!("catwalk_icons/my-idle-2-symbolic.svg"),
    ),
    (
        "idle-3.svg",
        include_bytes!("catwalk_icons/my-idle-3-symbolic.svg"),
    ),
];
const NOCTALIA_ACTIVE: [(&str, &[u8]); 4] = [
    (
        "active-0.svg",
        include_bytes!("catwalk_icons/my-active-0-symbolic.svg"),
    ),
    (
        "active-1.svg",
        include_bytes!("catwalk_icons/my-active-1-symbolic.svg"),
    ),
    (
        "active-2.svg",
        include_bytes!("catwalk_icons/my-active-2-symbolic.svg"),
    ),
    (
        "active-3.svg",
        include_bytes!("catwalk_icons/my-active-3-symbolic.svg"),
    ),
];
const RUNCAT_ACTIVE: [(&str, &[u8]); 5] = [
    (
        "runcat-active-0.svg",
        include_bytes!("catwalk_icons/runcat-active-0.svg"),
    ),
    (
        "runcat-active-1.svg",
        include_bytes!("catwalk_icons/runcat-active-1.svg"),
    ),
    (
        "runcat-active-2.svg",
        include_bytes!("catwalk_icons/runcat-active-2.svg"),
    ),
    (
        "runcat-active-3.svg",
        include_bytes!("catwalk_icons/runcat-active-3.svg"),
    ),
    (
        "runcat-active-4.svg",
        include_bytes!("catwalk_icons/runcat-active-4.svg"),
    ),
];
const RUNCAT_IDLE: [(&str, &[u8]); 1] = [(
    "runcat-idle.svg",
    include_bytes!("catwalk_icons/runcat-idle.svg"),
)];

/// Bundled sprite frames, unpacked to the cache dir once at first run.
struct Frames {
    noctalia_idle: Vec<PathBuf>,
    noctalia_active: Vec<PathBuf>,
    runcat_idle: Vec<PathBuf>,
    runcat_active: Vec<PathBuf>,
}

pub(crate) struct CatwalkModel {
    frames: Frames,
    frame: usize,
    cpu_percent: u32,
    prev_total: u64,
    prev_idle: u64,
    last_sample: Instant,
    /// Last (style, active, frame) painted — only re-set the image on change.
    last_key: Option<(CatStyle, bool, usize)>,
    /// Last CPU% rendered into the label/tooltip — guards needless updates.
    last_pct: Option<u32>,
    /// Last display mode applied (visibility) — guards needless updates.
    last_display: Option<CatwalkDisplay>,
    /// Last sprite size applied — guards needless `set_pixel_size`.
    last_size: Option<u32>,
}

#[derive(Debug)]
pub(crate) enum CatwalkInput {
    Tick,
    Clicked,
}

#[derive(Debug)]
pub(crate) enum CatwalkOutput {
    Clicked,
}

pub(crate) struct CatwalkInit {}

#[relm4::component(pub(crate))]
impl Component for CatwalkModel {
    type CommandOutput = ();
    type Input = CatwalkInput;
    type Output = CatwalkOutput;
    type Init = CatwalkInit;

    view! {
        // Canonical bar-pill anatomy (DESIGN.md §4): outer Box + an inner
        // button carrying `.ok-button-surface .ok-bar-widget`, which is where
        // the standard transparent surface + 14%-primary hover wash come from.
        #[root]
        gtk::Box {
            add_css_class: "catwalk-bar-widget",
            set_hexpand: false,
            set_vexpand: false,

            #[name = "btn"]
            gtk::Button {
                set_css_classes: &["ok-button-surface", "ok-bar-widget"],

                #[name = "content"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 4,
                    set_valign: gtk::Align::Center,

                    #[name = "img"]
                    gtk::Image {
                        set_pixel_size: 22,
                    },
                    #[name = "lbl"]
                    gtk::Label {
                        add_css_class: "catwalk-cpu-label",
                        set_visible: false,
                    },
                },

                connect_clicked[sender] => move |_| {
                    sender.input(CatwalkInput::Clicked);
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let frames = ensure_frames();

        let hide_background = config_manager()
            .config()
            .bars()
            .widgets()
            .catwalk()
            .hide_background()
            .get_untracked();

        let (prev_total, prev_idle) = read_cpu_stat_pub();

        // Self-rearming tick: the cadence varies with load (down to the 30 ms
        // RunCat floor), so we schedule each next frame from the handler rather
        // than running a fixed-interval source.
        arm_tick(&sender, 120);

        let model = CatwalkModel {
            frames,
            frame: 0,
            cpu_percent: 0,
            prev_total,
            prev_idle,
            last_sample: Instant::now(),
            last_key: None,
            last_pct: None,
            last_display: None,
            last_size: None,
        };

        let widgets = view_output!();

        // hide_background drops the surface fill but keeps `.ok-bar-widget`,
        // so the cat floats yet still gets the standard hover wash. (This one
        // applies on the next widget rebuild, matching the other pills.)
        if hide_background {
            widgets.btn.set_css_classes(&["ok-bar-widget"]);
        }

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            CatwalkInput::Clicked => {
                let _ = sender.output(CatwalkOutput::Clicked);
                return;
            }
            CatwalkInput::Tick => {
                let cfg: CatwalkConfig = config_manager()
                    .config()
                    .bars()
                    .widgets()
                    .catwalk()
                    .get_untracked();

                // Re-sample CPU at the configured cadence (independent of the
                // animation tick, which can fire ~33×/s under full load).
                let poll = Duration::from_secs(cfg.poll_secs.clamp(1, 10) as u64);
                if self.last_sample.elapsed() >= poll {
                    let (total, idle) = read_cpu_stat_pub();
                    let dt = total.saturating_sub(self.prev_total);
                    let di = idle.saturating_sub(self.prev_idle);
                    if dt > 0 {
                        self.cpu_percent = (dt.saturating_sub(di) * 100 / dt) as u32;
                    }
                    self.prev_total = total;
                    self.prev_idle = idle;
                    self.last_sample = Instant::now();
                }

                let active = self.cpu_percent >= cfg.minimum_threshold;

                // Advance the frame and paint it (only when the picture changes).
                let paths = self.frames.set_for(cfg.style, active);
                let len = paths.len().max(1);
                self.frame = (self.frame + 1) % len;
                let key = (cfg.style, active, self.frame);
                if self.last_key != Some(key) {
                    self.last_key = Some(key);
                    if let Some(p) = paths.get(self.frame % len) {
                        widgets.img.set_from_file(Some(p));
                    }
                }

                // Sprite size (guarded).
                let size = cfg.size.clamp(12, 48);
                if self.last_size != Some(size) {
                    self.last_size = Some(size);
                    widgets.img.set_pixel_size(size as i32);
                }

                // CPU-% label text + severity (only when the value changes).
                if self.last_pct != Some(self.cpu_percent) {
                    self.last_pct = Some(self.cpu_percent);
                    let pct = self.cpu_percent;
                    widgets.lbl.set_label(&format!("{pct}%"));
                    let classes: &[&str] = if pct >= 80 {
                        &["catwalk-cpu-label", "danger"]
                    } else if pct >= 50 {
                        &["catwalk-cpu-label", "warn"]
                    } else {
                        &["catwalk-cpu-label"]
                    };
                    widgets.lbl.set_css_classes(classes);
                    widgets.btn.set_tooltip_text(Some(&format!(
                        "{} · {pct}%",
                        mood(pct, cfg.minimum_threshold)
                    )));
                }

                // Display mode → which of {cat, %} are visible (guarded).
                if self.last_display != Some(cfg.display) {
                    self.last_display = Some(cfg.display);
                    let (show_img, show_lbl) = match cfg.display {
                        CatwalkDisplay::Icon => (true, false),
                        CatwalkDisplay::Text => (false, true),
                        CatwalkDisplay::Both => (true, true),
                    };
                    widgets.img.set_visible(show_img);
                    widgets.lbl.set_visible(show_lbl);
                }

                // Schedule the next frame: RunCat easing while active (smooth
                // saunter → zoom), a slow plod while idle.
                let next_ms = if active {
                    let cpu = self.cpu_percent as f64;
                    (5000.0 / (cpu + 35.0).sqrt() - 400.0).ceil().max(30.0) as u64
                } else {
                    280
                };
                arm_tick(&sender, next_ms);
            }
        }

        self.update_view(widgets, sender);
    }
}

impl Frames {
    fn set_for(&self, style: CatStyle, active: bool) -> &[PathBuf] {
        match (style, active) {
            (CatStyle::RunCat, true) => &self.runcat_active,
            (CatStyle::RunCat, false) => &self.runcat_idle,
            (CatStyle::Noctalia, true) => &self.noctalia_active,
            (CatStyle::Noctalia, false) => &self.noctalia_idle,
        }
    }
}

/// CPU-load mood line for the tooltip — bands lifted from the DMS CatWidget.
fn mood(cpu: u32, threshold: u32) -> &'static str {
    if cpu < threshold {
        "Cat is sleeping…"
    } else if cpu < 15 {
        "Cat is strolling"
    } else if cpu < 40 {
        "Cat is walking"
    } else if cpu < 70 {
        "Cat is trotting"
    } else if cpu < 90 {
        "Cat is running!"
    } else {
        "Cat is ZOOMING!!"
    }
}

/// Schedule a single `Tick` after `ms`. The chain re-arms itself from the
/// handler; if the component is gone the send simply fails and the chain ends.
fn arm_tick(sender: &ComponentSender<CatwalkModel>, ms: u64) {
    let sender = sender.clone();
    glib::timeout_add_local_once(Duration::from_millis(ms), move || {
        let _ = sender.input_sender().send(CatwalkInput::Tick);
    });
}

/// Cache dir for the unpacked frames: `$XDG_CACHE_HOME/mshell/catwalk`.
fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join(".cache")
        })
        .join("mshell")
        .join("catwalk")
}

/// Write the bundled frames to the cache dir (once) and return their paths.
fn ensure_frames() -> Frames {
    let dir = cache_dir();
    let _ = std::fs::create_dir_all(&dir);
    let write = |frames: &[(&str, &[u8])]| -> Vec<PathBuf> {
        frames
            .iter()
            .map(|(name, bytes)| {
                let path = dir.join(name);
                if !path.exists() {
                    let _ = std::fs::write(&path, bytes);
                }
                path
            })
            .collect()
    };
    Frames {
        noctalia_idle: write(&NOCTALIA_IDLE),
        noctalia_active: write(&NOCTALIA_ACTIVE),
        runcat_idle: write(&RUNCAT_IDLE),
        runcat_active: write(&RUNCAT_ACTIVE),
    }
}
