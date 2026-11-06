//! Settings → Widgets → Control Center → "Logo LED automation": the
//! `control_center.logo_led` rules (off while locked, off during Do Not
//! Disturb, blink on notifications) and whether they can run here. They
//! write the LED directly, so they need its `brightness` writable by the
//! user — the section shows the udev rule that does that when it isn't.

use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::LogoLedAutomation;
use reactive_graph::prelude::GetUntracked;
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, WidgetExt};
use std::path::Path;

/// The LED's brightness file.
const BRIGHTNESS: &str = "/sys/class/leds/tpacpi::lid_logo_dot/brightness";

/// The udev rule that lets the `video` group write the LED (shown, never
/// installed by mshell).
pub(crate) const UDEV_RULE: &str = "# /etc/udev/rules.d/90-thinkpad-logo-led.rules\n\
ACTION==\"add\", SUBSYSTEM==\"leds\", KERNEL==\"tpacpi::lid_logo_dot\", \
RUN+=\"/bin/chgrp video /sys/class/leds/%k/brightness\", \
RUN+=\"/bin/chmod g+w /sys/class/leds/%k/brightness\"";

/// Whether the automation can run on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    NoLed,
    ReadOnly,
    Writable,
}

/// Probe `brightness`: opening it for writing writes nothing.
pub(crate) fn access(brightness: &Path) -> Access {
    if !brightness.exists() {
        return Access::NoLed;
    }
    match std::fs::OpenOptions::new().write(true).open(brightness) {
        Ok(_) => Access::Writable,
        Err(_) => Access::ReadOnly,
    }
}

fn toggle(
    title: &str,
    subtitle: &str,
    initial: bool,
    set: fn(&mut LogoLedAutomation, bool),
) -> (gtk::Box, gtk::Switch) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
    col.set_hexpand(true);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("label-medium");
    t.set_halign(gtk::Align::Start);
    let s = gtk::Label::new(Some(subtitle));
    s.add_css_class("label-small");
    s.set_halign(gtk::Align::Start);
    s.set_xalign(0.0);
    s.set_wrap(true);
    col.append(&t);
    col.append(&s);
    row.append(&col);
    let sw = gtk::Switch::new();
    sw.set_valign(gtk::Align::Center);
    sw.set_active(initial);
    sw.connect_active_notify(move |sw| {
        let v = sw.is_active();
        config_manager().update_config(move |c| set(&mut c.control_center.logo_led, v));
    });
    row.append(&sw);
    (row, sw)
}

/// The status line under the switches.
pub(crate) fn status_text(a: Access) -> &'static str {
    match a {
        Access::NoLed => "This machine has no ThinkPad logo LED (tpacpi::lid_logo_dot).",
        Access::ReadOnly => {
            "Inactive: these rules switch the LED directly, so its brightness must be writable \
             by you — otherwise every lock or notification would ask for your password. Add \
             this udev rule as root, then reboot (or run `udevadm trigger -s leds -c add`):"
        }
        Access::Writable => "The LED is writable: the rules you turn on apply right away.",
    }
}

pub(crate) fn build_logo_led_section() -> gtk::Box {
    let section = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let label = gtk::Label::new(Some("Logo LED automation"));
    label.add_css_class("label-large-bold");
    label.set_halign(gtk::Align::Start);
    section.append(&label);

    let cur = config_manager()
        .config()
        .get_untracked()
        .control_center
        .logo_led;
    let a = access(Path::new(BRIGHTNESS));
    let rows = [
        toggle(
            "Off while locked",
            "Turn the ThinkPad logo LED off while the screen is locked; restore it on unlock.",
            cur.off_while_locked,
            |c, v| c.off_while_locked = v,
        ),
        toggle(
            "Off during Do Not Disturb",
            "Turn it off while Do Not Disturb is on; restore it after.",
            cur.off_while_dnd,
            |c, v| c.off_while_dnd = v,
        ),
        toggle(
            "Blink on notifications",
            "Blink it briefly when a notification arrives (not during Do Not Disturb or while locked).",
            cur.blink_on_notification,
            |c, v| c.blink_on_notification = v,
        ),
    ];
    for (row, sw) in rows {
        sw.set_sensitive(a != Access::NoLed);
        section.append(&row);
    }

    let status = gtk::Label::new(Some(status_text(a)));
    status.add_css_class("label-small");
    status.add_css_class("dim-label");
    status.set_halign(gtk::Align::Start);
    status.set_xalign(0.0);
    status.set_wrap(true);
    section.append(&status);
    if a == Access::ReadOnly {
        let rule = gtk::Label::new(Some(UDEV_RULE));
        rule.add_css_class("label-small");
        rule.add_css_class("settings-code");
        rule.set_halign(gtk::Align::Start);
        rule.set_xalign(0.0);
        rule.set_wrap(true);
        rule.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        rule.set_selectable(true);
        section.append(&rule);
    }
    section
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn access_tells_missing_read_only_and_writable_apart() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("brightness");
        assert_eq!(access(&f), Access::NoLed);
        std::fs::write(&f, "0\n").unwrap();
        assert_eq!(access(&f), Access::Writable);
        assert_eq!(
            std::fs::read_to_string(&f).unwrap(),
            "0\n",
            "probe wrote nothing"
        );
        // As root everything is writable; the read-only case needs a user.
        use std::os::unix::fs::MetadataExt;
        if std::fs::metadata("/proc/self")
            .map(|m| m.uid())
            .unwrap_or(0)
            != 0
        {
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o444)).unwrap();
            assert_eq!(access(&f), Access::ReadOnly);
        }
    }

    #[test]
    fn the_rule_targets_only_the_logo_led_and_the_status_says_why() {
        assert!(UDEV_RULE.contains("KERNEL==\"tpacpi::lid_logo_dot\""));
        assert!(UDEV_RULE.contains("chmod g+w /sys/class/leds/%k/brightness"));
        assert!(status_text(Access::ReadOnly).contains("udev rule"));
        assert!(status_text(Access::NoLed).contains("no ThinkPad logo LED"));
    }
}
