//! Volume module — eww `(volume)`.
//!
//! Same hover-slider pattern as brightness, but the icon glyph
//! flips between speaker / muted speaker depending on the current
//! `wpctl get-volume` mute flag. Right-click toggles mute (Stage 8
//! will add the speaker+mic popup that eww opens on `(audio_ctl)`).

use gtk::prelude::*;
use gtk::{
    Align, Box as GtkBox, GestureClick, Label, Orientation, Popover, PositionType, Scale,
};

use crate::services::audio;
use crate::widgets::hover_slider::{self, HoverSlider};

const ICON_ON: &str = "\u{f028}"; // nerd-font: nf-fa-volume_up
const ICON_MUTED: &str = "\u{f6a9}"; // nerd-font: nf-fa-volume_xmark

pub fn build() -> Option<HoverSlider> {
    let initial = audio::current()?;
    let slider = hover_slider::build("volume", icon_for(initial.muted), |v| {
        audio::set_volume(v.round().clamp(0.0, 100.0) as u8);
    });
    slider.scale.set_value(initial.volume_percent as f64);
    slider.widget.add_css_class("volume");
    if initial.muted {
        slider.widget.add_css_class("muted");
    }

    // Right-click → speaker + mic detail popover (eww `audio_ctl`).
    let popover = build_popover();
    popover.set_parent(&slider.widget);

    let popover_for_click = popover.clone();
    let right = GestureClick::builder().button(3).build();
    right.connect_pressed(move |_, _, _, _| {
        refresh_popover(&popover_for_click);
        popover_for_click.popup();
    });
    slider.widget.add_controller(right);

    Some(slider)
}

/// Build the speaker + mic popover (eww `(audio)`). Two labelled
/// GtkScale rows + a mute toggle button under each. We name every
/// widget so `refresh_popover` can find it without juggling clones.
fn build_popover() -> Popover {
    let body = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .build();
    body.add_css_class("audio-popup");

    body.append(&speaker_section());
    body.append(&mic_section());

    let popover = Popover::builder()
        .child(&body)
        .position(PositionType::Bottom)
        .has_arrow(true)
        .autohide(true)
        .build();
    popover.add_css_class("popover-audio");
    popover
}

fn speaker_section() -> GtkBox {
    let col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .build();

    let header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();
    let icon = Label::builder()
        .label("\u{f028}") // speaker
        .halign(Align::Start)
        .build();
    icon.add_css_class("audio-popup-icon");
    let lbl = Label::builder()
        .label("Speaker")
        .halign(Align::Start)
        .hexpand(true)
        .build();
    lbl.add_css_class("audio-popup-heading");
    let mute_btn = gtk::Button::builder().label("\u{f6a9}").build();
    mute_btn.add_css_class("audio-popup-mute");
    mute_btn.connect_clicked(|_| audio::toggle_mute());

    header.append(&icon);
    header.append(&lbl);
    header.append(&mute_btn);

    let scale = Scale::with_range(Orientation::Horizontal, 0.0, 100.0, 1.0);
    scale.set_widget_name("audio-popup-speaker");
    scale.set_hexpand(true);
    scale.set_draw_value(true);
    scale.add_css_class("audio-popup-scale");
    scale.connect_value_changed(|s| {
        audio::set_volume(s.value().round().clamp(0.0, 100.0) as u8);
    });

    col.append(&header);
    col.append(&scale);
    col
}

fn mic_section() -> GtkBox {
    let col = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(4)
        .build();

    let header = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();
    let icon = Label::builder()
        .label("\u{f130}") // microphone
        .halign(Align::Start)
        .build();
    icon.add_css_class("audio-popup-icon");
    let lbl = Label::builder()
        .label("Microphone")
        .halign(Align::Start)
        .hexpand(true)
        .build();
    lbl.add_css_class("audio-popup-heading");
    let mute_btn = gtk::Button::builder().label("\u{f131}").build();
    mute_btn.add_css_class("audio-popup-mute");
    mute_btn.connect_clicked(|_| audio::source_toggle_mute());

    header.append(&icon);
    header.append(&lbl);
    header.append(&mute_btn);

    let scale = Scale::with_range(Orientation::Horizontal, 0.0, 100.0, 1.0);
    scale.set_widget_name("audio-popup-mic");
    scale.set_hexpand(true);
    scale.set_draw_value(true);
    scale.add_css_class("audio-popup-scale");
    scale.connect_value_changed(|s| {
        audio::source_set_volume(s.value().round().clamp(0.0, 100.0) as u8);
    });

    col.append(&header);
    col.append(&scale);
    col
}

fn refresh_popover(popover: &Popover) {
    let Some(child) = popover.child() else {
        return;
    };
    let sink = audio::current();
    let source = audio::source_current();

    for_each_scale(&child, &mut |scale| match scale.widget_name().as_str() {
        "audio-popup-speaker" => {
            if let Some(s) = &sink {
                scale.set_value(s.volume_percent as f64);
            }
        }
        "audio-popup-mic" => {
            if let Some(s) = &source {
                scale.set_value(s.volume_percent as f64);
            }
        }
        _ => {}
    });
}

fn for_each_scale(widget: &gtk::Widget, f: &mut impl FnMut(&Scale)) {
    if let Some(s) = widget.downcast_ref::<Scale>() {
        f(s);
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        for_each_scale(&c, f);
        child = c.next_sibling();
    }
}

fn icon_for(muted: bool) -> &'static str {
    if muted { ICON_MUTED } else { ICON_ON }
}
