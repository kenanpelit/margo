//! Keep Awake menu widget — the panel content for
//! `MenuType::KeepAwake`. Ports the noctalia `keep-awake-plus` panel:
//! a status line, a duration grid (15m / 30m / 1h / 2h / 4h / 8h /
//! 24h / ∞), and
//! quick-extend + turn-off once a session is running. The timed
//! inhibit lives in [`crate::keep_awake::KeepAwakeSession`].

use crate::keep_awake::{KeepAwakeSession, format_remaining};
use mshell_idle::inhibitor::IdleInhibitor;
use relm4::gtk::prelude::{BoxExt, ButtonExt, FlowBoxChildExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Preset durations (minutes, label). `None` minutes (the ∞ tile) is
/// handled separately.
const PRESETS: &[(u64, &str)] = &[
    (15, "15m"),
    (30, "30m"),
    (60, "1h"),
    (120, "2h"),
    (240, "4h"),
    (480, "8h"),
    (1440, "24h"),
];
/// The `+Xm` quick-extend step.
const EXTEND_MINUTES: u64 = 30;

pub(crate) struct KeepAwakeMenuWidgetModel {
    status_label: gtk::Label,
    extend_button: gtk::Button,
    off_button: gtk::Button,
    /// The 1 s countdown heartbeat is started lazily on the first reveal
    /// (this menu is built eagerly per-monitor and embedded as a Control
    /// Center detail page) and paused while hidden.
    tick_started: bool,
    revealed: Arc<AtomicBool>,
}

#[derive(Debug)]
pub(crate) enum KeepAwakeMenuWidgetInput {
    /// Panel shown / hidden — starts the countdown heartbeat lazily on
    /// first reveal and pauses it while hidden.
    ParentRevealChanged(bool),
    /// Activate for N minutes (`None` = unlimited).
    Activate(Option<u64>),
    Extend,
    Off,
}

#[derive(Debug)]
pub(crate) enum KeepAwakeMenuWidgetOutput {
    CloseMenu,
}

pub(crate) struct KeepAwakeMenuWidgetInit {}

#[relm4::component(pub(crate))]
impl Component for KeepAwakeMenuWidgetModel {
    type CommandOutput = ();
    type Input = KeepAwakeMenuWidgetInput;
    type Output = KeepAwakeMenuWidgetOutput;
    type Init = KeepAwakeMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "keep-awake-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 12,

            // ── §12 panel header ──
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,
                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("eye-symbolic"),
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_hexpand: true,
                    gtk::Label {
                        add_css_class: "panel-title",
                        set_halign: gtk::Align::Start,
                        set_label: "Keep Awake",
                    },
                    #[local_ref]
                    status_label_widget -> gtk::Label {
                        add_css_class: "label-small",
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                    },
                },
            },

            // Duration grid.
            #[name = "grid"]
            gtk::FlowBox {
                add_css_class: "keep-awake-grid",
                set_selection_mode: gtk::SelectionMode::None,
                set_homogeneous: true,
                set_min_children_per_line: 3,
                set_max_children_per_line: 4,
                set_row_spacing: 6,
                set_column_spacing: 6,
            },

            // Running-session controls.
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,
                set_homogeneous: true,
                #[local_ref]
                extend_button_widget -> gtk::Button {
                    add_css_class: "ok-button-surface",
                    add_css_class: "ok-button-cell",
                    set_label: "+30m",
                    set_tooltip_text: Some("Extend the running session"),
                    connect_clicked[sender] => move |_| {
                        sender.input(KeepAwakeMenuWidgetInput::Extend);
                    },
                },
                #[local_ref]
                off_button_widget -> gtk::Button {
                    add_css_class: "ok-button-surface",
                    add_css_class: "ok-button-cell",
                    set_label: "Turn off",
                    connect_clicked[sender] => move |_| {
                        sender.input(KeepAwakeMenuWidgetInput::Off);
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
        let status_label_widget = gtk::Label::new(None);
        let extend_button_widget = gtk::Button::new();
        let off_button_widget = gtk::Button::new();

        let model = KeepAwakeMenuWidgetModel {
            status_label: status_label_widget.clone(),
            extend_button: extend_button_widget.clone(),
            off_button: off_button_widget.clone(),
            tick_started: false,
            revealed: Arc::new(AtomicBool::new(false)),
        };
        let widgets = view_output!();

        // Build the duration tiles (presets + ∞).
        for (mins, label) in PRESETS {
            widgets.grid.insert(&duration_tile(label, Some(*mins), &sender), -1);
        }
        widgets.grid.insert(&duration_tile("∞", None, &sender), -1);

        sync(&model);

        // The 1 s countdown heartbeat is started lazily on the first
        // reveal (see `ensure_polling`).

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            KeepAwakeMenuWidgetInput::ParentRevealChanged(revealed) => {
                self.revealed.store(revealed, Ordering::Relaxed);
                if revealed {
                    self.ensure_polling(&sender);
                    // Resync the countdown immediately on open.
                    sync(self);
                }
            }
            KeepAwakeMenuWidgetInput::Activate(minutes) => {
                KeepAwakeSession::global().activate(minutes);
                let _ = sender.output(KeepAwakeMenuWidgetOutput::CloseMenu);
            }
            KeepAwakeMenuWidgetInput::Extend => {
                KeepAwakeSession::global().extend(EXTEND_MINUTES);
                sync(self);
            }
            KeepAwakeMenuWidgetInput::Off => {
                KeepAwakeSession::global().deactivate();
                let _ = sender.output(KeepAwakeMenuWidgetOutput::CloseMenu);
            }
        }
    }

    fn update_cmd(
        &mut self,
        _message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        sync(self);
    }
}

impl KeepAwakeMenuWidgetModel {
    /// Start the 1 s countdown heartbeat once (on first reveal). The loop
    /// runs until the widget is dropped but only emits a tick while the
    /// panel is revealed, so a closed (or off-screen, per-monitor) panel
    /// doesn't refresh its countdown for nothing.
    fn ensure_polling(&mut self, sender: &ComponentSender<Self>) {
        if self.tick_started {
            return;
        }
        self.tick_started = true;
        let revealed = self.revealed.clone();
        sender.command(move |out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            loop {
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    _ = tick.tick() => {
                        if revealed.load(Ordering::Relaxed) {
                            let _ = out.send(());
                        }
                    }
                }
            }
        });
    }
}

/// Refresh the status line + running-session controls from the live
/// inhibitor + session state.
fn sync(model: &KeepAwakeMenuWidgetModel) {
    let active = IdleInhibitor::global().get();
    let remaining = KeepAwakeSession::global().remaining();

    let status = if !active {
        "Off — your screen sleeps normally".to_string()
    } else if let Some(left) = remaining {
        format!("Active · {} left", format_remaining(left))
    } else {
        "Active · no time limit".to_string()
    };
    model.status_label.set_label(&status);

    // Extend only makes sense for a timed session; off whenever active.
    model.extend_button.set_visible(active && remaining.is_some());
    model.off_button.set_visible(active);
}

/// One duration tile button wired to `Activate`.
fn duration_tile(
    label: &str,
    minutes: Option<u64>,
    sender: &ComponentSender<KeepAwakeMenuWidgetModel>,
) -> gtk::FlowBoxChild {
    let btn = gtk::Button::with_label(label);
    btn.add_css_class("keep-awake-tile");
    btn.set_hexpand(true);
    {
        let sender = sender.clone();
        btn.connect_clicked(move |_| {
            sender.input(KeepAwakeMenuWidgetInput::Activate(minutes));
        });
    }
    let child = gtk::FlowBoxChild::new();
    child.set_child(Some(&btn));
    child.set_focusable(false);
    child
}
