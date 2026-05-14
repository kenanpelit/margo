//! Podman bar pill — port of the noctalia `npodman` plugin's
//! bar half.
//!
//! Render-only widget. Polls `podman ps` / `podman pod ps` /
//! `podman images` every 60 s and draws an icon + tooltip
//! summary. Click emits `NpodmanOutput::Clicked`; frame toggles
//! `MenuType::Npodman`. Menu lives in
//! `menu_widgets/npodman/npodman_menu_widget.rs`.

use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

const REFRESH_INTERVAL: Duration = Duration::from_secs(60);
const STARTUP_DELAY: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PodmanSummary {
    pub(crate) running_containers: usize,
    pub(crate) total_containers: usize,
    pub(crate) running_pods: usize,
    pub(crate) total_pods: usize,
    pub(crate) image_count: usize,
    pub(crate) error: Option<String>,
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
pub(crate) enum NpodmanOutput {
    Clicked,
}

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
                    let s = fetch_podman_summary().await;
                    let _ = out.send(NpodmanCommandOutput::Refreshed(s));
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
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NpodmanInput::Clicked => {
                let _ = sender.output(NpodmanOutput::Clicked);
            }
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
    let icon = if s.error.is_some() {
        "firewall-error-symbolic"
    } else {
        "package-symbolic"
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

    root.remove_css_class("active");
    root.remove_css_class("error");
    if s.error.is_some() {
        root.add_css_class("error");
    } else if s.running_containers > 0 {
        root.add_css_class("active");
    }
}

/// Quick summary probe (for the bar pill only). Exposed
/// pub(crate) so the menu widget can reuse the same probe path
/// after each action.
pub(crate) async fn fetch_podman_summary() -> PodmanSummary {
    let mut s = PodmanSummary::default();

    let containers = match run_capture("podman", &["ps", "--all", "--format", "json"]).await {
        Some(out) => out,
        None => {
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
            .filter(|c| is_running_state(c.get("State").and_then(|v| v.as_str()).unwrap_or("")))
            .count();
    }

    if let Some(pods_raw) = run_capture("podman", &["pod", "ps", "--format", "json"]).await {
        if let Ok(pods) = serde_json::from_str::<serde_json::Value>(&pods_raw) {
            if let Some(arr) = pods.as_array() {
                s.total_pods = arr.len();
                s.running_pods = arr
                    .iter()
                    .filter(|p| {
                        is_running_state(p.get("Status").and_then(|v| v.as_str()).unwrap_or(""))
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

fn is_running_state(state: &str) -> bool {
    state.eq_ignore_ascii_case("running")
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
