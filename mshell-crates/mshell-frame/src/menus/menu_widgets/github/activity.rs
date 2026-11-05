//! The GitHub menu's Activity tab: the contribution heat-map (one column
//! per week, Sunday on top, GitHub's quartile buckets tinted with
//! `--primary`) and the today / streak / best / year figures.

use crate::github::{ContribCalendar, ContribStats};
use relm4::gtk;
use relm4::gtk::prelude::*;

/// `(column, "Nov")` for every week that starts in a new month (and the
/// first week) — the labels over the heat-map. A label with the next one
/// fewer than two columns away is dropped so they never overlap.
pub(crate) fn month_labels(cal: &ContribCalendar) -> Vec<(usize, String)> {
    use chrono::Datelike;
    let mut all: Vec<(usize, String)> = Vec::new();
    let mut last_month = None;
    for (col, week) in cal.weeks.iter().enumerate() {
        let Some(first) = week.first() else { continue };
        if last_month != Some(first.date.month()) {
            all.push((col, first.date.format("%b").to_string()));
            last_month = Some(first.date.month());
        }
    }
    let next_cols: Vec<Option<usize>> = (0..all.len())
        .map(|i| all.get(i + 1).map(|(c, _)| *c))
        .collect();
    all.into_iter()
        .zip(next_cols)
        .filter(|((col, _), next)| next.is_none_or(|n| n - col >= 2))
        .map(|(label, _)| label)
        .collect()
}

fn plural(n: u32, one: &str, many: &str) -> String {
    if n == 1 {
        format!("{n} {one}")
    } else {
        format!("{n} {many}")
    }
}

/// The Activity tab's widgets; rebuilt in place when a calendar arrives.
pub(crate) struct ActivityView {
    pub(crate) root: gtk::Box,
    stats: [gtk::Label; 4],
    grid_scroll: gtk::ScrolledWindow,
    shown: std::cell::RefCell<Option<ContribCalendar>>,
}

impl ActivityView {
    pub(crate) fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.add_css_class("github-activity");
        let stats_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        stats_row.set_homogeneous(true);
        let mk = |caption: &str| {
            let card = gtk::Box::new(gtk::Orientation::Vertical, 2);
            card.add_css_class("github-stat");
            let value = gtk::Label::new(Some("–"));
            value.add_css_class("github-stat-value");
            let cap = gtk::Label::new(Some(caption));
            cap.add_css_class("github-stat-caption");
            card.append(&value);
            card.append(&cap);
            stats_row.append(&card);
            value
        };
        let stats = [
            mk("today"),
            mk("day streak"),
            mk("best streak"),
            mk("this year"),
        ];
        root.append(&stats_row);
        let grid_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .build();
        grid_scroll.add_css_class("github-heatmap-scroll");
        root.append(&grid_scroll);
        Self {
            root,
            stats,
            grid_scroll,
            shown: std::cell::RefCell::new(None),
        }
    }

    pub(crate) fn set_calendar(&self, cal: &ContribCalendar, st: ContribStats) {
        let values = [st.today, st.current_streak, st.best_streak, st.total];
        for (label, v) in self.stats.iter().zip(values) {
            label.set_label(&v.to_string());
        }
        self.stats[0].set_tooltip_text(Some(&plural(st.today, "contribution", "contributions")));
        if self.shown.borrow().as_ref() == Some(cal) {
            return;
        }
        self.grid_scroll.set_child(Some(&heatmap(cal)));
        // Newest week in view: scroll to the right end once laid out.
        let sw = self.grid_scroll.clone();
        gtk::glib::idle_add_local_once(move || {
            let adj = sw.hadjustment();
            adj.set_value(adj.upper());
        });
        self.shown.replace(Some(cal.clone()));
    }
}

fn heatmap(cal: &ContribCalendar) -> gtk::Grid {
    use chrono::Datelike;
    let grid = gtk::Grid::new();
    grid.add_css_class("github-heatmap");
    grid.set_row_spacing(2);
    grid.set_column_spacing(2);
    for (col, name) in month_labels(cal) {
        let l = gtk::Label::new(Some(&name));
        l.add_css_class("github-heatmap-month");
        l.set_xalign(0.0);
        grid.attach(&l, col as i32 + 1, 0, 3, 1);
    }
    for (row, name) in [(1, "Mon"), (3, "Wed"), (5, "Fri")] {
        let l = gtk::Label::new(Some(name));
        l.add_css_class("github-heatmap-day");
        l.set_xalign(0.0);
        grid.attach(&l, 0, row + 1, 1, 1);
    }
    for (col, week) in cal.weeks.iter().enumerate() {
        for day in week {
            let cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            cell.set_css_classes(&["github-heatmap-cell", level_class(day.level)]);
            cell.set_tooltip_text(Some(&format!(
                "{} on {}",
                plural(day.count, "contribution", "contributions"),
                day.date.format("%a %-d %b %Y")
            )));
            let row = day.date.weekday().num_days_from_sunday() as i32;
            grid.attach(&cell, col as i32 + 1, row + 1, 1, 1);
        }
    }
    grid
}

fn level_class(level: u8) -> &'static str {
    match level {
        0 => "l0",
        1 => "l1",
        2 => "l2",
        3 => "l3",
        _ => "l4",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::ContribDay;

    fn week(start: chrono::NaiveDate) -> Vec<ContribDay> {
        (0..7)
            .map(|i| ContribDay {
                date: start + chrono::Days::new(i),
                count: 0,
                level: 0,
            })
            .collect()
    }

    #[test]
    fn month_labels_mark_new_months_without_collisions() {
        // Sundays from 2026-09-20 to 2026-11-01.
        let start = chrono::NaiveDate::from_ymd_opt(2026, 9, 20).unwrap();
        let weeks: Vec<_> = (0..7)
            .map(|w| week(start + chrono::Days::new(7 * w)))
            .collect();
        let cal = ContribCalendar {
            login: String::new(),
            total: 0,
            weeks,
        };
        let labels = month_labels(&cal);
        let names: Vec<(usize, &str)> = labels.iter().map(|(c, n)| (*c, n.as_str())).collect();
        // Sep 20 / Sep 27, Oct 4 … Oct 25, Nov 1.
        assert_eq!(names, [(0, "Sep"), (2, "Oct"), (6, "Nov")]);
        // A one-week month at the start is crowded out by the next label.
        let cal = ContribCalendar {
            weeks: vec![
                week(chrono::NaiveDate::from_ymd_opt(2026, 8, 30).unwrap()),
                week(start),
            ],
            ..cal
        };
        let names: Vec<String> = month_labels(&cal).into_iter().map(|(_, n)| n).collect();
        assert_eq!(names, ["Sep"]);
        assert_eq!(level_class(9), "l4");
    }
}
