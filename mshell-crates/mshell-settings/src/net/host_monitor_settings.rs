//! Settings → Widgets → Host Monitor → "Checks": how often the Host
//! Monitor pings (`network.host_monitor.interval_seconds`), how long it
//! waits for a reply (`timeout_seconds`) and which changes notify
//! (`notify_down`, `notify_recovery`). The host list itself is edited in
//! the Host Monitor menu. Appended to the generic menu page for
//! `MenuKind::HostMonitor`, like the Valent page's mirroring options.

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    ConfigStoreFields, HostMonitorConfig, NetworkConfigStoreFields,
};
use mshell_utils::host_monitor::{INTERVAL_RANGE, TIMEOUT_RANGE};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, WidgetExt};

fn current() -> HostMonitorConfig {
    config_manager()
        .config()
        .network()
        .host_monitor()
        .get_untracked()
}

/// Write one field when it differs from the stored value.
fn save(differs: impl Fn(&HostMonitorConfig) -> bool, apply: impl FnOnce(&mut HostMonitorConfig)) {
    if differs(&current()) {
        config_manager().update_config(move |c| apply(&mut c.network.host_monitor));
    }
}

/// Build the section. The returned scope keeps the "external edit" mirror
/// alive; the owner stores it next to its other effects.
pub(crate) fn build_host_monitor_section() -> (gtk::Box, EffectScope) {
    let cfg = current();
    let section = gtk::Box::new(gtk::Orientation::Vertical, 8);

    let title = gtk::Label::new(Some("Checks"));
    title.add_css_class("label-large-bold");
    title.set_halign(gtk::Align::Start);
    section.append(&title);

    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.add_css_class("boxed-list");
    section.append(&list);

    let interval = spin(INTERVAL_RANGE, 5.0, cfg.interval_seconds);
    interval.connect_value_changed(|s| {
        let v = s.value() as u32;
        save(|c| c.interval_seconds != v, move |c| c.interval_seconds = v);
    });
    let r = row(
        "setting:/network/host_monitor/interval_seconds",
        "Check every (seconds)",
        "How often each host is pinged while the Host Monitor pill or menu is shown.",
    );
    r.append(&interval);
    list.append(&r);

    let timeout = spin(TIMEOUT_RANGE, 1.0, cfg.timeout_seconds);
    timeout.connect_value_changed(|s| {
        let v = s.value() as u32;
        save(|c| c.timeout_seconds != v, move |c| c.timeout_seconds = v);
    });
    let r = row(
        "setting:/network/host_monitor/timeout_seconds",
        "Reply timeout (seconds)",
        "How long to wait for an answer. Each check sends three pings, so one lost packet is not an outage.",
    );
    r.append(&timeout);
    list.append(&r);

    let down = switch(cfg.notify_down);
    down.connect_active_notify(|s| {
        let v = s.is_active();
        save(|c| c.notify_down != v, move |c| c.notify_down = v);
    });
    let r = row(
        "setting:/network/host_monitor/notify_down",
        "Notify when a host goes down",
        "A notification when a host stops answering (also for one that is down when the shell starts).",
    );
    r.append(&down);
    list.append(&r);

    let recovery = switch(cfg.notify_recovery);
    recovery.connect_active_notify(|s| {
        let v = s.is_active();
        save(|c| c.notify_recovery != v, move |c| c.notify_recovery = v);
    });
    let r = row(
        "setting:/network/host_monitor/notify_recovery",
        "Notify when a host is back",
        "Replaces the host's down notification when it answers again.",
    );
    r.append(&recovery);
    list.append(&r);

    // Mirror external edits (a hand-edited profile, `mshellctl config`).
    let mut effects = EffectScope::new();
    let (interval_c, timeout_c, down_c, recovery_c) = (
        interval.clone(),
        timeout.clone(),
        down.clone(),
        recovery.clone(),
    );
    effects.push(move |_| {
        let c = config_manager().config().network().host_monitor().get();
        if interval_c.value() as u32 != c.interval_seconds {
            interval_c.set_value(f64::from(c.interval_seconds));
        }
        if timeout_c.value() as u32 != c.timeout_seconds {
            timeout_c.set_value(f64::from(c.timeout_seconds));
        }
        if down_c.is_active() != c.notify_down {
            down_c.set_active(c.notify_down);
        }
        if recovery_c.is_active() != c.notify_recovery {
            recovery_c.set_active(c.notify_recovery);
        }
    });

    (section, effects)
}

fn spin((lo, hi): (u32, u32), step: f64, value: u32) -> gtk::SpinButton {
    let s = gtk::SpinButton::with_range(f64::from(lo), f64::from(hi), step);
    s.set_digits(0);
    s.set_valign(gtk::Align::Center);
    s.set_value(f64::from(value.clamp(lo, hi)));
    s
}

fn switch(on: bool) -> gtk::Switch {
    let s = gtk::Switch::new();
    s.set_valign(gtk::Align::Center);
    s.set_active(on);
    s
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
