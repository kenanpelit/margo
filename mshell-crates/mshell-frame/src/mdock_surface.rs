//! Standalone **mdock** surface — a per-output layer-shell window hosting the
//! dock strip (`MargoDockModel`) inside a `Revealer`. Behaviour (Always /
//! AutoHide / Toggle) + edge come from the `dock` config. Port of hydock's
//! standalone dock onto margo IPC.
//!
//! hydock (https://github.com/desyatkoff/hydock) © Sergey Desyatkov, GPL-3.0 —
//! same licence as margo.

use crate::bars::bar::BarType;
use crate::bars::bar_widgets::margo_dock::{MargoDockInit, MargoDockModel, MargoDockOutput};
use crate::bars::bar_widgets::mdock_layout::{
    edge_for, orientation_for, reserves_exclusive_zone, uses_edge_trigger,
};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, DockBehavior, DockPosition, DockStyle};
use reactive_graph::traits::GetUntracked;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};

pub struct MdockSurface {
    revealer: gtk::Revealer,
    window: gtk::Window,
    /// The auto-hide edge trigger (only present in AutoHide behaviour) — kept
    /// alive for the surface's lifetime.
    _trigger: Option<gtk::Window>,
    _dock: Controller<MargoDockModel>,
}

#[derive(Debug)]
pub enum MdockSurfaceInput {
    Show,
    Hide,
    Toggle,
    /// The dock's launcher button was clicked — open the app launcher.
    LauncherClicked,
}

pub struct MdockSurfaceInit {
    /// Output to pin the dock to (None = let the compositor place it).
    pub monitor: Option<gtk::gdk::Monitor>,
}

fn bar_type_for(p: DockPosition) -> BarType {
    match p {
        DockPosition::Top => BarType::Top,
        _ => BarType::Bottom,
    }
}

/// Align the popup card against its `position` edge (centred on the cross
/// axis), like a menu anchored to that side.
fn align_popup(revealer: &gtk::Revealer, position: DockPosition) {
    use gtk::Align::{Center, End, Start};
    let m = 8;
    match position {
        DockPosition::Left => {
            revealer.set_halign(Start);
            revealer.set_valign(Center);
            revealer.set_margin_start(m);
        }
        DockPosition::Right => {
            revealer.set_halign(End);
            revealer.set_valign(Center);
            revealer.set_margin_end(m);
        }
        DockPosition::Top => {
            revealer.set_halign(Center);
            revealer.set_valign(Start);
            revealer.set_margin_top(m);
        }
        DockPosition::Bottom => {
            revealer.set_halign(Center);
            revealer.set_valign(End);
            revealer.set_margin_bottom(m);
        }
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

        let popup = matches!(cfg.style, DockStyle::Popup);

        // The dock strip — the SAME component the bar pill embeds. Forward its
        // launcher-button output so the button actually opens the launcher.
        let dock = MargoDockModel::builder()
            .launch(MargoDockInit {
                orientation,
                bar_type: bar_type_for(cfg.position),
            })
            .forward(sender.input_sender(), |msg| match msg {
                MargoDockOutput::AppLauncherClicked => MdockSurfaceInput::LauncherClicked,
            });

        // No slide — instant show/hide (the slide read poorly). The card just
        // appears/disappears; for a popup it behaves like the session menu.
        let revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::None)
            .child(dock.widget())
            .build();
        revealer.add_css_class("mdock-surface");

        // Unmap the window once hidden so it stops capturing input. An always-on
        // layer-shell dock stays mapped; everything else unmaps when concealed.
        let keep_mapped = !popup && matches!(cfg.behavior, DockBehavior::Always);
        {
            let window = root.clone();
            let rev = revealer.clone();
            revealer.connect_child_revealed_notify(move |_| {
                if !rev.is_child_revealed() && !keep_mapped {
                    window.set_visible(false);
                }
            });
        }

        // `all: unset` on `.mdock-window` strips the default opaque window
        // background so only the rounded surface shows.
        root.add_css_class("mdock-window");
        root.init_layer_shell();
        if let Some(m) = &params.monitor {
            root.set_monitor(Some(m));
        }
        root.set_namespace(Some("mdock"));
        root.set_layer(Layer::Top);
        root.set_decorated(false);
        root.set_child(Some(&revealer));

        let mut trigger = None;

        if popup {
            // Session-menu-style popup: a full-screen transparent surface with
            // the card anchored to its `position` edge. Grabs the keyboard so
            // Esc closes it; clicking outside the card closes it too. Opens on
            // `mshellctl dock toggle`.
            root.set_keyboard_mode(KeyboardMode::Exclusive);
            for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
                root.set_anchor(e, true);
            }
            root.set_exclusive_zone(0);
            align_popup(&revealer, cfg.position);

            // Esc → close.
            let key = gtk::EventControllerKey::new();
            let s = sender.clone();
            key.connect_key_pressed(move |_, k, _, _| {
                if k == gtk::gdk::Key::Escape {
                    s.input(MdockSurfaceInput::Hide);
                    gtk::glib::Propagation::Stop
                } else {
                    gtk::glib::Propagation::Proceed
                }
            });
            root.add_controller(key);

            // Click outside the card → close.
            let click = gtk::GestureClick::new();
            let s = sender.clone();
            let card = revealer.clone();
            let win = root.clone();
            click.connect_pressed(move |_, _, x, y| {
                let inside = card
                    .compute_bounds(&win)
                    .map(|b| b.contains_point(&gtk::graphene::Point::new(x as f32, y as f32)))
                    .unwrap_or(false);
                if !inside {
                    s.input(MdockSurfaceInput::Hide);
                }
            });
            root.add_controller(click);

            root.set_visible(false);
        } else {
            // Edge layer-shell dock.
            root.set_keyboard_mode(KeyboardMode::None);
            // Centre the card on the anchored edge.
            revealer.set_halign(gtk::Align::Center);
            revealer.set_valign(gtk::Align::Center);
            root.set_anchor(edge, true);
            if reserves_exclusive_zone(cfg.behavior) {
                root.auto_exclusive_zone_enable();
            } else {
                root.set_exclusive_zone(0);
            }

            trigger = uses_edge_trigger(cfg.behavior)
                .then(|| build_trigger(&params.monitor, edge, orientation, &sender, &root));

            match cfg.behavior {
                DockBehavior::Always => {
                    root.set_visible(true);
                    revealer.set_reveal_child(true);
                }
                DockBehavior::AutoHide | DockBehavior::Toggle => {
                    root.set_visible(false);
                }
            }
        }

        ComponentParts {
            model: MdockSurface {
                revealer,
                window: root,
                _trigger: trigger,
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
            MdockSurfaceInput::LauncherClicked => {
                open_launcher();
                return;
            }
        };
        if show {
            self.window.set_visible(true);
            self.revealer.set_reveal_child(true);
        } else {
            self.revealer.set_reveal_child(false);
        }
    }
}

/// Open the app launcher from the dock's launcher button: run the configured
/// `launcher_command` if set, else toggle the shell's app-launcher menu.
fn open_launcher() {
    let cmd = config_manager()
        .config()
        .dock()
        .get_untracked()
        .launcher_command;
    let cmd = if cmd.trim().is_empty() {
        "mshellctl menu app-launcher".to_string()
    } else {
        cmd
    };
    let _ = std::process::Command::new("sh").arg("-c").arg(cmd).spawn();
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
