//! MPRIS backend via the `playerctl` CLI.

use super::status::{Command, Status};
use std::process::Command as Proc;

/// Parse `playerctl -l` output: one player name per line, dropping blanks
/// and the `playerctld` proxy, de-duplicated (first occurrence wins).
pub fn parse_player_list(out: &str) -> Vec<String> {
    let mut seen = Vec::new();
    for line in out.lines() {
        let name = line.trim();
        if name.is_empty() || name == "playerctld" {
            continue;
        }
        if !seen.iter().any(|s| s == name) {
            seen.push(name.to_string());
        }
    }
    seen
}

pub fn list() -> Vec<String> {
    match Proc::new("playerctl").arg("-l").output() {
        Ok(o) => parse_player_list(&String::from_utf8_lossy(&o.stdout)),
        Err(_) => Vec::new(),
    }
}

pub fn status(player: &str) -> Status {
    match Proc::new("playerctl")
        .args(["-p", player, "status"])
        .output()
    {
        Ok(o) if o.status.success() => Status::normalize(&String::from_utf8_lossy(&o.stdout)),
        _ => Status::Unknown,
    }
}

pub fn metadata(player: &str, field: &str) -> Option<String> {
    let o = Proc::new("playerctl")
        .args(["-p", player, "metadata", field])
        .output()
        .ok()?;
    if !o.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

pub fn control(player: &str, cmd: Command) -> bool {
    Proc::new("playerctl")
        .args(["-p", player, cmd.mpris_action()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_drops_proxy_and_blanks_and_dedups() {
        let out = "vlc\nplayerctld\n\nfirefox\nvlc\n";
        assert_eq!(parse_player_list(out), vec!["vlc", "firefox"]);
    }
}
