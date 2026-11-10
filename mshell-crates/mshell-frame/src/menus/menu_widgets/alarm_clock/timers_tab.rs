//! The Alarm Clock menu's Timers tab: one-line quick entry with a live
//! "Rings at 14:35 (in 10 min) — tea" preview, the preset buttons
//! (+5m … +1h) and the pending timers counting down, each with Cancel.
//! Everything goes through [`crate::timers`], so this tab, the bar pill,
//! the launcher's `timer` prefix and `mshellctl timer` always agree; the
//! menu repaints from the timers' change channel.

use crate::timers::{self, PRESETS, Timer, TimerKind, format};
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ButtonExt, EditableExt, EntryExt, WidgetExt};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub(super) struct TimersTab {
    pub(super) root: gtk::Box,
    list: gtk::Box,
    empty: gtk::Label,
    broken: gtk::Label,
    /// The countdown readouts of the rows on screen.
    rows: RefCell<Vec<RowReadout>>,
    /// The timers' change counter the rows were built for.
    built_for: Cell<Option<u64>>,
}

struct RowReadout {
    timer: Timer,
    time: gtk::Label,
    progress: gtk::ProgressBar,
}

impl TimersTab {
    pub(super) fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        root.add_css_class("alarm-timers");

        // ── quick entry ──
        let entry_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let entry = gtk::Entry::new();
        entry.set_hexpand(true);
        entry.set_placeholder_text(Some("10m tea · 14:30 meeting · yarın 9:00"));
        mshell_common::a11y::name(&entry, "New timer or reminder");
        entry_row.append(&entry);
        let add = gtk::Button::with_label("Add");
        add.add_css_class("ok-button-primary");
        add.add_css_class("ok-button-cell");
        entry_row.append(&add);
        root.append(&entry_row);

        let preview = gtk::Label::new(None);
        preview.add_css_class("label-small");
        preview.add_css_class("alarm-timer-preview");
        preview.set_halign(gtk::Align::Start);
        preview.set_xalign(0.0);
        preview.set_wrap(true);
        preview.set_visible(false);
        root.append(&preview);

        // ── presets ──
        let presets = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        presets.set_homogeneous(true);
        for (secs, label) in PRESETS {
            let b = gtk::Button::with_label(label);
            b.add_css_class("ok-button-surface");
            b.add_css_class("ok-button-cell");
            b.set_tooltip_text(Some(&format!(
                "Start a {} timer (text in the box above becomes its label)",
                format::format_in(*secs)
            )));
            let entry = entry.clone();
            let preview = preview.clone();
            let secs = *secs;
            b.connect_clicked(move |_| {
                let text = entry.text().to_string();
                let label = preset_label(&text, timers::preview(&text).is_ok());
                match timers::add_after(secs, &label) {
                    Ok(_) => {
                        if !label.is_empty() {
                            entry.set_text("");
                        }
                    }
                    Err(e) => show_preview(&preview, &Err(e)),
                }
            });
            presets.append(&b);
        }
        root.append(&presets);

        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

        // ── pending timers ──
        let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
        list.add_css_class("alarm-list");
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroller.set_propagate_natural_height(true);
        scroller.set_child(Some(&list));
        crate::menus::list_fit::mark_menu_list(&scroller, 280);
        root.append(&scroller);

        let empty = gtk::Label::new(Some("No timers — type one above or tap a preset."));
        empty.add_css_class("label-small");
        empty.set_halign(gtk::Align::Center);
        root.append(&empty);

        let broken = gtk::Label::new(None);
        broken.add_css_class("label-small");
        broken.add_css_class("alarm-timer-error");
        broken.set_wrap(true);
        broken.set_xalign(0.0);
        broken.set_visible(false);
        root.append(&broken);

        // Live preview while typing; Enter or Add starts it.
        {
            let preview = preview.clone();
            entry.connect_changed(move |e| {
                let text = e.text();
                if text.trim().is_empty() {
                    preview.set_visible(false);
                } else {
                    show_preview(&preview, &timers::preview(&text));
                }
            });
        }
        let submit: Rc<dyn Fn()> = {
            let entry = entry.clone();
            let preview = preview.clone();
            Rc::new(move || {
                let text = entry.text().to_string();
                if text.trim().is_empty() {
                    return;
                }
                match timers::add(&text) {
                    Ok(_) => entry.set_text(""), // clears the preview too
                    Err(e) => show_preview(&preview, &Err(e)),
                }
            })
        };
        {
            let submit = submit.clone();
            entry.connect_activate(move |_| submit());
        }
        add.connect_clicked(move |_| submit());

        let tab = Self {
            root,
            list,
            empty,
            broken,
            rows: RefCell::new(Vec::new()),
            built_for: Cell::new(None),
        };
        tab.sync();
        tab
    }

    /// Repaint: rebuild the rows when the timers changed, else just tick
    /// the countdowns.
    pub(super) fn sync(&self) {
        let generation = timers::generation();
        if self.built_for.get() != Some(generation) {
            self.built_for.set(Some(generation));
            self.rebuild();
        }
        let now = timers::now_unix();
        for row in self.rows.borrow().iter() {
            row.time
                .set_label(&format::pill_text(row.timer.remaining(now)));
            row.progress.set_fraction(row.timer.progress(now));
        }
    }

    /// Rebuild the rows from the pending timers.
    fn rebuild(&self) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let now = timers::now_unix();
        let pending = timers::list();
        self.empty.set_visible(pending.is_empty());
        self.list.set_visible(!pending.is_empty());
        match timers::broken() {
            Some(error) => {
                self.broken.set_label(&format!(
                    "{} doesn't parse ({error}); timers still ring but aren't saved until it's fixed or moved away.",
                    timers::store_path().display()
                ));
                self.broken.set_visible(true);
            }
            None => self.broken.set_visible(false),
        }
        let mut rows = Vec::with_capacity(pending.len());
        for timer in pending {
            let (row, readout) = timer_row(timer, now);
            self.list.append(&row);
            rows.push(readout);
        }
        *self.rows.borrow_mut() = rows;
    }
}

/// A preset's label: the entry's text when it isn't itself a time
/// ("tea" + `+5m` = a five-minute "tea" timer).
fn preset_label(text: &str, text_is_a_time: bool) -> String {
    if text_is_a_time {
        String::new()
    } else {
        text.trim().to_string()
    }
}

fn show_preview(preview: &gtk::Label, result: &Result<String, String>) {
    match result {
        Ok(text) => {
            preview.set_label(text);
            preview.remove_css_class("alarm-timer-error");
        }
        Err(e) => {
            preview.set_label(e);
            preview.add_css_class("alarm-timer-error");
        }
    }
    preview.set_visible(true);
}

/// One row: kind icon · time left over label + due time · progress ·
/// cancel.
fn timer_row(timer: Timer, now: i64) -> (gtk::Box, RowReadout) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("alarm-row");
    row.add_css_class("alarm-timer-row");

    let icon = gtk::Image::from_icon_name(match timer.kind {
        TimerKind::Reminder => "alarm-symbolic",
        TimerKind::Timer | TimerKind::Snooze => "timer-symbolic",
    });
    icon.set_valign(gtk::Align::Center);
    row.append(&icon);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    let time = gtk::Label::new(Some(&format::pill_text(timer.remaining(now))));
    time.add_css_class("alarm-row-time");
    time.set_halign(gtk::Align::Start);
    text.append(&time);

    let rings = format::rings_phrase(&timers::due_clock(&timer, now));
    let subtitle = match timer.label.as_str() {
        "" => capitalize(&rings),
        label => format!("{label} · {rings}"),
    };
    let sub = gtk::Label::new(Some(&subtitle));
    sub.add_css_class("label-small");
    sub.set_halign(gtk::Align::Start);
    sub.set_xalign(0.0);
    sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&sub);

    let progress = gtk::ProgressBar::new();
    progress.add_css_class("alarm-timer-progress");
    progress.set_fraction(timer.progress(now));
    text.append(&progress);
    row.append(&text);

    let cancel = gtk::Button::from_icon_name("window-close-symbolic");
    cancel.add_css_class("alarm-row-delete");
    cancel.set_valign(gtk::Align::Center);
    mshell_common::a11y::name(&cancel, "Cancel timer");
    let id = timer.id;
    cancel.connect_clicked(move |_| {
        timers::cancel(id);
    });
    row.append(&cancel);

    (
        row,
        RowReadout {
            timer,
            time,
            progress,
        },
    )
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preset_takes_plain_text_as_its_label() {
        assert_eq!(preset_label(" tea ", false), "tea");
        assert_eq!(preset_label("", false), "");
        // A line that is itself a time is left for Enter, not a label.
        assert_eq!(preset_label("10m tea", true), "");
    }

    #[test]
    fn capitalize_only_touches_the_first_letter() {
        assert_eq!(capitalize("rings at 14:35"), "Rings at 14:35");
        assert_eq!(capitalize(""), "");
    }
}
