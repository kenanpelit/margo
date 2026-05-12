//! Brightness module — eww `(bright)`.
//!
//! Hover-revealing slider over `brightnessctl`. Stage-4 doesn't yet
//! poll for external (Fn-key) changes — that lands together with
//! the bar's global state ticker in a later stage. The initial
//! value is read once at construction so the bar opens with the
//! right slider position.

use gtk::prelude::*;

use crate::services::brightness;
use crate::widgets::hover_slider::{self, HoverSlider};

const ICON: &str = "\u{f835}"; // nerd-font: nf-md-brightness_7

pub fn build() -> Option<HoverSlider> {
    let initial = brightness::current_percent()?;
    let slider = hover_slider::build("brightness", ICON, |v| {
        brightness::set_percent(v.round().clamp(0.0, 100.0) as u8);
    });
    slider.scale.set_value(initial as f64);
    slider.widget.add_css_class("brightness");
    Some(slider)
}
