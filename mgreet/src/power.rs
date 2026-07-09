//! Power actions — the greeter's F-key footer, mirrored from mlogind's TUI
//! (`ui/key_menu.rs`). The orchestrator serialises its resolved
//! `power_controls` (base + extra entries) into `MLOGIND_POWER` as one
//! `key<TAB>hint<TAB>cmd` line per action, so the GUI greeter shows and runs
//! exactly what the TUI greeter would.

use std::process::{Command, Stdio};

#[derive(Clone)]
pub struct PowerAction {
    /// The trigger key name as GTK reports it, e.g. "F1".
    pub key: String,
    /// Short label, e.g. "Shutdown".
    pub hint: String,
    /// Shell command run via `sh -c` when the key is pressed.
    pub cmd: String,
}

/// Parse `MLOGIND_POWER` (`key\thint\tcmd` per line). Falls back to a sensible
/// built-in set when the env var is absent (preview / bare run), so the footer
/// is never empty — though preview never actually runs them.
pub fn from_env() -> Vec<PowerAction> {
    let parsed = parse_power(&std::env::var("MLOGIND_POWER").unwrap_or_default());
    if parsed.is_empty() {
        defaults()
    } else {
        parsed
    }
}

/// Parse the `MLOGIND_POWER` payload — one `key\thint\tcmd` line per action.
/// Split from the env read so it is testable. A line needs all three
/// tab-separated fields with a non-empty key and hint; the command may be empty.
fn parse_power(raw: &str) -> Vec<PowerAction> {
    raw.lines()
        .filter_map(|line| {
            let mut f = line.splitn(3, '\t');
            match (f.next(), f.next(), f.next()) {
                (Some(k), Some(h), Some(c)) if !k.is_empty() && !h.is_empty() => {
                    Some(PowerAction {
                        key: k.to_string(),
                        hint: h.to_string(),
                        cmd: c.to_string(),
                    })
                }
                _ => None,
            }
        })
        .collect()
}

fn defaults() -> Vec<PowerAction> {
    [
        ("F1", "Shutdown", "systemctl poweroff"),
        ("F2", "Reboot", "systemctl reboot"),
        ("F3", "Suspend", "systemctl suspend"),
    ]
    .into_iter()
    .map(|(key, hint, cmd)| PowerAction {
        key: key.to_string(),
        hint: hint.to_string(),
        cmd: cmd.to_string(),
    })
    .collect()
}

/// Run the action's command via `sh -c`, detached. Fire-and-forget: a
/// poweroff/reboot takes the session down anyway; suspend returns and the
/// greeter stays up. Callers gate this on real-greeter mode so a preview run
/// under the live session can never trigger it.
pub fn run(action: &PowerAction) {
    let _ = Command::new("/bin/sh")
        .arg("-c")
        .arg(&action.cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tab_separated_actions() {
        let raw = "F1\tShutdown\tsystemctl poweroff\nF2\tReboot\tsystemctl reboot";
        let a = parse_power(raw);
        assert_eq!(a.len(), 2);
        assert_eq!((a[0].key.as_str(), a[0].hint.as_str()), ("F1", "Shutdown"));
        assert_eq!(a[0].cmd, "systemctl poweroff");
        assert_eq!(a[1].key, "F2");
    }

    #[test]
    fn line_missing_the_command_field_is_dropped() {
        // splitn needs all three tab fields; a two-field line has no cmd → dropped.
        assert!(parse_power("F1\tShutdown").is_empty());
    }

    #[test]
    fn empty_command_field_is_kept() {
        let a = parse_power("F1\tShutdown\t");
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].cmd, "");
    }

    #[test]
    fn blank_key_or_hint_is_rejected() {
        assert!(parse_power("\tShutdown\tcmd").is_empty());
        assert!(parse_power("F1\t\tcmd").is_empty());
    }

    #[test]
    fn command_keeps_embedded_tabs() {
        // splitn(3) stops after the third field, so tabs inside the command survive.
        let a = parse_power("F1\tShutdown\techo a\tb");
        assert_eq!(a[0].cmd, "echo a\tb");
    }

    #[test]
    fn empty_input_parses_to_nothing() {
        assert!(parse_power("").is_empty());
    }

    #[test]
    fn defaults_provide_the_standard_three() {
        let a = defaults();
        assert_eq!(a.len(), 3);
        assert_eq!((a[0].key.as_str(), a[0].hint.as_str()), ("F1", "Shutdown"));
        assert_eq!((a[1].key.as_str(), a[1].hint.as_str()), ("F2", "Reboot"));
        assert_eq!((a[2].key.as_str(), a[2].hint.as_str()), ("F3", "Suspend"));
    }
}
