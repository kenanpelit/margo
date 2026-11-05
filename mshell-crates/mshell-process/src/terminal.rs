//! "Open the user's terminal running X" — one rule for the whole shell.
//!
//! Pick: `$TERMINAL` when set (the user said which one), else the first of
//! [`CANDIDATES`] on `$PATH`. Run: each emulator wants a different lead-in
//! before the command (`-e`, `--`, `start --`, or nothing); [`argv`] holds
//! that table once. Spawn: [`command`] wraps it in [`crate::outliving_shell`]
//! so the terminal (a running upgrade, an ssh session, a resumed agent)
//! survives an mshell restart.

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

/// Emulators tried, in order, when `$TERMINAL` is unset.
pub const CANDIDATES: &[&str] = &[
    "kitty",
    "alacritty",
    "foot",
    "wezterm",
    "ghostty",
    "konsole",
    "gnome-terminal",
    "xterm",
];

/// The terminal to use: `$TERMINAL` (trimmed, non-empty), else the first of
/// [`CANDIDATES`] on `$PATH`. `None` when neither gives one.
pub fn pick() -> Option<String> {
    pick_from(
        std::env::var("TERMINAL").ok().as_deref(),
        margo_fs::path::on_path,
    )
}

/// [`pick`] over an explicit `$TERMINAL` value and PATH probe (testable).
pub fn pick_from(env_terminal: Option<&str>, on_path: impl Fn(&str) -> bool) -> Option<String> {
    if let Some(t) = env_terminal.map(str::trim).filter(|t| !t.is_empty()) {
        return Some(t.to_string());
    }
    CANDIDATES
        .iter()
        .find(|t| on_path(t))
        .map(|t| (*t).to_string())
}

/// Arguments (after the terminal binary itself) that make `terminal` run
/// `cmd`. The emulator is matched on the binary's file name, so a full
/// path (`/usr/bin/wezterm`) works too.
pub fn argv<S: AsRef<str>>(terminal: &str, cmd: &[S]) -> Vec<String> {
    let name = Path::new(terminal)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(terminal);
    let mut out: Vec<String> = match name {
        // Take the command as plain trailing arguments.
        "kitty" | "foot" => Vec::new(),
        "wezterm" => vec!["start".into(), "--".into()],
        "gnome-terminal" | "kgx" => vec!["--".into()],
        // alacritty, ghostty, konsole, xterm, st, urxvt, …
        _ => vec!["-e".into()],
    };
    out.extend(cmd.iter().map(|s| s.as_ref().to_string()));
    out
}

/// No terminal emulator: `$TERMINAL` is unset and none of [`CANDIDATES`]
/// is installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoTerminal;

impl std::fmt::Display for NoTerminal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("no terminal emulator found (set $TERMINAL)")
    }
}

impl std::error::Error for NoTerminal {}

/// A [`Command`] that opens the user's terminal running `cmd`, in its own
/// scope (see [`crate::outliving_shell`]; `label` names it). Set cwd / env
/// on the result and spawn it with [`crate::SpawnReaped::spawn_reaped`].
pub fn command<S: AsRef<str>>(cmd: &[S], label: &str) -> Result<(String, Command), NoTerminal> {
    let terminal = pick().ok_or(NoTerminal)?;
    let args = argv(&terminal, cmd);
    let command = crate::outliving_shell(OsStr::new(&terminal), &args, label);
    Ok((terminal, command))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_env_wins_over_the_candidates() {
        assert_eq!(
            pick_from(Some("  footclient "), |_| true).as_deref(),
            Some("footclient")
        );
    }

    /// The bug: Agents → resume and System Updates ignored `$TERMINAL` and
    /// opened kitty even when the user set `TERMINAL=foot`.
    #[test]
    fn unset_or_blank_terminal_falls_back_to_the_first_installed() {
        let installed = |t: &str| t == "alacritty" || t == "xterm";
        assert_eq!(pick_from(None, installed).as_deref(), Some("alacritty"));
        assert_eq!(pick_from(Some(""), installed).as_deref(), Some("alacritty"));
        assert_eq!(pick_from(None, |_| false), None);
    }

    #[test]
    fn argv_per_emulator() {
        let cmd = ["sh", "-c", "echo hi"];
        assert_eq!(argv("kitty", &cmd), ["sh", "-c", "echo hi"]);
        assert_eq!(argv("foot", &cmd), ["sh", "-c", "echo hi"]);
        assert_eq!(
            argv("/usr/bin/wezterm", &cmd),
            ["start", "--", "sh", "-c", "echo hi"]
        );
        assert_eq!(argv("gnome-terminal", &cmd), ["--", "sh", "-c", "echo hi"]);
        assert_eq!(argv("alacritty", &cmd), ["-e", "sh", "-c", "echo hi"]);
        assert_eq!(argv("ghostty", &cmd), ["-e", "sh", "-c", "echo hi"]);
        assert_eq!(argv("konsole", &cmd), ["-e", "sh", "-c", "echo hi"]);
        assert_eq!(argv("xterm", &cmd), ["-e", "sh", "-c", "echo hi"]);
    }
}
