//! Standalone microphone slider — eww/Noctalia ships volume +
//! mic as separate bar items. Same hover-slider widget as
//! volume, but driven from `audio::source_*`.

use gtk::prelude::*;
use gtk::GestureClick;

use crate::services::audio;
use crate::widgets::hover_slider::{self, HoverSlider};

const ICON_ON: &str = "\u{f130}";    // mic
const ICON_MUTED: &str = "\u{f131}"; // mic-slash

pub fn build() -> Option<HoverSlider> {
    let initial = audio::source_current()?;
    let slider = hover_slider::build("microphone", icon_for(initial.muted), |v| {
        audio::source_set_volume(v.round().clamp(0.0, 100.0) as u8);
    });
    slider.scale.set_value(initial.volume_percent as f64);
    slider.widget.add_css_class("microphone");
    if initial.muted {
        slider.widget.add_css_class("muted");
    }

    let right = GestureClick::builder().button(3).build();
    right.connect_pressed(|_, _, _, _| audio::source_toggle_mute());
    slider.widget.add_controller(right);

    Some(slider)
}

fn icon_for(muted: bool) -> &'static str {
    if muted { ICON_MUTED } else { ICON_ON }
}
