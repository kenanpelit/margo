//! Today: the timers & alarms card — the next alarm, the running timers
//! counting down (cancel in place), and the Timers tab's quick entry +
//! presets. Everything goes through [`crate::timers`] (the same parser and
//! store as the Alarm Clock menu, the pill, the launcher and `mshellctl
//! timer`), so they always agree.

use super::super::card::Card;
use crate::timers::{self, PRESETS, Timer, TimerKind, format};
use chrono::{Datelike, Duration as ChronoDuration, NaiveDateTime, NaiveTime};
use mshell_config::schema::config::Alarm;
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ButtonExt, EditableExt, EntryExt, WidgetExt};

/// Running timers listed on the card (the rest: "+N more").
pub(crate) const SHOWN_TIMERS: usize = 2;

/// The next time an enabled alarm rings after `now`, with its name.
/// `repeat_mask` bit `i` = weekday `i` (0 = Sunday); 0 = one-shot (rings
/// at the next occurrence of its time).
pub(crate) fn next_alarm(alarms: &[Alarm], now: NaiveDateTime) -> Option<(NaiveDateTime, String)> {
    alarms
        .iter()
        .filter(|a| a.enabled)
        .filter_map(|a| {
            let time = NaiveTime::from_hms_opt(u32::from(a.hour), u32::from(a.minutes), 0)?;
            (0..=7)
                .map(|d| (now.date() + ChronoDuration::days(d)).and_time(time))
                .filter(|t| *t > now)
                .find(|t| {
                    let weekday = t.weekday().num_days_from_sunday();
                    a.repeat_mask == 0 || a.repeat_mask & (1 << weekday) != 0
                })
                .map(|t| (t, a.name.trim().to_string()))
        })
        .min_by_key(|(t, _)| *t)
}

/// "Next alarm: tomorrow 07:00 · Work".
pub(crate) fn alarm_line(next: Option<&(NaiveDateTime, String)>, now: NaiveDateTime) -> String {
    let Some((at, name)) = next else {
        return "No alarm set".to_string();
    };
    let day = if at.date() == now.date() {
        "today".to_string()
    } else if Some(at.date()) == now.date().succ_opt() {
        "tomorrow".to_string()
    } else {
        at.format("%A").to_string()
    };
    let mut line = format!("Next alarm {day} {}", at.format("%H:%M"));
    if !name.is_empty() {
        line.push_str(" · ");
        line.push_str(name);
    }
    line
}

pub(crate) struct TimersCard {
    pub(crate) card: Card,
    alarm: gtk::Label,
    list: gtk::Box,
    /// The "No timers running" row.
    empty: gtk::Box,
}

/// A one-line dim row: a small leading icon and a label.
fn icon_row(icon: &str) -> (gtk::Box, gtk::Label) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("mdash-icon-row");
    let image = gtk::Image::from_icon_name(icon);
    row.append(&image);
    let label = gtk::Label::new(None);
    label.add_css_class("mdash-dim");
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    row.append(&label);
    (row, label)
}

impl TimersCard {
    pub(crate) fn new() -> Self {
        let card = Card::new("alarm-symbolic", "Timers");
        card.root.add_css_class("mdash-timers-card");

        // Compact: the next alarm and the running timers as single rows
        // (icon + text), then the entry and the presets.
        let (alarm_row, alarm) = icon_row("alarm-symbolic");
        card.body.append(&alarm_row);

        let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
        card.body.append(&list);
        let (empty_row, empty) = icon_row("timer-symbolic");
        empty.set_label("No timers running");
        card.body.append(&empty_row);
        let empty = empty_row;

        let entry_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let entry = gtk::Entry::new();
        entry.add_css_class("mdash-entry");
        entry.set_hexpand(true);
        entry.set_placeholder_text(Some("10m tea · 14:30 call"));
        mshell_common::a11y::name(&entry, "New timer or reminder");
        entry_row.append(&entry);
        card.body.append(&entry_row);

        let preview = gtk::Label::new(None);
        preview.add_css_class("label-small");
        preview.set_xalign(0.0);
        preview.set_ellipsize(gtk::pango::EllipsizeMode::End);
        preview.set_visible(false);
        card.body.append(&preview);

        let presets = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        presets.set_homogeneous(true);
        // +5m +10m +25m +1h: four fit a half-width card.
        for (secs, label) in PRESETS.iter().filter(|(s, _)| *s != 15 * 60) {
            let b = gtk::Button::with_label(label);
            b.add_css_class("mdash-preset");
            b.set_tooltip_text(Some(&format!(
                "Start a {} timer (text in the box becomes its label)",
                format::format_in(*secs)
            )));
            let (entry, secs) = (entry.clone(), *secs);
            b.connect_clicked(move |_| {
                let text = entry.text().to_string();
                let label = if timers::preview(&text).is_ok() {
                    String::new()
                } else {
                    text.trim().to_string()
                };
                if timers::add_after(secs, &label).is_ok() && !label.is_empty() {
                    entry.set_text("");
                }
            });
            presets.append(&b);
        }
        card.body.append(&presets);

        {
            let preview = preview.clone();
            entry.connect_changed(move |e| {
                let text = e.text();
                if text.trim().is_empty() {
                    preview.set_visible(false);
                    return;
                }
                let (line, error) = match timers::preview(&text) {
                    Ok(line) => (line, false),
                    Err(e) => (e, true),
                };
                preview.set_label(&line);
                if error {
                    preview.add_css_class("alarm-timer-error");
                } else {
                    preview.remove_css_class("alarm-timer-error");
                }
                preview.set_visible(true);
            });
        }
        {
            let preview = preview.clone();
            entry.connect_activate(move |e| {
                let text = e.text().to_string();
                if text.trim().is_empty() {
                    return;
                }
                match timers::add(&text) {
                    Ok(_) => e.set_text(""),
                    Err(err) => {
                        preview.set_label(&err);
                        preview.add_css_class("alarm-timer-error");
                        preview.set_visible(true);
                    }
                }
            });
        }

        Self {
            card,
            alarm,
            list,
            empty,
        }
    }

    /// Repaint from the pending timers at `now` (Unix seconds) and the
    /// alarm line.
    pub(crate) fn apply(&self, pending: &[Timer], now: i64, alarm: &str) {
        self.alarm.set_label(alarm);
        self.card.set_meta(&match pending.len() {
            0 => String::new(),
            1 => "1 running".to_string(),
            n => format!("{n} running"),
        });
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        self.empty.set_visible(pending.is_empty());
        self.list.set_visible(!pending.is_empty());
        for timer in pending.iter().take(SHOWN_TIMERS) {
            self.list.append(&timer_row(timer, now));
        }
        if pending.len() > SHOWN_TIMERS {
            let more = gtk::Label::new(Some(&format!(
                "+{} more in Alarm Clock",
                pending.len() - SHOWN_TIMERS
            )));
            more.add_css_class("mdash-dim");
            more.set_xalign(0.0);
            self.list.append(&more);
        }
    }
}

fn timer_row(timer: &Timer, now: i64) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("mdash-timer-row");
    let icon = gtk::Image::from_icon_name(match timer.kind {
        TimerKind::Reminder => "alarm-symbolic",
        TimerKind::Timer | TimerKind::Snooze => "timer-symbolic",
    });
    row.append(&icon);
    // One line: time left · label · a short progress bar · cancel.
    let left = gtk::Label::new(Some(&format::pill_text(timer.remaining(now))));
    left.add_css_class("mdash-timer-left");
    row.append(&left);
    let label = if timer.label.is_empty() {
        format::rings_phrase(&timers::due_clock(timer, now))
    } else {
        timer.label.clone()
    };
    let sub = gtk::Label::new(Some(&label));
    sub.add_css_class("mdash-dim");
    sub.set_xalign(0.0);
    sub.set_hexpand(true);
    sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
    row.append(&sub);
    let progress = gtk::ProgressBar::new();
    progress.add_css_class("mdash-timer-progress");
    progress.set_valign(gtk::Align::Center);
    progress.set_fraction(timer.progress(now));
    row.append(&progress);
    let cancel = gtk::Button::from_icon_name("window-close-symbolic");
    cancel.add_css_class("mdash-icon-button");
    cancel.set_valign(gtk::Align::Center);
    mshell_common::a11y::name(&cancel, "Cancel timer");
    let id = timer.id;
    cancel.connect_clicked(move |_| {
        timers::cancel(id);
    });
    row.append(&cancel);
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(d: u32, h: u32, m: u32) -> NaiveDateTime {
        // 2026-11-10 is a Tuesday.
        NaiveDate::from_ymd_opt(2026, 11, d)
            .and_then(|d| d.and_hms_opt(h, m, 0))
            .expect("valid")
    }

    fn alarm(hour: u8, minutes: u8, mask: u8, enabled: bool, name: &str) -> Alarm {
        Alarm {
            hour,
            minutes,
            name: name.to_string(),
            enabled,
            repeat_mask: mask,
        }
    }

    #[test]
    fn a_one_shot_rings_at_its_next_occurrence() {
        let now = at(10, 20, 0);
        let a = [alarm(7, 0, 0, true, "")];
        assert_eq!(next_alarm(&a, now).map(|x| x.0), Some(at(11, 7, 0)));
        let a = [alarm(21, 30, 0, true, "")];
        assert_eq!(next_alarm(&a, now).map(|x| x.0), Some(at(10, 21, 30)));
    }

    #[test]
    fn a_weekday_alarm_skips_the_weekend() {
        // Friday 13 Nov 2026, 08:00; weekdays (Mon–Fri) at 07:00.
        let weekdays = 0b0011_1110;
        let a = [alarm(7, 0, weekdays, true, "Work")];
        let next = next_alarm(&a, at(13, 8, 0)).expect("rings");
        assert_eq!(next.0, at(16, 7, 0), "Monday");
        assert_eq!(next.1, "Work");
    }

    #[test]
    fn disabled_alarms_never_ring_and_the_soonest_wins() {
        let now = at(10, 6, 0);
        let a = [
            alarm(6, 30, 0, false, "off"),
            alarm(9, 0, 0, true, "late"),
            alarm(7, 0, 0, true, "early"),
        ];
        assert_eq!(next_alarm(&a, now).map(|x| x.1), Some("early".to_string()));
        assert_eq!(next_alarm(&[alarm(6, 30, 0, false, "")], now), None);
    }

    #[test]
    fn alarm_line_names_the_day() {
        let now = at(10, 20, 0);
        assert_eq!(alarm_line(None, now), "No alarm set");
        assert_eq!(
            alarm_line(Some(&(at(11, 7, 0), "Work".into())), now),
            "Next alarm tomorrow 07:00 · Work"
        );
        assert_eq!(
            alarm_line(Some(&(at(10, 22, 15), String::new())), now),
            "Next alarm today 22:15"
        );
        assert_eq!(
            alarm_line(Some(&(at(13, 7, 0), String::new())), now),
            "Next alarm Friday 07:00"
        );
    }
}
