//! Catwalk — a CPU-reactive animated cat bar pill (native port of the
//! noctalia `catwalk` plugin).
//!
//! Below `bars.widgets.catwalk.minimum_threshold` CPU busy% the cat idles;
//! above it the cat walks, the frame rate scaling up with load. Click opens
//! the CPU dashboard. The eight frames ship bundled in the crate and are
//! written to a cache dir at first run so `GtkImage` can load them scalably.

use crate::bars::bar_widgets::sysstat::read_cpu_stat_pub;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, CatwalkConfigStoreFields, ConfigStoreFields,
};
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::PathBuf;

const IDLE_FRAMES: [(&str, &[u8]); 4] = [
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
const ACTIVE_FRAMES: [(&str, &[u8]); 4] = [
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

/// Base animation tick — frames advance every N of these.
const TICK: std::time::Duration = std::time::Duration::from_millis(110);

pub(crate) struct CatwalkModel {
    idle_paths: Vec<PathBuf>,
    active_paths: Vec<PathBuf>,
    frame: usize,
    tick: u32,
    cpu_percent: u32,
    prev_total: u64,
    prev_idle: u64,
    /// Last (frame, active) shown — only re-set the image when it changes.
    last_key: Option<(usize, bool)>,
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
                set_tooltip_text: Some("Catwalk — CPU activity"),
                connect_clicked[sender] => move |_| {
                    sender.input(CatwalkInput::Clicked);
                },

                #[name = "img"]
                gtk::Image {
                    set_pixel_size: 22,
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let (idle_paths, active_paths) = ensure_frames();

        let hide_background = config_manager()
            .config()
            .bars()
            .widgets()
            .catwalk()
            .hide_background()
            .get_untracked();

        let (prev_total, prev_idle) = read_cpu_stat_pub();

        let sender_clone = sender.clone();
        glib::timeout_add_local(TICK, move || {
            if sender_clone
                .input_sender()
                .send(CatwalkInput::Tick)
                .is_err()
            {
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });

        let model = CatwalkModel {
            idle_paths,
            active_paths,
            frame: 0,
            tick: 0,
            cpu_percent: 0,
            prev_total,
            prev_idle,
            last_key: None,
        };

        let widgets = view_output!();

        // hide_background drops the surface fill but keeps `.ok-bar-widget`,
        // so the cat floats yet still gets the standard hover wash.
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
                self.tick = self.tick.wrapping_add(1);

                // Re-sample CPU roughly once a second.
                if self.tick.is_multiple_of(9) {
                    let (total, idle) = read_cpu_stat_pub();
                    let dt = total.saturating_sub(self.prev_total);
                    let di = idle.saturating_sub(self.prev_idle);
                    if dt > 0 {
                        self.cpu_percent = (dt.saturating_sub(di) * 100 / dt) as u32;
                    }
                    self.prev_total = total;
                    self.prev_idle = idle;
                }

                let threshold = config_manager()
                    .config()
                    .bars()
                    .widgets()
                    .catwalk()
                    .minimum_threshold()
                    .get_untracked();
                let active = self.cpu_percent >= threshold;

                // Frame cadence: idle plods; active speeds up with load
                // (≈5 ticks/frame at the threshold → 1 tick/frame at 100%).
                let ticks_per_frame: u32 = if active {
                    let span = 100u32.saturating_sub(threshold).max(1);
                    let over = self.cpu_percent.saturating_sub(threshold).min(span);
                    5u32.saturating_sub(over * 4 / span).max(1)
                } else {
                    6
                };

                if self.tick.is_multiple_of(ticks_per_frame) {
                    self.frame = (self.frame + 1) % 4;
                }

                let key = (self.frame, active);
                if self.last_key != Some(key) {
                    self.last_key = Some(key);
                    let paths = if active {
                        &self.active_paths
                    } else {
                        &self.idle_paths
                    };
                    if let Some(p) = paths.get(self.frame) {
                        widgets.img.set_from_file(Some(p));
                    }
                }
            }
        }

        self.update_view(widgets, sender);
    }
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
fn ensure_frames() -> (Vec<PathBuf>, Vec<PathBuf>) {
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
    (write(&IDLE_FRAMES), write(&ACTIVE_FRAMES))
}
