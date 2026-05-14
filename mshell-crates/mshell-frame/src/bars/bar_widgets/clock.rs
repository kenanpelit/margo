//! Bar-clock widget.
//!
//! Reads the rotating-format list from `[tempo]` in the config
//! (`clock_format` = initial, `formats` = cycle list). A *double*
//! left-click on the pill bumps the in-memory index → the label
//! re-renders on the next 1-second tick (or immediately, since we
//! call `tick_label` straight after the cycle). A single click is
//! still forwarded as `ClockOutput::Clicked` (consumers use it to
//! open the calendar popover).
//!
//! Format strings are chrono-style strftime (`%H:%M`, `%a %d %b
//! %H:%M`, `%d.%m.%Y`, …) rather than the `time` crate's
//! `[hour]:[minute]` description — the user's mental model is
//! strftime, and chrono parses it lazily at render time so a typo
//! in the config produces a single literal-string label instead of
//! crashing the bar.
//!
//! Back-compat: when `clock_format` is empty, fall back to the
//! pre-tempo `clock_format_24_h` bool (so an existing config that
//! never mentioned `[tempo]` still gets the old behavior).

use chrono::Local;
use mshell_common::scoped_effects::EffectScope;
use mshell_config::schema::config::*;
use reactive_graph::traits::{Get, GetUntracked};
use relm4::{
    ComponentParts, ComponentSender, SimpleComponent,
    gtk::{
        self, Orientation,
        glib::{self, SourceId},
        prelude::{ButtonExt, GestureSingleExt, WidgetExt},
    },
};
use std::cell::Cell;
use std::rc::Rc;

/// Built-in fallback used when `[tempo].clock_format` is the empty
/// string AND `general.clock_format_24_h = true`.
const FALLBACK_24H: &str = "%H:%M";

/// Built-in fallback used when `[tempo].clock_format` is the empty
/// string AND `general.clock_format_24_h = false`.
const FALLBACK_12H: &str = "%I:%M %p";

/// Vertical bars: pretty stack of HH over MM.
const FALLBACK_24H_VERTICAL: &str = "%H\n%M";
const FALLBACK_12H_VERTICAL: &str = "%I\n%M";

#[derive(Debug)]
pub(crate) struct ClockModel {
    orientation: Orientation,
    /// The full ordered format list — index 0 is the "initial"
    /// (`clock_format`), the remainder is `formats` minus duplicates.
    /// Empty means "use the wired fallbacks below" (back-compat path
    /// for configs that never opted into `[tempo]`).
    formats: Vec<String>,
    fallback_24h: bool,
    /// Shared with the GestureClick callback. `Rc<Cell>` so the
    /// callback can `set()` without needing &mut self.
    current_idx: Rc<Cell<usize>>,
    time_label: String,
    timer_id: Option<SourceId>,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum ClockInput {
    UpdateTime,
    /// Bump `current_idx`, then re-render immediately. Triggered by
    /// the GestureClick on n_press == 2.
    CycleFormat,
    /// Reload the format list from config (fires on the reactive
    /// effect when the user `mshellctl config reload`s).
    ReloadFormats { formats: Vec<String>, fallback_24h: bool },
}

#[derive(Debug)]
pub(crate) enum ClockOutput {
    Clicked,
}

pub(crate) struct ClockInit {
    pub(crate) orientation: Orientation,
}

#[relm4::component(pub)]
impl SimpleComponent for ClockModel {
    type Input = ClockInput;
    type Output = ClockOutput;
    type Init = ClockInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "clock-bar-widget",
            set_hexpand: model.orientation == Orientation::Vertical,
            set_vexpand: model.orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,

            #[name="button"]
            gtk::Button {
                set_css_classes: &["ok-button-surface", "ok-bar-widget"],
                connect_clicked[sender] => move |_| {
                    sender.output(ClockOutput::Clicked).unwrap_or_default();
                },

                gtk::Label {
                    add_css_class: "clock-bar-label",
                    #[watch]
                    set_label: model.time_label.as_str(),
                }
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let base_config = mshell_config::config_manager::config_manager().config();

        // Drive the per-second re-render. We could do per-minute when
        // none of the formats contain `%S`, but the cost of an extra
        // 59 no-op frames a minute is rounding error — keep the path
        // uniform.
        let sender_clone = sender.clone();
        let id = glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
            sender_clone.input(ClockInput::UpdateTime);
            glib::ControlFlow::Continue
        });

        let fallback_24h = base_config
            .clone()
            .general()
            .clock_format_24_h()
            .get_untracked();

        let formats = collect_formats(base_config.clone().tempo().get_untracked());
        let current_idx = Rc::new(Cell::new(0usize));

        let time_label = render_now(
            &formats,
            current_idx.get(),
            fallback_24h,
            params.orientation,
        );

        // Reactive effect: re-fetch the format list whenever the
        // user reloads the config. Reset cycle to the initial format
        // (clock_format) so a config edit doesn't surface a stale
        // index pointing past the new array's end.
        let mut effects = EffectScope::new();
        let sender_clone = sender.clone();
        let base_config_eff = base_config.clone();
        effects.push(move |_| {
            let fallback_24h = base_config_eff
                .clone()
                .general()
                .clock_format_24_h()
                .get();
            let formats = collect_formats(base_config_eff.clone().tempo().get());
            sender_clone.input(ClockInput::ReloadFormats {
                formats,
                fallback_24h,
            });
        });

        let model = ClockModel {
            orientation: params.orientation,
            formats,
            fallback_24h,
            current_idx: current_idx.clone(),
            time_label,
            timer_id: Some(id),
            _effects: effects,
        };

        let widgets = view_output!();

        // Double-RIGHT-click cycles the format. Right-click was the
        // explicit choice (avoids conflicting with the calendar
        // popover that left-click opens, and matches Hyprpanel /
        // waybar muscle memory). A GestureClick on the SECONDARY
        // button is independent of the gtk::Button's own clicked
        // signal — left-click still flows through `connect_clicked`
        // → `ClockOutput::Clicked` untouched.
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
        let sender_dbl = sender.clone();
        gesture.connect_pressed(move |_, n_press, _, _| {
            if n_press >= 2 {
                sender_dbl.input(ClockInput::CycleFormat);
            }
        });
        widgets.button.add_controller(gesture);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>) {
        match message {
            ClockInput::UpdateTime => {
                self.time_label = render_now(
                    &self.formats,
                    self.current_idx.get(),
                    self.fallback_24h,
                    self.orientation,
                );
            }
            ClockInput::CycleFormat => {
                if self.formats.len() > 1 {
                    let next = (self.current_idx.get() + 1) % self.formats.len();
                    self.current_idx.set(next);
                    self.time_label = render_now(
                        &self.formats,
                        next,
                        self.fallback_24h,
                        self.orientation,
                    );
                }
            }
            ClockInput::ReloadFormats {
                formats,
                fallback_24h,
            } => {
                self.formats = formats;
                self.fallback_24h = fallback_24h;
                self.current_idx.set(0);
                self.time_label = render_now(
                    &self.formats,
                    0,
                    self.fallback_24h,
                    self.orientation,
                );
            }
        }
    }
}

impl Drop for ClockModel {
    fn drop(&mut self) {
        if let Some(id) = self.timer_id.take() {
            id.remove();
        }
    }
}

/// Build the ordered cycling list: `[clock_format, …formats]` with
/// the initial format de-duped if it also appears in the list, and
/// blank entries dropped. An empty result means "no tempo config,
/// fall back to clock_format_24_h" — handled by `render_now`.
fn collect_formats(tempo: Tempo) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(tempo.formats.len() + 1);
    let initial = tempo.clock_format.trim().to_string();
    if !initial.is_empty() {
        out.push(initial.clone());
    }
    for f in tempo.formats {
        let trimmed = f.trim();
        if !trimmed.is_empty() && !out.iter().any(|existing| existing == trimmed) {
            out.push(trimmed.to_string());
        }
    }
    out
}

/// Render the current `chrono::Local::now()` using the format at
/// `idx`, with fallback to the 12h / 24h preset when the formats
/// list is empty (back-compat) or `idx` is out of range.
fn render_now(formats: &[String], idx: usize, fallback_24h: bool, orientation: Orientation) -> String {
    let now = Local::now();
    if let Some(fmt) = formats.get(idx) {
        // chrono's Display impl wraps any strftime error into a
        // literal `?` byte in the output rather than panicking;
        // formatting() returns the same Result. Either way the user
        // sees what's wrong instead of an empty bar.
        return now.format(fmt).to_string();
    }
    let fmt = match (orientation, fallback_24h) {
        (Orientation::Vertical, true) => FALLBACK_24H_VERTICAL,
        (Orientation::Vertical, false) => FALLBACK_12H_VERTICAL,
        (_, true) => FALLBACK_24H,
        (_, false) => FALLBACK_12H,
    };
    now.format(fmt).to_string()
}

