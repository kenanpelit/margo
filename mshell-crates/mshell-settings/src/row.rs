//! Shared settings-row template.
//!
//! A horizontal row with a left-hand bold title + small description and the
//! control widget appended on the right by the caller. Keeps the big `view!`
//! blocks across the settings pages readable + consistent.
//!
//! ```ignore
//! #[template]
//! Row {
//!     #[template_child] title { set_label: "Repeat rate" },
//!     #[template_child] desc  { set_label: "Key repeats per second." },
//!     gtk::SpinButton { /* … */ },
//! }
//! ```

use relm4::gtk::prelude::*;
use relm4::{WidgetTemplate, gtk};

#[relm4::widget_template(pub)]
impl WidgetTemplate for Row {
    view! {
        gtk::Box {
            // The Adwaita action-row chrome (padding + hover wash +
            // boxed-list separators) — geometry from the central
            // component tokens (DESIGN.md §1/§5).
            add_css_class: "action-row",
            set_orientation: gtk::Orientation::Horizontal,
            set_spacing: 20,
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                #[name = "title"]
                gtk::Label {
                    add_css_class: "label-medium-bold",
                    set_halign: gtk::Align::Start,
                    set_hexpand: true,
                },
                #[name = "desc"]
                gtk::Label {
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },
            },
        }
    }
}
