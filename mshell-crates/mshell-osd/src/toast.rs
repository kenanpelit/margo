//! Transient state-change toast surface + the in-process toast bus.
//!
//! A toast is a small **opaque** corner card (DESIGN.md §20 — toasts stay
//! opaque regardless of the surface-opacity knob) that announces a system
//! *state change* — AC power, lock keys, keyboard layout, default audio
//! device, VPN, now-playing — and the battery warning ladder. It is distinct
//! from the notification popups (app notifications) and the volume/brightness
//! OSD (a value pulse).
//!
//! Architecture: a single central producer ([`crate::toast_producer`])
//! subscribes to every event source *once* and calls [`push_toast`], which
//! fans the event out over a process-wide broadcast channel. Each output then
//! owns one [`ToastSurfaceModel`] that mirrors the OSD pattern (one window per
//! monitor, show on event, auto-hide after a few seconds). Centralising the
//! producer keeps the subprocess pollers (VPN, lock-key sysfs reads) running
//! once total instead of once per monitor.

use gtk4::gdk;
use gtk4::prelude::{BoxExt, GtkWindowExt, OrientableExt, WidgetExt};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use mshell_common::WatcherToken;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::sync::OnceLock;
use tokio::sync::broadcast;

/// Severity tint for a toast — maps to the DESIGN.md §2 calm/warn/danger
/// ladder (plus the stable `positive` green) via a CSS class on the card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastSeverity {
    Calm,
    Warn,
    Danger,
    Positive,
}

impl ToastSeverity {
    fn class(self) -> &'static str {
        match self {
            ToastSeverity::Calm => "calm",
            ToastSeverity::Warn => "warn",
            ToastSeverity::Danger => "danger",
            ToastSeverity::Positive => "positive",
        }
    }
}

/// One toast: a symbolic icon, a title, an optional body line, and a severity.
#[derive(Debug, Clone)]
pub struct ToastEvent {
    pub icon: String,
    pub title: String,
    pub body: Option<String>,
    pub severity: ToastSeverity,
}

static TOAST_BUS: OnceLock<broadcast::Sender<ToastEvent>> = OnceLock::new();

fn bus() -> &'static broadcast::Sender<ToastEvent> {
    TOAST_BUS.get_or_init(|| broadcast::channel(16).0)
}

/// Broadcast a toast to every per-output toast surface. A no-op (the `Err`
/// when there are no live receivers is intentionally swallowed) before the
/// surfaces exist or after they're all gone.
pub fn push_toast(event: ToastEvent) {
    let _ = bus().send(event);
}

/// Subscribe a per-output surface to the toast bus.
fn subscribe() -> broadcast::Receiver<ToastEvent> {
    bus().subscribe()
}

#[derive(Debug)]
pub struct ToastSurfaceModel {
    icon: String,
    title: String,
    body: String,
    has_body: bool,
    severity_class: &'static str,
    hide_token: WatcherToken,
}

#[derive(Debug)]
pub enum ToastSurfaceInput {
    Show(ToastEvent),
    Hide,
}

#[derive(Debug)]
pub enum ToastSurfaceOutput {}

pub struct ToastSurfaceInit {
    pub monitor: gdk::Monitor,
}

#[derive(Debug)]
pub enum ToastSurfaceCommandOutput {
    Event(ToastEvent),
    Hide,
}

/// On-screen dwell time before a toast auto-dismisses.
const TOAST_DURATION: std::time::Duration = std::time::Duration::from_millis(3500);

#[relm4::component(pub)]
impl Component for ToastSurfaceModel {
    type CommandOutput = ToastSurfaceCommandOutput;
    type Input = ToastSurfaceInput;
    type Output = ToastSurfaceOutput;
    type Init = ToastSurfaceInit;

    view! {
        #[root]
        gtk::Window {
            set_css_classes: &["toast-window"],
            set_decorated: false,
            set_visible: false,
            set_default_height: 1,

            gtk::Box {
                #[watch]
                set_css_classes: &["toast-card", model.severity_class],
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,

                gtk::Image {
                    add_css_class: "toast-icon",
                    #[watch]
                    set_icon_name: Some(model.icon.as_str()),
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_valign: gtk::Align::Center,
                    set_spacing: 2,

                    gtk::Label {
                        add_css_class: "toast-title",
                        set_xalign: 0.0,
                        #[watch]
                        set_label: &model.title,
                    },

                    gtk::Label {
                        add_css_class: "toast-body",
                        set_xalign: 0.0,
                        set_wrap: true,
                        #[watch]
                        set_visible: model.has_body,
                        #[watch]
                        set_label: &model.body,
                    },
                },
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        root.init_layer_shell();
        root.set_monitor(Some(&params.monitor));
        root.set_namespace(Some("mshell-toast"));
        root.set_layer(Layer::Overlay);
        root.set_exclusive_zone(0);
        // Top-centre, clear of a typical top bar. Anchoring only the top edge
        // leaves the surface horizontally centred by the compositor.
        root.set_anchor(Edge::Top, true);
        root.set_margin(Edge::Top, 56);

        // Mirror the bus into this surface's command stream.
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut rx = subscribe();
            loop {
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    event = rx.recv() => match event {
                        Ok(event) => {
                            let _ = out.send(ToastSurfaceCommandOutput::Event(event));
                        }
                        // Dropped some events under a burst — fine, just keep going.
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        });

        let model = ToastSurfaceModel {
            icon: String::new(),
            title: String::new(),
            body: String::new(),
            has_body: false,
            severity_class: ToastSeverity::Calm.class(),
            hide_token: WatcherToken::new(),
        };

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            ToastSurfaceInput::Show(event) => {
                self.icon = event.icon;
                self.title = event.title;
                self.has_body = event.body.is_some();
                self.body = event.body.unwrap_or_default();
                self.severity_class = event.severity.class();
                root.set_visible(true);

                // Reset the auto-hide timer: a newer toast replacing this one
                // restarts the dwell rather than inheriting the old deadline.
                let token = self.hide_token.reset();
                sender.command(|out, shutdown| {
                    shutdown
                        .register(async move {
                            tokio::time::sleep(TOAST_DURATION).await;
                            if !token.is_cancelled() {
                                out.send(ToastSurfaceCommandOutput::Hide).ok();
                            }
                        })
                        .drop_on_shutdown()
                });
            }
            ToastSurfaceInput::Hide => {
                root.set_visible(false);
            }
        }

        self.update_view(widgets, sender);
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            ToastSurfaceCommandOutput::Event(event) => {
                sender.input(ToastSurfaceInput::Show(event));
            }
            ToastSurfaceCommandOutput::Hide => {
                sender.input(ToastSurfaceInput::Hide);
            }
        }
    }
}
