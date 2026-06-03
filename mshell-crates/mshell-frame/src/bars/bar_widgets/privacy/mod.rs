//! Privacy indicator — bar pill that lights up whenever an app is using
//! the microphone, a camera, or screen-sharing. Port of the noctalia
//! `privacy-indicator` plugin.
//!
//! All detection lives in the shared [`monitor`] singleton (so it runs once
//! regardless of monitor count); this pill is a thin reader: a 1 s tick
//! pulls the live snapshot, paints one glyph per active sensor in the
//! configured accent, names the apps in the tooltip, and left-click opens
//! the Privacy panel (live state + clearable access log).

pub(crate) mod monitor;

use monitor::PrivacyLive;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, PrivacyAccent,
    PrivacyWidgetConfigStoreFields,
};
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, GestureSingleExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

pub(crate) struct PrivacyModel {
    live: PrivacyLive,
    accent_class: &'static str,
    hide_inactive: bool,
    orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum PrivacyInput {
    Tick,
    Clicked,
}

#[derive(Debug)]
pub(crate) enum PrivacyOutput {
    Clicked,
}

pub(crate) struct PrivacyInit {
    pub(crate) orientation: Orientation,
}

#[relm4::component(pub)]
impl Component for PrivacyModel {
    type CommandOutput = ();
    type Input = PrivacyInput;
    type Output = PrivacyOutput;
    type Init = PrivacyInit;

    view! {
        #[root]
        gtk::Box {
            #[watch]
            set_css_classes: &model.root_classes(),
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,
            set_has_tooltip: true,
            #[watch]
            set_tooltip_text: Some(&tooltip(&model.live)),
            #[watch]
            set_visible: model.pill_visible(),

            gtk::Box {
                set_css_classes: &["ok-button-flat", "ok-bar-widget"],
                #[watch]
                set_orientation: model.orientation,
                set_spacing: 4,
                set_halign: gtk::Align::Center,
                set_valign: gtk::Align::Center,

                gtk::Image {
                    set_icon_name: Some("microphone-sensitivity-high-symbolic"),
                    #[watch]
                    set_css_classes: &icon_classes(!model.live.mic_apps.is_empty()),
                    #[watch]
                    set_visible: model.icon_visible(!model.live.mic_apps.is_empty()),
                },
                gtk::Image {
                    set_icon_name: Some("camera-video-symbolic"),
                    #[watch]
                    set_css_classes: &icon_classes(!model.live.cam_apps.is_empty()),
                    #[watch]
                    set_visible: model.icon_visible(!model.live.cam_apps.is_empty()),
                },
                gtk::Image {
                    set_icon_name: Some("video-display-symbolic"),
                    #[watch]
                    set_css_classes: &icon_classes(!model.live.scr_apps.is_empty()),
                    #[watch]
                    set_visible: model.icon_visible(!model.live.scr_apps.is_empty()),
                },
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Kick off the shared detection driver (idempotent).
        monitor::ensure_started();

        let model = PrivacyModel {
            live: monitor::live_snapshot(),
            accent_class: read_accent().css_class(),
            hide_inactive: read_hide_inactive(),
            orientation: params.orientation,
        };

        let widgets = view_output!();

        // Left-click → open the Privacy panel.
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_PRIMARY);
        let click_sender = sender.clone();
        gesture.connect_released(move |_, _, _, _| {
            click_sender.input(PrivacyInput::Clicked);
        });
        root.add_controller(gesture);

        // Pull the shared snapshot once a second (the driver itself only
        // recomputes every 2 s; this is a cheap in-memory read + diff).
        let tick_sender = sender.clone();
        gtk::glib::timeout_add_local(Duration::from_secs(1), move || {
            if tick_sender.input_sender().send(PrivacyInput::Tick).is_err() {
                return gtk::glib::ControlFlow::Break;
            }
            gtk::glib::ControlFlow::Continue
        });

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            PrivacyInput::Tick => {
                self.live = monitor::live_snapshot();
                self.accent_class = read_accent().css_class();
                self.hide_inactive = read_hide_inactive();
            }
            PrivacyInput::Clicked => {
                let _ = sender.output(PrivacyOutput::Clicked);
            }
        }
    }
}

impl PrivacyModel {
    /// Whole pill visible? Always, unless `hide_inactive` and nothing is in
    /// use right now.
    fn pill_visible(&self) -> bool {
        !self.hide_inactive || self.live.is_active()
    }

    /// A single sensor glyph: shown when active, or always when we're not
    /// hiding the inactive state (then it reads as a dimmed watchdog icon).
    fn icon_visible(&self, active: bool) -> bool {
        active || !self.hide_inactive
    }

    fn root_classes(&self) -> Vec<&'static str> {
        let mut classes = vec![
            "ok-button-surface",
            "ok-bar-widget",
            "privacy-bar-widget",
            self.accent_class,
        ];
        if self.live.is_active() {
            classes.push("active");
        }
        classes
    }
}

// ── View helpers ─────────────────────────────────────────────────────────

fn icon_classes(active: bool) -> Vec<&'static str> {
    if active {
        vec!["privacy-icon", "active"]
    } else {
        vec!["privacy-icon"]
    }
}

fn tooltip(live: &PrivacyLive) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !live.mic_apps.is_empty() {
        parts.push(format!("Microphone: {}", live.mic_apps.join(", ")));
    }
    if !live.cam_apps.is_empty() {
        parts.push(format!("Camera: {}", live.cam_apps.join(", ")));
    }
    if !live.scr_apps.is_empty() {
        parts.push(format!("Screen sharing: {}", live.scr_apps.join(", ")));
    }
    if parts.is_empty() {
        "Privacy: no sensors in use".to_string()
    } else {
        parts.join("\n")
    }
}

fn read_accent() -> PrivacyAccent {
    config_manager()
        .config()
        .bars()
        .widgets()
        .privacy()
        .accent()
        .get_untracked()
}

fn read_hide_inactive() -> bool {
    config_manager()
        .config()
        .bars()
        .widgets()
        .privacy()
        .hide_inactive()
        .get_untracked()
}
