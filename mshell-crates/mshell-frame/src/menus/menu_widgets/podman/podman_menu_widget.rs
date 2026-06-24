//! Podman menu widget — content surface for `MenuType::Podman`.
//!
//! Three sections stacked vertically:
//!   1. **Header** — title + running counter chip + Refresh button.
//!   2. **Tabs** — gtk::StackSwitcher over `Containers`, `Images`,
//!      `Pods`.
//!   3. **Footer** — (none; refresh lives in the header).
//!
//! ### Container rows — the expandable card (the house revealer-row
//! shape, §5/§12 of `DESIGN.md`). A collapsed row shows a leading
//! state-tinted glyph + name + image + a chevron. Clicking the header
//! expands a `gtk::Revealer` holding:
//!   * the human status line (`Up 2 hours` / `Exited (137) …`),
//!   * published **port mappings** as chips (`3000 → 8080`),
//!   * a vertical list of **actions** keyed off the container state:
//!     - running  → Restart · Pause · Stop · Shell · Logs
//!     - paused   → Unpause · Stop · Logs
//!     - stopped  → Start · Logs · Remove
//!
//! **Shell** opens an interactive shell in the user's terminal
//! (`<term> -e podman exec -it <id> …`, preferring `bash`, falling back
//! to `sh`). **Logs** follows `podman logs -f` in a terminal that stays
//! open after the stream ends. Both are ports of the DMS Docker
//! Manager plugin's terminal + log affordances.
//!
//! Per-row actions (pods tab): Start / Stop / Remove.
//! Per-row actions (images tab): Remove.
//!
//! `podman` is rootless on Arch's default config; no pkexec needed.
//! Errors surface as a banner at the top. Auto-refresh every 60 s, plus
//! an immediate re-poll after every action so the list mirrors live
//! state. The set of expanded rows is kept in the model so a refresh
//! (or a post-action repoll) never collapses a row out from under the
//! user (§13.3 continuity).

use crate::bars::bar_widgets::podman::{PodmanSummary, fetch_podman_summary};
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tracing::warn;

const REFRESH_INTERVAL: Duration = Duration::from_secs(60);
const POST_ACTION_DELAY: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PodmanPanelState {
    pub(crate) summary: PodmanSummary,
    pub(crate) containers: Vec<ContainerRow>,
    pub(crate) images: Vec<ImageRow>,
    pub(crate) pods: Vec<PodRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct PortMapping {
    pub(crate) host_port: String,
    pub(crate) container_port: String,
    pub(crate) protocol: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ContainerRow {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) image: String,
    pub(crate) state: String,
    /// Human-readable status string from `podman ps` (`Up 2 hours`,
    /// `Exited (137) 3 months ago`, …).
    pub(crate) status: String,
    /// Published host↔container port mappings (unpublished ports filtered).
    pub(crate) ports: Vec<PortMapping>,
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

pub(crate) struct PodmanMenuWidgetModel {
    state: PodmanPanelState,
    header_counter: gtk::Label,
    containers_list: gtk::ListBox,
    images_list: gtk::ListBox,
    pods_list: gtk::ListBox,
    error_banner: gtk::Label,
    /// IDs of the currently-expanded container rows. Shared into each
    /// row's header closure so a toggle updates it directly, and read
    /// back by `sync_view` so a rebuild restores the expansion state.
    expanded: Rc<RefCell<HashSet<String>>>,
    /// `true` once the poll loop has been spawned (on first reveal).
    poll_started: bool,
    /// Shared with the poll loop; gates the `podman` probe so it only
    /// runs while the panel is visible.
    visible: Arc<AtomicBool>,
}

impl std::fmt::Debug for PodmanMenuWidgetModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PodmanMenuWidgetModel")
            .field("state", &self.state)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum PodmanMenuWidgetInput {
    RefreshNow,
    /// `podman <subcmd…>` — runs the command then triggers a refresh.
    RunPodman(Vec<String>),
    /// Open an interactive shell into the container in the user's terminal.
    OpenShell(String),
    /// Follow `podman logs -f` for the container in the user's terminal.
    OpenLogs(String),
    /// Sent by the host menu on show/hide. The `podman ps`/`images`/`pods`
    /// poll is started lazily on first reveal, so a menu the user never
    /// opens spawns no podman subprocesses.
    ParentRevealChanged(bool),
}

#[derive(Debug)]
pub(crate) enum PodmanMenuWidgetOutput {}

pub(crate) struct PodmanMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum PodmanMenuWidgetCommandOutput {
    Refreshed(PodmanPanelState),
}

#[relm4::component(pub(crate))]
impl Component for PodmanMenuWidgetModel {
    type CommandOutput = PodmanMenuWidgetCommandOutput;
    type Input = PodmanMenuWidgetInput;
    type Output = PodmanMenuWidgetOutput;
    type Init = PodmanMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "podman-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            // ── §12 panel header ────────────────────────────────
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,

                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_icon_name: Some("cube-symbolic"),
                    set_valign: gtk::Align::Center,
                },

                gtk::Label {
                    add_css_class: "panel-title",
                    set_label: "Podman",
                    set_hexpand: true,
                    set_xalign: 0.0,
                },

                #[local_ref]
                header_counter_widget -> gtk::Label {
                    add_css_class: "podman-counter",
                    set_valign: gtk::Align::Center,
                },

                gtk::Button {
                    add_css_class: "panel-action-btn",
                    set_icon_name: "view-refresh-symbolic",
                    set_tooltip_text: Some("Refresh"),
                    set_valign: gtk::Align::Center,
                    connect_clicked[sender] => move |_| {
                        sender.input(PodmanMenuWidgetInput::RefreshNow);
                    },
                },
            },

            #[local_ref]
            error_banner_widget -> gtk::Label {
                add_css_class: "podman-error-banner",
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
                    set_min_content_height: 0,
                    set_max_content_height: 460,
                    set_hscrollbar_policy: gtk::PolicyType::Never,
                    set_propagate_natural_height: true,

                    #[local_ref]
                    containers_list_widget -> gtk::ListBox {
                        add_css_class: "podman-row-list",
                        set_selection_mode: gtk::SelectionMode::None,
                    },
                },
                add_titled[Some("images"), "Images"] = &gtk::ScrolledWindow {
                    set_min_content_height: 0,
                    set_max_content_height: 460,
                    set_hscrollbar_policy: gtk::PolicyType::Never,
                    set_propagate_natural_height: true,

                    #[local_ref]
                    images_list_widget -> gtk::ListBox {
                        add_css_class: "podman-row-list",
                        set_selection_mode: gtk::SelectionMode::None,
                    },
                },
                add_titled[Some("pods"), "Pods"] = &gtk::ScrolledWindow {
                    set_min_content_height: 0,
                    set_max_content_height: 460,
                    set_hscrollbar_policy: gtk::PolicyType::Never,
                    set_propagate_natural_height: true,

                    #[local_ref]
                    pods_list_widget -> gtk::ListBox {
                        add_css_class: "podman-row-list",
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

        // The poll loop is started lazily on first reveal — see
        // `ParentRevealChanged` — so a menu the user never opens spawns
        // no `podman` subprocesses.
        let model = PodmanMenuWidgetModel {
            state: PodmanPanelState::default(),
            header_counter: header_counter_widget.clone(),
            containers_list: containers_list_widget.clone(),
            images_list: images_list_widget.clone(),
            pods_list: pods_list_widget.clone(),
            error_banner: error_banner_widget.clone(),
            expanded: Rc::new(RefCell::new(HashSet::new())),
            poll_started: false,
            visible: Arc::new(AtomicBool::new(false)),
        };

        let widgets = view_output!();
        sync_view(&model, &sender);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            PodmanMenuWidgetInput::RefreshNow => {
                sender.command(|out, _shutdown| async move {
                    let s = fetch_panel_state().await;
                    let _ = out.send(PodmanMenuWidgetCommandOutput::Refreshed(s));
                });
            }
            PodmanMenuWidgetInput::RunPodman(args) => {
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
                    let _ = out.send(PodmanMenuWidgetCommandOutput::Refreshed(s));
                });
            }
            PodmanMenuWidgetInput::OpenShell(id) => open_shell(&id),
            PodmanMenuWidgetInput::OpenLogs(id) => open_logs(&id),
            PodmanMenuWidgetInput::ParentRevealChanged(visible) => {
                self.visible.store(visible, Ordering::Relaxed);
                if visible {
                    if !self.poll_started {
                        self.poll_started = true;
                        start_polling(&sender, self.visible.clone());
                    }
                    sender.input(PodmanMenuWidgetInput::RefreshNow);
                }
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
            PodmanMenuWidgetCommandOutput::Refreshed(state) => {
                if self.state != state {
                    // Drop expansion entries for containers that no longer
                    // exist so the set doesn't grow unbounded.
                    let live: HashSet<String> =
                        state.containers.iter().map(|c| c.id.clone()).collect();
                    self.expanded.borrow_mut().retain(|id| live.contains(id));
                    self.state = state;
                    sync_view(self, &sender);
                }
            }
        }
    }
}

/// Spawn the perpetual poll loop. Started lazily on first reveal; the
/// `podman` probe is gated on `visible`, so while the panel is hidden
/// the loop only does a cheap timer wake — no subprocess spawn.
fn start_polling(sender: &ComponentSender<PodmanMenuWidgetModel>, visible: Arc<AtomicBool>) {
    sender.command(move |out, shutdown| async move {
        let shutdown_fut = shutdown.wait();
        tokio::pin!(shutdown_fut);
        loop {
            tokio::select! {
                () = &mut shutdown_fut => break,
                _ = tokio::time::sleep(REFRESH_INTERVAL) => {}
            }
            if visible.load(Ordering::Relaxed) {
                let s = fetch_panel_state().await;
                let _ = out.send(PodmanMenuWidgetCommandOutput::Refreshed(s));
            }
        }
    });
}

fn sync_view(model: &PodmanMenuWidgetModel, sender: &ComponentSender<PodmanMenuWidgetModel>) {
    let s = &model.state;

    // Header counter — running / total.
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
                .append(&make_container_row(c, sender, &model.expanded));
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

/// `(icon-name, state-css-class)` for a container/pod state — drives the
/// leading status glyph tint (running = primary, paused = warn, stopped
/// = dim, anything error-ish = error).
fn status_glyph(state: &str) -> (&'static str, &'static str) {
    let s = state.to_ascii_lowercase();
    match s.as_str() {
        "running" => ("media-playback-start-symbolic", "running"),
        "paused" => ("media-playback-pause-symbolic", "paused"),
        "dead" | "degraded" => ("dialog-error-symbolic", "error"),
        _ => ("media-playback-stop-symbolic", "stopped"),
    }
}

fn make_container_row(
    c: &ContainerRow,
    sender: &ComponentSender<PodmanMenuWidgetModel>,
    expanded: &Rc<RefCell<HashSet<String>>>,
) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);

    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();

    let is_expanded = expanded.borrow().contains(&c.id);

    // ── Header (clickable; toggles the revealer) ─────────────────
    let header = gtk::Button::new();
    header.add_css_class("podman-row-header");
    header.set_hexpand(true);

    let head_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .build();

    let (glyph, glyph_class) = status_glyph(&c.state);
    let status_icon = gtk::Image::from_icon_name(glyph);
    status_icon.add_css_class("podman-status-icon");
    status_icon.add_css_class(glyph_class);
    status_icon.set_valign(gtk::Align::Center);
    head_box.append(&status_icon);

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
    image_label.add_css_class("dim-label");
    image_label.set_xalign(0.0);
    image_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    texts.append(&image_label);
    head_box.append(&texts);

    let chevron = gtk::Image::from_icon_name(if is_expanded {
        "pan-down-symbolic"
    } else {
        "pan-end-symbolic"
    });
    chevron.add_css_class("podman-chevron");
    chevron.set_valign(gtk::Align::Center);
    head_box.append(&chevron);

    header.set_child(Some(&head_box));
    card.append(&header);

    // ── Revealer with detail ─────────────────────────────────────
    let revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .transition_duration(200)
        .reveal_child(is_expanded)
        .build();

    let detail = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .build();
    detail.add_css_class("podman-detail");

    if !c.status.is_empty() {
        let status_line = gtk::Label::new(Some(&c.status));
        status_line.add_css_class("label-small");
        status_line.add_css_class("dim-label");
        status_line.set_xalign(0.0);
        status_line.set_wrap(true);
        detail.append(&status_line);
    }

    if !c.ports.is_empty() {
        let ports_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .build();
        ports_box.add_css_class("podman-ports");
        for p in &c.ports {
            ports_box.append(&port_chip(p));
        }
        detail.append(&ports_box);
    }

    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .build();
    actions.add_css_class("podman-actions");

    let is_running = c.state.eq_ignore_ascii_case("running");
    let is_paused = c.state.eq_ignore_ascii_case("paused");
    if is_running {
        actions.append(&run_action(
            sender,
            "view-refresh-symbolic",
            "Restart",
            vec!["restart".to_string(), c.id.clone()],
            false,
        ));
        actions.append(&run_action(
            sender,
            "media-playback-pause-symbolic",
            "Pause",
            vec!["pause".to_string(), c.id.clone()],
            false,
        ));
        actions.append(&run_action(
            sender,
            "media-playback-stop-symbolic",
            "Stop",
            vec![
                "stop".to_string(),
                "-t".to_string(),
                "3".to_string(),
                c.id.clone(),
            ],
            false,
        ));
        actions.append(&shell_action(sender, &c.id));
        actions.append(&logs_action(sender, &c.id));
    } else if is_paused {
        actions.append(&run_action(
            sender,
            "media-playback-start-symbolic",
            "Unpause",
            vec!["unpause".to_string(), c.id.clone()],
            false,
        ));
        actions.append(&run_action(
            sender,
            "media-playback-stop-symbolic",
            "Stop",
            vec![
                "stop".to_string(),
                "-t".to_string(),
                "3".to_string(),
                c.id.clone(),
            ],
            false,
        ));
        actions.append(&logs_action(sender, &c.id));
    } else {
        actions.append(&run_action(
            sender,
            "media-playback-start-symbolic",
            "Start",
            vec!["start".to_string(), c.id.clone()],
            false,
        ));
        actions.append(&logs_action(sender, &c.id));
        actions.append(&run_action(
            sender,
            "user-trash-symbolic",
            "Remove",
            vec!["rm".to_string(), "-f".to_string(), c.id.clone()],
            true,
        ));
    }
    detail.append(&actions);
    revealer.set_child(Some(&detail));
    card.append(&revealer);

    // Toggle wiring — flips the shared set + the local widgets directly,
    // so the animation plays and a later rebuild restores the state.
    let exp = expanded.clone();
    let id = c.id.clone();
    let rev = revealer.clone();
    let chev = chevron.clone();
    header.connect_clicked(move |_| {
        let now_expanded = {
            let mut set = exp.borrow_mut();
            if set.contains(&id) {
                set.remove(&id);
                false
            } else {
                set.insert(id.clone());
                true
            }
        };
        rev.set_reveal_child(now_expanded);
        chev.set_icon_name(Some(if now_expanded {
            "pan-down-symbolic"
        } else {
            "pan-end-symbolic"
        }));
    });

    row.set_child(Some(&card));
    row
}

/// One published port mapping rendered as a pill chip: `3000 → 8080`
/// (plus `/udp` when the protocol isn't TCP).
fn port_chip(p: &PortMapping) -> gtk::Label {
    let proto = if p.protocol.eq_ignore_ascii_case("tcp") || p.protocol.is_empty() {
        String::new()
    } else {
        format!("/{}", p.protocol.to_lowercase())
    };
    let text = format!("{} → {}{}", p.host_port, p.container_port, proto);
    let chip = gtk::Label::new(Some(&text));
    chip.add_css_class("podman-port-chip");
    chip.set_valign(gtk::Align::Center);
    chip
}

/// A full-width labelled action button for the expanded detail list.
fn action_row(icon: &str, label: &str, danger: bool) -> gtk::Button {
    let btn = gtk::Button::new();
    btn.add_css_class("podman-action-row");
    if danger {
        btn.add_css_class("danger");
    }
    let inner = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .build();
    let img = gtk::Image::from_icon_name(icon);
    inner.append(&img);
    let lbl = gtk::Label::new(Some(label));
    lbl.set_xalign(0.0);
    lbl.set_hexpand(true);
    inner.append(&lbl);
    btn.set_child(Some(&inner));
    btn
}

fn run_action(
    sender: &ComponentSender<PodmanMenuWidgetModel>,
    icon: &str,
    label: &str,
    args: Vec<String>,
    danger: bool,
) -> gtk::Button {
    let btn = action_row(icon, label, danger);
    let s = sender.clone();
    btn.connect_clicked(move |_| {
        s.input(PodmanMenuWidgetInput::RunPodman(args.clone()));
    });
    btn
}

fn shell_action(sender: &ComponentSender<PodmanMenuWidgetModel>, id: &str) -> gtk::Button {
    let btn = action_row("utilities-terminal-symbolic", "Shell", false);
    let s = sender.clone();
    let id = id.to_string();
    btn.connect_clicked(move |_| {
        s.input(PodmanMenuWidgetInput::OpenShell(id.clone()));
    });
    btn
}

fn logs_action(sender: &ComponentSender<PodmanMenuWidgetModel>, id: &str) -> gtk::Button {
    let btn = action_row("text-x-generic-symbolic", "Logs", false);
    let s = sender.clone();
    let id = id.to_string();
    btn.connect_clicked(move |_| {
        s.input(PodmanMenuWidgetInput::OpenLogs(id.clone()));
    });
    btn
}

fn make_image_row(
    img: &ImageRow,
    sender: &ComponentSender<PodmanMenuWidgetModel>,
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
    size_label.add_css_class("dim-label");
    size_label.set_xalign(0.0);
    texts.append(&size_label);
    outer.append(&texts);

    let target = if img.repository == "<none>" {
        img.id.clone()
    } else {
        title.clone()
    };
    outer.append(&action_button(
        "user-trash-symbolic",
        "Remove",
        sender,
        vec!["rmi".to_string(), "-f".to_string(), target],
    ));

    row.set_child(Some(&outer));
    row
}

fn make_pod_row(p: &PodRow, sender: &ComponentSender<PodmanMenuWidgetModel>) -> gtk::ListBoxRow {
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

    let (glyph, glyph_class) = status_glyph(&p.status);
    let status_icon = gtk::Image::from_icon_name(glyph);
    status_icon.add_css_class("podman-status-icon");
    status_icon.add_css_class(glyph_class);
    status_icon.set_valign(gtk::Align::Center);
    outer.append(&status_icon);

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
    detail.add_css_class("dim-label");
    detail.set_xalign(0.0);
    texts.append(&detail);
    outer.append(&texts);

    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(4)
        .build();
    let is_running = p.status.eq_ignore_ascii_case("running");
    if is_running {
        actions.append(&action_button(
            "media-playback-stop-symbolic",
            "Stop",
            sender,
            vec!["pod".to_string(), "stop".to_string(), p.id.clone()],
        ));
    } else {
        actions.append(&action_button(
            "media-playback-start-symbolic",
            "Start",
            sender,
            vec!["pod".to_string(), "start".to_string(), p.id.clone()],
        ));
    }
    actions.append(&action_button(
        "user-trash-symbolic",
        "Remove",
        sender,
        vec![
            "pod".to_string(),
            "rm".to_string(),
            "-f".to_string(),
            p.id.clone(),
        ],
    ));
    outer.append(&actions);

    row.set_child(Some(&outer));
    row
}

/// Icon-only flat action button (images / pods tabs).
fn action_button(
    icon: &str,
    tooltip: &str,
    sender: &ComponentSender<PodmanMenuWidgetModel>,
    args: Vec<String>,
) -> gtk::Button {
    let btn = gtk::Button::from_icon_name(icon);
    btn.add_css_class("ok-button-flat");
    btn.set_tooltip_text(Some(tooltip));
    let s = sender.clone();
    btn.connect_clicked(move |_| {
        s.input(PodmanMenuWidgetInput::RunPodman(args.clone()));
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

// ── Terminal-backed actions (Shell / Logs) ──────────────────────────

/// The user's terminal: honour `$TERMINAL`, else the first installed of
/// a sensible candidate list (kitty first — the project's default).
fn pick_terminal() -> String {
    if let Some(t) = std::env::var_os("TERMINAL")
        && !t.is_empty()
    {
        return t.to_string_lossy().into_owned();
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    for candidate in ["kitty", "alacritty", "foot", "wezterm", "xterm"] {
        if std::env::split_paths(&path).any(|dir| dir.join(candidate).is_file()) {
            return candidate.to_string();
        }
    }
    "xterm".to_string()
}

fn spawn_terminal(args: Vec<String>) {
    let term = pick_terminal();
    relm4::spawn(async move {
        if let Err(e) = tokio::process::Command::new(&term).args(&args).spawn() {
            warn!(error = %e, term, ?args, "podman: failed to spawn terminal");
        }
    });
}

/// `<term> -e podman exec -it <id> sh -c 'exec bash || sh'` — opens an
/// interactive shell, preferring bash and falling back to sh so it works
/// on minimal images. `id` is passed as its own argv element (no shell
/// quoting needed).
fn open_shell(id: &str) {
    spawn_terminal(vec![
        "-e".to_string(),
        "podman".to_string(),
        "exec".to_string(),
        "-it".to_string(),
        id.to_string(),
        "sh".to_string(),
        "-c".to_string(),
        "command -v bash >/dev/null 2>&1 && exec bash || exec sh".to_string(),
    ]);
}

/// `<term> -e sh -c 'podman logs -f --tail 200 <id>; …; read'` — follows
/// the log and keeps the window open after the stream ends.
fn open_logs(id: &str) {
    let script = format!(
        "podman logs -f --tail 200 {id}; echo; printf '\\n[log ended — press Enter to close] '; read _",
        id = sh_quote(id)
    );
    spawn_terminal(vec![
        "-e".to_string(),
        "sh".to_string(),
        "-c".to_string(),
        script,
    ]);
}

/// Single-quote a string for safe embedding in a `sh -c` script.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

// ── Probes ──────────────────────────────────────────────────────────

/// Probe all three lists + the summary. Each sub-probe is independent —
/// missing tools surface as empty lists rather than failing the panel.
async fn fetch_panel_state() -> PodmanPanelState {
    let summary = fetch_podman_summary().await;
    let mut state = PodmanPanelState {
        summary: summary.clone(),
        ..PodmanPanelState::default()
    };
    if summary.error.is_some() {
        return state;
    }

    if let Some(json) = run_capture("podman", &["ps", "--all", "--format", "json"]).await
        && let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&json)
    {
        state.containers = arr.iter().map(parse_container_row).collect();
        // Running first, then paused, then the rest; ties broken by name.
        state.containers.sort_by(|a, b| {
            state_rank(&a.state)
                .cmp(&state_rank(&b.state))
                .then_with(|| a.name.cmp(&b.name))
        });
    }
    if let Some(json) = run_capture("podman", &["images", "--format", "json"]).await
        && let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&json)
    {
        state.images = arr.iter().map(parse_image_row).collect();
    }
    if let Some(json) = run_capture("podman", &["pod", "ps", "--format", "json"]).await
        && let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&json)
    {
        state.pods = arr.iter().map(parse_pod_row).collect();
    }

    state
}

fn state_rank(state: &str) -> u8 {
    if state.eq_ignore_ascii_case("running") {
        0
    } else if state.eq_ignore_ascii_case("paused") {
        1
    } else {
        2
    }
}

/// `podman ps` reports `host_port` / `container_port` as JSON numbers;
/// accept either a number or a string.
fn port_field(v: Option<&serde_json::Value>) -> String {
    match v {
        Some(serde_json::Value::Number(n)) => n.to_string(),
        Some(serde_json::Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

fn parse_ports(v: Option<&serde_json::Value>) -> Vec<PortMapping> {
    let Some(arr) = v.and_then(|x| x.as_array()) else {
        return Vec::new();
    };
    let mut out: Vec<PortMapping> = Vec::new();
    for p in arr {
        let host_port = port_field(p.get("host_port"));
        // Skip unpublished ports (no host binding).
        if host_port.is_empty() || host_port == "0" {
            continue;
        }
        let mapping = PortMapping {
            host_port,
            container_port: port_field(p.get("container_port")),
            protocol: p
                .get("protocol")
                .and_then(|x| x.as_str())
                .unwrap_or("tcp")
                .to_string(),
        };
        if !out.contains(&mapping) {
            out.push(mapping);
        }
    }
    out
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
        status: v
            .get("Status")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string(),
        ports: parse_ports(v.get("Ports")),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_published_ports_and_skips_unpublished() {
        let v = serde_json::json!({
            "Id": "abc",
            "Names": ["web"],
            "Image": "nginx",
            "State": "running",
            "Status": "Up 2 hours",
            "Ports": [
                { "host_ip": "", "container_port": 8080, "host_port": 3000, "protocol": "tcp" },
                { "host_ip": "", "container_port": 9000, "host_port": 0, "protocol": "tcp" },
                { "host_ip": "", "container_port": 53, "host_port": 5353, "protocol": "udp" }
            ]
        });
        let row = parse_container_row(&v);
        assert_eq!(row.name, "web");
        assert_eq!(row.status, "Up 2 hours");
        assert_eq!(row.ports.len(), 2);
        assert_eq!(row.ports[0].host_port, "3000");
        assert_eq!(row.ports[0].container_port, "8080");
        assert_eq!(row.ports[1].protocol, "udp");
    }

    #[test]
    fn state_rank_orders_running_first() {
        assert!(state_rank("running") < state_rank("paused"));
        assert!(state_rank("paused") < state_rank("exited"));
        assert!(state_rank("Running") < state_rank("created"));
    }

    #[test]
    fn sh_quote_escapes_single_quotes() {
        assert_eq!(sh_quote("abc"), "'abc'");
        assert_eq!(sh_quote("a'b"), "'a'\\''b'");
    }
}
