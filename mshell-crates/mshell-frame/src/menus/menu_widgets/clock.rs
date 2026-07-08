//! Quick-settings clock widget — hero card matching the
//! standalone clock menu's `Calendar` widget visual language.
//!
//! Quick-settings is action-oriented (volume / network /
//! profile / power buttons), so the hero treats **time** as the
//! headline (vs the calendar widget where the day number is the
//! hero). Layout: a primary-tinted rectangle with a big tabular
//! time on the left and the weekday + full date stacked on the
//! right. 1 Hz tick.

use mshell_common::scoped_effects::EffectScope;
use mshell_config::schema::config::*;
use reactive_graph::traits::{Get, GetUntracked};
use relm4::gtk::prelude::OrientableExt;
use relm4::{
    ComponentParts, ComponentSender, SimpleComponent,
    gtk::{
        self, Orientation,
        glib::{self, SourceId},
        prelude::*,
    },
    once_cell,
};
use time::OffsetDateTime;
use time::format_description::parse;

static TIME_FORMAT_24: once_cell::sync::Lazy<Vec<time::format_description::FormatItem<'static>>> =
    once_cell::sync::Lazy::new(|| {
        parse("[hour repr:24 padding:zero]:[minute padding:zero]").unwrap()
    });

static TIME_FORMAT_12: once_cell::sync::Lazy<Vec<time::format_description::FormatItem<'static>>> =
    once_cell::sync::Lazy::new(|| {
        parse("[hour repr:12 padding:zero]:[minute padding:zero] [period case:lower]").unwrap()
    });

static WEEKDAY_FORMAT: once_cell::sync::Lazy<Vec<time::format_description::FormatItem<'static>>> =
    once_cell::sync::Lazy::new(|| parse("[weekday repr:long]").unwrap());

static DATE_FORMAT: once_cell::sync::Lazy<Vec<time::format_description::FormatItem<'static>>> =
    once_cell::sync::Lazy::new(|| {
        // Compact for the inline `WEEKDAY · MONTH DAY` strip —
        // dropping the year keeps the hero a single tight line.
        parse("[month repr:long] [day padding:none]").unwrap()
    });

#[derive(Debug)]
pub(crate) struct ClockModel {
    format_24_h: bool,
    time_label: String,
    weekday_label: String,
    date_label: String,
    timer_id: Option<SourceId>,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum ClockInput {
    UpdateTime,
    ChangeFormat(bool),
    /// Menu reveal state changed — the 1 Hz tick is useless while the
    /// menu is closed, so the timer is stopped on hide and restarted
    /// (with an immediate refresh) on show.
    ParentRevealChanged(bool),
}

#[derive(Debug)]
pub(crate) enum ClockOutput {}

pub(crate) struct ClockInit {}

#[relm4::component(pub)]
impl SimpleComponent for ClockModel {
    type Input = ClockInput;
    type Output = ClockOutput;
    type Init = ClockInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "qs-clock-hero",
            set_orientation: Orientation::Horizontal,
            set_hexpand: true,
            set_spacing: 12,
            set_valign: gtk::Align::Center,

            gtk::Label {
                add_css_class: "qs-clock-hero-time",
                #[watch]
                set_label: model.time_label.as_str(),
                set_valign: gtk::Align::Center,
                set_halign: gtk::Align::Start,
            },

            // Spacer pushes the date label to the far right.
            gtk::Box {
                set_hexpand: true,
            },

            // Single-line weekday · date — replaces the previous
            // stacked two-line right block. Reads as a status
            // strip footer rather than a header.
            gtk::Label {
                add_css_class: "qs-clock-hero-meta",
                #[watch]
                set_label: &format!(
                    "{} · {}",
                    model.weekday_label.to_uppercase(),
                    model.date_label,
                ),
                set_valign: gtk::Align::Center,
                set_halign: gtk::Align::End,
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let base_config = mshell_config::config_manager::config_manager().config();

        // Don't start the 1 Hz tick eagerly: this is the clock *menu* (built
        // per monitor, hidden until opened), so an eager timer means every
        // monitor's clock menu wakes once a second from login even when never
        // shown. The ParentRevealChanged(true) arm starts it on first reveal
        // and refreshes immediately.
        let format_24_h = base_config
            .clone()
            .general()
            .clock_format_24_h()
            .get_untracked();

        let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());

        let time = format_time(&now, format_24_h);
        let weekday = now.format(&WEEKDAY_FORMAT).unwrap_or_default();
        let date = now.format(&DATE_FORMAT).unwrap_or_default();

        let mut effects = EffectScope::new();

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = base_config.clone();
            let format_24_h = config.general().clock_format_24_h().get();
            sender_clone.input(ClockInput::ChangeFormat(format_24_h));
        });

        let model = ClockModel {
            format_24_h,
            time_label: time,
            weekday_label: weekday,
            date_label: date,
            timer_id: None,
            _effects: effects,
        };

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            ClockInput::UpdateTime => {
                let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
                self.time_label = format_time(&now, self.format_24_h);
                self.weekday_label = now.format(&WEEKDAY_FORMAT).unwrap_or_default();
                self.date_label = now.format(&DATE_FORMAT).unwrap_or_default();
            }
            ClockInput::ChangeFormat(format_24_h) => {
                self.format_24_h = format_24_h;
            }
            ClockInput::ParentRevealChanged(visible) => {
                if visible {
                    if self.timer_id.is_none() {
                        self.timer_id = Some(start_tick(&sender));
                    }
                    // Refresh immediately so the time is current the
                    // instant the menu opens, not up to a second stale.
                    sender.input(ClockInput::UpdateTime);
                } else if let Some(id) = self.timer_id.take() {
                    id.remove();
                }
            }
        }
    }
}

/// Start the 1 Hz tick that drives `UpdateTime`.
fn start_tick(sender: &ComponentSender<ClockModel>) -> SourceId {
    let sender = sender.clone();
    glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
        sender.input(ClockInput::UpdateTime);
        glib::ControlFlow::Continue
    })
}

fn format_time(now: &OffsetDateTime, format_24_h: bool) -> String {
    let fmt = if format_24_h {
        &*TIME_FORMAT_24
    } else {
        &*TIME_FORMAT_12
    };
    now.format(fmt).unwrap_or_default()
}

impl Drop for ClockModel {
    fn drop(&mut self) {
        if let Some(id) = self.timer_id.take() {
            id.remove();
        }
    }
}
