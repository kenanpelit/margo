//! Shared SSH-sessions client — port of the noctalia `ssh-sessions`
//! plugin. Reads `~/.ssh/config` (and its `Include`s) for `Host` entries and detects which
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

/// Read `~/.ssh/config` into hosts with the launcher's SSH reader
/// (`Include` followed, every literal `Host` alias its own entry,
/// wildcard / negated patterns and `Match` blocks skipped, sorted by
/// name), so the pill and the launcher list the same hosts.
pub(crate) fn load_hosts() -> Vec<SshHost> {
    load_hosts_from(&config_path())
}

fn load_hosts_from(path: &std::path::Path) -> Vec<SshHost> {
    mshell_launcher::providers::ssh::load_ssh_config(path)
        .into_iter()
        .map(|h| SshHost {
            name: h.name,
            hostname: h.hostname,
            user: h.user,
            port: h.port,
        })
        .collect()
}

/// Poll the live `ssh` client processes and return the set of target
/// hosts (the destination operand of each `ssh` command). Filters out the
/// daemon / agent / proxy noise the plugin filters.
pub(crate) async fn active_targets() -> Vec<String> {
    let Ok(out) = tokio::process::Command::new("pgrep")
        .args(["-af", "ssh "])
        .output()
        .await
    else {
        return Vec::new();
    };
    parse_targets(&String::from_utf8_lossy(&out.stdout))
}

/// Targets in `pgrep -af` output (`<pid> <cmdline>` per line).
fn parse_targets(text: &str) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // `<pid> <argv...>`: only an `ssh` program counts — not sshd,
        // ssh-agent, a `kitty -e ssh` launcher, or a shell / editor whose
        // command line merely contains the word (the program name is
        // checked, so a host called `sshd-box` still counts).
        let mut args = line.split_whitespace().skip(1);
        let Some(argv0) = args.next() else {
            continue;
        };
        if argv0 != "ssh" && !argv0.ends_with("/ssh") {
            continue;
        }
        // `ssh -W host:port jump` is a ProxyJump helper, not a session.
        if line.split_whitespace().any(|a| a.starts_with("-W")) {
            continue;
        }
        let target = destination(args).map(str::to_string);
        if let Some(target) = target
            && !seen.contains(&target)
        {
            seen.push(target);
        }
    }
    seen
}

/// ssh(1) options that take an argument (`-p 2222`, `-p2222`).
const OPTS_WITH_ARG: &str = "BbcDEeFIiJLlmOoPpQRSWw";

/// The destination of `ssh [options] destination [command]`: the first
/// operand, skipping option arguments, so a remote command's words are
/// never taken for the host.
fn destination<'a>(mut args: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    while let Some(a) = args.next() {
        if a == "--" {
            return args.next();
        }
        let Some(flags) = a.strip_prefix('-') else {
            return Some(a);
        };
        // Flags cluster (`-At`); the first one that takes an argument
        // uses the rest of the word, or the next word when none is left.
        if let Some(i) = flags.find(|c| OPTS_WITH_ARG.contains(c))
            && i + 1 == flags.len()
        {
            args.next();
        }
    }
    None
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

#[path = "ssh_tests.rs"]
#[cfg(test)]
mod ssh_tests;
