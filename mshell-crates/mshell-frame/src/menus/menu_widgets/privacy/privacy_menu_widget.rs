//! Privacy panel — the `privacy` bar pill's left-click surface.
//!
//! Top: a header (shield + "Privacy" + a clear-log button). Below it a
//! live "In use now" block (one row per active sensor naming the apps),
//! then the persisted access log (newest first: icon + app + time +
//! started/stopped). Both dynamic parts refresh on a 1 s tick, gated to
//! while the panel is actually mapped (per the lazy-poll convention).

use mshell_cache::privacy_history::{PrivacyEvent, clear_history, privacy_history_store};
use reactive_graph::prelude::ReadUntracked;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{ComponentParts, ComponentSender, SimpleComponent, gtk};
use std::time::Duration;

use crate::bars::bar_widgets::privacy::monitor::{PrivacyLive, live_snapshot};

pub(crate) struct PrivacyMenuWidgetModel {
    live_box: gtk::Box,
    list: gtk::Box,
    empty: gtk::Label,
    // Cheap change-detection so we don't rebuild the lists every tick.
    last_sig: String,
}

#[derive(Debug)]
pub(crate) enum PrivacyMenuWidgetInput {
    Tick,
    Clear,
}

pub(crate) struct PrivacyMenuWidgetInit {}

#[relm4::component(pub)]
impl SimpleComponent for PrivacyMenuWidgetModel {
    type Input = PrivacyMenuWidgetInput;
    type Output = ();
    type Init = PrivacyMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 12,
            add_css_class: "privacy-panel",

            gtk::Box {
                add_css_class: "privacy-panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,

                gtk::Image {
                    add_css_class: "privacy-panel-icon",
                    set_icon_name: Some("security-high-symbolic"),
                },
                gtk::Label {
                    add_css_class: "privacy-panel-title",
                    set_label: "Privacy",
                    set_hexpand: true,
                    set_xalign: 0.0,
                },
                gtk::Button {
                    add_css_class: "privacy-clear-button",
                    add_css_class: "flat",
                    set_icon_name: "user-trash-symbolic",
                    set_tooltip_text: Some("Clear access log"),
                    connect_clicked => PrivacyMenuWidgetInput::Clear,
                },
            },

            #[name = "live_box"]
            gtk::Box {
                add_css_class: "privacy-live",
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 6,
            },

            gtk::ScrolledWindow {
                set_vexpand: true,
                set_hscrollbar_policy: gtk::PolicyType::Never,

                #[name = "list"]
                gtk::Box {
                    add_css_class: "privacy-log",
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 4,

                    #[name = "empty"]
                    gtk::Label {
                        add_css_class: "privacy-log-empty",
                        set_label: "No sensor access recorded yet.",
                    },
                },
            },
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let widgets = view_output!();

        let model = PrivacyMenuWidgetModel {
            live_box: widgets.live_box.clone(),
            list: widgets.list.clone(),
            empty: widgets.empty.clone(),
            last_sig: String::new(),
        };

        // Refresh while mapped. The detection driver recomputes every 2 s;
        // a 1 s read + signature diff here keeps the panel current without
        // rebuilding the lists when nothing changed.
        let tick_sender = sender.clone();
        let root_ref = root.clone();
        gtk::glib::timeout_add_local(Duration::from_secs(1), move || {
            // Only refresh while actually on screen (menus are built eagerly
            // per monitor but mostly hidden).
            if root_ref.is_mapped()
                && tick_sender
                    .input_sender()
                    .send(PrivacyMenuWidgetInput::Tick)
                    .is_err()
            {
                return gtk::glib::ControlFlow::Break;
            }
            gtk::glib::ControlFlow::Continue
        });

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>) {
        match message {
            PrivacyMenuWidgetInput::Clear => {
                clear_history();
                self.last_sig.clear();
                self.rebuild();
            }
            PrivacyMenuWidgetInput::Tick => self.rebuild_if_changed(),
        }
    }
}

impl PrivacyMenuWidgetModel {
    fn rebuild_if_changed(&mut self) {
        let live = live_snapshot();
        let events = privacy_history_store().read_untracked().events.clone();
        let sig = signature(&live, &events);
        if sig == self.last_sig {
            return;
        }
        self.last_sig = sig;
        self.render(&live, &events);
    }

    fn rebuild(&mut self) {
        let live = live_snapshot();
        let events = privacy_history_store().read_untracked().events.clone();
        self.last_sig = signature(&live, &events);
        self.render(&live, &events);
    }

    fn render(&self, live: &PrivacyLive, events: &[PrivacyEvent]) {
        clear_children(&self.live_box);
        for (kind, apps) in [
            ("Microphone", &live.mic_apps),
            ("Camera", &live.cam_apps),
            ("Screen", &live.scr_apps),
        ] {
            if !apps.is_empty() {
                self.live_box.append(&live_row(kind, apps));
            }
        }
        if !live.is_active() {
            let idle = gtk::Label::new(Some("No sensors in use right now."));
            idle.add_css_class("privacy-live-idle");
            idle.set_xalign(0.0);
            self.live_box.append(&idle);
        }

        clear_children(&self.list);
        if events.is_empty() {
            self.list.append(&self.empty);
        } else {
            for ev in events {
                self.list.append(&log_row(ev));
            }
        }
    }
}

// ── Row builders ─────────────────────────────────────────────────────────

fn live_row(kind: &str, apps: &[String]) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("privacy-live-row");

    let icon = gtk::Image::from_icon_name(kind_icon(kind));
    icon.add_css_class("privacy-live-icon");
    icon.add_css_class("active");
    row.append(&icon);

    let label = gtk::Label::new(Some(&format!("{kind}: {}", apps.join(", "))));
    label.add_css_class("privacy-live-label");
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    row.append(&label);

    row
}

fn log_row(ev: &PrivacyEvent) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("privacy-log-row");

    let icon = gtk::Image::from_icon_name(ev.icon_name());
    icon.add_css_class("privacy-log-icon");
    row.append(&icon);

    let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
    col.set_hexpand(true);

    let app = gtk::Label::new(Some(&ev.app));
    app.add_css_class("privacy-log-app");
    app.set_xalign(0.0);
    app.set_ellipsize(gtk::pango::EllipsizeMode::End);
    col.append(&app);

    let meta = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let time = gtk::Label::new(Some(&ev.time));
    time.add_css_class("privacy-log-time");
    meta.append(&time);
    let dot = gtk::Label::new(Some("•"));
    dot.add_css_class("privacy-log-sep");
    meta.append(&dot);
    let action = gtk::Label::new(Some(&ev.action));
    action.add_css_class("privacy-log-action");
    action.add_css_class(if ev.action == "stopped" {
        "stopped"
    } else {
        "started"
    });
    meta.append(&action);
    col.append(&meta);

    row.append(&col);
    row
}

fn kind_icon(kind: &str) -> &'static str {
    match kind {
        "Camera" => "camera-video-symbolic",
        "Screen" => "video-display-symbolic",
        _ => "microphone-sensitivity-high-symbolic",
    }
}

/// A cheap fingerprint of the rendered state so the tick only rebuilds on
/// an actual change.
fn signature(live: &PrivacyLive, events: &[PrivacyEvent]) -> String {
    format!(
        "{}|{}|{}|{}",
        live.mic_apps.join(","),
        live.cam_apps.join(","),
        live.scr_apps.join(","),
        events.len(),
    )
}

fn clear_children(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}
