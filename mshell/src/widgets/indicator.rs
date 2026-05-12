//! Generic "icon + optional label" indicator button.
//!
//! Most of mshell's tiny status modules (dns, ufw, podman, updates,
//! power profile, …) collapse to the same shape: a Nerd-Font glyph,
//! an optional short text, and an `on_click` action that fires
//! either a subprocess or opens a menu. This widget centralises that
//! pattern so each module is ~30 lines instead of repeating the
//! same boilerplate.

use gtk::prelude::*;
use gtk::{Box as GtkBox, Button, Label, Orientation};

pub struct Indicator {
    pub widget: Button,
    pub icon: Label,
    pub label: Option<Label>,
}

#[allow(dead_code)]
impl Indicator {
    /// Icon-only indicator (no trailing text).
    pub fn icon_only(name: &str, glyph: &str) -> Self {
        let icon = Label::new(Some(glyph));
        icon.add_css_class("indicator-icon");
        let btn = Button::builder().name(name).child(&icon).build();
        btn.add_css_class("module");
        btn.add_css_class("indicator");
        Self {
            widget: btn,
            icon,
            label: None,
        }
    }

    /// Icon + short text indicator.
    pub fn icon_text(name: &str, glyph: &str, text: &str) -> Self {
        let icon = Label::new(Some(glyph));
        icon.add_css_class("indicator-icon");
        let label = Label::new(Some(text));
        label.add_css_class("indicator-text");
        let row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(4)
            .build();
        row.append(&icon);
        row.append(&label);
        let btn = Button::builder().name(name).child(&row).build();
        btn.add_css_class("module");
        btn.add_css_class("indicator");
        Self {
            widget: btn,
            icon,
            label: Some(label),
        }
    }

    /// Builder-style click handler.
    pub fn on_click(self, f: impl Fn() + 'static) -> Self {
        self.widget.connect_clicked(move |_| f());
        self
    }
}
