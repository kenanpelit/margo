//! Podman bar pill — port of the noctalia `podman` plugin's
//! bar half.
//!
//! Render-only widget. Streams `podman events --format json` and probes
//! `podman ps` / `podman pod ps` / `podman images` on a change, with a 60 s
//! polling fallback if the stream fails (ten minute safety refresh otherwise).
//! Draws an icon + tooltip summary. One shared poll ([`PODMAN_POLL`]) for every bar, running only
//! while a pill is mapped — not while it sits in a collapsed drawer. Click emits `PodmanOutput::Clicked`; frame toggles
//! `MenuType::Podman`. Menu lives in
//! `menu_widgets/podman/podman_menu_widget.rs`.

use mshell_utils::shared_poll::{EventFeed, PollSubscription, SharedPoll};
use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::Duration;

const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

pub(crate) static PODMAN_POLL: LazyLock<SharedPoll<PodmanSummary>> = LazyLock::new(|| {
    SharedPoll::new(|| REFRESH_INTERVAL, || Box::pin(fetch_podman_summary())).with_feed(
        EventFeed::new(
            || {
                let mut cmd = tokio::process::Command::new("podman");
                cmd.args(["events", "--format", "json"]);
                cmd
            },
            podman_changed,
            || Duration::from_secs(600),
            REFRESH_INTERVAL,
        ),
    )
});

/// Container/pod/image mutations affect the summary; inspect/exec/health
/// events do not. Invalid lines (or diagnostics from an older podman) never
/// trigger a probe.
fn podman_changed(line: &str) -> bool {
    let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
        return false;
    };
    let kind = event.get("Type").and_then(|v| v.as_str()).unwrap_or("");
    let status = event.get("Status").and_then(|v| v.as_str()).unwrap_or("");
    matches!(kind, "container" | "pod" | "image")
        && matches!(
            status,
            "create"
                | "start"
                | "stop"
                | "die"
                | "died"
                | "remove"
                | "prune"
                | "pull"
                | "push"
                | "load"
                | "save"
                | "tag"
                | "untag"
                | "build"
                | "import"
                | "commit"
                | "pause"
                | "unpause"
                | "restart"
                | "checkpoint"
                | "restore"
                | "cleanup"
        )
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PodmanSummary {
    pub(crate) running_containers: usize,
    pub(crate) total_containers: usize,
    pub(crate) running_pods: usize,
    pub(crate) total_pods: usize,
    pub(crate) image_count: usize,
    pub(crate) error: Option<String>,
}

pub(crate) struct PodmanModel {
    summary: PodmanSummary,
    /// Active while the pill is mapped; keeps the shared poll running.
    _poll: Rc<PollSubscription<PodmanSummary>>,
}

#[derive(Debug)]
pub(crate) enum PodmanInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum PodmanOutput {
    Clicked,
}

pub(crate) struct PodmanInit {}

#[derive(Debug)]
pub(crate) enum PodmanCommandOutput {
    Refreshed(PodmanSummary),
}

#[relm4::component(pub)]
impl Component for PodmanModel {
    type CommandOutput = PodmanCommandOutput;
    type Input = PodmanInput;
    type Output = PodmanOutput;
    type Init = PodmanInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["ok-button-surface", "ok-bar-widget", "podman-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,
            set_has_tooltip: true,

            #[name="button"]
            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| {
                    sender.input(PodmanInput::Clicked);
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
        let poll = Rc::new(PODMAN_POLL.subscribe());
        {
            let p = poll.clone();
            root.connect_map(move |_| p.set_active(true));
            let p = poll.clone();
            root.connect_unmap(move |_| p.set_active(false));
        }
        let mut rx = poll.receiver();
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            loop {
                let summary = rx.borrow_and_update().clone();
                if let Some(s) = summary {
                    let _ = out.send(PodmanCommandOutput::Refreshed(s));
                }
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    changed = rx.changed() => if changed.is_err() { break },
                }
            }
        });

        let model = PodmanModel {
            summary: PodmanSummary::default(),
            _poll: poll,
        };

        let widgets = view_output!();
        apply_visual(&widgets.image, &root, &model.summary);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            PodmanInput::Clicked => {
                let _ = sender.output(PodmanOutput::Clicked);
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
            PodmanCommandOutput::Refreshed(s) => {
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

    if let Some(pods_raw) = run_capture("podman", &["pod", "ps", "--format", "json"]).await
        && let Ok(pods) = serde_json::from_str::<serde_json::Value>(&pods_raw)
        && let Some(arr) = pods.as_array()
    {
        s.total_pods = arr.len();
        s.running_pods = arr
            .iter()
            .filter(|p| is_running_state(p.get("Status").and_then(|v| v.as_str()).unwrap_or("")))
            .count();
    }

    if let Some(images_raw) = run_capture("podman", &["images", "--format", "json"]).await
        && let Ok(images) = serde_json::from_str::<serde_json::Value>(&images_raw)
        && let Some(arr) = images.as_array()
    {
        s.image_count = arr.len();
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

#[cfg(test)]
mod tests {
    use super::{is_running_state, podman_changed};

    #[test]
    fn stream_only_refreshes_on_summary_changes() {
        for kind in ["container", "pod", "image"] {
            assert!(podman_changed(&format!(
                r#"{{"Type":"{kind}","Status":"create"}}"#
            )));
        }
        assert!(podman_changed(r#"{"Type":"container","Status":"died"}"#));
        // Real `podman events --format json` line (podman 6.1).
        assert!(podman_changed(
            r#"{"ID":"e2ac70e7","Name":"docker.io/library/hello-world","Status":"pull","time":1793794908,"timeNano":1793794908131113390,"Type":"image","Attributes":null}"#
        ));
        assert!(podman_changed(
            r#"{"Type":"container","Status":"checkpoint"}"#
        ));
        assert!(!podman_changed(r#"{"Type":"container","Status":"exec"}"#));
        assert!(!podman_changed(r#"{"Type":"network","Status":"create"}"#));
        assert!(!podman_changed("podman: not available"));
    }

    #[test]
    fn running_match_is_case_insensitive() {
        // Container `State` is lowercase "running"; pod `Status` may capitalize.
        assert!(is_running_state("running"));
        assert!(is_running_state("Running"));
        assert!(is_running_state("RUNNING"));
        assert!(!is_running_state("exited"));
        assert!(!is_running_state("paused"));
        assert!(!is_running_state(""));
    }
}
