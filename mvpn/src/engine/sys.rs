//! Process + environment helpers shared by the engine.
//!
//! Everything that touches the system goes through here so timeouts, env, and
//! non-interactive sudo are handled in one place. A binary launched from the
//! bar does NOT inherit the user's shell-rc env, so anything that needs e.g.
//! `PASSWORD_STORE_DIR` must set it explicitly (see `slot`).

use std::path::PathBuf;
use std::process::{Command, Stdio};

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

/// Run a command for its side effect; returns true on exit-success. Output is
/// silenced so probe commands (e.g. `systemctl list-unit-files`) don't leak
/// their tables onto the terminal when mvpn is run interactively.
pub fn ok(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// What a finished command left behind, for callers that must tell a
/// refused request (exit status + stderr) from a missing program.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    /// The program could be started at all.
    pub spawned: bool,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    /// stdout and stderr together, for message matching.
    pub fn text(&self) -> String {
        format!("{}\n{}", self.stdout, self.stderr)
    }
}

/// Run a command and keep its exit status, stdout and stderr. `input`,
/// when given, is written to its stdin (then closed): the way secrets
/// (the account number) reach `mullvad` without ever being in an argv,
/// where every process of the user can read them.
pub fn run(program: &str, args: &[&str], input: Option<&str>) -> Output {
    use std::io::Write as _;
    let child = Command::new(program)
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let Ok(mut child) = child else {
        return Output::default();
    };
    if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
        // A program that exits without reading closes the pipe: the
        // write fails and its exit status says the rest.
        let _ = stdin.write_all(text.as_bytes());
    }
    match child.wait_with_output() {
        Ok(o) => Output {
            spawned: true,
            success: o.status.success(),
            stdout: String::from_utf8_lossy(&o.stdout).trim().to_string(),
            stderr: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(_) => Output {
            spawned: true,
            ..Output::default()
        },
    }
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
