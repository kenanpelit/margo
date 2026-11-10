//! Today: the right column's flexible card. An **Events** section (the
//! selected day's events, then the next seven days' grouped by day) and a
//! **Notifications** section (the newest unread notifications), filling
//! the height the left column leaves in that order — whole rows only, the
//! rest folded into a "+N more" line that opens the calendar / the
//! notification centre (see [`super::fit`]). Room nothing needs goes to
//! the idle media card's station rows, then to the **Earlier** (already
//! read) notifications, so the card never shows an empty band.
//!
//! Data: the shared `calendar_data` events (the same fetch as the calendar
//! card) and the notification service's history cut at the shared unread
//! line (`notification_unread`) — no poller of its own.

use super::super::card::Card;
use super::fit::{self, Item, Kind, Plan, Section};
use crate::menus::menu_widgets::calendar_data;
use chrono::{Local, NaiveDate};
use relm4::gtk;
use relm4::gtk::prelude::{AdjustmentExt, BoxExt, ButtonExt, Cast, WidgetExt};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

/// The card's floor: one row. It never asks for more, so it can't make
/// the right column outgrow the left; it takes what is left over.
const MIN_HEIGHT: i32 = 28;
/// Spacing of the lines box.
const SPACING: i32 = 4;
/// At most this much is added to each gap to spread a part-row leftover
/// (rows are whole, so up to one row of room can remain).
const MAX_SPREAD: i32 = 12;
/// How far the Upcoming section looks ahead.
pub(crate) const UPCOMING_DAYS: i64 = 7;
/// Notification rows built at most (the rest is "+N more" anyway).
const MAX_NOTES: usize = 12;
const NOTE_ICON_SIZE: i32 = 16;

/// One unread notification as the card shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NoteLine {
    pub(crate) id: u32,
    /// The app icon (name or path), if the app sent one.
    pub(crate) icon: Option<String>,
    pub(crate) app: String,
    pub(crate) summary: String,
    /// "20:14" today, "Mon" earlier.
    pub(crate) time: String,
}

/// One line of the Events section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EventLine {
    /// A day heading ("Tomorrow", "Thursday 12 November").
    Day(String),
    Event {
        time: String,
        title: String,
        location: Option<String>,
    },
    /// The one-line empty state.
    Empty(String),
}

/// A click inside the card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TodayEvent {
    /// "+N more" under the events: the calendar / agenda menu.
    OpenCalendar,
    /// A notification row or "+N more": the notification centre.
    OpenNotifications,
    MarkAllRead,
}

/// "Tomorrow" / "Thursday 12 November" for an upcoming day heading.
pub(crate) fn day_heading(day: NaiveDate, today: NaiveDate) -> String {
    if today.succ_opt() == Some(day) {
        "Tomorrow".to_string()
    } else {
        day.format("%A %-d %B").to_string()
    }
}

/// The Events section for `selected`: its events, then the next
/// [`UPCOMING_DAYS`] days that have any, each under its day heading.
/// Empty states are one line: "No events this week" when the whole span is
/// free, "No events today" (or "on Friday") before upcoming days.
pub(crate) fn event_lines(
    events: &[mcal::Event],
    selected: NaiveDate,
    today: NaiveDate,
) -> Vec<EventLine> {
    let line = |e: &mcal::Event| EventLine::Event {
        time: calendar_data::time_label(e),
        title: e.summary.clone(),
        location: e.location.clone().filter(|l| !l.trim().is_empty()),
    };
    let mut out: Vec<EventLine> = mcal::events_on_day(events, selected)
        .iter()
        .map(line)
        .collect();
    let first_upcoming = out.len();
    for offset in 1..=UPCOMING_DAYS {
        let Some(day) = selected.checked_add_signed(chrono::Duration::days(offset)) else {
            break;
        };
        let on_day = mcal::events_on_day(events, day);
        if !on_day.is_empty() {
            out.push(EventLine::Day(day_heading(day, today)));
            out.extend(on_day.iter().map(line));
        }
    }
    if out.is_empty() {
        return vec![EventLine::Empty("No events this week".into())];
    }
    if first_upcoming == 0 {
        let free = if selected == today {
            "No events today".to_string()
        } else {
            format!("No events on {}", selected.format("%A"))
        };
        out.insert(0, EventLine::Empty(free));
    }
    out
}

/// Line kinds for the fitter.
fn event_kind(l: &EventLine) -> Kind {
    match l {
        EventLine::Day(_) => Kind::Caption,
        EventLine::Event { .. } => Kind::Row,
        EventLine::Empty(_) => Kind::Empty,
    }
}

/// What a section is called in its summary line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Noun {
    Events,
    Notifications,
}

/// The fold line: "+3 more" under shown rows; standing alone (no row of
/// the section fits) it names them — "5 events", "3 new notifications".
pub(crate) fn more_label(n: usize, alone: bool, noun: Noun) -> String {
    if !alone {
        return format!("+{n} more");
    }
    match (noun, n) {
        (Noun::Events, 1) => "1 event".to_string(),
        (Noun::Events, n) => format!("{n} events"),
        (Noun::Notifications, 1) => "1 new notification".to_string(),
        (Noun::Notifications, n) => format!("{n} new notifications"),
    }
}

/// The idle media card's spill rows the Today card can grow into.
pub(crate) trait Spill {
    /// (row height, cost of each row's spacing, rows available).
    fn spill_metrics(&self) -> (i32, i32, usize);
    /// Rows currently shown.
    fn spill_shown(&self) -> usize;
    fn set_spill(&self, rows: usize);
}

struct Lines {
    /// Events section widgets, in order, and its "+N more".
    events: Vec<gtk::Widget>,
    events_more: gtk::Button,
    /// Notifications section widgets (caption first when not empty).
    notes: Vec<gtk::Widget>,
    notes_kinds: Vec<Kind>,
    notes_more: gtk::Button,
    /// Unread notifications past [`MAX_NOTES`] (never built, still
    /// counted in "+N more").
    notes_beyond: usize,
    event_kinds: Vec<Kind>,
    /// "Earlier" (read) notifications: caption + rows, filler only.
    earlier: Vec<gtk::Widget>,
    earlier_kinds: Vec<Kind>,
}

pub(crate) struct TodayCard {
    pub(crate) card: Card,
    scroller: gtk::ScrolledWindow,
    lines_box: gtk::Box,
    lines: RefCell<Option<Lines>>,
    on: Rc<dyn Fn(TodayEvent)>,
    spill: RefCell<Option<Weak<dyn Spill>>>,
    pending: Cell<bool>,
    last: RefCell<Option<(Plan, usize, i32, usize)>>,
    weak_self: RefCell<Weak<TodayCard>>,
}

fn more_button(on: &Rc<dyn Fn(TodayEvent)>, ev: TodayEvent, name: &str) -> gtk::Button {
    let b = gtk::Button::with_label("");
    b.add_css_class("mdash-more");
    b.set_halign(gtk::Align::Start);
    mshell_common::a11y::name(&b, name);
    let on = on.clone();
    b.connect_clicked(move |_| on(ev));
    b
}

impl TodayCard {
    pub(crate) fn new(on: impl Fn(TodayEvent) + 'static) -> Rc<Self> {
        let on: Rc<dyn Fn(TodayEvent)> = Rc::new(on);
        let card = Card::new("view-list-symbolic", "Today");
        card.root.add_css_class("mdash-agenda-card");
        let lines_box = gtk::Box::new(gtk::Orientation::Vertical, SPACING);
        lines_box.add_css_class("mdash-today-lines");
        lines_box.set_valign(gtk::Align::Start);
        // The card asks for one row and takes what the left column leaves;
        // the fitter hides what doesn't fit, so nothing ever scrolls or is
        // cut (the scrollbar is never shown).
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroller.set_vscrollbar_policy(gtk::PolicyType::External);
        scroller.set_propagate_natural_height(false);
        scroller.set_min_content_height(MIN_HEIGHT);
        scroller.set_vexpand(true);
        scroller.set_child(Some(&lines_box));
        card.body.set_vexpand(true);
        card.body.append(&scroller);

        let this = Rc::new(Self {
            card,
            scroller,
            lines_box,
            lines: RefCell::new(None),
            on,
            spill: RefCell::new(None),
            pending: Cell::new(false),
            last: RefCell::new(None),
            weak_self: RefCell::new(Weak::new()),
        });
        *this.weak_self.borrow_mut() = Rc::downgrade(&this);
        // A new height (the column got its allocation): fit again.
        let weak = Rc::downgrade(&this);
        this.scroller
            .vadjustment()
            .connect_page_size_notify(move |_| {
                if let Some(t) = weak.upgrade() {
                    t.queue_fit();
                }
            });
        this
    }

    /// The neighbour that takes leftover room (the idle media card).
    pub(crate) fn set_spill(&self, spill: Weak<dyn Spill>) {
        *self.spill.borrow_mut() = Some(spill);
    }

    /// Fill the card: the Events lines for `selected`, the unread
    /// notifications and the read ones before them (`earlier`, filler for
    /// room nothing else needs), newest first.
    pub(crate) fn set_content(
        &self,
        title: &str,
        meta: &str,
        events: &[EventLine],
        notes: &[NoteLine],
        earlier: &[NoteLine],
    ) {
        self.card.set_title(title);
        self.card.set_meta(meta);
        while let Some(child) = self.lines_box.first_child() {
            self.lines_box.remove(&child);
        }
        let mut ev_widgets = Vec::new();
        for line in events {
            let w = event_widget(line);
            self.lines_box.append(&w);
            ev_widgets.push(w);
        }
        let events_more = more_button(&self.on, TodayEvent::OpenCalendar, "Open the calendar");
        self.lines_box.append(&events_more);

        let mut note_widgets: Vec<gtk::Widget> = Vec::new();
        let mut notes_kinds = Vec::new();
        if notes.is_empty() {
            let w = empty_line("No new notifications");
            note_widgets.push(w.upcast::<gtk::Widget>());
            notes_kinds.push(Kind::Empty);
        } else {
            note_widgets.push(self.notes_caption().upcast::<gtk::Widget>());
            notes_kinds.push(Kind::Caption);
            for n in notes.iter().take(MAX_NOTES) {
                note_widgets.push(self.note_widget(n).upcast::<gtk::Widget>());
                notes_kinds.push(Kind::Row);
            }
        }
        for w in &note_widgets {
            self.lines_box.append(w);
        }
        let notes_more = more_button(
            &self.on,
            TodayEvent::OpenNotifications,
            "Open the notification centre",
        );
        self.lines_box.append(&notes_more);

        let mut earlier_widgets: Vec<gtk::Widget> = Vec::new();
        let mut earlier_kinds = Vec::new();
        if !earlier.is_empty() {
            let caption = gtk::Label::new(Some("Earlier"));
            caption.add_css_class("mdash-section-caption");
            caption.set_xalign(0.0);
            earlier_widgets.push(caption.upcast());
            earlier_kinds.push(Kind::Caption);
            for n in earlier.iter().take(MAX_NOTES) {
                let w = self.note_widget(n);
                w.add_css_class("mdash-note-read");
                earlier_widgets.push(w.upcast());
                earlier_kinds.push(Kind::Row);
            }
        }
        for w in &earlier_widgets {
            w.set_visible(false);
            self.lines_box.append(w);
        }

        *self.lines.borrow_mut() = Some(Lines {
            event_kinds: events.iter().map(event_kind).collect(),
            events: ev_widgets,
            events_more,
            notes: note_widgets,
            notes_kinds,
            notes_more,
            notes_beyond: notes.len().saturating_sub(MAX_NOTES),
            earlier: earlier_widgets,
            earlier_kinds,
        });
        *self.last.borrow_mut() = None;
        self.queue_fit();
    }

    fn notes_caption(&self) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("mdash-today-caption-row");
        let l = gtk::Label::new(Some("Notifications"));
        l.add_css_class("mdash-section-caption");
        l.set_xalign(0.0);
        l.set_hexpand(true);
        l.set_valign(gtk::Align::Center);
        row.append(&l);
        let b = gtk::Button::with_label("Mark all read");
        b.add_css_class("mdash-more");
        mshell_common::a11y::name(&b, "Mark all notifications read");
        let on = self.on.clone();
        b.connect_clicked(move |_| on(TodayEvent::MarkAllRead));
        row.append(&b);
        row
    }

    fn note_widget(&self, n: &NoteLine) -> gtk::Button {
        let b = gtk::Button::new();
        b.add_css_class("mdash-today-row");
        b.add_css_class("mdash-note-row");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let icon = match n.icon.as_deref().map(str::trim).filter(|i| !i.is_empty()) {
            Some(i) => mshell_common::notification::build_image(i, NOTE_ICON_SIZE),
            None => {
                let img = gtk::Image::from_icon_name("preferences-system-notifications-symbolic");
                img.set_pixel_size(NOTE_ICON_SIZE);
                img
            }
        };
        icon.add_css_class("mdash-note-icon");
        row.append(&icon);
        let summary = gtk::Label::new(Some(if n.summary.is_empty() {
            &n.app
        } else {
            &n.summary
        }));
        summary.add_css_class("mdash-today-title");
        summary.set_xalign(0.0);
        summary.set_hexpand(true);
        summary.set_ellipsize(gtk::pango::EllipsizeMode::End);
        summary.set_single_line_mode(true);
        row.append(&summary);
        let time = gtk::Label::new(Some(&n.time));
        time.add_css_class("mdash-dim");
        row.append(&time);
        b.set_child(Some(&row));
        let label = if n.app.is_empty() {
            n.summary.clone()
        } else {
            format!("{}: {}", n.app, n.summary)
        };
        b.set_tooltip_text(Some(&label));
        mshell_common::a11y::name(&b, &format!("{label}, open the notification centre"));
        let on = self.on.clone();
        b.connect_clicked(move |_| on(TodayEvent::OpenNotifications));
        b
    }

    /// Fit on the next idle (never inside an allocation).
    pub(crate) fn queue_fit(&self) {
        if self.pending.replace(true) {
            return;
        }
        let weak = self.weak_self.borrow().clone();
        gtk::glib::idle_add_local_once(move || {
            if let Some(t) = weak.upgrade() {
                t.pending.set(false);
                t.fit();
            }
        });
    }

    /// Show the whole rows that fit the card's current height, the
    /// "+N more" lines, and hand any whole-row leftover to the spill.
    pub(crate) fn fit(&self) {
        let avail = self.scroller.height();
        let width = self.scroller.width();
        if avail <= 0 || width <= 0 {
            return;
        }
        let lines = self.lines.borrow();
        let Some(lines) = lines.as_ref() else {
            return;
        };
        // GTK measures a hidden widget as 0 px: a line hidden by the last
        // fit is shown for the measurement (same tick, never painted).
        let measure = |w: &gtk::Widget| {
            if w.is_visible() {
                w.measure(gtk::Orientation::Vertical, width).1
            } else {
                w.set_visible(true);
                let h = w.measure(gtk::Orientation::Vertical, width).1;
                w.set_visible(false);
                h
            }
        };
        let more_h = measure(lines.events_more.upcast_ref());
        let sections = [
            Section {
                items: lines
                    .events
                    .iter()
                    .zip(&lines.event_kinds)
                    .map(|(w, k)| Item {
                        kind: *k,
                        height: measure(w),
                    })
                    .collect(),
                more_height: more_h,
                beyond: 0,
            },
            Section {
                items: lines
                    .notes
                    .iter()
                    .zip(&lines.notes_kinds)
                    .map(|(w, k)| Item {
                        kind: *k,
                        height: measure(w),
                    })
                    .collect(),
                more_height: more_h,
                beyond: lines.notes_beyond,
            },
        ];
        let spill = self.spill.borrow().as_ref().and_then(Weak::upgrade);
        let (row_h, row_gap, available, shown_now) = spill
            .as_ref()
            .map(|s| {
                let (h, g, a) = s.spill_metrics();
                (h, g, a, s.spill_shown())
            })
            .unwrap_or((0, 0, 0, 0));
        // The room this card and the spill share: stable across the
        // relayout the spill itself causes.
        let pool = avail + shown_now as i32 * (row_h + row_gap);
        let plan = fit::plan(&sections, pool, SPACING);
        let spill_rows = if plan.complete() {
            fit::spill_rows(plan.leftover, row_h, row_gap, available)
        } else {
            0
        };
        // Still room once everything is shown and the spill took its
        // rows: the earlier (read) notifications fill it.
        let mut residue = plan.leftover - spill_rows as i32 * (row_h + row_gap);
        let earlier_items: Vec<Item> = lines
            .earlier
            .iter()
            .zip(&lines.earlier_kinds)
            .map(|(w, k)| Item {
                kind: *k,
                height: measure(w),
            })
            .collect();
        let earlier_shown = if plan.complete() {
            fit::filler(&earlier_items, residue, SPACING)
        } else {
            0
        };
        residue -= earlier_items[..earlier_shown]
            .iter()
            .map(|i| i.height + SPACING)
            .sum::<i32>();
        // Whole rows leave up to one row of room: spread it over the gaps
        // (a little air between lines) rather than an empty band below.
        let lines_shown = earlier_shown
            + plan
                .sections
                .iter()
                .map(|p| p.shown + usize::from(p.more > 0))
                .sum::<usize>();
        let spread = if lines_shown > 1 {
            (residue.max(0) / (lines_shown as i32 - 1)).min(MAX_SPREAD)
        } else {
            0
        };
        let state = (plan.clone(), spill_rows, spread, earlier_shown);
        if self.last.borrow().as_ref() == Some(&state) {
            return;
        }
        self.lines_box.set_spacing(SPACING + spread);
        for (i, w) in lines.events.iter().enumerate() {
            w.set_visible(i < plan.sections[0].shown);
        }
        set_more(
            &lines.events_more,
            plan.sections[0].more,
            !lines.event_kinds[..plan.sections[0].shown].contains(&Kind::Row),
            Noun::Events,
        );
        for (i, w) in lines.notes.iter().enumerate() {
            w.set_visible(i < plan.sections[1].shown);
        }
        set_more(
            &lines.notes_more,
            plan.sections[1].more,
            plan.sections[1].shown == 0,
            Noun::Notifications,
        );
        for (i, w) in lines.earlier.iter().enumerate() {
            w.set_visible(i < earlier_shown);
        }
        if let Some(s) = spill {
            s.set_spill(spill_rows);
        }
        *self.last.borrow_mut() = Some(state);
    }
}

fn set_more(b: &gtk::Button, n: usize, alone: bool, noun: Noun) {
    b.set_visible(n > 0);
    if n > 0 {
        b.set_label(&more_label(n, alone, noun));
    }
}

fn empty_line(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("mdash-empty");
    l.add_css_class("mdash-today-empty");
    l.set_xalign(0.0);
    l
}

fn event_widget(line: &EventLine) -> gtk::Widget {
    match line {
        EventLine::Day(day) => {
            let l = gtk::Label::new(Some(day));
            l.add_css_class("mdash-section-caption");
            l.set_xalign(0.0);
            l.upcast()
        }
        EventLine::Empty(text) => empty_line(text).upcast(),
        EventLine::Event {
            time,
            title,
            location,
        } => {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.add_css_class("mdash-today-row");
            let t = gtk::Label::new(Some(time));
            t.add_css_class("calendar-agenda-time");
            t.add_css_class("mdash-today-time");
            t.set_xalign(0.0);
            row.append(&t);
            let name = gtk::Label::new(Some(title));
            name.add_css_class("mdash-today-title");
            name.set_xalign(0.0);
            name.set_hexpand(true);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            name.set_single_line_mode(true);
            row.append(&name);
            if let Some(loc) = location {
                let l = gtk::Label::new(Some(loc));
                l.add_css_class("mdash-dim");
                l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                l.set_max_width_chars(14);
                row.append(&l);
            }
            row.set_tooltip_text(Some(&match location {
                Some(loc) => format!("{time} {title} · {loc}"),
                None => format!("{time} {title}"),
            }));
            row.upcast()
        }
    }
}

/// "20:14" for a notification from today, the weekday before that.
pub(crate) fn note_time(
    at: chrono::DateTime<Local>,
    now: chrono::DateTime<Local>,
    h24: bool,
) -> String {
    if at.date_naive() == now.date_naive() {
        if h24 {
            at.format("%H:%M").to_string()
        } else {
            at.format("%-I:%M %p").to_string()
        }
    } else if (now.date_naive() - at.date_naive()).num_days() < 7 {
        at.format("%a").to_string()
    } else {
        at.format("%-d %b").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, day).expect("date")
    }

    fn ev(summary: &str, day: u32, h: u32) -> mcal::Event {
        let start = Local
            .with_ymd_and_hms(2026, 11, day, h, 0, 0)
            .single()
            .expect("valid")
            .with_timezone(&Utc);
        mcal::Event {
            id: summary.into(),
            calendar_id: "t".into(),
            uid: summary.into(),
            summary: summary.into(),
            description: None,
            location: None,
            url: None,
            status: None,
            start,
            end: start + chrono::Duration::minutes(30),
            all_day: false,
            recurrence: Vec::new(),
            attendees: Vec::new(),
            categories: Vec::new(),
        }
    }

    fn titles(lines: &[EventLine]) -> Vec<String> {
        lines
            .iter()
            .map(|l| match l {
                EventLine::Day(d) => format!("# {d}"),
                EventLine::Event { time, title, .. } => format!("{time} {title}"),
                EventLine::Empty(t) => format!("({t})"),
            })
            .collect()
    }

    #[test]
    fn today_then_upcoming_grouped_by_day() {
        let events = [
            ev("Gym", 11, 7),
            ev("Stand-up", 10, 9),
            ev("Review", 13, 14),
            ev("Lunch", 11, 12),
            ev("Too far", 18, 9),
        ];
        assert_eq!(
            titles(&event_lines(&events, d(10), d(10))),
            [
                "09:00 Stand-up",
                "# Tomorrow",
                "07:00 Gym",
                "12:00 Lunch",
                "# Friday 13 November",
                "14:00 Review",
            ],
            "the 18th is past the seven days"
        );
    }

    #[test]
    fn a_free_today_says_so_before_the_upcoming_days() {
        let events = [ev("Gym", 12, 7)];
        assert_eq!(
            titles(&event_lines(&events, d(10), d(10))),
            ["(No events today)", "# Thursday 12 November", "07:00 Gym"]
        );
        assert_eq!(
            titles(&event_lines(&events, d(9), d(10)))[0],
            "(No events on Monday)"
        );
    }

    #[test]
    fn a_free_week_is_one_line() {
        assert_eq!(
            event_lines(&[], d(10), d(10)),
            [EventLine::Empty("No events this week".into())]
        );
    }

    #[test]
    fn note_times_are_short() {
        let now = Local
            .with_ymd_and_hms(2026, 11, 10, 20, 23, 0)
            .single()
            .expect("valid");
        let at = |day, h| {
            Local
                .with_ymd_and_hms(2026, 11, day, h, 5, 0)
                .single()
                .expect("valid")
        };
        assert_eq!(note_time(at(10, 9), now, true), "09:05");
        assert_eq!(note_time(at(10, 21), now, false), "9:05 PM");
        assert_eq!(note_time(at(9, 9), now, true), "Mon");
        assert_eq!(note_time(at(1, 9), now, true), "1 Nov");
    }

    #[test]
    fn more_reads_naturally() {
        assert_eq!(more_label(1, false, Noun::Events), "+1 more");
        assert_eq!(more_label(12, false, Noun::Notifications), "+12 more");
        assert_eq!(more_label(5, true, Noun::Events), "5 events");
        assert_eq!(more_label(1, true, Noun::Events), "1 event");
        assert_eq!(
            more_label(3, true, Noun::Notifications),
            "3 new notifications"
        );
    }
}
