//! Should a greeter power key be allowed to take the machine down right now?
//!
//! The runner runs power actions as root, and root skips logind's own checks:
//! `systemctl poweroff` without a terminal does not look at inhibitors, and
//! logind grants uid 0 `power-off-multiple-sessions` and `power-off-ignore-inhibit`
//! without asking polkit. So a key on the login screen could shut down or
//! suspend the machine under another user's running session, or through an
//! inhibitor a backup or an update holds — exactly what
//! `60-mlogind-greeter.rules` refuses the greeter user. This puts those two
//! checks back in front of the command.
//!
//! The state comes from logind's files under `/run/systemd`, the same way
//! [`super::session_active`] reads session state rather than linking
//! `libsystemd`. A file whose format we do not recognise blocks nothing: a
//! renamed key in a future systemd must not take the power keys away.

use std::path::Path;

const SESSIONS: &str = "/run/systemd/sessions";
const INHIBITORS: &str = "/run/systemd/inhibit";

/// What a power command does, for matching inhibitors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerKind {
    Shutdown,
    Sleep,
}

impl PowerKind {
    /// The `WHAT=` word an inhibitor uses for this kind.
    fn inhibit_word(self) -> &'static str {
        match self {
            PowerKind::Shutdown => "shutdown",
            PowerKind::Sleep => "sleep",
        }
    }
}

/// Classify a configured power command. Anything that suspends or hibernates
/// is sleep; everything else (poweroff, reboot, halt, a custom script) is
/// treated as shutdown, the stricter guess.
pub fn kind_of(cmd: &str) -> PowerKind {
    let sleeps = cmd
        .split(|c: char| c.is_whitespace() || c == ';' || c == '&')
        .any(|word| {
            matches!(
                word,
                "suspend" | "hibernate" | "hybrid-sleep" | "suspend-then-hibernate" | "sleep"
            )
        });
    if sleeps {
        PowerKind::Sleep
    } else {
        PowerKind::Shutdown
    }
}

/// `KEY=value` lines, quotes around the value stripped.
fn field<'a>(contents: &'a str, key: &str) -> Option<&'a str> {
    contents.lines().find_map(|line| {
        let (k, v) = line.split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"'))
    })
}

/// A logged-in user session: class `user`, not on its way out. A `closing`
/// session is one whose user logged out but left processes behind (tmux, a
/// lingering job); counting it would disable the power keys until reboot.
/// Greeter, manager and background sessions are not someone at a desktop.
fn is_logged_in_user(session: &str) -> bool {
    field(session, "CLASS") == Some("user") && field(session, "STATE") != Some("closing")
}

/// A `block` (or `block-weak`) inhibitor covering `kind`. `delay` only
/// postpones the action, and logind applies it for root callers too.
fn blocks(inhibitor: &str, kind: PowerKind) -> bool {
    matches!(field(inhibitor, "MODE"), Some("block" | "block-weak"))
        && field(inhibitor, "WHAT")
            .is_some_and(|what| what.split(':').any(|w| w == kind.inhibit_word()))
}

/// Why `kind` must not run now, for the greeter to show; `None` to go ahead.
pub fn refusal(sessions: &[String], inhibitors: &[String], kind: PowerKind) -> Option<String> {
    if sessions.iter().any(|s| is_logged_in_user(s)) {
        return Some("Another user is logged in".to_string());
    }
    inhibitors.iter().find(|i| blocks(i, kind)).map(|i| {
        let who = field(i, "WHO")
            .filter(|w| !w.is_empty())
            .unwrap_or("an application");
        // The greeter's status line is short and the text comes from whoever
        // took the inhibitor: keep it bounded and printable.
        let who: String = who.chars().filter(|c| !c.is_control()).take(40).collect();
        format!("Blocked by {who}")
    })
}

/// The regular files in `dir` (logind keeps `.ref` FIFOs next to them, which
/// a read would block on).
fn read_state_files(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .collect()
}

/// [`refusal`] against the live logind state.
pub fn check(kind: PowerKind) -> Option<String> {
    refusal(
        &read_state_files(Path::new(SESSIONS)),
        &read_state_files(Path::new(INHIBITORS)),
        kind,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER_SESSION: &str = "# This is private data. Do not parse.\nUID=1000\nUSER=kenan\nACTIVE=1\nSTATE=active\nCLASS=user\n";
    const GREETER_SESSION: &str = "UID=964\nSTATE=active\nCLASS=greeter\n";
    const MANAGER_SESSION: &str = "UID=1000\nSTATE=active\nCLASS=manager\n";
    const CLOSING_SESSION: &str = "UID=1000\nSTATE=closing\nCLASS=user\n";

    fn inhibitor(what: &str, mode: &str) -> String {
        format!("WHAT={what}\nMODE={mode}\nUID=0\nWHO=pacman\nWHY=\"Updating\"\n")
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn a_logged_in_user_blocks_every_power_action() {
        let sessions = s(&[GREETER_SESSION, USER_SESSION]);
        for kind in [PowerKind::Shutdown, PowerKind::Sleep] {
            assert_eq!(
                refusal(&sessions, &[], kind).as_deref(),
                Some("Another user is logged in")
            );
        }
    }

    #[test]
    fn greeter_manager_and_closing_sessions_do_not_count() {
        let sessions = s(&[GREETER_SESSION, MANAGER_SESSION, CLOSING_SESSION]);
        assert_eq!(refusal(&sessions, &[], PowerKind::Shutdown), None);
    }

    #[test]
    fn a_block_inhibitor_stops_only_what_it_covers() {
        let inhibitors = vec![inhibitor("shutdown:idle", "block")];
        assert_eq!(
            refusal(&[], &inhibitors, PowerKind::Shutdown).as_deref(),
            Some("Blocked by pacman")
        );
        assert_eq!(refusal(&[], &inhibitors, PowerKind::Sleep), None);
    }

    #[test]
    fn delay_inhibitors_do_not_block_but_weak_blocks_do() {
        let inhibitors = vec![inhibitor("sleep", "delay")];
        assert_eq!(refusal(&[], &inhibitors, PowerKind::Sleep), None);
        let weak = vec![inhibitor("shutdown", "block-weak")];
        assert!(refusal(&[], &weak, PowerKind::Shutdown).is_some());
    }

    #[test]
    fn an_unrecognised_file_blocks_nothing() {
        let junk = s(&["garbage", ""]);
        assert_eq!(refusal(&junk, &junk, PowerKind::Shutdown), None);
    }

    #[test]
    fn power_commands_are_classified() {
        assert_eq!(kind_of("systemctl suspend -l"), PowerKind::Sleep);
        assert_eq!(kind_of("systemctl hibernate"), PowerKind::Sleep);
        assert_eq!(kind_of("loginctl suspend-then-hibernate"), PowerKind::Sleep);
        assert_eq!(kind_of("systemctl poweroff -l"), PowerKind::Shutdown);
        assert_eq!(kind_of("systemctl reboot -l"), PowerKind::Shutdown);
        assert_eq!(kind_of("/usr/local/bin/my-off"), PowerKind::Shutdown);
    }

    #[test]
    fn the_inhibitor_owner_is_bounded_and_printable() {
        let i = "WHAT=shutdown\nMODE=block\nWHO=\"evil\u{1b}[2Jname that goes on and on and on and on\"\n";
        let msg = refusal(&[], &[i.to_string()], PowerKind::Shutdown).unwrap();
        assert!(!msg.contains('\u{1b}'));
        assert!(msg.chars().count() <= "Blocked by ".len() + 40);
    }
}
