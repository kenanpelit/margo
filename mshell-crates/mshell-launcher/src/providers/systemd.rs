//! `svc <query>` — find systemd units and start / stop / restart them.
//!
//! Ported from noctalia's `systemd` plugin, rebuilt native. Units from
//! both the **user** and the **system** manager are listed together with
//! their live state on every row; failed units sort first, then running
//! ones, then the rest.
//!
//! | Query | Result |
//! |---|---|
//! | `svc` | every unit, failed → active → inactive |
//! | `svc <filter>` | fuzzy match on unit name (description counts half) |
//! | `svc <unit> ` | that unit's actions: Stop / Restart when running, Start otherwise |
//!
//! Enter on a unit row drills into its actions (the entry becomes
//! `svc <unit> `); Ctrl+Enter runs the default action straight away
//! (Restart for a running unit, Start for a stopped/failed one).
//!
//! System units are controlled with a plain `systemctl <verb> <unit>`:
//! systemd asks polkit, which prompts through the session's polkit agent —
//! never sudo. The action runs on a worker thread and ends with a desktop
//! notification carrying the unit's new state, or systemctl's error text
//! (e.g. "Interactive authentication required." when no agent runs).
//!
//! Listing needs four `systemctl … -o json` calls, so the snapshot is
//! built off the GTK thread through [`BgCache`] and the rows repaint when
//! it lands.

use crate::providers::bg_cache::BgCache;
use crate::{
    item::LauncherItem,
    notify::toast,
    provider::{Provider, RefreshNotifier},
    scoring::{fuzzy_score, make_matcher},
};
use serde::Deserialize;
use std::process::Command;
use std::rc::Rc;
use std::time::Duration;

const NAME: &str = "Systemd";
const PREFIX: &str = "svc";
/// Rows built per query; a filter narrows well below this.
const MAX_RESULTS: usize = 60;
/// Unit list freshness. Unit states change rarely while the launcher is
/// open; panel-open invalidates it anyway.
const SNAPSHOT_TTL: Duration = Duration::from_secs(10);

/// Which manager a unit belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    User,
    System,
}

impl Scope {
    fn label(self) -> &'static str {
        match self {
            Scope::User => "user",
            Scope::System => "system",
        }
    }

    fn from_label(s: &str) -> Option<Self> {
        match s {
            "user" => Some(Scope::User),
            "system" => Some(Scope::System),
            _ => None,
        }
    }
}

/// One listed unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    pub name: String,
    pub scope: Scope,
    /// `ActiveState`: active / inactive / failed / activating / …
    pub active: String,
    /// `SubState`: running / dead / exited / …
    pub sub: String,
    pub description: String,
}

impl Unit {
    fn is_running(&self) -> bool {
        matches!(self.active.as_str(), "active" | "activating" | "reloading")
    }

    fn state_text(&self) -> String {
        if !self.sub.is_empty() && self.sub != self.active {
            format!("{} ({})", self.active, self.sub)
        } else {
            self.active.clone()
        }
    }

    fn subtitle(&self) -> String {
        let mut parts = vec![self.scope.label().to_string(), self.state_text()];
        if !self.description.is_empty() {
            parts.push(self.description.clone());
        }
        parts.join(" · ")
    }

    fn icon(&self) -> &'static str {
        match self.active.as_str() {
            "active" => "emblem-ok-symbolic",
            "failed" => "dialog-error-symbolic",
            "activating" | "deactivating" | "reloading" => "content-loading-symbolic",
            _ => "system-run-symbolic",
        }
    }

    /// Browse-order rank: failed first, then running, then the rest.
    fn rank(&self) -> u8 {
        match self.active.as_str() {
            "failed" => 3,
            "active" => 2,
            _ => 1,
        }
    }
}

/// A `systemctl` verb the provider offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Start,
    Stop,
    Restart,
}

impl Verb {
    fn as_str(self) -> &'static str {
        match self {
            Verb::Start => "start",
            Verb::Stop => "stop",
            Verb::Restart => "restart",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Verb::Start => "Start",
            Verb::Stop => "Stop",
            Verb::Restart => "Restart",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Verb::Start => "media-playback-start-symbolic",
            Verb::Stop => "media-playback-stop-symbolic",
            Verb::Restart => "view-refresh-symbolic",
        }
    }
}

/// Actions offered for a unit: a running unit can be stopped or restarted,
/// anything else (inactive, failed) can be started.
pub fn verbs_for(unit: &Unit) -> Vec<Verb> {
    if unit.is_running() {
        vec![Verb::Stop, Verb::Restart]
    } else {
        vec![Verb::Start]
    }
}

/// The Ctrl+Enter shortcut: Restart a running unit, Start anything else.
pub fn default_verb(unit: &Unit) -> Verb {
    if unit.is_running() {
        Verb::Restart
    } else {
        Verb::Start
    }
}

/// argv for `systemctl` in `scope`. System units go through plain
/// `systemctl`, which authorises via polkit (interactive auth is allowed
/// by default, so the session's polkit agent prompts).
pub fn systemctl_argv(scope: Scope, args: &[&str]) -> Vec<String> {
    let mut argv = vec!["systemctl".to_string()];
    if scope == Scope::User {
        argv.push("--user".into());
    }
    argv.extend(args.iter().map(|a| a.to_string()));
    argv
}

#[derive(Deserialize)]
struct UnitFileRow {
    unit_file: Option<String>,
    state: Option<String>,
}

#[derive(Deserialize)]
struct UnitRow {
    unit: Option<String>,
    load: Option<String>,
    active: Option<String>,
    sub: Option<String>,
    description: Option<String>,
}

/// Merge one scope's `list-unit-files -o json` and `list-units --all -o
/// json` output. Unit files give every startable unit (as inactive/dead);
/// list-units overlays live state and adds instances of templates.
/// Templates (`foo@.service`), aliases and masked units are dropped —
/// none of the offered actions apply to them. Malformed JSON yields
/// whatever the other half produced.
pub fn parse_scope(scope: Scope, files_json: &str, units_json: &str) -> Vec<Unit> {
    let mut map: std::collections::BTreeMap<String, Unit> = std::collections::BTreeMap::new();
    let files: Vec<UnitFileRow> = serde_json::from_str(files_json).unwrap_or_default();
    for f in files {
        let (Some(name), state) = (f.unit_file, f.state.unwrap_or_default()) else {
            continue;
        };
        if name.contains("@.") || matches!(state.as_str(), "alias" | "masked" | "masked-runtime") {
            continue;
        }
        map.insert(
            name.clone(),
            Unit {
                name,
                scope,
                active: "inactive".into(),
                sub: "dead".into(),
                description: String::new(),
            },
        );
    }
    let units: Vec<UnitRow> = serde_json::from_str(units_json).unwrap_or_default();
    for u in units {
        let Some(name) = u.unit else { continue };
        let load = u.load.unwrap_or_default();
        if load == "not-found" || load == "masked" {
            continue;
        }
        let entry = map.entry(name.clone()).or_insert_with(|| Unit {
            name,
            scope,
            active: String::new(),
            sub: String::new(),
            description: String::new(),
        });
        entry.active = u.active.unwrap_or_default();
        entry.sub = u.sub.unwrap_or_default();
        entry.description = u.description.unwrap_or_default();
    }
    map.into_values().collect()
}

/// Rank `units` for `filter`. Empty filter → failed, then active, then the
/// rest, alphabetical within a rank. Otherwise a fuzzy match on the name;
/// a description-only match counts half so name hits lead.
pub fn rank_units<'a>(units: &'a [Unit], filter: &str) -> Vec<(&'a Unit, f64)> {
    let mut matcher = make_matcher();
    let mut out: Vec<(&Unit, f64)> = units
        .iter()
        .filter_map(|u| {
            if filter.is_empty() {
                return Some((u, u.rank() as f64));
            }
            fuzzy_score(&mut matcher, filter, &u.name)
                .or_else(|| fuzzy_score(&mut matcher, filter, &u.description).map(|s| s * 0.5))
                .map(|s| (u, s))
        })
        .collect();
    out.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.name.cmp(&b.0.name))
            .then_with(|| a.0.scope.cmp(&b.0.scope))
    });
    out.truncate(MAX_RESULTS);
    out
}

/// What a `svc …` query asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum SvcQuery<'a> {
    /// List units matching the filter.
    List(&'a str),
    /// Show the actions of the named unit, filtered by the remainder.
    Actions { unit: &'a str, filter: &'a str },
}

/// Split the text after `svc`. A token followed by whitespace that names
/// a known unit is a drill-in (`svc sshd.service ` → its actions); any
/// other text is a list filter.
pub fn parse_query<'a>(rest: &'a str, is_unit: impl Fn(&str) -> bool) -> SvcQuery<'a> {
    let rest = rest.trim_start();
    if let Some((head, tail)) = rest.split_once(char::is_whitespace)
        && is_unit(head)
    {
        return SvcQuery::Actions {
            unit: head,
            filter: tail.trim(),
        };
    }
    SvcQuery::List(rest.trim())
}

/// Summary + body of the result notification.
pub fn outcome_message(
    verb: Verb,
    unit: &str,
    success: bool,
    stderr: &str,
    new_state: &str,
) -> (String, String) {
    if success {
        let state = new_state.trim();
        let state = if state.is_empty() { "unknown" } else { state };
        (
            format!("{} {unit}", verb.title()),
            format!("{unit} is now {state}"),
        )
    } else {
        let err = stderr
            .lines()
            .map(str::trim)
            .rfind(|l| !l.is_empty())
            .unwrap_or("systemctl failed");
        (format!("{} {unit} failed", verb.title()), err.to_string())
    }
}

/// Build the unit list for both managers (blocking; runs on a worker).
fn snapshot() -> Snapshot {
    let mut units = Vec::new();
    let mut any_ok = false;
    for scope in [Scope::User, Scope::System] {
        let run = |sub: &str, all: bool| -> Option<String> {
            let mut args = vec![sub, "--plain", "--no-pager", "--type=service", "-o", "json"];
            if all {
                args.push("--all");
            }
            let argv = systemctl_argv(scope, &args);
            let out = Command::new(&argv[0]).args(&argv[1..]).output().ok()?;
            out.status
                .success()
                .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
        };
        let files = run("list-unit-files", false);
        let live = run("list-units", true);
        any_ok |= files.is_some() || live.is_some();
        units.extend(parse_scope(
            scope,
            files.as_deref().unwrap_or("[]"),
            live.as_deref().unwrap_or("[]"),
        ));
    }
    units.sort_by(|a, b| a.name.cmp(&b.name).then(a.scope.cmp(&b.scope)));
    Snapshot {
        units,
        systemctl_ok: any_ok,
    }
}

#[derive(Debug, Clone, Default)]
struct Snapshot {
    units: Vec<Unit>,
    /// False when every systemctl call failed (missing binary / no bus).
    systemctl_ok: bool,
}

/// Run `systemctl <verb> <unit>` on a worker thread and notify with the
/// unit's new state (or the error text).
fn run_action(scope: Scope, verb: Verb, unit: String) {
    std::thread::spawn(move || {
        let argv = systemctl_argv(scope, &[verb.as_str(), &unit]);
        let (success, stderr) = match Command::new(&argv[0]).args(&argv[1..]).output() {
            Ok(out) => (
                out.status.success(),
                String::from_utf8_lossy(&out.stderr).into_owned(),
            ),
            Err(err) => (false, err.to_string()),
        };
        let state = if success {
            let argv = systemctl_argv(scope, &["is-active", &unit]);
            Command::new(&argv[0])
                .args(&argv[1..])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default()
        } else {
            String::new()
        };
        if !success {
            tracing::warn!(unit, verb = verb.as_str(), %stderr, "systemd: action failed");
        }
        let (title, body) = outcome_message(verb, &unit, success, &stderr, &state);
        toast(title, body);
    });
}

pub struct SystemdProvider {
    cache: BgCache<Snapshot>,
    set_search: Option<Rc<dyn Fn(&str) + 'static>>,
}

impl SystemdProvider {
    pub fn new() -> Self {
        Self {
            cache: BgCache::new(SNAPSHOT_TTL),
            set_search: None,
        }
    }

    /// Wire the search-entry setter used for the unit → actions drill-in.
    pub fn with_search_setter(mut self, set_search: Rc<dyn Fn(&str) + 'static>) -> Self {
        self.set_search = Some(set_search);
        self
    }

    #[cfg(test)]
    fn seeded(units: Vec<Unit>) -> Self {
        let p = Self::new();
        p.cache.seed(Snapshot {
            units,
            systemctl_ok: true,
        });
        p
    }

    fn hint(id: &str, name: &str, description: &str, icon: &str) -> LauncherItem {
        LauncherItem {
            id: format!("svc:{id}"),
            name: name.into(),
            description: description.into(),
            icon: icon.into(),
            icon_is_path: false,
            score: 100.0,
            provider_name: NAME.into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }
    }

    fn unit_row(&self, u: &Unit, score: f64) -> LauncherItem {
        let setter = self.set_search.clone();
        let name = u.name.clone();
        let scope = u.scope;
        let verb = default_verb(u);
        LauncherItem {
            id: format!("svc:unit:{}:{}", u.scope.label(), u.name),
            name: u.name.clone(),
            description: u.subtitle(),
            icon: u.icon().into(),
            icon_is_path: false,
            score: 150.0 + score,
            provider_name: NAME.into(),
            usage_key: Some(format!("svc:{}:{}", u.scope.label(), u.name)),
            on_activate: Rc::new(move || match &setter {
                // Drill into the unit's actions.
                Some(set) => set(&format!("{PREFIX} {name} ")),
                // No UI wiring (tests / standalone): run the default.
                None => run_action(scope, verb, name.clone()),
            }),
        }
    }

    fn action_rows(&self, units: &[Unit], unit: &str, filter: &str) -> Vec<LauncherItem> {
        let filter = filter.to_ascii_lowercase();
        let mut rows = Vec::new();
        for u in units.iter().filter(|u| u.name == unit) {
            for verb in verbs_for(u) {
                if !filter.is_empty() && !verb.as_str().contains(&filter) {
                    continue;
                }
                let name = u.name.clone();
                let scope = u.scope;
                rows.push(LauncherItem {
                    id: format!("svc:act:{}:{}:{}", verb.as_str(), scope.label(), u.name),
                    name: format!("{} {} ({})", verb.title(), u.name, scope.label()),
                    description: u.subtitle(),
                    icon: verb.icon().into(),
                    icon_is_path: false,
                    score: 200.0 - rows.len() as f64,
                    provider_name: NAME.into(),
                    usage_key: None,
                    on_activate: Rc::new(move || run_action(scope, verb, name.clone())),
                });
            }
        }
        rows
    }
}

impl Default for SystemdProvider {
    fn default() -> Self {
        Self::new()
    }
}

fn strip_prefix(query: &str) -> Option<&str> {
    let q = query.trim_start();
    if q == PREFIX {
        Some("")
    } else {
        q.strip_prefix(PREFIX)
            .filter(|r| r.starts_with(char::is_whitespace))
    }
}

impl Provider for SystemdProvider {
    fn name(&self) -> &str {
        NAME
    }

    fn category(&self) -> &str {
        "System"
    }

    fn handles_search(&self) -> bool {
        // Hundreds of units would drown app results; prefix only.
        false
    }

    fn handles_command(&self, query: &str) -> bool {
        strip_prefix(query).is_some()
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![Self::hint(
            "palette",
            PREFIX,
            "Start / stop / restart systemd units (user + system)",
            "system-run-symbolic",
        )]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let Some(rest) = strip_prefix(query) else {
            return Vec::new();
        };
        let Some(snap) = self.cache.get(snapshot) else {
            return vec![Self::hint(
                "loading",
                "Loading units…",
                "Asking systemctl for user and system units",
                "content-loading-symbolic",
            )];
        };
        if !snap.systemctl_ok {
            return vec![Self::hint(
                "missing",
                "systemctl unavailable",
                "Could not list units — is systemd running and systemctl on PATH?",
                "dialog-warning-symbolic",
            )];
        }
        let units = &snap.units;
        match parse_query(rest, |n| units.iter().any(|u| u.name == n)) {
            SvcQuery::Actions { unit, filter } => self.action_rows(units, unit, filter),
            SvcQuery::List(filter) => {
                let rows: Vec<LauncherItem> = rank_units(units, filter)
                    .into_iter()
                    .map(|(u, s)| self.unit_row(u, s))
                    .collect();
                if rows.is_empty() {
                    vec![Self::hint(
                        "nomatch",
                        "No matching units",
                        filter,
                        "edit-find-symbolic",
                    )]
                } else {
                    rows
                }
            }
        }
    }

    /// Ctrl+Enter on a unit row → its default action (Restart if running,
    /// else Start) without drilling in.
    fn alt_action(&self, item: &LauncherItem) -> Option<Rc<dyn Fn() + 'static>> {
        let rest = item.id.strip_prefix("svc:unit:")?;
        let (scope, name) = rest.split_once(':')?;
        let scope = Scope::from_label(scope)?;
        let snap = self.cache.get(snapshot)?;
        let unit = snap
            .units
            .iter()
            .find(|u| u.scope == scope && u.name == name)?;
        let verb = default_verb(unit);
        let name = name.to_string();
        Some(Rc::new(move || run_action(scope, verb, name.clone())))
    }

    fn on_opened(&mut self) {
        // States change between opens (and after our own actions).
        self.cache.invalidate();
    }

    fn set_refresh_notifier(&mut self, notifier: RefreshNotifier) {
        self.cache.set_notifier(notifier);
    }

    fn browse(&self, filter: &str) -> Vec<LauncherItem> {
        if filter.is_empty() {
            self.search(PREFIX)
        } else {
            self.search(&format!("{PREFIX} {filter}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    // Real output captured on the owner's machine (trimmed).
    const SYS_FILES: &str = r#"[{"unit_file": "blocky.service", "state": "disabled", "preset": "disabled"}, {"unit_file": "cups.service", "state": "disabled", "preset": "disabled"}, {"unit_file": "dbus-org.freedesktop.login1.service", "state": "alias", "preset": null}, {"unit_file": "getty@.service", "state": "enabled", "preset": "enabled"}, {"unit_file": "sshd.service", "state": "disabled", "preset": "disabled"}, {"unit_file": "systemd-journald.service", "state": "static", "preset": null}]"#;
    const SYS_UNITS: &str = r#"[{"unit": "getty@tty1.service", "load": "loaded", "active": "active", "sub": "running", "description": "Getty on tty1"}, {"unit": "systemd-journald.service", "load": "loaded", "active": "active", "sub": "running", "description": "Journal Service"}, {"unit": "ghost.service", "load": "not-found", "active": "inactive", "sub": "dead", "description": "ghost.service"}]"#;
    const USER_UNITS: &str = r#"[{"unit": "app-snapper\\x2dtools\\x2dcheck@autostart.service", "load": "loaded", "active": "failed", "sub": "failed", "description": "BTRFS Assistant Snapper check"}, {"unit": "mpd.service", "load": "loaded", "active": "active", "sub": "running", "description": "Music Player Daemon"}, {"unit": "pipewire.service", "load": "loaded", "active": "active", "sub": "running", "description": "PipeWire Multimedia Service"}]"#;

    fn sample() -> Vec<Unit> {
        let mut v = parse_scope(Scope::System, SYS_FILES, SYS_UNITS);
        v.extend(parse_scope(Scope::User, "[]", USER_UNITS));
        v
    }

    #[test]
    fn parse_drops_templates_aliases_and_not_found() {
        let v = parse_scope(Scope::System, SYS_FILES, SYS_UNITS);
        let names: Vec<&str> = v.iter().map(|u| u.name.as_str()).collect();
        assert!(names.contains(&"blocky.service"));
        assert!(
            names.contains(&"getty@tty1.service"),
            "template instance kept"
        );
        assert!(!names.contains(&"getty@.service"), "template dropped");
        assert!(
            !names.iter().any(|n| n.starts_with("dbus-org")),
            "alias dropped"
        );
        assert!(!names.contains(&"ghost.service"), "not-found dropped");
        let blocky = v.iter().find(|u| u.name == "blocky.service").unwrap();
        assert_eq!(
            (blocky.active.as_str(), blocky.sub.as_str()),
            ("inactive", "dead")
        );
        let journald = v
            .iter()
            .find(|u| u.name == "systemd-journald.service")
            .unwrap();
        assert_eq!(journald.active, "active");
        assert_eq!(journald.description, "Journal Service");
    }

    #[test]
    fn parse_tolerates_garbage() {
        assert!(parse_scope(Scope::User, "not json", "").is_empty());
    }

    #[test]
    fn empty_filter_puts_failed_first_then_active() {
        let units = sample();
        let ranked = rank_units(&units, "");
        assert_eq!(ranked[0].0.active, "failed");
        let first_inactive = ranked
            .iter()
            .position(|(u, _)| u.active == "inactive")
            .unwrap();
        assert!(
            ranked[..first_inactive]
                .iter()
                .all(|(u, _)| u.active != "inactive")
        );
    }

    #[test]
    fn filter_fuzzy_matches_name_and_description() {
        let units = sample();
        let names: Vec<&str> = rank_units(&units, "blky")
            .iter()
            .map(|(u, _)| u.name.as_str())
            .collect();
        assert_eq!(names.first(), Some(&"blocky.service"));
        let by_desc: Vec<&str> = rank_units(&units, "music")
            .iter()
            .map(|(u, _)| u.name.as_str())
            .collect();
        assert!(by_desc.contains(&"mpd.service"));
    }

    #[test]
    fn verbs_depend_on_state() {
        let units = sample();
        let mpd = units.iter().find(|u| u.name == "mpd.service").unwrap();
        assert_eq!(verbs_for(mpd), vec![Verb::Stop, Verb::Restart]);
        assert_eq!(default_verb(mpd), Verb::Restart);
        let blocky = units.iter().find(|u| u.name == "blocky.service").unwrap();
        assert_eq!(verbs_for(blocky), vec![Verb::Start]);
        let failed = units.iter().find(|u| u.active == "failed").unwrap();
        assert_eq!(default_verb(failed), Verb::Start);
    }

    #[test]
    fn system_scope_uses_plain_systemctl_for_polkit() {
        assert_eq!(
            systemctl_argv(Scope::System, &["restart", "blocky.service"]),
            vec!["systemctl", "restart", "blocky.service"]
        );
        assert_eq!(
            systemctl_argv(Scope::User, &["stop", "mpd.service"]),
            vec!["systemctl", "--user", "stop", "mpd.service"]
        );
    }

    #[test]
    fn query_drills_into_known_unit_only_after_space() {
        let known = |n: &str| n == "mpd.service";
        assert_eq!(
            parse_query("mpd.service", known),
            SvcQuery::List("mpd.service")
        );
        assert_eq!(
            parse_query("mpd.service ", known),
            SvcQuery::Actions {
                unit: "mpd.service",
                filter: ""
            }
        );
        assert_eq!(
            parse_query("mpd.service re", known),
            SvcQuery::Actions {
                unit: "mpd.service",
                filter: "re"
            }
        );
        assert_eq!(
            parse_query("music player", known),
            SvcQuery::List("music player")
        );
    }

    #[test]
    fn outcome_reports_state_or_last_error_line() {
        let (t, b) = outcome_message(Verb::Restart, "blocky.service", true, "", "active\n");
        assert_eq!(t, "Restart blocky.service");
        assert_eq!(b, "blocky.service is now active");
        let (t, b) = outcome_message(
            Verb::Start,
            "sshd.service",
            false,
            "Failed to start sshd.service: Interactive authentication required.\nSee system logs and 'systemctl status sshd.service' for details.\n",
            "",
        );
        assert_eq!(t, "Start sshd.service failed");
        assert!(b.starts_with("See system logs"));
    }

    #[test]
    fn unit_activation_drills_in_and_actions_listed() {
        let captured = Rc::new(RefCell::new(String::new()));
        let cap = captured.clone();
        let p = SystemdProvider::seeded(sample()).with_search_setter(Rc::new(move |t| {
            *cap.borrow_mut() = t.to_string();
        }));
        let rows = p.search("svc mpd");
        let mpd = rows.iter().find(|r| r.name == "mpd.service").unwrap();
        assert!(mpd.description.starts_with("user · active (running)"));
        (mpd.on_activate)();
        assert_eq!(*captured.borrow(), "svc mpd.service ");
        let actions: Vec<String> = p
            .search("svc mpd.service ")
            .into_iter()
            .map(|r| r.name)
            .collect();
        assert_eq!(
            actions,
            vec!["Stop mpd.service (user)", "Restart mpd.service (user)"]
        );
        let filtered: Vec<String> = p
            .search("svc mpd.service res")
            .into_iter()
            .map(|r| r.name)
            .collect();
        assert_eq!(filtered, vec!["Restart mpd.service (user)"]);
    }

    #[test]
    fn prefix_handling() {
        let p = SystemdProvider::seeded(sample());
        assert!(p.handles_command("svc"));
        assert!(p.handles_command("svc mpd"));
        assert!(!p.handles_command("svcx"));
        assert!(p.search("svcx").is_empty());
        assert!(p.alt_action(&p.search("svc mpd")[0]).is_some());
    }
}
