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
        parse("[month repr:long] [day padding:none], [year repr:full base:calendar]").unwrap()
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
            set_spacing: 16,
            set_valign: gtk::Align::Center,

            gtk::Label {
                add_css_class: "qs-clock-hero-time",
                #[watch]
                set_label: model.time_label.as_str(),
                set_valign: gtk::Align::Center,
                set_halign: gtk::Align::Start,
            },

            gtk::Box {
                set_orientation: Orientation::Vertical,
                set_hexpand: true,
                set_valign: gtk::Align::Center,
                set_halign: gtk::Align::End,
                set_spacing: 0,

                gtk::Label {
                    add_css_class: "qs-clock-hero-weekday",
                    #[watch]
                    set_label: model.weekday_label.as_str(),
                    set_halign: gtk::Align::End,
                },

                gtk::Label {
                    add_css_class: "qs-clock-hero-date",
                    #[watch]
                    set_label: model.date_label.as_str(),
                    set_halign: gtk::Align::End,
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let base_config = mshell_config::config_manager::config_manager().config();

        let sender_clone = sender.clone();
        let id = glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
            sender_clone.input(ClockInput::UpdateTime);
            glib::ControlFlow::Continue
        });

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
            timer_id: Some(id),
            _effects: effects,
        };

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>) {
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
        }
    }
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
