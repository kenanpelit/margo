//! mshelldash Screen Time tab — a port of the noctalia v5
//! `screen_time` control-center tab.
//!
//! Reads aggregated per-app focus time from the always-on
//! `screen_time_service()` and renders today's total plus a ranked
//! list of applications with proportional usage bars. Refresh is lazy:
//! the snapshot is folded once at build and again each time the dash
//! switches to this tab (the service keeps the live session ticking),
//! so nothing polls while the dash is closed. DESIGN.md compliant —
//! reuses the shared `mshelldash-card` / `-bar` / `-stat-*` vocabulary.

use mshell_services::screen_time_service;
use relm4::gtk::prelude::*;
use relm4::{ComponentParts, ComponentSender, SimpleComponent, gtk};

/// How many apps to list, at most.
const MAX_APPS: usize = 10;

pub(crate) struct ScreenTimeModel {
    total_text: String,
    /// The dynamic app-row container, rebuilt on each refresh.
    list: gtk::Box,
}

impl std::fmt::Debug for ScreenTimeModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScreenTimeModel")
            .field("total_text", &self.total_text)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum ScreenTimeInput {
    /// Re-read the snapshot and rebuild the list.
    Refresh,
}

pub(crate) struct ScreenTimeInit {}

#[relm4::component(pub(crate))]
impl SimpleComponent for ScreenTimeModel {
    type Init = ScreenTimeInit;
    type Input = ScreenTimeInput;
    type Output = ();

    view! {
        #[root]
        gtk::Box {
            add_css_class: "mshelldash-screentime",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 12,
            set_hexpand: true,

            // ── Today hero ─────────────────────────────────────────
            gtk::Box {
                add_css_class: "mshelldash-hero",
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 2,
                set_hexpand: true,

                gtk::Label {
                    add_css_class: "mshelldash-hero-time",
                    set_halign: gtk::Align::Start,
                    #[watch]
                    set_label: &model.total_text,
                },
                gtk::Label {
                    add_css_class: "mshelldash-hero-date",
                    set_halign: gtk::Align::Start,
                    set_label: "Screen time today",
                },
            },

            // ── App breakdown ──────────────────────────────────────
            gtk::Box {
                add_css_class: "mshelldash-card",
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 10,
                set_hexpand: true,

                gtk::Label {
                    add_css_class: "mshelldash-section-label",
                    set_halign: gtk::Align::Start,
                    set_label: "APPLICATIONS",
                },

                #[local_ref]
                list -> gtk::Box {
                    add_css_class: "screentime-list",
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 10,
                    set_hexpand: true,
                },
            },
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = ScreenTimeModel {
            total_text: fmt_duration(0),
            list: gtk::Box::new(gtk::Orientation::Vertical, 10),
        };
        let list = &model.list;
        let widgets = view_output!();

        sender.input(ScreenTimeInput::Refresh);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>) {
        match message {
            ScreenTimeInput::Refresh => {
                let snap = screen_time_service().snapshot(1);
                self.total_text = fmt_duration(snap.total_secs);

                // Clear existing rows.
                while let Some(child) = self.list.first_child() {
                    self.list.remove(&child);
                }

                if snap.apps.is_empty() {
                    let empty = gtk::Label::new(Some("No activity tracked yet."));
                    empty.add_css_class("mshelldash-stat-detail");
                    empty.set_halign(gtk::Align::Start);
                    self.list.append(&empty);
                    return;
                }

                let max = snap.apps.first().map(|a| a.secs).unwrap_or(1).max(1);
                for app in snap.apps.iter().take(MAX_APPS) {
                    self.list.append(&app_row(&app.display, app.secs, max));
                }
            }
        }
    }
}

/// Build one app row: `[name … duration]` over a proportional bar.
fn app_row(display: &str, secs: u64, max: u64) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
    row.add_css_class("screentime-row");
    row.set_hexpand(true);

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);

    let icon = gtk::Image::from_icon_name(&display.to_lowercase());
    icon.add_css_class("screentime-icon");
    header.append(&icon);

    let name = gtk::Label::new(Some(display));
    name.add_css_class("mshelldash-stat-caption");
    name.set_halign(gtk::Align::Start);
    name.set_hexpand(true);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    header.append(&name);

    let dur = gtk::Label::new(Some(&fmt_duration(secs)));
    dur.add_css_class("mshelldash-stat-value");
    dur.set_halign(gtk::Align::End);
    header.append(&dur);

    row.append(&header);

    let bar = gtk::ProgressBar::new();
    bar.add_css_class("mshelldash-bar");
    bar.set_fraction(secs as f64 / max as f64);
    row.append(&bar);

    row
}

/// Format a duration in seconds as `Xh Ym`, `Ym`, or `Ys`.
fn fmt_duration(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        format!("{m}m")
    } else {
        format!("{s}s")
    }
}
