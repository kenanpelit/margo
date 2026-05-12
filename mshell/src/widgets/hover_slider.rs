//! Hover-revealing slider — eww's `(volume) / (bright)` pattern.
//!
//! GtkBox row: [icon GtkLabel] [GtkRevealer { GtkScale }]
//!
//! On pointer enter the revealer slides the scale out from the left
//! (`RevealerTransitionType::SlideLeft` is the GTK4 name for what
//! eww calls `:transition "slideleft"`). On leave the scale slides
//! back, no extra Rust state needed.
//!
//! Callers wire `on_change` to drive the underlying service
//! (brightnessctl / wpctl). External polling — e.g. picking up a
//! Fn-brightness keypress — happens in the caller too, via
//! `sync_external()` which respects a small grace window around the
//! user's last drag so we don't fight the slider thumb.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use gtk::prelude::*;
use gtk::{
    Align, Box as GtkBox, EventControllerMotion, Label, Orientation, Revealer,
    RevealerTransitionType, Scale,
};

/// Time we treat the user as "still touching the slider" after their
/// most recent `value-changed` emission. Polling skips its sync()
/// during this window so the thumb doesn't jump back to the kernel-
/// reported value mid-drag.
const USER_GRACE_MS: u64 = 1500;

pub struct HoverSlider {
    pub widget: GtkBox,
    pub scale: Scale,
    last_user_change: Rc<Cell<Option<Instant>>>,
}

impl HoverSlider {
    /// Apply an externally-sourced value (e.g. polled brightnessctl)
    /// only when the user isn't currently dragging the thumb. Used
    /// by the shared poller landing in a follow-up patch.
    #[allow(dead_code)]
    pub fn sync_external(&self, percent: u8) {
        let last = self.last_user_change.get();
        if let Some(t) = last {
            if t.elapsed().as_millis() < USER_GRACE_MS as u128 {
                return;
            }
        }
        let target = percent as f64;
        if (self.scale.value() - target).abs() >= 1.0 {
            self.scale.set_value(target);
        }
    }
}

/// Build a slider row. `on_change` fires every time the user moves
/// the thumb — the caller is expected to push the value back into
/// the underlying service (e.g. spawn `brightnessctl set N%`).
pub fn build(
    name: &str,
    icon: &str,
    on_change: impl Fn(f64) + 'static,
) -> HoverSlider {
    let row = GtkBox::builder()
        .name(name)
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();
    row.add_css_class("module");
    row.add_css_class("hover-slider");

    let icon_label = Label::builder()
        .label(icon)
        .halign(Align::Center)
        .build();
    icon_label.add_css_class("hover-slider-icon");

    let scale = Scale::with_range(Orientation::Horizontal, 0.0, 100.0, 1.0);
    scale.set_width_request(80);
    scale.set_draw_value(false);
    scale.add_css_class("hover-slider-scale");

    let last_user_change: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(None));
    let last_for_emit = last_user_change.clone();
    scale.connect_value_changed(move |s| {
        last_for_emit.set(Some(Instant::now()));
        on_change(s.value());
    });

    let revealer = Revealer::builder()
        .transition_type(RevealerTransitionType::SlideLeft)
        .transition_duration(220)
        .child(&scale)
        .build();

    row.append(&icon_label);
    row.append(&revealer);

    let motion = EventControllerMotion::new();
    let revealer_enter = revealer.clone();
    motion.connect_enter(move |_, _, _| revealer_enter.set_reveal_child(true));
    let revealer_leave = revealer.clone();
    motion.connect_leave(move |_| revealer_leave.set_reveal_child(false));
    row.add_controller(motion);

    HoverSlider {
        widget: row,
        scale,
        last_user_change,
    }
}
