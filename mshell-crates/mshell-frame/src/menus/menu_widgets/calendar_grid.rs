//! Month-grid-only calendar widget — same `gtk::Calendar` card
//! the full `Calendar` widget renders at the bottom, without the
//! primary-tinted hero band on top.
//!
//! Used by the dashboard menu where the hero role is filled by
//! a separate `Clock` widget. Pairing the full `Calendar` there
//! would duplicate the time + date display.

use relm4::{
    Component, ComponentParts, ComponentSender,
    gtk::{
        self,
        glib::{self, SourceId},
        prelude::*,
    },
};
use time::{Date, OffsetDateTime};

#[derive(Debug)]
pub(crate) struct CalendarGridModel {
    timer_id: Option<SourceId>,
    current_date: Date,
}

#[derive(Debug)]
pub(crate) enum CalendarGridInput {
    /// Day-rollover tick — re-anchor the grid selection on today.
    /// The user may have navigated forward / back; we mirror the
    /// full Calendar widget's behaviour and jump back when a new
    /// day starts so the grid stays useful as a glanceable
    /// "today" reference.
    CheckDayRollover,
}

#[derive(Debug)]
pub(crate) enum CalendarGridOutput {}

pub(crate) struct CalendarGridInit {}

#[relm4::component(pub)]
impl Component for CalendarGridModel {
    type Input = CalendarGridInput;
    type Output = CalendarGridOutput;
    type Init = CalendarGridInit;
    type CommandOutput = ();

    view! {
        #[root]
        gtk::Box {
            add_css_class: "calendar-grid-card",
            add_css_class: "calendar-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_hexpand: true,

            #[name = "calendar"]
            gtk::Calendar {
                set_can_focus: false,
                set_focus_on_click: false,
                set_show_heading: true,
                set_show_day_names: true,
                set_hexpand: true,
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // 60 s tick — finer than necessary, but cheap. We only act
        // on day rollover; the rest of the time the closure is a
        // no-op. Sub-minute resolution lets a click "today" land
        // promptly when crossing midnight.
        let sender_clone = sender.clone();
        let id = glib::timeout_add_local(std::time::Duration::from_secs(60), move || {
            sender_clone.input(CalendarGridInput::CheckDayRollover);
            glib::ControlFlow::Continue
        });

        let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());

        let model = CalendarGridModel {
            timer_id: Some(id),
            current_date: now.date(),
        };

        let widgets = view_output!();

        widgets.calendar.set_year(now.year());
        widgets.calendar.set_month(now.month() as i32 - 1);
        widgets.calendar.set_day(now.day() as i32);

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            CalendarGridInput::CheckDayRollover => {
                let now =
                    OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
                if now.date() != self.current_date {
                    self.current_date = now.date();
                    widgets.calendar.set_year(now.year());
                    widgets.calendar.set_month(now.month() as i32 - 1);
                    widgets.calendar.set_day(now.day() as i32);
                }
            }
        }

        self.update_view(widgets, sender);
    }
}

impl Drop for CalendarGridModel {
    fn drop(&mut self) {
        if let Some(id) = self.timer_id.take() {
            id.remove();
        }
    }
}
