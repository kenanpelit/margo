//! What the firewall's state means for the user: the unit's state from
//! `systemctl show ufw.service`, the saved state from [`super::rules`],
//! classified into the bar pill's four colours and the fixes to offer.
//! Everything here is read without root.

use super::rules::{Conf, Defaults, RuleSet};

/// The `ufw.service` properties [`super::argv::SHOW_ARGS`] asks for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Unit {
    /// `loaded`, `not-found`, …
    pub load: String,
    /// `active`, `inactive`, `failed`, `activating`, …
    pub active: String,
    /// `exited` for the oneshot ufw unit once it ran.
    pub sub: String,
    /// `enabled`, `disabled`, `static`, `masked`, …
    pub file: String,
}

pub fn parse_show(text: &str) -> Unit {
    let mut u = Unit::default();
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim().to_string();
        match k.trim() {
            "LoadState" => u.load = v,
            "ActiveState" => u.active = v,
            "SubState" => u.sub = v,
            "UnitFileState" => u.file = v,
            _ => {}
        }
    }
    u
}

impl Unit {
    /// systemd has the unit at all.
    pub fn exists(&self) -> bool {
        !self.load.is_empty() && self.load != "not-found"
    }

    pub fn running(&self) -> bool {
        matches!(self.active.as_str(), "active" | "reloading")
    }

    /// Starts at boot (enabled, or pulled in some other fixed way).
    pub fn starts_at_boot(&self) -> bool {
        self.file.starts_with("enabled")
            || matches!(self.file.as_str(), "static" | "generated" | "alias")
    }
}

/// The unit's state as far as it could be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitRead {
    Unit(Unit),
    /// No `systemctl` (a non-systemd system): ufw.conf alone decides.
    NoSystemctl,
    /// `systemctl show` failed or timed out.
    Failed(String),
}

/// Why a file couldn't be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    Missing,
    /// Permission denied: root has to read it ([`crate::Request`] `ufw-status`).
    Denied,
    Other(String),
}

impl ReadError {
    pub fn from_io(e: &std::io::Error) -> ReadError {
        match e.kind() {
            std::io::ErrorKind::NotFound => ReadError::Missing,
            std::io::ErrorKind::PermissionDenied => ReadError::Denied,
            _ => ReadError::Other(e.to_string()),
        }
    }
}

/// Everything read for one look at the firewall.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The `ufw` program is installed.
    pub installed: bool,
    pub conf: Result<Conf, ReadError>,
    pub defaults: Option<Defaults>,
    pub unit: UnitRead,
    pub rules: Result<RuleSet, ReadError>,
}

impl Snapshot {
    /// Nothing read yet.
    pub fn loading() -> Snapshot {
        Snapshot {
            installed: true,
            conf: Err(ReadError::Other("not read yet".into())),
            defaults: None,
            unit: UnitRead::Failed("not read yet".into()),
            rules: Err(ReadError::Other("not read yet".into())),
        }
    }

    pub fn unit(&self) -> Option<&Unit> {
        match &self.unit {
            UnitRead::Unit(u) if u.exists() => Some(u),
            _ => None,
        }
    }
}

/// The pill's colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// On and enforced.
    Green,
    /// Off.
    Red,
    /// On, but not enforced now or not after a reboot.
    Amber,
    /// ufw missing, or its state can't be read.
    Muted,
}

/// The firewall's state in the user's terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// `ENABLED=yes`, the unit active and starting at boot (or no
    /// systemd unit to ask).
    On,
    /// On now, but the unit doesn't start at boot: after a reboot the
    /// rules aren't loaded.
    OnNotAtBoot,
    /// `ENABLED=no`.
    Off,
    /// `ENABLED=yes` but the unit isn't running: the rules may not be
    /// loaded.
    NotRunning,
    /// The unit failed.
    Failed,
    /// ufw isn't installed.
    Missing,
    /// ufw.conf or the unit can't be read.
    Unknown,
}

/// A one-click fix the pill tooltip names and the menu banner offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fix {
    /// `systemctl start ufw.service`.
    StartUnit,
    /// `systemctl enable ufw.service`.
    EnableAtBoot,
}

impl Fix {
    pub fn label(self) -> &'static str {
        match self {
            Fix::StartUnit => "Start ufw",
            Fix::EnableAtBoot => "Start at boot",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Health {
    pub status: Status,
    pub level: Level,
    /// One short line: `On`, `Off`, `On — not running`, …
    pub headline: &'static str,
    /// What it means, a sentence (empty when green).
    pub detail: String,
    pub fixes: Vec<Fix>,
}

pub fn classify(s: &Snapshot) -> Health {
    let health = |status, level, headline, detail: &str, fixes: Vec<Fix>| Health {
        status,
        level,
        headline,
        detail: detail.to_string(),
        fixes,
    };
    if !s.installed {
        return health(
            Status::Missing,
            Level::Muted,
            "Not installed",
            "ufw isn't installed (package ufw).",
            vec![],
        );
    }
    let enabled = match &s.conf {
        Ok(c) => c.enabled,
        Err(_) => None,
    };
    let Some(enabled) = enabled else {
        let why = match &s.conf {
            Err(ReadError::Missing) => "/etc/ufw/ufw.conf is missing.".to_string(),
            Err(ReadError::Denied) => "/etc/ufw/ufw.conf can't be read without root.".to_string(),
            Err(ReadError::Other(e)) => format!("/etc/ufw/ufw.conf can't be read: {e}."),
            Ok(_) => "/etc/ufw/ufw.conf has no ENABLED setting.".to_string(),
        };
        return health(Status::Unknown, Level::Muted, "Unknown", &why, vec![]);
    };
    if !enabled {
        return health(
            Status::Off,
            Level::Red,
            "Off",
            "The firewall is off: no rule is enforced.",
            vec![],
        );
    }
    let unit = match &s.unit {
        UnitRead::NoSystemctl => None,
        UnitRead::Unit(u) if !u.exists() => None,
        UnitRead::Unit(u) => Some(u),
        UnitRead::Failed(e) => {
            return health(
                Status::Unknown,
                Level::Muted,
                "Unknown",
                &format!("ufw is on, but its service state can't be read ({e})."),
                vec![],
            );
        }
    };
    let Some(unit) = unit else {
        return health(Status::On, Level::Green, "On", "", vec![]);
    };
    let boot = unit.starts_at_boot();
    let with_boot = |mut v: Vec<Fix>| {
        if !boot {
            v.push(Fix::EnableAtBoot);
        }
        v
    };
    if unit.running() {
        if boot {
            return health(Status::On, Level::Green, "On", "", vec![]);
        }
        return health(
            Status::OnNotAtBoot,
            Level::Amber,
            "On — not at boot",
            "ufw is on now, but its service doesn't start at boot: after a reboot the rules are not enforced.",
            vec![Fix::EnableAtBoot],
        );
    }
    if unit.active == "failed" {
        return health(
            Status::Failed,
            Level::Amber,
            "On — service failed",
            "ufw is on, but its service failed: the rules may not be enforced.",
            with_boot(vec![Fix::StartUnit]),
        );
    }
    health(
        Status::NotRunning,
        Level::Amber,
        "On — not running",
        if boot {
            "ufw is on, but its service isn't running: the rules may not be enforced."
        } else {
            "ufw is on, but its service isn't running and doesn't start at boot: the rules may not be enforced."
        },
        with_boot(vec![Fix::StartUnit]),
    )
}

impl Health {
    /// Rule changes are refused while ufw is on but its rules aren't
    /// loaded: ufw then writes `user.rules`, fails on the live firewall,
    /// and leaves `user6.rules` unchanged.
    pub fn rules_editable(&self) -> bool {
        matches!(self.status, Status::On | Status::OnNotAtBoot | Status::Off)
    }

    /// The firewall enforces rules right now (the pill's count shows).
    pub fn enforcing(&self) -> bool {
        matches!(self.status, Status::On | Status::OnNotAtBoot)
    }
}

#[cfg(test)]
mod tests {
    use super::super::rules::{parse_conf, parse_rules};
    use super::*;

    /// `systemctl show ufw.service --property=…` on this machine.
    const REAL_SHOW: &str =
        "LoadState=loaded\nActiveState=active\nSubState=exited\nUnitFileState=enabled\n";

    fn snap(enabled: Option<bool>, show: Option<&str>) -> Snapshot {
        Snapshot {
            installed: true,
            conf: Ok(Conf {
                enabled,
                loglevel: None,
            }),
            defaults: None,
            unit: match show {
                Some(t) => UnitRead::Unit(parse_show(t)),
                None => UnitRead::NoSystemctl,
            },
            rules: Ok(RuleSet::default()),
        }
    }

    fn show(active: &str, file: &str) -> String {
        format!("LoadState=loaded\nActiveState={active}\nSubState=dead\nUnitFileState={file}\n")
    }

    #[test]
    fn the_real_state_is_green() {
        let u = parse_show(REAL_SHOW);
        assert!(u.exists() && u.running() && u.starts_at_boot());
        let mut s = snap(None, Some(REAL_SHOW));
        s.conf = Ok(parse_conf(include_str!("../../tests/data/ufw/ufw.conf")));
        s.rules = Ok(parse_rules(
            include_str!("../../tests/data/ufw/user.rules"),
            Some(include_str!("../../tests/data/ufw/user6.rules")),
            true,
        ));
        let h = classify(&s);
        assert_eq!((h.status, h.level), (Status::On, Level::Green));
        assert!(h.fixes.is_empty() && h.enforcing() && h.rules_editable());
    }

    #[test]
    fn every_state_has_its_colour_and_fix() {
        let c = |enabled, show: Option<String>| {
            let h = classify(&snap(enabled, show.as_deref()));
            (h.status, h.level, h.fixes)
        };
        assert_eq!(
            c(Some(false), Some(show("active", "enabled"))),
            (Status::Off, Level::Red, vec![])
        );
        assert_eq!(c(Some(false), None), (Status::Off, Level::Red, vec![]));
        assert_eq!(
            c(Some(true), Some(show("active", "disabled"))),
            (Status::OnNotAtBoot, Level::Amber, vec![Fix::EnableAtBoot])
        );
        assert_eq!(
            c(Some(true), Some(show("inactive", "enabled"))),
            (Status::NotRunning, Level::Amber, vec![Fix::StartUnit])
        );
        assert_eq!(
            c(Some(true), Some(show("inactive", "disabled"))),
            (
                Status::NotRunning,
                Level::Amber,
                vec![Fix::StartUnit, Fix::EnableAtBoot]
            )
        );
        assert_eq!(
            c(Some(true), Some(show("failed", "enabled"))),
            (Status::Failed, Level::Amber, vec![Fix::StartUnit])
        );
        assert_eq!(c(Some(true), Some(show("active", "static"))).0, Status::On);
        assert_eq!(
            c(Some(true), Some(show("active", "enabled-runtime"))).0,
            Status::On
        );
        assert_eq!(
            c(Some(true), Some(show("activating", "enabled"))).0,
            Status::NotRunning
        );
        // No systemd unit, or no systemctl: ufw.conf decides.
        assert_eq!(c(Some(true), None).1, Level::Green);
        assert_eq!(
            c(
                Some(true),
                Some("LoadState=not-found\nActiveState=inactive\n".into())
            )
            .1,
            Level::Green
        );
        assert_eq!(c(None, Some(show("active", "enabled"))).1, Level::Muted);
    }

    #[test]
    fn unreadable_states_are_muted_not_red() {
        let mut s = snap(Some(true), None);
        s.unit = UnitRead::Failed("timed out".into());
        let h = classify(&s);
        assert_eq!((h.status, h.level), (Status::Unknown, Level::Muted));
        assert!(h.detail.contains("timed out"));
        s.conf = Err(ReadError::Denied);
        assert!(classify(&s).detail.contains("without root"));
        s.installed = false;
        assert_eq!(classify(&s).status, Status::Missing);
        assert_eq!(classify(&Snapshot::loading()).level, Level::Muted);
    }

    #[test]
    fn rule_edits_wait_for_a_running_firewall() {
        let h = |active: &str| classify(&snap(Some(true), Some(&show(active, "enabled"))));
        assert!(h("active").rules_editable());
        assert!(!h("inactive").rules_editable());
        assert!(!h("failed").rules_editable());
        assert!(classify(&snap(Some(false), None)).rules_editable());
        assert!(!classify(&snap(Some(false), None)).enforcing());
    }

    #[test]
    fn read_errors_from_io() {
        use std::io::{Error, ErrorKind};
        assert_eq!(
            ReadError::from_io(&Error::from(ErrorKind::NotFound)),
            ReadError::Missing
        );
        assert_eq!(
            ReadError::from_io(&Error::from(ErrorKind::PermissionDenied)),
            ReadError::Denied
        );
    }
}
