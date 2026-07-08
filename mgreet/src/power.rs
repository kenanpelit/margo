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
    let parsed: Vec<PowerAction> = std::env::var("MLOGIND_POWER")
        .unwrap_or_default()
        .lines()
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
        .collect();

    if parsed.is_empty() {
        defaults()
    } else {
        parsed
    }
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
