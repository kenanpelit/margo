//! Privacy indicator — bar pill that lights up whenever an app
//! is using the microphone or a camera.
//!
//! Two independent state sources:
//!
//!   * **Mic**: subscribes to `audio_service().recording_streams`.
//!     PipeWire / PulseAudio already exposes the active recording
//!     streams as a reactive property, filtered to actual
//!     `StreamType::Record` (monitor / mirror streams are not in
//!     the list). Any non-empty list → mic is in use.
//!
//!   * **Camera**: there's no comparable always-on watcher for
//!     `/dev/video*`, so we poll. Every 3 s we spawn
//!     `fuser /dev/video*` — exits 0 when at least one process
//!     holds a video device open. Cheap; only runs while mshell
//!     is alive.
//!
//! **Screencast** detection is deliberately out of scope here —
//! mshell's own recordings already show up in the dedicated
//! `RecordingIndicator` pill, and a robust "any app is
//! screencasting" check requires PipeWire client integration
//! (the `xdg-desktop-portal` flow opens a node we'd need to
//! enumerate). That's tracked as a follow-up.
//!
//! The pill hides itself when nothing is active so the bar stays
//! quiet by default. Each active sensor adds its own glyph
//! inline; the tooltip names which apps are using each one.

use mshell_common::watch;
use mshell_services::audio_service;
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

const CAMERA_POLL_INTERVAL: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PrivacyState {
    /// Recording app names — the labels we surface in the tooltip
    /// when the mic is in use. Empty when no app is recording.
    mic_apps: Vec<String>,
    /// Whether any process currently holds `/dev/video*` open.
    /// We can't enumerate the holders without parsing `fuser`'s
    /// stderr (which prints PIDs there, not names) — for now we
    /// just report "yes / no".
    camera_in_use: bool,
}

impl PrivacyState {
    fn is_active(&self) -> bool {
        !self.mic_apps.is_empty() || self.camera_in_use
    }
}

pub(crate) struct PrivacyModel {
    state: PrivacyState,
    _orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum PrivacyInput {}

#[derive(Debug)]
pub(crate) enum PrivacyOutput {}

pub(crate) struct PrivacyInit {
    pub(crate) orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum PrivacyCommandOutput {
    MicChanged(Vec<String>),
    CameraChanged(bool),
}

#[relm4::component(pub)]
impl Component for PrivacyModel {
    type CommandOutput = PrivacyCommandOutput;
    type Input = PrivacyInput;
    type Output = PrivacyOutput;
    type Init = PrivacyInit;

    view! {
        #[root]
        gtk::Box {
            #[watch]
            set_css_classes: &css_classes(&model.state),
            set_hexpand: model._orientation == Orientation::Vertical,
            set_vexpand: model._orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,
            set_has_tooltip: true,
            #[watch]
            set_tooltip_text: Some(&tooltip(&model.state)),
            // Stay hidden when nothing is using mic / camera so
            // the bar reads as quiet by default.
            #[watch]
            set_visible: model.state.is_active(),

            gtk::Box {
                set_css_classes: &["ok-button-flat", "ok-bar-widget"],
                set_orientation: Orientation::Horizontal,
                set_spacing: 4,
                set_halign: gtk::Align::Center,
                set_valign: gtk::Align::Center,

                gtk::Image {
                    add_css_class: "privacy-mic",
                    set_icon_name: Some("microphone-sensitivity-high-symbolic"),
                    #[watch]
                    set_visible: !model.state.mic_apps.is_empty(),
                },
                gtk::Image {
                    add_css_class: "privacy-camera",
                    set_icon_name: Some("camera-video-symbolic"),
                    #[watch]
                    set_visible: model.state.camera_in_use,
                },
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // ── Mic watcher ────────────────────────────────────────
        // The `recording_streams` property is already filtered to
        // real `StreamType::Record` streams (PulseAudio's monitor
        // streams are excluded upstream). Subscribe via the
        // shared `watch!` helper — it spawns a command task that
        // re-emits whenever the property's watch stream fires.
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
            let _ = out.send(PrivacyCommandOutput::MicChanged(snapshot));
        });

        // ── Camera poll ────────────────────────────────────────
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut last_state: Option<bool> = None;
            loop {
                let now = camera_in_use().await;
                if last_state != Some(now) {
                    let _ = out.send(PrivacyCommandOutput::CameraChanged(now));
                    last_state = Some(now);
                }
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    _ = tokio::time::sleep(CAMERA_POLL_INTERVAL) => {}
                }
            }
        });

        // Prime mic state synchronously so the first render isn't
        // a flash of "no mic" before the watcher fires. `Property`
        // is wayle's own primitive — `.get()` reads it without
        // subscribing.
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

        let model = PrivacyModel {
            state: PrivacyState {
                mic_apps: initial_mic,
                camera_in_use: false,
            },
            _orientation: params.orientation,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        _message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        // No inputs — both signals come through the command channel.
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            PrivacyCommandOutput::MicChanged(apps) => {
                self.state.mic_apps = apps;
            }
            PrivacyCommandOutput::CameraChanged(yes) => {
                self.state.camera_in_use = yes;
            }
        }
    }
}

// ── View helpers ────────────────────────────────────────────────

fn css_classes(state: &PrivacyState) -> Vec<&'static str> {
    let mut classes = vec!["ok-button-surface", "ok-bar-widget", "privacy-bar-widget"];
    if state.is_active() {
        classes.push("active");
    }
    classes
}

fn tooltip(state: &PrivacyState) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !state.mic_apps.is_empty() {
        if state.mic_apps.len() == 1 {
            parts.push(format!("Microphone: {}", state.mic_apps[0]));
        } else {
            parts.push(format!(
                "Microphone ({} apps): {}",
                state.mic_apps.len(),
                state.mic_apps.join(", ")
            ));
        }
    }
    if state.camera_in_use {
        parts.push("Camera: in use".to_string());
    }
    if parts.is_empty() {
        "Privacy: no sensors in use".to_string()
    } else {
        parts.join("\n")
    }
}

// ── Camera probe ────────────────────────────────────────────────
//
// `fuser /dev/video*` exits 0 when at least one process holds any
// of the listed character devices open, 1 when none do, and >1 on
// unexpected errors. We only ever care about the 0/non-0 split.
// Output is discarded — we don't need the PID list for the pill,
// just the boolean.
async fn camera_in_use() -> bool {
    // Glob the device list ourselves so we don't depend on shell
    // expansion. If there are zero `/dev/video*` files, skip the
    // probe entirely (no camera attached → permanently false).
    let Ok(entries) = std::fs::read_dir("/dev") else {
        return false;
    };
    let mut devices: Vec<std::path::PathBuf> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let s = name.to_string_lossy();
            if s.starts_with("video")
                && s.chars().skip(5).all(|c| c.is_ascii_digit())
            {
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
