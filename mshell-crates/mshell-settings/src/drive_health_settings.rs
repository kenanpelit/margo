//! Settings → Widgets → CPU Dashboard → "Drive health": background alerts,
//! trend history and the thresholds the Drives tab colours with
//! (`drive_health.*`). Appended to the generic menu page for
//! `MenuKind::CpuDashboard`, like the Valent mirroring section.

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, DriveHealthConfig};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, WidgetExt};

fn current() -> DriveHealthConfig {
    config_manager().config().drive_health().get_untracked()
}

/// Apply `f` to the config unless it changes nothing.
fn update(f: impl Fn(&mut DriveHealthConfig)) {
    let mut probe = current();
    f(&mut probe);
    if probe != current() {
        config_manager().update_config(move |c| f(&mut c.drive_health));
    }
}

/// An Adwaita action row: title + subtitle on the left; the caller appends
/// the control.
fn row(tag: &str, title: &str, subtitle: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 20);
    row.add_css_class("action-row");
    row.set_widget_name(tag);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_valign(gtk::Align::Center);
    text.set_hexpand(true);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("label-medium-bold");
    t.set_halign(gtk::Align::Start);
    let s = gtk::Label::new(Some(subtitle));
    s.add_css_class("label-small");
    s.set_halign(gtk::Align::Start);
    s.set_xalign(0.0);
    s.set_wrap(true);
    s.set_natural_wrap_mode(gtk::NaturalWrapMode::None);
    text.append(&t);
    text.append(&s);
    row.append(&text);
    row
}

fn switch(
    list: &gtk::Box,
    tag: &str,
    title: &str,
    subtitle: &str,
    get: fn(&DriveHealthConfig) -> bool,
    set: fn(&mut DriveHealthConfig, bool),
) -> gtk::Switch {
    let r = row(tag, title, subtitle);
    let sw = gtk::Switch::new();
    sw.set_valign(gtk::Align::Center);
    sw.set_active(get(&current()));
    sw.connect_active_notify(move |s| {
        let v = s.is_active();
        update(move |c| set(c, v));
    });
    r.append(&sw);
    list.append(&r);
    sw
}

fn spin(
    list: &gtk::Box,
    tag: &str,
    title: &str,
    subtitle: &str,
    range: (f64, f64),
    get: fn(&DriveHealthConfig) -> u32,
    set: fn(&mut DriveHealthConfig, u32),
) -> gtk::SpinButton {
    let r = row(tag, title, subtitle);
    let sp = gtk::SpinButton::with_range(range.0, range.1, 1.0);
    sp.set_valign(gtk::Align::Center);
    sp.set_digits(0);
    sp.set_value(f64::from(get(&current())));
    sp.connect_value_changed(move |s| {
        let v = s.value() as u32;
        update(move |c| set(c, v));
    });
    r.append(&sp);
    list.append(&r);
    sp
}

/// Build the section. The returned scope keeps the "external edit" mirror
/// alive; the owner stores it next to its other effects.
pub(crate) fn build_drive_health_section() -> (gtk::Box, EffectScope) {
    let section = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let title = gtk::Label::new(Some("Drive health"));
    title.add_css_class("label-large-bold");
    title.set_halign(gtk::Align::Start);
    section.append(&title);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.add_css_class("boxed-list");
    section.append(&list);

    let alerts = switch(
        &list,
        "setting:/drive_health/alerts",
        "Drive alerts",
        "Notify when a drive gets hot, goes missing, or a full SMART read (Drives tab) shows a new or worse issue: wear, spare, media errors, bad sectors, a failed self-assessment.",
        |c| c.alerts,
        |c, v| c.alerts = v,
    );
    let recovery = switch(
        &list,
        "setting:/drive_health/notify_recovery",
        "Notify when an issue clears",
        "Also tell me when a drive's issue is back to normal.",
        |c| c.notify_recovery,
        |c, v| c.notify_recovery = v,
    );
    let missing = switch(
        &list,
        "setting:/drive_health/missing_alerts",
        "Missing-drive alerts",
        "Alert when an internal drive that was attached is no longer detected.",
        |c| c.missing_alerts,
        |c, v| c.missing_alerts = v,
    );
    let grace = spin(
        &list,
        "setting:/drive_health/missing_grace_scans",
        "Missing-drive grace",
        "Minutes a drive may be absent before the alert.",
        (1.0, 20.0),
        |c| c.missing_grace_scans,
        |c, v| c.missing_grace_scans = v,
    );
    let warn = spin(
        &list,
        "setting:/drive_health/temp_warning_c",
        "Hot at (°C)",
        "Drive temperature shown amber and alerted as running hot.",
        (30.0, 100.0),
        |c| c.temp_warning_c,
        |c, v| c.temp_warning_c = v,
    );
    let crit = spin(
        &list,
        "setting:/drive_health/temp_critical_c",
        "Critical at (°C)",
        "Drive temperature shown red and alerted as critical.",
        (30.0, 110.0),
        |c| c.temp_critical_c,
        |c, v| c.temp_critical_c = v,
    );
    let wear = spin(
        &list,
        "setting:/drive_health/wear_warning_pct",
        "Wear warning (%)",
        "Rated endurance used (NVMe \"Percentage Used\") that needs attention.",
        (1.0, 100.0),
        |c| c.wear_warning_pct,
        |c, v| c.wear_warning_pct = v,
    );
    let history = switch(
        &list,
        "setting:/drive_health/history",
        "Trend history",
        "Keep temperature samples and every full SMART read, and chart them per drive in the Drives tab.",
        |c| c.history,
        |c, v| c.history = v,
    );
    let interval = spin(
        &list,
        "setting:/drive_health/history_interval_minutes",
        "Sample every (minutes)",
        "How often a temperature sample is saved (15–1440).",
        (15.0, 1440.0),
        |c| c.history_interval_minutes,
        |c, v| c.history_interval_minutes = v,
    );
    let retention = spin(
        &list,
        "setting:/drive_health/history_retention_days",
        "Keep history (days)",
        "Older samples are dropped (1–365).",
        (1.0, 365.0),
        |c| c.history_retention_days,
        |c, v| c.history_retention_days = v,
    );

    // Mirror external edits (a hand-edited profile, `mshellctl config`).
    let mut effects = EffectScope::new();
    effects.push(move |_| {
        let c = config_manager().config().drive_health().get();
        for (sw, v) in [
            (&alerts, c.alerts),
            (&recovery, c.notify_recovery),
            (&missing, c.missing_alerts),
            (&history, c.history),
        ] {
            if sw.is_active() != v {
                sw.set_active(v);
            }
        }
        for (sp, v) in [
            (&grace, c.missing_grace_scans),
            (&warn, c.temp_warning_c),
            (&crit, c.temp_critical_c),
            (&wear, c.wear_warning_pct),
            (&interval, c.history_interval_minutes),
            (&retention, c.history_retention_days),
        ] {
            if sp.value() as u32 != v {
                sp.set_value(f64::from(v));
            }
        }
        // The alert details only matter while alerts are on.
        recovery.set_sensitive(c.alerts);
        missing.set_sensitive(c.alerts);
        grace.set_sensitive(c.alerts && c.missing_alerts);
        interval.set_sensitive(c.history);
        retention.set_sensitive(c.history);
    });

    (section, effects)
}
