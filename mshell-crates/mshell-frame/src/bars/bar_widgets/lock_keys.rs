//! Lock-key indicator bar pill — Caps / Num / Scroll lock state.
//!
//! Shows three single-letter capsules (A / N / S) that light up
//! when the corresponding lock is engaged. The whole pill is
//! hidden when all three are off so it doesn't clutter the bar
//! at rest.
//!
//! State source: `/sys/class/leds/*::capslock/brightness` (and
//! sibling LED files). The kernel maintains these as `0` / `1`
//! independent of any input method, so they work under both X
//! and Wayland regardless of focus state — exactly what a bar
//! widget needs. Polled every 500 ms; lock keys never toggle
//! rapidly, so faster polling is wasted work.

use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::PathBuf;
use std::time::Duration;

pub(crate) struct LockKeysModel {
    caps: bool,
    num: bool,
    scroll: bool,
    _orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum LockKeysInput {
    Poll,
}

#[derive(Debug)]
pub(crate) enum LockKeysOutput {}

pub(crate) struct LockKeysInit {
    pub(crate) orientation: Orientation,
}

#[relm4::component(pub)]
impl Component for LockKeysModel {
    type CommandOutput = ();
    type Input = LockKeysInput;
    type Output = LockKeysOutput;
    type Init = LockKeysInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Box {
            add_css_class: "lock-keys-bar-widget",
            set_hexpand: model._orientation == Orientation::Vertical,
            set_vexpand: model._orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,
            set_spacing: 4,
            // Hide the whole pill when nothing is locked so the
            // bar isn't polluted by dim letters most of the time.
            #[watch]
            set_visible: model.caps || model.num || model.scroll,

            gtk::Label {
                add_css_class: "lock-key-indicator",
                #[watch]
                set_css_classes: if model.caps {
                    &["lock-key-indicator", "active"]
                } else {
                    &["lock-key-indicator"]
                },
                #[watch]
                set_visible: model.caps,
                set_label: "A",
            },
            gtk::Label {
                add_css_class: "lock-key-indicator",
                #[watch]
                set_css_classes: if model.num {
                    &["lock-key-indicator", "active"]
                } else {
                    &["lock-key-indicator"]
                },
                #[watch]
                set_visible: model.num,
                set_label: "N",
            },
            gtk::Label {
                add_css_class: "lock-key-indicator",
                #[watch]
                set_css_classes: if model.scroll {
                    &["lock-key-indicator", "active"]
                } else {
                    &["lock-key-indicator"]
                },
                #[watch]
                set_visible: model.scroll,
                set_label: "S",
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let (caps, num, scroll) = read_lock_state();

        // Glib main-loop poll. Lock keys don't toggle fast — a
        // 500 ms tick keeps the indicator responsive without
        // measurable CPU cost (three tiny sysfs reads per tick).
        {
            let s = sender.clone();
            relm4::gtk::glib::timeout_add_local(
                Duration::from_millis(500),
                move || {
                    s.input(LockKeysInput::Poll);
                    relm4::gtk::glib::ControlFlow::Continue
                },
            );
        }

        let model = LockKeysModel {
            caps,
            num,
            scroll,
            _orientation: params.orientation,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            LockKeysInput::Poll => {
                let (c, n, s) = read_lock_state();
                if (c, n, s) != (self.caps, self.num, self.scroll) {
                    self.caps = c;
                    self.num = n;
                    self.scroll = s;
                }
            }
        }
    }
}

/// Walk `/sys/class/leds` and pick the first input device that
/// exposes capslock / numlock / scrolllock LED files. Multiple
/// keyboards may each carry their own copy; we take the first
/// hit per lock, which matches what the user sees on their
/// active keyboard 99 % of the time.
fn read_lock_state() -> (bool, bool, bool) {
    let dir = PathBuf::from("/sys/class/leds");
    let mut caps = None;
    let mut num = None;
    let mut scroll = None;
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return (false, false, false);
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(n) = name.to_str() else { continue };
        let target = if n.ends_with("::capslock") {
            &mut caps
        } else if n.ends_with("::numlock") {
            &mut num
        } else if n.ends_with("::scrolllock") {
            &mut scroll
        } else {
            continue;
        };
        if target.is_some() {
            continue;
        }
        let p = entry.path().join("brightness");
        if let Ok(s) = std::fs::read_to_string(&p) {
            *target = Some(s.trim() != "0");
        }
    }
    (
        caps.unwrap_or(false),
        num.unwrap_or(false),
        scroll.unwrap_or(false),
    )
}
