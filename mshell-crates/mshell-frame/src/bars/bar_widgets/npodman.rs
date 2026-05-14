//! Podman container bar widget — MVP port of the `npodman`
//! noctalia plugin. Bar pill that summarises running containers
//! + pods + images. The upstream Panel.qml has a full table UI
//! with start/stop/delete actions — that's deferred to follow-up
//! work; this widget is the always-visible indicator.
//!
//! Probe is `podman ps --all --format json` + `podman pod ps
//! --format json` + `podman images --format json` every 60 s.
//! `podman` JSON output is a stable contract (Podman 4+; we
//! support both Podman 4 and Podman 5 shape — the fields we care
//! about are the same in both). Tooling defaults to rootless
//! podman on the current user; if you run rootful, run mshell as
//! root or set `CONTAINER_CONNECTION` before launching.
//!
//! Click opens `podman ps` in the user's terminal. Same terminal-
//! lookup chain as `nufw.rs` because the requirement is identical.

use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;
use tracing::warn;

const REFRESH_INTERVAL: Duration = Duration::from_secs(60);
const STARTUP_DELAY: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PodmanSummary {
    running_containers: usize,
    total_containers: usize,
    running_pods: usize,
    total_pods: usize,
    image_count: usize,
    error: Option<String>,
}

#[derive(Debug)]
pub(crate) struct NpodmanModel {
    summary: PodmanSummary,
}

#[derive(Debug)]
pub(crate) enum NpodmanInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum NpodmanOutput {}

pub(crate) struct NpodmanInit {}

#[derive(Debug)]
pub(crate) enum NpodmanCommandOutput {
    Refreshed(PodmanSummary),
}

#[relm4::component(pub)]
impl Component for NpodmanModel {
    type CommandOutput = NpodmanCommandOutput;
    type Input = NpodmanInput;
    type Output = NpodmanOutput;
    type Init = NpodmanInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["ok-button-surface", "ok-bar-widget", "npodman-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,
            set_has_tooltip: true,

            #[name="button"]
            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| {
                    sender.input(NpodmanInput::Clicked);
                },

                #[name="image"]
                gtk::Image {
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                }
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        sender.command(|out, shutdown| {
            async move {
                let shutdown_fut = shutdown.wait();
                tokio::pin!(shutdown_fut);
                let mut first = true;
                loop {
                    let delay = if first { STARTUP_DELAY } else { REFRESH_INTERVAL };
                    first = false;
                    tokio::select! {
                        () = &mut shutdown_fut => break,
                        _ = tokio::time::sleep(delay) => {}
                    }
                    let summary = fetch_podman_summary().await;
                    let _ = out.send(NpodmanCommandOutput::Refreshed(summary));
                }
            }
        });

        let model = NpodmanModel {
            summary: PodmanSummary::default(),
        };

        let widgets = view_output!();
        apply_visual(&widgets.image, &root, &model.summary);

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NpodmanInput::Clicked => spawn_terminal_ps(),
        }
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            NpodmanCommandOutput::Refreshed(s) => {
                if self.summary != s {
                    self.summary = s;
                    apply_visual(&widgets.image, root, &self.summary);
                }
            }
        }
    }
}

fn apply_visual(image: &gtk::Image, root: &gtk::Box, s: &PodmanSummary) {
    // Stock GNOME / Adwaita doesn't ship a "container" symbolic
    // glyph; `package-x-generic-symbolic` (the box icon) is the
    // standard substitute used by Cockpit, k3sup, Lazydocker, etc.
    // `dialog-warning-symbolic` for the error / not-installed
    // state so the visual matches the other plugin widgets.
    let icon = if s.error.is_some() {
        "dialog-warning-symbolic"
    } else if s.running_containers > 0 {
        "package-x-generic-symbolic"
    } else {
        "package-x-generic-symbolic"
    };
    image.set_icon_name(Some(icon));

    let tooltip = if let Some(err) = &s.error {
        format!("Podman: {err}")
    } else {
        let mut lines = Vec::with_capacity(3);
        lines.push(format!(
            "Containers: {} running / {} total",
            s.running_containers, s.total_containers
        ));
        if s.total_pods > 0 {
            lines.push(format!(
                "Pods: {} running / {} total",
                s.running_pods, s.total_pods
            ));
        }
        if s.image_count > 0 {
            lines.push(format!("Images: {}", s.image_count));
        }
        lines.join("\n")
    };
    root.set_tooltip_text(Some(&tooltip));

    if s.running_containers > 0 {
        root.add_css_class("active");
    } else {
        root.remove_css_class("active");
    }
}

fn spawn_terminal_ps() {
    tokio::spawn(async move {
        for term in ["kitty", "alacritty", "foot", "wezterm", "xterm"] {
            if let Ok(true) = which_async(term).await {
                let _ = tokio::process::Command::new(term)
                    .args(["-e", "sh", "-c", "podman ps --all; echo; echo Press any key…; read -n 1"])
                    .status()
                    .await;
                return;
            }
        }
        warn!("no terminal emulator found for npodman ps");
    });
}

async fn which_async(bin: &str) -> std::io::Result<bool> {
    let status = tokio::process::Command::new("which")
        .arg(bin)
        .status()
        .await?;
    Ok(status.success())
}

async fn fetch_podman_summary() -> PodmanSummary {
    let mut s = PodmanSummary::default();

    let containers = match run_capture("podman", &["ps", "--all", "--format", "json"]).await {
        Some(out) => out,
        None => {
            // Most common: podman not installed, or rootless socket
            // isn't running yet. Either way, surface the situation
            // without crashing the bar.
            return PodmanSummary {
                error: Some("podman not available".to_string()),
                ..PodmanSummary::default()
            };
        }
    };

    let parsed: serde_json::Value = match serde_json::from_str(&containers) {
        Ok(v) => v,
        Err(e) => {
            return PodmanSummary {
                error: Some(format!("podman ps JSON parse: {e}")),
                ..PodmanSummary::default()
            };
        }
    };

    if let Some(arr) = parsed.as_array() {
        s.total_containers = arr.len();
        s.running_containers = arr
            .iter()
            .filter(|c| {
                // Podman 4 puts state under .State (string),
                // Podman 5 sometimes lowercases. Match liberally.
                c.get("State")
                    .and_then(|v| v.as_str())
                    .map(|st| st.eq_ignore_ascii_case("running"))
                    .unwrap_or(false)
            })
            .count();
    }

    // Pods + images probes — optional, don't error out if missing.
    if let Some(pods_raw) = run_capture("podman", &["pod", "ps", "--format", "json"]).await {
        if let Ok(pods) = serde_json::from_str::<serde_json::Value>(&pods_raw) {
            if let Some(arr) = pods.as_array() {
                s.total_pods = arr.len();
                s.running_pods = arr
                    .iter()
                    .filter(|p| {
                        p.get("Status")
                            .and_then(|v| v.as_str())
                            .map(|st| st.eq_ignore_ascii_case("running"))
                            .unwrap_or(false)
                    })
                    .count();
            }
        }
    }

    if let Some(images_raw) = run_capture("podman", &["images", "--format", "json"]).await {
        if let Ok(images) = serde_json::from_str::<serde_json::Value>(&images_raw) {
            if let Some(arr) = images.as_array() {
                s.image_count = arr.len();
            }
        }
    }

    s
}

async fn run_capture(cmd: &str, args: &[&str]) -> Option<String> {
    let out = tokio::process::Command::new(cmd)
        .args(args)
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}
