//! The level OSD: icon + progress bar + numeric value in the shared OSD window
//! ([`crate::osd_window`]). One generic component drives the volume, mic and
//! brightness capsules; a [`LevelSource`] supplies what is being shown and
//! when it changed.
//!
//! Message flow, identical for every source:
//! * [`LevelOsdCommand::Rebind`] — the watched device itself changed (new
//!   default sink/source). The source re-subscribes to it; the capsule takes
//!   the new reading without popping up.
//! * [`LevelOsdCommand::Changed`] — the level changed. The capsule takes the
//!   reading and appears (past the start-up [`crate::ShowGate`]).
//! * Each reading (re)arms a one-second auto-hide.

use crate::osd_window;
use gtk4::gdk;
use gtk4::prelude::{BoxExt, GtkWindowExt, RangeExt, WidgetExt};
use mshell_common::WatcherToken;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::marker::PhantomData;
use std::time::Duration;

/// How long a level OSD stays up after the last change.
const HIDE_AFTER: Duration = Duration::from_secs(1);

/// What a level capsule renders.
#[derive(Debug, Clone, PartialEq)]
pub struct OsdLevel {
    pub icon: &'static str,
    /// Slider position, `0.0..=1.0` (GTK clamps anything outside the range).
    pub fraction: f64,
    /// Text in the value label, e.g. `"42%"` or `"Off"`.
    pub label: String,
}

/// A thing a level OSD can show (default sink volume, mic, backlight).
pub trait LevelSource: 'static {
    /// Icon shown before the first reading arrives.
    const INITIAL_ICON: &'static str;

    /// Subscribe to the source. Emit [`LevelOsdCommand::Rebind`] when the
    /// watched device is replaced and [`LevelOsdCommand::Changed`] when its
    /// level changes.
    fn spawn_watchers(sender: &ComponentSender<LevelOsdModel<Self>>)
    where
        Self: Sized;

    /// Re-subscribe to the current device (resetting `token` cancels the
    /// previous device's subscription) and return its reading. The default
    /// just reads, for sources with a single fixed device.
    fn rebind(
        token: &mut WatcherToken,
        sender: &ComponentSender<LevelOsdModel<Self>>,
    ) -> Option<OsdLevel>
    where
        Self: Sized,
    {
        let _ = (token, sender);
        Self::read()
    }

    /// The current reading, or `None` when there is no device.
    fn read() -> Option<OsdLevel>;
}

pub struct LevelOsdInit {
    pub monitor: gdk::Monitor,
}

#[derive(Debug)]
pub enum LevelOsdCommand {
    Rebind,
    Changed,
    Hide,
}

pub struct LevelOsdModel<S> {
    device_token: WatcherToken,
    hide_token: WatcherToken,
    show_gate: crate::ShowGate,
    _source: PhantomData<S>,
}

pub struct LevelOsdWidgets {
    icon: gtk::Image,
    scale: gtk::Scale,
    value: gtk::Label,
}

impl LevelOsdWidgets {
    /// Build the capsule content inside `root`.
    fn build(root: &gtk::Window, initial_icon: &str) -> Self {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);

        let icon = gtk::Image::from_icon_name(initial_icon);
        icon.add_css_class("osd-icon");

        let scale = gtk::Scale::default();
        scale.add_css_class("ok-progress-bar");
        scale.set_hexpand(true);
        scale.set_can_focus(false);
        scale.set_focus_on_click(false);
        scale.set_range(0.0, 1.0);
        scale.set_value(0.0);

        // Numeric level — the value the user is dialling. Fixed width +
        // tabular figures so the bar doesn't reflow as the digits change
        // (DESIGN.md §13.7 state continuity).
        let value = gtk::Label::new(Some(&crate::percent_label(0.0)));
        value.add_css_class("osd-value");
        value.set_width_chars(4);
        value.set_xalign(1.0);

        row.append(&icon);
        row.append(&scale);
        row.append(&value);
        root.set_child(Some(&row));

        Self { icon, scale, value }
    }

    fn show_level(&self, level: &OsdLevel) {
        self.icon.set_icon_name(Some(level.icon));
        self.scale.set_value(level.fraction);
        self.value.set_label(&level.label);
    }
}

impl<S: LevelSource> LevelOsdModel<S> {
    fn take(
        &mut self,
        widgets: &LevelOsdWidgets,
        level: &OsdLevel,
        sender: &ComponentSender<Self>,
    ) {
        widgets.show_level(level);
        osd_window::arm_hide(
            sender,
            &mut self.hide_token,
            HIDE_AFTER,
            LevelOsdCommand::Hide,
        );
    }
}

impl<S: LevelSource> Component for LevelOsdModel<S> {
    type CommandOutput = LevelOsdCommand;
    type Input = ();
    type Output = ();
    type Init = LevelOsdInit;
    type Root = gtk::Window;
    type Widgets = LevelOsdWidgets;

    fn init_root() -> Self::Root {
        osd_window::new_window()
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        osd_window::init_layer_shell(&root, &params.monitor);
        let widgets = LevelOsdWidgets::build(&root, S::INITIAL_ICON);

        S::spawn_watchers(&sender);

        let model = LevelOsdModel {
            device_token: WatcherToken::new(),
            hide_token: WatcherToken::new(),
            show_gate: crate::ShowGate::default(),
            _source: PhantomData,
        };

        ComponentParts { model, widgets }
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            LevelOsdCommand::Rebind => {
                if let Some(level) = S::rebind(&mut self.device_token, &sender) {
                    self.take(widgets, &level, &sender);
                }
            }
            LevelOsdCommand::Changed => {
                if let Some(level) = S::read() {
                    self.take(widgets, &level, &sender);
                    // The gate counts every request, suppressed or not.
                    if self.show_gate.allow() && !crate::suppressed() {
                        root.set_visible(true);
                    }
                }
            }
            LevelOsdCommand::Hide => root.set_visible(false),
        }
    }
}

/// The reading for an audio device: muted-aware icon from the caller, slider
/// at the average channel volume, label in whole percent.
pub(crate) fn audio_level(icon: &'static str, average: f64, average_percent: f64) -> OsdLevel {
    OsdLevel {
        icon,
        fraction: average,
        label: crate::percent_label(average_percent),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_level_puts_the_average_on_the_bar_and_rounds_the_label() {
        let level = audio_level("audio-volume-high-symbolic", 0.735, 73.5);
        assert_eq!(
            level,
            OsdLevel {
                icon: "audio-volume-high-symbolic",
                fraction: 0.735,
                label: "74%".to_string(),
            }
        );
    }

    #[test]
    fn audio_level_keeps_a_boost_in_the_label() {
        // The bar saturates at 1.0 (its range), the label says what it is.
        assert_eq!(audio_level("x", 1.5, 150.0).label, "150%");
    }
}
