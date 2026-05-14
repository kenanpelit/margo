//! Podman menu widget — content surface for `MenuType::Npodman`.
//!
//! Three sections stacked vertically:
//!   1. **Header** — title + running counter chip + Refresh button.
//!   2. **Tabs** — gtk::StackSwitcher over `Containers`, `Images`,
//!      `Pods`. The active tab keeps the same layout: a
//!      scrollable list of rows, each with an icon + name +
//!      status badge + action buttons.
//!   3. **Footer** — small docs / `podman` man-page link.
//!
//! Per-row actions (containers tab):
//!   * `▶ Start` — `podman start <id>` for stopped containers
//!   * `⏸ Stop`  — `podman stop -t 3 <id>` for running ones
//!   * `↻ Restart` — `podman restart <id>`
//!   * `🗑 Remove` — `podman rm -f <id>`
//!
//! Per-row actions (pods tab):
//!   * Start / Stop / Remove (mapped to `podman pod {start,stop,rm}`).
//!
//! Per-row actions (images tab):
//!   * Remove — `podman image rm <name>:<tag>` (with -f when the
//!     image is in use by a running container — best-effort).
//!
//! `podman` itself is rootless on Arch's default config; no
//! pkexec needed. Errors surface as a banner at the top of the
//! tab. Auto-refresh every 60 s, plus immediate re-poll after
//! every action so the list mirrors live state.

use crate::bars::bar_widgets::npodman::{PodmanSummary, fetch_podman_summary};
use relm4::gtk::prelude::{BoxExt, ButtonExt, ListBoxRowExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;
use tracing::warn;

const REFRESH_INTERVAL: Duration = Duration::from_secs(60);
const STARTUP_DELAY: Duration = Duration::from_millis(250);
const POST_ACTION_DELAY: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PodmanPanelState {
    pub(crate) summary: PodmanSummary,
    pub(crate) containers: Vec<ContainerRow>,
    pub(crate) images: Vec<ImageRow>,
    pub(crate) pods: Vec<PodRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ContainerRow {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) image: String,
    pub(crate) state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ImageRow {
    pub(crate) id: String,
    pub(crate) repository: String,
    pub(crate) tag: String,
    pub(crate) size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct PodRow {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) status: String,
    pub(crate) container_count: usize,
}

pub(crate) struct NpodmanMenuWidgetModel {
    state: PodmanPanelState,
    header_counter: gtk::Label,
    containers_list: gtk::ListBox,
    images_list: gtk::ListBox,
    pods_list: gtk::ListBox,
    error_banner: gtk::Label,
}

impl std::fmt::Debug for NpodmanMenuWidgetModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NpodmanMenuWidgetModel")
            .field("state", &self.state)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum NpodmanMenuWidgetInput {
    RefreshNow,
    /// `podman <subcmd…>` — runs the command then triggers a refresh.
    RunPodman(Vec<String>),
}

#[derive(Debug)]
pub(crate) enum NpodmanMenuWidgetOutput {}

pub(crate) struct NpodmanMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum NpodmanMenuWidgetCommandOutput {
    Refreshed(PodmanPanelState),
}

#[relm4::component(pub(crate))]
impl Component for NpodmanMenuWidgetModel {
    type CommandOutput = NpodmanMenuWidgetCommandOutput;
    type Input = NpodmanMenuWidgetInput;
    type Output = NpodmanMenuWidgetOutput;
    type Init = NpodmanMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "npodman-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            // ── Header ──────────────────────────────────────────
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,

                gtk::Image {
                    set_icon_name: Some("package-symbolic"),
                    set_pixel_size: 24,
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Podman",
                    set_hexpand: true,
                    set_xalign: 0.0,
                },

                #[local_ref]
                header_counter_widget -> gtk::Label {
                    add_css_class: "npodman-counter",
                    set_valign: gtk::Align::Center,
                },

                gtk::Button {
                    set_css_classes: &["ok-button-surface"],
                    set_label: "Refresh",
                    connect_clicked[sender] => move |_| {
                        sender.input(NpodmanMenuWidgetInput::RefreshNow);
                    },
                },
            },

            #[local_ref]
            error_banner_widget -> gtk::Label {
                add_css_class: "npodman-error-banner",
                set_xalign: 0.0,
                set_wrap: true,
                set_wrap_mode: gtk::pango::WrapMode::WordChar,
                set_visible: false,
            },

            // ── Tabs ───────────────────────────────────────────
            #[name = "stack_switcher"]
            gtk::StackSwitcher {
                set_stack: Some(&stack),
                set_halign: gtk::Align::Start,
            },

            #[name = "stack"]
            gtk::Stack {
                set_transition_type: gtk::StackTransitionType::SlideLeftRight,
                set_transition_duration: 200,

                add_titled[Some("containers"), "Containers"] = &gtk::ScrolledWindow {
                    set_min_content_height: 240,
                    set_max_content_height: 420,
                    set_hscrollbar_policy: gtk::PolicyType::Never,
                    set_propagate_natural_height: true,

                    #[local_ref]
                    containers_list_widget -> gtk::ListBox {
                        add_css_class: "npodman-row-list",
                        set_selection_mode: gtk::SelectionMode::None,
                    },
                },
                add_titled[Some("images"), "Images"] = &gtk::ScrolledWindow {
                    set_min_content_height: 240,
                    set_max_content_height: 420,
                    set_hscrollbar_policy: gtk::PolicyType::Never,
                    set_propagate_natural_height: true,

                    #[local_ref]
                    images_list_widget -> gtk::ListBox {
                        add_css_class: "npodman-row-list",
                        set_selection_mode: gtk::SelectionMode::None,
                    },
                },
                add_titled[Some("pods"), "Pods"] = &gtk::ScrolledWindow {
                    set_min_content_height: 240,
                    set_max_content_height: 420,
                    set_hscrollbar_policy: gtk::PolicyType::Never,
                    set_propagate_natural_height: true,

                    #[local_ref]
                    pods_list_widget -> gtk::ListBox {
                        add_css_class: "npodman-row-list",
                        set_selection_mode: gtk::SelectionMode::None,
                    },
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let header_counter_widget = gtk::Label::new(Some("0 / 0"));
        let containers_list_widget = gtk::ListBox::new();
        let images_list_widget = gtk::ListBox::new();
        let pods_list_widget = gtk::ListBox::new();
        let error_banner_widget = gtk::Label::new(None);

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
                    let s = fetch_panel_state().await;
                    let _ = out.send(NpodmanMenuWidgetCommandOutput::Refreshed(s));
                }
            }
        });

        let model = NpodmanMenuWidgetModel {
            state: PodmanPanelState::default(),
            header_counter: header_counter_widget.clone(),
            containers_list: containers_list_widget.clone(),
            images_list: images_list_widget.clone(),
            pods_list: pods_list_widget.clone(),
            error_banner: error_banner_widget.clone(),
        };

        let widgets = view_output!();
        sync_view(&model, &sender);

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NpodmanMenuWidgetInput::RefreshNow => {
                sender.command(|out, _shutdown| async move {
                    let s = fetch_panel_state().await;
                    let _ = out.send(NpodmanMenuWidgetCommandOutput::Refreshed(s));
                });
            }
            NpodmanMenuWidgetInput::RunPodman(args) => {
                sender.command(move |out, _shutdown| async move {
                    let status = tokio::process::Command::new("podman")
                        .args(&args)
                        .status()
                        .await;
                    match status {
                        Ok(s) if s.success() => {}
                        Ok(s) => warn!(?s, ?args, "podman action returned non-zero"),
                        Err(e) => warn!(error = %e, ?args, "podman spawn failed"),
                    }
                    tokio::time::sleep(POST_ACTION_DELAY).await;
                    let s = fetch_panel_state().await;
                    let _ = out.send(NpodmanMenuWidgetCommandOutput::Refreshed(s));
                });
            }
        }
    }

    fn update_cmd_with_view(
        &mut self,
        _widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NpodmanMenuWidgetCommandOutput::Refreshed(state) => {
                if self.state != state {
                    self.state = state;
                    sync_view(self, &sender);
                }
            }
        }
    }
}

fn sync_view(model: &NpodmanMenuWidgetModel, sender: &ComponentSender<NpodmanMenuWidgetModel>) {
    let s = &model.state;

    // Header counter — total / running.
    model.header_counter.set_label(&format!(
        "{} / {}",
        s.summary.running_containers, s.summary.total_containers
    ));

    if let Some(err) = &s.summary.error {
        model.error_banner.set_label(err);
        model.error_banner.set_visible(true);
    } else {
        model.error_banner.set_visible(false);
    }

    // ── Containers list ─────────────────────────────────────────
    clear_listbox(&model.containers_list);
    if s.containers.is_empty() {
        model
            .containers_list
            .append(&placeholder_row("(no containers)"));
    } else {
        for c in &s.containers {
            model
                .containers_list
                .append(&make_container_row(c, sender));
        }
    }

    // ── Images list ─────────────────────────────────────────────
    clear_listbox(&model.images_list);
    if s.images.is_empty() {
        model.images_list.append(&placeholder_row("(no images)"));
    } else {
        for img in &s.images {
            model.images_list.append(&make_image_row(img, sender));
        }
    }

    // ── Pods list ───────────────────────────────────────────────
    clear_listbox(&model.pods_list);
    if s.pods.is_empty() {
        model.pods_list.append(&placeholder_row("(no pods)"));
    } else {
        for p in &s.pods {
            model.pods_list.append(&make_pod_row(p, sender));
        }
    }
}

fn clear_listbox(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

fn placeholder_row(text: &str) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);
    let label = gtk::Label::new(Some(text));
    label.add_css_class("label-small");
    label.set_xalign(0.0);
    label.set_margin_top(8);
    label.set_margin_bottom(8);
    label.set_margin_start(12);
    row.set_child(Some(&label));
    row
}

fn make_container_row(
    c: &ContainerRow,
    sender: &ComponentSender<NpodmanMenuWidgetModel>,
) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);
    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(8)
        .margin_end(8)
        .build();

    let texts = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .build();
    let name = gtk::Label::new(Some(&c.name));
    name.add_css_class("label-medium-bold");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    texts.append(&name);
    let image_label = gtk::Label::new(Some(&c.image));
    image_label.add_css_class("label-small");
    image_label.set_xalign(0.0);
    image_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    texts.append(&image_label);
    outer.append(&texts);

    let badge = gtk::Label::new(Some(&c.state));
    badge.add_css_class("npodman-state-badge");
    badge.add_css_class(&format!("npodman-state-{}", c.state.to_lowercase()));
    badge.set_valign(gtk::Align::Center);
    outer.append(&badge);

    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(4)
        .build();
    let is_running = c.state.eq_ignore_ascii_case("running");
    if is_running {
        actions.append(&action_button(
            "media-pause-symbolic",
            "Stop",
            sender,
            vec!["stop".to_string(), "-t".to_string(), "3".to_string(), c.id.clone()],
        ));
        actions.append(&action_button(
            "view-refresh-symbolic",
            "Restart",
            sender,
            vec!["restart".to_string(), c.id.clone()],
        ));
    } else {
        actions.append(&action_button(
            "media-play-symbolic",
            "Start",
            sender,
            vec!["start".to_string(), c.id.clone()],
        ));
    }
    actions.append(&action_button(
        "trash-symbolic",
        "Remove",
        sender,
        vec!["rm".to_string(), "-f".to_string(), c.id.clone()],
    ));
    outer.append(&actions);

    row.set_child(Some(&outer));
    row
}

fn make_image_row(
    img: &ImageRow,
    sender: &ComponentSender<NpodmanMenuWidgetModel>,
) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);
    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(8)
        .margin_end(8)
        .build();

    let texts = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .build();
    let title = format!("{}:{}", img.repository, img.tag);
    let name = gtk::Label::new(Some(&title));
    name.add_css_class("label-medium-bold");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    texts.append(&name);
    let size_label = gtk::Label::new(Some(&format_bytes(img.size_bytes)));
    size_label.add_css_class("label-small");
    size_label.set_xalign(0.0);
    texts.append(&size_label);
    outer.append(&texts);

    let target = if img.repository == "<none>" {
        img.id.clone()
    } else {
        title.clone()
    };
    outer.append(&action_button(
        "trash-symbolic",
        "Remove",
        sender,
        vec!["rmi".to_string(), "-f".to_string(), target],
    ));

    row.set_child(Some(&outer));
    row
}

fn make_pod_row(
    p: &PodRow,
    sender: &ComponentSender<NpodmanMenuWidgetModel>,
) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);
    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(8)
        .margin_end(8)
        .build();

    let texts = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .build();
    let name = gtk::Label::new(Some(&p.name));
    name.add_css_class("label-medium-bold");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    texts.append(&name);
    let detail = gtk::Label::new(Some(&format!("{} containers", p.container_count)));
    detail.add_css_class("label-small");
    detail.set_xalign(0.0);
    texts.append(&detail);
    outer.append(&texts);

    let badge = gtk::Label::new(Some(&p.status));
    badge.add_css_class("npodman-state-badge");
    badge.add_css_class(&format!("npodman-state-{}", p.status.to_lowercase()));
    badge.set_valign(gtk::Align::Center);
    outer.append(&badge);

    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(4)
        .build();
    let is_running = p.status.eq_ignore_ascii_case("running");
    if is_running {
        actions.append(&action_button(
            "media-pause-symbolic",
            "Stop",
            sender,
            vec!["pod".to_string(), "stop".to_string(), p.id.clone()],
        ));
    } else {
        actions.append(&action_button(
            "media-play-symbolic",
            "Start",
            sender,
            vec!["pod".to_string(), "start".to_string(), p.id.clone()],
        ));
    }
    actions.append(&action_button(
        "trash-symbolic",
        "Remove",
        sender,
        vec!["pod".to_string(), "rm".to_string(), "-f".to_string(), p.id.clone()],
    ));
    outer.append(&actions);

    row.set_child(Some(&outer));
    row
}

fn action_button(
    icon: &str,
    tooltip: &str,
    sender: &ComponentSender<NpodmanMenuWidgetModel>,
    args: Vec<String>,
) -> gtk::Button {
    let btn = gtk::Button::from_icon_name(icon);
    btn.add_css_class("ok-button-flat");
    btn.set_tooltip_text(Some(tooltip));
    let s = sender.clone();
    btn.connect_clicked(move |_| {
        s.input(NpodmanMenuWidgetInput::RunPodman(args.clone()));
    });
    btn
}

fn format_bytes(b: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if b >= GB {
        format!("{:.2} GiB", b as f64 / GB as f64)
    } else if b >= MB {
        format!("{:.1} MiB", b as f64 / MB as f64)
    } else if b >= KB {
        format!("{:.0} KiB", b as f64 / KB as f64)
    } else {
        format!("{b} B")
    }
}

/// Probe all three lists + the summary in parallel. Each sub-
/// probe is independent — missing tools surface as empty lists
/// rather than failing the whole panel.
async fn fetch_panel_state() -> PodmanPanelState {
    let summary = fetch_podman_summary().await;
    let mut state = PodmanPanelState {
        summary: summary.clone(),
        ..PodmanPanelState::default()
    };
    if summary.error.is_some() {
        return state;
    }

    if let Some(json) = run_capture("podman", &["ps", "--all", "--format", "json"]).await {
        if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&json) {
            state.containers = arr.iter().map(parse_container_row).collect();
        }
    }
    if let Some(json) = run_capture("podman", &["images", "--format", "json"]).await {
        if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&json) {
            state.images = arr.iter().map(parse_image_row).collect();
        }
    }
    if let Some(json) = run_capture("podman", &["pod", "ps", "--format", "json"]).await {
        if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&json) {
            state.pods = arr.iter().map(parse_pod_row).collect();
        }
    }

    state
}

fn parse_container_row(v: &serde_json::Value) -> ContainerRow {
    ContainerRow {
        id: v
            .get("Id")
            .and_then(|s| s.as_str())
            .or_else(|| v.get("ID").and_then(|s| s.as_str()))
            .unwrap_or("")
            .to_string(),
        name: v
            .get("Names")
            .and_then(|n| n.as_array())
            .and_then(|a| a.first())
            .and_then(|s| s.as_str())
            .unwrap_or("(unnamed)")
            .to_string(),
        image: v
            .get("Image")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string(),
        state: v
            .get("State")
            .and_then(|s| s.as_str())
            .unwrap_or("unknown")
            .to_string(),
    }
}

fn parse_image_row(v: &serde_json::Value) -> ImageRow {
    let names = v
        .get("Names")
        .and_then(|n| n.as_array())
        .and_then(|a| a.first())
        .and_then(|s| s.as_str())
        .unwrap_or("<none>:<none>");
    let (repo, tag) = match names.rsplit_once(':') {
        Some((r, t)) => (r.to_string(), t.to_string()),
        None => (names.to_string(), "latest".to_string()),
    };
    ImageRow {
        id: v
            .get("Id")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string(),
        repository: repo,
        tag,
        size_bytes: v.get("Size").and_then(|s| s.as_u64()).unwrap_or(0),
    }
}

fn parse_pod_row(v: &serde_json::Value) -> PodRow {
    PodRow {
        id: v
            .get("Id")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string(),
        name: v
            .get("Name")
            .and_then(|s| s.as_str())
            .unwrap_or("(unnamed)")
            .to_string(),
        status: v
            .get("Status")
            .and_then(|s| s.as_str())
            .unwrap_or("unknown")
            .to_string(),
        container_count: v
            .get("Containers")
            .and_then(|c| c.as_array())
            .map(|a| a.len())
            .unwrap_or_else(|| {
                v.get("NumberOfContainers")
                    .and_then(|n| n.as_u64())
                    .map(|n| n as usize)
                    .unwrap_or(0)
            }),
    }
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
