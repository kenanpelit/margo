//! Process + environment helpers shared by the engine.
//!
//! Everything that touches the system goes through here so timeouts, env, and
//! non-interactive sudo are handled in one place. A binary launched from the
//! bar does NOT inherit the user's shell-rc env, so anything that needs e.g.
//! `PASSWORD_STORE_DIR` must set it explicitly (see `slot`).

use std::path::PathBuf;
use std::process::Command;

/// Mullvad data root, honouring `$OSC_MULLVAD_DIR` (osc-mullvad's root
/// override), else `~/.mullvad`. Per-file/dir env vars take precedence over
/// this in the callers.
pub fn mullvad_dir() -> PathBuf {
    if let Ok(d) = std::env::var("OSC_MULLVAD_DIR")
        && !d.is_empty()
    {
        return PathBuf::from(d);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".mullvad")
}

/// Run a command, returning trimmed stdout (stderr discarded). Returns an
/// empty string on spawn failure — callers treat empty as "no data".
pub fn out(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Run a command for its side effect; returns true on exit-success.
pub fn ok(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Run `mullvad <args>` → trimmed stdout.
pub fn mullvad(args: &[&str]) -> String {
    out("mullvad", args)
}

/// Run `mullvad <args>` for effect → success bool.
pub fn mullvad_ok(args: &[&str]) -> bool {
    ok("mullvad", args)
}

/// Non-interactive sudo (`sudo -n …`). Never prompts — if a password would be
/// required it fails fast (returns false) rather than blocking on a polkit /
/// tty prompt, which would deadlock a focused layer-shell panel.
pub fn sudo_n(args: &[&str]) -> bool {
    let mut full = vec!["-n"];
    full.extend_from_slice(args);
    ok("sudo", &full)
}
