//! Shared SSH-sessions client — port of the noctalia `ssh-sessions`
//! plugin. Parses `~/.ssh/config` for `Host` entries and detects which
//! of them have a live `ssh` client process, so the bar pill can show
//! an active count and the menu can list hosts (active first) and
//! connect on click. Connections open in **kitty** (`kitty -e ssh
//! <host>`). State is read by polling `pgrep -af "ssh "`; both the pill
//! and the menu drive it through this module.

use std::path::PathBuf;
use tracing::warn;

/// Terminal used to open SSH connections (the user's terminal).
const TERMINAL: &str = "kitty";

/// One parsed `Host` block from `~/.ssh/config`.
#[derive(Debug, Clone)]
pub(crate) struct SshHost {
    /// The `Host` alias — passed verbatim to `ssh`.
    pub name: String,
    pub hostname: String,
    pub user: String,
    pub port: String,
}

impl SshHost {
    /// `user@hostname:port` — the bits that are set, for the row
    /// subtitle. Falls back to the alias when no `Hostname`.
    pub fn subtitle(&self) -> String {
        let mut s = String::new();
        if !self.user.is_empty() {
            s.push_str(&self.user);
            s.push('@');
        }
        s.push_str(if self.hostname.is_empty() {
            &self.name
        } else {
            &self.hostname
        });
        if !self.port.is_empty() && self.port != "22" {
            s.push(':');
            s.push_str(&self.port);
        }
        s
    }
}

fn config_path() -> PathBuf {
    let base = if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home)
    } else {
        PathBuf::from("/root")
    };
    base.join(".ssh").join("config")
}

/// Parse `~/.ssh/config` into hosts, skipping wildcard (`*`) aliases.
/// Mirrors the plugin: only `Host` / `Hostname` / `User` / `Port`
/// matter for display; comments and blanks are ignored.
pub(crate) fn load_hosts() -> Vec<SshHost> {
    let Ok(text) = std::fs::read_to_string(config_path()) else {
        return Vec::new();
    };
    let mut hosts: Vec<SshHost> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let lower = t.to_ascii_lowercase();
        if let Some(rest) = strip_key(t, &lower, "host") {
            // A single `Host` line can list several aliases; the first
            // non-wildcard one names the block.
            let name = rest.split_whitespace().next().unwrap_or("").to_string();
            if name.is_empty() || name.contains('*') {
                // Wildcard / pattern block — don't start a host, and
                // make sure following keys don't attach to the previous.
                hosts.push(SshHost {
                    name: String::new(),
                    hostname: String::new(),
                    user: String::new(),
                    port: String::new(),
                });
                continue;
            }
            hosts.push(SshHost {
                name,
                hostname: String::new(),
                user: String::new(),
                port: String::new(),
            });
        } else if let Some(cur) = hosts.last_mut() {
            if cur.name.is_empty() {
                continue; // inside a wildcard block — ignore
            }
            if let Some(v) = strip_key(t, &lower, "hostname") {
                cur.hostname = v.to_string();
            } else if let Some(v) = strip_key(t, &lower, "user") {
                cur.user = v.to_string();
            } else if let Some(v) = strip_key(t, &lower, "port") {
                cur.port = v.to_string();
            }
        }
    }
    hosts.retain(|h| !h.name.is_empty());
    hosts
}

/// If `line` (with `lower` = its lowercase form) begins with the
/// case-insensitive `key` followed by whitespace, return the trimmed
/// remainder.
fn strip_key<'a>(line: &'a str, lower: &str, key: &str) -> Option<&'a str> {
    let rest = lower.strip_prefix(key)?;
    if !rest.starts_with(|c: char| c.is_whitespace() || c == '=') {
        return None;
    }
    // Slice the original (preserving case) at the same offset.
    let off = key.len();
    Some(
        line[off..]
            .trim_start_matches(|c: char| c.is_whitespace() || c == '=')
            .trim(),
    )
}

/// Poll the live `ssh` client processes and return the set of target
/// hosts (the last non-flag argument after `ssh`). Filters out the
/// daemon / agent / proxy noise the plugin filters.
pub(crate) async fn active_targets() -> Vec<String> {
    let Ok(out) = tokio::process::Command::new("pgrep")
        .args(["-af", "ssh "])
        .output()
        .await
    else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut seen: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Non-client ssh processes + proxy/launcher sub-lines.
        if [
            "sshd",
            "ssh-agent",
            "ssh-add",
            "ssh-keygen",
            "ssh-copy-id",
            "autossh",
            "pgrep",
            "-e ssh",
            " -W ",
        ]
        .iter()
        .any(|needle| line.contains(needle))
        {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        // Index of the `ssh` binary token.
        let Some(ssh_idx) = parts
            .iter()
            .position(|p| *p == "ssh" || p.ends_with("/ssh"))
        else {
            continue;
        };
        // Target = last non-flag arg after `ssh`.
        let target = parts[ssh_idx + 1..]
            .iter()
            .rev()
            .find(|p| !p.starts_with('-'))
            .map(|s| s.to_string());
        if let Some(target) = target
            && !seen.contains(&target)
        {
            seen.push(target);
        }
    }
    seen
}

/// Does any active target resolve to this configured host?
pub(crate) fn host_is_active(host: &SshHost, targets: &[String]) -> bool {
    targets.iter().any(|t| target_matches(t, host))
}

fn target_matches(target: &str, host: &SshHost) -> bool {
    if target == host.name || target == host.hostname {
        return true;
    }
    if !host.user.is_empty()
        && (target == format!("{}@{}", host.user, host.hostname)
            || target == format!("{}@{}", host.user, host.name))
    {
        return true;
    }
    false
}

/// Open a connection to `host` in a fresh kitty window.
pub(crate) fn connect(host: &str) {
    let host = host.to_string();
    relm4::spawn(async move {
        match tokio::process::Command::new(TERMINAL)
            .args(["-e", "ssh", &host])
            .spawn()
        {
            Ok(_) => {}
            Err(e) => warn!(error = %e, host, "ssh: failed to spawn kitty"),
        }
    });
}
