//! Lock-key indicator bar pill — Caps / Num / Scroll lock state.
//!
//! Shows three single-letter capsules (A / N / S) that light up
//! when the corresponding lock is engaged. The whole pill is
//! hidden when all three are off so it doesn't clutter the bar
//! at rest.
//!
//! State source: margo's `lock_keys` in `watch state` (the seat
//! keyboard's LEDs), pushed on every toggle, so nothing polls. On an
//! older margo that doesn't report it, the pill falls back to polling
//! `/sys/class/leds/*::capslock/brightness` (and sibling LED files)
//! every 500 ms.

use futures::StreamExt;
use mshell_services::margo_service;
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

pub(crate) struct LockKeysModel {
    caps: bool,
    num: bool,
    scroll: bool,
    _orientation: Orientation,
    /// margo pushes the state; the fallback poll (if running) stops.
    margo_reports: Rc<Cell<bool>>,
    /// The fallback poll timer is installed.
    polling: Rc<Cell<bool>>,
}

#[derive(Debug)]
pub(crate) enum LockKeysInput {
    Poll,
}

#[derive(Debug)]
pub(crate) enum LockKeysCmd {
    /// margo's (caps, num, scroll), or `None` when it doesn't report them.
    Margo(Option<(bool, bool, bool)>),
}

#[derive(Debug)]
pub(crate) enum LockKeysOutput {}

pub(crate) struct LockKeysInit {
    pub(crate) orientation: Orientation,
}

#[relm4::component(pub)]
impl Component for LockKeysModel {
    type CommandOutput = LockKeysCmd;
    type Input = LockKeysInput;
    type Output = LockKeysOutput;
    type Init = LockKeysInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Box {
            add_css_class: "lock-keys-bar-widget",
            add_css_class: "ok-bar-widget",
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
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut stream = margo_service().lock_keys.watch();
            loop {
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    next = stream.next() => match next {
                        Some(keys) => {
                            let keys = keys.map(|k| (k.caps, k.num, k.scroll));
                            let _ = out.send(LockKeysCmd::Margo(keys));
                        }
                        None => break,
                    },
                }
            }
        });

        let model = LockKeysModel {
            caps: false,
            num: false,
            scroll: false,
            _orientation: params.orientation,
            margo_reports: Rc::new(Cell::new(false)),
            polling: Rc::new(Cell::new(false)),
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        let LockKeysCmd::Margo(keys) = message;
        self.margo_reports.set(keys.is_some());
        match keys {
            Some(state) => self.set_state(state),
            None => {
                self.set_state(read_lock_state());
                self.ensure_fallback_poll(&sender);
            }
        }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            LockKeysInput::Poll => self.set_state(read_lock_state()),
        }
    }
}

impl LockKeysModel {
    fn set_state(&mut self, (c, n, s): (bool, bool, bool)) {
        if (c, n, s) != (self.caps, self.num, self.scroll) {
            self.caps = c;
            self.num = n;
            self.scroll = s;
        }
    }

    /// Install the sysfs fallback poll (a margo that doesn't push the
    /// state). It removes itself once margo reports.
    ///
    /// `input_sender().send()` (Result) instead of `s.input()` (panics on
    /// closed channel) so the timer self-cancels when the bar rebuild path
    /// drops this controller.
    fn ensure_fallback_poll(&self, sender: &ComponentSender<Self>) {
        if self.polling.replace(true) {
            return;
        }
        let s = sender.clone();
        let margo_reports = self.margo_reports.clone();
        let polling = self.polling.clone();
        relm4::gtk::glib::timeout_add_local(Duration::from_millis(500), move || {
            if !keep_polling(margo_reports.get())
                || s.input_sender().send(LockKeysInput::Poll).is_err()
            {
                polling.set(false);
                return relm4::gtk::glib::ControlFlow::Break;
            }
            relm4::gtk::glib::ControlFlow::Continue
        });
    }
}

/// The sysfs poll is only the fallback for a margo without `lock_keys`.
fn keep_polling(margo_reports: bool) -> bool {
    !margo_reports
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sysfs_poll_stops_once_margo_reports() {
        assert!(keep_polling(false));
        assert!(!keep_polling(true));
    }
}
