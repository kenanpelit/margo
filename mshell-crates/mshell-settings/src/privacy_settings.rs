//! Settings → Privacy page.
//!
//! Sections:
//!   * **Location Services** — geoclue toggle (pkexec mask/unmask).
//!     Insensitive + note when geoclue is not installed.
//!   * **Active Sensors** — read-only mic + camera indicators.
//!     Mic: subscribes `audio_service().recording_streams` via `watch!`.
//!     Camera: 3 s `fuser /dev/video*` poll while the page is visible.
//!   * **Screen Lock** — one-line summary of idle lock state + deep
//!     link to Settings → Widgets → Lock.
//!
//! Attachment points for future tasks:
//!   // File History — Task 5
//!   // App Permissions — Task 6

use mshell_common::watch;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, IdleStoreFields};
use mshell_launcher::notify;
use mshell_services::audio_service;
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

use crate::sys;

const CAMERA_POLL_INTERVAL: Duration = Duration::from_secs(3);

// ── Model ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) struct PrivacySettingsModel {
    // Location Services (geoclue)
    location_installed: bool,
    location_enabled: bool,
    // Active sensors
    mic_apps: Vec<String>,
    camera_in_use: bool,
    // Screen lock summary
    lock_enabled: bool,
    lock_timeout: u32,
}

#[derive(Debug)]
pub(crate) enum PrivacySettingsInput {
    /// Async result from `sys::geoclue::status()` on init.
    GeoclueStatus(bool, bool),
    /// User toggled the Location Services switch.
    SetLocation(bool),
    // Mic watch result (driven via command channel).
    // Camera poll result (driven via command channel).
}

#[derive(Debug)]
pub(crate) enum PrivacySettingsOutput {}

pub(crate) struct PrivacySettingsInit {}

#[derive(Debug)]
pub(crate) enum PrivacySettingsCommandOutput {
    MicChanged(Vec<String>),
    CameraChanged(bool),
    /// Camera poll loop exited (shutdown).
    #[allow(dead_code)]
    CameraLoopDone,
}

// ── Component ─────────────────────────────────────────────────────────────────

#[relm4::component(pub)]
impl Component for PrivacySettingsModel {
    type CommandOutput = PrivacySettingsCommandOutput;
    type Input = PrivacySettingsInput;
    type Output = PrivacySettingsOutput;
    type Init = PrivacySettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_propagate_natural_height: false,
            set_propagate_natural_width: false,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                // ── Hero ──────────────────────────────────────────
                gtk::Box {
                    add_css_class: "settings-hero",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Start,
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("security-high-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Privacy",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Location services, microphone and camera usage, and screen lock.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                // ── Location Services ─────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Location Services",
                    set_halign: gtk::Align::Start,
                },

                // geoclue not-installed banner
                gtk::Box {
                    add_css_class: "privacy-location-missing",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    set_halign: gtk::Align::Start,
                    #[watch]
                    set_visible: !model.location_installed,

                    gtk::Image {
                        set_icon_name: Some("dialog-information-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "geoclue is not installed — location services unavailable.",
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                        set_wrap: true,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    #[watch]
                    set_visible: model.location_installed,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Enabled",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Controls the system geoclue location provider.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(location_toggle_handler)]
                        set_active: model.location_enabled,
                        #[watch]
                        set_sensitive: model.location_installed,
                        connect_state_set[sender] => move |_, on| {
                            sender.input(PrivacySettingsInput::SetLocation(on));
                            glib::Propagation::Proceed
                        } @location_toggle_handler,
                    },
                },

                // ── Active Sensors ────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Active Sensors",
                    set_halign: gtk::Align::Start,
                },

                // Microphone row
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,

                    gtk::Image {
                        set_icon_name: Some("microphone-sensitivity-high-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        set_hexpand: true,

                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_label: "Microphone",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                            #[watch]
                            set_label: &mic_status_label(&model.mic_apps),
                        },
                    },
                },

                // Camera row
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,

                    gtk::Image {
                        set_icon_name: Some("camera-video-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        set_hexpand: true,

                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_label: "Camera",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                            #[watch]
                            set_label: if model.camera_in_use { "Camera: in use" } else { "Camera: not in use" },
                        },
                    },
                },

                // ── Screen Lock ───────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Screen Lock",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        set_valign: gtk::Align::Center,

                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Automatic lock",
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                            #[watch]
                            set_label: &lock_summary(model.lock_enabled, model.lock_timeout),
                        },
                    },

                    gtk::Button {
                        add_css_class: "ok-button-primary",
                        set_label: "Open Lock settings",
                        set_valign: gtk::Align::Center,
                        connect_clicked[sender] => move |_| {
                            let _ = sender.clone();
                            crate::open_settings_at_section("widgets/lock");
                        },
                    },
                },

                // File History — Task 5
                // App Permissions — Task 6
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // ── Geoclue status (async, fires GeoclueStatus) ────────────
        {
            let s = sender.clone();
            glib::spawn_future_local(async move {
                let (installed, enabled) = sys::geoclue::status().await;
                s.input(PrivacySettingsInput::GeoclueStatus(installed, enabled));
            });
        }

        // ── Mic watcher ─────────────────────────────────────────────
        // Copy the bar widget's `watch!` pattern exactly.
        let recording_streams = audio_service().recording_streams.clone();
        watch!(sender, [recording_streams.watch()], |out| {
            let snapshot: Vec<String> = recording_streams
                .get()
                .iter()
                .map(|s| {
                    s.application_name
                        .get()
                        .unwrap_or_else(|| s.name.get())
                })
                .collect();
            let _ = out.send(PrivacySettingsCommandOutput::MicChanged(snapshot));
        });

        // Prime mic state synchronously (same as bar widget).
        let initial_mic: Vec<String> = audio_service()
            .recording_streams
            .get()
            .iter()
            .map(|s| {
                s.application_name
                    .get()
                    .unwrap_or_else(|| s.name.get())
            })
            .collect();

        // ── Camera poll (start on map, stop on unmap) ────────────────
        // We use a glib SourceId approach: start a periodic idle that polls
        // `fuser` every 3 s while the page is visible. On unmap we stop the
        // poll so we don't waste cycles when the page is hidden.
        // The `sender.command` approach (as in the bar widget) runs
        // forever while the component lives; for a settings page we gate
        // on visibility instead.
        {
            use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
            let active = Arc::new(AtomicBool::new(false));
            let active_map = active.clone();
            let active_unmap = active.clone();
            let sender_poll = sender.clone();

            // On map: mark active, kick off the poll loop
            root.connect_map(move |_| {
                active_map.store(true, Ordering::Relaxed);
                let active_loop = active_map.clone();
                let sender_loop = sender_poll.clone();
                glib::spawn_future_local(async move {
                    loop {
                        if !active_loop.load(Ordering::Relaxed) {
                            break;
                        }
                        let in_use = camera_in_use().await;
                        let _ = sender_loop
                            .command_sender()
                            .send(PrivacySettingsCommandOutput::CameraChanged(in_use));
                        // Sleep for the poll interval, but check active periodically
                        let mut elapsed = 0u64;
                        while elapsed < CAMERA_POLL_INTERVAL.as_millis() as u64 {
                            glib::timeout_future(Duration::from_millis(200)).await;
                            elapsed += 200;
                            if !active_loop.load(Ordering::Relaxed) {
                                return;
                            }
                        }
                    }
                });
            });

            // On unmap: mark inactive (the loop above will exit on next iteration)
            root.connect_unmap(move |_| {
                active_unmap.store(false, Ordering::Relaxed);
            });
        }

        // Read lock state from config
        let lock_enabled = config_manager()
            .config()
            .idle()
            .lock_enabled()
            .get_untracked();
        let lock_timeout = config_manager()
            .config()
            .idle()
            .lock_timeout_minutes()
            .get_untracked();

        let model = PrivacySettingsModel {
            location_installed: false,
            location_enabled: false,
            mic_apps: initial_mic,
            camera_in_use: false,
            lock_enabled,
            lock_timeout,
        };

        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            PrivacySettingsCommandOutput::MicChanged(apps) => {
                self.mic_apps = apps;
            }
            PrivacySettingsCommandOutput::CameraChanged(in_use) => {
                self.camera_in_use = in_use;
            }
            PrivacySettingsCommandOutput::CameraLoopDone => {}
        }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            PrivacySettingsInput::GeoclueStatus(installed, enabled) => {
                self.location_installed = installed;
                self.location_enabled = enabled;
            }
            PrivacySettingsInput::SetLocation(on) => {
                let s = sender.clone();
                glib::spawn_future_local(async move {
                    if let Err(e) = sys::geoclue::set_enabled(on).await {
                        notify::toast("Privacy", &e);
                    }
                    // Re-query to reflect actual state (in case pkexec was denied)
                    let (installed, enabled) = sys::geoclue::status().await;
                    s.input(PrivacySettingsInput::GeoclueStatus(installed, enabled));
                });
            }
        }
    }
}

// ── View helpers ──────────────────────────────────────────────────────────────

fn mic_status_label(apps: &[String]) -> String {
    if apps.is_empty() {
        "Microphone: not in use".to_string()
    } else if apps.len() == 1 {
        format!("Microphone: in use by {}", apps[0])
    } else {
        format!(
            "Microphone: in use by {} apps ({})",
            apps.len(),
            apps.join(", ")
        )
    }
}

fn lock_summary(enabled: bool, timeout_minutes: u32) -> String {
    if enabled {
        format!("Screen locks after {} minute{} idle", timeout_minutes, if timeout_minutes == 1 { "" } else { "s" })
    } else {
        "Automatic screen lock is off".to_string()
    }
}

// ── Camera probe ──────────────────────────────────────────────────────────────
//
// Same implementation as the privacy bar widget: glob `/dev/video*` ourselves,
// then call `fuser` — exit 0 means at least one process holds a device open.
async fn camera_in_use() -> bool {
    let Ok(entries) = std::fs::read_dir("/dev") else {
        return false;
    };
    let mut devices: Vec<std::path::PathBuf> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let s = name.to_string_lossy();
            if s.starts_with("video") && s.chars().skip(5).all(|c| c.is_ascii_digit()) {
                Some(e.path())
            } else {
                None
            }
        })
        .collect();
    if devices.is_empty() {
        return false;
    }
    devices.sort();

    let res = tokio::process::Command::new("fuser")
        .args(devices.iter().map(|p| p.as_os_str()))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await;
    matches!(res, Ok(s) if s.success())
}
