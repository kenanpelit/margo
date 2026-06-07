//! Standalone **mdock** surface — a per-output layer-shell window hosting the
//! dock strip (`MargoDockModel`) inside a `Revealer`. Behaviour (Always /
//! AutoHide / Toggle) + edge come from the `dock` config. Port of hydock's
//! standalone dock onto margo IPC.
//!
//! hydock (https://github.com/desyatkoff/hydock) © Sergey Desyatkov, GPL-3.0 —
//! same licence as margo.

use crate::bars::bar::BarType;
use crate::bars::bar_widgets::margo_dock::{MargoDockInit, MargoDockModel};
use crate::bars::bar_widgets::mdock_layout::{
    edge_for, orientation_for, reserves_exclusive_zone, uses_edge_trigger,
};
use gtk4_layer_shell::{Layer, LayerShell};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, DockBehavior, DockPosition};
use reactive_graph::traits::GetUntracked;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};

pub struct MdockSurface {
    revealer: gtk::Revealer,
    window: gtk::Window,
    /// The auto-hide edge trigger (only present in AutoHide behaviour) — kept
    /// alive for the surface's lifetime.
    _trigger: Option<gtk::Window>,
    behavior: DockBehavior,
    _dock: Controller<MargoDockModel>,
}

#[derive(Debug)]
pub enum MdockSurfaceInput {
    Show,
    Hide,
    Toggle,
}

pub struct MdockSurfaceInit {
    /// Output to pin the dock to (None = let the compositor place it).
    pub monitor: Option<gtk::gdk::Monitor>,
}

/// Slide animation duration, ms — matches the bar toggle's smooth feel.
const SLIDE_MS: u32 = 300;

fn bar_type_for(p: DockPosition) -> BarType {
    match p {
        DockPosition::Top => BarType::Top,
        _ => BarType::Bottom,
    }
}

impl Component for MdockSurface {
    type CommandOutput = ();
    type Input = MdockSurfaceInput;
    type Output = ();
    type Init = MdockSurfaceInit;
    type Root = gtk::Window;
    type Widgets = ();

    fn init_root() -> Self::Root {
        gtk::Window::new()
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let cfg = config_manager().config().dock().get_untracked();
        let edge = edge_for(cfg.position);
        let orientation = orientation_for(cfg.position);

        // The dock strip — the SAME component the bar pill embeds.
        let dock = MargoDockModel::builder()
            .launch(MargoDockInit {
                orientation,
                bar_type: bar_type_for(cfg.position),
            })
            .detach();

        let revealer = gtk::Revealer::builder()
            // Slide in/out from the anchored edge (like the bar toggle) for a
            // smooth reveal both ways — not a snap.
            .transition_type(transition_for(cfg.position))
            .transition_duration(SLIDE_MS)
            // Centre the card on the anchored edge so the dock doesn't stretch
            // to fill the (possibly full-length) layer-shell window.
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .child(dock.widget())
            .build();
        revealer.add_css_class("mdock-surface");

        // Keep the window mapped during the slide-OUT, then unmap it only after
        // the revealer animation finishes — otherwise the window vanishes
        // before the slide plays (the snap the user saw). Always-on docks never
        // unmap. This is what makes `mshellctl dock toggle` smooth like the bar.
        {
            let window = root.clone();
            let behavior = cfg.behavior;
            let rev = revealer.clone();
            revealer.connect_child_revealed_notify(move |_| {
                if !rev.is_child_revealed() && !matches!(behavior, DockBehavior::Always) {
                    window.set_visible(false);
                }
            });
        }

        // Layer-shell window setup. `all: unset` on `.mdock-window` strips the
        // default opaque window background so only the rounded surface shows
        // (same trick as the notification-popup window).
        root.add_css_class("mdock-window");
        root.init_layer_shell();
        if let Some(m) = &params.monitor {
            root.set_monitor(Some(m));
        }
        root.set_namespace(Some("mdock"));
        root.set_layer(Layer::Top);
        root.set_decorated(false);
        root.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::None);
        root.set_anchor(edge, true);
        if reserves_exclusive_zone(cfg.behavior) {
            root.auto_exclusive_zone_enable();
        } else {
            root.set_exclusive_zone(0);
        }
        root.set_child(Some(&revealer));

        // Auto-hide edge trigger reveals the dock on pointer-enter.
        let trigger = if uses_edge_trigger(cfg.behavior) {
            Some(build_trigger(
                &params.monitor,
                edge,
                orientation,
                &sender,
                &root,
            ))
        } else {
            None
        };

        // Initial visibility per behaviour.
        match cfg.behavior {
            DockBehavior::Always => {
                root.set_visible(true);
                revealer.set_reveal_child(true);
            }
            DockBehavior::AutoHide | DockBehavior::Toggle => {
                root.set_visible(false);
                revealer.set_reveal_child(false);
            }
        }

        ComponentParts {
            model: MdockSurface {
                revealer,
                window: root,
                _trigger: trigger,
                behavior: cfg.behavior,
                _dock: dock,
            },
            widgets: (),
        }
    }

    fn update(&mut self, msg: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        let show = match msg {
            MdockSurfaceInput::Show => true,
            MdockSurfaceInput::Hide => false,
            MdockSurfaceInput::Toggle => !self.revealer.reveals_child(),
        };
        if show {
            // Map first, then slide in.
            self.window.set_visible(true);
            self.revealer.set_reveal_child(true);
        } else {
            // Slide out; the `child_revealed` handler unmaps the window once
            // the animation finishes (smooth hide, not a snap).
            self.revealer.set_reveal_child(false);
        }
    }
}

/// Revealer slide direction so the dock slides in from its anchored edge.
fn transition_for(p: DockPosition) -> gtk::RevealerTransitionType {
    use gtk::RevealerTransitionType as T;
    match p {
        DockPosition::Bottom => T::SlideUp,
        DockPosition::Top => T::SlideDown,
        DockPosition::Left => T::SlideRight,
        DockPosition::Right => T::SlideLeft,
    }
}

/// Build the 1px auto-hide trigger strip along `edge`. Pointer-enter reveals
/// the dock; a `leave` controller on the dock window hides it again.
fn build_trigger(
    monitor: &Option<gtk::gdk::Monitor>,
    edge: gtk4_layer_shell::Edge,
    orientation: gtk::Orientation,
    sender: &ComponentSender<MdockSurface>,
    dock_window: &gtk::Window,
) -> gtk::Window {
    let trigger = gtk::Window::new();
    trigger.init_layer_shell();
    if let Some(m) = monitor {
        trigger.set_monitor(Some(m));
    }
    trigger.set_namespace(Some("mdock-trigger"));
    trigger.set_layer(Layer::Top);
    trigger.set_decorated(false);
    trigger.set_exclusive_zone(0);
    trigger.set_anchor(edge, true);
    match orientation {
        gtk::Orientation::Horizontal => trigger.set_default_height(1),
        _ => trigger.set_default_width(1),
    }

    let enter = gtk::EventControllerMotion::new();
    let s = sender.clone();
    enter.connect_enter(move |_, _, _| s.input(MdockSurfaceInput::Show));
    trigger.add_controller(enter);

    let leave = gtk::EventControllerMotion::new();
    let s2 = sender.clone();
    leave.connect_leave(move |_| s2.input(MdockSurfaceInput::Hide));
    dock_window.add_controller(leave);

    trigger.set_visible(true);
    trigger
}
