//! Bar window builder.
//!
//! Each output gets its own `ApplicationWindow` anchored to the top
//! edge via `gtk4-layer-shell`. Inside, a `GtkCenterBox` splits the
//! bar into three flex regions — `left` (workspaces / window title),
//! `center` (media player), `right` (system indicators + clock) —
//! mirroring the saimoom/eww `bar_1` widget's halign-driven layout.
//!
//! Stage 1 keeps the regions as empty `GtkBox`es with placeholder
//! labels so the layout, paddings and CSS hooks can be verified
//! before any real modules land. Subsequent stages populate each
//! region in turn.

use gtk::gdk;
use gtk::prelude::*;
use gtk::{Align, ApplicationWindow, Box as GtkBox, CenterBox, Label, Orientation, Widget};
use gtk4_layer_shell::{Edge, Layer, LayerShell};

use crate::modules;

/// Pinned bar height — keep in lockstep with `#bar { min-height /
/// max-height }` in style.css.
const BAR_HEIGHT: i32 = 32;

/// Vertical slot every child widget gets clamped to. Slightly
/// shorter than `BAR_HEIGHT` so internal padding has room without
/// pushing the surface up.
const CHILD_HEIGHT: i32 = 24;

/// Walk a widget tree and clamp every descendant to a sub-bar
/// height + center alignment. GTK4 propagates child size-requests
/// up the tree by default; without this clamp a single child's
/// hover-induced size bump (e.g. GtkButton focus ring, GtkScale
/// trough on focus) is enough to push the bar window allocation
/// past BAR_HEIGHT for one frame — visible to the user as a
/// 3-4× growth pulse.
///
/// Popovers are excluded: their `Widget::is_visible()` is false
/// when collapsed and they aren't laid out inline anyway.
fn clamp_height_recursively(w: &Widget) {
    // Don't touch GtkPopover-rooted subtrees — they're floating
    // children, not laid out inline.
    let class_name = w.css_name();
    if class_name.as_str() == "popover" {
        return;
    }

    w.set_valign(Align::Center);
    w.set_vexpand(false);
    // Set both min/max via size_request. -1 keeps width unconstrained.
    w.set_size_request(-1, CHILD_HEIGHT);

    let mut child = w.first_child();
    while let Some(c) = child {
        clamp_height_recursively(&c);
        child = c.next_sibling();
    }
}

/// Vertical "|" separator label between module groups, ported from
/// eww's `(sep)` widget.
fn sep() -> Label {
    let lbl = Label::new(Some("|"));
    lbl.add_css_class("module-sep");
    lbl
}

/// Construct + present a bar window pinned to `monitor`. Returns the
/// window so the caller (or future hot-reload code) can keep it
/// alive.
pub fn build(app: &gtk::Application, monitor: &gdk::Monitor) -> ApplicationWindow {
    let window = ApplicationWindow::builder()
        .application(app)
        .name("bar")
        .build();

    // wlr-layer-shell anchoring: top edge, span the full output
    // width, request an exclusive zone so tiled clients aren't
    // drawn behind the bar.
    window.init_layer_shell();
    window.set_layer(Layer::Top);
    window.set_namespace(Some("mshell"));
    window.set_monitor(Some(monitor));
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Left, true);
    window.set_anchor(Edge::Right, true);
    // Match noctalia `bar.marginVertical / marginHorizontal = 4`.
    window.set_margin(Edge::Top, 4);
    window.set_margin(Edge::Left, 4);
    window.set_margin(Edge::Right, 4);
    // PIN the bar height. `auto_exclusive_zone_enable` lets wlroots
    // recompute the zone from the window's current allocation —
    // which means any hover-induced size bump (button focus rings,
    // GtkPopover anchoring etc.) momentarily inflates the zone and
    // the user sees the bar grow 3-4× before snapping back. Set an
    // explicit zone + size_request so the surface height is locked
    // regardless of what the children do.
    window.set_exclusive_zone(BAR_HEIGHT + 8); // + marginTop
    window.set_size_request(0, BAR_HEIGHT);
    window.set_default_size(0, BAR_HEIGHT);

    let layout = build_layout();
    // Pin layout height. GTK4 propagates child size requests up
    // the tree; constraining both ends keeps any one widget's
    // hover-induced bump from bleeding into the window allocation.
    layout.set_size_request(-1, BAR_HEIGHT);
    layout.set_valign(Align::Center);
    layout.set_vexpand(false);
    window.set_child(Some(&layout));

    // Window itself: disable any toplevel resize affordance the
    // GTK side might honour. Layer-shell controls the actual
    // surface size, but `set_resizable(false)` keeps GTK from
    // re-requesting it on focus changes.
    window.set_resizable(false);
    window.set_default_size(0, BAR_HEIGHT);

    // Walk the tree once after the layout is built to clamp every
    // descendant's vertical request. Run after `set_child` so the
    // walk hits the just-attached widgets too.
    clamp_height_recursively(layout.upcast_ref::<Widget>());

    window.present();
    window
}

/// The three-region bar body. A `GtkCenterBox` is the natural fit:
/// `start_widget` always hugs the left edge, `end_widget` the right,
/// `center_widget` the middle regardless of how wide the side
/// regions grow. eww's `bar_1` widget uses three `halign`-tagged
/// boxes inside a horizontal `box` to get the same effect; this is
/// the GTK idiom for it.
fn build_layout() -> CenterBox {
    let layout = CenterBox::builder()
        .name("bar-layout")
        .orientation(Orientation::Horizontal)
        .build();

    let left = region("bar-left");
    left.append(&modules::workspaces::build());
    left.append(&modules::keymode::build().widget);
    left.append(&modules::notes::build().widget);
    left.append(&modules::window_title::build());

    // Noctalia center order: weather · clock · notification history
    // · media (media gets the centre slot here because the calendar
    // popover already lives on the clock).
    let center = region("bar-center");
    center.append(&modules::weather::build().widget);
    center.append(&modules::media::build());

    // saimoom's eww `bar_1` groups: brightness/volume/wifi |
    // battery/memory | clock. The `(sep)` widget between groups is
    // a literal `|` label tinted with the surface1 palette — we
    // mirror that in the spacing.
    // Right region: noctalia-style sequence —
    //   updates · sysmon | podman ufw power twilight |
    //   brightness volume mic network bluetooth | battery memory |
    //   public-ip · clock tray notifications
    let right = region("bar-right");
    right.append(&modules::updates::build().widget);
    right.append(&modules::system_info::build());
    right.append(&modules::podman::build().widget);
    right.append(&modules::ufw::build().widget);
    if let Some(power) = modules::power::build() {
        right.append(&power.widget);
    }
    if let Some(twilight) = modules::twilight::build() {
        right.append(&twilight.widget);
    }
    right.append(&sep());
    if let Some(brightness) = modules::brightness::build() {
        right.append(&brightness.widget);
    }
    if let Some(volume) = modules::volume::build() {
        right.append(&volume.widget);
    }
    if let Some(mic) = modules::microphone::build() {
        right.append(&mic.widget);
    }
    right.append(&modules::network::build());
    right.append(&modules::bluetooth::build().widget);
    right.append(&sep());
    if let Some(battery) = modules::battery::build() {
        right.append(&battery.widget);
    }
    right.append(&modules::memory::build().widget);
    right.append(&modules::public_ip::build().widget);
    right.append(&sep());
    right.append(&modules::tempo::build());
    right.append(&modules::tray::build());
    right.append(&modules::notifications::build());

    layout.set_start_widget(Some(&left));
    layout.set_center_widget(Some(&center));
    layout.set_end_widget(Some(&right));

    layout
}

fn region(name: &str) -> GtkBox {
    GtkBox::builder()
        .name(name)
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .build()
}

