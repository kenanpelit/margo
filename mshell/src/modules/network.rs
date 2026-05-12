//! Network module — eww `(wifi)`.
//!
//! Icon glyph + hover-revealing SSID label. VPN state swaps the
//! glyph for a lock and tags the row with `.vpn` so the stylesheet
//! can paint it green.

use gtk::prelude::*;
use gtk::{
    Box as GtkBox, EventControllerMotion, Label, Orientation, Revealer, RevealerTransitionType,
};

use crate::services::network;

const POLL_SECS: u32 = 5;

pub fn build() -> GtkBox {
    let row = GtkBox::builder()
        .name("network")
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();
    row.add_css_class("module");
    row.add_css_class("network");

    let icon = Label::builder().name("network-icon").build();
    icon.add_css_class("network-icon");

    let ssid = Label::builder().name("network-ssid").build();
    ssid.add_css_class("network-ssid");
    ssid.set_max_width_chars(14);
    ssid.set_ellipsize(gtk::pango::EllipsizeMode::End);

    let revealer = Revealer::builder()
        .transition_type(RevealerTransitionType::SlideRight)
        .transition_duration(220)
        .child(&ssid)
        .build();

    row.append(&icon);
    row.append(&revealer);

    refresh(&row, &icon, &ssid);

    let row_tick = row.clone();
    let icon_tick = icon.clone();
    let ssid_tick = ssid.clone();
    glib::timeout_add_seconds_local(POLL_SECS, move || {
        refresh(&row_tick, &icon_tick, &ssid_tick);
        glib::ControlFlow::Continue
    });

    let motion = EventControllerMotion::new();
    let rev_enter = revealer.clone();
    motion.connect_enter(move |_, _, _| rev_enter.set_reveal_child(true));
    let rev_leave = revealer.clone();
    motion.connect_leave(move |_| rev_leave.set_reveal_child(false));
    row.add_controller(motion);

    row
}

fn refresh(row: &GtkBox, icon: &Label, ssid: &Label) {
    let snap = network::current();
    let connected = snap.ssid.is_some();
    icon.set_label(network::glyph(snap.signal, connected, snap.vpn));
    ssid.set_label(snap.ssid.as_deref().unwrap_or("–"));
    row.remove_css_class("vpn");
    row.remove_css_class("disconnected");
    if snap.vpn {
        row.add_css_class("vpn");
    }
    if !connected {
        row.add_css_class("disconnected");
    }
}
